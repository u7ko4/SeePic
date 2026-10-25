//! Human-friendly ("natural") ordering for file names.
//!
//! Plain lexicographic ordering gets comic archives wrong: it puts `page_10`
//! before `page_2`. Splitting names into digit and text runs and comparing
//! digit runs by value fixes that.

use std::cmp::Ordering;

/// One run of characters inside a name: either all digits or all non-digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chunk<'a> {
    Digits(&'a str),
    Text(&'a str),
}

/// Split `s` into alternating digit and text runs.
///
/// Runs are never empty, and adjacent runs are always of different kinds, so
/// the chunk list round-trips back to the original string.
pub fn chunks(s: &str) -> Vec<Chunk<'_>> {
    ChunkIter::new(s).collect()
}

/// The same split as [`chunks`], but lazily and without allocating.
///
/// Sorting a long list compares the same name many times, so the comparison
/// path walks two of these instead of building a `Vec` per comparison.
pub struct ChunkIter<'a> {
    rest: &'a str,
}

impl<'a> ChunkIter<'a> {
    pub fn new(s: &'a str) -> Self {
        Self { rest: s }
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Chunk<'a>;

    fn next(&mut self) -> Option<Chunk<'a>> {
        if self.rest.is_empty() {
            return None;
        }
        let is_digit = self.rest.starts_with(|c: char| c.is_ascii_digit());
        let end = self
            .rest
            .find(|c: char| c.is_ascii_digit() != is_digit)
            .unwrap_or(self.rest.len());
        let (head, tail) = self.rest.split_at(end);
        self.rest = tail;
        Some(if is_digit {
            Chunk::Digits(head)
        } else {
            Chunk::Text(head)
        })
    }
}

/// Compare two digit runs by numeric value, without parsing into an integer.
///
/// Comparing the significant digits avoids overflow on absurdly long runs.
/// Ties on value are broken by fewer leading zeros first, so `2` sorts before
/// `02`.
fn cmp_digits(a: &str, b: &str) -> Ordering {
    let a_sig = a.trim_start_matches('0');
    let b_sig = b.trim_start_matches('0');

    a_sig
        .len()
        .cmp(&b_sig.len())
        .then_with(|| a_sig.cmp(b_sig))
        .then_with(|| {
            let a_zeros = a.len() - a_sig.len();
            let b_zeros = b.len() - b_sig.len();
            a_zeros.cmp(&b_zeros)
        })
}

/// Compare two strings case-insensitively, without allocating.
///
/// Equivalent to `a.to_lowercase().cmp(&b.to_lowercase())`, but the lowercase
/// forms are walked char by char instead of being built as two `String`s.
pub fn cmp_ignore_case(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().flat_map(char::to_lowercase);
    let mut right = b.chars().flat_map(char::to_lowercase);

    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x != y => return x.cmp(&y),
            _ => {}
        }
    }
}

/// Compare two text runs, case-insensitively, with a case-sensitive tiebreak
/// so the ordering is total and stable.
fn cmp_text(a: &str, b: &str) -> Ordering {
    cmp_ignore_case(a, b).then_with(|| a.cmp(b))
}

/// Order one pair of runs.
fn cmp_pair(a: Chunk<'_>, b: Chunk<'_>) -> Ordering {
    match (a, b) {
        (Chunk::Digits(x), Chunk::Digits(y)) => cmp_digits(x, y),
        (Chunk::Text(x), Chunk::Text(y)) => cmp_text(x, y),
        // A digit run sorts before a text run at the same position, which
        // keeps `a1` before `ab` consistently.
        (Chunk::Digits(_), Chunk::Text(_)) => Ordering::Less,
        (Chunk::Text(_), Chunk::Digits(_)) => Ordering::Greater,
    }
}

/// Order two already-tokenized names.
pub fn cmp_chunks(a: &[Chunk<'_>], b: &[Chunk<'_>]) -> Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        let ord = cmp_pair(*x, *y);
        if ord != Ordering::Equal {
            return ord;
        }
    }

    a.len().cmp(&b.len())
}

/// Compare two names the way a human expects.
///
/// Allocation-free: the runs are walked lazily on both sides.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = ChunkIter::new(a);
    let mut right = ChunkIter::new(b);

    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = cmp_pair(x, y);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

/// Compare two names the way the platform's own shell does.
///
/// This is not a locale collation: it calls `StrCmpLogicalW`, which is what
/// makes `2` come before `10` and what decides how punctuation, embedded
/// numbers and non-ASCII digits are treated. Calling the platform function is
/// the only way to agree with the rest of the system exactly rather than
/// approximately, which is what the "逻辑顺序" option promises.
#[cfg(windows)]
pub fn logical_cmp(a: &str, b: &str) -> Ordering {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::StrCmpLogicalW;

    let wide = |text: &str| {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (left, right) = (wide(a), wide(b));
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    let ordering = unsafe { StrCmpLogicalW(left.as_ptr(), right.as_ptr()) };
    ordering.cmp(&0)
}

/// Compare two names the way the platform's file browser does.
///
/// Off Windows there is no `StrCmpLogicalW`, so the natural order stands in;
/// it agrees on the common cases and keeps the option meaningful.
#[cfg(not(windows))]
pub fn logical_cmp(a: &str, b: &str) -> Ordering {
    natural_cmp(a, b)
}

/// A pre-tokenized sort key, so sorting a long list tokenizes each name once
/// instead of once per comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturalKey<'a>(Vec<Chunk<'a>>);

impl<'a> NaturalKey<'a> {
    pub fn new(s: &'a str) -> Self {
        Self(chunks(s))
    }
}

impl Ord for NaturalKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_chunks(&self.0, &other.0)
    }
}

impl PartialOrd for NaturalKey<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_by(|a, b| natural_cmp(a, b));
        names
    }

    #[test]
    fn orders_page_numbers_numerically() {
        assert_eq!(
            sorted(vec!["page_10.jpg", "page_2.jpg", "page_1.jpg"]),
            vec!["page_1.jpg", "page_2.jpg", "page_10.jpg"]
        );
    }

    #[test]
    fn handles_runs_longer_than_any_integer() {
        // Must not panic or overflow, and must still order by magnitude.
        let big = "9".repeat(40);
        let bigger = format!("1{}", "0".repeat(40));
        assert_eq!(natural_cmp(&big, &bigger), Ordering::Less);
        assert_eq!(natural_cmp(&bigger, &big), Ordering::Greater);
    }

    #[test]
    fn fewer_leading_zeros_first() {
        assert_eq!(natural_cmp("2", "02"), Ordering::Less);
        assert_eq!(natural_cmp("002", "02"), Ordering::Greater);
    }

    #[test]
    fn is_case_insensitive_but_deterministic() {
        assert_eq!(natural_cmp("Apple", "apple"), Ordering::Less);
        assert_eq!(natural_cmp("apple", "Apple"), Ordering::Greater);
    }

    #[test]
    fn digits_before_text_at_same_position() {
        assert_eq!(natural_cmp("a1", "ab"), Ordering::Less);
    }

    #[test]
    fn shorter_is_prefix_sorted_first() {
        assert_eq!(natural_cmp("page", "page_2"), Ordering::Less);
    }

    #[test]
    fn chunks_round_trip() {
        for s in ["", "abc", "a1b2", "001", "x_09_y", "页面1"] {
            let rebuilt: String = chunks(s)
                .iter()
                .map(|c| match c {
                    Chunk::Digits(d) => *d,
                    Chunk::Text(t) => *t,
                })
                .collect();
            assert_eq!(rebuilt, s, "round-trip failed for {s:?}");
        }
    }

    #[test]
    fn the_logical_order_agrees_on_numbers() {
        // The case the option exists for: page_2 before page_10.
        assert_eq!(logical_cmp("page_2.png", "page_10.png"), Ordering::Less);
        assert_eq!(logical_cmp("page_10.png", "page_2.png"), Ordering::Greater);
        assert_eq!(logical_cmp("a.png", "a.png"), Ordering::Equal);
    }

    #[test]
    fn the_logical_order_is_a_total_order() {
        let names = ["page_1.png", "page_2.png", "page_10.png", "Page_3.PNG", "1.png"];
        for a in names {
            for b in names {
                let forward = logical_cmp(a, b);
                let backward = logical_cmp(b, a);
                assert_eq!(
                    forward,
                    backward.reverse(),
                    "not antisymmetric for {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn the_logical_order_sorts_a_comic_by_page() {
        let mut names = vec![
            "page_10.png",
            "page_2.png",
            "page_1.png",
            "page_20.png",
            "page_3.png",
        ];
        names.sort_by(|a, b| logical_cmp(a, b));
        assert_eq!(
            names,
            vec![
                "page_1.png",
                "page_2.png",
                "page_3.png",
                "page_10.png",
                "page_20.png"
            ]
        );
    }

    #[test]
    fn key_ordering_matches_direct_comparison() {
        let names = ["p10", "p2", "P1", "p1a", "p1"];
        for a in names {
            for b in names {
                assert_eq!(
                    NaturalKey::new(a).cmp(&NaturalKey::new(b)),
                    natural_cmp(a, b),
                    "key mismatch for {a:?} vs {b:?}"
                );
            }
        }
    }
}
