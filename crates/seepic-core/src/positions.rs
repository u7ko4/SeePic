//! Remembering which page of a container was being read.
//!
//! Reopening a comic and being dropped back on page 1 is the single most
//! annoying thing a reader can do, so the last position is remembered per
//! container — separately for folders and archives, because the
//! two are used differently.
//!
//! The record stores the *entry name* as well as the position: sorting can
//! change what index a page has, and a name is what the user actually thinks
//! of as "where I was". The position is only the fallback for a renamed file.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One remembered place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// The entry's name, so a sort order change does not lose the place.
    pub entry: String,
    /// The entry's index, used when the name no longer matches.
    pub index: usize,
    /// Seconds since the store was created, for evicting the coldest records.
    used: u64,
}

/// A cap on remembered containers, so the file cannot grow without bound.
pub const DEFAULT_CAPACITY: usize = 2_000;

/// A small, file-backed map from a container key to the last place read in it.
pub struct PositionStore {
    path: PathBuf,
    records: HashMap<String, Position>,
    capacity: usize,
    clock: u64,
    dirty: bool,
}

impl PositionStore {
    /// Open the store at `path`, loading whatever is already there.
    ///
    /// A missing or unreadable file starts empty: this is an optimisation and a
    /// convenience, never a reason to fail to open an image.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let records: Vec<(String, Position)> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();

        let clock = records
            .iter()
            .map(|(_, position)| position.used)
            .max()
            .unwrap_or(0);
        Self {
            path,
            records: records.into_iter().collect(),
            capacity: DEFAULT_CAPACITY,
            clock,
            dirty: false,
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.evict_to_fit();
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Where reading last stopped in `key`.
    pub fn recall(&mut self, key: &str) -> Option<Position> {
        self.clock += 1;
        let clock = self.clock;
        let position = self.records.get_mut(key)?;
        position.used = clock;
        Some(position.clone())
    }

    /// Record a place. Writing to disk is deferred to [`Self::flush`].
    pub fn remember(&mut self, key: &str, entry: &str, index: usize) {
        self.clock += 1;
        self.dirty = true;
        self.records.insert(
            key.to_string(),
            Position {
                entry: entry.to_string(),
                index,
                used: self.clock,
            },
        );
        self.evict_to_fit();
    }

    /// Write the store out, if anything changed.
    pub fn flush(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let records: Vec<(&String, &Position)> = self.records.iter().collect();
        let bytes = serde_json::to_vec(&records)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        std::fs::write(&self.path, bytes)?;
        self.dirty = false;
        Ok(())
    }

    fn evict_to_fit(&mut self) {
        if self.records.len() <= self.capacity {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .records
            .iter()
            .map(|(key, position)| (position.used, key.clone()))
            .collect();
        by_age.sort_unstable();

        let excess = self.records.len() - self.capacity;
        for (_, key) in by_age.into_iter().take(excess) {
            self.records.remove(&key);
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (PositionStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("iv-pos-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (PositionStore::open(dir.join("positions.json")), dir)
    }

    #[test]
    fn remembers_and_recalls_a_place() {
        let (mut store, dir) = scratch("basic");
        store.remember("comic.cbz|1|2", "page_12.png", 11);
        let position = store.recall("comic.cbz|1|2").expect("recall");
        assert_eq!(position.entry, "page_12.png");
        assert_eq!(position.index, 11);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unknown_container_has_no_place() {
        let (mut store, dir) = scratch("unknown");
        assert!(store.recall("nope").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_newest_place_replaces_the_old_one() {
        let (mut store, dir) = scratch("replace");
        store.remember("a", "page_1.png", 0);
        store.remember("a", "page_5.png", 4);
        assert_eq!(store.recall("a").unwrap().entry, "page_5.png");
        assert_eq!(store.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn places_survive_a_reopen() {
        let (mut store, dir) = scratch("persist");
        store.remember("comic.cbz|9|9", "page_7.png", 6);
        store.flush().expect("flush");

        let mut reopened = PositionStore::open(dir.join("positions.json"));
        assert_eq!(reopened.recall("comic.cbz|9|9").unwrap().index, 6);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn flushing_an_unchanged_store_writes_nothing() {
        let (mut store, dir) = scratch("clean");
        store.flush().expect("flush");
        assert!(!dir.join("positions.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_file_starts_empty() {
        let dir = std::env::temp_dir().join(format!("iv-pos-broken-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("positions.json");
        std::fs::write(&path, b"not json").unwrap();

        let mut store = PositionStore::open(&path);
        assert!(store.is_empty());
        assert!(store.recall("anything").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_coldest_places_are_evicted_first() {
        let (mut store, dir) = scratch("evict");
        store.set_capacity(2);
        store.remember("cold", "page_1.png", 0);
        store.remember("warm", "page_2.png", 1);
        // Touching "cold" makes "warm" the oldest.
        assert!(store.recall("cold").is_some());
        store.remember("new", "page_3.png", 2);

        assert_eq!(store.len(), 2);
        assert!(store.recall("cold").is_some());
        assert!(store.recall("new").is_some());
        assert!(store.recall("warm").is_none(), "oldest should be gone");
        let _ = std::fs::remove_dir_all(dir);
    }
}
