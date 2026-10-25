//! Minimal geometry types, kept independent of any UI framework's own math so
//! the layout rules can be unit-tested.

/// A width/height pair in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }

    /// True when neither dimension is usable, so nothing should be drawn.
    pub fn is_empty(&self) -> bool {
        self.w <= 0. || self.h <= 0.
    }

    /// Scale both dimensions.
    pub fn scaled(&self, factor: f32) -> Self {
        Self::new(self.w * factor, self.h * factor)
    }

    /// Swap width and height, for a quarter-turn rotation.
    pub fn swapped(&self) -> Self {
        Self::new(self.h, self.w)
    }
}

/// A position, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point::new(0., 0.);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn clamp_abs(self, max_x: f32, max_y: f32) -> Self {
        Self::new(
            self.x.clamp(-max_x, max_x),
            self.y.clamp(-max_y, max_y),
        )
    }

    /// Keep within `limit` on both axes.
    pub fn clamp_to(self, limit: Point) -> Self {
        self.clamp_abs(limit.x, limit.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_and_swapped() {
        let s = Size::new(800., 600.);
        assert_eq!(s.scaled(0.5), Size::new(400., 300.));
        assert_eq!(s.swapped(), Size::new(600., 800.));
    }

    #[test]
    fn empty_when_either_dimension_is_unusable() {
        assert!(Size::new(0., 100.).is_empty());
        assert!(Size::new(-1., 100.).is_empty());
        assert!(!Size::new(1., 1.).is_empty());
    }

    #[test]
    fn clamp_is_symmetric() {
        assert_eq!(Point::new(-50., 50.).clamp_abs(10., 10.), Point::new(-10., 10.));
        assert_eq!(Point::new(3., 4.).clamp_abs(10., 10.), Point::new(3., 4.));
    }
}
