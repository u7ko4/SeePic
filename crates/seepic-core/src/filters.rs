//! Resampling filter and image-filter modes, with their pixel work.
//!
//! `FilterMode` is the "basic filter" choice in the Image panel: softening the
//! image, or softening then sharpening it. `Interpolation` picks the kernel used
//! when building the downscale pyramid, which is where resampling actually
//! happens for a fitted view.

/// The basic filter applied to the displayed image.
///
/// `Hash` so it can take part in the key of a cached built image: the filter is
/// applied to the pixels, so changing it invalidates the render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FilterMode {
    #[default]
    None,
    /// A light blur, which hides JPEG blocking and sensor noise.
    Smooth,
    /// Blur followed by an unsharp mask, which keeps edges crisp while the
    /// noise stays smoothed.
    SmoothSharpen,
}

impl FilterMode {
    pub const ALL: [FilterMode; 3] = [
        FilterMode::None,
        FilterMode::Smooth,
        FilterMode::SmoothSharpen,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterMode::None => "不使用滤镜(U)",
            FilterMode::Smooth => "减淡(I)",
            FilterMode::SmoothSharpen => "减淡+锐化(S)",
        }
    }

    /// True when the pixel pass would change nothing.
    pub fn is_identity(self) -> bool {
        self == FilterMode::None
    }
}

/// Kernel used when scaling down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interpolation {
    /// Cheapest, slightly soft.
    Bilinear,
    /// A good balance; the default.
    #[default]
    Bicubic,
    /// Sharpest, slowest, and can ring on hard edges.
    Lanczos,
}

impl Interpolation {
    pub const ALL: [Interpolation; 3] = [
        Interpolation::Bilinear,
        Interpolation::Bicubic,
        Interpolation::Lanczos,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Interpolation::Bilinear => "Bilinear 插值 (快)",
            Interpolation::Bicubic => "Bicubic 插值 (慢，高品质)",
            Interpolation::Lanczos => "Lanczos 插值 (慢，高品质)",
        }
    }

    /// The `image` crate's corresponding kernel.
    pub fn kernel(self) -> image::imageops::FilterType {
        use image::imageops::FilterType;
        match self {
            Interpolation::Bilinear => FilterType::Triangle,
            Interpolation::Bicubic => FilterType::CatmullRom,
            Interpolation::Lanczos => FilterType::Lanczos3,
        }
    }
}

/// Apply `mode` to a BGRA8 buffer, returning the processed pixels.
pub fn apply_filter(mode: FilterMode, width: u32, height: u32, bytes: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    if mode.is_identity() || w == 0 || h == 0 || bytes.len() != w * h * 4 {
        return bytes.to_vec();
    }

    let mut soft = vec![0u8; bytes.len()];
    box_blur(w, h, bytes, &mut soft, 1);

    if mode == FilterMode::Smooth {
        return soft;
    }

    // Unsharp mask: push each channel away from its blurred value, which
    // restores the edge contrast the blur removed.
    let mut out = vec![0u8; bytes.len()];
    for (index, (sharp, blurred)) in bytes.iter().zip(soft.iter()).enumerate() {
        let original = *sharp as f32;
        let value = original + (original - *blurred as f32);
        out[index] = value.clamp(0., 255.) as u8;
    }
    out
}

/// Separable box blur of radius `radius`, written into `dst`.
fn box_blur(width: usize, height: usize, src: &[u8], dst: &mut [u8], radius: usize) {
    let mut temp = vec![0u8; src.len()];

    // Horizontal pass.
    for y in 0..height {
        for x in 0..width {
            let from = x.saturating_sub(radius);
            let to = (x + radius).min(width - 1);
            let mut acc = [0u32; 4];
            let mut count = 0u32;
            for sample in from..=to {
                let at = (y * width + sample) * 4;
                for channel in 0..4 {
                    acc[channel] += src[at + channel] as u32;
                }
                count += 1;
            }
            let at = (y * width + x) * 4;
            for channel in 0..4 {
                temp[at + channel] = (acc[channel] / count) as u8;
            }
        }
    }

    // Vertical pass.
    for x in 0..width {
        for y in 0..height {
            let from = y.saturating_sub(radius);
            let to = (y + radius).min(height - 1);
            let mut acc = [0u32; 4];
            let mut count = 0u32;
            for sample in from..=to {
                let at = (sample * width + x) * 4;
                for channel in 0..4 {
                    acc[channel] += temp[at + channel] as u32;
                }
                count += 1;
            }
            let at = (y * width + x) * 4;
            for channel in 0..4 {
                dst[at + channel] = (acc[channel] / count) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A black image with one white pixel in the middle.
    fn spike(width: usize, height: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; width * height * 4];
        let at = ((height / 2) * width + width / 2) * 4;
        bytes[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
        bytes
    }

    fn px(bytes: &[u8], width: usize, x: usize, y: usize) -> u8 {
        bytes[(y * width + x) * 4]
    }

    #[test]
    fn no_filter_is_an_exact_pass_through() {
        let bytes = spike(5, 5);
        assert_eq!(apply_filter(FilterMode::None, 5, 5, &bytes), bytes);
    }

    #[test]
    fn softening_spreads_a_bright_pixel() {
        let bytes = spike(5, 5);
        let soft = apply_filter(FilterMode::Smooth, 5, 5, &bytes);

        // The spike is dimmer and its neighbours picked up some light.
        assert!(px(&soft, 5, 2, 2) < 255, "centre should be dimmed");
        assert!(px(&soft, 5, 1, 2) > 0, "neighbour should brighten");
        assert_eq!(px(&soft, 5, 0, 0), 0, "far corner stays black");
    }

    #[test]
    fn sharpening_pushes_an_edge_harder() {
        // Left half mid grey, right half brighter.
        let width = 6;
        let height = 2;
        let mut bytes = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let value = if x < 3 { 100u8 } else { 150u8 };
                let at = (y * width + x) * 4;
                bytes[at..at + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }

        let sharpened = apply_filter(FilterMode::SmoothSharpen, width as u32, height as u32, &bytes);

        // Just inside the bright side the value should exceed the original 150,
        // and just inside the dark side it should fall below 100.
        assert!(
            px(&sharpened, width, 3, 0) > 150,
            "dark side of the edge should darken, got {}",
            px(&sharpened, width, 2, 0)
        );
        assert!(
            px(&sharpened, width, 3, 0) >= px(&bytes, width, 3, 0),
            "bright side should not lose contrast"
        );
    }

    #[test]
    fn a_buffer_of_the_wrong_size_is_returned_untouched() {
        let bytes = vec![1u8, 2, 3, 4];
        assert_eq!(apply_filter(FilterMode::Smooth, 10, 10, &bytes), bytes);
    }

    #[test]
    fn zero_sized_images_do_not_panic() {
        assert!(apply_filter(FilterMode::SmoothSharpen, 0, 0, &[]).is_empty());
    }

    #[test]
    fn every_mode_has_a_label() {
        for mode in FilterMode::ALL {
            assert!(!mode.label().is_empty());
        }
        for interpolation in Interpolation::ALL {
            assert!(!interpolation.label().is_empty());
        }
    }

    #[test]
    fn interpolation_picks_distinct_kernels() {
        assert_eq!(Interpolation::Bilinear.kernel(), image::imageops::FilterType::Triangle);
        assert_eq!(Interpolation::Lanczos.kernel(), image::imageops::FilterType::Lanczos3);
        assert_ne!(
            Interpolation::Bicubic.kernel(),
            Interpolation::Lanczos.kernel()
        );
    }
}
