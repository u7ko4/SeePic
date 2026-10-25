//! One abstraction over "a place images come from", so navigation, the canvas,
//! thumbnails and the slideshow never need to know whether they are looking at
//! a folder on disk or a comic archive.
//!
//! `[` and `]` step between "folders (archives)" as if they were the same kind
//! of thing.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::archive::{self, ArchiveBackend, ArchiveError, ArchiveFormat};
use crate::geom::Size;
use crate::image_formats::{is_archive_extension, is_image_extension};
use crate::library::Entry;

/// What a container is backed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    Dir,
    Archive(ArchiveFormat),
}

impl ContainerKind {
    pub fn label(self) -> &'static str {
        match self {
            ContainerKind::Dir => "文件夹",
            ContainerKind::Archive(format) => format.label(),
        }
    }

    pub fn is_archive(self) -> bool {
        matches!(self, ContainerKind::Archive(_))
    }
}

/// Identifies one image, enough to read it again later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    ArchiveMember { archive: PathBuf, member: usize },
}

/// Why a container could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error("{0}")]
    Io(String),
}

/// Where images are coming from, plus the ordered list of viewable images.
pub struct Container {
    kind: ContainerKind,
    path: PathBuf,
    entries: Vec<Entry>,
    /// Parallel to `entries`; for archives, the member index to read.
    members: Vec<usize>,
    backend: Backend,
}

enum Backend {
    Dir,
    Archive(Box<dyn ArchiveBackend>),
}

impl Container {
    /// Open a directory or an archive, whichever `path` is.
    pub fn open(path: &Path) -> Result<Self, ContainerError> {
        if path.is_dir() {
            Self::open_dir(path)
        } else {
            Self::open_archive(path)
        }
    }

    /// List the image files directly inside `dir`.
    pub fn open_dir(dir: &Path) -> Result<Self, ContainerError> {
        let read_dir = std::fs::read_dir(dir)
            .map_err(|e| ContainerError::Io(format!("{}: {e}", dir.display())))?;

        let mut entries = Vec::new();
        for item in read_dir {
            let Ok(item) = item else { continue };
            let file_name = item.file_name().to_string_lossy().into_owned();
            if !is_image_extension(&file_name) {
                continue;
            }
            // `metadata()` follows symlinks; failures just mean no date/size.
            let metadata = item.metadata().ok();
            let is_file = metadata.as_ref().is_none_or(|m| m.is_file());
            if !is_file {
                continue;
            }

            entries.push(Entry {
                name: file_name,
                size: metadata.as_ref().map_or(0, |m| m.len()),
                modified: metadata.as_ref().and_then(|m| m.modified().ok()),
                created: metadata.as_ref().and_then(|m| m.created().ok()),
                taken: None,
                dimensions: None,
            });
        }

        // The directory lists in read_dir order, which is arbitrary. Sorting
        // naturally here makes the container's own order the one a user
        // expects, so the position-to-source mapping starts out as the identity.
        entries.sort_by(|a, b| crate::natsort::natural_cmp(&a.name, &b.name));

        Ok(Self {
            kind: ContainerKind::Dir,
            path: dir.to_path_buf(),
            members: (0..entries.len()).collect(),
            entries,
            backend: Backend::Dir,
        })
    }

    /// List the viewable images inside an archive.
    pub fn open_archive(path: &Path) -> Result<Self, ContainerError> {
        let backend = archive::open(path)?;
        let format = archive::detect_format(path)?;

        let mut entries = Vec::new();
        let mut members = Vec::new();
        for (index, member) in backend.members().iter().enumerate() {
            if !archive::is_viewable_member(member) {
                continue;
            }
            entries.push(Entry {
                name: member.name.clone(),
                size: member.size,
                modified: None,
                created: None,
                taken: None,
                dimensions: None,
            });
            members.push(index);
        }

        if backend.members().is_empty() {
            return Err(ContainerError::Archive(ArchiveError::Empty));
        }
        if entries.is_empty() {
            return Err(ContainerError::Archive(ArchiveError::NoImages));
        }

        Ok(Self {
            kind: ContainerKind::Archive(format),
            path: path.to_path_buf(),
            entries,
            members,
            backend: Backend::Archive(backend),
        })
    }

    pub fn kind(&self) -> ContainerKind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// A short label for the title bar: the folder or archive file name.
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// What to read for entry `index`.
    ///
    /// Note that an archive member's `member` field is the *backend's* index,
    /// which is not the same as the entry index: the archive lists its own
    /// members in its own order, and only the ones that are images survive
    /// into `entries`. Use `entries()[index]` for the name, never
    /// `entries()[member]`.
    pub fn source(&self, index: usize) -> Option<Source> {
        match (&self.backend, self.entries.get(index)) {
            (Backend::Dir, Some(entry)) => Some(Source::File(self.path.join(&entry.name))),
            (Backend::Archive(_), Some(_)) => Some(Source::ArchiveMember {
                archive: self.path.clone(),
                member: *self.members.get(index)?,
            }),
            _ => None,
        }
    }

    /// Read the raw bytes of entry `index`.
    pub fn read(&mut self, index: usize) -> Result<Vec<u8>, ContainerError> {
        let member = *self
            .members
            .get(index)
            .ok_or_else(|| ContainerError::Io("索引超出范围".to_string()))?;

        match &mut self.backend {
            Backend::Dir => {
                let name = &self.entries[index].name;
                let path = self.path.join(name);
                std::fs::read(&path)
                    .map_err(|e| ContainerError::Io(format!("{}: {e}", path.display())))
            }
            Backend::Archive(backend) => backend.read(member).map_err(Into::into),
        }
    }

    /// Whether the container holds encrypted members needing a password.
    pub fn needs_password(&self) -> bool {
        match &self.backend {
            Backend::Dir => false,
            Backend::Archive(backend) => backend.has_encrypted_members(),
        }
    }

    /// Read at most `max` bytes from the start of entry `index`.
    ///
    /// Used to learn a page's dimensions: the header sits at the front, so the
    /// rest of a multi-megabyte image never has to be decompressed for it, and
    /// listing the sizes of a big archive costs a fraction of a full read per
    /// page.
    pub fn read_prefix(
        &mut self,
        index: usize,
        max: usize,
    ) -> Result<archive::Prefix, ContainerError> {
        let member = *self
            .members
            .get(index)
            .ok_or_else(|| ContainerError::Io("索引超出范围".to_string()))?;

        match &mut self.backend {
            Backend::Dir => {
                let name = &self.entries[index].name;
                let path = self.path.join(name);
                let file = std::fs::File::open(&path)
                    .map_err(|e| ContainerError::Io(format!("{}: {e}", path.display())))?;
                let mut bytes = Vec::new();
                file.take(max as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|e| ContainerError::Io(format!("{}: {e}", path.display())))?;
                let complete = bytes.len() < max || self.entries[index].size <= max as u64;
                Ok(archive::Prefix { bytes, complete })
            }
            Backend::Archive(backend) => backend.read_prefix(member, max).map_err(Into::into),
        }
    }

    /// Supply a password and retry, for encrypted archives.
    pub fn set_password(&mut self, password: &str) {
        if let Backend::Archive(backend) = &mut self.backend {
            backend.set_password(password);
        }
    }
}

/// A place `[` and `]` can step to: a sibling folder or archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerRef {
    Dir(PathBuf),
    Archive(PathBuf),
}

impl ContainerRef {
    pub fn path(&self) -> &Path {
        match self {
            ContainerRef::Dir(path) | ContainerRef::Archive(path) => path,
        }
    }
}

/// Every container reachable from `dir` with `[` and `]`: its subdirectories
/// plus any archive files sitting directly in it.
///
/// Results are sorted naturally by name so the order matches what the user sees
/// in a file manager.
pub fn list_containers(dir: &Path) -> Vec<ContainerRef> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for item in read_dir.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();

        if path.is_dir() {
            found.push(ContainerRef::Dir(path));
        } else if is_archive_extension(&name) {
            found.push(ContainerRef::Archive(path));
        }
    }

    found.sort_by(|a, b| {
        let name = |c: &ContainerRef| {
            c.path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        crate::natsort::natural_cmp(&name(a), &name(b))
    });
    found
}

/// Fill in the pixel dimensions of an entry once they are known, which the
/// info panel and the fit-to-window maths both need.
pub fn set_entry_dimensions(entries: &mut [Entry], index: usize, dimensions: Size) {
    if let Some(entry) = entries.get_mut(index) {
        entry.dimensions = Some(dimensions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A tiny valid PNG, so the tests do not need fixture files on disk.
    fn png_bytes() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 30, 40, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode png");
        out.into_inner()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iv-container-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_only_images_in_a_directory() {
        let dir = scratch("dir-list");
        std::fs::write(dir.join("p10.png"), png_bytes()).unwrap();
        std::fs::write(dir.join("p2.png"), png_bytes()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
        std::fs::create_dir_all(dir.join("subdir.png")).unwrap();

        let container = Container::open_dir(&dir).unwrap();
        let mut names: Vec<&str> = container.entries().iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["p10.png", "p2.png"], "text and directories excluded");
        assert_eq!(container.kind(), ContainerKind::Dir);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_a_file_from_a_directory_container() {
        let dir = scratch("dir-read");
        let bytes = png_bytes();
        std::fs::write(dir.join("a.png"), &bytes).unwrap();

        let mut container = Container::open_dir(&dir).unwrap();
        assert_eq!(container.read(0).unwrap(), bytes);
        assert!(container.read(9).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opens_a_cbz_and_reads_a_page() {
        let dir = scratch("cbz");
        let bytes = png_bytes();
        let archive_path = dir.join("book.cbz");

        // Build a comic archive whose page order only natural sorting gets right.
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            for name in ["page_1.png", "page_2.png", "page_10.png", "readme.txt"] {
                writer.start_file(name, options).unwrap();
                if name.ends_with(".txt") {
                    writer.write_all(b"not an image").unwrap();
                } else {
                    writer.write_all(&bytes).unwrap();
                }
            }
            writer.finish().unwrap();
        }

        let container = Container::open(&archive_path).unwrap();
        assert_eq!(container.kind(), ContainerKind::Archive(ArchiveFormat::Zip));

        // The text file is filtered out.
        assert_eq!(container.len(), 3);

        let mut names: Vec<String> = container
            .entries()
            .iter()
            .map(|e| e.file_name().to_string())
            .collect();
        names.sort_by(|a, b| crate::natsort::natural_cmp(a, b));
        assert_eq!(names, vec!["page_1.png", "page_2.png", "page_10.png"]);

        // Reading by entry index resolves to the right member.
        for index in 0..container.len() {
            let mut container = Container::open(&archive_path).unwrap();
            assert_eq!(container.read(index).unwrap(), bytes, "entry {index}");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reports_an_archive_with_no_images() {
        let dir = scratch("no-images");
        let archive_path = dir.join("docs.zip");
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("readme.txt", options).unwrap();
            writer.write_all(b"nothing to see").unwrap();
            writer.finish().unwrap();
        }

        assert!(matches!(
            Container::open(&archive_path),
            Err(ContainerError::Archive(ArchiveError::NoImages))
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_sibling_containers_for_bracket_navigation() {
        let dir = scratch("siblings");
        std::fs::create_dir_all(dir.join("chapter1")).unwrap();
        std::fs::create_dir_all(dir.join("chapter10")).unwrap();
        std::fs::create_dir_all(dir.join("chapter2")).unwrap();
        std::fs::write(dir.join("book.cbz"), b"PK\x03\x04").unwrap();
        std::fs::write(dir.join("cover.png"), png_bytes()).unwrap();

        let found = list_containers(&dir);
        let names: Vec<String> = found
            .iter()
            .map(|c| c.path().file_name().unwrap().to_string_lossy().into_owned())
            .collect();

        // Directories and archives, in natural order; loose images are not
        // containers because they are not somewhere you navigate *into*.
        assert_eq!(names, vec!["book.cbz", "chapter1", "chapter2", "chapter10"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
