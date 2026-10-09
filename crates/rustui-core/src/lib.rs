//! # rustui-core
//!
//! Foundation of the RustUI GUI framework: object model, geometry, colors, properties, signals,
//! events, the widget contract and the contexts used by widgets.
//!
//! This crate has **no** dependency on `winit` or `wgpu` (design principles P1/P2) and only
//! depends on `std` + `log`, which keeps the object model portable and cheap to test.
//!
//! ## Object model
//!
//! ```text
//! ObjectRegistry  -> identity, parent/child links
//! WidgetStore     -> widget payloads (Box<dyn Widget>)
//! ```
//!
//! Widgets address each other through [`ObjectId`](object::ObjectId) values; they never hold
//! `Rc<RefCell<..>>` pointers to each other.

pub mod color;
pub mod context;
pub mod error;
pub mod event;
pub mod focus;
pub mod geometry;
pub mod keyboard;
pub mod notify;
pub mod object;
pub mod paint;
pub mod property;
pub mod signal;
pub mod style;
pub mod text;
pub mod theme;
pub mod widget;
pub mod widget_store;

pub use color::Color;
pub use context::{
    ArrangeContext, Clipboard, EventContext, EventQueue, FrameClock, Interaction, MeasureContext,
    MemoryClipboard, OverlayList, OverlayRequest, PaintContext, UiContext, UpdateContext,
};
pub use error::{Result, UiError};
pub use event::{EventPhase, EventResult, QueuedEvent, TaskId, UiEvent};
pub use focus::FocusManager;
pub use geometry::{
    Align, AlignSelf, CrossAxisAlign, EdgeRadius, EdgeWidths, Insets, MainAxisAlign, MainAxisSize,
    Point, Rect, Size, Vec2,
};
pub use keyboard::{CursorIcon, FocusReason, Key, Modifiers, MouseButton, NamedKey, ScrollPhase};
pub use object::{ObjectError, ObjectId, ObjectRegistry};
pub use paint::{
    FontId, Gradient, PaintCommand, Painter, TextAlign, TextArena, TextRunId, TextStyle, TextureId,
    Transform,
};
pub use property::{Property, Shared, SharedProperty};
pub use signal::{Connection, Signal};
pub use style::{Border, Fill, Style, WidgetState};
pub use text::{AtlasInfo, GlyphSource, PositionedGlyph, TextLayout, TextLine, TextMeasurer};
pub use theme::{Radius, Spacing, StateColors, TextRole, Theme, ThemeColors, Typography};
pub use widget::{clamp_scroll, scroll_delta_to_offset, RootWidget, SizeHint, Widget, WidgetBase};
pub use widget_store::WidgetStore;

/// Everything an application typically needs.
pub mod prelude {
    pub use crate::color::Color;
    pub use crate::context::{
        ArrangeContext, Clipboard, EventContext, FrameClock, MeasureContext, PaintContext,
        UiContext, UpdateContext,
    };
    pub use crate::error::{Result, UiError};
    pub use crate::event::{EventResult, UiEvent};
    pub use crate::geometry::{Align, EdgeRadius, Insets, Point, Rect, Size, Vec2};
    pub use crate::keyboard::{CursorIcon, FocusReason, Key, Modifiers, MouseButton};
    pub use crate::object::{ObjectId, ObjectRegistry};
    pub use crate::paint::{FontId, Painter, TextAlign, TextStyle, TextureId};
    pub use crate::property::{Property, Shared, SharedProperty};
    pub use crate::style::{Border, WidgetState};
    pub use crate::text::TextMeasurer;
    pub use crate::theme::TextRole;
    pub use crate::theme::{StateColors, Theme, ThemeColors};
    pub use crate::widget::{RootWidget, SizeHint, Widget, WidgetBase};
    pub use crate::widget_store::WidgetStore;
    pub use crate::{impl_widget_base, Layout};
}

/// Marker trait implemented by layout primitives of `rustui-layout`.
///
/// It lives here so that generic code can be written against "things that lay out children"
/// without depending on `rustui-layout` itself.
pub trait Layout: Widget {}
