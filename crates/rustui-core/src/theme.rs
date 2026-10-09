//! Theming (§35).
//!
//! Widgets never hard-code colors: they look values up in the [`Theme`] carried by the
//! `UiContext`. Widgets honour pseudo states (`hovered`, `pressed`, `focused`, disabled) through
//! [`StateColors`], which keeps the "no fixed colors in widgets" rule of design document §35
//! while still supporting `Button:hover`-like behaviour (§36).

use crate::color::Color;
use crate::geometry::{EdgeRadius, EdgeWidths, Insets};
use crate::paint::{FontId, TextStyle};
use crate::style::{Border, WidgetState};

/// Named colors of a theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColors {
    pub window_background: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub surface_hover: Color,
    pub surface_active: Color,
    pub primary: Color,
    pub primary_hover: Color,
    pub primary_active: Color,
    pub on_primary: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_disabled: Color,
    pub border: Color,
    pub border_strong: Color,
    pub accent: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    pub selection: Color,
    pub shadow: Color,
    /// Colour of the keyboard focus ring.
    pub focus_ring: Color,
}

/// A named text role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextRole {
    Body,
    Title,
    Heading,
    Caption,
    Button,
    Mono,
}

/// Typography: font family role -> size / weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub font: FontId,
    pub mono_font: FontId,
    pub body: f32,
    pub caption: f32,
    pub title: f32,
    pub heading: f32,
    pub button: f32,
    pub line_spacing: f32,
}

/// Spacing scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub none: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

/// Corner radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radius {
    pub none: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub pill: f32,
}

/// Standard border widths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderWidths {
    pub thin: f32,
    pub normal: f32,
    pub thick: f32,
}

impl BorderWidths {
    pub fn thin(&self, color: Color) -> Border {
        Border {
            widths: EdgeWidths::all(self.thin),
            color,
        }
    }
}

/// Per state colors of an interactive surface (button, list item, ...).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateColors {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
    pub disabled: Color,
    pub border: Color,
    pub border_focused: Color,
}

impl StateColors {
    /// Picks the color matching the widget state.
    pub fn background(&self, state: WidgetState) -> Color {
        if !state.enabled {
            self.disabled
        } else if state.pressed {
            self.pressed
        } else if state.hovered {
            self.hovered
        } else {
            self.normal
        }
    }

    /// Picks the border matching the widget state.
    pub fn border(&self, state: WidgetState) -> Border {
        let color = if state.focused {
            self.border_focused
        } else {
            self.border
        };
        Border {
            widths: EdgeWidths::all(if color.a > 0.0 { 1.0 } else { 0.0 }),
            color,
        }
    }
}

/// The complete look of an application.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub colors: ThemeColors,
    pub typography: Typography,
    pub spacing: Spacing,
    pub radius: Radius,
    pub borders: BorderWidths,
    pub dark: bool,
    /// Colors of push buttons.
    pub button: StateColors,
    /// Colors of flat / secondary surfaces.
    pub surface_button: StateColors,
    /// Colors of list / table rows.
    pub item: StateColors,
    /// Text input background.
    pub input: StateColors,
}

impl Theme {
    /// The built in light theme.
    pub fn light() -> Theme {
        let colors = ThemeColors {
            window_background: Color::from_rgb32(0xf3f3f3),
            surface: Color::from_rgb32(0xffffff),
            surface_alt: Color::from_rgb32(0xe9e9ec),
            surface_hover: Color::from_rgb32(0xe4e4e8),
            surface_active: Color::from_rgb32(0xd8d8de),
            primary: Color::from_rgb32(0x2563eb),
            primary_hover: Color::from_rgb32(0x1d4ed8),
            primary_active: Color::from_rgb32(0x1e40af),
            on_primary: Color::WHITE,
            text: Color::from_rgb32(0x1f2430),
            text_muted: Color::from_rgb32(0x6b7280),
            text_disabled: Color::from_rgb32(0xa8adb8),
            border: Color::from_rgb32(0xd4d7dd),
            border_strong: Color::from_rgb32(0x9aa1ad),
            accent: Color::from_rgb32(0x7c3aed),
            danger: Color::from_rgb32(0xdc2626),
            success: Color::from_rgb32(0x16a34a),
            warning: Color::from_rgb32(0xd97706),
            selection: Color::from_rgb32(0x2563eb).with_alpha(0.18),
            shadow: Color::BLACK.with_alpha(0.18),
            focus_ring: Color::from_rgb32(0x2563eb),
        };
        Theme {
            colors,
            typography: default_typography(),
            spacing: default_spacing(),
            radius: default_radius(),
            borders: BorderWidths {
                thin: 1.0,
                normal: 2.0,
                thick: 3.0,
            },
            dark: false,
            button: StateColors {
                normal: colors.primary,
                hovered: colors.primary_hover,
                pressed: colors.primary_active,
                disabled: Color::from_rgb32(0xcdd2da),
                border: Color::TRANSPARENT,
                border_focused: colors.focus_ring,
            },
            surface_button: StateColors {
                normal: Color::from_rgb32(0xffffff),
                hovered: colors.surface_hover,
                pressed: colors.surface_active,
                disabled: Color::from_rgb32(0xf6f7f9),
                border: colors.border,
                border_focused: colors.focus_ring,
            },
            item: StateColors {
                normal: Color::TRANSPARENT,
                hovered: colors.surface_hover,
                pressed: colors.surface_active,
                disabled: colors.text_disabled,
                border: Color::TRANSPARENT,
                border_focused: colors.focus_ring,
            },
            input: StateColors {
                normal: Color::WHITE,
                hovered: Color::WHITE,
                pressed: Color::WHITE,
                disabled: Color::from_rgb32(0xf1f2f4),
                border: colors.border,
                border_focused: colors.focus_ring,
            },
        }
    }

    /// The built in dark theme.
    pub fn dark() -> Theme {
        let colors = ThemeColors {
            window_background: Color::from_rgb32(0x1b1e24),
            surface: Color::from_rgb32(0x252932),
            surface_alt: Color::from_rgb32(0x2d323c),
            surface_hover: Color::from_rgb32(0x353b47),
            surface_active: Color::from_rgb32(0x3f4653),
            primary: Color::from_rgb32(0x3b82f6),
            primary_hover: Color::from_rgb32(0x60a5fa),
            primary_active: Color::from_rgb32(0x2563eb),
            on_primary: Color::from_rgb32(0x0b1220),
            text: Color::from_rgb32(0xe6e8ee),
            text_muted: Color::from_rgb32(0x9aa3b2),
            text_disabled: Color::from_rgb32(0x5c6472),
            border: Color::from_rgb32(0x39404d),
            border_strong: Color::from_rgb32(0x4d5563),
            accent: Color::from_rgb32(0xa78bfa),
            danger: Color::from_rgb32(0xef4444),
            success: Color::from_rgb32(0x22c55e),
            warning: Color::from_rgb32(0xf59e0b),
            selection: Color::from_rgb32(0x3b82f6).with_alpha(0.28),
            shadow: Color::BLACK.with_alpha(0.45),
            focus_ring: Color::from_rgb32(0x60a5fa),
        };
        Theme {
            colors,
            typography: default_typography(),
            spacing: default_spacing(),
            radius: default_radius(),
            borders: BorderWidths {
                thin: 1.0,
                normal: 2.0,
                thick: 3.0,
            },
            dark: true,
            button: StateColors {
                normal: colors.primary,
                hovered: colors.primary_hover,
                pressed: colors.primary_active,
                disabled: Color::from_rgb32(0x3a4150),
                border: Color::TRANSPARENT,
                border_focused: colors.focus_ring,
            },
            surface_button: StateColors {
                normal: Color::from_rgb32(0x2d323c),
                hovered: colors.surface_hover,
                pressed: colors.surface_active,
                disabled: Color::from_rgb32(0x23272f),
                border: colors.border,
                border_focused: colors.focus_ring,
            },
            item: StateColors {
                normal: Color::TRANSPARENT,
                hovered: colors.surface_hover,
                pressed: colors.surface_active,
                disabled: colors.text_disabled,
                border: Color::TRANSPARENT,
                border_focused: colors.focus_ring,
            },
            input: StateColors {
                normal: Color::from_rgb32(0x1f232b),
                hovered: Color::from_rgb32(0x1f232b),
                pressed: Color::from_rgb32(0x1f232b),
                disabled: Color::from_rgb32(0x272c35),
                border: colors.border,
                border_focused: colors.focus_ring,
            },
        }
    }

    /// Padding for a content area.
    pub fn content_padding(&self) -> Insets {
        Insets::all(self.spacing.md)
    }

    /// Resolves a text role into a concrete style.
    pub fn text(&self, role: TextRole, color: Option<Color>) -> TextStyle {
        let size = match role {
            TextRole::Body => self.typography.body,
            TextRole::Caption => self.typography.caption,
            TextRole::Title => self.typography.title,
            TextRole::Heading => self.typography.heading,
            TextRole::Button => self.typography.button,
            TextRole::Mono => self.typography.body,
        };
        let font = match role {
            TextRole::Mono => self.typography.mono_font,
            _ => self.typography.font,
        };
        TextStyle {
            font,
            size,
            color: color.unwrap_or(self.colors.text),
            ..Default::default()
        }
    }

    /// The default size of a control (button height, input height, ...).
    pub fn control_height(&self) -> f32 {
        self.typography.button * 2.0 + self.spacing.md
    }

    /// Line height used by text widgets.
    pub fn line_height(&self, role: TextRole) -> f32 {
        match role {
            TextRole::Heading => self.typography.heading * 1.35,
            TextRole::Title => self.typography.title * 1.35,
            _ => self.typography.body * 1.45,
        }
    }

    /// Standard corner radius for controls.
    pub fn control_radius(&self) -> EdgeRadius {
        EdgeRadius::all(self.radius.md)
    }
}

fn default_typography() -> Typography {
    Typography {
        font: FontId::DEFAULT,
        mono_font: FontId(1),
        body: 14.0,
        caption: 12.0,
        title: 18.0,
        heading: 24.0,
        button: 14.0,
        line_spacing: 1.45,
    }
}

fn default_spacing() -> Spacing {
    Spacing {
        none: 0.0,
        xs: 2.0,
        sm: 4.0,
        md: 8.0,
        lg: 16.0,
        xl: 24.0,
    }
}

fn default_radius() -> Radius {
    Radius {
        none: 0.0,
        sm: 2.0,
        md: 4.0,
        lg: 8.0,
        pill: 999.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_differ() {
        let light = Theme::light();
        let dark = Theme::dark();
        assert!(!light.dark && dark.dark);
        assert_ne!(
            light.colors.window_background,
            dark.colors.window_background
        );
        assert_eq!(light.typography.body, dark.typography.body);
    }

    #[test]
    fn state_colors_pick_by_state() {
        let theme = Theme::light();
        let mut state = WidgetState::new();
        assert_eq!(theme.button.background(state), theme.colors.primary);
        state.hovered = true;
        assert_eq!(theme.button.background(state), theme.colors.primary_hover);
        state.pressed = true;
        assert_eq!(theme.button.background(state), theme.colors.primary_active);
        state.hovered = false;
        state.pressed = false;
        state.enabled = false;
        assert_eq!(theme.button.background(state), theme.button.disabled);
    }

    #[test]
    fn text_roles_resolve() {
        let theme = Theme::light();
        let body = theme.text(TextRole::Body, None);
        let heading = theme.text(TextRole::Heading, None);
        assert!(heading.size > body.size);
        assert_eq!(body.color, theme.colors.text);
        assert_eq!(
            theme.text(TextRole::Caption, Some(Color::RED)).color,
            Color::RED
        );
    }
}
