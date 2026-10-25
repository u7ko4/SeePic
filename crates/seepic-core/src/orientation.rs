//! User-applied orientation, on top of whatever EXIF already baked into the
//! decoded pixels.

use crate::geom::Size;

/// A quarter-turn rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    /// Whether this rotation exchanges width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Cw90 | Rotation::Cw270)
    }

    /// Turn further clockwise.
    pub fn rotate_cw(self) -> Self {
        match self {
            Rotation::None => Rotation::Cw90,
            Rotation::Cw90 => Rotation::Cw180,
            Rotation::Cw180 => Rotation::Cw270,
            Rotation::Cw270 => Rotation::None,
        }
    }

    /// Turn further counter-clockwise.
    pub fn rotate_ccw(self) -> Self {
        match self {
            Rotation::None => Rotation::Cw270,
            Rotation::Cw270 => Rotation::Cw180,
            Rotation::Cw180 => Rotation::Cw90,
            Rotation::Cw90 => Rotation::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Rotation::None => "不旋转",
            Rotation::Cw90 => "顺时针 90°",
            Rotation::Cw180 => "180°",
            Rotation::Cw270 => "逆时针 90°",
        }
    }
}

/// Rotation plus mirroring, as applied by the view commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct Orientation {
    pub rotation: Rotation,
    /// Mirror left-to-right.
    pub flip_h: bool,
    /// Mirror top-to-bottom.
    pub flip_v: bool,
}

impl Orientation {
    /// True when this does nothing, so callers can skip the pixel pass.
    pub fn is_identity(&self) -> bool {
        self.rotation == Rotation::None && !self.flip_h && !self.flip_v
    }

    /// The orientation an EXIF `Orientation` tag asks for.
    ///
    /// The tag has eight values: the identity, three rotations, two mirrors and
    /// two transposes. Each maps onto this type's "mirror, then rotate" model;
    /// `decode`'s tests check every value against the `image` crate's own
    /// implementation of the same table.
    pub fn from_exif(value: u8) -> Option<Self> {
        let orientation = match value {
            1 => Self::default(),
            2 => Self {
                flip_h: true,
                ..Self::default()
            },
            3 => Self {
                rotation: Rotation::Cw180,
                ..Self::default()
            },
            4 => Self {
                flip_v: true,
                ..Self::default()
            },
            // 5 and 7 are the two transposes: a quarter turn plus a mirror.
            5 => Self {
                rotation: Rotation::Cw270,
                flip_h: true,
                flip_v: false,
            },
            6 => Self {
                rotation: Rotation::Cw90,
                ..Self::default()
            },
            7 => Self {
                rotation: Rotation::Cw90,
                flip_h: true,
                flip_v: false,
            },
            8 => Self {
                rotation: Rotation::Cw270,
                ..Self::default()
            },
            // Anything else is not a rotation this type can express, and is
            // better left alone than guessed at.
            _ => return None,
        };
        Some(orientation)
    }

    /// The size an image takes on once this orientation is applied.
    pub fn applied_size(&self, size: Size) -> Size {
        if self.rotation.swaps_axes() {
            size.swapped()
        } else {
            size
        }
    }

    pub fn rotate_cw(&mut self) {
        self.rotation = self.rotation.rotate_cw();
    }

    pub fn rotate_ccw(&mut self) {
        self.rotation = self.rotation.rotate_ccw();
    }

    /// Jump straight to a quarter-turn count, where 0 is upright, 1 is 90°
    /// clockwise, and so on. The rotation presets map onto this.
    pub fn set_quarter_turns(&mut self, turns: u8) {
        self.rotation = match turns % 4 {
            0 => Rotation::None,
            1 => Rotation::Cw90,
            2 => Rotation::Cw180,
            _ => Rotation::Cw270,
        };
    }

    /// The current quarter-turn count.
    pub fn quarter_turns(&self) -> u8 {
        match self.rotation {
            Rotation::None => 0,
            Rotation::Cw90 => 1,
            Rotation::Cw180 => 2,
            Rotation::Cw270 => 3,
        }
    }

    pub fn flip_horizontal(&mut self) {
        self.flip_h = !self.flip_h;
    }

    pub fn flip_vertical(&mut self) {
        self.flip_v = !self.flip_v;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Where each source pixel ends up, as the index of the destination column
    /// and row for a source pixel at `(x, y)` in an image of `size`.
    ///
    /// Returning the destination rather than the source keeps the resampling
    /// pass a simple scatter, and makes the rules easy to test.
    pub fn map_point(&self, x: u32, y: u32, size: Size) -> (u32, u32) {
        let (w, h) = (size.w as u32, size.h as u32);
        let (mut sx, mut sy) = (x, y);

        if self.flip_h {
            sx = w.saturating_sub(1).saturating_sub(sx);
        }
        if self.flip_v {
            sy = h.saturating_sub(1).saturating_sub(sy);
        }

        // Rotate about the centre. Each rule is expressed against the *source*
        // extent, so a quarter turn lands inside the swapped destination.
        match self.rotation {
            Rotation::None => (sx, sy),
            // Clockwise: the left column becomes the top row.
            Rotation::Cw90 => (h.saturating_sub(1).saturating_sub(sy), sx),
            Rotation::Cw180 => (
                w.saturating_sub(1).saturating_sub(sx),
                h.saturating_sub(1).saturating_sub(sy),
            ),
            // Counter-clockwise: the top row becomes the left column.
            Rotation::Cw270 => (sy, w.saturating_sub(1).saturating_sub(sx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> Size {
        Size::new(4., 2.)
    }

    #[test]
    fn applied_size_swaps_on_quarter_turns() {
        let mut o = Orientation::default();
        assert_eq!(o.applied_size(size()), Size::new(4., 2.));
        o.rotate_cw();
        assert_eq!(o.applied_size(size()), Size::new(2., 4.));
        o.rotate_cw();
        assert_eq!(o.applied_size(size()), Size::new(4., 2.));
    }

    #[test]
    fn rotation_wraps_both_ways() {
        let mut o = Orientation::default();
        o.rotate_ccw();
        assert_eq!(o.rotation, Rotation::Cw270);
        o.rotate_cw();
        assert_eq!(o.rotation, Rotation::None);
    }

    #[test]
    fn identity_is_recognised() {
        let mut o = Orientation::default();
        assert!(o.is_identity());
        o.flip_horizontal();
        assert!(!o.is_identity());
        o.flip_horizontal();
        o.rotate_cw();
        assert!(!o.is_identity());
    }

    #[test]
    fn flips_are_involutions() {
        let mut o = Orientation::default();
        o.flip_horizontal();
        assert_eq!(o.map_point(0, 0, size()), (3, 0));
        o.flip_horizontal();
        assert_eq!(o.map_point(0, 0, size()), (0, 0));
    }

    #[test]
    fn quarter_turn_moves_the_corners_where_expected() {
        // A 4x2 image turned clockwise becomes 2x4. The top-left source pixel
        // must land at the top-right of the result.
        let o = Orientation {
            rotation: Rotation::Cw90,
            ..Default::default()
        };
        assert_eq!(o.map_point(0, 0, size()), (1, 0));
        assert_eq!(o.map_point(3, 0, size()), (1, 3));
        assert_eq!(o.map_point(0, 1, size()), (0, 0));
    }

    #[test]
    fn half_turn_is_a_double_flip() {
        let o = Orientation {
            rotation: Rotation::Cw180,
            ..Default::default()
        };
        assert_eq!(o.map_point(0, 0, size()), (3, 1));
        assert_eq!(o.map_point(3, 1, size()), (0, 0));
    }

    #[test]
    fn every_mapping_stays_inside_the_destination() {
        let combos = [
            Orientation::default(),
            Orientation { rotation: Rotation::Cw90, flip_h: true, flip_v: false },
            Orientation { rotation: Rotation::Cw180, flip_h: false, flip_v: true },
            Orientation { rotation: Rotation::Cw270, flip_h: true, flip_v: true },
        ];

        for o in combos {
            let dest = o.applied_size(size());
            let (dw, dh) = (dest.w as u32, dest.h as u32);
            let mut seen = vec![false; (dw * dh) as usize];

            for y in 0..size().h as u32 {
                for x in 0..size().w as u32 {
                    let (dx, dy) = o.map_point(x, y, size());
                    assert!(dx < dw && dy < dh, "{o:?} mapped ({x},{y}) out of {dest:?}");
                    let idx = (dy * dw + dx) as usize;
                    assert!(!seen[idx], "{o:?} mapped two pixels onto ({dx},{dy})");
                    seen[idx] = true;
                }
            }
            assert!(seen.iter().all(|s| *s), "{o:?} left a destination pixel unfilled");
        }
    }
}
