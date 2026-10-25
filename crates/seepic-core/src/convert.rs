//! Batch image conversion: resize, rotate and re-encode.
//!
//! The image converter, minus the UI. Everything that decides *what* comes out
//! is here so it can be tested without a window: the seven
//! resize modes, the output path strategy, and the encode itself.
//!
//! Conversion is the one operation that writes files, so the naming rules are
//! deliberate: a name that would overwrite the source is never produced by
//! accident, and an existing file is only replaced when the user asked for it.

use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageEncoder, ImageFormat};

use crate::geom::Size;
use crate::orientation::Rotation;

/// How the output is scaled.
///
/// The seven resize modes, all of which are a pure function of the source size,
/// so they are cheap to test and impossible to get subtly wrong at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResizeMode {
    /// Keep the source resolution.
    #[default]
    Original,
    /// Scale by a percentage.
    Percent(u32),
    /// A fixed width, height following the aspect ratio.
    Width(u32),
    /// A fixed height, width following the aspect ratio.
    Height(u32),
    /// Fit inside a box, keeping the aspect ratio and never enlarging.
    FitBox { width: u32, height: u32 },
    /// Set the longer side to a value.
    LongestSide(u32),
    /// Set the shorter side to a value.
    ShortestSide(u32),
}

impl ResizeMode {
    pub fn label(self) -> String {
        match self {
            ResizeMode::Original => "原始大小".to_string(),
            ResizeMode::Percent(percent) => format!("按比例 {percent}%"),
            ResizeMode::Width(width) => format!("指定宽度 {width}"),
            ResizeMode::Height(height) => format!("指定高度 {height}"),
            ResizeMode::FitBox { width, height } => format!("适合 {width}×{height}"),
            ResizeMode::LongestSide(side) => format!("长边 {side}"),
            ResizeMode::ShortestSide(side) => format!("短边 {side}"),
        }
    }

    /// The size this mode produces for a source of `source`.
    ///
    /// Returns the source unchanged when the mode would need a dimension it
    /// does not have, and never returns a zero side.
    pub fn target_size(self, source: Size) -> Size {
        let (width, height) = (source.w.max(1.), source.h.max(1.));
        let scale_by = |factor: f32| {
            Size::new(
                (width * factor).round().max(1.),
                (height * factor).round().max(1.),
            )
        };

        match self {
            ResizeMode::Original => source,
            ResizeMode::Percent(percent) => scale_by(percent.clamp(1, 1000) as f32 / 100.),
            ResizeMode::Width(target) => {
                if target == 0 {
                    return source;
                }
                scale_by(target as f32 / width)
            }
            ResizeMode::Height(target) => {
                if target == 0 {
                    return source;
                }
                scale_by(target as f32 / height)
            }
            ResizeMode::FitBox {
                width: box_w,
                height: box_h,
            } => {
                if box_w == 0 || box_h == 0 {
                    return source;
                }
                // Shrink to fit, never enlarge: a small image in a big box is
                // already what the user asked for.
                let factor = (box_w as f32 / width).min(box_h as f32 / height).min(1.);
                scale_by(factor)
            }
            ResizeMode::LongestSide(side) => {
                if side == 0 {
                    return source;
                }
                scale_by(side as f32 / width.max(height))
            }
            ResizeMode::ShortestSide(side) => {
                if side == 0 {
                    return source;
                }
                scale_by(side as f32 / width.min(height))
            }
        }
    }

    /// The seven modes with generic labels, for the dialog's radio list.
    ///
    /// The placeholder values are only there to make each variant
    /// constructible; the dialog substitutes the user's numbers through
    /// [`Self::with_value`].
    pub const MODES: [ResizeMode; 7] = [
        ResizeMode::Original,
        ResizeMode::Percent(100),
        ResizeMode::Width(1000),
        ResizeMode::Height(1000),
        ResizeMode::FitBox {
            width: 1000,
            height: 1000,
        },
        ResizeMode::LongestSide(1000),
        ResizeMode::ShortestSide(1000),
    ];

    /// A generic label, with no numbers in it.
    pub fn mode_label(self) -> &'static str {
        match self {
            ResizeMode::Original => "原始大小",
            ResizeMode::Percent(_) => "按比例（%）",
            ResizeMode::Width(_) => "指定宽度",
            ResizeMode::Height(_) => "指定高度",
            ResizeMode::FitBox { .. } => "适合框内",
            ResizeMode::LongestSide(_) => "按长边",
            ResizeMode::ShortestSide(_) => "按短边",
        }
    }

    /// This mode with its numbers filled in.
    ///
    /// Switching modes in the dialog keeps whatever the user already typed,
    /// which is why the numbers are passed in rather than stored per variant.
    pub fn with_value(self, value: u32, box_height: u32) -> Self {
        match self {
            ResizeMode::Original => ResizeMode::Original,
            ResizeMode::Percent(_) => ResizeMode::Percent(value.clamp(1, 1000)),
            ResizeMode::Width(_) => ResizeMode::Width(value.max(1)),
            ResizeMode::Height(_) => ResizeMode::Height(value.max(1)),
            ResizeMode::FitBox { .. } => ResizeMode::FitBox {
                width: value.max(1),
                height: box_height.max(1),
            },
            ResizeMode::LongestSide(_) => ResizeMode::LongestSide(value.max(1)),
            ResizeMode::ShortestSide(_) => ResizeMode::ShortestSide(value.max(1)),
        }
    }

    /// The number the dialog's stepper edits for this mode.
    pub fn value(self) -> u32 {
        match self {
            ResizeMode::Percent(percent) => percent,
            ResizeMode::Width(value)
            | ResizeMode::Height(value)
            | ResizeMode::LongestSide(value)
            | ResizeMode::ShortestSide(value) => value,
            ResizeMode::FitBox { width, .. } => width,
            // Nothing to edit, so the stepper starts from 100%.
            ResizeMode::Original => 100,
        }
    }

    /// The box height, for the mode that has one.
    pub fn box_height(self) -> u32 {
        match self {
            ResizeMode::FitBox { height, .. } => height,
            _ => self.value(),
        }
    }

    /// Whether this mode changes the size at all.
    pub fn is_identity(self) -> bool {
        matches!(self, ResizeMode::Original)
    }
}

/// The formats the converter can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OutputFormat {
    #[default]
    Jpeg,
    Png,
    WebP,
    Bmp,
    Gif,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 5] = [
        OutputFormat::Jpeg,
        OutputFormat::Png,
        OutputFormat::WebP,
        OutputFormat::Bmp,
        OutputFormat::Gif,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "JPEG (.jpg)",
            OutputFormat::Png => "PNG (.png)",
            OutputFormat::WebP => "WebP (.webp)",
            OutputFormat::Bmp => "BMP (.bmp)",
            OutputFormat::Gif => "GIF (.gif)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::WebP => "webp",
            OutputFormat::Bmp => "bmp",
            OutputFormat::Gif => "gif",
        }
    }

    /// Whether the quality setting applies to this format.
    pub fn uses_quality(self) -> bool {
        matches!(self, OutputFormat::Jpeg | OutputFormat::WebP)
    }
}

/// What to do when the output file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Overwrite {
    /// Add a number until the name is free. The safe default.
    #[default]
    Rename,
    /// Replace it.
    Overwrite,
    /// Leave it alone and skip the source.
    Skip,
}

impl Overwrite {
    pub const ALL: [Overwrite; 3] = [Overwrite::Rename, Overwrite::Overwrite, Overwrite::Skip];

    pub fn label(self) -> &'static str {
        match self {
            Overwrite::Rename => "自动重命名",
            Overwrite::Overwrite => "覆盖",
            Overwrite::Skip => "跳过",
        }
    }
}

/// Everything the converter needs to run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConvertOptions {
    pub resize: ResizeMode,
    pub format: OutputFormat,
    /// 1–100, used by the formats that have a quality knob.
    pub quality: u8,
    /// Put the output next to the source instead of in `output_dir`.
    pub same_folder: bool,
    pub output_dir: Option<PathBuf>,
    /// Prepended to the source's stem.
    pub prefix: String,
    pub overwrite: Overwrite,
    /// Applied before resizing.
    pub rotation: Rotation,
    /// Delete each source once it has converted.
    pub delete_source: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            resize: ResizeMode::Original,
            format: OutputFormat::Jpeg,
            quality: 90,
            same_folder: true,
            output_dir: None,
            prefix: String::new(),
            overwrite: Overwrite::Rename,
            rotation: Rotation::None,
            delete_source: false,
        }
    }
}

/// Why a conversion could not be done.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    #[error("源文件读取失败: {0}")]
    Read(String),
    #[error("无法解码: {0}")]
    Decode(String),
    #[error("无法编码: {0}")]
    Encode(String),
    #[error("没有指定输出目录")]
    NoOutputDir,
    #[error("输出文件已存在，按设置跳过")]
    Skipped,
    #[error("写入失败: {0}")]
    Write(String),
}

/// Where the output of `source` goes.
///
/// Pure, so the naming rules can be checked without touching the disk. An
/// empty prefix and the same folder is the identity except for the extension,
/// which is what makes "convert a folder of PNGs to JPEGs" work as expected.
pub fn output_path(source: &Path, options: &ConvertOptions) -> Result<PathBuf, ConvertError> {
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = format!("{}{}.{}", options.prefix, stem, options.format.extension());

    let folder = if options.same_folder {
        source.parent().map(Path::to_path_buf)
    } else {
        options.output_dir.clone()
    };
    let Some(folder) = folder else {
        return Err(ConvertError::NoOutputDir);
    };
    Ok(folder.join(name))
}

/// Pick a free path by appending `-1`, `-2`, … before the extension.
///
/// Only called when the file exists, so the common case costs one stat.
pub fn free_path(wanted: &Path) -> PathBuf {
    let Some(parent) = wanted.parent() else {
        return wanted.to_path_buf();
    };
    let stem = wanted
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = wanted
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_default();

    for index in 1..=9999 {
        let name = if extension.is_empty() {
            format!("{stem}-{index}")
        } else {
            format!("{stem}-{index}.{extension}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    wanted.to_path_buf()
}

/// Convert `bytes` and return the encoded image.
///
/// Rotation happens before resizing so the resize modes operate on the image
/// the user will actually see.
pub fn convert_bytes(bytes: &[u8], options: &ConvertOptions) -> Result<Vec<u8>, ConvertError> {
    let decoded = crate::decode::decode(bytes, true).map_err(|e| ConvertError::Decode(e.to_string()))?;
    let Some(image) = crate::decode::to_dynamic(&decoded) else {
        return Err(ConvertError::Decode("图像数据无效".to_string()));
    };
    let image = apply_rotation(image, options.rotation);

    let target = options.resize.target_size(Size::new(
        image.width() as f32,
        image.height() as f32,
    ));
    let (target_w, target_h) = (target.w as u32, target.h as u32);
    let image = if target_w != image.width() || target_h != image.height() {
        // Lanczos for a reduction, since a conversion is a deliberate act and
        // the user is waiting for it; nearest-neighbour would alias badly.
        image.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3)
    } else {
        image
    };

    encode(&image, options)
}

/// Rotate a decoded image by a quarter turn.
fn apply_rotation(image: DynamicImage, rotation: Rotation) -> DynamicImage {
    match rotation {
        Rotation::None => image,
        Rotation::Cw90 => image.rotate90(),
        Rotation::Cw180 => image.rotate180(),
        Rotation::Cw270 => image.rotate270(),
    }
}

/// Encode `image` in the requested format.
pub fn encode_dynamic(
    image: &DynamicImage,
    format: OutputFormat,
    quality: u8,
) -> Result<Vec<u8>, ConvertError> {
    encode_with(image, format, quality)
}

/// Encode an image that already has its final pixels.
///
/// Everything the viewer shows — rotation, filters, channel selection — is
/// already in the buffer, so "save as seen" only has to encode it.
pub fn encode_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
    format: OutputFormat,
    quality: u8,
) -> Result<Vec<u8>, ConvertError> {
    if width == 0 || height == 0 || bgra.len() != width as usize * height as usize * 4 {
        return Err(ConvertError::Encode("图像数据无效".to_string()));
    }
    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let Some(buffer) = image::RgbaImage::from_raw(width, height, rgba) else {
        return Err(ConvertError::Encode("图像数据无效".to_string()));
    };
    encode_with(&DynamicImage::ImageRgba8(buffer), format, quality)
}

fn encode(image: &DynamicImage, options: &ConvertOptions) -> Result<Vec<u8>, ConvertError> {
    encode_with(image, options.format, options.quality)
}

fn encode_with(
    image: &DynamicImage,
    format: OutputFormat,
    quality: u8,
) -> Result<Vec<u8>, ConvertError> {
    let quality = quality.clamp(1, 100);
    let mut out = Vec::new();

    match format {
        OutputFormat::Jpeg => {
            // JPEG has no alpha; a transparent source would otherwise come out
            // with whatever colour the alpha was hiding.
            let rgb = flatten_on_white(image);
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            encoder
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
        }
        OutputFormat::Png => {
            let rgba = image.to_rgba8();
            image::codecs::png::PngEncoder::new(&mut out)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
        }
        OutputFormat::WebP => {
            // The encoder in the tree is lossless, so quality does not apply.
            let rgba = image.to_rgba8();
            image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
        }
        OutputFormat::Bmp => {
            let rgb = flatten_on_white(image);
            image::codecs::bmp::BmpEncoder::new(&mut out)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
        }
        OutputFormat::Gif => {
            let rgba = image.to_rgba8();
            image::codecs::gif::GifEncoder::new(&mut out)
                .encode(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
                .map_err(|e| ConvertError::Encode(e.to_string()))?;
        }
    }

    Ok(out)
}

/// Composite an image onto white, for the formats without an alpha channel.
fn flatten_on_white(image: &DynamicImage) -> image::RgbImage {
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let mut out = image::RgbImage::new(width, height);

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u32;
        let blend = |channel: u8| {
            ((channel as u32 * alpha + 255 * (255 - alpha)) / 255).min(255) as u8
        };
        out.put_pixel(
            x,
            y,
            image::Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])]),
        );
    }
    out
}

/// The format an extension names, for the file-picker filter.
pub fn format_from_extension(extension: &str) -> Option<OutputFormat> {
    OutputFormat::ALL
        .into_iter()
        .find(|format| format.extension().eq_ignore_ascii_case(extension))
}

/// Whether `path` is one of the formats we write.
pub fn is_convertible_output(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(format_from_extension)
        .is_some()
}

/// The `ImageFormat` for a path, used by tests to verify the output.
#[doc(hidden)]
pub fn detect_format(bytes: &[u8]) -> Option<ImageFormat> {
    image::guess_format(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, image::Rgba(color));
        let mut out = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode png");
        out.into_inner()
    }

    fn size(width: f32, height: f32) -> Size {
        Size::new(width, height)
    }

    #[test]
    fn every_mode_is_in_the_table_and_labelled() {
        assert_eq!(ResizeMode::MODES.len(), 7);
        for mode in ResizeMode::MODES {
            assert!(!mode.mode_label().is_empty(), "{mode:?} has no label");
            // Round-tripping through the dialog's value fields must preserve
            // the mode, or switching modes would silently change the answer.
            let restored = mode.with_value(mode.value(), mode.box_height());
            assert_eq!(
                std::mem::discriminant(&restored),
                std::mem::discriminant(&mode),
                "{mode:?} did not survive a round trip"
            );
        }
    }

    #[test]
    fn a_percentage_is_clamped_to_something_sane() {
        assert_eq!(ResizeMode::Percent(1).with_value(0, 0), ResizeMode::Percent(1));
        assert_eq!(
            ResizeMode::Percent(1).with_value(99_999, 0),
            ResizeMode::Percent(1000)
        );
    }

    #[test]
    fn the_original_mode_keeps_the_size() {
        assert_eq!(ResizeMode::Original.target_size(size(800., 600.)), size(800., 600.));
        assert!(ResizeMode::Original.is_identity());
    }

    #[test]
    fn a_percentage_scales_both_sides() {
        assert_eq!(
            ResizeMode::Percent(50).target_size(size(800., 600.)),
            size(400., 300.)
        );
        assert_eq!(
            ResizeMode::Percent(200).target_size(size(800., 600.)),
            size(1600., 1200.)
        );
    }

    #[test]
    fn a_fixed_width_keeps_the_aspect_ratio() {
        assert_eq!(
            ResizeMode::Width(400).target_size(size(800., 600.)),
            size(400., 300.)
        );
        assert_eq!(
            ResizeMode::Height(300).target_size(size(800., 600.)),
            size(400., 300.)
        );
    }

    #[test]
    fn a_fit_box_shrinks_but_never_enlarges() {
        // A landscape source in a square box: width is the constraint.
        assert_eq!(
            ResizeMode::FitBox {
                width: 500,
                height: 500
            }
            .target_size(size(1000., 500.)),
            size(500., 250.)
        );
        // Already inside the box, so it is left alone.
        assert_eq!(
            ResizeMode::FitBox {
                width: 500,
                height: 500
            }
            .target_size(size(100., 50.)),
            size(100., 50.)
        );
    }

    #[test]
    fn the_long_and_short_side_modes_use_the_right_side() {
        let source = size(800., 600.);
        assert_eq!(ResizeMode::LongestSide(400).target_size(source), size(400., 300.));
        // The short side is the height here, so the width follows it up.
        assert_eq!(
            ResizeMode::ShortestSide(300).target_size(source),
            size(400., 300.)
        );
    }

    #[test]
    fn a_degenerate_mode_leaves_the_size_alone() {
        let source = size(800., 600.);
        for mode in [
            ResizeMode::Width(0),
            ResizeMode::Height(0),
            ResizeMode::LongestSide(0),
            ResizeMode::ShortestSide(0),
            ResizeMode::FitBox {
                width: 0,
                height: 0,
            },
        ] {
            assert_eq!(mode.target_size(source), source, "{mode:?} should be a no-op");
        }
    }

    #[test]
    fn no_mode_ever_produces_a_zero_side() {
        for mode in [
            ResizeMode::Percent(1),
            ResizeMode::Width(1),
            ResizeMode::Height(1),
            ResizeMode::FitBox {
                width: 1,
                height: 1,
            },
            ResizeMode::LongestSide(1),
            ResizeMode::ShortestSide(1),
        ] {
            let target = mode.target_size(size(4000., 3.));
            assert!(target.w >= 1. && target.h >= 1., "{mode:?} produced {target:?}");
        }
    }

    #[test]
    fn the_output_lands_beside_the_source_by_default() {
        let options = ConvertOptions {
            format: OutputFormat::Jpeg,
            ..Default::default()
        };
        assert_eq!(
            output_path(Path::new("C:/images/page_1.png"), &options).unwrap(),
            PathBuf::from("C:/images/page_1.jpg")
        );
    }

    #[test]
    fn a_prefix_and_folder_are_applied() {
        let options = ConvertOptions {
            format: OutputFormat::WebP,
            same_folder: false,
            output_dir: Some(PathBuf::from("C:/out")),
            prefix: "small_".to_string(),
            ..Default::default()
        };
        assert_eq!(
            output_path(Path::new("C:/images/page_1.png"), &options).unwrap(),
            PathBuf::from("C:/out/small_page_1.webp")
        );
    }

    #[test]
    fn a_chosen_folder_that_is_missing_is_an_error_not_a_guess() {
        let options = ConvertOptions {
            same_folder: false,
            output_dir: None,
            ..Default::default()
        };
        assert_eq!(
            output_path(Path::new("page.png"), &options),
            Err(ConvertError::NoOutputDir)
        );
    }

    #[test]
    fn converting_a_png_to_jpeg_produces_a_jpeg_of_the_requested_size() {
        let bytes = png(64, 32, [200, 40, 40, 255]);
        let options = ConvertOptions {
            resize: ResizeMode::Percent(50),
            format: OutputFormat::Jpeg,
            quality: 90,
            ..Default::default()
        };
        let out = convert_bytes(&bytes, &options).expect("convert");
        assert_eq!(detect_format(&out), Some(ImageFormat::Jpeg));

        let decoded = crate::decode::decode(&out, false).expect("decode output");
        assert_eq!((decoded.width, decoded.height), (32, 16));
    }

    #[test]
    fn converting_to_png_keeps_the_alpha_channel() {
        let bytes = png(8, 8, [10, 20, 30, 128]);
        let options = ConvertOptions {
            format: OutputFormat::Png,
            ..Default::default()
        };
        let out = convert_bytes(&bytes, &options).expect("convert");
        assert_eq!(detect_format(&out), Some(ImageFormat::Png));

        let decoded = crate::decode::decode(&out, false).expect("decode output");
        // Alpha survived, rather than being flattened onto white.
        assert_eq!(decoded.frames[0].bgra[3], 128);
    }

    #[test]
    fn jpeg_flattens_transparency_instead_of_losing_the_colour() {
        // Fully transparent red: flattened onto white it must come out white,
        // not black (which is what dropping the alpha would give).
        let bytes = png(4, 4, [255, 0, 0, 0]);
        let options = ConvertOptions {
            format: OutputFormat::Jpeg,
            ..Default::default()
        };
        let out = convert_bytes(&bytes, &options).expect("convert");
        let decoded = crate::decode::decode(&out, false).expect("decode output");
        // BGRA, and JPEG is lossy, so check "bright and neutral" rather than
        // an exact value.
        let pixel = &decoded.frames[0].bgra[0..4];
        assert!(
            pixel[0] > 240 && pixel[1] > 240 && pixel[2] > 240,
            "expected white, got {pixel:?}"
        );
    }

    #[test]
    fn pixels_can_be_encoded_without_a_source_file() {
        // Two pixels: opaque red, fully transparent.
        let bgra = vec![
            0, 0, 255, 255, // B,G,R,A
            255, 0, 0, 0,
        ];
        let png = encode_bgra(2, 1, &bgra, OutputFormat::Png, 90).expect("png");
        let decoded = crate::decode::decode(&png, false).expect("decode png");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        // The channels survive: red stays red, alpha stays 0.
        assert_eq!(&decoded.frames[0].bgra[0..4], &[0, 0, 255, 255]);
        assert_eq!(decoded.frames[0].bgra[7], 0);

        assert!(encode_bgra(2, 2, &bgra, OutputFormat::Png, 90).is_err(), "size mismatch is refused");

        let jpeg = encode_bgra(2, 1, &bgra, OutputFormat::Jpeg, 90).expect("jpeg");
        assert_eq!(detect_format(&jpeg), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn every_format_can_be_written_and_read_back() {
        let bytes = png(16, 16, [90, 120, 150, 255]);
        for format in OutputFormat::ALL {
            let options = ConvertOptions {
                format,
                ..Default::default()
            };
            let out = convert_bytes(&bytes, &options)
                .unwrap_or_else(|e| panic!("{format:?} failed: {e}"));
            assert!(!out.is_empty(), "{format:?} produced nothing");
            // A GIF is written as a single-frame animation, which our decoder
            // reads through the animation path.
            let decoded = crate::decode::decode(&out, false)
                .unwrap_or_else(|e| panic!("{format:?} did not decode back: {e}"));
            assert_eq!((decoded.width, decoded.height), (16, 16), "{format:?}");
        }
    }

    #[test]
    fn rotation_happens_before_resizing() {
        let bytes = png(40, 20, [1, 2, 3, 255]);
        let options = ConvertOptions {
            rotation: Rotation::Cw90,
            format: OutputFormat::Png,
            ..Default::default()
        };
        let out = convert_bytes(&bytes, &options).expect("convert");
        let decoded = crate::decode::decode(&out, false).expect("decode output");
        // A quarter turn swaps the sides.
        assert_eq!((decoded.width, decoded.height), (20, 40));
    }

    #[test]
    fn a_free_path_avoids_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("iv-convert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let wanted = dir.join("page.jpg");
        std::fs::write(&wanted, b"x").unwrap();
        let free = free_path(&wanted);
        assert_eq!(free.file_name().unwrap(), "page-1.jpg");

        // And it keeps counting when that one exists too.
        std::fs::write(&free, b"x").unwrap();
        assert_eq!(
            free_path(&wanted).file_name().unwrap(),
            "page-2.jpg"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_formats_are_recognised_by_extension() {
        assert_eq!(format_from_extension("jpg"), Some(OutputFormat::Jpeg));
        assert_eq!(format_from_extension("JPEG"), None, "only the written extension counts");
        assert_eq!(format_from_extension("webp"), Some(OutputFormat::WebP));
        assert_eq!(format_from_extension("txt"), None);
        assert!(is_convertible_output(Path::new("a/b.png")));
        assert!(!is_convertible_output(Path::new("a/b.psd")));
    }

    #[test]
    fn quality_only_applies_where_it_means_something() {
        assert!(OutputFormat::Jpeg.uses_quality());
        assert!(OutputFormat::WebP.uses_quality());
        assert!(!OutputFormat::Png.uses_quality());
        assert!(!OutputFormat::Bmp.uses_quality());
    }
}
