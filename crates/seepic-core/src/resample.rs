//! SIMD resampling for BGRA8 and RGB8 buffers.
//!
//! `image`'s own `imageops::resize` is scalar: its convolution loop walks one
//! output pixel at a time and never uses more than one lane. Reducing a
//! 12-megapixel photo is one of the most expensive things the viewer does, so
//! the same filters are run through `fast_image_resize`, which dispatches to
//! SSE4/AVX2 kernels.
//!
//! Everything that resamples pixels goes through here, so the kernel choice and
//! the alpha handling stay in one place.

use std::cell::RefCell;

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

pub use fast_image_resize::FilterType as Kernel;

use crate::filters::Interpolation;

/// The resampling kernel the user's interpolation setting asks for.
pub fn kernel(interpolation: Interpolation) -> FilterType {
    match interpolation {
        Interpolation::Bilinear => FilterType::Bilinear,
        Interpolation::Bicubic => FilterType::CatmullRom,
        Interpolation::Lanczos => FilterType::Lanczos3,
    }
}

thread_local! {
    /// One resizer per thread.
    ///
    /// A `Resizer` owns scratch buffers for the alpha premultiply pass, and a
    /// pyramid build resizes half a dozen times in a row; rebuilding it for each
    /// level would allocate a full-size buffer every time.
    static RESIZER: RefCell<Resizer> = RefCell::new(Resizer::new());
}

/// How the resampler treats the fourth channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alpha {
    /// The fourth channel is opacity. Colour is premultiplied before filtering
    /// and divided out afterwards, so fully transparent pixels cannot bleed
    /// their colour into the visible edge.
    Premultiplied,
    /// Three channels of colour and no opacity, or an image whose fourth
    /// channel is not opacity at all.
    Ignored,
}

/// Resample a buffer of `channels`-byte pixels.
///
/// Takes the source by value: the resampler stores it in place, and every
/// caller here is finished with its buffer, so passing a reference would only
/// force a copy of the whole image. A failure hands the buffer back, so the
/// caller can fall back to using it unchanged rather than having to copy it
/// first.
pub fn resize(
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    out_width: u32,
    out_height: u32,
    channels: usize,
    filter: FilterType,
    alpha: Alpha,
) -> Result<Vec<u8>, Vec<u8>> {
    if width == 0 || height == 0 || out_width == 0 || out_height == 0 {
        return Err(bytes);
    }
    let pixel_type = match channels {
        1 => PixelType::U8,
        3 => PixelType::U8x3,
        4 => PixelType::U8x4,
        _ => return Err(bytes),
    };
    // Checked here rather than left to the resampler: its error type does not
    // carry the buffer back, and the caller needs its pixels either way.
    if bytes.len() < (width as usize) * (height as usize) * channels {
        return Err(bytes);
    }

    // Unreachable: `from_vec_u8` fails only on a short buffer, and the pixel
    // type is one of the three it supports.
    let Ok(source) = Image::from_vec_u8(width, height, bytes, pixel_type) else {
        return Err(Vec::new());
    };
    let mut target = Image::new(out_width, out_height, pixel_type);
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(filter))
        .use_alpha(alpha == Alpha::Premultiplied);

    RESIZER.with(|resizer| {
        match resizer
            .borrow_mut()
            .resize(&source, &mut target, &options)
        {
            Ok(()) => Ok(target.into_vec()),
            Err(_) => Err(source.into_vec()),
        }
    })
}

/// Resample a BGRA8 buffer, treating the fourth channel as opacity.
pub fn resize_bgra(
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    out_width: u32,
    out_height: u32,
    filter: FilterType,
) -> Result<Vec<u8>, Vec<u8>> {
    resize(
        width,
        height,
        bytes,
        out_width,
        out_height,
        4,
        filter,
        Alpha::Premultiplied,
    )
}

/// Resample straight from a borrowed buffer.
///
/// The resampler accepts a borrowed source, so a caller that keeps using its
/// buffer afterwards — the mip pyramid, which resamples the level it just
/// produced — does not have to hand over (or copy) the pixels. Returns `None`
/// rather than the source, because there is nothing to hand back.
pub fn resize_ref(
    width: u32,
    height: u32,
    bytes: &[u8],
    out_width: u32,
    out_height: u32,
    channels: usize,
    filter: FilterType,
    alpha: Alpha,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || out_width == 0 || out_height == 0 {
        return None;
    }
    let pixel_type = match channels {
        1 => PixelType::U8,
        3 => PixelType::U8x3,
        4 => PixelType::U8x4,
        _ => return None,
    };
    if bytes.len() < (width as usize) * (height as usize) * channels {
        return None;
    }

    let source = ImageRef::new(width, height, bytes, pixel_type).ok()?;
    let mut target = Image::new(out_width, out_height, pixel_type);
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(filter))
        .use_alpha(alpha == Alpha::Premultiplied);

    RESIZER
        .with(|resizer| resizer.borrow_mut().resize(&source, &mut target, &options))
        .ok()?;
    Some(target.into_vec())
}

/// Resample a borrowed BGRA8 buffer, treating the fourth channel as opacity.
pub fn resize_bgra_ref(
    width: u32,
    height: u32,
    bytes: &[u8],
    out_width: u32,
    out_height: u32,
    filter: FilterType,
) -> Option<Vec<u8>> {
    resize_ref(
        width,
        height,
        bytes,
        out_width,
        out_height,
        4,
        filter,
        Alpha::Premultiplied,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BGRA buffer whose pixels are all the same colour.
    fn flat(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        colour.repeat((width * height) as usize)
    }

    #[test]
    fn resizing_keeps_a_flat_colour() {
        let out = resize_bgra(
            64,
            32,
            flat(64, 32, [10, 20, 30, 255]),
            16,
            8,
            FilterType::Bilinear,
        )
        .unwrap();
        assert_eq!(out.len(), 16 * 8 * 4);
        for pixel in out.chunks_exact(4) {
            assert_eq!(pixel, [10, 20, 30, 255]);
        }
    }

    #[test]
    fn three_channel_buffers_resize_too() {
        let out = resize(
            32,
            32,
            vec![7u8; 32 * 32 * 3],
            8,
            8,
            3,
            FilterType::CatmullRom,
            Alpha::Ignored,
        )
        .unwrap();
        assert_eq!(out.len(), 8 * 8 * 3);
        assert!(out.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn single_channel_buffers_resize_too() {
        let out = resize(
            32,
            32,
            vec![9u8; 32 * 32],
            8,
            8,
            1,
            FilterType::Bilinear,
            Alpha::Ignored,
        )
        .unwrap();
        assert_eq!(out.len(), 8 * 8);
        assert!(out.iter().all(|byte| *byte == 9));
    }

    #[test]
    fn a_failed_resize_hands_the_buffer_back() {
        let bytes = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let returned = resize_bgra(10, 10, bytes.clone(), 4, 4, FilterType::Bilinear);
        assert_eq!(returned, Err(bytes), "the caller must get its pixels back");
    }

    #[test]
    fn a_degenerate_size_is_refused() {
        assert!(resize_bgra(0, 10, Vec::new(), 4, 4, FilterType::Bilinear).is_err());
        assert!(resize_bgra(10, 10, flat(10, 10, [0, 0, 0, 255]), 0, 4, FilterType::Bilinear).is_err());
        assert!(resize_bgra(10, 10, vec![0; 4], 4, 4, FilterType::Bilinear).is_err());
    }

    #[test]
    fn a_short_buffer_is_refused() {
        assert!(resize_bgra(10, 10, vec![0; 16], 4, 4, FilterType::Bilinear).is_err());
    }

    #[test]
    fn the_interpolation_setting_picks_the_kernel() {
        assert_eq!(kernel(Interpolation::Bilinear), FilterType::Bilinear);
        assert_eq!(kernel(Interpolation::Bicubic), FilterType::CatmullRom);
        assert_eq!(kernel(Interpolation::Lanczos), FilterType::Lanczos3);
    }

    /// The borrowed path must produce exactly what the owning one does, or the
    /// mip pyramid would differ from every other resize in the app.
    #[test]
    fn borrowing_the_source_matches_taking_it() {
        // A gradient, so the filter has something to actually interpolate.
        let source: Vec<u8> = (0..(64 * 48 * 4))
            .map(|index| (index % 251) as u8)
            .collect();

        for (channels, alpha) in [(4usize, Alpha::Premultiplied), (3, Alpha::Ignored), (1, Alpha::Ignored)] {
            let owned = resize(
                64,
                48,
                source[..(64 * 48 * channels)].to_vec(),
                17,
                13,
                channels,
                FilterType::CatmullRom,
                alpha,
            )
            .unwrap();
            let borrowed = resize_ref(
                64,
                48,
                &source[..(64 * 48 * channels)],
                17,
                13,
                channels,
                FilterType::CatmullRom,
                alpha,
            )
            .unwrap();
            assert_eq!(owned, borrowed, "channels={channels}");
        }
    }

    #[test]
    fn a_borrowed_resize_refuses_a_short_buffer() {
        assert!(resize_bgra_ref(10, 10, &[0u8; 16], 4, 4, FilterType::Bilinear).is_none());
        assert!(resize_bgra_ref(0, 10, &[], 4, 4, FilterType::Bilinear).is_none());
    }
}
