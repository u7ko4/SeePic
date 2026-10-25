//! Ordering and navigation over the images inside one container.

use std::cmp::Ordering;
use std::time::SystemTime;

use crate::geom::Size;

/// Which field the image list is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    DateModified,
    DateCreated,
    Size,
    Type,
    /// EXIF capture date.
    DateTaken,
    /// Order by the platform's own comparison of names, which splits them into
    /// digit and text runs rather than using a locale collation.
    Logical,
}

impl SortKey {
    pub const ALL: [SortKey; 7] = [
        SortKey::Name,
        SortKey::Logical,
        SortKey::DateModified,
        SortKey::DateCreated,
        SortKey::Size,
        SortKey::Type,
        SortKey::DateTaken,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "名称",
            SortKey::DateModified => "修改日期",
            SortKey::DateCreated => "创建日期",
            SortKey::Size => "大小",
            SortKey::Type => "类型",
            SortKey::DateTaken => "拍摄日期",
            SortKey::Logical => "逻辑顺序（数字按数值）",
        }
    }
}

/// A sort field plus direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SortOrder {
    pub key: SortKey,
    pub descending: bool,
}

impl SortOrder {
    pub fn new(key: SortKey) -> Self {
        Self {
            key,
            descending: false,
        }
    }

    /// Select a key, toggling direction if it is already active.
    pub fn toggle(&mut self, key: SortKey) {
        if self.key == key {
            self.descending = !self.descending;
        } else {
            self.key = key;
            self.descending = false;
        }
    }
}

/// What to do when stepping past the first or last image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BoundaryAction {
    /// Stay put.
    #[default]
    DoNothing,
    /// Ask the user, in the UI layer.
    Ask,
    /// Jump to the other end.
    Wrap,
    /// Leave this container for the next/previous one.
    NextContainer,
}

impl BoundaryAction {
    pub const ALL: [BoundaryAction; 4] = [
        BoundaryAction::DoNothing,
        BoundaryAction::Ask,
        BoundaryAction::Wrap,
        BoundaryAction::NextContainer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BoundaryAction::DoNothing => "无操作",
            BoundaryAction::Ask => "询问",
            BoundaryAction::Wrap => "转到首个文件",
            BoundaryAction::NextContainer => "转到下一个文件夹或文件",
        }
    }
}

/// One image inside a container, whether it lives on disk or in an archive.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Display name. For archive members this is the path inside the archive.
    pub name: String,
    /// Uncompressed size in bytes, when known.
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    /// EXIF capture time, when known.
    pub taken: Option<SystemTime>,
    /// Pixel dimensions, once the header has been read.
    pub dimensions: Option<Size>,
}

impl Entry {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            size: 0,
            modified: None,
            created: None,
            taken: None,
            dimensions: None,
        }
    }

    /// The name without any directory part, for display.
    pub fn file_name(&self) -> &str {
        self.name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.name)
    }

    fn extension(&self) -> &str {
        self.file_name()
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .unwrap_or("")
    }

    /// Order against another entry under `order`. The name is always the final
    /// tiebreak so the result is total and stable.
    fn cmp_with(&self, other: &Self, order: SortOrder) -> Ordering {
        let primary = match order.key {
            SortKey::Name => Ordering::Equal,
            // The platform's own comparison is the whole key here, so the
            // natural tiebreak below must not run as well.
            SortKey::Logical => {
                let ordering = crate::natsort::logical_cmp(self.file_name(), other.file_name());
                return if order.descending {
                    ordering.reverse()
                } else {
                    ordering
                };
            }
            SortKey::Size => self.size.cmp(&other.size),
            SortKey::Type => crate::natsort::cmp_ignore_case(self.extension(), other.extension()),
            SortKey::DateModified => cmp_time(self.modified, other.modified),
            SortKey::DateCreated => cmp_time(self.created, other.created),
            SortKey::DateTaken => cmp_time(self.taken, other.taken),
        };

        let primary = if order.descending {
            primary.reverse()
        } else {
            primary
        };

        primary.then_with(|| {
            crate::natsort::natural_cmp(self.file_name(), other.file_name())
        })
    }
}

/// Compare optional timestamps, sorting entries with no timestamp last.
fn cmp_time(a: Option<SystemTime>, b: Option<SystemTime>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The result of a navigation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The cursor moved.
    Moved,
    /// The cursor was already at an end and did not move.
    Blocked,
}

impl Step {
    pub fn moved(self) -> bool {
        matches!(self, Step::Moved)
    }
}

/// The ordered image list of one container, plus a cursor into it.
///
/// Entries carry a *source index*: their position in the container's own list.
/// Sorting reorders what the user sees without disturbing how the container is
/// read, so the two can never drift apart.
#[derive(Debug, Clone)]
pub struct Navigator {
    entries: Vec<Entry>,
    /// Parallel to `entries`: the container index each displayed entry came from.
    sources: Vec<usize>,
    cursor: usize,
    order: SortOrder,
    boundary: BoundaryAction,
}

impl Default for Navigator {
    fn default() -> Self {
        Self::new()
    }
}

impl Navigator {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            sources: Vec::new(),
            cursor: 0,
            order: SortOrder::default(),
            boundary: BoundaryAction::default(),
        }
    }

    pub fn with_order(order: SortOrder, boundary: BoundaryAction) -> Self {
        Self {
            order,
            boundary,
            ..Self::new()
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn order(&self) -> SortOrder {
        self.order
    }

    pub fn boundary(&self) -> BoundaryAction {
        self.boundary
    }

    pub fn set_boundary(&mut self, boundary: BoundaryAction) {
        self.boundary = boundary;
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    /// The container index behind the entry at `position`.
    pub fn source_index(&self, position: usize) -> Option<usize> {
        self.sources.get(position).copied()
    }

    /// The container index of the entry under the cursor.
    ///
    /// This is what to hand to the container when reading, because sorting the
    /// list does not reorder the container itself.
    pub fn current_source(&self) -> Option<usize> {
        self.source_index(self.cursor)
    }

    /// The display position showing container index `source`.
    ///
    /// The inverse of [`Self::source_index`]. A decoded image carries a
    /// container index, so matching it against a slot in a two-page spread
    /// needs this translation: a comic whose members are stored in
    /// lexicographic order is displayed in natural order, and the two differ.
    pub fn position_of_source(&self, source: usize) -> Option<usize> {
        self.sources.iter().position(|slot| *slot == source)
    }

    /// Replace the entry list, treating positions as container indices.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        let sources = (0..entries.len()).collect();
        self.set_items(entries, sources);
    }

    /// Replace the entry list with an explicit position-to-container mapping.
    pub fn set_items(&mut self, entries: Vec<Entry>, sources: Vec<usize>) {
        let sources = if entries.len() != sources.len() {
            // A mismatched mapping would silently read the wrong image, so fall
            // back to the identity rather than trusting it.
            (0..entries.len()).collect()
        } else {
            sources
        };

        // Captured before sorting, because sorting moves the entries around.
        let anchor = self.anchor();
        let (entries, sources) = self.sort(entries, sources);
        self.entries = entries;
        self.sources = sources;
        self.restore_cursor(anchor);
    }

    /// Change the sort order, keeping the cursor on the same image.
    pub fn set_order(&mut self, order: SortOrder) {
        if order == self.order {
            return;
        }
        self.order = order;

        // `sort` needs ownership, so take the lists apart here rather than
        // routing through `set_items`, which would capture the anchor too late.
        let anchor = self.anchor();
        let entries = std::mem::take(&mut self.entries);
        let sources = std::mem::take(&mut self.sources);
        let (entries, sources) = self.sort(entries, sources);
        self.entries = entries;
        self.sources = sources;
        self.restore_cursor(anchor);
    }

    /// What the cursor is currently pointing at, in a form that survives
    /// reordering.
    fn anchor(&self) -> Option<(String, usize, usize)> {
        let entry = self.current()?;
        Some((
            entry.name.clone(),
            self.current_source().unwrap_or(self.cursor),
            self.cursor,
        ))
    }

    /// Put the cursor back on the same image, in order of preference: the same
    /// name, then the same container slot, then the same position clamped into
    /// range.
    fn restore_cursor(&mut self, anchor: Option<(String, usize, usize)>) {
        let Some((name, source, position)) = anchor else {
            self.cursor = 0;
            return;
        };

        self.cursor = self
            .entries
            .iter()
            .position(|entry| entry.name == name)
            .or_else(|| self.sources.iter().position(|slot| *slot == source))
            .unwrap_or(position)
            .min(self.entries.len().saturating_sub(1));
    }

    /// Reorder both lists into display order, moving rather than cloning.
    ///
    /// The obvious implementation clones every `Entry` into the new order,
    /// which allocates a `String` per entry; on a folder of tens of thousands
    /// of files that is the whole cost of a sort. Taking the lists by value
    /// and permuting them in place keeps a re-sort allocation-free per entry.
    fn sort(&self, entries: Vec<Entry>, sources: Vec<usize>) -> (Vec<Entry>, Vec<usize>) {
        let order = self.order;
        // `cmp_with` already falls back to a natural comparison of the name, so
        // a stable sort keeps same-key entries in whatever order the container
        // listed them, which is the sensible tiebreak for archive members.
        let mut permutation: Vec<usize> = (0..entries.len()).collect();
        permutation.sort_by(|a, b| entries[*a].cmp_with(&entries[*b], order));

        let mut slots: Vec<Option<Entry>> = entries.into_iter().map(Some).collect();
        let sorted_entries = permutation
            .iter()
            .map(|index| {
                slots[*index]
                    .take()
                    .expect("a permutation visits every index once")
            })
            .collect();
        let sorted_sources = permutation.iter().map(|index| sources[*index]).collect();
        (sorted_entries, sorted_sources)
    }

    /// Move the cursor to an absolute index, clamped into range.
    pub fn select(&mut self, index: usize) -> Step {
        if self.entries.is_empty() {
            return Step::Blocked;
        }
        let clamped = index.min(self.entries.len() - 1);
        if clamped == self.cursor {
            return Step::Blocked;
        }
        self.cursor = clamped;
        Step::Moved
    }

    /// Move by `delta`, honouring the boundary policy.
    pub fn step(&mut self, delta: isize) -> Step {
        if self.entries.is_empty() {
            return Step::Blocked;
        }

        let len = self.entries.len() as isize;
        let target = self.cursor as isize + delta;

        if target >= 0 && target < len {
            self.cursor = target as usize;
            return Step::Moved;
        }

        match self.boundary {
            BoundaryAction::Wrap => {
                // Wrap within a single step's magnitude so ±10 on a 3-image
                // list still lands somewhere sensible.
                self.cursor = target.rem_euclid(len) as usize;
                Step::Moved
            }
            BoundaryAction::DoNothing | BoundaryAction::Ask | BoundaryAction::NextContainer => {
                Step::Blocked
            }
        }
    }

    /// Jump to the first or last entry, whatever the boundary setting says.
    ///
    /// The "go to the other end" answer to the prompt has to move even when the
    /// boundary is set to ask, so it cannot go through [`Self::step`].
    pub fn wrap_to_end(&mut self, forward: bool) -> Step {
        if self.entries.is_empty() {
            return Step::Blocked;
        }
        let target = if forward { 0 } else { self.entries.len() - 1 };
        if target == self.cursor {
            return Step::Blocked;
        }
        self.cursor = target;
        Step::Moved
    }

    pub fn next(&mut self) -> Step {
        self.step(1)
    }

    pub fn prev(&mut self) -> Step {
        self.step(-1)
    }

    pub fn first(&mut self) -> Step {
        self.select(0)
    }

    pub fn last(&mut self) -> Step {
        if self.entries.is_empty() {
            return Step::Blocked;
        }
        self.select(self.entries.len() - 1)
    }

    /// Jump to the next image, which may be `count` ahead.
    pub fn step_by(&mut self, count: usize) -> Step {
        self.step(count as isize)
    }

    /// Jump backwards by `count`.
    pub fn step_back_by(&mut self, count: usize) -> Step {
        self.step(-(count as isize))
    }

    /// True when the cursor sits on the last image.
    pub fn at_last(&self) -> bool {
        self.entries.is_empty() || self.cursor + 1 >= self.entries.len()
    }

    /// True when the cursor sits on the first image.
    pub fn at_first(&self) -> bool {
        self.cursor == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64) -> Entry {
        Entry {
            size,
            ..Entry::new(name)
        }
    }

    fn nav(names: &[&str], order: SortOrder) -> Navigator {
        let mut n = Navigator::with_order(order, BoundaryAction::DoNothing);
        n.set_entries(names.iter().map(|s| entry(s, 0)).collect());
        n
    }

    #[test]
    fn sorts_naturally_by_default() {
        let n = nav(&["p10.jpg", "p2.jpg", "p1.jpg"], SortOrder::default());
        let names: Vec<&str> = n.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["p1.jpg", "p2.jpg", "p10.jpg"]);
    }

    #[test]
    fn wrapping_to_an_end_moves_and_reports_it() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        // Paging forward from the last image lands on the first.
        n.select(2);
        assert_eq!(n.wrap_to_end(true), Step::Moved);
        assert_eq!(n.cursor(), 0);
        // And paging backward from the first lands on the last.
        assert_eq!(n.wrap_to_end(false), Step::Moved);
        assert_eq!(n.cursor(), 2);
        // Already there: nothing to do, and the caller is told so.
        assert_eq!(n.wrap_to_end(false), Step::Blocked);
    }

    #[test]
    fn the_logical_key_orders_numerically() {
        let mut n = Navigator::with_order(
            SortOrder::new(SortKey::Logical),
            BoundaryAction::DoNothing,
        );
        n.set_entries(vec![
            entry("page_10.png", 0),
            entry("page_2.png", 0),
            entry("page_1.png", 0),
        ]);
        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["page_1.png", "page_2.png", "page_10.png"]
        );
    }

    #[test]
    fn every_sort_key_orders_the_same_list_without_losing_entries() {
        // A key that silently dropped or duplicated entries would be a nasty
        // bug to find later, so every key is checked on one list.
        let names = ["b.png", "a10.png", "a2.png", "c.png"];
        for key in SortKey::ALL {
            let mut n = Navigator::with_order(SortOrder::new(key), BoundaryAction::DoNothing);
            n.set_entries(names.iter().map(|name| entry(name, 0)).collect());
            let mut sorted: Vec<&str> =
                n.entries().iter().map(|e| e.name.as_str()).collect();
            sorted.sort_unstable();
            let mut expected = names.to_vec();
            expected.sort_unstable();
            assert_eq!(sorted, expected, "sort key {key:?} lost or duplicated entries");
        }
    }

    #[test]
    fn source_and_position_translate_both_ways() {
        // The container lists its members lexicographically, the list is shown
        // in natural order, so the two indices differ after the first page.
        let n = nav(&["p1.png", "p10.png", "p2.png"], SortOrder::default());
        let names: Vec<&str> = n.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["p1.png", "p2.png", "p10.png"]);

        assert_eq!(n.source_index(0), Some(0));
        assert_eq!(n.source_index(1), Some(2));
        assert_eq!(n.source_index(2), Some(1));

        assert_eq!(n.position_of_source(0), Some(0));
        assert_eq!(n.position_of_source(2), Some(1));
        assert_eq!(n.position_of_source(1), Some(2));
        assert_eq!(n.position_of_source(9), None);
    }

    #[test]
    fn sorts_by_size_both_directions() {
        let mut n = Navigator::with_order(SortOrder::new(SortKey::Size), BoundaryAction::DoNothing);
        n.set_entries(vec![entry("a", 300), entry("b", 100), entry("c", 200)]);
        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );

        n.set_order(SortOrder {
            key: SortKey::Size,
            descending: true,
        });
        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "c", "b"]
        );
    }

    #[test]
    fn changing_sort_keeps_the_current_image() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        n.select(2);
        assert_eq!(n.current().unwrap().name, "c");

        n.set_order(SortOrder::new(SortKey::Size));
        assert_eq!(n.current().unwrap().name, "c", "cursor should follow the image");
    }

    #[test]
    fn replacing_entries_keeps_the_current_image() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        n.select(1);

        n.set_entries(vec![entry("z", 0), entry("b", 0), entry("y", 0)]);
        assert_eq!(n.current().unwrap().name, "b");
    }

    #[test]
    fn replacing_entries_falls_back_when_the_image_is_gone() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        n.select(2);

        n.set_entries(vec![entry("x", 0), entry("y", 0)]);
        assert_eq!(n.cursor(), 1, "should clamp into range");
        assert_eq!(n.current().unwrap().name, "y");
    }

    #[test]
    fn sorting_does_not_break_the_source_mapping() {
        // Regression: the container lists entries in its own order while the
        // navigator sorts them, so reading must go through the source index.
        // Getting this wrong shows one image's name over another image.
        let mut n = Navigator::new();
        n.set_entries(vec![entry("p10.png", 0), entry("p2.png", 0), entry("p1.png", 0)]);

        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["p1.png", "p2.png", "p10.png"]
        );
        // Display position -> container index.
        assert_eq!(n.source_index(0), Some(2));
        assert_eq!(n.source_index(1), Some(1));
        assert_eq!(n.source_index(2), Some(0));
        assert_eq!(n.current_source(), Some(2), "cursor starts on p1.png");
    }

    #[test]
    fn source_mapping_follows_the_cursor() {
        let mut n = Navigator::new();
        n.set_entries(vec![entry("p10.png", 0), entry("p2.png", 0), entry("p1.png", 0)]);

        n.next();
        assert_eq!(n.current().unwrap().name, "p2.png");
        assert_eq!(n.current_source(), Some(1));

        n.next();
        assert_eq!(n.current().unwrap().name, "p10.png");
        assert_eq!(n.current_source(), Some(0));
    }

    #[test]
    fn changing_sort_keeps_the_source_with_the_name() {
        let mut n = Navigator::new();
        n.set_items(
            vec![entry("a", 300), entry("b", 100), entry("c", 200)],
            vec![10, 11, 12],
        );
        assert_eq!(n.source_index(0), Some(10));

        n.set_order(SortOrder::new(SortKey::Size));
        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
        // Each name must still point at its own container slot.
        assert_eq!(n.source_index(0), Some(11), "b came from slot 11");
        assert_eq!(n.source_index(1), Some(12), "c came from slot 12");
        assert_eq!(n.source_index(2), Some(10), "a came from slot 10");
    }

    #[test]
    fn a_mismatched_mapping_falls_back_to_the_identity() {
        // Trusting a short mapping would read the wrong image, so it is ignored.
        let mut n = Navigator::new();
        n.set_items(vec![entry("a", 0), entry("b", 0), entry("c", 0)], vec![7]);

        assert_eq!(n.source_index(0), Some(0));
        assert_eq!(n.source_index(1), Some(1));
        assert_eq!(n.source_index(2), Some(2));
    }

    #[test]
    fn empty_list_is_safe() {
        let mut n = Navigator::new();
        assert!(n.is_empty());
        assert!(n.current().is_none());
        assert_eq!(n.next(), Step::Blocked);
        assert_eq!(n.first(), Step::Blocked);
        assert_eq!(n.last(), Step::Blocked);
        assert!(n.at_first() && n.at_last());
    }

    #[test]
    fn boundary_do_nothing_blocks() {
        let mut n = nav(&["a", "b"], SortOrder::default());
        n.last();
        assert_eq!(n.next(), Step::Blocked);
        assert_eq!(n.current().unwrap().name, "b");

        n.first();
        assert_eq!(n.prev(), Step::Blocked);
    }

    #[test]
    fn boundary_wrap_jumps_to_the_other_end() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        n.set_boundary(BoundaryAction::Wrap);
        n.last();
        assert_eq!(n.next(), Step::Moved);
        assert_eq!(n.cursor(), 0);

        n.first();
        assert_eq!(n.prev(), Step::Moved);
        assert_eq!(n.cursor(), 2);
    }

    #[test]
    fn wrap_handles_multi_step_jumps() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        n.set_boundary(BoundaryAction::Wrap);
        n.last();
        // +10 from index 2 on a 3-image list lands on 2 again (12 mod 3 = 0).
        n.step(10);
        assert_eq!(n.cursor(), 0);
    }

    #[test]
    fn next_container_boundary_blocks_instead_of_wrapping() {
        let mut n = nav(&["a", "b"], SortOrder::default());
        n.set_boundary(BoundaryAction::NextContainer);
        n.last();
        assert_eq!(n.next(), Step::Blocked, "the caller decides to switch container");
    }

    #[test]
    fn stepping_by_counts_moves_that_far() {
        let mut n = nav(&["a", "b", "c", "d", "e"], SortOrder::default());
        n.step_by(2);
        assert_eq!(n.cursor(), 2);
        n.step_back_by(1);
        assert_eq!(n.cursor(), 1);
        assert_eq!(n.step_by(10), Step::Blocked);
    }

    #[test]
    fn select_clamps_and_reports_movement() {
        let mut n = nav(&["a", "b"], SortOrder::default());
        assert_eq!(n.select(99), Step::Moved);
        assert_eq!(n.cursor(), 1);
        assert_eq!(n.select(1), Step::Blocked, "already there");
    }

    #[test]
    fn at_first_and_last_track_the_cursor() {
        let mut n = nav(&["a", "b", "c"], SortOrder::default());
        assert!(n.at_first() && !n.at_last());
        n.next();
        assert!(!n.at_first() && !n.at_last());
        n.next();
        assert!(!n.at_first() && n.at_last());
    }

    #[test]
    fn missing_timestamps_sort_last() {
        let mut n =
            Navigator::with_order(SortOrder::new(SortKey::DateModified), BoundaryAction::DoNothing);
        let base = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        n.set_entries(vec![
            Entry {
                modified: None,
                ..Entry::new("undated")
            },
            Entry {
                modified: Some(base),
                ..Entry::new("old")
            },
            Entry {
                modified: Some(base + std::time::Duration::from_secs(60)),
                ..Entry::new("new")
            },
        ]);
        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["old", "new", "undated"]
        );
    }

    #[test]
    fn archive_member_names_sort_by_their_file_name() {
        let n = nav(
            &["sub/p10.png", "p2.png", "sub/p1.png"],
            SortOrder::default(),
        );
        assert_eq!(
            n.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["sub/p1.png", "p2.png", "sub/p10.png"]
        );
    }

    #[test]
    fn toggle_switches_key_and_direction() {
        let mut order = SortOrder::default();
        order.toggle(SortKey::Name);
        assert_eq!(order.key, SortKey::Name);
        assert!(order.descending, "same key toggles direction");

        order.toggle(SortKey::Size);
        assert_eq!(order.key, SortKey::Size);
        assert!(!order.descending, "new key resets direction");
    }

    #[test]
    fn permutation_is_applied_correctly_for_long_lists() {
        // Guards the in-place permutation against cycle bugs.
        let names: Vec<String> = (0..64).map(|i| format!("f{i}")).collect();
        let n = nav(
            &names.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            SortOrder::default(),
        );
        let got: Vec<String> = n.entries().iter().map(|e| e.name.clone()).collect();
        let mut want = names.clone();
        want.sort_by(|a, b| crate::natsort::natural_cmp(a, b));
        assert_eq!(got, want);
    }
}
