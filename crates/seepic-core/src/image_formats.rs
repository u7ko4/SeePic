//! Which file extensions we treat as viewable images.

/// Extensions we can decode, without the leading dot and lowercased.
///
/// Kept in sync with the `image` crate features enabled in `Cargo.toml`.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "dib", "ico", "tif", "tiff",
    "tga", "pnm", "pbm", "pgm", "ppm", "pam", "dds", "hdr", "exr", "qoi", "ff", "farbfeld",
];

/// Extensions that hold other images inside them.
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "cbz", "7z", "cb7", "rar", "cbr", "tar", "cbt",
];

/// True when `name` looks like something we can display directly.
pub fn is_image_extension(name: &str) -> bool {
    extension_of(name).is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// True when `name` looks like an archive we can browse into.
pub fn is_archive_extension(name: &str) -> bool {
    extension_of(name).is_some_and(|ext| ARCHIVE_EXTENSIONS.contains(&ext.as_str()))
}

/// Lowercased extension without the dot, or `None` when there is none.
pub fn extension_of(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_plain_images() {
        assert!(is_image_extension("photo.JPG"));
        assert!(is_image_extension("a.b.png"));
        assert!(!is_image_extension("notes.txt"));
        assert!(!is_image_extension("no_extension"));
        assert!(!is_image_extension("trailing."));
    }

    #[test]
    fn recognises_archives() {
        assert!(is_archive_extension("book.cbz"));
        assert!(is_archive_extension("BOOK.CBR"));
        assert!(is_archive_extension("pack.zip"));
        assert!(!is_archive_extension("pack.rar.txt"));
    }
}
