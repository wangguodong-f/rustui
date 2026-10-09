//! # rustui-layout
//!
//! Layout primitives of RustUI (design document §15). Everything in this crate is an ordinary
//! [`rustui_core::Widget`] and only depends on `rustui-core`, so it can be used by widgets,
//! examples and tests without pulling in the platform or the renderer.
//!
//! ## Containers
//!
//! | Widget      | Behaviour                                                        |
//! |-------------|------------------------------------------------------------------|
//! | [`VBox`]    | children top to bottom, `spacing` apart, optional flex children  |
//! | [`HBox`]    | children left to right, same rules                               |
//! | [`Grid`]    | rows and columns of `Auto` / `Px` / `Fraction` tracks            |
//! | [`Stack`]   | children overlap in the same box, later children on top          |
//! | [`Flow`]    | children flow left to right and wrap at the container width      |
//! | [`Container`]| one child plus padding, background, border, radius and clip     |
//! | [`Spacer`]  | invisible flexible gap (or a fixed size gap)                     |
//! | [`Separator`]| thin rule between two groups                                    |
//!
//! ## The three passes
//!
//! ```text
//! measure(available)   -> desired size, stored in `WidgetBase::measured`
//! arrange(rect)        -> local bounds + `window_bounds` for every child
//! paint(ctx)           -> decoration first, then the children
//! ```
//!
//! * **measure** never sees a `rect`: it only receives the space the parent can offer. Each child
//!   is measured with the space that is still free on the main axis, so a scroll view at the end
//!   of a `VBox` learns how tall it may become.
//! * **arrange** never re-measures — the arrange context has no text engine behind it. Containers
//!   read their children's sizes back from the store ([`style::measured_size`]) and distribute the
//!   free space: first to children that ask for it ([`rustui_core::SizeHint::flex`] or an infinite
//!   size preference), then according to [`rustui_core::MainAxisAlign`].
//! * **paint** starts with the container decoration ([`ContainerStyle::paint_begin`]), paints the
//!   children and closes with the border ([`ContainerStyle::paint_end`]).
//!
//! ## Coordinate conventions
//!
//! `arrange` receives the widget's rect *inside its parent*; the widget stores only its **local**
//! bounds (`(0, 0, width, height)`) and arranges its children in its own space. The window
//! position lives in `window_bounds`, which is what `paint` uses to translate a child. That is
//! why `ctx.fill_rect(self.base().bounds(), color)` paints correctly at any nesting depth.

pub mod boxes;
pub mod container;
pub mod flow;
pub mod grid;
pub mod stack;
pub mod style;

pub use boxes::{BoxLayout, HBox, Separator, Spacer, VBox};
pub use container::Container;
pub use flow::Flow;
pub use grid::{Grid, Track};
pub use stack::Stack;
pub use style::{Axis, ContainerStyle};

/// Common imports for code that builds layouts.
pub mod prelude {
    pub use crate::{
        Axis, BoxLayout, Container, ContainerStyle, Flow, Grid, HBox, Separator, Spacer, Stack,
        Track, VBox,
    };
    pub use rustui_core::{Align, AlignSelf, CrossAxisAlign, MainAxisAlign, MainAxisSize};
}
