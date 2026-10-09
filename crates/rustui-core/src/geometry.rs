//! Basic geometric primitives used across the whole framework.
//!
//! RustUI works in **logical pixels** (`f32`). Conversion to physical pixels only happens in
//! the renderer (see `rustui-renderer`) using the window scale factor.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// A 2D vector, also used as a scroll delta.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const ONE: Vec2 = Vec2 { x: 1.0, y: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    #[inline]
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    #[inline]
    pub fn distance(self, other: Vec2) -> f32 {
        (self - other).length()
    }

    #[inline]
    pub fn dot(self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    #[inline]
    pub fn normalize(self) -> Vec2 {
        let len = self.length();
        if len > f32::EPSILON {
            self / len
        } else {
            Vec2::ZERO
        }
    }

    #[inline]
    pub fn max_component(self) -> f32 {
        self.x.max(self.y)
    }

    #[inline]
    pub fn min_component(self) -> f32 {
        self.x.min(self.y)
    }

    #[inline]
    pub fn lerp(self, other: Vec2, t: f32) -> Vec2 {
        self + (other - self) * t
    }

    #[inline]
    pub fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Vec2) {
        *self = *self + rhs;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Vec2) {
        *self = *self - rhs;
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, rhs: f32) -> Vec2 {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, rhs: f32) -> Vec2 {
        Vec2::new(self.x / rhs, self.y / rhs)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// A point in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn to_vec(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

impl From<Vec2> for Point {
    #[inline]
    fn from(v: Vec2) -> Self {
        Point::new(v.x, v.y)
    }
}

impl Add<Vec2> for Point {
    type Output = Point;
    #[inline]
    fn add(self, rhs: Vec2) -> Point {
        Point::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub<Vec2> for Point {
    type Output = Point;
    #[inline]
    fn sub(self, rhs: Vec2) -> Point {
        Point::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Add<Point> for Point {
    type Output = Point;
    #[inline]
    fn add(self, rhs: Point) -> Point {
        Point::new(self.x + rhs.x, self.y + rhs.y)
    }
}

/// A size in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };

    #[inline]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    #[inline]
    pub fn splat(v: f32) -> Self {
        Self::new(v, v)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    #[inline]
    pub fn width(&self) -> f32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> f32 {
        self.height
    }

    #[inline]
    pub fn to_vec(self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }

    #[inline]
    pub fn min(self, other: Size) -> Size {
        Size::new(self.width.min(other.width), self.height.min(other.height))
    }

    #[inline]
    pub fn max(self, other: Size) -> Size {
        Size::new(self.width.max(other.width), self.height.max(other.height))
    }

    /// Grows the size in both directions (negative values shrink it).
    #[inline]
    pub fn expand(self, insets: Insets) -> Size {
        Size::new(
            self.width + insets.horizontal(),
            self.height + insets.vertical(),
        )
    }

    #[inline]
    pub fn constrain(self, max: Size) -> Size {
        self.min(max)
    }
}

impl Add<Size> for Size {
    type Output = Size;
    #[inline]
    fn add(self, rhs: Size) -> Size {
        Size::new(self.width + rhs.width, self.height + rhs.height)
    }
}

impl Add<Vec2> for Size {
    type Output = Size;
    #[inline]
    fn add(self, rhs: Vec2) -> Size {
        Size::new(self.width + rhs.x, self.height + rhs.y)
    }
}

/// An axis aligned rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    #[inline]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[inline]
    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    #[inline]
    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    #[inline]
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    #[inline]
    pub fn left(&self) -> f32 {
        self.x
    }

    #[inline]
    pub fn top(&self) -> f32 {
        self.y
    }

    #[inline]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    #[inline]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    #[inline]
    pub fn center(&self) -> Point {
        Point::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    #[inline]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    #[inline]
    pub fn contains_rect(&self, other: Rect) -> bool {
        other.left() >= self.left()
            && other.top() >= self.top()
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    #[inline]
    pub fn intersects(&self, other: Rect) -> bool {
        !self.intersection(other).is_empty()
    }

    #[inline]
    pub fn intersection(&self, other: Rect) -> Rect {
        let x0 = self.left().max(other.left());
        let y0 = self.top().max(other.top());
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            Rect::ZERO
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    #[inline]
    pub fn union(&self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        let x0 = self.left().min(other.left());
        let y0 = self.top().min(other.top());
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Returns the rectangle translated by `(dx, dy)`.
    #[inline]
    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    /// Returns the rectangle translated by `delta`.
    #[inline]
    pub fn offset(&self, delta: Vec2) -> Rect {
        self.translate(delta.x, delta.y)
    }

    /// Deflates (positive values) or inflates (negative values) the rectangle.
    #[inline]
    pub fn inflate(&self, by: f32) -> Rect {
        Rect::new(
            self.x - by,
            self.y - by,
            self.width + by * 2.0,
            self.height + by * 2.0,
        )
    }

    #[inline]
    pub fn shrink(&self, insets: Insets) -> Rect {
        Rect::new(
            self.x + insets.left,
            self.y + insets.top,
            (self.width - insets.horizontal()).max(0.0),
            (self.height - insets.vertical()).max(0.0),
        )
    }

    /// Clamps `p` into the rectangle.
    #[inline]
    pub fn clamp(&self, p: Point) -> Point {
        Point::new(
            p.x.clamp(self.left(), self.right()),
            p.y.clamp(self.top(), self.bottom()),
        )
    }
}

/// Per-side spacing (padding / margin / insets).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const ZERO: Insets = Insets::all(0.0);

    #[inline]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[inline]
    pub const fn all(v: f32) -> Self {
        Self::new(v, v, v, v)
    }

    #[inline]
    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self::new(horizontal, vertical, horizontal, vertical)
    }

    #[inline]
    pub const fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    #[inline]
    pub const fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        self.left == 0.0 && self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0
    }

    #[inline]
    pub fn max(&self, other: Insets) -> Insets {
        Insets::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }
}

/// Per-side stroke width (also used for per-corner radii, see [`EdgeRadius`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct EdgeWidths {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl EdgeWidths {
    pub const ZERO: EdgeWidths = EdgeWidths {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    #[inline]
    pub const fn all(v: f32) -> Self {
        Self {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }

    #[inline]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    #[inline]
    pub fn max_side(&self) -> f32 {
        self.left.max(self.top).max(self.right).max(self.bottom)
    }
}

/// Per-corner radius.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct EdgeRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl EdgeRadius {
    pub const ZERO: EdgeRadius = EdgeRadius {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };

    #[inline]
    pub const fn all(v: f32) -> Self {
        Self {
            top_left: v,
            top_right: v,
            bottom_right: v,
            bottom_left: v,
        }
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    #[inline]
    pub fn max(&self, other: EdgeRadius) -> EdgeRadius {
        EdgeRadius {
            top_left: self.top_left.max(other.top_left),
            top_right: self.top_right.max(other.top_right),
            bottom_right: self.bottom_right.max(other.bottom_right),
            bottom_left: self.bottom_left.max(other.bottom_left),
        }
    }

    /// The largest of the four corner radii.
    ///
    /// Used where only a single value fits - the renderer rounds a quad with one radius, so mixed
    /// corners are approximated by their largest radius.
    #[inline]
    pub fn max_side(&self) -> f32 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

impl From<f32> for EdgeRadius {
    fn from(v: f32) -> Self {
        EdgeRadius::all(v)
    }
}

/// Alignment inside a rectangle (used by layouts and text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// How a child is positioned along the main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainAxisAlign {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// How children are distributed along the cross axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrossAxisAlign {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

/// Distribution of free space along the main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainAxisSize {
    /// Take exactly as much space as needed.
    #[default]
    Auto,
    /// Take all available space.
    Fill,
    /// Fixed size.
    Fixed,
}

/// Child alignment inside the cross axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignSelf {
    #[default]
    Auto,
    Start,
    Center,
    End,
    Stretch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_intersection_and_containment() {
        let a = Rect::new(0.0, 0.0, 100.0, 50.0);
        let b = Rect::new(80.0, 20.0, 40.0, 40.0);
        assert_eq!(a.intersection(b), Rect::new(80.0, 20.0, 20.0, 30.0));
        assert!(a.contains(Point::new(99.9, 49.9)));
        assert!(!a.contains(Point::new(100.0, 10.0)));
        assert!(a.intersects(b));
        assert!(!a.intersects(Rect::new(200.0, 200.0, 10.0, 10.0)));
    }

    #[test]
    fn rect_shrink_by_insets() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let inner = r.shrink(Insets::all(8.0));
        assert_eq!(inner, Rect::new(8.0, 8.0, 84.0, 84.0));
    }

    #[test]
    fn rect_union_and_clamp() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 10.0, 10.0);
        assert_eq!(a.union(b), Rect::new(0.0, 0.0, 30.0, 30.0));
        assert_eq!(a.clamp(Point::new(100.0, -5.0)), Point::new(10.0, 0.0));
    }

    #[test]
    fn vec_math() {
        let v = Vec2::new(3.0, 4.0);
        assert_eq!(v.length(), 5.0);
        assert_eq!(v.normalize(), Vec2::new(0.6, 0.8));
        assert_eq!(Vec2::ZERO.normalize(), Vec2::ZERO);
        assert_eq!(v + Vec2::ONE, Vec2::new(4.0, 5.0));
    }
}
