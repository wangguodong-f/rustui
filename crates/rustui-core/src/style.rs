//! Visual style primitives: fills, borders, widget state and the aggregate `Style` (§36, §45).

use crate::color::Color;
use crate::geometry::{EdgeRadius, EdgeWidths, Insets, Rect};
use crate::paint::Gradient;

/// A stroke definition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub widths: EdgeWidths,
    pub color: Color,
}

impl Default for Border {
    fn default() -> Self {
        Self {
            widths: EdgeWidths::ZERO,
            color: Color::TRANSPARENT,
        }
    }
}

impl Border {
    pub const NONE: Border = Border {
        widths: EdgeWidths::ZERO,
        color: Color::TRANSPARENT,
    };

    pub fn uniform(width: f32, color: Color) -> Self {
        Self {
            widths: EdgeWidths::all(width),
            color,
        }
    }

    pub fn left(width: f32, color: Color) -> Self {
        Self {
            widths: EdgeWidths {
                left: width,
                ..EdgeWidths::ZERO
            },
            color,
        }
    }

    pub fn top(width: f32, color: Color) -> Self {
        Self {
            widths: EdgeWidths {
                top: width,
                ..EdgeWidths::ZERO
            },
            color,
        }
    }

    pub fn is_visible(&self) -> bool {
        !self.widths.is_zero() && self.color.a > 0.0
    }
}

/// Fill description of a shape: base color, optional gradient, stroke and corner radius.
#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub color: Color,
    pub gradient: Option<Gradient>,
    pub border: Border,
    pub radius: EdgeRadius,
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            color: Color::TRANSPARENT,
            gradient: None,
            border: Border::NONE,
            radius: EdgeRadius::ZERO,
        }
    }
}

impl Fill {
    pub fn solid(color: Color) -> Self {
        Self {
            color,
            ..Default::default()
        }
    }

    pub fn rounded(color: Color, radius: f32) -> Self {
        Self {
            color,
            radius: EdgeRadius::all(radius),
            ..Default::default()
        }
    }

    pub fn with_border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }

    pub fn with_gradient(mut self, gradient: Gradient) -> Self {
        self.gradient = Some(gradient);
        self
    }

    pub fn with_radius(mut self, radius: EdgeRadius) -> Self {
        self.radius = radius;
        self
    }

    /// `true` when the fill would not draw anything.
    pub fn is_invisible(&self) -> bool {
        self.color.a <= 0.0 && self.gradient.is_none() && !self.border.is_visible()
    }
}

/// The visual state of a widget at paint time (§45).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetState {
    pub visible: bool,
    pub enabled: bool,
    pub focused: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub selected: bool,
    /// `true` for widgets that are on an active tab / expanded / open.
    pub active: bool,
}

impl WidgetState {
    /// The default state of a freshly created widget.
    pub fn new() -> Self {
        Self {
            visible: true,
            enabled: true,
            ..Default::default()
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// `true` when the widget should use its "interactive" (normal) visual style.
    pub fn is_interactive(&self) -> bool {
        self.enabled && self.visible
    }
}

/// Aggregate style of a widget (§36).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub background: Color,
    pub foreground: Color,
    pub border: Border,
    pub padding: Insets,
    pub margin: Insets,
    pub radius: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: Color::TRANSPARENT,
            foreground: Color::WHITE,
            border: Border::NONE,
            padding: Insets::ZERO,
            margin: Insets::ZERO,
            radius: 0.0,
        }
    }
}

impl Style {
    /// Applies `margin` to the given rectangle.
    pub fn margin_rect(&self, rect: Rect) -> Rect {
        rect.shrink(self.margin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_visibility() {
        assert!(Fill::default().is_invisible());
        assert!(!Fill::solid(Color::RED).is_invisible());
        let bordered = Fill::default().with_border(Border::uniform(1.0, Color::BLACK));
        assert!(!bordered.is_invisible());
    }

    #[test]
    fn widget_state_defaults() {
        let s = WidgetState::new();
        assert!(s.visible && s.enabled);
        assert!(!s.disabled().enabled);
        assert!(!s.disabled().is_interactive());
    }

    #[test]
    fn style_margin() {
        let style = Style {
            margin: Insets::all(4.0),
            ..Style::default()
        };
        assert_eq!(
            style.margin_rect(Rect::new(0.0, 0.0, 20.0, 20.0)),
            Rect::new(4.0, 4.0, 12.0, 12.0)
        );
    }
}
