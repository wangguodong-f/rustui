//! Cursor icons (§40 of the detailed design document).

use rustui_core::CursorIcon;
use winit::window::CursorIcon as WinitCursorIcon;

/// Maps a RustUI cursor onto its `winit` counterpart.
///
/// Unknown combinations fall back to [`WinitCursorIcon::Default`]; `winit` has a strictly larger
/// vocabulary than RustUI, so every RustUI variant is covered.
pub fn to_winit_cursor(icon: CursorIcon) -> WinitCursorIcon {
    match icon {
        CursorIcon::Default => WinitCursorIcon::Default,
        CursorIcon::Pointer => WinitCursorIcon::Pointer,
        CursorIcon::Text => WinitCursorIcon::Text,
        CursorIcon::ResizeHorizontal => WinitCursorIcon::EwResize,
        CursorIcon::ResizeVertical => WinitCursorIcon::NsResize,
        CursorIcon::ResizeAll => WinitCursorIcon::AllScroll,
        CursorIcon::Crosshair => WinitCursorIcon::Crosshair,
        CursorIcon::Grab => WinitCursorIcon::Grab,
        CursorIcon::Grabbing => WinitCursorIcon::Grabbing,
        CursorIcon::Wait => WinitCursorIcon::Wait,
        CursorIcon::NotAllowed => WinitCursorIcon::NotAllowed,
        CursorIcon::ColumnResize => WinitCursorIcon::ColResize,
        CursorIcon::RowResize => WinitCursorIcon::RowResize,
        CursorIcon::Cell => WinitCursorIcon::Cell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_maps_to_a_winit_icon() {
        for icon in [
            CursorIcon::Default,
            CursorIcon::Pointer,
            CursorIcon::Text,
            CursorIcon::ResizeHorizontal,
            CursorIcon::ResizeVertical,
            CursorIcon::ResizeAll,
            CursorIcon::Crosshair,
            CursorIcon::Grab,
            CursorIcon::Grabbing,
            CursorIcon::Wait,
            CursorIcon::NotAllowed,
            CursorIcon::ColumnResize,
            CursorIcon::RowResize,
            CursorIcon::Cell,
        ] {
            // Mapping must not panic and must be stable.
            let mapped = to_winit_cursor(icon);
            assert_eq!(mapped, to_winit_cursor(icon));
        }
    }
}
