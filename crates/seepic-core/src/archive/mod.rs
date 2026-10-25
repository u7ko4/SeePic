//! Reading images out of compressed files, so `CBZ`/`CBR`/`CB7`/`CBT` comics can
//! be browsed directly without extracting them first.
//!
//! Each format gets a backend behind one trait. Formats differ a lot in what
//! they allow: ZIP and 7z can seek straight to a member, while RAR and TAR are
//! sequential cursors that must be walked from the start. The trait hides that,
//! and each backend does whatever it must to keep the common case — stepping to
//! the next page — cheap.

mod rar_backend;
mod sevenz_backend;
mod tar_backend;
mod zip_backend;

use std::io::Read;
use std::path::Path;

pub use rar_backend::RarBackend;
pub use sevenz_backend::SevenZBackend;
pub use tar_backend::TarBackend;
pub use zip_backend::ZipBackend;

/// Why an archive could not be read.
///
/// The variants are phrased for the user, distinguishing the failures a reader
/// needs to tell apart.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("无法打开压缩包: {0}")]
    Open(String),
    #[error("压缩包已损坏: {0}")]
    Corrupt(String),
    #[error("不支持的压缩包版本: {0}")]
    Unsupported(String),
    #[error("此压缩包已加密，需要密码")]
    PasswordRequired,
    #[error("密码错误")]
    WrongPassword,
    #[error("压缩包内没有图片")]
    NoImages,
    #[error("压缩包为空")]
    Empty,
    #[error("读取压缩包条目失败: {0}")]
    Read(String),
}

/// A file inside an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Path inside the archive, using `/` separators.
    pub name: String,
    /// Uncompressed size in bytes, when the format reports it.
    pub size: u64,
    pub is_dir: bool,
    pub encrypted: bool,
}

/// The first bytes of a member, and whether that is the whole member.
#[derive(Debug)]
pub struct Prefix {
    pub bytes: Vec<u8>,
    /// True when `bytes` is everything the member holds, so a header parser
    /// that failed on it would fail on a full read too.
    pub complete: bool,
}

/// Which archive format a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
    Rar,
    Tar,
}

impl ArchiveFormat {
    pub fn label(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "ZIP",
            ArchiveFormat::SevenZ => "7Z",
            ArchiveFormat::Rar => "RAR",
            ArchiveFormat::Tar => "TAR",
        }
    }
}

/// Read and write images inside one archive.
pub trait ArchiveBackend: Send {
    /// Every member, in the archive's own order.
    fn members(&self) -> &[Member];

    /// Decompress one member fully into memory.
    fn read(&mut self, index: usize) -> Result<Vec<u8>, ArchiveError>;

    /// Read at most `max` bytes from the front of one member.
    ///
    /// The default reads the member whole and keeps the front, for the formats
    /// whose API has no partial read. ZIP and TAR override it: a page's header
    /// is a few hundred kilobytes at most, and the rest of a four-megabyte page
    /// never has to be decompressed just to learn its dimensions.
    fn read_prefix(&mut self, index: usize, max: usize) -> Result<Prefix, ArchiveError> {
        let complete = self
            .members()
            .get(index)
            .is_some_and(|member| member.size <= max as u64);
        let bytes = self.read(index)?;
        if complete {
            return Ok(Prefix { bytes, complete });
        }
        let mut bytes = bytes;
        bytes.truncate(max);
        Ok(Prefix {
            bytes,
            complete: false,
        })
    }

    /// Remember a password for subsequent reads.
    fn set_password(&mut self, password: &str);

    /// Whether any member is encrypted, so the UI can offer a password prompt
    /// before the first read fails.
    fn has_encrypted_members(&self) -> bool {
        self.members().iter().any(|m| m.encrypted)
    }
}

/// Sniff the format from the file's magic bytes.
///
/// Trusting the extension alone would mis-handle a `.cbz` that is really a RAR,
/// which happens often enough with renamed comic files to be worth the extra
/// read.
pub fn detect_format(path: &Path) -> Result<ArchiveFormat, ArchiveError> {
    let mut header = [0u8; 512];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut header))
        .map_err(|e| ArchiveError::Open(format!("{}: {e}", path.display())))?;
    let header = &header[..read];

    if header.starts_with(b"PK\x03\x04")
        || header.starts_with(b"PK\x05\x06")
        || header.starts_with(b"PK\x07\x08")
    {
        return Ok(ArchiveFormat::Zip);
    }
    // RAR5 before RAR4, since the shorter signature is a prefix of the longer.
    if header.starts_with(b"Rar!\x1a\x07\x01\x00") || header.starts_with(b"Rar!\x1a\x07\x00") {
        return Ok(ArchiveFormat::Rar);
    }
    if header.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Ok(ArchiveFormat::SevenZ);
    }
    // TAR has no leading magic; the marker sits in the first header block.
    if header.len() >= 262 && &header[257..262] == b"ustar" {
        return Ok(ArchiveFormat::Tar);
    }

    Err(ArchiveError::Unsupported(format!(
        "无法识别的压缩包格式: {}",
        path.display()
    )))
}

/// Open whichever backend matches `path`.
pub fn open(path: &Path) -> Result<Box<dyn ArchiveBackend>, ArchiveError> {
    match detect_format(path)? {
        ArchiveFormat::Zip => Ok(Box::new(ZipBackend::open(path)?)),
        ArchiveFormat::SevenZ => Ok(Box::new(SevenZBackend::open(path)?)),
        ArchiveFormat::Rar => Ok(Box::new(RarBackend::open(path)?)),
        ArchiveFormat::Tar => Ok(Box::new(TarBackend::open(path)?)),
    }
}

/// Normalise a member path: `/` separators, no leading `./` or `/`.
pub(crate) fn normalize_member_name(name: &str) -> String {
    let name = name.replace('\\', "/");
    let name = name.trim_start_matches("./").trim_start_matches('/');
    name.to_string()
}

/// True when a member path is worth showing as an image.
///
/// Directory entries and macOS resource-fork junk are filtered out here so the
/// navigation list only ever holds viewable images.
pub(crate) fn is_viewable_member(member: &Member) -> bool {
    if member.is_dir {
        return false;
    }
    let name = &member.name;
    // `__MACOSX/` and `.DS_Store` are packaging noise, not comic pages.
    if name.starts_with("__MACOSX/") || name.ends_with(".DS_Store") {
        return false;
    }
    if name.rsplit('/').next().is_some_and(|f| f.starts_with('.')) {
        return false;
    }
    crate::image_formats::is_image_extension(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str) -> Member {
        Member {
            name: name.to_string(),
            size: 0,
            is_dir: false,
            encrypted: false,
        }
    }

    #[test]
    fn normalises_separators_and_prefixes() {
        assert_eq!(normalize_member_name("a\\b\\c.png"), "a/b/c.png");
        assert_eq!(normalize_member_name("./page.png"), "page.png");
        assert_eq!(normalize_member_name("/page.png"), "page.png");
    }

    #[test]
    fn filters_non_images_and_packaging_noise() {
        assert!(is_viewable_member(&member("page_01.png")));
        assert!(is_viewable_member(&member("sub/dir/page.jpg")));

        assert!(!is_viewable_member(&member("readme.txt")));
        assert!(!is_viewable_member(&member("__MACOSX/page.png")));
        assert!(!is_viewable_member(&member(".DS_Store")));
        assert!(!is_viewable_member(&member("sub/.hidden.png")));
        assert!(!is_viewable_member(&Member {
            is_dir: true,
            ..member("folder.png")
        }));
    }

    #[test]
    fn detects_formats_from_magic_bytes() {
        let dir = std::env::temp_dir().join("iv-detect-test");
        std::fs::create_dir_all(&dir).unwrap();

        let cases: [(&str, &[u8], ArchiveFormat); 4] = [
            ("a.zip", b"PK\x03\x04rest", ArchiveFormat::Zip),
            ("b.cbr", b"Rar!\x1a\x07\x01\x00rest", ArchiveFormat::Rar),
            ("c.cb7", b"7z\xbc\xaf\x27\x1crest", ArchiveFormat::SevenZ),
            ("d.cbt", {
                let mut tar = vec![0u8; 512];
                tar[257..262].copy_from_slice(b"ustar");
                // Leaked so the array element can borrow it for the whole test.
                Box::leak(tar.into_boxed_slice())
            }, ArchiveFormat::Tar),
        ];

        for (name, bytes, want) in cases {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(detect_format(&path).unwrap(), want, "{name}");
        }

        // A file whose extension lies is still identified by content.
        let path = dir.join("misleading.cbz");
        std::fs::write(&path, b"Rar!\x1a\x07\x00rest").unwrap();
        assert_eq!(detect_format(&path).unwrap(), ArchiveFormat::Rar);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_non_archives() {
        let dir = std::env::temp_dir().join("iv-detect-test-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plain.zip");
        std::fs::write(&path, b"this is not an archive at all").unwrap();

        assert!(matches!(
            detect_format(&path),
            Err(ArchiveError::Unsupported(_))
        ));

        std::fs::remove_dir_all(&dir).ok();
    }
}
