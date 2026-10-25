//! RAR backend, which also covers `CBR`.
//!
//! RAR has no central directory and no seek-by-name: the API is a sequential
//! cursor, so reaching member *N* means walking past members `0..N`. To keep
//! that off the hot path a small archive is extracted in a single pass the first
//! time a member is read. That pass is a memory/CPU tradeoff — it holds every
//! member's bytes — so it is capped well below the size of a realistic comic
//! archive, and a larger archive instead walks to the member it needs and keeps
//! only a bounded window of recent members.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use unrar::Archive;
use unrar::error::UnrarError;

use super::{ArchiveBackend, ArchiveError, Member, normalize_member_name};

/// Archives up to this uncompressed size are pulled into memory in one pass.
///
/// The whole archive stays resident for as long as the container is open, so
/// this is a ceiling on what a RAR can cost the viewer.
const SINGLE_PASS_LIMIT: u64 = 64 * 1024 * 1024;

/// How much of a larger archive's recently read members stay resident.
///
/// Without this, paging back and forth in a large archive would re-walk (and so
/// re-decompress) from the start on every step.
const RECENT_BYTES_LIMIT: usize = 64 * 1024 * 1024;
/// How many recently read members stay resident, whatever their size.
const RECENT_ENTRIES: usize = 8;

pub struct RarBackend {
    path: PathBuf,
    members: Vec<Member>,
    password: Option<String>,
    /// Fully extracted contents, once a single pass has been done.
    cache: Option<Vec<Option<Vec<u8>>>>,
    /// Recently walked-to members, oldest first, for archives too large for the
    /// single pass above.
    recent: VecDeque<(usize, Vec<u8>)>,
    recent_bytes: usize,
}

impl RarBackend {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let archive = Archive::new(path);
        let listing = archive.open_for_listing().map_err(map_err)?;

        let mut members = Vec::new();
        for entry in listing {
            let header = entry.map_err(map_err)?;
            members.push(Member {
                name: normalize_member_name(&header.filename.to_string_lossy()),
                size: header.unpacked_size,
                is_dir: header.is_directory(),
                encrypted: header.is_encrypted(),
            });
        }

        Ok(Self {
            path: path.to_path_buf(),
            members,
            password: None,
            cache: None,
            recent: VecDeque::new(),
            recent_bytes: 0,
        })
    }

    fn open_processing(&self) -> Result<unrar::OpenArchive<unrar::Process, unrar::CursorBeforeHeader>, ArchiveError> {
        let password = self.password.clone();
        let archive = match password.as_deref() {
            Some(value) => Archive::with_password(&self.path, value),
            None => Archive::new(&self.path),
        };
        archive.open_for_processing().map_err(map_err)
    }

    /// Extract every member in one sequential pass.
    fn extract_all(&self) -> Result<Vec<Option<Vec<u8>>>, ArchiveError> {
        let mut cursor = self.open_processing()?;
        let mut out: Vec<Option<Vec<u8>>> = vec![None; self.members.len()];
        let mut index = 0usize;

        while let Some(file) = cursor.read_header().map_err(map_err)? {
            let is_dir = file.entry().is_directory();
            if is_dir {
                cursor = file.skip().map_err(map_err)?;
            } else {
                let (data, next) = file.read().map_err(map_err)?;
                if let Some(slot) = out.get_mut(index) {
                    *slot = Some(data);
                }
                cursor = next;
            }
            index += 1;
        }

        Ok(out)
    }

    /// Walk to `target`, decompressing only that member.
    fn read_by_walking(&self, target: usize) -> Result<Vec<u8>, ArchiveError> {
        let mut cursor = self.open_processing()?;
        let mut index = 0usize;

        while let Some(file) = cursor.read_header().map_err(map_err)? {
            if index == target {
                let (data, _next) = file.read().map_err(map_err)?;
                return Ok(data);
            }
            cursor = file.skip().map_err(map_err)?;
            index += 1;
        }

        Err(ArchiveError::Read("压缩包内不存在该条目".to_string()))
    }

    /// Look in the bounded recent window, promoting a hit to newest.
    fn take_recent(&mut self, index: usize) -> Option<Vec<u8>> {
        let position = self.recent.iter().position(|(slot, _)| *slot == index)?;
        let entry = self.recent.remove(position)?;
        let bytes = entry.1.clone();
        self.recent_bytes -= bytes.len();
        self.recent.push_back(entry);
        self.recent_bytes += bytes.len();
        Some(bytes)
    }

    /// Remember a member just walked to, dropping the oldest to stay in budget.
    fn remember(&mut self, index: usize, bytes: Vec<u8>) {
        if bytes.len() > RECENT_BYTES_LIMIT {
            // One member larger than the whole window would evict everything
            // and still not fit, so it is simply not kept.
            return;
        }
        self.recent.push_back((index, bytes));
        self.recent_bytes += self.recent.back().map_or(0, |(_, bytes)| bytes.len());
        while self.recent.len() > RECENT_ENTRIES || self.recent_bytes > RECENT_BYTES_LIMIT {
            let Some((_, dropped)) = self.recent.pop_front() else {
                break;
            };
            self.recent_bytes -= dropped.len();
        }
    }
}

impl ArchiveBackend for RarBackend {
    fn members(&self) -> &[Member] {
        &self.members
    }

    fn read(&mut self, index: usize) -> Result<Vec<u8>, ArchiveError> {
        if index >= self.members.len() {
            return Err(ArchiveError::Read("压缩包内不存在该条目".to_string()));
        }
        if self.members[index].encrypted && self.password.is_none() {
            return Err(ArchiveError::PasswordRequired);
        }

        if self.cache.is_none() {
            let total: u64 = self.members.iter().map(|m| m.size).sum();
            if total <= SINGLE_PASS_LIMIT {
                self.cache = Some(self.extract_all()?);
            }
        }

        if let Some(cache) = &self.cache {
            return cache
                .get(index)
                .and_then(|slot| slot.clone())
                .ok_or_else(|| ArchiveError::Read("压缩包内不存在该条目".to_string()));
        }

        if let Some(bytes) = self.take_recent(index) {
            return Ok(bytes);
        }
        let bytes = self.read_by_walking(index)?;
        self.remember(index, bytes.clone());
        Ok(bytes)
    }

    fn set_password(&mut self, password: &str) {
        self.password = Some(password.to_string());
        // Cached contents were extracted with the old (or no) password.
        self.cache = None;
        self.recent.clear();
        self.recent_bytes = 0;
    }
}

fn map_err(error: UnrarError) -> ArchiveError {
    use unrar::error::Code;

    #[allow(unreachable_patterns)]
    match error.code {
        Code::BadPassword => ArchiveError::WrongPassword,
        Code::MissingPassword => ArchiveError::PasswordRequired,
        Code::BadArchive => ArchiveError::Corrupt("RAR 数据损坏".to_string()),
        Code::BadData => ArchiveError::Corrupt("RAR 数据校验失败".to_string()),
        Code::UnknownFormat => ArchiveError::Unsupported("不是有效的 RAR 文件".to_string()),
        Code::EOpen => ArchiveError::Open("无法打开 RAR 文件".to_string()),
        other => ArchiveError::Read(format!("RAR 解压失败（错误码 {}）", other as i32)),
    }
}
