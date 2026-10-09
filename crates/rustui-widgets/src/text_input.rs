//! Single line text input with caret, selection, clipboard and IME support (design document §48).

use rustui_core::{
    clamp_scroll, impl_widget_base, ArrangeContext, CursorIcon, EventContext, EventResult, Fill,
    Key, MeasureContext, Modifiers, MouseButton, NamedKey, PaintContext, Point, Rect,
    SharedProperty, Signal, Size, SizeHint, TextRole, TextStyle, UiContext, UiEvent, UpdateContext,
    Widget, WidgetBase,
};

use crate::common::{control_radius, focus_ring};

/// A single line text field.
///
/// The field owns no window of state that the application cannot reach: the edited string lives in
/// a [`SharedProperty`] (read it any time), and every edit is mirrored into
/// [`TextInput::changed`] while `Enter` produces [`TextInput::submitted`].
///
/// ```ignore
/// let name = SharedProperty::new(String::new());
/// let input = TextInput::new().with_text(name.clone()).placeholder("Your name");
/// input.changed().connect(|text| println!("edited: {text}"));
/// ```
#[derive(Debug)]
pub struct TextInput {
    base: WidgetBase,
    text: SharedProperty<String>,
    placeholder: String,
    /// Caret, in **characters** (converted to bytes only when slicing).
    caret: usize,
    /// Selection anchor, in characters; equals `caret` when the selection is collapsed.
    anchor: usize,
    scroll_x: f32,
    password: bool,
    focused: bool,
    /// IME composition in progress, painted with an underline.
    composing: Option<String>,
    /// Last time the caret was moved or edited, in seconds, for the blink phase.
    blink_epoch: f32,
    /// Set by the event handlers (which have no clock); consumed by the next `update`.
    blink_reset: bool,
    blink_on: bool,
    changed: Signal<String>,
    submitted: Signal<String>,
    tooltip: Option<String>,
}

impl TextInput {
    /// An empty text field.
    pub fn new() -> Self {
        Self::with_text(SharedProperty::new(String::new()))
    }

    /// A text field bound to `text`.
    pub fn with_text(text: SharedProperty<String>) -> Self {
        Self {
            base: WidgetBase::new(),
            text,
            placeholder: String::new(),
            caret: 0,
            anchor: 0,
            scroll_x: 0.0,
            password: false,
            focused: false,
            composing: None,
            blink_epoch: 0.0,
            blink_reset: false,
            blink_on: true,
            changed: Signal::new(),
            submitted: Signal::new(),
            tooltip: None,
        }
    }

    /// Current text.
    pub fn text(&self) -> String {
        self.text.get()
    }

    /// Replaces the text and collapses the selection at the end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.text.set(text);
        let chars = self.text.get().chars().count();
        self.caret = chars;
        self.anchor = chars;
        self.scroll_x = 0.0;
    }

    /// The shared text property (clone it to keep a handle).
    pub fn text_property(&self) -> SharedProperty<String> {
        self.text.clone()
    }

    /// Fires after every edit, with the new value.
    pub fn changed(&self) -> &Signal<String> {
        &self.changed
    }

    /// Fires when `Enter` is pressed while the field has focus.
    pub fn submitted(&self) -> &Signal<String> {
        &self.submitted
    }

    /// Placeholder shown while the field is empty.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Masks the content (for passwords).
    pub fn password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    /// Sets the tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Caret position in characters.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Selected range as `(start, end)` in characters, ordered.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// `true` when a non empty range is selected.
    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    /// The selected text.
    pub fn selected_text(&self) -> String {
        let text = self.text.get();
        let (start, end) = self.selection();
        let bytes = (char_to_byte(&text, start), char_to_byte(&text, end));
        text.get(bytes.0..bytes.1).unwrap_or_default().to_string()
    }

    /// Selects the whole content and moves the caret to the end.
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.get().chars().count();
    }

    /// Collapses the selection onto the caret.
    pub fn clear_selection(&mut self) {
        self.anchor = self.caret;
    }

    // -- geometry ---------------------------------------------------------

    fn style(&self, ui: &UiContext, display_empty: bool) -> TextStyle {
        let theme = ui.theme();
        let color = if display_empty {
            theme.colors.text_muted
        } else {
            theme.colors.text
        };
        theme.text(TextRole::Body, Some(color))
    }

    /// What is painted: the real text, or one mask character per input character.
    fn display_text(&self) -> String {
        let text = self.text.get();
        if self.password {
            text.chars().map(|_| '\u{2022}').collect()
        } else if let Some(composing) = &self.composing {
            format!("{text}{composing}")
        } else {
            text
        }
    }

    /// Rectangle the text is drawn in, in window coordinates.
    fn text_rect(&self, ui: &UiContext, bounds: Rect) -> Rect {
        bounds.shrink(ui.theme().content_padding())
    }

    /// Caret rectangle in **window** coordinates, given the field's bounds in window coordinates.
    ///
    /// Shared by [`Widget::ime_cursor_area`] and [`Widget::paint`] so the input method candidate
    /// window can never drift away from the caret that is actually drawn (T-1).
    fn caret_rect(&self, ui: &UiContext, bounds: Rect) -> Rect {
        let text_rect = self.text_rect(ui, bounds);
        let display = self.display_text();
        let style = self.style(ui, display.is_empty());
        let byte = char_to_byte(&display, self.caret.min(display.chars().count()));
        let x = ui.text().measure_text(&display[..byte], &style, None).width;
        Rect::new(
            text_rect.x + x - self.scroll_x,
            text_rect.y,
            ui.theme().borders.thin.max(1.0),
            ui.text().line_height(&style),
        )
    }

    /// Caret (character index) at a window `x` position.
    fn caret_at(&self, ctx: &EventContext<'_>, position: Point) -> usize {
        let bounds = self.base.window_bounds();
        let rect = self.text_rect(ctx.ui(), bounds);
        let display = self.display_text();
        let style = self.style(ctx.ui(), display.is_empty());
        let x = position.x - rect.x + self.scroll_x;
        if x <= 0.0 {
            return 0;
        }
        let mut previous = 0.0;
        let mut index = 0usize;
        for (byte, ch) in display.char_indices() {
            let width = ctx
                .measure_text(&display[..byte + ch.len_utf8()], &style, None)
                .width;
            if x < (previous + width) * 0.5 {
                break;
            }
            previous = width;
            index = byte + ch.len_utf8();
        }
        // Convert the byte offset inside the display string back to a character index.
        display[..index].chars().count()
    }

    /// Keeps the caret inside the visible strip.
    fn ensure_caret_visible(&mut self, ctx: &EventContext<'_>) {
        let bounds = self.base.window_bounds();
        let rect = self.text_rect(ctx.ui(), bounds);
        let display = self.display_text();
        let style = self.style(ctx.ui(), display.is_empty());
        let prefix_end = char_to_byte(&display, self.caret.min(display.chars().count()));
        let caret_x = ctx.measure_text(&display[..prefix_end], &style, None).width;
        let view = rect.width.max(1.0);
        if caret_x - self.scroll_x > view {
            self.scroll_x = caret_x - view;
        } else if caret_x - self.scroll_x < 0.0 {
            self.scroll_x = caret_x;
        }
        let total = ctx.measure_text(&display, &style, None).width;
        self.scroll_x = clamp_scroll(self.scroll_x, total, view);
    }

    // -- editing ----------------------------------------------------------

    fn emit_changed(&self) {
        self.changed.emit(&self.text.get());
    }

    /// Replaces the selection (or nothing) with `insert`, placing the caret behind it.
    fn replace_selection(&mut self, insert: &str) {
        let mut text = self.text.get();
        let (start, end) = self.selection();
        let begin = char_to_byte(&text, start);
        let stop = char_to_byte(&text, end);
        if begin <= stop && stop <= text.len() {
            text.replace_range(begin..stop, insert);
        }
        let caret = start + insert.chars().count();
        self.text.set(text);
        self.caret = caret;
        self.anchor = caret;
        self.emit_changed();
    }

    fn delete_backward(&mut self) {
        if self.has_selection() {
            self.replace_selection("");
        } else if self.caret > 0 {
            self.anchor = self.caret - 1;
            self.replace_selection("");
        }
    }

    fn delete_forward(&mut self) {
        if self.has_selection() {
            self.replace_selection("");
        } else if self.caret < self.text.get().chars().count() {
            self.anchor = self.caret + 1;
            self.replace_selection("");
        }
    }

    fn move_caret(&mut self, target: usize, extend: bool) {
        let count = self.text.get().chars().count();
        self.caret = target.min(count);
        if !extend {
            self.anchor = self.caret;
        }
    }

    fn move_left(&mut self, word: bool, extend: bool) {
        let text = self.text.get();
        let target = if word {
            word_left(&text, self.caret)
        } else if !extend && self.has_selection() {
            self.selection().0
        } else {
            self.caret.saturating_sub(1)
        };
        self.move_caret(target, extend);
    }

    fn move_right(&mut self, word: bool, extend: bool) {
        let text = self.text.get();
        let target = if word {
            word_right(&text, self.caret)
        } else if !extend && self.has_selection() {
            self.selection().1
        } else {
            (self.caret + 1).min(text.chars().count())
        };
        self.move_caret(target, extend);
    }

    /// Selects the word under `caret`, falling back to the whole content.
    fn select_word(&mut self) {
        let text = self.text.get();
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            self.caret = 0;
            self.anchor = 0;
            return;
        }
        let at = self.caret.min(chars.len().saturating_sub(1));
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(chars[at]) {
            // Not on a word: double click on whitespace selects the whole field.
            self.select_all();
            return;
        }
        let mut start = at;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < chars.len() && is_word(chars[end]) {
            end += 1;
        }
        self.anchor = start;
        self.caret = end;
    }

    fn copy(&self, ctx: &EventContext<'_>) {
        let selected = self.selected_text();
        if !selected.is_empty() {
            if let Some(clipboard) = ctx.clipboard() {
                clipboard.set_text(&selected);
            }
        }
    }

    fn paste(&mut self, ctx: &EventContext<'_>) {
        let Some(clipboard) = ctx.clipboard() else {
            return;
        };
        let pasted = clipboard.get_text().unwrap_or_default();
        if !pasted.is_empty() {
            self.replace_selection(&pasted);
        }
    }

    // -- events -----------------------------------------------------------

    fn handle_key(
        &mut self,
        ctx: &mut EventContext<'_>,
        key: &Key,
        modifiers: &Modifiers,
    ) -> EventResult {
        if !self.base.is_enabled() {
            return EventResult::Unhandled;
        }
        let extend = modifiers.shift;
        let word = modifiers.control;
        let command = modifiers.command();
        let handled = match key {
            Key::Named(NamedKey::ArrowLeft) => {
                self.move_left(word, extend);
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.move_right(word, extend);
                true
            }
            Key::Named(NamedKey::Home) => {
                self.move_caret(0, extend);
                true
            }
            Key::Named(NamedKey::End) => {
                self.move_caret(self.text.get().chars().count(), extend);
                true
            }
            Key::Named(NamedKey::Backspace) => {
                self.delete_backward();
                true
            }
            Key::Named(NamedKey::Delete) => {
                self.delete_forward();
                true
            }
            Key::Named(NamedKey::Return) | Key::Named(NamedKey::Enter) => {
                let text = self.text.get();
                self.submitted.emit(&text);
                true
            }
            Key::Character(c) if command && c.eq_ignore_ascii_case("a") => {
                self.select_all();
                true
            }
            Key::Character(c) if command && c.eq_ignore_ascii_case("c") => {
                self.copy(ctx);
                true
            }
            Key::Character(c) if command && c.eq_ignore_ascii_case("x") => {
                self.copy(ctx);
                self.replace_selection("");
                true
            }
            Key::Character(c) if command && c.eq_ignore_ascii_case("v") => {
                self.paste(ctx);
                true
            }
            _ => false,
        };
        if !handled {
            return EventResult::Unhandled;
        }
        self.blink_reset = true;
        self.ensure_caret_visible(ctx);
        ctx.request_redraw();
        ctx.accept();
        EventResult::Handled
    }
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}

impl_widget_base!(TextInput);

impl Widget for TextInput {
    fn kind(&self) -> &'static str {
        "TextInput"
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn is_opaque(&self) -> bool {
        true
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Text
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    /// The candidate window of an input method follows the caret while the field is focused
    /// (T-1); `None` while nothing editable owns the keyboard, which switches composition off.
    fn ime_cursor_area(&self, ui: &UiContext) -> Option<Rect> {
        if !self.focused || !self.base.is_visible() || !self.base.is_enabled() {
            return None;
        }
        Some(self.caret_rect(ui, self.base.window_bounds()))
    }

    /// The field takes the width it is offered but hugs its control height.
    fn size_hint(&self) -> SizeHint {
        SizeHint {
            width: f32::INFINITY,
            ..SizeHint::default()
        }
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let theme = ctx.ui().theme();
        let pad = theme.content_padding();
        let style = self.style(ctx.ui(), true);
        let line = ctx.measure_text(" ", &style, None).height;
        let height = (line + pad.vertical()).max(theme.control_height());
        // The field fills the width it is offered; with an infinite offer (shrink-to-fit parent)
        // it keeps room for a few characters plus the placeholder.
        let width = if available.width.is_finite() {
            available.width.max(0.0)
        } else {
            160.0
        };
        let size = Size::new(
            width,
            if available.height.is_finite() {
                height.min(available.height.max(0.0))
            } else {
                height
            },
        );
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, _ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
    }

    fn update(&mut self, ctx: &mut UpdateContext<'_>) {
        let now = ctx.clock().elapsed_seconds();
        if self.blink_reset {
            self.blink_reset = false;
            self.blink_epoch = now;
            self.blink_on = true;
        }
        if !self.focused {
            return;
        }
        let on = ((now - self.blink_epoch) % 1.0) < 0.6;
        if on != self.blink_on {
            self.blink_on = on;
            ctx.request_redraw();
        }
    }

    fn event(&mut self, ctx: &mut EventContext<'_>, event: &UiEvent) -> EventResult {
        let id = self.id();
        let enabled = self.base.is_enabled();
        match event {
            UiEvent::FocusIn(_) => {
                self.focused = true;
                self.blink_reset = true;
                self.blink_on = true;
                ctx.request_redraw();
                EventResult::Handled
            }
            UiEvent::FocusOut => {
                self.focused = false;
                ctx.request_redraw();
                EventResult::Handled
            }
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
            } if enabled => {
                let target = self.caret_at(ctx, *position);
                self.caret = target;
                self.anchor = target;
                self.composing = None;
                self.blink_reset = true;
                self.blink_on = true;
                ctx.capture_pointer(id);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseDoubleClick {
                button: MouseButton::Left,
                position,
            } if enabled => {
                self.caret = self.caret_at(ctx, *position);
                self.select_word();
                self.blink_reset = true;
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::MouseMove { position } if enabled => {
                if ctx.interaction().owns_pointer(id) {
                    self.caret = self.caret_at(ctx, *position);
                    self.ensure_caret_visible(ctx);
                    ctx.request_redraw();
                    ctx.accept();
                    return EventResult::Handled;
                }
                EventResult::Unhandled
            }
            UiEvent::MouseUp { .. } if ctx.interaction().owns_pointer(id) => {
                ctx.release_pointer();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::KeyDown { key, modifiers, .. } => self.handle_key(ctx, key, modifiers),
            UiEvent::TextInput(text) if enabled => {
                self.composing = None;
                self.replace_selection(text);
                self.blink_reset = true;
                self.blink_on = true;
                self.ensure_caret_visible(ctx);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::ImeStart if enabled => {
                self.composing = Some(String::new());
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::ImeUpdate { text, .. } if enabled => {
                self.composing = Some(text.clone());
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::ImeCommit(text) if enabled => {
                let composed = self.composing.take().unwrap_or_default();
                // Some platforms commit the composition, others only the final chunk.
                let insert = if composed.is_empty() || text.is_empty() {
                    composed + text
                } else if text.starts_with(&composed) || composed.contains(text) {
                    composed
                } else {
                    text.clone()
                };
                if !insert.is_empty() {
                    self.replace_selection(&insert);
                }
                self.blink_reset = true;
                self.ensure_caret_visible(ctx);
                ctx.request_redraw();
                ctx.accept();
                EventResult::Handled
            }
            UiEvent::ImeEnd => {
                self.composing = None;
                ctx.request_redraw();
                EventResult::Handled
            }
            UiEvent::MouseEnter => {
                ctx.set_cursor(CursorIcon::Text);
                EventResult::Handled
            }
            _ => EventResult::Unhandled,
        }
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let id = self.id();
        let state = ctx.state_of(id, self.base.state());
        let theme = ctx.ui().theme();
        // Window coordinates: the painter takes no offset.
        let bounds = ctx.to_window_rect(self.base.bounds());
        let radius = control_radius(ctx);
        let border = theme.input.border(state);
        let bg = theme.input.background(state);
        let mut fill = Fill::rounded(bg, theme.radius.sm);
        fill.border = border;
        ctx.painter().styled_rect(bounds, radius, fill);

        let pad = theme.content_padding();
        let text_rect = Rect::new(
            bounds.x + pad.left,
            bounds.y + pad.top,
            (bounds.width - pad.horizontal()).max(0.0),
            (bounds.height - pad.vertical()).max(0.0),
        );
        let display = self.display_text();
        let empty = display.is_empty();
        let mut style = self.style(ctx.ui(), empty);
        if !state.enabled {
            style.color = theme.colors.text_disabled;
        }
        let line_height = ctx.line_height(&style);

        // `ctx.push_clip*` takes local coordinates; `bounds` above is already in window space.
        ctx.push_clip_rounded(self.base.bounds(), theme.radius.sm);

        // Selection highlight behind the text.
        let (sel_start, sel_end) = self.selection();
        if sel_start != sel_end && !empty {
            let start_byte = char_to_byte(&display, sel_start.min(display.chars().count()));
            let end_byte = char_to_byte(&display, sel_end.min(display.chars().count()));
            let x0 = ctx.measure_text(&display[..start_byte], &style, None).width;
            let x1 = ctx.measure_text(&display[..end_byte], &style, None).width;
            let selection = Rect::new(
                text_rect.x + x0 - self.scroll_x,
                text_rect.y,
                (x1 - x0).max(1.0),
                line_height,
            );
            ctx.painter().rect(selection, theme.colors.selection);
        }

        // Text (or placeholder).
        let position = Point::new(text_rect.x - self.scroll_x, text_rect.y);
        if empty && !self.placeholder.is_empty() {
            let mut placeholder_style = style.clone();
            placeholder_style.color = theme.colors.text_muted;
            if !state.enabled {
                placeholder_style.color = theme.colors.text_disabled;
            }
            ctx.painter()
                .text(position, &self.placeholder, &placeholder_style);
        } else {
            ctx.painter().text(position, &display, &style);
            // IME composition gets an underline.
            if let Some(composing) = &self.composing {
                if !composing.is_empty() && !self.password {
                    let composed_bytes = self.text.get().len();
                    let prefix = &display[..display.len().min(composed_bytes)];
                    let x0 = ctx.measure_text(prefix, &style, None).width;
                    let end = (composed_bytes + composing.len()).min(display.len());
                    let x1 = ctx.measure_text(&display[..end], &style, None).width;
                    let underline = Rect::new(
                        text_rect.x + x0 - self.scroll_x,
                        text_rect.y + line_height - theme.borders.thin,
                        (x1 - x0).max(1.0),
                        theme.borders.thin.max(1.0),
                    );
                    ctx.painter().rect(underline, theme.colors.accent);
                }
            }
        }

        // Caret.
        if self.focused && self.blink_on && state.enabled {
            let caret = self.caret_rect(ctx.ui(), bounds);
            ctx.painter().rect(caret, theme.colors.text);
        }

        ctx.pop_clip();

        if state.focused {
            focus_ring(ctx, self.base.bounds(), theme.radius.sm);
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Byte offset of character `index` inside `text`.
fn char_to_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

/// Start of the word before the character at `index`.
fn word_left(text: &str, index: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = index.min(chars.len());
    while at > 0 && !is_word(chars[at - 1]) {
        at -= 1;
    }
    while at > 0 && is_word(chars[at - 1]) {
        at -= 1;
    }
    at
}

/// End of the word after the character at `index`.
fn word_right(text: &str, index: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = index.min(chars.len());
    while at < chars.len() && !is_word(chars[at]) {
        at += 1;
    }
    while at < chars.len() && is_word(chars[at]) {
        at += 1;
    }
    at
}
