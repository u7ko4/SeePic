//! Deleting and renaming files.
//!
//! Deletion goes to the recycle bin rather than being permanent, so a misclick
//! is recoverable. Names are validated here rather than relying on the OS error,
//! because "the filename is invalid" is more useful than a raw errno.

use std::path::{Path, PathBuf};

/// Why a rename was refused before touching the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenameError {
    #[error("文件名不能为空")]
    Empty,
    #[error("文件名没有变化")]
    Unchanged,
    #[error("文件名包含非法字符")]
    InvalidChar,
    #[error("这是 Windows 保留的文件名")]
    Reserved,
    #[error("已存在同名文件")]
    Exists,
    #[error("重命名失败")]
    Failed,
}

/// Characters Windows refuses in a file name.
const ILLEGAL: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Device names Windows reserves regardless of extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Check a proposed file name before attempting the rename.
pub fn validate_name(current: &str, proposed: &str) -> Result<String, RenameError> {
    let name = proposed.trim();

    if name.is_empty() {
        return Err(RenameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(RenameError::InvalidChar);
    }
    if name.chars().any(|c| ILLEGAL.contains(&c) || c == '\0') {
        return Err(RenameError::InvalidChar);
    }
    // A trailing dot or space is silently stripped by Windows, which would make
    // the rename appear to succeed with a different name than requested.
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(RenameError::InvalidChar);
    }
    if name == current {
        return Err(RenameError::Unchanged);
    }

    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(RenameError::Reserved);
    }

    Ok(name.to_string())
}

/// Rename `path` to `proposed`, returning the new path.
///
/// Refuses to overwrite an existing file rather than silently replacing it.
pub fn rename_file(path: &Path, proposed: &str) -> Result<PathBuf, RenameError> {
    let current = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = validate_name(&current, proposed)?;

    let parent = path.parent().unwrap_or(Path::new("."));
    let target = parent.join(&name);

    if target.exists() {
        return Err(RenameError::Exists);
    }

    std::fs::rename(path, &target).map_err(|_| RenameError::Failed)?;
    Ok(target)
}

/// Send `path` to the recycle bin.
///
/// Used for deletion so that the operation can be undone from the recycle bin.
pub fn delete_to_recycle_bin(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_an_ordinary_name() {
        assert_eq!(validate_name("a.png", "b.png"), Ok("b.png".to_string()));
        // Surrounding whitespace is trimmed rather than rejected.
        assert_eq!(validate_name("a.png", "  b.png  "), Ok("b.png".to_string()));
    }

    #[test]
    fn rejects_empty_and_unchanged() {
        assert_eq!(validate_name("a.png", ""), Err(RenameError::Empty));
        assert_eq!(validate_name("a.png", "   "), Err(RenameError::Empty));
        assert_eq!(validate_name("a.png", "a.png"), Err(RenameError::Unchanged));
    }

    #[test]
    fn rejects_path_separators_and_wildcards() {
        for bad in ["a/b.png", "a\\b.png", "a:b.png", "a*b.png", "a?b.png"] {
            assert_eq!(
                validate_name("x.png", bad),
                Err(RenameError::InvalidChar),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_dot_names_and_trailing_dot() {
        assert_eq!(validate_name("x.png", "."), Err(RenameError::InvalidChar));
        assert_eq!(validate_name("x.png", ".."), Err(RenameError::InvalidChar));
        assert_eq!(validate_name("x.png", "a."), Err(RenameError::InvalidChar));
        // A trailing space is trimmed rather than rejected, so this is fine.
        assert_eq!(validate_name("x.png", "a "), Ok("a".to_string()));
    }

    #[test]
    fn rejects_reserved_device_names() {
        assert_eq!(validate_name("x.png", "CON"), Err(RenameError::Reserved));
        assert_eq!(validate_name("x.png", "con.txt"), Err(RenameError::Reserved));
        assert_eq!(validate_name("x.png", "LPT1.png"), Err(RenameError::Reserved));
        // A name that merely starts with a reserved word is fine.
        assert!(validate_name("x.png", "console.txt").is_ok());
    }

    #[test]
    fn renames_on_disk() {
        let dir = std::env::temp_dir().join("iv-fileops-rename");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let from = dir.join("old.png");
        std::fs::write(&from, b"data").unwrap();

        let to = rename_file(&from, "new.png").expect("should rename");
        assert_eq!(to.file_name().unwrap(), "new.png");
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"data");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refuses_to_overwrite() {
        let dir = std::env::temp_dir().join("iv-fileops-overwrite");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let from = dir.join("a.png");
        let other = dir.join("b.png");
        std::fs::write(&from, b"a").unwrap();
        std::fs::write(&other, b"b").unwrap();

        assert_eq!(rename_file(&from, "b.png"), Err(RenameError::Exists));
        // The original is untouched.
        assert_eq!(std::fs::read(&from).unwrap(), b"a");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deleting_a_missing_file_reports_an_error() {
        let missing = std::env::temp_dir().join("iv-fileops-never-existed.png");
        std::fs::remove_file(&missing).ok();
        assert!(delete_to_recycle_bin(&missing).is_err());
    }
}
