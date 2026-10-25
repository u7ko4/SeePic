//! 7z backend, which also covers `CB7`.
//!
//! The reader keeps an internal name index, so extraction is by member name
//! rather than by index; we map indices to names once at open time.

use std::fs::File;
use std::path::{Path, PathBuf};

use sevenz_rust2::{ArchiveReader, Password};

use super::{ArchiveBackend, ArchiveError, Member, normalize_member_name};

pub struct SevenZBackend {
    path: PathBuf,
    reader: ArchiveReader<File>,
    members: Vec<Member>,
    /// Member names by index, used because the reader extracts by name.
    names: Vec<String>,
    password: Option<String>,
}

impl SevenZBackend {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let reader = ArchiveReader::open(path, Password::empty()).map_err(map_err)?;

        let mut members = Vec::new();
        let mut names = Vec::new();
        for entry in &reader.archive().files {
            names.push(entry.name.clone());
            members.push(Member {
                name: normalize_member_name(&entry.name),
                size: entry.size,
                is_dir: entry.is_directory(),
                // 7z does not expose per-entry encryption flags through this API,
                // so encryption is discovered when a read fails.
                encrypted: false,
            });
        }

        Ok(Self {
            path: path.to_path_buf(),
            reader,
            members,
            names,
            password: None,
        })
    }

    fn reopen(&mut self) -> Result<(), ArchiveError> {
        let password = match &self.password {
            Some(value) => Password::new(value),
            None => Password::empty(),
        };
        self.reader = ArchiveReader::open(&self.path, password).map_err(map_err)?;
        Ok(())
    }
}

impl ArchiveBackend for SevenZBackend {
    fn members(&self) -> &[Member] {
        &self.members
    }

    fn read(&mut self, index: usize) -> Result<Vec<u8>, ArchiveError> {
        let name = self
            .names
            .get(index)
            .ok_or_else(|| ArchiveError::Read("压缩包内不存在该条目".to_string()))?
            .clone();

        self.reader.read_file(&name).map_err(map_err)
    }

    fn set_password(&mut self, password: &str) {
        self.password = Some(password.to_string());
        // The password is a constructor argument, so the reader must be rebuilt.
        let _ = self.reopen();
    }
}

fn map_err(error: sevenz_rust2::Error) -> ArchiveError {
    use sevenz_rust2::Error as E;

    #[allow(unreachable_patterns)]
    match error {
        E::PasswordRequired => ArchiveError::PasswordRequired,
        E::MaybeBadPassword(_) => ArchiveError::WrongPassword,
        E::BadSignature(_) => ArchiveError::Unsupported("7z 签名无效".to_string()),
        E::UnsupportedVersion { major, minor } => {
            ArchiveError::Unsupported(format!("7z {major}.{minor}"))
        }
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch => {
            ArchiveError::Corrupt("校验失败".to_string())
        }
        E::FileNotFound => ArchiveError::Read("压缩包内不存在该条目".to_string()),
        other => ArchiveError::Read(other.to_string()),
    }
}
