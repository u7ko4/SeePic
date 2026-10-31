//! Per-format icons for the file-association page.
//!
//! The icons are embedded, so the page works the same on a machine that has
//! never seen the viewer they were taken from. The same `.ico` bytes are
//! written to the app data folder for Explorer when a format is registered,
//! because `DefaultIcon` needs a file on disk.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use gpui_kit::RenderImage;
use seepic_core::settings::Settings;

/// The size list icons are decoded at, in pixels.
const ICON_PIXELS: u32 = 64;

/// Extension → embedded icon. Each file carries the usual ICO sizes, so the
/// same bytes serve both the settings list and Explorer's `DefaultIcon`.
macro_rules! icon_assets {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../../assets/formats/", $name, ".ico")))),*]
    };
}

const ICON_ASSETS: &[(&str, &[u8])] = icon_assets![
    "7z", "apng", "arw", "avif", "bmp", "cbr", "cb7", "cbt", "cbz", "clip", "cr2", "cr3",
    "dds", "dib", "dng", "exr", "ff", "farbfeld", "gif", "hdr", "heic", "heif", "ico", "jfif",
    "jpe", "jpeg", "jpg", "nef", "nrw", "orf", "pam", "pbm", "pgm", "png", "pnm", "ppm",
    "psb", "psd", "qoi", "rar", "svg", "tar", "tga", "tif", "tiff", "webp", "zip",
];

/// The embedded icon that stands for `extension`.
///
/// A dedicated icon wins; the aliases are a fallback for extensions whose
/// format family has an icon under another name. `None` means the list falls
/// back to the generic file-image glyph.
pub fn canonical(extension: &str) -> Option<&'static str> {
    let wanted = if ICON_ASSETS.iter().any(|(name, _)| *name == extension) {
        extension
    } else {
        match extension {
            "jpe" | "jfif" => "jpeg",
            "dib" => "bmp",
            "apng" => "png",
            "cb7" | "cbt" => "cbz",
            other => other,
        }
    };
    ICON_ASSETS
        .iter()
        .find(|(name, _)| *name == wanted)
        .map(|(name, _)| *name)
}

/// The drawable icon for `extension`, decoded on first use and shared.
///
/// Decoding is lazy: the settings list only pulls the icons it actually draws,
/// and the per-frame cost afterwards is a hash lookup.
pub fn render(extension: &str) -> Option<Arc<RenderImage>> {
    let key = canonical(extension)?;
    let mut cache = cache().lock().ok()?;
    if let Some(icon) = cache.get(key) {
        return Some(icon.clone());
    }
    let (_, bytes) = ICON_ASSETS.iter().find(|(name, _)| *name == key)?;
    let icon = decode(bytes)?;
    cache.insert(key, icon.clone());
    Some(icon)
}

/// Build a list-sized image from the largest frame of the ICO.
fn decode(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image
        .resize_exact(
            ICON_PIXELS,
            ICON_PIXELS,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgba8();
    let frame = image::Frame::from_parts(rgba, 0, 0, image::Delay::from_numer_denom_ms(100, 1));
    Some(Arc::new(RenderImage::new(vec![frame])))
}

fn cache() -> &'static Mutex<HashMap<&'static str, Arc<RenderImage>>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Arc<RenderImage>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Write the `.ico` copy of `extension`'s icon into the app's data folder and
/// return its path, for the ProgID's `DefaultIcon`.
///
/// A missing icon is not fatal: the caller falls back to the executable's own
/// icon. The file is only rewritten when the embedded copy changes, so
/// re-registering a format does not churn the disk.
pub fn ensure_icon_file(extension: &str) -> Option<PathBuf> {
    let key = canonical(extension)?;
    let (_, ico) = ICON_ASSETS.iter().find(|(name, _)| *name == key)?;
    let folder = Settings::data_dir().join("assoc-icons");
    let path = folder.join(format!("{key}.ico"));
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == ico.len() as u64) {
        return Some(path);
    }
    std::fs::create_dir_all(&folder).ok()?;
    std::fs::write(&path, ico).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_icons_and_fallbacks() {
        assert_eq!(canonical("jpe"), Some("jpe"));
        assert_eq!(canonical("jfif"), Some("jfif"));
        assert_eq!(canonical("apng"), Some("apng"));
        assert_eq!(canonical("dib"), Some("dib"));
        assert_eq!(canonical("cb7"), Some("cb7"));
        assert_eq!(canonical("cbt"), Some("cbt"));
        assert_eq!(canonical("tar"), Some("tar"));
        assert_eq!(canonical("unknown"), None);
    }

    #[test]
    fn every_openable_format_has_an_icon() {
        for extension in seepic_core::image_formats::IMAGE_EXTENSIONS
            .iter()
            .chain(seepic_core::image_formats::ARCHIVE_EXTENSIONS)
        {
            assert!(
                canonical(extension).is_some(),
                ".{extension} can be opened but has no icon"
            );
        }
    }

    #[test]
    fn the_embedded_icons_decode() {
        for (name, _) in ICON_ASSETS {
            let icon = render(name).unwrap_or_else(|| panic!("{name} did not decode"));
            assert_eq!(icon.frame_count(), 1);
            assert_eq!(icon.size(0).width.0 as u32, ICON_PIXELS);
        }
    }
}
