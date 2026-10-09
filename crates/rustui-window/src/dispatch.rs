//! Hit testing, event delivery and focus traversal for one window (§10, §11, §12).
//!
//! Delivery follows one rule: an event is *queued*, never delivered re-entrantly.
//! `dispatch` pushes the target into the [`rustui_core::EventQueue`] and then drains the queue;
//! a widget that does not consume an event gets its parent queued for the bubble phase, so a
//! label inside a button still lets the button react to the click.

use std::cell::Cell;

use rustui_core::{
    CursorIcon, EventContext, EventPhase, EventResult, FocusReason, Key, NamedKey, ObjectId,
    ObjectRegistry, Point, UiContext, UiEvent, WidgetStore,
};

use crate::window::{EventOutcome, FocusDirection, Window};

/// Upper bound on queued deliveries per dispatched event; guards against widgets that keep
/// re-sending events to each other.
const MAX_DELIVERIES: usize = 10_000;

impl Window {
    // ---- hit testing (§11) ---------------------------------------------

    /// Chain of widgets under `position` in window coordinates, root first.
    ///
    /// Overlays are tested before the content because they are painted above it; children are
    /// tested in reverse paint order because the last child is the topmost one.
    pub(crate) fn hit_path(&self, store: &WidgetStore, position: Point) -> Vec<ObjectId> {
        let mut path = Vec::new();
        for id in self.overlays.ordered().into_iter().rev() {
            if id == self.root {
                continue;
            }
            path.clear();
            if hit_subtree(store, id, position, &mut path) {
                path.reverse();
                return path;
            }
        }
        path.clear();
        if hit_subtree(store, self.root, position, &mut path) {
            path.reverse();
            return path;
        }
        Vec::new()
    }

    // ---- event delivery (§10) ------------------------------------------

    /// Routes `event` through the tree and drains the resulting queue.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch(
        &mut self,
        ui: &UiContext,
        store: &WidgetStore,
        registry: &ObjectRegistry,
        event: &UiEvent,
    ) -> EventOutcome {
        // Events may arrive before the first frame; lay out once so hit testing works.
        if !self.laid_out {
            self.layout(ui, store, registry);
        }

        let mut outcome = EventOutcome::default();

        // ---- window level bookkeeping ----
        match event {
            UiEvent::Resize(size) => {
                self.set_size(*size);
                outcome.request_redraw = true;
            }
            UiEvent::ScaleFactorChanged(factor) => {
                self.set_scale_factor(*factor);
                outcome.request_redraw = true;
            }
            UiEvent::WindowFocus(focused) => {
                self.interaction.window_focused = *focused;
                outcome.request_redraw = true;
            }
            _ => {}
        }

        // ---- window wide keyboard shortcuts ----
        if let UiEvent::KeyDown {
            key: Key::Named(NamedKey::Tab),
            modifiers,
            ..
        } = event
        {
            let direction = if modifiers.shift {
                FocusDirection::Backward
            } else {
                FocusDirection::Forward
            };
            self.focus_step(store, direction);
            self.drain(ui, store, registry);
            self.sync_interaction();
            return EventOutcome {
                handled: true,
                request_redraw: true,
                ..EventOutcome::default()
            };
        }
        // The key event above already moved focus; a literal tab must not reach a text field.
        if let UiEvent::TextInput(text) = event {
            if text == "\t" {
                return EventOutcome {
                    handled: true,
                    ..EventOutcome::default()
                };
            }
        }

        // ---- hit test and hover (§11) ----
        let captured = self.queue.captured();
        let path: Vec<ObjectId> = match (event.position(), captured) {
            (Some(_), Some(owner)) => ancestry(registry, owner),
            (Some(position), None) => self.hit_path(store, position),
            (None, Some(owner)) => ancestry(registry, owner),
            (None, None) => self.interaction.hover_path.clone(),
        };

        if let Some(position) = event.position() {
            self.interaction.pointer = position;
        }
        if event.position().is_some() && captured.is_none() {
            self.update_hover(store, &path, &mut outcome);
        }

        // ---- focus follows the pointer (§12) ----
        if matches!(event, UiEvent::MouseDown { .. }) {
            match path.iter().rev().find(|id| focusable(store, **id)).copied() {
                Some(id) => self.focus.set_focus(id, FocusReason::Pointer),
                None => self.focus.clear_focus(),
            }
        }

        // ---- route and deliver ----
        let target = self.route(&path, event);
        if target.is_valid() && store.contains(target) {
            self.queue.push(target, event, EventPhase::Target);
        }
        let handled = self.drain(ui, store, registry);
        outcome.handled = handled;

        self.finish_pointer_state(store, event, &path);
        // Overlay requests made while handling the event are visible right away.
        self.overlays.apply_pending();

        if matches!(event, UiEvent::CloseRequested) {
            outcome.close_requested = !handled;
            self.close_requested = outcome.close_requested;
        }
        if handled {
            outcome.request_redraw = true;
        }
        outcome
    }

    /// Chooses the object an event starts at.
    fn route(&self, path: &[ObjectId], event: &UiEvent) -> ObjectId {
        match event {
            UiEvent::KeyDown { .. }
            | UiEvent::KeyUp { .. }
            | UiEvent::TextInput(_)
            | UiEvent::ImeStart
            | UiEvent::ImeUpdate { .. }
            | UiEvent::ImeCommit(_)
            | UiEvent::ImeEnd
            | UiEvent::FocusIn(_)
            | UiEvent::FocusOut => self.focus.focused().unwrap_or(self.root),
            _ => path
                .last()
                .copied()
                .or(self.interaction.hovered)
                .unwrap_or(self.root),
        }
    }

    /// Delivers everything that is queued, including the focus notifications generated meanwhile.
    fn drain(&mut self, ui: &UiContext, store: &WidgetStore, registry: &ObjectRegistry) -> bool {
        let mut handled = false;
        let mut deliveries = 0usize;

        loop {
            if self.queue.is_empty() {
                if !self.focus.has_pending() {
                    break;
                }
                self.flush_focus(store);
                if self.queue.is_empty() {
                    break;
                }
            }
            deliveries += 1;
            if deliveries > MAX_DELIVERIES {
                log::warn!("event dispatch did not settle after {deliveries} deliveries");
                self.queue.clear();
                break;
            }

            let Some(queued) = self.queue.pop() else {
                break;
            };
            if !store.contains(queued.target) {
                self.queue.finish();
                continue;
            }
            self.sync_interaction();

            let depth = registry.depth_of(queued.target);
            let consumed = Cell::new(false);
            let result;
            {
                let mut ctx = EventContext::new(
                    ui,
                    store,
                    &self.interaction,
                    &self.focus,
                    &self.queue,
                    &self.overlays,
                    depth,
                    &consumed,
                );
                let Some(mut widget) = store.take(queued.target) else {
                    self.queue.finish();
                    continue;
                };
                result = widget.event(&mut ctx, &queued.event);
                store.put(queued.target, widget);
            }

            let accepted = consumed.get() || result == EventResult::Handled;
            handled |= accepted;

            // Synthetic notifications are addressed to every widget of the chain individually,
            // so they must not travel a second time through the ancestors.
            let synthetic = matches!(
                queued.event,
                UiEvent::MouseEnter | UiEvent::MouseLeave | UiEvent::FocusIn(_) | UiEvent::FocusOut
            );
            if !accepted && !synthetic {
                if let Some(parent) = registry.parent(queued.target) {
                    self.queue
                        .push_front(parent, &queued.event, EventPhase::Bubble);
                }
            }
            self.queue.finish();
        }

        self.sync_interaction();
        handled
    }

    /// Converts queued focus changes into `FocusIn` / `FocusOut` deliveries.
    fn flush_focus(&self, store: &WidgetStore) {
        for (id, reason) in self.focus.take_pending() {
            if !store.contains(id) {
                continue;
            }
            let event = match reason {
                Some(reason) => UiEvent::FocusIn(reason),
                None => UiEvent::FocusOut,
            };
            self.queue.push(id, &event, EventPhase::Target);
        }
    }

    fn sync_interaction(&mut self) {
        self.interaction.focused = self.focus.focused();
        self.interaction.captured = self.queue.captured();
    }

    /// Updates hover state, generating `MouseLeave` / `MouseEnter` for the widgets that changed.
    fn update_hover(&mut self, store: &WidgetStore, path: &[ObjectId], outcome: &mut EventOutcome) {
        if self.interaction.hover_path == path {
            return;
        }
        let previous = std::mem::replace(&mut self.interaction.hover_path, path.to_vec());
        self.interaction.hovered = path.last().copied();

        for id in previous.iter().rev() {
            if !path.contains(id) && store.contains(*id) {
                self.queue
                    .push(*id, &UiEvent::MouseLeave, EventPhase::Target);
            }
        }
        for id in path {
            if !previous.contains(id) && store.contains(*id) {
                self.queue
                    .push(*id, &UiEvent::MouseEnter, EventPhase::Target);
            }
        }
        outcome.request_redraw = true;
    }

    /// Pressed / capture / cursor bookkeeping that depends on the delivered event.
    fn finish_pointer_state(&mut self, store: &WidgetStore, event: &UiEvent, path: &[ObjectId]) {
        match event {
            UiEvent::MouseDown { .. } => {
                // The widget that grabbed the pointer is the pressed one; without a grab fall
                // back to the deepest focusable widget of the chain (a button behind its label).
                let pressed = self
                    .queue
                    .captured()
                    .or_else(|| path.iter().rev().find(|id| focusable(store, **id)).copied())
                    .or_else(|| path.last().copied());
                self.interaction.pressed = pressed;
            }
            UiEvent::MouseUp { .. } => {
                self.interaction.pressed = None;
                // A press & drag ends with the release, capture included.
                self.queue.release_pointer();
            }
            _ => {}
        }
        self.sync_interaction();

        // Cursor: explicit request wins, otherwise the hovered widget decides (§40).
        let requested = self.queue.take_cursor();
        if requested != CursorIcon::Default {
            self.cursor = requested;
        } else if event.position().is_some() {
            self.cursor = path
                .last()
                .and_then(|id| store.get(*id).map(|widget| widget.cursor()))
                .unwrap_or_default();
        }
    }

    // ---- focus traversal (§12) -----------------------------------------

    /// Moves focus to the next focusable widget; returns the new focus owner.
    pub(crate) fn focus_step(
        &mut self,
        store: &WidgetStore,
        direction: FocusDirection,
    ) -> Option<ObjectId> {
        let mut order = Vec::new();
        if self.root.is_valid() && store.contains(self.root) {
            store.for_each_in_order(self.root, &mut |id, _depth| {
                if focusable(store, id) {
                    order.push(id);
                }
            });
        }
        if order.is_empty() {
            self.focus.clear_focus();
            return None;
        }

        let current = self
            .focus
            .focused()
            .and_then(|focused| order.iter().position(|id| *id == focused));
        let next = match (current, direction) {
            (Some(index), FocusDirection::Forward) => (index + 1) % order.len(),
            (Some(index), FocusDirection::Backward) => {
                if index == 0 {
                    order.len() - 1
                } else {
                    index - 1
                }
            }
            (None, FocusDirection::Forward) => 0,
            (None, FocusDirection::Backward) => order.len() - 1,
        };
        let target = order[next];
        self.focus.set_focus(target, FocusReason::Tab);
        Some(target)
    }

    /// Moves focus to `id` and delivers the notifications; reports whether `id` is focusable
    /// content of this window.
    pub(crate) fn focus_object(
        &mut self,
        ui: &UiContext,
        store: &WidgetStore,
        registry: &ObjectRegistry,
        id: ObjectId,
        reason: FocusReason,
    ) -> bool {
        let known = store.contains(id);
        if known {
            self.focus.set_focus(id, reason);
        } else {
            self.focus.clear_focus();
        }
        self.drain(ui, store, registry);
        self.sync_interaction();
        known
    }

    /// Focuses the next / previous widget and delivers the notifications.
    pub(crate) fn focus_move(
        &mut self,
        ui: &UiContext,
        store: &WidgetStore,
        registry: &ObjectRegistry,
        direction: FocusDirection,
    ) -> Option<ObjectId> {
        let target = self.focus_step(store, direction);
        self.drain(ui, store, registry);
        self.sync_interaction();
        target
    }
}

/// `true` when `id` can take keyboard focus right now.
fn focusable(store: &WidgetStore, id: ObjectId) -> bool {
    store
        .get(id)
        .map(|widget| {
            widget.accepts_focus() && widget.base().is_visible() && widget.base().is_enabled()
        })
        .unwrap_or(false)
}

/// Chain from the window root down to `id`, root first.
fn ancestry(registry: &ObjectRegistry, id: ObjectId) -> Vec<ObjectId> {
    let mut chain = vec![id];
    let mut current = id;
    while let Some(parent) = registry.parent(current) {
        chain.push(parent);
        current = parent;
    }
    chain.reverse();
    chain
}

/// Pushes the chain of widgets under `position` into `out` (leaf first) and reports a hit.
///
/// Descendants are visited in reverse paint order, so the topmost widget wins.
fn hit_subtree(
    store: &WidgetStore,
    id: ObjectId,
    position: Point,
    out: &mut Vec<ObjectId>,
) -> bool {
    let Some(widget) = store.get(id) else {
        return false;
    };
    if !widget.base().is_visible() || !widget.hit_test(position) {
        return false;
    }
    let children = widget.children().to_vec();
    drop(widget);

    for child in children.into_iter().rev() {
        if hit_subtree(store, child, position, out) {
            out.push(id);
            return true;
        }
    }
    out.push(id);
    true
}
