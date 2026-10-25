//! TAR backend, which also covers `CBT`.
//!
//! TAR is a plain sequence of headers with no index, so a member can only be
//! reached by walking the headers before it. Rather than repeat that walk for
//! every page, the header walk is done once at open and the byte offset of each
//! member's payload is kept, so a read is a seek plus one `read_to_end`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tar::Archive;

use super::{ArchiveBackend, ArchiveError, Member, Prefix, normalize_member_name};

pub struct TarBackend {
    path: PathBuf,
    members: Vec<Member>,
    /// Byte offset of each member's payload, parallel to `members`.
    ///
    /// Without this a read has to walk every header before the target, which is
    /// one seek per preceding member.
    offsets: Vec<u64>,
}

impl TarBackend {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let mut archive = Archive::new(open_file(path)?);
        let entries = archive
            .entries_with_seek()
            .map_err(|e| ArchiveError::Open(format!("{}: {e}", path.display())))?;

        let mut members = Vec::new();
        let mut offsets = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| ArchiveError::Corrupt(e.to_string()))?;
            let name = entry
                .path()
                .map_err(|e| ArchiveError::Corrupt(e.to_string()))?
                .to_string_lossy()
                .into_owned();
            members.push(Member {
                name: normalize_member_name(&name),
                size: entry.size(),
                is_dir: entry.header().entry_type().is_dir(),
                encrypted: false,
            });
            offsets.push(entry.raw_file_position());
        }

        Ok(Self {
            path: path.to_path_buf(),
            members,
            offsets,
        })
    }
}

impl ArchiveBackend for TarBackend {
    fn members(&self) -> &[Member] {
        &self.members
    }

    fn read(&mut self, index: usize) -> Result<Vec<u8>, ArchiveError> {
        let Some(member) = self.members.get(index) else {
            return Err(ArchiveError::Read("压缩包内不存在该条目".to_string()));
        };
        let Some(offset) = self.offsets.get(index).copied() else {
            return Err(ArchiveError::Read("压缩包内不存在该条目".to_string()));
        };
        let size = member.size;

        let mut file = open_file(&self.path)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| ArchiveError::Read(e.to_string()))?;
        let mut buffer = Vec::with_capacity(size as usize);
        file.take(size)
            .read_to_end(&mut buffer)
            .map_err(|e| ArchiveError::Read(e.to_string()))?;
        Ok(buffer)
    }

    fn read_prefix(&mut self, index: usize, max: usize) -> Result<Prefix, ArchiveError> {
        let Some(member) = self.members.get(index) else {
            return Err(ArchiveError::Read("压缩包内不存在该条目".to_string()));
        };
        let Some(offset) = self.offsets.get(index).copied() else {
            return Err(ArchiveError::Read("压缩包内不存在该条目".to_string()));
        };
        let size = member.size;
        let complete = size <= max as u64;
        let limit = size.min(max as u64);

        let mut file = open_file(&self.path)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| ArchiveError::Read(e.to_string()))?;
        let mut buffer = Vec::with_capacity(limit as usize);
        file.take(limit)
            .read_to_end(&mut buffer)
            .map_err(|e| ArchiveError::Read(e.to_string()))?;
        Ok(Prefix {
            bytes: buffer,
            complete,
        })
    }

    fn set_password(&mut self, _password: &str) {
        // TAR has no encryption.
    }
}

fn open_file(path: &Path) -> Result<File, ArchiveError> {
    File::open(path).map_err(|e| ArchiveError::Open(format!("{}: {e}", path.display())))
}
