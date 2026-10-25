//! An LRU cache for decoded images.
//!
//! Decoding is the expensive part of navigation, so the decoded pixels are what
//! gets cached; GPUI's `RenderImage` cannot be stored here because it is not
//! `Send`. Entries are evicted least-recently-used first once the byte budget is
//! exceeded, so the cache cannot grow without bound.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::decode::DecodedImage;

/// Identifies one image: which container, and which entry inside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Folder or archive the image came from.
    pub container: PathBuf,
    /// Index into that container's entry list.
    pub index: usize,
}

impl CacheKey {
    pub fn new(container: impl AsRef<Path>, index: usize) -> Self {
        Self {
            container: container.as_ref().to_path_buf(),
            index,
        }
    }
}

/// The detail a cached image carries: the longest side it was reduced to.
///
/// `None` means the full resolution, which satisfies any request. Storing this
/// is what lets a zoomed-in view ask for more detail than the cache holds
/// instead of drawing a soft, upscaled image.
pub type Detail = Option<u32>;

/// A decoded image plus the size of the bytes it came from.
#[derive(Clone)]
pub struct CachedImage {
    pub image: Arc<DecodedImage>,
    /// Size of the encoded source, for the info panel.
    pub byte_size: u64,
    /// The longest side this image was reduced to, if it was.
    pub detail: Detail,
}

impl CachedImage {
    /// The detail this image was built at.
    pub fn longest_side(&self) -> u32 {
        self.image.width.max(self.image.height)
    }

    /// Whether this image can be drawn at `needed_side` without sampling up.
    ///
    /// A reduction to `None` is the full image, so it always qualifies.
    pub fn covers(&self, needed_side: u32) -> bool {
        match self.detail {
            None => true,
            Some(reduced_to) => reduced_to >= needed_side,
        }
    }
}

/// What an image costs the cache.
///
/// Charged the pixels it actually holds, in either backend. It used to be
/// charged the whole slot for a mapped frame, on the reasoning that the store
/// reserved that much for it — but reserving disk is not what the budget is
/// for, and it made an eight-slot store hold only eight pages when each page
/// uses half a slot, so the freshly prepared pages ahead of the cursor were
/// evicted by the pages just read. The store's slot count still bounds how many
/// frames can be mapped; the cache's budget bounds the bytes.
pub fn decoded_size(image: &DecodedImage) -> usize {
    image
        .frames
        .iter()
        .map(|frame| frame.bgra.len())
        .sum()
}

/// A size-bounded, least-recently-used cache.
#[derive(Default)]
pub struct ImageCache {
    capacity: usize,
    /// Most entries to keep, regardless of the byte budget.
    max_entries: usize,
    /// The container index in view, so eviction can keep the pages around it.
    focus: Option<usize>,
    used: usize,
    entries: HashMap<CacheKey, CachedImage>,
    /// Most recently used at the back.
    recency: Vec<CacheKey>,
    hits: usize,
    misses: usize,
}

impl ImageCache {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity: capacity_bytes,
            max_entries: usize::MAX,
            ..Default::default()
        }
    }

    /// Tell the cache which page is in view.
    ///
    /// Eviction then drops the pages farthest from it rather than the least
    /// recently used: while a held key runs forward, the prepared pages ahead
    /// are the ones worth keeping, and the least recently used page is the one
    /// just read.
    pub fn set_focus(&mut self, index: Option<usize>) {
        self.focus = index;
    }

    /// Cap the number of entries, evicting the least recently used if needed.
    ///
    /// The byte budget alone lets a folder of small pages fill the cache with
    /// more entries than the pages around the cursor — which are the ones a
    /// reader flips back to.
    pub fn set_max_entries(&mut self, entries: usize) {
        self.max_entries = entries.max(1);
        self.evict_to_fit();
    }

    /// Change the budget, evicting immediately if the new one is smaller.
    pub fn set_capacity(&mut self, capacity_bytes: usize) {
        self.capacity = capacity_bytes;
        self.evict_to_fit();
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look an image up, marking it as most recently used.
    pub fn get(&mut self, key: &CacheKey) -> Option<CachedImage> {
        let Some(entry) = self.entries.get(key).cloned() else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        self.touch(key);
        Some(entry)
    }

    /// Whether the key is present, without affecting recency or statistics.
    pub fn contains(&self, key: &CacheKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Look an image up, but only if it carries enough detail for the view.
    ///
    /// A cache hit that is too coarse is treated as a miss so the caller
    /// re-decodes at full resolution and replaces it; drawing the coarser copy
    /// would show a visibly soft image at 100%.
    pub fn get_for_detail(&mut self, key: &CacheKey, needed_side: u32) -> Option<CachedImage> {
        let covers = self
            .entries
            .get(key)
            .is_some_and(|entry| entry.covers(needed_side));
        if !covers {
            self.misses += 1;
            return None;
        }
        self.get(key)
    }

    /// Store an image, evicting older entries to stay within budget.
    ///
    /// An image larger than the whole budget is not stored at all: keeping it
    /// would evict everything else and still be over the limit.
    pub fn insert(&mut self, key: CacheKey, image: CachedImage) {
        let size = decoded_size(&image.image);
        if size > self.capacity || self.capacity == 0 {
            return;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.used = self.used.saturating_sub(decoded_size(&previous.image));
            self.recency.retain(|existing| existing != &key);
        }

        self.used += size;
        self.entries.insert(key.clone(), image);
        self.recency.push(key);
        self.evict_to_fit();
    }

    /// Drop the entry whose container index is farthest from `index`.
    ///
    /// The swap store charges a mapped frame its whole slot, so a full store
    /// means the cache holds as many frames as there are slots — and since the
    /// cache only evicts when something is inserted, and an insert needs a slot,
    /// nothing more can ever be stored. The viewer calls this before decoding
    /// the visible page when the store is full, so the slot taken by the page
    /// the reader is least likely to want back is the one that is freed.
    ///
    /// Returns whether an entry was dropped.
    pub fn evict_farthest_from(&mut self, index: usize) -> bool {
        let farthest = self
            .entries
            .keys()
            .max_by_key(|key| key.index.abs_diff(index))
            .cloned();
        let Some(key) = farthest else {
            return false;
        };
        self.recency.retain(|existing| existing != &key);
        if let Some(entry) = self.entries.remove(&key) {
            self.used = self.used.saturating_sub(decoded_size(&entry.image));
            return true;
        }
        false
    }

    /// Drop everything, e.g. when the user opens a different container.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.used = 0;
    }

    /// `(hits, misses)`, for the status readout and tests.
    pub fn stats(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    fn touch(&mut self, key: &CacheKey) {
        self.recency.retain(|existing| existing != key);
        self.recency.push(key.clone());
    }

    fn evict_to_fit(&mut self) {
        while self.used > self.capacity || self.entries.len() > self.max_entries {
            let Some(victim) = self.farthest() else {
                break;
            };
            self.recency.retain(|existing| existing != &victim);
            if let Some(entry) = self.entries.remove(&victim) {
                self.used = self.used.saturating_sub(decoded_size(&entry.image));
            }
        }
    }

    /// The entry to drop: farthest from the page in view, oldest on a tie.
    ///
    /// With no page in view — before the first image is adopted — this is plain
    /// least-recently-used.
    fn farthest(&self) -> Option<CacheKey> {
        let Some(focus) = self.focus else {
            return self.recency.first().cloned();
        };
        // Rank by distance, then by age (the front of `recency` is oldest).
        self.entries
            .keys()
            .enumerate()
            .max_by_key(|(_, key)| {
                let age = self
                    .recency
                    .iter()
                    .position(|existing| existing == *key)
                    .unwrap_or(0);
                (key.index.abs_diff(focus), self.recency.len().saturating_sub(age))
            })
            .map(|(_, key)| key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::DecodedFrame;

    /// A decoded image whose pixel buffer is exactly `bytes` long.
    fn image(bytes: usize) -> Arc<DecodedImage> {
        Arc::new(DecodedImage {
            width: (bytes / 4) as u32,
            height: 1,
            frames: vec![DecodedFrame {
                bgra: Arc::new(vec![0u8; bytes].into()),
                delay_ms: 0,
            }],
            animated: false,
            exif_orientation: None,
            reduced_from: None,
        })
    }

    fn entry(bytes: usize) -> CachedImage {
        CachedImage {
            image: image(bytes),
            byte_size: bytes as u64,
            detail: None,
        }
    }

    /// An entry standing in for an image reduced to `side`.
    fn reduced(bytes: usize, side: u32) -> CachedImage {
        CachedImage {
            detail: Some(side),
            ..entry(bytes)
        }
    }

    #[test]
    fn stores_and_returns_an_image() {
        let mut cache = ImageCache::new(1024);
        let key = CacheKey::new("a.cbz", 3);
        cache.insert(key.clone(), entry(100));

        let got = cache.get(&key).expect("should be cached");
        assert_eq!(decoded_size(&got.image), 100);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used(), 100);
    }

    #[test]
    fn reports_misses() {
        let mut cache = ImageCache::new(1024);
        assert!(cache.get(&CacheKey::new("a.cbz", 0)).is_none());
        assert_eq!(cache.stats(), (0, 1));
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let mut cache = ImageCache::new(300);
        let (a, b, c) = (
            CacheKey::new("x", 0),
            CacheKey::new("x", 1),
            CacheKey::new("x", 2),
        );
        cache.insert(a.clone(), entry(100));
        cache.insert(b.clone(), entry(100));
        cache.insert(c.clone(), entry(100));

        // Touch `a` so `b` becomes the oldest.
        assert!(cache.get(&a).is_some());
        cache.insert(CacheKey::new("x", 3), entry(100));

        assert!(cache.contains(&a), "recently used entry must survive");
        assert!(!cache.contains(&b), "oldest entry should be evicted");
        assert!(cache.contains(&c));
        assert!(cache.used() <= 300);
    }

    #[test]
    fn an_oversized_image_is_not_stored() {
        let mut cache = ImageCache::new(100);
        cache.insert(CacheKey::new("x", 0), entry(500));
        assert!(cache.is_empty(), "must not blow the budget");
        assert_eq!(cache.used(), 0);
    }

    #[test]
    fn a_zero_budget_disables_caching() {
        let mut cache = ImageCache::new(0);
        cache.insert(CacheKey::new("x", 0), entry(1));
        assert!(cache.is_empty());
    }

    #[test]
    fn shrinking_the_budget_evicts_immediately() {
        let mut cache = ImageCache::new(1000);
        for index in 0..5 {
            cache.insert(CacheKey::new("x", index), entry(100));
        }
        assert_eq!(cache.len(), 5);

        cache.set_capacity(250);
        assert!(cache.used() <= 250, "used was {}", cache.used());
        assert!(cache.len() <= 3);
    }

    #[test]
    fn growing_the_budget_keeps_entries() {
        let mut cache = ImageCache::new(300);
        cache.insert(CacheKey::new("x", 0), entry(100));
        cache.set_capacity(3000);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.capacity(), 3000);
    }

    #[test]
    fn reinserting_the_same_key_does_not_double_count() {
        let mut cache = ImageCache::new(1000);
        let key = CacheKey::new("x", 0);
        cache.insert(key.clone(), entry(100));
        cache.insert(key.clone(), entry(100));

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used(), 100, "usage must not accumulate");
    }

    #[test]
    fn clear_frees_everything() {
        let mut cache = ImageCache::new(1000);
        cache.insert(CacheKey::new("x", 0), entry(100));
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.used(), 0);
    }

    #[test]
    fn a_reduced_entry_serves_a_view_that_needs_less_detail() {
        let mut cache = ImageCache::new(1000);
        let key = CacheKey::new("x", 0);
        cache.insert(key.clone(), reduced(100, 1200));

        let got = cache
            .get_for_detail(&key, 800)
            .expect("1200px of detail covers an 800px draw");
        assert_eq!(got.detail, Some(1200));
    }

    #[test]
    fn a_reduced_entry_is_a_miss_when_more_detail_is_needed() {
        let mut cache = ImageCache::new(1000);
        let key = CacheKey::new("x", 0);
        cache.insert(key.clone(), reduced(100, 1200));

        assert!(
            cache.get_for_detail(&key, 3000).is_none(),
            "zooming in must force a full re-decode"
        );
        // The coarse copy is still there for a zoom back out.
        assert!(cache.get_for_detail(&key, 900).is_some());
    }

    #[test]
    fn a_full_resolution_entry_covers_everything() {
        let mut cache = ImageCache::new(1000);
        let key = CacheKey::new("x", 0);
        cache.insert(key.clone(), entry(100));
        assert!(cache.get_for_detail(&key, 100_000).is_some());
    }

    #[test]
    fn keys_distinguish_containers() {
        let mut cache = ImageCache::new(1000);
        let a = CacheKey::new("one.cbz", 0);
        let b = CacheKey::new("two.cbz", 0);
        cache.insert(a.clone(), entry(100));

        assert!(cache.contains(&a));
        assert!(!cache.contains(&b), "same index, different container");
    }
}
