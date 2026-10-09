//! RGBA colors.
//!
//! Colors are stored in **sRGB** space with components in `0.0 ..= 1.0`. The renderer uploads
//! them to `*Srgb` GPU formats so the GPU performs the transfer-function conversion and blending
//! happens in linear space.

/// A color with sRGB encoded components.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Color::TRANSPARENT
    }
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const RED: Color = Color::rgb(1.0, 0.0, 0.0);
    pub const GREEN: Color = Color::rgb(0.0, 1.0, 0.0);
    pub const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);
    pub const YELLOW: Color = Color::rgb(1.0, 1.0, 0.0);
    pub const CYAN: Color = Color::rgb(0.0, 1.0, 1.0);
    pub const MAGENTA: Color = Color::rgb(1.0, 0.0, 1.0);
    /// Fully opaque gray with the given value.
    pub const fn gray(v: f32) -> Color {
        Color::rgb(v, v, v)
    }
    /// Fully opaque gray derived from a 8 bit value.
    pub const fn gray8(v: u8) -> Color {
        Color::gray(v as f32 / 255.0)
    }

    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Color {
        Color { r, g, b, a: 1.0 }
    }

    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }

    /// Builds a color from 8 bit components.
    #[inline]
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color::rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Builds a color from `0xRRGGBB`.
    #[inline]
    pub fn from_rgb32(v: u32) -> Color {
        Color::from_rgba8(
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
            0xff,
        )
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse(s: &str) -> Option<Color> {
        let hex = s.strip_prefix('#')?;
        let expand = |c: char| -> Option<u8> { c.to_digit(16).map(|v| (v * 16 + v) as u8) };
        let parse2 = |s: &str| -> Option<u8> {
            let mut it = s.chars();
            let hi = it.next()?.to_digit(16)? as u8;
            let lo = it.next()?.to_digit(16)? as u8;
            if it.next().is_some() {
                return None;
            }
            Some(hi * 16 + lo)
        };
        match hex.len() {
            3 => {
                let mut it = hex.chars();
                let r = expand(it.next()?)?;
                let g = expand(it.next()?)?;
                let b = expand(it.next()?)?;
                Some(Color::from_rgba8(r, g, b, 0xff))
            }
            6 => Some(Color::from_rgba8(
                parse2(&hex[0..2])?,
                parse2(&hex[2..4])?,
                parse2(&hex[4..6])?,
                0xff,
            )),
            8 => Some(Color::from_rgba8(
                parse2(&hex[0..2])?,
                parse2(&hex[2..4])?,
                parse2(&hex[4..6])?,
                parse2(&hex[6..8])?,
            )),
            _ => None,
        }
    }

    #[inline]
    pub fn to_rgba8(self) -> [u8; 4] {
        [
            (self.r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (self.g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (self.b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (self.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }

    #[inline]
    pub fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    #[inline]
    pub fn scale_alpha(self, factor: f32) -> Color {
        Color {
            a: (self.a * factor).clamp(0.0, 1.0),
            ..self
        }
    }

    /// Component-wise linear interpolation in sRGB space.
    #[inline]
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// Multiplies the RGB components (used for hover/pressed overlays).
    #[inline]
    pub fn modulate(self, factor: f32) -> Color {
        Color {
            r: (self.r * factor).min(1.0),
            g: (self.g * factor).min(1.0),
            b: (self.b * factor).min(1.0),
            a: self.a,
        }
    }

    /// Brightens the color towards white.
    #[inline]
    pub fn lighten(self, amount: f32) -> Color {
        self.lerp(Color::WHITE, amount)
    }

    /// Darkens the color towards black.
    #[inline]
    pub fn darken(self, amount: f32) -> Color {
        self.lerp(Color::BLACK, amount)
    }

    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    #[inline]
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_bytes() {
        let c = Color::from_rgba8(12, 34, 56, 78);
        assert_eq!(c.to_rgba8(), [12, 34, 56, 78]);
    }

    #[test]
    fn parse_hex_forms() {
        assert_eq!(Color::parse("#f00"), Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(Color::parse("#00ff00"), Some(Color::rgb(0.0, 1.0, 0.0)));
        assert_eq!(
            Color::parse("#0000ff80").map(|c| c.to_rgba8()),
            Some([0, 0, 255, 128])
        );
        assert_eq!(Color::parse("bad"), None);
    }

    #[test]
    fn lerp_and_modulate() {
        assert_eq!(Color::BLACK.lerp(Color::WHITE, 0.5), Color::gray(0.5));
        assert_eq!(Color::gray(0.5).lighten(1.0), Color::WHITE);
    }
}
