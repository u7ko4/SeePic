//! Batch image conversion: the engine behind the standalone converter window.
//!
//! The UI lives in the application crate; everything here is a pure function of
//! its inputs, so the naming rules, the resize maths, the conflict policy and
//! the encoders can be tested without a window or a file dialog.
//!
//! The user-facing decisions are deliberately kept together in [`BatchOptions`]
//! in the order the window presents them: size, output, conflict, format.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::codecs::avif::AvifEncoder;
use image::codecs::gif::GifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::tiff::TiffEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageEncoder};

use crate::geom::Size;

/// How the output is sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SizeMode {
    /// Keep the source resolution.
    Original,
    /// Set the longer side, keeping the aspect ratio.
    LongEdge(u32),
    /// Fit inside a box, keeping the aspect ratio.
    Fit { width: u32, height: u32 },
    /// Fill a box and crop the overflow.
    Crop {
        width: u32,
        height: u32,
        anchor: Anchor,
    },
}

impl Default for SizeMode {
    fn default() -> Self {
        SizeMode::LongEdge(1600)
    }
}

impl SizeMode {
    pub fn label(self) -> &'static str {
        match self {
            SizeMode::Original => "Original",
            SizeMode::LongEdge(_) => "Long edge",
            SizeMode::Fit { .. } => "Fit",
            SizeMode::Crop { .. } => "Crop",
        }
    }

    /// The width and height boxes the mode edits, if any.
    pub fn box_size(self) -> Option<(u32, u32)> {
        match self {
            SizeMode::Fit { width, height } | SizeMode::Crop { width, height, .. } => {
                Some((width, height))
            }
            _ => None,
        }
    }

    pub fn with_box(self, width: u32, height: u32) -> Self {
        match self {
            SizeMode::Fit { .. } => SizeMode::Fit { width, height },
            SizeMode::Crop { anchor, .. } => SizeMode::Crop {
                width,
                height,
                anchor,
            },
            other => other,
        }
    }

    pub fn long_edge(self) -> Option<u32> {
        match self {
            SizeMode::LongEdge(value) => Some(value),
            _ => None,
        }
    }

    pub fn anchor(self) -> Anchor {
        match self {
            SizeMode::Crop { anchor, .. } => anchor,
            _ => Anchor::Center,
        }
    }
}

/// Where a crop takes its rectangle from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    /// Horizontal fraction of the free space, 0 = left, 1 = right.
    pub fn x(self) -> f32 {
        match self {
            Anchor::TopLeft | Anchor::Left | Anchor::BottomLeft => 0.,
            Anchor::Top | Anchor::Center | Anchor::Bottom => 0.5,
            Anchor::TopRight | Anchor::Right | Anchor::BottomRight => 1.,
        }
    }

    /// Vertical fraction of the free space, 0 = top, 1 = bottom.
    pub fn y(self) -> f32 {
        match self {
            Anchor::TopLeft | Anchor::Top | Anchor::TopRight => 0.,
            Anchor::Left | Anchor::Center | Anchor::Right => 0.5,
            Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => 1.,
        }
    }
}

/// The formats the batch converter can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BatchFormat {
    /// Re-encode in the source's own format.
    #[default]
    Keep,
    Jpeg,
    Png,
    WebP,
    Avif,
    Gif,
    Bmp,
    Tiff,
    Jxl,
    Pdf,
}

impl BatchFormat {
    pub const ALL: [BatchFormat; 10] = [
        BatchFormat::Keep,
        BatchFormat::Jpeg,
        BatchFormat::Png,
        BatchFormat::WebP,
        BatchFormat::Avif,
        BatchFormat::Jxl,
        BatchFormat::Gif,
        BatchFormat::Bmp,
        BatchFormat::Tiff,
        BatchFormat::Pdf,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BatchFormat::Keep => "Keep original format",
            BatchFormat::Jpeg => "JPEG",
            BatchFormat::Png => "PNG",
            BatchFormat::WebP => "WebP",
            BatchFormat::Avif => "AVIF",
            BatchFormat::Gif => "GIF",
            BatchFormat::Bmp => "BMP",
            BatchFormat::Tiff => "TIFF",
            BatchFormat::Jxl => "JPEG XL",
            BatchFormat::Pdf => "PDF",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            BatchFormat::Keep => "Original",
            BatchFormat::Jpeg => "JPEG",
            BatchFormat::Png => "PNG",
            BatchFormat::WebP => "WebP",
            BatchFormat::Avif => "AVIF",
            BatchFormat::Gif => "GIF",
            BatchFormat::Bmp => "BMP",
            BatchFormat::Tiff => "TIFF",
            BatchFormat::Jxl => "JXL",
            BatchFormat::Pdf => "PDF",
        }
    }

    /// The extension written for this format.
    ///
    /// `Keep` and a source without a usable extension fall back to PNG, so the
    /// output always has a name a decoder can recognise.
    pub fn extension(self, source_extension: &str) -> String {
        match self {
            BatchFormat::Keep => {
                let extension = source_extension
                    .trim_start_matches('.')
                    .to_ascii_lowercase();
                match BatchFormat::from_extension(&extension) {
                    Some(format) => format.extension("").into(),
                    None => "png".to_string(),
                }
            }
            BatchFormat::Jpeg => "jpg".to_string(),
            BatchFormat::Png => "png".to_string(),
            BatchFormat::WebP => "webp".to_string(),
            BatchFormat::Avif => "avif".to_string(),
            BatchFormat::Gif => "gif".to_string(),
            BatchFormat::Bmp => "bmp".to_string(),
            BatchFormat::Tiff => "tiff".to_string(),
            BatchFormat::Jxl => "jxl".to_string(),
            BatchFormat::Pdf => "pdf".to_string(),
        }
    }

    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" | "jpe" | "jfif" => Some(BatchFormat::Jpeg),
            "png" => Some(BatchFormat::Png),
            "webp" => Some(BatchFormat::WebP),
            "avif" => Some(BatchFormat::Avif),
            "gif" => Some(BatchFormat::Gif),
            "bmp" => Some(BatchFormat::Bmp),
            "tif" | "tiff" => Some(BatchFormat::Tiff),
            "jxl" => Some(BatchFormat::Jxl),
            "pdf" => Some(BatchFormat::Pdf),
            _ => None,
        }
    }

    /// Formats whose encoder can keep an alpha channel.
    pub fn supports_alpha(self) -> bool {
        matches!(
            self,
            BatchFormat::Keep
                | BatchFormat::Png
                | BatchFormat::WebP
                | BatchFormat::Avif
                | BatchFormat::Gif
                | BatchFormat::Tiff
                | BatchFormat::Jxl
        )
    }

    /// Whether the quality value changes the output for this format.
    pub fn uses_quality(self) -> bool {
        matches!(
            self,
            BatchFormat::Jpeg | BatchFormat::Avif | BatchFormat::Pdf
        )
    }
}

/// What to do when the output file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ConflictPolicy {
    /// Add ` (2)`, ` (3)`, … until the name is free. The safe default.
    #[default]
    Rename,
    /// Leave the old file and count the source as skipped.
    Skip,
    /// Replace the old file. The window asks before starting.
    Overwrite,
    /// Move the old file into `_backup` first.
    Backup,
}

impl ConflictPolicy {
    pub const ALL: [ConflictPolicy; 4] = [
        ConflictPolicy::Rename,
        ConflictPolicy::Skip,
        ConflictPolicy::Overwrite,
        ConflictPolicy::Backup,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConflictPolicy::Rename => "Rename",
            ConflictPolicy::Skip => "Skip existing",
            ConflictPolicy::Overwrite => "Overwrite",
            ConflictPolicy::Backup => "Move to _backup",
        }
    }
}

/// Where the output folder comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OutputPlacement {
    /// A subfolder under each source folder. The safe default.
    #[default]
    Subfolder,
    /// One folder chosen by the user for the whole batch.
    Custom,
    /// Keep the source's folder structure under one root.
    Mirror,
    /// Write beside each source. The window warns before starting.
    SameFolder,
}

impl OutputPlacement {
    pub const ALL: [OutputPlacement; 4] = [
        OutputPlacement::Subfolder,
        OutputPlacement::Custom,
        OutputPlacement::Mirror,
        OutputPlacement::SameFolder,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutputPlacement::Subfolder => "Source folder / output",
            OutputPlacement::Custom => "Choose a folder",
            OutputPlacement::Mirror => "Mirror source folders",
            OutputPlacement::SameFolder => "Beside the source",
        }
    }
}

/// How a resize interpolates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Resample {
    Nearest,
    #[default]
    Smooth,
    SmoothSharpen,
}

impl Resample {
    pub const ALL: [Resample; 3] = [Resample::Nearest, Resample::Smooth, Resample::SmoothSharpen];

    pub fn label(self) -> &'static str {
        match self {
            Resample::Nearest => "No filter",
            Resample::Smooth => "Smooth",
            Resample::SmoothSharpen => "Smooth + sharpen",
        }
    }

    fn filter(self) -> FilterType {
        match self {
            Resample::Nearest => FilterType::Nearest,
            Resample::Smooth => FilterType::CatmullRom,
            Resample::SmoothSharpen => FilterType::Lanczos3,
        }
    }
}

/// Everything the batch converter needs to run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BatchOptions {
    pub size: SizeMode,
    /// Never turn a smaller image into a bigger one.
    pub only_shrink: bool,
    /// Apply the EXIF orientation while decoding.
    pub auto_orient: bool,
    pub resample: Resample,
    pub placement: OutputPlacement,
    /// Name of the subfolder used by [`OutputPlacement::Subfolder`].
    pub subfolder: String,
    /// Root folder for [`OutputPlacement::Custom`] and [`OutputPlacement::Mirror`].
    pub output_dir: Option<PathBuf>,
    /// File-name template, e.g. `{name}_{w}x{h}`.
    pub template: String,
    pub conflict: ConflictPolicy,
    pub format: BatchFormat,
    /// 1–100, for the formats that have a lossy quality knob.
    pub quality: u8,
    pub keep_exif: bool,
    pub keep_iptc: bool,
    pub keep_color_profile: bool,
    pub keep_timestamps: bool,
    /// Remove GPS fields from the EXIF block that is written.
    pub strip_location: bool,
    /// Colour transparent pixels are composited onto for formats without alpha.
    pub background: [u8; 3],
    /// Worker threads. 1 runs serially.
    pub jobs: usize,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            size: SizeMode::LongEdge(1600),
            only_shrink: true,
            auto_orient: true,
            resample: Resample::Smooth,
            placement: OutputPlacement::Subfolder,
            subfolder: "output".to_string(),
            output_dir: None,
            template: "{name}_{w}x{h}".to_string(),
            conflict: ConflictPolicy::Rename,
            format: BatchFormat::WebP,
            quality: 80,
            keep_exif: true,
            keep_iptc: true,
            keep_color_profile: true,
            keep_timestamps: true,
            strip_location: true,
            background: [255, 255, 255],
            jobs: default_jobs(),
        }
    }
}

/// `min(available_parallelism, 8)`, matching the PRD's default.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().min(8))
        .unwrap_or(4)
        .max(1)
}

/// The largest concurrency the machine can actually run: every logical
/// processor, hyperthreads included.
pub fn max_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .max(1)
}

/// The target size for a source of `source`.
///
/// The size is computed before the image is decoded, so the window can show it
/// in the list and in the preview while the batch has not started.
pub fn target_size(source: Size, mode: SizeMode, only_shrink: bool) -> Size {
    let width = source.w.max(1.);
    let height = source.h.max(1.);
    let scale = |factor: f32| {
        Size::new(
            (width * factor).round().max(1.),
            (height * factor).round().max(1.),
        )
    };

    match mode {
        SizeMode::Original => source,
        SizeMode::LongEdge(edge) => {
            if edge == 0 {
                return source;
            }
            let mut factor = edge as f32 / width.max(height);
            if only_shrink {
                factor = factor.min(1.);
            }
            scale(factor)
        }
        SizeMode::Fit {
            width: box_w,
            height: box_h,
        } => {
            if box_w == 0 || box_h == 0 {
                return source;
            }
            let mut factor = (box_w as f32 / width).min(box_h as f32 / height);
            if only_shrink {
                factor = factor.min(1.);
            }
            scale(factor)
        }
        SizeMode::Crop {
            width: box_w,
            height: box_h,
            ..
        } => {
            if box_w == 0 || box_h == 0 {
                return source;
            }
            let mut factor = (box_w as f32 / width).max(box_h as f32 / height);
            if only_shrink {
                factor = factor.min(1.);
            }
            let scaled = scale(factor);
            // The crop cannot invent pixels: a source smaller than the target
            // keeps its own size even when the user asked for a bigger box.
            Size::new(
                scaled.w.min(box_w as f32).max(1.),
                scaled.h.min(box_h as f32).max(1.),
            )
        }
    }
}

/// The size a decoded image ends up at, honouring the EXIF orientation.
pub fn oriented_size(source: Size, orientation: Option<u8>, auto_orient: bool) -> Size {
    if !auto_orient {
        return source;
    }
    match orientation.and_then(crate::orientation::Orientation::from_exif) {
        Some(orientation) => orientation.applied_size(source),
        None => source,
    }
}

/// Expand a file-name template into the stem of the output name.
///
/// Supported tokens: `{name}`, `{name_ext}`, `{w}`, `{h}`, `{date}`,
/// `{date:ymd}`, `{date:y-m-d}`, `{seq}` and `{seq:3}`. Unknown tokens are
/// left alone rather than silently dropped, so a typo is visible in the
/// example line instead of producing a different name than the user expected.
pub fn render_name(
    template: &str,
    source_name: &str,
    width: u32,
    height: u32,
    sequence: usize,
) -> String {
    let stem = source_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(source_name);
    let date = Date::now();
    let mut out = String::new();
    let mut characters = template.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '{' {
            out.push(character);
            continue;
        }
        let mut token = String::new();
        let mut closed = false;
        for next in characters.by_ref() {
            if next == '}' {
                closed = true;
                break;
            }
            token.push(next);
        }
        if !closed {
            out.push('{');
            out.push_str(&token);
            continue;
        }

        let replacement = match token.as_str() {
            "name" => Some(stem.to_string()),
            "name_ext" => Some(source_name.to_string()),
            "w" => Some(width.to_string()),
            "h" => Some(height.to_string()),
            "date" | "date:ymd" => Some(date.ymd()),
            "date:y-m-d" => Some(date.y_m_d()),
            "date:ymd-hms" => Some(date.ymd_hms()),
            other if other.starts_with("seq") => {
                let padding = other
                    .strip_prefix("seq:")
                    .and_then(|digits| digits.parse::<usize>().ok())
                    .unwrap_or(0)
                    .clamp(1, 12);
                Some(format!("{sequence:0padding$}"))
            }
            _ => None,
        };
        match replacement {
            Some(text) => out.push_str(&text),
            None => {
                out.push('{');
                out.push_str(&token);
                out.push('}');
            }
        }
    }

    if out.trim().is_empty() {
        return sanitize_name(stem);
    }
    sanitize_name(&out)
}

/// Replace characters a Windows file name cannot contain, and keep the result
/// to a length the whole path can still hold.
pub fn sanitize_name(name: &str) -> String {
    let mut cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_control() || "\\/:*?\"<>|".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    // Windows silently strips trailing dots and spaces, which turns a template
    // ending in `{name}.` into a name that is not what was previewed.
    cleaned = cleaned.trim_end_matches([' ', '.']).to_string();
    cleaned = cleaned.trim_start().to_string();
    if cleaned.is_empty() {
        cleaned = "image".to_string();
    }
    if cleaned.chars().count() > 180 {
        cleaned = cleaned.chars().take(180).collect();
        cleaned = cleaned.trim_end_matches([' ', '.']).to_string();
    }

    // Device names are reserved even with an extension.
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or(&cleaned)
        .to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        cleaned.insert(0, '_');
    }
    cleaned
}

/// The file name for one output.
pub fn output_file_name(
    options: &BatchOptions,
    source_name: &str,
    width: u32,
    height: u32,
    sequence: usize,
) -> String {
    let source_ext = source_name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or("");
    let stem = render_name(&options.template, source_name, width, height, sequence);
    let extension = options.format.extension(source_ext);
    format!("{stem}.{extension}")
}

/// The folder a source's output belongs in.
pub fn output_folder(
    source: &Path,
    relative: Option<&Path>,
    options: &BatchOptions,
    mirror_root: Option<&Path>,
) -> Result<PathBuf, BatchError> {
    let source_folder = source
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    match options.placement {
        OutputPlacement::Subfolder => {
            let name = if options.subfolder.trim().is_empty() {
                "output"
            } else {
                options.subfolder.trim()
            };
            Ok(source_folder.join(sanitize_name(name)))
        }
        OutputPlacement::Custom => options.output_dir.clone().ok_or(BatchError::NoOutputDir),
        OutputPlacement::Mirror => {
            let root = options
                .output_dir
                .clone()
                .or_else(|| mirror_root.map(Path::to_path_buf))
                .unwrap_or_else(|| source_folder.join("output"));
            let relative_folder = relative
                .and_then(Path::parent)
                .filter(|parent| !parent.as_os_str().is_empty());
            Ok(match relative_folder {
                Some(parent) => root.join(parent),
                None => root,
            })
        }
        OutputPlacement::SameFolder => Ok(source_folder),
    }
}

/// The full path one output will be written to, before conflicts are resolved.
pub fn planned_output_path(
    source: &Path,
    relative: Option<&Path>,
    options: &BatchOptions,
    mirror_root: Option<&Path>,
    source_name: &str,
    width: u32,
    height: u32,
    sequence: usize,
) -> Result<PathBuf, BatchError> {
    let folder = output_folder(source, relative, options, mirror_root)?;
    Ok(folder.join(output_file_name(
        options,
        source_name,
        width,
        height,
        sequence,
    )))
}

/// What the conflict policy decided for one output path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictDecision {
    /// Write to this path (it is free, or overwriting was requested).
    Write(PathBuf),
    /// Leave the existing file and count the source as skipped.
    Skip,
    /// Move the existing file to the backup path, then write to `target`.
    Backup { target: PathBuf, backup: PathBuf },
}

/// Apply the conflict policy to a planned output path.
pub fn resolve_conflict(path: &Path, policy: ConflictPolicy) -> ConflictDecision {
    if !path.exists() {
        return ConflictDecision::Write(path.to_path_buf());
    }
    match policy {
        ConflictPolicy::Rename => ConflictDecision::Write(free_rename_path(path)),
        ConflictPolicy::Skip => ConflictDecision::Skip,
        ConflictPolicy::Overwrite => ConflictDecision::Write(path.to_path_buf()),
        ConflictPolicy::Backup => {
            let backup = backup_path(path);
            ConflictDecision::Backup {
                target: path.to_path_buf(),
                backup,
            }
        }
    }
}

/// `page.jpg` → `page (2).jpg`, then `page (3).jpg`.
pub fn free_rename_path(path: &Path) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();

    for index in 2..=9999 {
        let name = if extension.is_empty() {
            format!("{stem} ({index})")
        } else {
            format!("{stem} ({index}).{extension}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// The path an old file is moved to under [`ConflictPolicy::Backup`].
pub fn backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    let folder = parent.join("_backup");
    let wanted = folder.join(name);
    if !wanted.exists() {
        return wanted;
    }
    free_rename_path(&wanted)
}

/// Why a conversion could not be done.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BatchError {
    #[error("{0}")]
    Read(String),
    #[error("{0}")]
    Decode(String),
    #[error("{0}")]
    Encode(String),
    #[error("No output folder is chosen")]
    NoOutputDir,
    #[error("The source format cannot be kept")]
    UnsupportedSource,
    #[error("The target size is invalid")]
    InvalidSize,
    #[error("{0}")]
    Write(String),
}

impl BatchError {
    /// The PRD failure code a user can act on.
    pub fn code(&self) -> FailureCode {
        match self {
            BatchError::Read(_) | BatchError::Decode(_) => FailureCode::E01,
            BatchError::NoOutputDir => FailureCode::E02,
            BatchError::Write(message) => {
                let lower = message.to_ascii_lowercase();
                if lower.contains("space") || lower.contains("空间") {
                    FailureCode::E03
                } else {
                    FailureCode::E02
                }
            }
            BatchError::InvalidSize => FailureCode::E04,
            BatchError::UnsupportedSource => FailureCode::E07,
            BatchError::Encode(_) => FailureCode::E99,
        }
    }
}

/// The reason codes from the PRD's report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
    E01,
    E02,
    E03,
    E04,
    E05,
    E06,
    E07,
    E99,
}

impl FailureCode {
    pub fn code(self) -> &'static str {
        match self {
            FailureCode::E01 => "E01",
            FailureCode::E02 => "E02",
            FailureCode::E03 => "E03",
            FailureCode::E04 => "E04",
            FailureCode::E05 => "E05",
            FailureCode::E06 => "E06",
            FailureCode::E07 => "E07",
            FailureCode::E99 => "E99",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FailureCode::E01 => "Source is damaged or unreadable",
            FailureCode::E02 => "No write permission for the output folder",
            FailureCode::E03 => "Not enough disk space",
            FailureCode::E04 => "Target size is invalid",
            FailureCode::E05 => "Not enough memory for this image",
            FailureCode::E06 => "File name is invalid or too long",
            FailureCode::E07 => "The format cannot hold this colour/alpha combination",
            FailureCode::E99 => "Unknown error",
        }
    }
}

/// A decoded, resized and encoded image, ready to be written.
#[derive(Debug, Clone)]
pub struct Conversion {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: BatchFormat,
    /// Whether the source carried an alpha channel.
    pub had_alpha: bool,
}

/// Convert one image in memory.
pub fn convert(
    bytes: &[u8],
    source_name: &str,
    options: &BatchOptions,
) -> Result<Conversion, BatchError> {
    let format = match options.format {
        BatchFormat::Keep => {
            let extension = source_name
                .rsplit_once('.')
                .map(|(_, extension)| extension)
                .unwrap_or("");
            BatchFormat::from_extension(extension).ok_or(BatchError::UnsupportedSource)?
        }
        other => other,
    };
    if format == BatchFormat::Pdf {
        // PDF has no encoder in the image libraries; a one-page PDF wrapping a
        // JPEG is exact, self-contained and opens everywhere.
        return convert_to_pdf(bytes, options);
    }

    let decoded = crate::decode::decode(bytes, options.auto_orient)
        .map_err(|error| BatchError::Decode(error.to_string()))?;
    let image = crate::decode::to_dynamic(&decoded)
        .ok_or_else(|| BatchError::Decode("The decoded image is empty".to_string()))?;
    let had_alpha = image.color().has_alpha();

    let image = apply_geometry(&image, options);
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err(BatchError::InvalidSize);
    }

    let mut encoded = encode(&image, format, options)?;
    if options.keep_exif {
        let exif = source_exif(bytes, options.strip_location);
        if let Some(exif) = exif {
            encoded = inject_exif(encoded, format, &exif)?;
        }
    }
    if format == BatchFormat::Jpeg {
        encoded = copy_jpeg_metadata(bytes, encoded, options);
    }
    if format == BatchFormat::Png {
        encoded = copy_png_metadata(bytes, encoded, options);
    }

    Ok(Conversion {
        bytes: encoded,
        width,
        height,
        format,
        had_alpha,
    })
}

/// Decode, rotate and resize, but do not encode. Used by the preview.
pub fn apply_geometry(image: &DynamicImage, options: &BatchOptions) -> DynamicImage {
    let source = Size::new(image.width() as f32, image.height() as f32);
    let target = target_size(source, options.size, options.only_shrink);
    let (target_w, target_h) = (target.w.round() as u32, target.h.round() as u32);

    match options.size {
        SizeMode::Original => image.clone(),
        SizeMode::LongEdge(_) | SizeMode::Fit { .. } => {
            if target_w == image.width() && target_h == image.height() {
                return image.clone();
            }
            let resized = image.resize_exact(target_w, target_h, options.resample.filter());
            sharpen(resized, options.resample)
        }
        SizeMode::Crop { anchor, .. } => {
            // Crop at the source resolution, then scale the window that
            // remains. Resizing first would shrink the whole image and then
            // "crop" the already-small result, which loses the detail the
            // anchor was supposed to choose.
            let source_aspect = image.width() as f32 / image.height() as f32;
            let target_aspect = target_w as f32 / target_h as f32;
            let (crop_x, crop_y, crop_w, crop_h) = if source_aspect > target_aspect {
                let crop_w = (image.height() as f32 * target_aspect)
                    .round()
                    .clamp(1., image.width() as f32) as u32;
                let free = image.width().saturating_sub(crop_w);
                (
                    (free as f32 * anchor.x()).round() as u32,
                    0,
                    crop_w,
                    image.height(),
                )
            } else {
                let crop_h = (image.width() as f32 / target_aspect)
                    .round()
                    .clamp(1., image.height() as f32) as u32;
                let free = image.height().saturating_sub(crop_h);
                (
                    0,
                    (free as f32 * anchor.y()).round() as u32,
                    image.width(),
                    crop_h,
                )
            };
            let cropped =
                image::imageops::crop_imm(image, crop_x, crop_y, crop_w, crop_h).to_image();
            let cropped = DynamicImage::ImageRgba8(cropped);
            let resized = cropped.resize_exact(target_w, target_h, options.resample.filter());
            sharpen(resized, options.resample)
        }
    }
}

fn sharpen(image: DynamicImage, resample: Resample) -> DynamicImage {
    if resample != Resample::SmoothSharpen {
        return image;
    }
    let rgba = image.to_rgba8();
    DynamicImage::ImageRgba8(image::imageops::unsharpen(&rgba, 1.0, 1))
}

/// Encode an already-final image.
pub fn encode(
    image: &DynamicImage,
    format: BatchFormat,
    options: &BatchOptions,
) -> Result<Vec<u8>, BatchError> {
    let quality = options.quality.clamp(1, 100);
    let mut out = Vec::new();

    match format {
        BatchFormat::Jpeg => {
            let rgb = flatten(image, options.background);
            JpegEncoder::new_with_quality(&mut out, quality)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Png => {
            let rgba = image.to_rgba8();
            PngEncoder::new(&mut out)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::WebP => {
            // `image`'s WebP encoder is lossless; quality is reported as
            // inapplicable by the window rather than silently ignored.
            let rgba = image.to_rgba8();
            WebPEncoder::new_lossless(&mut out)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Avif => {
            let rgba = image.to_rgba8();
            AvifEncoder::new_with_speed_quality(&mut out, 6, quality)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Gif => {
            let rgba = image.to_rgba8();
            GifEncoder::new(&mut out)
                .encode(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Bmp => {
            let rgb = flatten(image, options.background);
            image::codecs::bmp::BmpEncoder::new(&mut out)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Tiff => {
            let rgba = image.to_rgba8();
            TiffEncoder::new(Cursor::new(&mut out))
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Jxl => {
            let rgba = image.to_rgba8();
            let options = zune_core::options::EncoderOptions::new(
                rgba.width() as usize,
                rgba.height() as usize,
                zune_core::colorspace::ColorSpace::RGBA,
                zune_core::bit_depth::BitDepth::Eight,
            );
            let encoder = zune_jpegxl::JxlSimpleEncoder::new(rgba.as_raw(), options);
            encoder
                .encode(&mut out)
                .map_err(|error| BatchError::Encode(error.to_string()))?;
        }
        BatchFormat::Keep | BatchFormat::Pdf => unreachable!("resolved before encoding"),
    }

    Ok(out)
}

fn flatten(image: &DynamicImage, background: [u8; 3]) -> image::RgbImage {
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let mut out = image::RgbImage::new(width, height);

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u32;
        let blend = |channel: u8, base: u8| {
            ((channel as u32 * alpha + base as u32 * (255 - alpha)) / 255).min(255) as u8
        };
        out.put_pixel(
            x,
            y,
            image::Rgb([
                blend(pixel[0], background[0]),
                blend(pixel[1], background[1]),
                blend(pixel[2], background[2]),
            ]),
        );
    }
    out
}

fn convert_to_pdf(bytes: &[u8], options: &BatchOptions) -> Result<Conversion, BatchError> {
    let decoded = crate::decode::decode(bytes, options.auto_orient)
        .map_err(|error| BatchError::Decode(error.to_string()))?;
    let image = crate::decode::to_dynamic(&decoded)
        .ok_or_else(|| BatchError::Decode("The decoded image is empty".to_string()))?;
    let had_alpha = image.color().has_alpha();
    let image = apply_geometry(&image, options);
    let (width, height) = (image.width(), image.height());
    let rgb = flatten(&image, options.background);

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, options.quality.clamp(1, 100))
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| BatchError::Encode(error.to_string()))?;

    Ok(Conversion {
        bytes: wrap_jpeg_in_pdf(&jpeg, width, height),
        width,
        height,
        format: BatchFormat::Pdf,
        had_alpha,
    })
}

/// A minimal single-page PDF with one DCT (JPEG) image on it.
fn wrap_jpeg_in_pdf(jpeg: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let mut object = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(body);
    };

    object(
        &mut out,
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    );
    object(
        &mut out,
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
    );
    let page = format!(
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
         /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>\nendobj\n"
    );
    object(&mut out, page.as_bytes());

    offsets.push(out.len());
    out.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            jpeg.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(jpeg);
    out.extend_from_slice(b"\nendstream\nendobj\n");

    let content = format!("q\n{width} 0 0 {height} 0 0 cm\n/Im0 Do\nQ\n");
    offsets.push(out.len());
    out.extend_from_slice(format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes());
    out.extend_from_slice(content.as_bytes());
    out.extend_from_slice(b"endstream\nendobj\n");

    let xref = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// The raw TIFF block of a source's EXIF, with GPS removed when asked.
pub fn source_exif(bytes: &[u8], strip_location: bool) -> Option<Vec<u8>> {
    let raw = if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_segments(bytes)
            .into_iter()
            .find(|(marker, payload)| *marker == 0xE1 && payload.starts_with(b"Exif\0\0"))
            .map(|(_, payload)| payload[6..].to_vec())
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_chunks(bytes).and_then(|chunks| {
            chunks
                .into_iter()
                .find(|chunk| chunk.kind == *b"eXIf")
                .map(|chunk| chunk.data)
        })
    } else if bytes.starts_with(b"II") || bytes.starts_with(b"MM") {
        Some(bytes.to_vec())
    } else {
        None
    }?;

    if !strip_location {
        return Some(raw);
    }
    // If the block cannot be rebuilt, drop it rather than write the location
    // the user asked to remove.
    filter_exif(&raw)
}

/// Rebuild an EXIF block without the GPS IFD.
fn filter_exif(raw: &[u8]) -> Option<Vec<u8>> {
    use exif::{Context, In};

    let reader = exif::Reader::new();
    let exif = reader.read_raw(raw.to_vec()).ok()?;
    let mut writer = exif::experimental::Writer::new();
    for field in exif.fields() {
        if field.tag.context() == Context::Gps {
            continue;
        }
        // A thumbnail IFD's offsets point into the original file; copying the
        // fields without the thumbnail itself would corrupt them.
        if field.ifd_num != In::PRIMARY {
            continue;
        }
        writer.push_field(field);
    }
    let mut out = Cursor::new(Vec::new());
    writer.write(&mut out, exif.little_endian()).ok()?;
    let bytes = out.into_inner();
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// Insert an EXIF block into an encoded image.
pub fn inject_exif(
    encoded: Vec<u8>,
    format: BatchFormat,
    exif: &[u8],
) -> Result<Vec<u8>, BatchError> {
    match format {
        BatchFormat::Jpeg => Ok(insert_jpeg_segment(&encoded, 0xE1, &exif_payload(exif))),
        BatchFormat::Png => Ok(insert_png_chunk(&encoded, b"eXIf", exif)),
        // The remaining encoders have no compatible metadata slot in this
        // dependency set; the image itself is still correct.
        _ => Ok(encoded),
    }
}

fn exif_payload(exif: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(exif.len() + 6);
    payload.extend_from_slice(b"Exif\0\0");
    payload.extend_from_slice(exif);
    payload
}

/// Copy IPTC/XMP/ICC blocks from a JPEG source into the JPEG output.
fn copy_jpeg_metadata(source: &[u8], mut output: Vec<u8>, options: &BatchOptions) -> Vec<u8> {
    if !source.starts_with(&[0xFF, 0xD8]) {
        return output;
    }
    for (marker, payload) in jpeg_segments(source) {
        let wanted = match marker {
            0xE1 if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => options.keep_iptc,
            0xE1 if payload.starts_with(b"Exif\0\0") => false,
            0xE2 if payload.starts_with(b"ICC_PROFILE\0") => options.keep_color_profile,
            0xED => options.keep_iptc,
            _ => false,
        };
        if wanted {
            output = insert_jpeg_segment(&output, marker, &payload);
        }
    }
    output
}

/// Copy eXIf/iCCP/XMP blocks from a PNG source into the PNG output.
fn copy_png_metadata(source: &[u8], mut output: Vec<u8>, options: &BatchOptions) -> Vec<u8> {
    let Some(chunks) = png_chunks(source) else {
        return output;
    };
    for chunk in chunks {
        let wanted = match &chunk.kind {
            b"iCCP" => options.keep_color_profile,
            b"iTXt" => options.keep_iptc && chunk.data.starts_with(b"XML:com.adobe.xmp\0"),
            b"eXIf" => false,
            _ => false,
        };
        if wanted {
            output = insert_png_chunk(&output, &chunk.kind, &chunk.data);
        }
    }
    output
}

/// JPEG segments that can be copied, as `(marker, payload)`.
fn jpeg_segments(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut segments = Vec::new();
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return segments;
    }
    let mut position = 2;
    while position + 4 <= bytes.len() {
        if bytes[position] != 0xFF {
            break;
        }
        let mut marker_at = position + 1;
        while marker_at < bytes.len() && bytes[marker_at] == 0xFF {
            marker_at += 1;
        }
        if marker_at >= bytes.len() {
            break;
        }
        let marker = bytes[marker_at];
        if marker == 0xD8 || marker == 0xD9 || marker == 0xDA {
            break;
        }
        // Markers without a length field.
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            position = marker_at + 1;
            continue;
        }
        if marker_at + 2 >= bytes.len() {
            break;
        }
        let length = u16::from_be_bytes([bytes[marker_at + 1], bytes[marker_at + 2]]) as usize;
        if length < 2 || marker_at + 1 + length > bytes.len() {
            break;
        }
        let payload = bytes[marker_at + 3..marker_at + 1 + length].to_vec();
        if (0xE0..=0xEF).contains(&marker) {
            segments.push((marker, payload));
        }
        position = marker_at + 1 + length;
    }
    segments
}

/// Insert a segment immediately after the JPEG's SOI marker.
fn insert_jpeg_segment(jpeg: &[u8], marker: u8, payload: &[u8]) -> Vec<u8> {
    if !jpeg.starts_with(&[0xFF, 0xD8]) || payload.len() + 2 > u16::MAX as usize {
        return jpeg.to_vec();
    }
    let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
    out.extend_from_slice(&[0xFF, 0xD8]);
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

struct PngChunk {
    kind: [u8; 4],
    data: Vec<u8>,
    /// Offset of the length field of this chunk in the source.
    offset: usize,
}

fn png_chunks(bytes: &[u8]) -> Option<Vec<PngChunk>> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut chunks = Vec::new();
    let mut position = 8;
    while position + 12 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[position],
            bytes[position + 1],
            bytes[position + 2],
            bytes[position + 3],
        ]) as usize;
        if position + 12 + length > bytes.len() {
            return None;
        }
        let kind = [
            bytes[position + 4],
            bytes[position + 5],
            bytes[position + 6],
            bytes[position + 7],
        ];
        let data = bytes[position + 8..position + 8 + length].to_vec();
        chunks.push(PngChunk {
            kind,
            data,
            offset: position,
        });
        position += 12 + length;
        if &kind == b"IEND" {
            break;
        }
    }
    Some(chunks)
}

/// Insert a PNG chunk before IEND.
fn insert_png_chunk(png: &[u8], kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let Some(chunks) = png_chunks(png) else {
        return png.to_vec();
    };
    let Some(iend) = chunks.iter().find(|chunk| &chunk.kind == b"IEND") else {
        return png.to_vec();
    };
    let mut out = Vec::with_capacity(png.len() + data.len() + 16);
    out.extend_from_slice(&png[..iend.offset]);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&kind[..], data);
    out.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&png[iend.offset..]);
    out
}

fn crc32(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in kind.iter().chain(data.iter()) {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A rough output size for the preview, in bytes.
///
/// Encoders differ by content far more than this model can capture; the window
/// presents the result with a `~` for that reason.
pub fn estimate_bytes(width: u32, height: u32, format: BatchFormat, quality: u8) -> u64 {
    let pixels = width.max(1) as f64 * height.max(1) as f64;
    let quality = quality.clamp(1, 100) as f64 / 80.;
    let bytes_per_pixel = match format {
        BatchFormat::Jpeg | BatchFormat::Pdf => 0.25 * quality.powf(1.4),
        BatchFormat::WebP => 0.35,
        BatchFormat::Avif => 0.08 * quality.powf(1.2),
        BatchFormat::Png => 0.70,
        BatchFormat::Gif => 0.35,
        BatchFormat::Bmp => 3.0,
        BatchFormat::Tiff => 1.2,
        BatchFormat::Jxl => 0.50,
        BatchFormat::Keep => 0.30,
    };
    (pixels * bytes_per_pixel).max(64.) as u64
}

/// The two images and the facts the preview panel shows.
pub struct Preview {
    /// RGBA pixels at the size they should be drawn.
    pub before: (u32, u32, Vec<u8>),
    /// RGBA pixels at the size they should be drawn, using the same scale as
    /// `before` so the relative difference is visible.
    pub after: (u32, u32, Vec<u8>),
    pub output_width: u32,
    pub output_height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub had_alpha: bool,
    pub estimated_bytes: u64,
}

/// Decode one source and produce the two preview images.
///
/// The two images share one scale factor: scaling each into its own box would
/// make 6000×4000 and 1600×1067 look the same size and turn the comparison into
/// decoration.
pub fn preview(
    bytes: &[u8],
    options: &BatchOptions,
    max_width: u32,
    max_height: u32,
) -> Result<Preview, BatchError> {
    let decoded = crate::decode::decode(bytes, options.auto_orient)
        .map_err(|error| BatchError::Decode(error.to_string()))?;
    let image = crate::decode::to_dynamic(&decoded)
        .ok_or_else(|| BatchError::Decode("The decoded image is empty".to_string()))?;
    let source_width = image.width();
    let source_height = image.height();
    let had_alpha = image.color().has_alpha();
    let output = apply_geometry(&image, options);
    let output_width = output.width();
    let output_height = output.height();

    let largest_width = source_width.max(output_width).max(1) as f32;
    let largest_height = source_height.max(output_height).max(1) as f32;
    let scale = (max_width.max(1) as f32 / largest_width)
        .min(max_height.max(1) as f32 / largest_height)
        .min(4.0);

    let before_w = (source_width as f32 * scale).round().max(1.) as u32;
    let before_h = (source_height as f32 * scale).round().max(1.) as u32;
    let after_w = (output_width as f32 * scale).round().max(1.) as u32;
    let after_h = (output_height as f32 * scale).round().max(1.) as u32;

    let before = image
        .resize_exact(before_w, before_h, FilterType::Triangle)
        .to_rgba8();
    let after = output
        .resize_exact(after_w, after_h, FilterType::Triangle)
        .to_rgba8();

    Ok(Preview {
        before: (before_w, before_h, before.into_raw()),
        after: (after_w, after_h, after.into_raw()),
        output_width,
        output_height,
        source_width,
        source_height,
        had_alpha,
        estimated_bytes: estimate_bytes(
            output_width,
            output_height,
            resolved_format(options, ""),
            options.quality,
        ),
    })
}

fn resolved_format(options: &BatchOptions, source_extension: &str) -> BatchFormat {
    match options.format {
        BatchFormat::Keep => {
            BatchFormat::from_extension(source_extension).unwrap_or(BatchFormat::Png)
        }
        other => other,
    }
}

/// Write an encoded file atomically: a sibling temporary file is fully written
/// and then renamed over the target, so a crash never leaves a half image.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), BatchError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| BatchError::Write(format!("{}: {error}", parent.display())))?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    let temp = parent.join(format!(".{name}.{}.part", unique_suffix()));

    std::fs::write(&temp, bytes)
        .map_err(|error| BatchError::Write(format!("{}: {error}", temp.display())))?;

    // `rename` does not replace on Windows, so an overwrite removes first.
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(BatchError::Write(format!("{}: {error}", path.display())));
    }
    Ok(())
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    time ^ (count << 32) ^ std::process::id() as u64
}

/// Give a freshly written output the source's modification time.
pub fn copy_timestamp(source: &Path, target: &Path) {
    let Ok(metadata) = std::fs::metadata(source) else {
        return;
    };
    let Ok(modified) = metadata.modified() else {
        return;
    };
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(target) else {
        return;
    };
    let _ = file.set_times(std::fs::FileTimes::new().set_modified(modified));
}

/// Which output paths already exist, for the overwrite confirmation.
pub fn scan_conflicts(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .iter()
        .filter(|path| path.exists())
        .filter(|path| seen.insert((*path).clone()))
        .cloned()
        .collect()
}

/// How long a retry waits before the next attempt.
pub fn retry_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_millis(200),
        _ => Duration::from_millis(800),
    }
}

/// `1.4 MB`, for the report and the preview.
pub fn human_size(bytes: u64) -> String {
    crate::metadata::human_size(bytes)
}

/// A calendar date, without pulling a date library into the core.
struct Date {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl Date {
    fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let days = (seconds / 86_400) as i64;
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: ((seconds / 3_600) % 24) as u32,
            minute: ((seconds / 60) % 60) as u32,
            second: (seconds % 60) as u32,
        }
    }

    fn ymd(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    fn y_m_d(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    fn ymd_hms(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Howard Hinnant's days-to-civil algorithm, for the `{date}` token.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

/// Write the CSV report.
pub fn report_csv(rows: &[(String, String, String, String, String)]) -> String {
    let mut out = String::from("\u{FEFF}name,status,code,message,output\n");
    for (name, status, code, message, output) in rows {
        out.push_str(&csv_field(name));
        out.push(',');
        out.push_str(&csv_field(status));
        out.push(',');
        out.push_str(&csv_field(code));
        out.push(',');
        out.push_str(&csv_field(message));
        out.push(',');
        out.push_str(&csv_field(output));
        out.push('\n');
    }
    out
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Write a small text file next to the output, for reports.
pub fn write_text(path: &Path, text: &str) -> Result<(), BatchError> {
    std::fs::write(path, text.as_bytes())
        .map_err(|error| BatchError::Write(format!("{}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn png(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, image::Rgba(colour));
        let mut out = Cursor::new(Vec::new());
        image
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode png");
        out.into_inner()
    }

    fn size(width: f32, height: f32) -> Size {
        Size::new(width, height)
    }

    #[test]
    fn the_defaults_are_the_promised_ones() {
        let options = BatchOptions::default();
        assert_eq!(options.size, SizeMode::LongEdge(1600));
        assert!(options.only_shrink);
        assert_eq!(options.format, BatchFormat::WebP);
        assert_eq!(options.quality, 80);
        assert_eq!(options.conflict, ConflictPolicy::Rename);
        assert!(options.strip_location);
        assert_eq!(options.template, "{name}_{w}x{h}");
    }

    #[test]
    fn a_long_edge_shrinks_landscape_and_portrait_alike() {
        assert_eq!(
            target_size(size(6000., 4000.), SizeMode::LongEdge(1600), true),
            size(1600., 1067.)
        );
        assert_eq!(
            target_size(size(4000., 6000.), SizeMode::LongEdge(1600), true),
            size(1067., 1600.)
        );
    }

    #[test]
    fn only_shrink_keeps_a_small_image_small() {
        assert_eq!(
            target_size(size(800., 600.), SizeMode::LongEdge(1600), true),
            size(800., 600.)
        );
        assert_eq!(
            target_size(size(800., 600.), SizeMode::LongEdge(1600), false),
            size(1600., 1200.)
        );
    }

    #[test]
    fn a_crop_never_invents_pixels() {
        assert_eq!(
            target_size(
                size(800., 600.),
                SizeMode::Crop {
                    width: 1080,
                    height: 1080,
                    anchor: Anchor::Center
                },
                true
            ),
            size(800., 600.)
        );
        assert_eq!(
            target_size(
                size(1600., 800.),
                SizeMode::Crop {
                    width: 1080,
                    height: 1080,
                    anchor: Anchor::Center
                },
                false
            ),
            size(1080., 1080.)
        );
    }

    #[test]
    fn a_template_expands_every_token() {
        let name = render_name("{name}_{w}x{h}", "DSC_0142.JPG", 1600, 1067, 1);
        assert_eq!(name, "DSC_0142_1600x1067");

        let name = render_name("shot-{seq:3}-{name_ext}", "a b.png", 10, 20, 7);
        assert_eq!(name, "shot-007-a b.png");
    }

    #[test]
    fn an_empty_template_falls_back_to_the_source_name() {
        assert_eq!(render_name("", "a.png", 10, 10, 1), "a");
    }

    #[test]
    fn illegal_characters_are_replaced_and_windows_rules_are_respected() {
        assert_eq!(sanitize_name("a/b:c*?\"<>|d"), "a_b_c______d");
        assert_eq!(sanitize_name("trailing..."), "trailing");
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("   "), "image");
    }

    #[test]
    fn output_names_take_the_format_extension() {
        let options = BatchOptions::default();
        assert_eq!(
            output_file_name(&options, "photo.JPG", 100, 50, 1),
            "photo_100x50.webp"
        );
        let options = BatchOptions {
            format: BatchFormat::Keep,
            ..Default::default()
        };
        assert_eq!(
            output_file_name(&options, "photo.JPG", 100, 50, 1),
            "photo_100x50.jpg"
        );
    }

    #[test]
    fn the_default_placement_writes_an_output_subfolder() {
        let options = BatchOptions::default();
        let folder = output_folder(Path::new("C:/pics/a.png"), None, &options, None).unwrap();
        assert_eq!(folder, PathBuf::from("C:/pics/output"));
    }

    #[test]
    fn mirroring_keeps_the_relative_folders() {
        let options = BatchOptions {
            placement: OutputPlacement::Mirror,
            output_dir: Some(PathBuf::from("C:/out")),
            ..Default::default()
        };
        let folder = output_folder(
            Path::new("C:/pics/a.png"),
            Some(Path::new("sub/deep/a.png")),
            &options,
            None,
        )
        .unwrap();
        assert_eq!(folder, PathBuf::from("C:/out/sub/deep"));
    }

    #[test]
    fn a_custom_placement_without_a_folder_is_an_error() {
        let options = BatchOptions {
            placement: OutputPlacement::Custom,
            output_dir: None,
            ..Default::default()
        };
        assert_eq!(
            output_folder(Path::new("C:/pics/a.png"), None, &options, None),
            Err(BatchError::NoOutputDir)
        );
    }

    #[test]
    fn rename_finds_the_next_free_number() {
        let dir = std::env::temp_dir().join(format!("seepic-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wanted = dir.join("page.jpg");
        std::fs::write(&wanted, b"x").unwrap();
        assert_eq!(free_rename_path(&wanted), dir.join("page (2).jpg"));
        std::fs::write(dir.join("page (2).jpg"), b"x").unwrap();
        assert_eq!(free_rename_path(&wanted), dir.join("page (3).jpg"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_png_converts_to_jpeg_and_flattens_alpha() {
        let source = png(8, 8, [255, 0, 0, 0]);
        let options = BatchOptions {
            size: SizeMode::Original,
            format: BatchFormat::Jpeg,
            ..Default::default()
        };
        let converted = convert(&source, "a.png", &options).expect("convert");
        assert_eq!(converted.format, BatchFormat::Jpeg);
        assert!(converted.had_alpha);
        assert_eq!(converted.bytes[0..2], [0xFF, 0xD8]);
        assert_eq!((converted.width, converted.height), (8, 8));
    }

    #[test]
    fn every_output_format_can_be_written() {
        let source = png(16, 16, [10, 20, 30, 128]);
        for format in [
            BatchFormat::Jpeg,
            BatchFormat::Png,
            BatchFormat::WebP,
            BatchFormat::Avif,
            BatchFormat::Gif,
            BatchFormat::Bmp,
            BatchFormat::Tiff,
            BatchFormat::Jxl,
            BatchFormat::Pdf,
        ] {
            let options = BatchOptions {
                size: SizeMode::Original,
                format,
                ..Default::default()
            };
            let converted = convert(&source, "a.png", &options)
                .unwrap_or_else(|error| panic!("{format:?} failed: {error}"));
            assert!(!converted.bytes.is_empty(), "{format:?} produced nothing");
            if format == BatchFormat::Pdf {
                assert!(converted.bytes.starts_with(b"%PDF-1.4"));
            } else {
                let decoded = crate::decode::decode(&converted.bytes, false)
                    .unwrap_or_else(|error| panic!("{format:?} did not decode: {error}"));
                assert_eq!(
                    (decoded.width, decoded.height),
                    (16, 16),
                    "{format:?} changed the size"
                );
            }
        }
    }

    #[test]
    fn avif_and_jxl_keep_the_alpha_channel() {
        let source = png(8, 8, [10, 20, 30, 128]);
        for format in [BatchFormat::Avif, BatchFormat::Jxl] {
            let options = BatchOptions {
                size: SizeMode::Original,
                format,
                ..Default::default()
            };
            let converted = convert(&source, "a.png", &options).expect("convert");
            let decoded = crate::decode::decode(&converted.bytes, false)
                .unwrap_or_else(|error| panic!("{format:?} did not decode: {error}"));
            assert_eq!(
                decoded.frames[0].bgra[3], 128,
                "{format:?} dropped the alpha channel"
            );
        }
    }

    #[test]
    fn avif_and_jxl_probe_their_headers() {
        let source = png(32, 20, [10, 20, 30, 255]);
        for format in [BatchFormat::Avif, BatchFormat::Jxl] {
            let options = BatchOptions {
                size: SizeMode::Original,
                format,
                ..Default::default()
            };
            let converted = convert(&source, "a.png", &options).expect("convert");
            let size = crate::decode::probe_dimensions(&converted.bytes)
                .unwrap_or_else(|| panic!("{format:?} header probe failed"));
            assert_eq!(
                (size.w.round() as u32, size.h.round() as u32),
                (32, 20),
                "{format:?} reported the wrong size"
            );
        }
    }

    #[test]
    fn a_crop_anchor_moves_the_window() {
        let mut pixels = RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 255]));
        for x in 0..10 {
            for y in 0..10 {
                pixels.put_pixel(x, y, image::Rgba([x as u8 * 20, y as u8 * 20, 0, 255]));
            }
        }
        let image = image::DynamicImage::ImageRgba8(pixels);
        let options = BatchOptions {
            size: SizeMode::Crop {
                width: 5,
                height: 6,
                anchor: Anchor::BottomRight,
            },
            only_shrink: false,
            ..Default::default()
        };
        let cropped = apply_geometry(&image, &options);
        assert_eq!((cropped.width(), cropped.height()), (5, 6));
        // A 5x6 window out of a 10x10 source leaves only a little vertical
        // freedom; the bottom anchor takes it from the bottom, so the first
        // row is already below the top edge.
        let pixel = cropped.to_rgba8().get_pixel(0, 0).0;
        assert!(
            pixel[1] > 0,
            "the crop should not start at the top: {pixel:?}"
        );
    }

    #[test]
    fn pdf_is_a_single_page_with_the_jpeg_inside() {
        let pdf = wrap_jpeg_in_pdf(b"\xFF\xD8fake\xFF\xD9", 100, 50);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        assert!(pdf.windows(4).any(|window| window == b"/DCT"));
    }

    #[test]
    fn the_preview_keeps_both_images_in_one_scale() {
        let source = png(400, 200, [10, 20, 30, 255]);
        let options = BatchOptions {
            size: SizeMode::LongEdge(100),
            ..Default::default()
        };
        let preview = preview(&source, &options, 500, 300).expect("preview");
        assert_eq!((preview.source_width, preview.source_height), (400, 200));
        assert_eq!((preview.output_width, preview.output_height), (100, 50));
        // The original is drawn four times as wide as the output.
        assert_eq!(preview.before.0, preview.after.0 * 4);
    }

    #[test]
    fn estimates_scale_with_pixels_and_quality() {
        let small = estimate_bytes(100, 100, BatchFormat::Jpeg, 80);
        let big = estimate_bytes(1000, 1000, BatchFormat::Jpeg, 80);
        assert!(big > small * 50);
        let low = estimate_bytes(1000, 1000, BatchFormat::Jpeg, 20);
        let high = estimate_bytes(1000, 1000, BatchFormat::Jpeg, 95);
        assert!(high > low);
    }

    #[test]
    fn csv_fields_are_quoted_when_needed() {
        let csv = report_csv(&[(
            "a,b".to_string(),
            "失败".to_string(),
            "E01".to_string(),
            "say \"no\"".to_string(),
            "C:/out/x.png".to_string(),
        )]);
        assert!(csv.contains("\"a,b\""));
        assert!(csv.contains("\"say \"\"no\"\"\""));
    }
}
