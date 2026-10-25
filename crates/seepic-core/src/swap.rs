//! Frames kept in mapped temporary files — the process's swap pages.
//!
//! Decoded pixels are the largest thing this program holds: a 24-megapixel
//! photo is 96 MB of BGRA, and a handful of those is what makes the task
//! manager's number for the process climb. Kept on the heap they are *private*
//! pages: what that number shows, what counts against the commit charge, and
//! what the OS cannot take back under pressure. Written into a mapped temporary
//! file they are *file* pages instead — the same bytes are still resident, but
//! they can be dropped and read back from the file at any time, so the private
//! working set stops growing with the number of frames held.
//!
//! That is the whole point of this module, and also its limit: it changes where
//! the bytes live, not how many there are. Cutting the byte count is the
//! decoder's job — the reduced decode — and it comes first. A frame in a slot
//! file still occupies physical memory; it is just reclaimable, and it is
//! written back to the file when its slot is released.
//!
//! Three properties are worth knowing before measuring anything here:
//!
//! - Writes go through a writable mapping, which does **not** put the pages on
//!   the system's dirty list, so the lazy writer leaves them alone while they
//!   are mapped. Browsing therefore writes almost nothing.
//! - Releasing a slot is what triggers the writeback, and it continues for a
//!   minute or two after the process exits. Measuring disk writes over the ten
//!   seconds after exit understates them by an order of magnitude.
//! - The files are created delete-on-close (with share-delete, so a mapped file
//!   can still be deleted), and a recycled slot is truncated before it is
//!   reused, so it can never show the frame that was in it before.

use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use memmap2::{MmapMut, MmapOptions};

/// Prefix of every slot file, so leftovers from a killed process can be found.
pub const SWAP_PREFIX: &str = "seepic-swap-";

/// Frames smaller than this stay on the heap.
///
/// A slot file costs metadata, a truncate and a mapping; a thumbnail of a few
/// kilobytes cannot pay for that.
pub const MIN_SWAP_BYTES: usize = 256 * 1024;

/// A frame smaller than this fraction of a slot is not worth one.
///
/// A slot is reserved and charged whole (see `cache::decoded_size`), so mapping
/// a one-megabyte burst preview into a sixteen-megabyte slot would spend
/// sixteen times what the frame costs and, worse, let the decode cache hold only
/// as many previews as there are slots. Previews are numerous and short-lived;
/// on the heap they are charged their real size, and the cache's byte budget
/// keeps them bounded.
const SLOT_FILL_DIV: usize = 4;

/// How the store is set up. Straight from the settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapConfig {
    /// Whether frames are moved into slot files at all.
    pub enabled: bool,
    /// How many slot files the store keeps, so the file count stays constant
    /// however large the directory is.
    pub slots: usize,
    /// Capacity of one slot. A frame larger than this gets a file of its own.
    pub slot_bytes: usize,
    /// Upper bound on the bytes mapped at once. `slots × slot_bytes` is the
    /// disk the store reserves, and this is the ceiling on it.
    pub capacity_bytes: usize,
}

impl Default for SwapConfig {
    fn default() -> Self {
        Self::from_mb(true, 8, 16, 128)
    }
}

impl SwapConfig {
    /// Build a configuration from the settings page's figures.
    pub fn from_mb(enabled: bool, slots: u32, slot_mb: u32, capacity_mb: u32) -> Self {
        Self {
            enabled,
            slots: slots.clamp(1, 64) as usize,
            slot_bytes: slot_mb.max(1) as usize * 1024 * 1024,
            capacity_bytes: capacity_mb.max(1) as usize * 1024 * 1024,
        }
    }

    /// Whether the store would take a frame of `bytes`.
    ///
    /// Kept next to the settings so the panel can say why a figure has no
    /// effect, and so the tests can state the rule in one place.
    pub fn takes(&self, bytes: usize) -> bool {
        self.enabled && bytes >= MIN_SWAP_BYTES && bytes <= self.capacity_bytes
    }

    /// Whether a frame of `bytes` should live in a slot rather than the heap.
    ///
    /// This is the whole decision, used both when the store takes a frame and
    /// when the cache decides whether a frame it was handed may be kept, so the
    /// two can never disagree about which backend a frame belongs to.
    pub fn maps(&self, bytes: usize) -> bool {
        self.takes(bytes) && bytes >= self.slot_threshold()
    }

    /// The size below which a frame stays on the heap.
    pub fn slot_threshold(&self) -> usize {
        MIN_SWAP_BYTES.max(self.slot_bytes / SLOT_FILL_DIV)
    }

    /// How many slot files actually fit inside the ceiling.
    pub fn usable_slots(&self) -> usize {
        if !self.enabled || self.slot_bytes == 0 {
            return 0;
        }
        self.slots.min(self.capacity_bytes / self.slot_bytes)
    }
}

/// What the store is doing, for the info panel and the tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SwapStats {
    /// Bytes currently mapped, counting each slot's whole capacity.
    pub mapped_bytes: usize,
    /// The high-water mark of `mapped_bytes`.
    pub peak_bytes: usize,
    /// Frames living in slot files right now.
    pub live_frames: usize,
    /// Slot files the store has created.
    pub slot_files: usize,
    /// Slot files waiting to be reused.
    pub free_slots: usize,
    /// Frames too large for a slot, each in a file of its own.
    pub large_frames: usize,
    /// Slot files that were recycled rather than created.
    pub reused: usize,
    /// Frames the store would not take, which stayed on the heap.
    pub refused: usize,
    /// Leftover files from an earlier run that startup cleaned up.
    pub swept: usize,
    /// Bytes one slot reserves, whether or not a frame fills it.
    pub slot_bytes: usize,
}

/// A frame's pixels inside a slot file.
///
/// Dropping it releases the mapping and returns the slot to the store, off the
/// caller's thread: unmapping a dirty mapping is what starts the writeback, and
/// that has no business happening on the UI thread while a page is turning.
pub struct SwapFrame {
    map: Option<MmapMut>,
    /// Bytes of pixel data the frame holds.
    bytes: usize,
    /// What the store counts while the frame lives: a slot's whole capacity, or
    /// the frame's own length when it has a file of its own.
    reserved: usize,
    /// The slot it came from, when it is one of the store's own.
    slot: Option<usize>,
    /// A file of its own, for a frame too large for a slot.
    own_file: Option<tempfile::NamedTempFile>,
    store: Arc<Mutex<Inner>>,
    reaper: Option<Sender<Reap>>,
}

impl std::fmt::Debug for SwapFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwapFrame")
            .field("bytes", &self.bytes)
            .field("slot", &self.slot)
            .finish()
    }
}

impl SwapFrame {
    /// The frame's pixels.
    pub fn as_slice(&self) -> &[u8] {
        self.map.as_deref().unwrap_or(&[])
    }

    /// The frame's pixels, to write a decoded image into.
    pub fn as_mut(&mut self) -> Option<&mut [u8]> {
        self.map.as_mut().map(|map| &mut map[..])
    }

    /// How many bytes of pixel data the frame holds.
    pub fn len(&self) -> usize {
        self.bytes
    }

    /// What the frame costs the store: a slot's whole capacity, or the frame's
    /// own length when it has a file of its own.
    ///
    /// The decoded-image cache charges this rather than the pixel count, so the
    /// two backends agree on how much room exists and the cache cannot ask for
    /// more frames than the store has slots for.
    pub fn reserved_bytes(&self) -> usize {
        self.reserved
    }

    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
}

impl Drop for SwapFrame {
    fn drop(&mut self) {
        let Some(map) = self.map.take() else {
            return;
        };
        // The file handle is kept until the mapping is gone, so the data the
        // mapping reads is still there while it is unmapped.
        let reap = Reap {
            map,
            bytes: self.reserved,
            slot: self.slot,
            file: self.own_file.take(),
        };
        // Handing the unmapping to the reaper keeps a page turn from waiting on
        // it. With no reaper — the store was dropped first — it happens here,
        // which is still correct, only synchronous.
        let bytes = reap.bytes;
        let slot = reap.slot;
        let sent = self
            .reaper
            .as_ref()
            .is_some_and(|reaper| reaper.send(reap).is_ok());
        if sent {
            return;
        }
        let mut inner = self.store.lock().unwrap_or_else(|e| e.into_inner());
        inner.released(bytes, slot);
    }
}

/// One release, handed to the reaper thread.
struct Reap {
    map: MmapMut,
    bytes: usize,
    slot: Option<usize>,
    /// A frame's own file, when it had one. It is dropped after the mapping,
    /// so the file outlives everything that reads it.
    file: Option<tempfile::NamedTempFile>,
}

/// A slot file the store keeps for reuse.
struct SlotFile {
    /// Delete-on-close, so a clean exit leaves nothing behind.
    file: tempfile::NamedTempFile,
    /// Where the slot lives, for the tests and for the log.
    #[cfg_attr(not(test), allow(dead_code))]
    path: PathBuf,
}

impl SlotFile {
    fn as_file(&self) -> &File {
        self.file.as_file()
    }

    /// Give the slot the length the next frame needs.
    ///
    /// Truncating to zero first is what makes a recycled slot safe: whatever
    /// the previous frame left in it is gone before the new frame is written.
    fn resize(&self, len: usize) -> io::Result<()> {
        self.as_file().set_len(0)?;
        self.as_file().set_len(len as u64)
    }
}

struct Inner {
    config: SwapConfig,
    /// Slot files, by id. An entry is `None` once its file has been discarded.
    pool: Vec<Option<SlotFile>>,
    /// Ids of the free slot files, most recently released first.
    free: VecDeque<usize>,
    mapped_bytes: usize,
    peak_bytes: usize,
    live_frames: usize,
    large_frames: usize,
    reused: usize,
    refused: usize,
    swept: usize,
}

impl Inner {
    /// Record a release. The mapping is already gone by the time this runs.
    fn released(&mut self, bytes: usize, slot: Option<usize>) {
        self.mapped_bytes = self.mapped_bytes.saturating_sub(bytes);
        self.live_frames = self.live_frames.saturating_sub(1);
        match slot {
            Some(id) => {
                if self.pool.get(id).is_some_and(Option::is_some) {
                    self.free.push_front(id);
                }
            }
            None => self.large_frames = self.large_frames.saturating_sub(1),
        }
    }

    /// The number of slot files that still exist.
    fn live_slots(&self) -> usize {
        self.pool.iter().filter(|slot| slot.is_some()).count()
    }
}

/// Create a slot file, preallocated to `capacity`.
///
/// Preallocating rather than growing as the frame is written keeps the file's
/// length fixed: no metadata update and no remapping halfway through a frame.
/// NTFS reserves the clusters without writing them, so the reservation costs
/// disk quota but no data.
fn create_slot(capacity: usize) -> io::Result<SlotFile> {
    let file = tempfile::Builder::new().prefix(SWAP_PREFIX).tempfile()?;
    file.as_file().set_len(capacity as u64)?;
    let path = file.path().to_path_buf();
    Ok(SlotFile { file, path })
}

/// Create a file holding exactly one oversized frame.
fn create_large(len: usize) -> io::Result<tempfile::NamedTempFile> {
    let file = tempfile::Builder::new().prefix(SWAP_PREFIX).tempfile()?;
    file.as_file().set_len(len as u64)?;
    Ok(file)
}

/// How old a slot file has to be before startup treats it as leftover.
///
/// A live store creates its files seconds before it uses them, so sweeping
/// anything younger would delete the slots of another running instance (and of
/// a parallel test). Deleting them would not corrupt anything — a mapping
/// outlives its directory entry — but the files are the store's, so leave them.
const STALE_AFTER: Duration = Duration::from_secs(60);

/// Delete leftover slot files from an earlier run.
///
/// A clean exit leaves none, since they are created delete-on-close; a process
/// that was killed can leave a few behind. Only stale ones are removed.
fn sweep() -> usize {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return 0;
    };
    let mut swept = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(SWAP_PREFIX) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            swept += 1;
        }
    }
    swept
}

/// Map `len` bytes of `file` for writing.
///
/// SAFETY: the mapping is over a file this store owns and keeps alive for as
/// long as the mapping exists, and nothing else writes to it, so the bytes
/// cannot change under a reader.
fn map_writable(file: &File, len: usize) -> io::Result<MmapMut> {
    unsafe { MmapOptions::new().len(len).map_mut(file) }
}

/// The frame store: a fixed set of slot files, reused in turn.
#[derive(Clone)]
pub struct FrameStore {
    inner: Arc<Mutex<Inner>>,
    reaper: Option<Sender<Reap>>,
}

impl Default for FrameStore {
    fn default() -> Self {
        Self::new(SwapConfig::default())
    }
}

impl FrameStore {
    /// Start a store, sweeping what an earlier run left behind.
    pub fn new(config: SwapConfig) -> Self {
        let swept = sweep();
        let inner = Arc::new(Mutex::new(Inner {
            config,
            pool: Vec::new(),
            free: VecDeque::new(),
            mapped_bytes: 0,
            peak_bytes: 0,
            live_frames: 0,
            large_frames: 0,
            reused: 0,
            refused: 0,
            swept,
        }));

        // The reaper owns unmapping. It holds the state but not the channel, so
        // dropping the store closes the channel and ends the thread.
        let (sender, receiver) = mpsc::channel::<Reap>();
        let worker = inner.clone();
        let spawned = std::thread::Builder::new()
            .name("seepic-swap-reaper".to_string())
            .spawn(move || {
                while let Ok(reap) = receiver.recv() {
                    let Reap {
                        map,
                        bytes,
                        slot,
                        file,
                    } = reap;
                    drop(map);
                    drop(file);
                    let mut inner = worker.lock().unwrap_or_else(|e| e.into_inner());
                    inner.released(bytes, slot);
                }
            })
            .is_ok();

        Self {
            inner,
            reaper: spawned.then_some(sender),
        }
    }

    /// Take a slot for a frame of `bytes`, or `None` to keep it on the heap.
    ///
    /// `None` is an ordinary answer, not a failure: the frame simply stays on
    /// the heap, which is what happens when the store is off, when the frame is
    /// too small to be worth a file, and when the ceiling is reached.
    pub fn acquire(&self, bytes: usize) -> Option<SwapFrame> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let config = inner.config;
        if !config.maps(bytes) {
            // A frame below the slot threshold is not a refusal: the heap is
            // where it belongs. One above it that still cannot be taken — the
            // ceiling, or a failed file — is.
            if config.enabled && bytes >= config.slot_threshold() {
                inner.refused += 1;
            }
            return None;
        }

        // A frame larger than a slot gets a file of its own, which goes away
        // with the frame.
        if bytes > config.slot_bytes {
            if inner.mapped_bytes + bytes > config.capacity_bytes {
                inner.refused += 1;
                return None;
            }
            let file = match create_large(bytes) {
                Ok(file) => file,
                Err(_) => {
                    inner.refused += 1;
                    return None;
                }
            };
            let map = match map_writable(file.as_file(), bytes) {
                Ok(map) => map,
                Err(_) => {
                    inner.refused += 1;
                    return None;
                }
            };
            inner.mapped_bytes += bytes;
            inner.peak_bytes = inner.peak_bytes.max(inner.mapped_bytes);
            inner.live_frames += 1;
            inner.large_frames += 1;
            return Some(SwapFrame {
                map: Some(map),
                bytes,
                reserved: bytes,
                slot: None,
                own_file: Some(file),
                store: self.inner.clone(),
                reaper: self.reaper.clone(),
            });
        }

        if inner.mapped_bytes + config.slot_bytes > config.capacity_bytes
            || inner.live_slots() >= config.usable_slots() && inner.free.is_empty()
        {
            inner.refused += 1;
            return None;
        }

        let id = match inner.free.pop_front() {
            Some(id) => {
                let Some(slot) = inner.pool.get(id).and_then(Option::as_ref) else {
                    inner.refused += 1;
                    return None;
                };
                if slot.resize(bytes).is_err() {
                    inner.refused += 1;
                    return None;
                }
                inner.reused += 1;
                id
            }
            None => {
                let slot = match create_slot(config.slot_bytes) {
                    Ok(slot) => slot,
                    Err(_) => {
                        inner.refused += 1;
                        return None;
                    }
                };
                inner.pool.push(Some(slot));
                inner.pool.len() - 1
            }
        };

        let map = inner
            .pool
            .get(id)
            .and_then(Option::as_ref)
            .and_then(|slot| map_writable(slot.as_file(), bytes).ok());
        let Some(map) = map else {
            // The file stays in the pool; only the frame is not created.
            if !inner.free.contains(&id) {
                inner.free.push_front(id);
            }
            inner.refused += 1;
            return None;
        };

        inner.mapped_bytes += config.slot_bytes;
        inner.peak_bytes = inner.peak_bytes.max(inner.mapped_bytes);
        inner.live_frames += 1;
        Some(SwapFrame {
            map: Some(map),
            bytes,
            reserved: config.slot_bytes,
            slot: Some(id),
            own_file: None,
            store: self.inner.clone(),
            reaper: self.reaper.clone(),
        })
    }

    /// What the store is doing right now.
    pub fn stats(&self) -> SwapStats {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        SwapStats {
            mapped_bytes: inner.mapped_bytes,
            peak_bytes: inner.peak_bytes,
            live_frames: inner.live_frames,
            slot_files: inner.live_slots(),
            free_slots: inner.free.len(),
            large_frames: inner.large_frames,
            reused: inner.reused,
            refused: inner.refused,
            swept: inner.swept,
            slot_bytes: inner.config.slot_bytes,
        }
    }

    /// Where slot files are created.
    pub fn directory() -> PathBuf {
        std::env::temp_dir()
    }

    /// The configuration in force.
    pub fn config(&self) -> SwapConfig {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).config
    }

    /// Apply a new configuration.
    ///
    /// Slot files are discarded rather than reshaped: one in use cannot be
    /// resized under the frame reading it, and the pool is cheap to rebuild.
    /// Frames already in slot files keep working — a mapping outlives the
    /// file's directory entry — and are simply not returned to the pool when
    /// they are released.
    pub fn reconfigure(&self, config: SwapConfig) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.config == config {
            return;
        }
        let reshaped = inner.config.slots != config.slots
            || inner.config.slot_bytes != config.slot_bytes
            || !config.enabled;
        inner.config = config;
        if reshaped {
            inner.pool.clear();
            inner.free.clear();
        }
    }

    /// Wait until every released mapping has been unmapped.
    ///
    /// For tests and a clean shutdown; nothing in the viewer's own path needs
    /// it.
    pub fn settle(&self) {
        for _ in 0..2000 {
            if {
                let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.live_frames == 0 && inner.mapped_bytes == 0
            } {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(slots: u32, slot_mb: u32, capacity_mb: u32) -> FrameStore {
        FrameStore::new(SwapConfig::from_mb(true, slots, slot_mb, capacity_mb))
    }

    /// A frame's bytes, as if they had just been decoded.
    fn pixels(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    /// Put a frame into the store the way the viewer does: take a slot, copy
    /// the decoded pixels in, and let the heap copy go.
    fn store_frame(store: &FrameStore, len: usize, seed: u8) -> Option<SwapFrame> {
        let mut frame = store.acquire(len)?;
        let pixels = pixels(len, seed);
        frame.as_mut()?.copy_from_slice(&pixels);
        Some(frame)
    }

    fn slot_paths(store: &FrameStore) -> Vec<PathBuf> {
        let inner = store.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .pool
            .iter()
            .flatten()
            .map(|slot| slot.path.clone())
            .collect()
    }

    #[test]
    fn a_frame_round_trips_through_its_slot() {
        let store = store(2, 4, 16);
        let len = 1 << 20;
        let frame = store_frame(&store, len, 7).expect("a slot");
        assert_eq!(frame.as_slice(), pixels(len, 7).as_slice());
        assert_eq!(frame.len(), len);
        // The file exists while the frame is mapped.
        let paths = slot_paths(&store);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());

        let stats = store.stats();
        assert_eq!(stats.live_frames, 1);
        assert_eq!(stats.slot_files, 1);
        // A slot reserves its whole capacity, which is what the ceiling counts.
        assert_eq!(stats.mapped_bytes, 4 * 1024 * 1024);

        // Releasing the frame leaves the file in the pool, ready to be reused.
        drop(frame);
        store.settle();
        assert!(paths[0].exists(), "the slot file is kept for reuse");

        // Dropping the store takes the files with it: they are delete-on-close,
        // so a clean exit leaves nothing in the temp directory.
        drop(store);
        for _ in 0..100 {
            if !paths[0].exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!paths[0].exists(), "slot files are delete-on-close");
    }

    #[test]
    fn small_frames_stay_on_the_heap() {
        // A quarter of a 16 MB slot is the threshold; a one-megabyte preview is
        // far below it and must not spend a whole slot.
        let store = store(4, 16, 64);
        assert!(store.acquire(MIN_SWAP_BYTES - 1).is_none());
        assert!(store.acquire(1024 * 1024).is_none());
        assert!(store.acquire(4 * 1024 * 1024).is_some());
        // A frame too small to be worth a file is not a refusal to report: the
        // heap is the right place for it.
        assert_eq!(store.stats().refused, 0);
    }

    #[test]
    fn the_capacity_ceiling_is_never_crossed() {
        // Four slots of 4 MB fit in 16 MB; the fifth cannot be taken.
        let store = store(8, 4, 16);
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(store_frame(&store, 1 << 20, 1).expect("within the ceiling"));
        }
        assert!(store.stats().mapped_bytes <= store.config().capacity_bytes);
        assert!(store_frame(&store, 1 << 20, 1).is_none());
        assert_eq!(store.stats().refused, 1);
        // And the ceiling is what limits the slot count, not the slot setting.
        assert_eq!(store.config().usable_slots(), 4);
    }

    #[test]
    fn the_slot_count_is_a_second_ceiling() {
        // Room for eight slots, but only two exist.
        let store = store(2, 4, 64);
        let a = store_frame(&store, 1 << 20, 1).expect("slot one");
        let b = store_frame(&store, 1 << 20, 2).expect("slot two");
        assert!(store_frame(&store, 1 << 20, 3).is_none());
        assert_eq!(store.stats().slot_files, 2);

        drop(a);
        store.settle();
        // Releasing one frees it for the next frame, and the file is reused
        // rather than added to.
        let c = store_frame(&store, 1 << 20, 3).expect("the freed slot");
        assert_eq!(store.stats().slot_files, 2);
        assert_eq!(store.stats().reused, 1);
        drop((b, c));
        store.settle();
    }

    #[test]
    fn a_recycled_slot_never_shows_the_previous_frame() {
        // Both frames are above the slot threshold (a quarter of 16 MB), and
        // the second is shorter, so anything left over from the first would
        // show up past the end of it.
        let store = store(1, 16, 32);
        let first = store_frame(&store, 6 << 20, 0xff).expect("slot");
        assert_eq!(first.as_slice()[0], 0xff);
        drop(first);
        store.settle();

        let second = store_frame(&store, 4 << 20, 0x11).expect("the recycled slot");
        assert_eq!(second.as_slice().len(), 4 << 20);
        assert_eq!(second.as_slice(), pixels(4 << 20, 0x11).as_slice());
        assert_eq!(store.stats().reused, 1);
    }

    #[test]
    fn a_frame_larger_than_a_slot_gets_a_file_of_its_own() {
        let store = store(2, 4, 64);
        let len = 6 * 1024 * 1024;
        let frame = store_frame(&store, len, 3).expect("an oversized frame");
        assert_eq!(frame.len(), len);
        assert_eq!(frame.as_slice(), pixels(len, 3).as_slice());
        let stats = store.stats();
        assert_eq!(stats.large_frames, 1);
        // Counted by what it holds, not by a slot's capacity.
        assert_eq!(stats.mapped_bytes, len);
        assert_eq!(stats.slot_files, 0);

        drop(frame);
        store.settle();
        assert_eq!(store.stats().live_frames, 0);
        assert_eq!(store.stats().mapped_bytes, 0);
    }

    #[test]
    fn releasing_a_frame_returns_its_slot() {
        let store = store(2, 4, 16);
        let frame = store_frame(&store, 1 << 20, 9).expect("slot");
        assert_eq!(store.stats().free_slots, 0);
        drop(frame);
        store.settle();
        let stats = store.stats();
        assert_eq!(stats.live_frames, 0);
        assert_eq!(stats.mapped_bytes, 0);
        assert_eq!(stats.free_slots, 1);
        assert_eq!(stats.slot_files, 1, "the file is kept for reuse");
    }

    #[test]
    fn the_peak_records_the_high_water_mark() {
        let store = store(4, 4, 32);
        let a = store_frame(&store, 1 << 20, 1).expect("slot");
        let b = store_frame(&store, 1 << 20, 2).expect("slot");
        assert_eq!(store.stats().peak_bytes, 8 * 1024 * 1024);
        drop((a, b));
        store.settle();
        assert_eq!(store.stats().mapped_bytes, 0);
        assert_eq!(store.stats().peak_bytes, 8 * 1024 * 1024);
    }

    #[test]
    fn a_disabled_store_keeps_everything_on_the_heap() {
        let store = FrameStore::new(SwapConfig::from_mb(false, 8, 16, 128));
        assert!(store.acquire(4 << 20).is_none());
        assert_eq!(store.stats().live_frames, 0);
        assert_eq!(store.config().usable_slots(), 0);
    }

    #[test]
    fn reconfiguring_does_not_disturb_a_frame_in_flight() {
        // Two megabytes sits above the threshold for both the old 4 MB slots
        // (one megabyte) and the new 8 MB ones (two).
        let store = store(2, 4, 16);
        let frame = store_frame(&store, 2 << 20, 5).expect("slot");
        store.reconfigure(SwapConfig::from_mb(true, 4, 8, 64));
        assert_eq!(frame.as_slice(), pixels(2 << 20, 5).as_slice());
        assert_eq!(store.config().slot_bytes, 8 * 1024 * 1024);
        drop(frame);
        store.settle();
        assert_eq!(store.stats().mapped_bytes, 0);
        assert_eq!(store.stats().live_frames, 0);
        // The new size is what the next slot uses.
        let next = store_frame(&store, 2 << 20, 6).expect("a slot at the new size");
        assert_eq!(store.stats().mapped_bytes, 8 * 1024 * 1024);
        drop(next);
        store.settle();
    }

    #[test]
    fn the_configuration_reports_what_it_will_take() {
        let config = SwapConfig::from_mb(true, 8, 16, 128);
        assert!(config.takes(1 << 20));
        assert!(!config.takes(MIN_SWAP_BYTES - 1));
        assert!(!config.takes(256 * 1024 * 1024));
        let off = SwapConfig::from_mb(false, 8, 16, 128);
        assert!(!off.takes(1 << 20));
    }
}
