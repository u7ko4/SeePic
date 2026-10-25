//! Per-pixel colour adjustments from the Image panel.
//!
//! Everything works on BGRA8, the layout GPUI paints from, and is applied in a
//! fixed order so that combining toggles is predictable: tone first, then
//! colour, then the destructive ones (grayscale/sepia/negative), and finally
//! channel isolation, which can only make sense last.

/// Which colour channels to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum Channel {
    #[default]
    All,
    Red,
    Green,
    Blue,
    Alpha,
}

impl Channel {
    pub const ALL: [Channel; 5] = [
        Channel::All,
        Channel::Red,
        Channel::Green,
        Channel::Blue,
        Channel::Alpha,
    ];

    /// Single-letter label, as shown in the channel row.
    pub fn label(self) -> &'static str {
        match self {
            Channel::All => "全部",
            Channel::Red => "R",
            Channel::Green => "G",
            Channel::Blue => "B",
            Channel::Alpha => "A",
        }
    }

    /// The next channel in the cycle, for the "switch channel" command.
    pub fn next(self) -> Channel {
        let index = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

/// The state of the Image panel's effect controls.
///
/// `Hash` so it can take part in the key of a cached built image: the effects
/// are applied to the pixels, so a change to them invalidates the render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Adjustments {
    /// Percentage, `0` is neutral.
    pub brightness: i32,
    /// Percentage, `0` is neutral.
    pub contrast: i32,
    /// Percentage, `0` is neutral.
    pub saturation: i32,
    /// Percent, `100` is neutral.
    pub gamma: i32,
    pub negative: bool,
    pub grayscale: bool,
    pub sepia: bool,
    pub channel: Channel,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: 100,
            negative: false,
            grayscale: false,
            sepia: false,
            channel: Channel::All,
        }
    }
}

impl Adjustments {
    /// Limits, kept in one place so the panel and the clamps agree.
    pub const BRIGHTNESS_RANGE: (i32, i32) = (-100, 100);
    pub const CONTRAST_RANGE: (i32, i32) = (-100, 100);
    pub const SATURATION_RANGE: (i32, i32) = (-100, 100);
    pub const GAMMA_RANGE: (i32, i32) = (10, 300);

    /// True when the pixel pass would change nothing, so callers can skip it.
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Apply `adjustments` to a BGRA8 buffer in place.
pub fn apply_bgra(bytes: &mut [u8], adjustments: &Adjustments) {
    if adjustments.is_neutral() {
        return;
    }

    // Precompute the tone curve so the per-pixel loop stays cheap.
    let brightness = adjustments.brightness as f32 * 2.55;
    // A contrast of +100 doubles the slope; -100 flattens to mid grey.
    let contrast = 1. + adjustments.contrast as f32 / 100.;
    let saturation = 1. + adjustments.saturation as f32 / 100.;
    let gamma = (adjustments.gamma as f32 / 100.).max(0.01);

    for pixel in bytes.chunks_exact_mut(4) {
        // Read as RGB; the buffer is BGRA.
        let (mut r, mut g, mut b) = (pixel[2] as f32, pixel[1] as f32, pixel[0] as f32);

        r = (r - 127.5) * contrast + 127.5 + brightness;
        g = (g - 127.5) * contrast + 127.5 + brightness;
        b = (b - 127.5) * contrast + 127.5 + brightness;

        // Saturation pulls towards or away from the pixel's luminance.
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = luma + (r - luma) * saturation;
        g = luma + (g - luma) * saturation;
        b = luma + (b - luma) * saturation;

        // Gamma: values above 100 brighten midtones.
        let inverse = 1. / gamma;
        r = 255. * (r.max(0.) / 255.).powf(inverse);
        g = 255. * (g.max(0.) / 255.).powf(inverse);
        b = 255. * (b.max(0.) / 255.).powf(inverse);

        if adjustments.grayscale {
            let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            r = y;
            g = y;
            b = y;
        } else if adjustments.sepia {
            let (sr, sg, sb) = (
                0.393 * r + 0.769 * g + 0.189 * b,
                0.349 * r + 0.686 * g + 0.168 * b,
                0.272 * r + 0.534 * g + 0.131 * b,
            );
            r = sr;
            g = sg;
            b = sb;
        }

        if adjustments.negative {
            r = 255. - r;
            g = 255. - g;
            b = 255. - b;
        }

        let (mut r, mut g, mut b) = (clamp(r), clamp(g), clamp(b));
        let mut a = pixel[3];

        // Channel isolation zeroes the others, so the kept channel shows alone.
        match adjustments.channel {
            Channel::All => {}
            Channel::Red => {
                g = 0;
                b = 0;
            }
            Channel::Green => {
                r = 0;
                b = 0;
            }
            Channel::Blue => {
                r = 0;
                g = 0;
            }
            Channel::Alpha => {
                a = a.max(1);
            }
        }

        pixel[0] = b;
        pixel[1] = g;
        pixel[2] = r;
        pixel[3] = a;
    }
}

fn clamp(value: f32) -> u8 {
    value.clamp(0., 255.) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        // Two BGRA pixels: a mid grey and a red.
        vec![128, 128, 128, 255, 0, 0, 255, 255]
    }

    fn px(bytes: &[u8], index: usize) -> [u8; 4] {
        let at = index * 4;
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    }

    #[test]
    fn neutral_changes_nothing() {
        let mut bytes = sample();
        let original = bytes.clone();
        apply_bgra(&mut bytes, &Adjustments::default());
        assert_eq!(bytes, original);
    }

    #[test]
    fn negative_inverts_each_channel() {
        let mut bytes = sample();
        apply_bgra(
            &mut bytes,
            &Adjustments {
                negative: true,
                ..Default::default()
            },
        );
        // Grey 128 -> 127; red (255,0,0) -> (0,255,255) in RGB.
        assert_eq!(px(&bytes, 0), [127, 127, 127, 255]);
        assert_eq!(px(&bytes, 1), [255, 255, 0, 255], "BGRA of cyan");
    }

    #[test]
    fn grayscale_equalises_the_channels() {
        let mut bytes = sample();
        apply_bgra(
            &mut bytes,
            &Adjustments {
                grayscale: true,
                ..Default::default()
            },
        );
        let [b, g, r, _] = px(&bytes, 1);
        assert_eq!(r, g);
        assert_eq!(g, b);
    }

    #[test]
    fn sepia_warms_the_image() {
        let mut bytes = vec![128, 128, 128, 255]; // mid grey
        apply_bgra(
            &mut bytes,
            &Adjustments {
                sepia: true,
                ..Default::default()
            },
        );
        let [b, g, r, _] = px(&bytes, 0);
        assert!(r > g && g > b, "sepia should be red > green > blue");
    }

    #[test]
    fn brightness_moves_towards_white_and_black() {
        let mut up = vec![100, 100, 100, 255];
        apply_bgra(
            &mut up,
            &Adjustments {
                brightness: 20,
                ..Default::default()
            },
        );
        assert!(px(&up, 0)[0] > 100);

        let mut down = vec![100, 100, 100, 255];
        apply_bgra(
            &mut down,
            &Adjustments {
                brightness: -20,
                ..Default::default()
            },
        );
        assert!(px(&down, 0)[0] < 100);
    }

    #[test]
    fn contrast_pushes_away_from_mid_grey() {
        let mut bytes = vec![160, 160, 160, 255];
        apply_bgra(
            &mut bytes,
            &Adjustments {
                contrast: 50,
                ..Default::default()
            },
        );
        assert!(px(&bytes, 0)[0] > 160, "above mid grey should get lighter");

        let mut bytes = vec![90, 90, 90, 255];
        apply_bgra(
            &mut bytes,
            &Adjustments {
                contrast: 50,
                ..Default::default()
            },
        );
        assert!(px(&bytes, 0)[0] < 90, "below mid grey should get darker");
    }

    #[test]
    fn saturation_zero_produces_grey() {
        let mut bytes = sample();
        apply_bgra(
            &mut bytes,
            &Adjustments {
                saturation: -100,
                ..Default::default()
            },
        );
        let [b, g, r, _] = px(&bytes, 1);
        assert_eq!(r, g);
        assert_eq!(g, b);
    }

    #[test]
    fn gamma_above_neutral_brightens() {
        let mut bytes = vec![64, 64, 64, 255];
        apply_bgra(
            &mut bytes,
            &Adjustments {
                gamma: 200,
                ..Default::default()
            },
        );
        assert!(px(&bytes, 0)[0] > 64);
    }

    #[test]
    fn channel_isolation_keeps_only_that_channel() {
        let mut bytes = vec![10, 20, 30, 255]; // B=10, G=20, R=30
        apply_bgra(
            &mut bytes,
            &Adjustments {
                channel: Channel::Red,
                ..Default::default()
            },
        );
        assert_eq!(px(&bytes, 0), [0, 0, 30, 255]);

        let mut bytes = vec![10, 20, 30, 255];
        apply_bgra(
            &mut bytes,
            &Adjustments {
                channel: Channel::Green,
                ..Default::default()
            },
        );
        assert_eq!(px(&bytes, 0), [0, 20, 0, 255]);
    }

    #[test]
    fn alpha_is_never_modified() {
        let mut bytes = vec![10, 20, 30, 77];
        apply_bgra(
            &mut bytes,
            &Adjustments {
                negative: true,
                brightness: 40,
                contrast: 30,
                ..Default::default()
            },
        );
        assert_eq!(px(&bytes, 0)[3], 77);
    }

    #[test]
    fn extreme_values_stay_in_range() {
        // Guards against overflow panics on the clamped cast.
        let mut bytes = vec![0, 0, 0, 255, 255, 255, 255, 255];
        apply_bgra(
            &mut bytes,
            &Adjustments {
                brightness: 100,
                contrast: 100,
                saturation: 100,
                gamma: 300,
                ..Default::default()
            },
        );
        assert_eq!(bytes.len(), 8);
    }

    #[test]
    fn is_neutral_matches_the_default() {
        assert!(Adjustments::default().is_neutral());
        assert!(!Adjustments {
            gamma: 120,
            ..Default::default()
        }
        .is_neutral());
    }
}
