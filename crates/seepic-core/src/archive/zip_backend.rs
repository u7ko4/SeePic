//! ZIP backend, which also covers `CBZ`.
//!
//! ZIP has a central directory, so opening it parses the whole member list and
//! reading member *N* seeks straight to it without touching members 0..N.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;
use zip::read::{ArchiveOffset, Config};

use super::{ArchiveBackend, ArchiveError, Member, Prefix, normalize_member_name};

pub struct ZipBackend {
    archive: ZipArchive<BufReader<File>>,
    members: Vec<Member>,
    password: Option<Vec<u8>>,
    path: PathBuf,
}

impl ZipBackend {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let archive = open_archive(path)?;

        // The central directory already holds every field the listing needs.
        // `by_index_raw` would also read each entry's local header, one seek per
        // entry — on a network share or a spinner that is the difference between
        // a list that appears and one that does not.
        let count = archive.len();
        let mut members = Vec::with_capacity(count);
        for index in 0..count {
            let entry = archive.by_index_data(index).map_err(map_zip_err)?;
            let name = entry.name().map_err(map_zip_err)?.into_owned();
            members.push(Member {
                name: normalize_member_name(&name),
                size: entry.size(),
                is_dir: entry.is_dir(),
                encrypted: entry.encrypted(),
            });
        }

        Ok(Self {
            archive,
            members,
            password: None,
            path: path.to_path_buf(),
        })
    }
}

/// Open a ZIP, telling the parser where the archive starts when that is the
/// common case.
///
/// `zip`'s offset detection searches the whole file for the central directory
/// signature before it will believe the offset the end record points at. On a
/// 700 MB comic that is a full read of the file — seconds — just to list it, so
/// the normal case (archive at offset 0) is stated outright. Anything with
/// prepended data, such as a self-extracting stub, is left to the detector.
fn open_archive(path: &Path) -> Result<ZipArchive<BufReader<File>>, ArchiveError> {
    let reader = || -> Result<BufReader<File>, ArchiveError> {
        File::open(path)
            .map(BufReader::new)
            .map_err(|e| ArchiveError::Open(format!("{}: {e}", path.display())))
    };

    let fast = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
        reader()?,
    );
    match fast {
        Ok(archive) => Ok(archive),
        // Prepended data shifts every offset; only then is the scan worth its
        // cost.
        Err(_) => ZipArchive::new(reader()?).map_err(map_zip_err),
    }
}

impl ArchiveBackend for ZipBackend {
    fn members(&self) -> &[Member] {
        &self.members
    }

    fn read(&mut self, index: usize) -> Result<Vec<u8>, ArchiveError> {
        let encrypted = self.members.get(index).is_some_and(|m| m.encrypted);
        if encrypted && self.password.is_none() {
            return Err(ArchiveError::PasswordRequired);
        }

        let mut file = match &self.password {
            Some(password) => self.archive.by_index_decrypt(index, password),
            None => self.archive.by_index(index),
        }
        .map_err(|e| match e {
            zip::result::ZipError::InvalidPassword => ArchiveError::WrongPassword,
            other => map_zip_err(other),
        })?;

        let mut buffer = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buffer)
            .map_err(|e| ArchiveError::Read(format!("{}: {e}", self.path.display())))?;
        Ok(buffer)
    }

    fn read_prefix(&mut self, index: usize, max: usize) -> Result<Prefix, ArchiveError> {
        let encrypted = self.members.get(index).is_some_and(|m| m.encrypted);
        if encrypted && self.password.is_none() {
            return Err(ArchiveError::PasswordRequired);
        }

        let file = match &self.password {
            Some(password) => self.archive.by_index_decrypt(index, password),
            None => self.archive.by_index(index),
        }
        .map_err(|e| match e {
            zip::result::ZipError::InvalidPassword => ArchiveError::WrongPassword,
            other => map_zip_err(other),
        })?;

        let size = file.size();
        let complete = size <= max as u64;
        let limit = if complete { size } else { max as u64 };
        let mut buffer = Vec::with_capacity(limit as usize);
        file.take(limit)
            .read_to_end(&mut buffer)
            .map_err(|e| ArchiveError::Read(format!("{}: {e}", self.path.display())))?;
        Ok(Prefix {
            bytes: buffer,
            complete,
        })
    }

    fn set_password(&mut self, password: &str) {
        self.password = Some(password.as_bytes().to_vec());
    }
}

fn map_zip_err(error: zip::result::ZipError) -> ArchiveError {
    #[allow(unreachable_patterns)]
    match error {
        zip::result::ZipError::InvalidPassword => ArchiveError::WrongPassword,
        zip::result::ZipError::InvalidArchive(message) => ArchiveError::Corrupt(message.into_owned()),
        // Without a password the zip crate reports the missing key as an
        // unsupported archive, with this exact message. It is really a request
        // for credentials, so route it to the password prompt.
        zip::result::ZipError::UnsupportedArchive(what)
            if what == zip::result::ZipError::PASSWORD_REQUIRED =>
        {
            ArchiveError::PasswordRequired
        }
        zip::result::ZipError::UnsupportedArchive(what) => ArchiveError::Unsupported(what.to_string()),
        zip::result::ZipError::CompressionMethodNotSupported(method) => {
            ArchiveError::Unsupported(format!("压缩方式 {method}"))
        }
        zip::result::ZipError::FileNotFound => ArchiveError::Read("压缩包内不存在该条目".to_string()),
        zip::result::ZipError::Io(error) => ArchiveError::Read(error.to_string()),
        other => ArchiveError::Read(other.to_string()),
    }
}
