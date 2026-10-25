//! View modes and the page layouts they imply.
//!
//! Three of the seven browse modes need more than a single scaled image: the
//! two smart-double-page modes pair adjacent pages, and the
//! webtoon mode stacks every page into one continuous scroll. The maths for both
//! lives here so it can be unit tested without a window.

use crate::geom::Size;
use crate::zoom::FitAxis;

/// How the container is laid out on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ViewMode {
    /// One image pixel per physical pixel.
    Actual,
    /// Fit the whole image in the window.
    #[default]
    FitWindow,
    /// Fill the width; vertical scrolling is expected.
    FitWidth,
    /// Fill the height; horizontal scrolling is expected.
    FitHeight,
    /// Two pages side by side, reading left to right.
    TwoPageL2R,
    /// Two pages side by side, reading right to left (manga).
    TwoPageR2L,
    /// Every page stacked vertically, scrolled continuously.
    Webtoon,
}

impl ViewMode {
    pub const ALL: [ViewMode; 7] = [
        ViewMode::Actual,
        ViewMode::FitWindow,
        ViewMode::FitWidth,
        ViewMode::FitHeight,
        ViewMode::TwoPageL2R,
        ViewMode::TwoPageR2L,
        ViewMode::Webtoon,
    ];

    /// The six modes the Image panel lists as browse options.
    pub const PANEL: [ViewMode; 7] = Self::ALL;

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Actual => "原始大小 (100%)",
            ViewMode::FitWindow => "适合窗口",
            ViewMode::FitWidth => "适合宽度",
            ViewMode::FitHeight => "适合高度",
            ViewMode::TwoPageL2R => "智能双页浏览 (左→右)",
            ViewMode::TwoPageR2L => "智能双页浏览 (左←右)",
            ViewMode::Webtoon => "漫画模式",
        }
    }

    /// The next mode in the cycle, for the "switch browse mode" command.
    pub fn next(self) -> ViewMode {
        let index = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn is_two_page(self) -> bool {
        matches!(self, ViewMode::TwoPageL2R | ViewMode::TwoPageR2L)
    }

    pub fn is_webtoon(self) -> bool {
        self == ViewMode::Webtoon
    }

    /// True when the mode shows one image at a time, so the ordinary
    /// zoom/pan canvas applies.
    pub fn is_single_page(self) -> bool {
        !self.is_two_page() && !self.is_webtoon()
    }

    /// The reading direction, for the two-page modes.
    pub fn direction(self) -> ReadingDirection {
        match self {
            ViewMode::TwoPageR2L => ReadingDirection::RightToLeft,
            _ => ReadingDirection::LeftToRight,
        }
    }

    /// The fit axis for the single-page modes.
    pub fn fit_axis(self) -> Option<FitAxis> {
        match self {
            ViewMode::Actual => None,
            ViewMode::FitWindow => Some(FitAxis::Window),
            ViewMode::FitWidth => Some(FitAxis::Width),
            ViewMode::FitHeight => Some(FitAxis::Height),
            ViewMode::TwoPageL2R | ViewMode::TwoPageR2L | ViewMode::Webtoon => None,
        }
    }
}

/// Which way pages read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadingDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// A page is shown on its own when it is wider than it is tall: a double-page
/// spread already, so pairing it would put three pages on screen at once.
pub fn is_wide_spread(size: Size) -> bool {
    size.w > size.h
}

/// For every page, the index of the page that begins its spread.
///
/// Computed once per container rather than per frame, because the greedy pairing
/// depends on every preceding page: a wide page shifts the parity of everything
/// after it.
pub fn spread_starts(sizes: &[Size], separate_cover: bool) -> Vec<usize> {
    let mut starts = vec![0usize; sizes.len()];
    let mut index = 0;

    // The cover page stands alone, and pairing resumes at page 1.
    if separate_cover && !sizes.is_empty() {
        starts[0] = 0;
        index = 1;
    }

    while index < sizes.len() {
        starts[index] = index;
        if is_wide_spread(sizes[index]) {
            index += 1;
            continue;
        }
        // Pair with the next page when there is one and it is not itself wide.
        if index + 1 < sizes.len() && !is_wide_spread(sizes[index + 1]) {
            starts[index + 1] = index;
            index += 2;
        } else {
            index += 1;
        }
    }

    starts
}

/// The pages drawn together, and where each sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
    /// Page indices, in left-to-right screen order.
    pub pages: Vec<usize>,
    /// Left edge of each page within the spread, in image pixels.
    pub offsets: Vec<f32>,
    /// Combined size, in image pixels.
    pub size: Size,
}

impl Spread {
    pub fn single(index: usize, size: Size) -> Self {
        Self {
            pages: vec![index],
            offsets: vec![0.],
            size,
        }
    }

    /// The page the cursor should step to when leaving this spread backwards.
    pub fn first(&self) -> Option<usize> {
        self.pages.first().copied()
    }

    /// The page to step to when leaving this spread forwards.
    pub fn last(&self) -> Option<usize> {
        self.pages.last().copied()
    }
}

/// The spread containing `index`, laid out for the given reading direction.
///
/// `starts` comes from [`spread_starts`]; `sizes` must be the same length.
pub fn spread(
    sizes: &[Size],
    starts: &[usize],
    index: usize,
    direction: ReadingDirection,
    gap: f32,
) -> Spread {
    let Some(size) = sizes.get(index).copied() else {
        return Spread::single(index, Size::default());
    };
    let Some(&start) = starts.get(index) else {
        return Spread::single(index, size);
    };

    // A page belongs to a pair when the next page's spread starts at this one.
    let paired = sizes
        .get(start + 1)
        .map(|_| starts.get(start + 1) == Some(&start))
        .unwrap_or(false);

    let mut pages = if paired {
        vec![start, start + 1]
    } else {
        vec![start]
    };
    if direction == ReadingDirection::RightToLeft {
        pages.reverse();
    }

    let mut offsets = Vec::with_capacity(pages.len());
    let mut x = 0.;
    let mut width = 0.;
    let mut height: f32 = 0.;
    for (position, page) in pages.iter().enumerate() {
        let page_size = sizes.get(*page).copied().unwrap_or_default();
        if position > 0 {
            x += gap;
        }
        offsets.push(x);
        x += page_size.w;
        width = x;
        height = height.max(page_size.h);
    }

    Spread {
        pages,
        offsets,
        size: Size::new(width, height),
    }
}

/// The drawn size of every page in webtoon mode, at the given content width.
///
/// Each page is scaled to `width` and separated from the next by `gap`, so the
/// result doubles as the virtual list's item sizes.
pub fn webtoon_sizes(sizes: &[Size], width: f32, gap: f32) -> Vec<Size> {
    sizes
        .iter()
        .map(|size| {
            let height = if size.w > 0. {
                width * size.h / size.w
            } else {
                0.
            };
            Size::new(width, height.max(1.) + gap)
        })
        .collect()
}

/// The content width webtoon mode uses for a given viewport.
///
/// `percent` is the user's width setting as a percentage of the viewport;
/// `explicit` overrides it with a pixel width when the user typed one.
pub fn webtoon_width(viewport_width: f32, percent: u32, explicit: Option<f32>) -> f32 {
    let width = explicit.unwrap_or(viewport_width * (percent.max(1) as f32) / 100.);
    width.clamp(64., 20_000.)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portrait() -> Size {
        Size::new(1000., 1500.)
    }

    fn landscape() -> Size {
        Size::new(3000., 1500.)
    }

    #[test]
    fn the_cycle_visits_every_mode_once() {
        let mut seen = vec![ViewMode::Actual];
        let mut mode = ViewMode::Actual;
        for _ in 1..ViewMode::ALL.len() {
            mode = mode.next();
            assert!(!seen.contains(&mode), "{mode:?} visited twice");
            seen.push(mode);
        }
        assert_eq!(mode.next(), ViewMode::Actual, "the cycle must close");
    }

    #[test]
    fn single_page_modes_report_a_fit_axis() {
        assert_eq!(ViewMode::Actual.fit_axis(), None);
        assert_eq!(ViewMode::FitWindow.fit_axis(), Some(FitAxis::Window));
        assert_eq!(ViewMode::FitWidth.fit_axis(), Some(FitAxis::Width));
        assert_eq!(ViewMode::FitHeight.fit_axis(), Some(FitAxis::Height));
        assert!(ViewMode::TwoPageL2R.fit_axis().is_none());
        assert!(ViewMode::Webtoon.fit_axis().is_none());
    }

    #[test]
    fn only_the_two_page_modes_are_two_page() {
        assert!(ViewMode::TwoPageL2R.is_two_page());
        assert!(ViewMode::TwoPageR2L.is_two_page());
        assert!(!ViewMode::Webtoon.is_two_page());
        assert!(ViewMode::Webtoon.is_webtoon());
        assert!(ViewMode::FitWindow.is_single_page());
        assert!(!ViewMode::Webtoon.is_single_page());
    }

    #[test]
    fn adjacent_portrait_pages_pair_up() {
        let sizes = vec![portrait(); 4];
        let starts = spread_starts(&sizes, false);
        assert_eq!(starts, vec![0, 0, 2, 2]);
    }

    #[test]
    fn a_wide_page_stands_alone_and_resets_the_parity() {
        let sizes = vec![portrait(), landscape(), portrait(), portrait()];
        let starts = spread_starts(&sizes, false);
        // Page 0 pairs with nothing (page 1 is wide), page 1 is alone, pages
        // 2 and 3 pair together.
        assert_eq!(starts, vec![0, 1, 2, 2]);
    }

    #[test]
    fn a_separate_cover_stands_alone() {
        let sizes = vec![portrait(); 4];
        let starts = spread_starts(&sizes, true);
        assert_eq!(starts, vec![0, 1, 1, 3]);
    }

    #[test]
    fn an_empty_container_has_no_spreads() {
        assert!(spread_starts(&[], false).is_empty());
    }

    #[test]
    fn the_spread_combines_both_pages_with_a_gap() {
        let sizes = vec![portrait(), portrait()];
        let starts = spread_starts(&sizes, false);
        let laid_out = spread(&sizes, &starts, 0, ReadingDirection::LeftToRight, 20.);

        assert_eq!(laid_out.pages, vec![0, 1]);
        assert_eq!(laid_out.offsets, vec![0., 1020.]);
        assert_eq!(laid_out.size, Size::new(2020., 1500.));
    }

    #[test]
    fn right_to_left_reverses_the_screen_order() {
        let sizes = vec![portrait(), portrait()];
        let starts = spread_starts(&sizes, false);
        let laid_out = spread(&sizes, &starts, 0, ReadingDirection::RightToLeft, 0.);

        assert_eq!(laid_out.pages, vec![1, 0], "page 1 sits on the left");
        assert_eq!(laid_out.size.w, 2000.);
    }

    #[test]
    fn either_page_of_a_pair_resolves_to_the_same_spread() {
        let sizes = vec![portrait(); 4];
        let starts = spread_starts(&sizes, false);
        let from_first = spread(&sizes, &starts, 2, ReadingDirection::LeftToRight, 0.);
        let from_second = spread(&sizes, &starts, 3, ReadingDirection::LeftToRight, 0.);
        assert_eq!(from_first.pages, from_second.pages);
    }

    #[test]
    fn a_lone_page_is_its_own_spread() {
        let sizes = vec![portrait(), landscape()];
        let starts = spread_starts(&sizes, false);
        let laid_out = spread(&sizes, &starts, 1, ReadingDirection::LeftToRight, 0.);
        assert_eq!(laid_out.pages, vec![1]);
        assert_eq!(laid_out.size, landscape());
    }

    #[test]
    fn a_spread_out_of_range_is_empty() {
        let laid_out = spread(&[], &[], 5, ReadingDirection::LeftToRight, 0.);
        assert!(laid_out.size.is_empty());
    }

    #[test]
    fn webtoon_scales_each_page_to_the_content_width() {
        let sizes = vec![portrait(), landscape()];
        let laid_out = webtoon_sizes(&sizes, 600., 10.);
        assert_eq!(laid_out[0], Size::new(600., 900. + 10.));
        assert_eq!(laid_out[1], Size::new(600., 300. + 10.));
    }

    #[test]
    fn webtoon_width_follows_the_percentage_until_overridden() {
        assert_eq!(webtoon_width(1000., 100, None), 1000.);
        assert_eq!(webtoon_width(1000., 50, None), 500.);
        assert_eq!(webtoon_width(1000., 50, Some(240.)), 240.);
        // Absurd values are clamped rather than producing a zero-width image.
        assert_eq!(webtoon_width(1000., 0, None), 64.);
    }
}
