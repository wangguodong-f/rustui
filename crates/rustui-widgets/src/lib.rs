//! # rustui-widgets
//!
//! The widget library of RustUI (design document §26, §45 – §48).
//!
//! Every widget here is a plain `rustui_core::Widget`: it is stored in a
//! [`WidgetStore`](rustui_core::WidgetStore), addressed by an
//! [`ObjectId`](rustui_core::ObjectId) and never holds a reference to its parent or children, so
//! the object model stays free of `Rc<RefCell<T>>` (§52). Widgets that need to expose their state
//! to the application share it through a [`SharedProperty`] or a [`Signal`] instead of handing out
//! `&mut self`.
//!
//! ## Groups
//!
//! | Group | Widgets |
//! | --- | --- |
//! | Basic | [`Label`], [`Button`], [`CheckBox`], [`ProgressBar`] |
//! | Editing | [`TextInput`] |
//! | Containers | [`ScrollView`] |
//! | Item views | [`ListView`], [`TableView`], [`TreeView`] |
//! | Selection | [`ComboBox`] |
//! | Popups | [`Menu`], [`Dialog`] |
//!
//! ## Example
//!
//! ```ignore
//! let ok = Button::new("OK");
//! ok.clicked().connect(|_| println!("clicked"));
//! ```
//!
//! Item views are *self painting*: they draw their rows themselves instead of instantiating one
//! child widget per row, which is what makes virtualization (§51) possible — only the rows inside
//! the viewport ever reach the painter and the widget tree keeps a constant number of nodes.

mod button;
mod checkbox;
mod combo_box;
mod common;
mod dialog;
mod label;
mod list_view;
mod menu;
mod progress_bar;
mod scroll_view;
mod table_view;
mod text_input;
mod tree_view;
mod view;

pub use button::{Button, ButtonVariant};
pub use checkbox::CheckBox;
pub use combo_box::ComboBox;
pub use dialog::Dialog;
pub use label::Label;
pub use list_view::{ActivateMode, ListView};
pub use menu::{Menu, MenuItem};
pub use progress_bar::ProgressBar;
pub use scroll_view::ScrollView;
pub use table_view::TableView;
pub use text_input::TextInput;
pub use tree_view::TreeView;

/// Convenient imports for application code.
pub mod prelude {
    pub use crate::{
        ActivateMode, Button, ButtonVariant, CheckBox, ComboBox, Dialog, Label, ListView, Menu,
        MenuItem, ProgressBar, ScrollView, TableView, TextInput, TreeView,
    };
    pub use rustui_core::{SharedProperty, Signal, Widget};
}
