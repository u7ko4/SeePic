//! A persistent cache of image headers.
//!
//! Reading a page's dimensions costs a file open and a header parse. That is
//! microseconds per page, but two-page pairing and webtoon layout need it for
//! *every* page in the container, so a 2000-page comic pays it 2000 times on
//! every open. A JSON file is enough for this, keeps the dependency count down,
//! and stays inspectable.
//!
//! Entries are keyed by a caller-built string that must include everything that
//! invalidates the record — for a file, its size and modification time; for an
//! archive member, the archive's size and time plus the member name. A stale
//! key simply misses, so a wrong key can never produce a wrong answer.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What a header read produces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MetaEntry {
    pub width: u32,
    pub height: u32,
    /// The EXIF orientation tag, when the file carries one.
    pub orientation: Option<u8>,
}

/// One stored record: the key, when it was last used, and the entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    key: u64,
    entry: MetaEntry,
    /// Seconds since the store was created, for evicting the coldest entries.
    used: u64,
}

/// How many entries are kept. At 32 bytes each this is well under a megabyte,
/// which is a fair price for never re-probing a folder twice.
pub const DEFAULT_CAPACITY: usize = 20_000;

/// A size-bounded, file-backed map from a caller's key to an image's header.
pub struct MetaCache {
    path: PathBuf,
    entries: HashMap<u64, Record>,
    capacity: usize,
    /// Bumped on every store, so the writer knows whether there is anything to do.
    dirty: bool,
    clock: u64,
    hits: usize,
    misses: usize,
}

impl MetaCache {
    /// Open the cache at `path`, loading whatever is already there.
    ///
    /// A missing or unreadable file is not an error: the cache is an
    /// optimisation, so it starts empty and gets rebuilt.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let records: Vec<Record> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();

        let clock = records.iter().map(|record| record.used).max().unwrap_or(0);
        let entries = records
            .into_iter()
            .map(|record| (record.key, record))
            .collect::<HashMap<_, _>>();

        Self {
            path,
            entries,
            capacity: DEFAULT_CAPACITY,
            dirty: false,
            clock,
            hits: 0,
            misses: 0,
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.evict_to_fit();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    /// The header for `key`, if it was recorded before.
    pub fn lookup(&mut self, key: &str) -> Option<MetaEntry> {
        let hashed = hash_key(key);
        self.clock += 1;
        match self.entries.get_mut(&hashed) {
            Some(record) => {
                record.used = self.clock;
                self.hits += 1;
                Some(record.entry)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Look several keys up at once, in order.
    pub fn lookup_all(&mut self, keys: &[String]) -> Vec<Option<MetaEntry>> {
        keys.iter().map(|key| self.lookup(key)).collect()
    }

    pub fn store(&mut self, key: &str, entry: MetaEntry) {
        let hashed = hash_key(key);
        self.clock += 1;
        self.entries.insert(
            hashed,
            Record {
                key: hashed,
                entry,
                used: self.clock,
            },
        );
        self.dirty = true;
        self.evict_to_fit();
    }

    /// Write the cache out, if anything changed.
    ///
    /// Called on a timer rather than on every store: the file is a convenience,
    /// and rewriting it per page would cost more than the reads it saves.
    pub fn flush(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let mut records: Vec<Record> = self.entries.values().cloned().collect();
        records.sort_unstable_by_key(|record| record.used);

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(&records)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        std::fs::write(&self.path, bytes)?;
        self.dirty = false;
        Ok(())
    }

    /// Drop the coldest entries once the store is over its limit.
    fn evict_to_fit(&mut self) {
        if self.entries.len() <= self.capacity {
            return;
        }
        let mut by_age: Vec<(u64, u64)> = self
            .entries
            .values()
            .map(|record| (record.used, record.key))
            .collect();
        by_age.sort_unstable();

        let excess = self.entries.len() - self.capacity;
        for (_, key) in by_age.into_iter().take(excess) {
            self.entries.remove(&key);
        }
        self.dirty = true;
    }
}

/// FNV-1a over the key's bytes.
///
/// A full cryptographic hash would be overkill: the key already contains the
/// file's size and modification time, so a collision would need two files to
/// agree on both as well.
fn hash_key(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The key for a file on disk: path, size and modification time.
///
/// `None` when the metadata cannot be read, which means the caller should skip
/// the cache rather than store something that cannot be validated later.
pub fn file_key(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    Some(format!("f:{}|{}|{}", path.display(), meta.len(), modified))
}

/// The key for an entry inside an archive.
///
/// The archive's own size and time stand in for the member's, which an archive
/// does not always record; that makes any edit to the archive invalidate all of
/// its members, which is the safe direction to be wrong in.
pub fn member_key(archive: &Path, member: &str) -> Option<String> {
    let meta = std::fs::metadata(archive).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    Some(format!(
        "m:{}|{}|{}|{}",
        archive.display(),
        meta.len(),
        modified,
        member
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(width: u32, height: u32) -> MetaEntry {
        MetaEntry {
            width,
            height,
            orientation: None,
        }
    }

    /// A cache in a fresh temporary directory, cleaned up by the caller.
    fn scratch(name: &str) -> (MetaCache, PathBuf) {
        let dir = std::env::temp_dir().join(format!("iv-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("meta.json");
        (MetaCache::open(&path), dir)
    }

    #[test]
    fn stores_and_returns_an_entry() {
        let (mut cache, dir) = scratch("store");
        cache.store("a|1|2", entry(800, 1200));
        assert_eq!(cache.lookup("a|1|2"), Some(entry(800, 1200)));
        assert_eq!(cache.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unknown_key_is_a_miss() {
        let (mut cache, dir) = scratch("miss");
        assert_eq!(cache.lookup("nope"), None);
        assert_eq!(cache.stats(), (0, 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_changed_key_is_a_miss() {
        let (mut cache, dir) = scratch("changed");
        cache.store("a|100|5", entry(10, 20));
        // Same path, different size and time: the file changed.
        assert_eq!(cache.lookup("a|200|5"), None);
        assert_eq!(cache.lookup("a|100|6"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn entries_survive_a_reopen() {
        let (mut cache, dir) = scratch("persist");
        cache.store("a|1|2", entry(640, 480));
        cache.flush().expect("flush");

        let mut reopened = MetaCache::open(dir.join("meta.json"));
        assert_eq!(reopened.lookup("a|1|2"), Some(entry(640, 480)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_flush_with_no_changes_writes_nothing() {
        let (mut cache, dir) = scratch("clean");
        // Nothing stored, so the file must not appear.
        cache.flush().expect("flush");
        assert!(!dir.join("meta.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_file_starts_empty() {
        let dir = std::env::temp_dir().join(format!("iv-meta-broken-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("meta.json");
        std::fs::write(&path, b"this is not json").unwrap();

        let mut cache = MetaCache::open(&path);
        assert!(cache.is_empty());
        assert_eq!(cache.lookup("anything"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_coldest_entries_are_evicted_first() {
        let (mut cache, dir) = scratch("evict");
        cache.set_capacity(2);
        cache.store("cold", entry(1, 1));
        cache.store("warm", entry(2, 2));
        // Touch "cold" so "warm" becomes the oldest.
        assert!(cache.lookup("cold").is_some());
        cache.store("new", entry(3, 3));

        assert_eq!(cache.len(), 2);
        assert!(cache.lookup("cold").is_some(), "recently used must survive");
        assert!(cache.lookup("new").is_some());
        assert_eq!(cache.lookup("warm"), None, "oldest should be gone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn lookup_all_answers_in_order() {
        let (mut cache, dir) = scratch("batch");
        cache.store("b", entry(2, 2));
        let answers = cache.lookup_all(&["a".to_string(), "b".to_string()]);
        assert_eq!(answers, vec![None, Some(entry(2, 2))]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_key_changes_when_the_file_does() {
        let dir = std::env::temp_dir().join(format!("iv-meta-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.png");
        std::fs::write(&file, b"one").unwrap();
        let first = file_key(&file).expect("key");
        assert!(first.starts_with("f:"));

        std::fs::write(&file, b"a longer body").unwrap();
        let second = file_key(&file).expect("key");
        assert_ne!(first, second, "size must be part of the key");

        assert_eq!(file_key(&dir.join("missing.png")), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_member_key_names_the_archive_and_the_member() {
        let dir = std::env::temp_dir().join(format!("iv-meta-member-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("a.cbz");
        std::fs::write(&archive, b"body").unwrap();

        let one = member_key(&archive, "page_1.png").expect("key");
        let two = member_key(&archive, "page_2.png").expect("key");
        assert_ne!(one, two, "the member name must be part of the key");
        assert!(one.starts_with("m:"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
