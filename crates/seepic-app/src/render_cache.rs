//! Built, drawable images, kept so a page can be shown again without rebuilding.
//!
//! Every visit to a page used to construct a fresh `RenderImage`. GPUI gives
//! each one a new identity, and the DirectX sprite atlas allocates a *new
//! texture* for an identity it has not seen: its `remove` frees the tile inside
//! the texture and keeps the texture itself on a free list rather than
//! destroying it. Measured on the reference machine, paging back and forth
//! between two images committed another ~10 MB per page turn and never gave it
//! back, while the pixels on screen never changed.
//!
//! Caching the built image by what it was built *from* means the same identity
//! is drawn again, so the atlas entry is reused and nothing is committed. It
//! also makes coming back to a page cost no rebuild at all.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::RenderImage;
use seepic_core::Size as IvSize;
use seepic_core::cache::{CacheKey, Detail};
use seepic_core::effects::Adjustments;
use seepic_core::filters::FilterMode;
use seepic_core::orientation::Orientation;

/// Everything that decides what a built image looks like beyond its pixels.
///
/// A rotation, a filter or a background change leaves the source key untouched
/// but makes the built pixels wrong, so they are part of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Transform {
    pub orientation: Orientation,
    pub adjustments: Adjustments,
    pub filter: FilterMode,
    /// Whether the alpha channel was forced opaque.
    pub flatten_alpha: bool,
}

/// Which frames of an animation a built image holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FramePick {
    /// Every frame, so GPUI plays the animation itself.
    All,
    /// One frame, held on screen.
    One(usize),
}

/// What a built image was made from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderKey {
    pub source: CacheKey,
    pub transform: Transform,
    pub frame: FramePick,
}

impl RenderKey {
    pub fn new(source: CacheKey, transform: Transform, frame: FramePick) -> Self {
        Self {
            source,
            transform,
            frame,
        }
    }
}

/// What a built image costs the sprite atlas, in bytes.
///
/// The atlas packs images into square power-of-two textures, and an image that
/// cannot share a texture takes one of its own — a 2000x1333 photograph needs a
/// 2048x2048 texture, which is 16.8 MB against the 10.6 MB of the pixels
/// themselves. Budgeting by pixel count would therefore under-count the process
/// cost by half again, which is exactly how a cache with a 128 MB budget came
/// to hold 200 MB of texture. Charging each image the texture it needs makes
/// the budget mean something in process terms.
///
/// Small images share a texture, so this over-counts them; that is the safe
/// direction to be wrong in, and it only matters for images whose whole point
/// is that they are cheap.
fn atlas_bytes(size: IvSize) -> usize {
    /// The atlas' own limit, past which an image is split across textures.
    const MAX_SIDE: u32 = 8192;

    let longest = size.w.max(size.h).max(1.).ceil() as u32;
    let side = longest.next_power_of_two().min(MAX_SIDE);
    (side as usize) * (side as usize) * 4
}

struct Entry {
    render: Arc<RenderImage>,
    size: IvSize,
    bytes: usize,
    /// The decode this was built from, so a coarser one can be replaced.
    detail: Detail,
    used: u64,
}

/// A byte-bounded, least-recently-used cache of built images.
pub struct RenderCache {
    entries: HashMap<RenderKey, Entry>,
    bytes: usize,
    budget: usize,
    clock: u64,
    /// The image on screen, which must never be evicted while it is drawn.
    pinned: Option<RenderKey>,
    hits: usize,
    misses: usize,
}

impl RenderCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            budget: budget_bytes,
            clock: 0,
            pinned: None,
            hits: 0,
            misses: 0,
        }
    }

    pub fn used(&self) -> usize {
        self.bytes
    }

    /// How many built images are held; the tests assert on this.
    #[allow(dead_code)]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Hit and miss counts; the tests assert on this.
    #[allow(dead_code)]
    pub fn stats(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    /// How much memory the cache is allowed, evicting straight away if the new
    /// budget is below what is already held.
    ///
    /// Returns the images that were evicted, so the caller can hand their
    /// textures back to the atlas — this type deliberately knows nothing about
    /// GPUI's context.
    pub fn set_budget(&mut self, budget: usize) -> Vec<Arc<RenderImage>> {
        self.budget = budget;
        self.evict()
    }

    /// Keep `key` resident whatever else has to go.
    pub fn pin(&mut self, key: Option<RenderKey>) {
        self.pinned = key;
    }

    /// Look a built image up, marking it as most recently used.
    pub fn get(&mut self, key: &RenderKey) -> Option<(Arc<RenderImage>, IvSize)> {
        self.clock += 1;
        let clock = self.clock;
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.used = clock;
                self.hits += 1;
                Some((entry.render.clone(), entry.size))
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Whether `have` is a decode at least as detailed as `needed`.
    ///
    /// `None` means the full image, which satisfies any request; otherwise the
    /// figure is the longest side the decode was reduced to.
    fn at_least_as_fine(have: Detail, needed: Detail) -> bool {
        match (have, needed) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(have), Some(needed)) => have >= needed,
        }
    }

    /// Look a built image up, but only accept it if it came from a decode at
    /// least as detailed as `needed`.
    ///
    /// Zooming past what the texture holds re-decodes the file at full
    /// resolution, and the coarser render already in the cache must not be
    /// handed back for it.
    pub fn get_if_covers(&mut self, key: &RenderKey, needed: Detail) -> Option<(Arc<RenderImage>, IvSize)> {
        let covered = self
            .entries
            .get(key)
            .is_some_and(|entry| Self::at_least_as_fine(entry.detail, needed));
        if !covered {
            return None;
        }
        self.get(key)
    }

    /// Store a built image, evicting to fit.
    ///
    /// Returns the images that were evicted. An image larger than the whole
    /// budget is not stored, because it would evict everything and still not
    /// fit.
    pub fn insert(
        &mut self,
        key: RenderKey,
        render: Arc<RenderImage>,
        size: IvSize,
        detail: Detail,
    ) -> Vec<Arc<RenderImage>> {
        let bytes = atlas_bytes(size);
        if self.budget == 0 || bytes > self.budget {
            return Vec::new();
        }

        self.clock += 1;
        let used = self.clock;
        let mut evicted = Vec::new();

        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
            // The replaced image may still be the one on screen; the caller's
            // own reference decides, so it is only handed back when the cache
            // was its only owner.
            if Arc::strong_count(&old.render) == 1 {
                evicted.push(old.render);
            }
        }

        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                render,
                size,
                bytes,
                detail,
                used,
            },
        );
        evicted.extend(self.evict());
        evicted
    }

    /// Drop least-recently-used entries until the budget is met.
    fn evict(&mut self) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        while self.bytes > self.budget {
            let Some(victim) = self
                .entries
                .iter()
                .filter(|(key, _)| Some(*key) != self.pinned.as_ref())
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                // Everything left is pinned.
                break;
            };
            let Some(entry) = self.entries.remove(&victim) else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            if Arc::strong_count(&entry.render) == 1 {
                evicted.push(entry.render);
            }
        }
        evicted
    }

    /// Forget everything, returning the images so their textures can be freed.
    pub fn clear(&mut self) -> Vec<Arc<RenderImage>> {
        self.pinned = None;
        self.bytes = 0;
        self.entries
            .drain()
            .map(|(_, entry)| entry.render)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seepic_core::Size as CoreSize;

    fn key(index: usize) -> RenderKey {
        RenderKey::new(
            CacheKey::new("c", index),
            Transform {
                orientation: Orientation::default(),
                adjustments: Adjustments::default(),
                filter: FilterMode::default(),
                flatten_alpha: false,
            },
            FramePick::All,
        )
    }

    /// A `RenderImage` of a known pixel size.
    ///
    /// Its cache cost is `atlas_bytes`, which rounds the longest side up to a
    /// power of two, so a 64x64 image costs 64x64x4 = 16 KB.
    fn render(width: u32, height: u32) -> Arc<RenderImage> {
        let buffer = image::RgbaImage::new(width, height);
        let frame = image::Frame::from_parts(
            buffer,
            0,
            0,
            image::Delay::from_numer_denom_ms(100, 1),
        );
        Arc::new(RenderImage::new(vec![frame]))
    }

    fn size(width: f32, height: f32) -> IvSize {
        CoreSize::new(width, height)
    }

    #[test]
    fn a_stored_image_comes_back_without_rebuilding() {
        let mut cache = RenderCache::new(64 * 1024 * 1024);
        let image = render(64, 64);
        cache.insert(key(0), image.clone(), size(64., 64.), None);

        let (found, found_size) = cache.get(&key(0)).expect("must be a hit");
        assert!(Arc::ptr_eq(&found, &image), "the same texture must be reused");
        assert_eq!(found_size, size(64., 64.));
        assert_eq!(cache.stats(), (1, 0));
    }

    #[test]
    fn a_different_transform_is_a_different_entry() {
        let mut cache = RenderCache::new(64 * 1024 * 1024);
        cache.insert(key(0), render(16, 16), size(16., 16.), None);

        let mut rotated = key(0);
        rotated.transform.orientation.flip_h = true;
        assert!(cache.get(&rotated).is_none());
        assert_eq!(cache.stats(), (0, 1));
    }

    #[test]
    fn a_coarse_decode_does_not_cover_a_finer_request() {
        let mut cache = RenderCache::new(64 * 1024 * 1024);
        cache.insert(key(0), render(16, 16), size(16., 16.), Some(512));

        assert!(cache.get_if_covers(&key(0), Some(512)).is_some());
        assert!(
            cache.get_if_covers(&key(0), Some(2048)).is_none(),
            "512 is not enough for 2048, so the finer decode must rebuild"
        );
        assert!(
            cache.get_if_covers(&key(0), None).is_none(),
            "and neither is it enough for full resolution"
        );
        // A full-resolution entry satisfies everything.
        cache.insert(key(1), render(16, 16), size(16., 16.), None);
        assert!(cache.get_if_covers(&key(1), None).is_some());
        assert!(cache.get_if_covers(&key(1), Some(4096)).is_some());
    }

    #[test]
    fn the_least_recently_used_entry_is_evicted_first() {
        // Room for two 64x64 images (16 KB each) and no more.
        let mut cache = RenderCache::new(40 * 1024);
        cache.insert(key(0), render(64, 64), size(64., 64.), None);
        cache.insert(key(1), render(64, 64), size(64., 64.), None);
        // Touch the first so the second becomes the oldest.
        assert!(cache.get(&key(0)).is_some());

        let evicted = cache.insert(key(2), render(64, 64), size(64., 64.), None);
        assert_eq!(evicted.len(), 1, "one image must be handed back");
        assert!(cache.get(&key(1)).is_none(), "the oldest must be gone");
        assert!(cache.get(&key(0)).is_some(), "the recently used must survive");
        assert!(cache.get(&key(2)).is_some());
    }

    #[test]
    fn the_pinned_entry_is_never_evicted() {
        let mut cache = RenderCache::new(40 * 1024);
        cache.insert(key(0), render(64, 64), size(64., 64.), None);
        cache.pin(Some(key(0)));
        cache.insert(key(1), render(64, 64), size(64., 64.), None);

        // Eviction has to take the unpinned one, even though it is newer.
        assert!(cache.get(&key(0)).is_some(), "the pinned image must stay");
    }

    #[test]
    fn an_image_larger_than_the_budget_is_not_stored() {
        let mut cache = RenderCache::new(1024);
        let evicted = cache.insert(key(0), render(64, 64), size(64., 64.), None);
        assert!(evicted.is_empty());
        assert!(cache.get(&key(0)).is_none());
        assert_eq!(cache.used(), 0);
    }

    #[test]
    fn shrinking_the_budget_evicts_down_to_it() {
        let mut cache = RenderCache::new(64 * 1024);
        cache.insert(key(0), render(64, 64), size(64., 64.), None);
        cache.insert(key(1), render(64, 64), size(64., 64.), None);
        assert!(cache.used() > 8 * 1024);

        let evicted = cache.set_budget(8 * 1024);
        assert!(!evicted.is_empty());
        assert!(cache.used() <= 8 * 1024);
    }

    #[test]
    fn replacing_an_entry_hands_the_old_image_back_when_sole_owner() {
        let mut cache = RenderCache::new(64 * 1024 * 1024);
        cache.insert(key(0), render(32, 32), size(32., 32.), Some(1024));
        let evicted = cache.insert(key(0), render(32, 32), size(32., 32.), Some(2048));

        assert_eq!(evicted.len(), 1, "the superseded texture must be freed");
        assert!(cache.get_if_covers(&key(0), Some(2048)).is_some());
        assert_eq!(cache.len(), 1, "and must not linger as a second entry");
    }

    #[test]
    fn clearing_returns_everything_held() {
        let mut cache = RenderCache::new(64 * 1024 * 1024);
        cache.insert(key(0), render(16, 16), size(16., 16.), None);
        cache.insert(key(1), render(16, 16), size(16., 16.), None);

        assert_eq!(cache.clear().len(), 2);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.used(), 0);
    }
}
