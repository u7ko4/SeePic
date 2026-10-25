//! How the transparent parts of an image are drawn.
//!
//! Five treatments, cycled with `Alt+T`. The choice is a pure display decision,
//! so it lives here rather than in the canvas
//! module: the canvas only asks which colour or pattern to put behind the image,
//! and whether the alpha channel should be flattened during decode.

/// What sits behind an image that has an alpha channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Transparency {
    /// A two-tone checkerboard, the usual "this is transparent" cue.
    Grid,
    /// Flat white.
    White,
    /// Flat black.
    Black,
    /// Ignore the alpha channel entirely, drawing transparent pixels as-is.
    IgnoreAlpha,
    /// Use the configured background colour.
    #[default]
    BackgroundColor,
}

impl Transparency {
    pub const ALL: [Transparency; 5] = [
        Transparency::Grid,
        Transparency::White,
        Transparency::Black,
        Transparency::IgnoreAlpha,
        Transparency::BackgroundColor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Transparency::Grid => "网格",
            Transparency::White => "白色",
            Transparency::Black => "黑色",
            Transparency::IgnoreAlpha => "忽略 Alpha",
            Transparency::BackgroundColor => "使用背景色",
        }
    }

    /// The next treatment in the cycle, for `Alt+T`.
    pub fn next(self) -> Transparency {
        let index = Self::ALL
            .iter()
            .position(|mode| *mode == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// The flat colour to put behind the image, or `None` when the mode needs a
    /// pattern or the alpha channel preserved.
    pub fn flat_color(self) -> Option<[u8; 3]> {
        match self {
            Transparency::White => Some([0xff, 0xff, 0xff]),
            Transparency::Black => Some([0x00, 0x00, 0x00]),
            Transparency::Grid | Transparency::IgnoreAlpha | Transparency::BackgroundColor => None,
        }
    }

    /// True when the alpha channel must be forced opaque before drawing.
    pub fn flattens_alpha(self) -> bool {
        matches!(self, Transparency::IgnoreAlpha)
    }
}

/// How the canvas is filled around and behind the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BackgroundMode {
    /// A single colour the user picked.
    #[default]
    Custom,
    /// Derived from the image's own edge pixels, so it blends into the photo.
    Auto,
    /// A blurred, enlarged copy of the image.
    Blur,
}

impl BackgroundMode {
    pub const ALL: [BackgroundMode; 3] = [
        BackgroundMode::Custom,
        BackgroundMode::Auto,
        BackgroundMode::Blur,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BackgroundMode::Custom => "Custom",
            BackgroundMode::Auto => "Auto",
            BackgroundMode::Blur => "Blur",
        }
    }
}

/// The `Auto` background, from the image's average colour.
///
/// A photograph's average is usually a washed-out mid-tone, so it is darkened
/// before use: the image in front has to stay the brightest thing on screen.
pub fn auto_background(average: u32) -> u32 {
    let r = (average >> 16) & 0xff;
    let g = (average >> 8) & 0xff;
    let b = average & 0xff;
    let dim = |channel: u32| (channel * 35 / 100).min(255);
    (dim(r) << 16) | (dim(g) << 8) | dim(b)
}

/// Parse `#rrggbb` or `#rgb` into `0xRRGGBB`.
///
/// The colour settings are stored as text so the config file stays readable and
/// hand-editable; anything unparseable falls back to the caller's default rather
/// than failing the load.
pub fn parse_hex_color(text: &str) -> Option<u32> {
    let digits = text.trim().trim_start_matches('#');
    let expanded;
    let digits = match digits.len() {
        3 => {
            expanded = digits
                .chars()
                .flat_map(|c| [c, c])
                .collect::<String>();
            expanded.as_str()
        }
        6 => digits,
        _ => return None,
    };
    u32::from_str_radix(digits, 16).ok()
}

/// The size of one checkerboard square, in logical pixels.
pub const CHECKER_SIZE: usize = 8;

/// Build a two-tone checkerboard tile, as BGRA.
///
/// The canvas draws this scaled to the image's on-screen box, which keeps the
/// square size constant regardless of zoom without generating a full-size
/// pattern every frame.
pub fn checker_tile(light: [u8; 3], dark: [u8; 3]) -> (u32, u32, Vec<u8>) {
    let size = CHECKER_SIZE * 2;
    let mut bytes = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let dark_square = ((x / CHECKER_SIZE) + (y / CHECKER_SIZE)) % 2 == 1;
            let [r, g, b] = if dark_square { dark } else { light };
            bytes.extend_from_slice(&[b, g, r, 0xff]);
        }
    }
    (size as u32, size as u32, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_visits_every_treatment_and_returns() {
        let mut mode = Transparency::Grid;
        for _ in 0..Transparency::ALL.len() {
            mode = mode.next();
        }
        assert_eq!(mode, Transparency::Grid, "the cycle must close");
    }

    #[test]
    fn the_cycle_visits_each_treatment_once() {
        let mut seen = vec![Transparency::Grid];
        let mut mode = Transparency::Grid;
        for _ in 1..Transparency::ALL.len() {
            mode = mode.next();
            assert!(!seen.contains(&mode), "{mode:?} visited twice");
            seen.push(mode);
        }
        assert_eq!(seen.len(), Transparency::ALL.len());
    }

    #[test]
    fn only_ignore_alpha_flattens() {
        assert!(Transparency::IgnoreAlpha.flattens_alpha());
        for mode in Transparency::ALL {
            if mode != Transparency::IgnoreAlpha {
                assert!(!mode.flattens_alpha(), "{mode:?} should keep alpha");
            }
        }
    }

    #[test]
    fn flat_colours_are_only_offered_where_they_apply() {
        assert_eq!(Transparency::White.flat_color(), Some([255, 255, 255]));
        assert_eq!(Transparency::Black.flat_color(), Some([0, 0, 0]));
        assert_eq!(Transparency::Grid.flat_color(), None);
        assert_eq!(Transparency::BackgroundColor.flat_color(), None);
    }

    #[test]
    fn the_checker_tile_alternates() {
        let (w, h, bytes) = checker_tile([255, 255, 255], [0, 0, 0]);
        assert_eq!((w, h), (CHECKER_SIZE as u32 * 2, CHECKER_SIZE as u32 * 2));
        assert_eq!(bytes.len(), w as usize * h as usize * 4);

        // The top-left square is light and its diagonal neighbour is dark.
        let at = |x: usize, y: usize| bytes[(y * w as usize + x) * 4];
        assert_eq!(at(0, 0), 255);
        assert_eq!(at(CHECKER_SIZE, CHECKER_SIZE), 255);
        assert_eq!(at(CHECKER_SIZE, 0), 0);
        assert_eq!(at(0, CHECKER_SIZE), 0);
    }

    #[test]
    fn parses_the_colour_formats_the_settings_file_holds() {
        assert_eq!(parse_hex_color("#1a1a1a"), Some(0x1a1a1a));
        assert_eq!(parse_hex_color("1A1A1A"), Some(0x1a1a1a));
        assert_eq!(parse_hex_color("  #fff  "), Some(0xffffff));
        assert_eq!(parse_hex_color("#f0a"), Some(0xff00aa));
        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("#12345"), None);
        assert_eq!(parse_hex_color("#gggggg"), None);
    }

    #[test]
    fn the_auto_background_is_a_darker_take_on_the_image() {
        // Pure white would wash the image out, so it comes back dimmed.
        assert_eq!(auto_background(0xffffff), 0x595959);
        assert_eq!(auto_background(0x000000), 0x000000);
        // The hue is preserved.
        let dimmed = auto_background(0xff0000);
        assert_eq!(dimmed >> 16, 0x59);
        assert_eq!(dimmed & 0xffff, 0);
    }
}
