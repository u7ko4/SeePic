//! The thumbnail grid, opened over the whole window.
//!
//! The filmstrip only ever shows one row, which is the wrong shape for a
//! hundred-page comic: finding a page means scrolling a long way with very
//! little on screen. The grid shows many rows at once, so a page can be found
//! by looking rather than by scrolling.
//!
//! Thumbnails are the same ones the strip uses, decoded in the background and
//! cached per display position, so switching between the two costs nothing.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::ViewerApp;

/// Cell size, and how many fit across a row.
const CELL_WIDTH: f32 = 132.;
/// Horizontal gap between cells. Also part of the column count: fitting only
/// `CELL_WIDTH` per column ignored the gaps, so the last column was cut off by
/// `(columns - 1) * 8` px — always visible in a maximized window.
const CELL_GAP: f32 = 8.;
/// Left and right padding of the grid body.
const BODY_PADDING: f32 = 24.;
/// Also used by the viewer to translate wheel pixels into rows.
pub const CELL_HEIGHT: f32 = 156.;
const THUMB_WIDTH: f32 = 112.;
const THUMB_HEIGHT: f32 = 116.;

/// How many cells fit across the window at its current size.
pub fn columns(window: &Window) -> usize {
    columns_for_width(f32::from(window.bounds().size.width))
}

/// Column count for a window width, gaps included.
fn columns_for_width(width: f32) -> usize {
    let available = width - BODY_PADDING * 2.;
    (((available + CELL_GAP) / (CELL_WIDTH + CELL_GAP)).floor() as usize).max(1)
}

/// Whether a virtual-list render range is the real viewport.
///
/// The list calls the render closure twice per frame: once for the rows on
/// screen, and once with a single item so it can measure a cell's width. Only
/// the first may request thumbnails or define the protected set. The expected
/// top row follows from the scroll offset, which separates the two: at an
/// offset of -8578 px the real range starts at row 54 and the measurement call
/// is `0..1`.
fn is_visible_pass(range: &std::ops::Range<usize>, offset: f32) -> bool {
    let expected = ((-offset) / CELL_HEIGHT).floor().max(0.) as usize;
    range.start.abs_diff(expected) <= 1
}

pub fn render(
    view: &ViewerApp,
    window: &Window,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let columns = columns(window);
    let total = view.entry_count();
    // The grid draws the filtered list, so the rows follow the matches rather
    // than the whole container.
    let count = view.contact_matches.len();
    let rows = count.div_ceil(columns);
    let available =
        f32::from(window.bounds().size.width) - BODY_PADDING * 2.;
    let row_sizes = Rc::new(vec![size(px(available), px(CELL_HEIGHT)); rows]);
    let entity = cx.entity();
    let selected = view.navigator_selection;

    div()
        .id("contact-sheet-root")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(0x000000CC))
        // Clicks and hover are blocked for the canvas behind, but the wheel
        // is allowed through: the canvas handler scrolls this grid.
        .block_mouse_except_scroll()
        .child(
            v_flex()
                .w_full()
                .h_full()
                .py_4()
                .gap_2()
                .child(header(view, count, total, cx))
                .child(
                    div()
                        .id("contact-sheet-body")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .px(px(BODY_PADDING))
                        // The virtual list scrolls itself (its own overflow
                        // area); a scroll container here would take the wheel
                        // and have nothing to move, since the list fills it.
                        .child(if count == 0 {
                            div()
                                .w_full()
                                .py_10()
                                .flex()
                                .justify_center()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("没有匹配的文件")
                                .into_any_element()
                        } else {
                            v_virtual_list(
                                entity,
                                "contact-sheet",
                                row_sizes,
                                move |view, range, window, cx| {
                                    let scale = window.scale_factor();
                                    let offset = f32::from(view.contact_scroll.offset().y);
                                    // `v_virtual_list` calls this closure once
                                    // per frame with `0..1` to measure a cell,
                                    // in addition to the real visible range.
                                    // Treating the measurement as a viewport
                                    // requested row 0 no matter where the grid
                                    // was scrolled, and overwrote the protected
                                    // set with row 0, so the two ranges kept
                                    // evicting each other's thumbnails.
                                    let real_pass = is_visible_pass(&range, offset);
                                    // While the wheel is moving, the grid only
                                    // draws what it already has. Requests and
                                    // texture uploads resume on the frame it
                                    // stops, so the animation is not competing
                                    // with decodes and atlas churn.
                                    let ask = real_pass && !view.contact_scrolling();
                                    let visible = range.clone();
                                    // On-screen tiles (and the prefetch rows)
                                    // are exempt from LRU eviction; see
                                    // `thumbnail_protected`.
                                    let mut protected = Vec::with_capacity(count.min(256));
                                    let mut cells = Vec::with_capacity(range.len());
                                    for row in range {
                                        let first = row * columns;
                                        let last = (first + columns).min(count);
                                        let mut line = h_flex()
                                            .w_full()
                                            .gap(px(CELL_GAP))
                                            .items_start()
                                            .py_1();
                                        for slot in first..last {
                                            let position = view.contact_matches[slot];
                                            if real_pass {
                                                protected.push(position);
                                            }
                                            // Only the visible rows ask for a
                                            // thumbnail, which is what keeps a
                                            // long container cheap.
                                            if ask {
                                                view.ensure_thumbnail(position, scale, cx);
                                            }
                                            line = line.child(
                                                cell(
                                                    view,
                                                    position,
                                                    position == selected,
                                                    cx,
                                                )
                                                .into_any_element(),
                                            );
                                        }
                                        cells.push(line.into_any_element());
                                    }
                                    // One extra row on each side, after the
                                    // visible cells have claimed the decode
                                    // slots: a wheel step now moves
                                    // fractionally, so the next row is often
                                    // already partly in view and should not
                                    // flash a placeholder as it arrives.
                                    if real_pass {
                                        let rows = count.div_ceil(columns.max(1));
                                        let start = visible.start.saturating_sub(1);
                                        let end = (visible.end + 1).min(rows);
                                        for row in start..end {
                                            if visible.contains(&row) {
                                                continue;
                                            }
                                            let first = row * columns;
                                            let last = (first + columns).min(count);
                                            for slot in first..last {
                                                let position = view.contact_matches[slot];
                                                protected.push(position);
                                                if ask {
                                                    view.ensure_thumbnail(position, scale, cx);
                                                }
                                            }
                                        }
                                    }
                                    if real_pass {
                                        view.thumbnail_protected.clear();
                                        view.thumbnail_protected.extend(protected);
                                    }
                                    cells
                                },
                            )
                            // The list needs the scroll position to decide which
                            // rows exist; without it the grid draws the first
                            // screenful and the body has nothing to scroll.
                            .track_scroll(&view.contact_scroll)
                            .into_any_element()
                        })
                        .when(count > 0, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right(px(4.))
                                    .bottom_0()
                                    .w(px(10.))
                                    .child(
                                        Scrollbar::vertical(&view.contact_scroll)
                                            .mode(ScrollbarMode::Scrolling),
                                    ),
                            )
                        }),
                ),
        )
        .child(index_bar(view, columns, cx))
}

/// The index letter slot for a file name: A–Z, or `#` for anything else.
pub(crate) fn letter_slot(name: &str) -> usize {
    match name.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => (c.to_ascii_uppercase() as u8 - b'A') as usize,
        _ => 26,
    }
}

/// The A–Z index bar down the left edge.
///
/// It is the grid's scroll control as well as a section jump list: only the
/// letters present in the current list are selectable, and the selected item's
/// section is highlighted. Dragging along it jumps as the pointer crosses each
/// letter, the way a contacts index does. The right edge is left for the
/// auto-hiding scrollbar.
fn index_bar(view: &ViewerApp, columns: usize, cx: &mut Context<ViewerApp>) -> AnyElement {
    let theme = cx.theme().clone();
    // Built when the match list changes, not on every frame: scanning tens of
    // thousands of names per frame is what made the grid stutter while the
    // wheel was moving.
    let first = view.contact_index_first;
    let current = letter_slot(&view.entry_name(view.navigator_selection));
    let hover_bg = theme.button_hover;

    let bar = v_flex()
        .px_0p5()
        .py_1()
        .gap_0()
        .rounded(px(8.))
        .bg(theme.secondary.opacity(0.88))
        .block_mouse_except_scroll()
        .children((0..27).map(|slot| {
            let first_match = first[slot];
            let label = if slot == 26 {
                "#".to_string()
            } else {
                ((b'A' + slot as u8) as char).to_string()
            };
            div()
                .id(("index-letter", slot))
                .w(px(14.))
                .h(px(13.))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .when(first_match.is_some(), |this| this.cursor_pointer())
                .text_color(if slot == current {
                    theme.primary
                } else if first_match.is_some() {
                    theme.muted_foreground
                } else {
                    theme.muted_foreground.opacity(0.3)
                })
                .when(first_match.is_some(), |this| {
                    this.hover(move |this| this.bg(hover_bg))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    if let Some(index) = first_match {
                        this.jump_contact_index(index, columns, cx);
                    }
                }))
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        if let Some(index) = first_match {
                            this.jump_contact_index(index, columns, cx);
                        }
                    }
                }))
                .child(label)
        }));

    // A full-height strip with the bar centred in it, so the index sits in the
    // middle of the window rather than under the header. It lives in the left
    // padding, clear of the first column.
    div()
        .absolute()
        .left(px(4.))
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .block_mouse_except_scroll()
        .child(bar)
        .into_any_element()
}

fn header(
    view: &ViewerApp,
    count: usize,
    total: usize,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let filtered = !view.contact_filter.is_empty();

    v_flex()
        .w_full()
        .px(px(BODY_PADDING))
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("缩略图"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if filtered {
                            format!("{count} / {total} 张")
                        } else {
                            format!("{total} 张")
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("方向键选择 · Enter 打开 · Esc 清除/关闭"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("contact-sheet-close")
                        .p_1()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.button_hover))
                        .child(
                            Icon::new(IconName::Close)
                                .size(px(16.))
                                .text_color(theme.foreground),
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.contact_sheet = false;
                            this.contact_filter.clear();
                            cx.notify();
                        })),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .bg(theme.input)
                .child(
                    Icon::new(IconName::Search)
                        .size(px(14.))
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .when(view.contact_filter.is_empty(), |this| {
                            this.text_color(theme.muted_foreground)
                        })
                        .child(if view.contact_filter.is_empty() {
                            "输入文件名以筛选…".to_string()
                        } else {
                            view.contact_filter.clone()
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{count} 项")),
                ),
        )
}

fn cell(
    view: &ViewerApp,
    index: usize,
    selected: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let name = view.entry_name(index);
    let thumb = view.thumbnails.get(&index).cloned();
    let has_thumb = thumb.is_some();

    div()
        .id(("sheet-cell", index))
        .w(px(CELL_WIDTH))
        .h(px(CELL_HEIGHT))
        .p_1()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .rounded(px(6.))
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.list_active))
        .hover(|this| this.bg(theme.list_hover))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.contact_sheet = false;
            this.select_position(index, cx);
        }))
        .child(
            div()
                .w(px(THUMB_WIDTH))
                .h(px(THUMB_HEIGHT))
                .flex_shrink_0()
                .rounded(px(4.))
                .overflow_hidden()
                .bg(theme.muted)
                .border_1()
                .border_color(if selected { theme.primary } else { theme.border })
                .flex()
                .items_center()
                .justify_center()
                .when_some(thumb, |this, render| {
                    this.child(
                        img(ImageSource::Render(render))
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    )
                })
                .when(!has_thumb, |this| {
                    this.child(
                        Icon::new(IconName::Image)
                            .size(px(18.))
                            .text_color(theme.muted_foreground),
                    )
                }),
        )
        .child(
            div()
                .w_full()
                .text_xs()
                .text_center()
                .truncate()
                .text_color(if selected {
                    theme.primary
                } else {
                    theme.muted_foreground
                })
                .child(name),
        )
}

#[cfg(test)]
mod tests {
    use super::{is_visible_pass, columns_for_width, BODY_PADDING, CELL_GAP, CELL_WIDTH};

    #[test]
    fn the_measurement_pass_is_not_a_viewport() {
        // The grid is ~55 rows down; the list still measures row 0 each frame.
        assert!(is_visible_pass(&(54..62), -8578.4));
        assert!(!is_visible_pass(&(0..1), -8578.4));
    }

    #[test]
    fn the_top_of_the_list_accepts_the_first_rows() {
        assert!(is_visible_pass(&(0..1), 0.));
        assert!(is_visible_pass(&(0..8), 0.));
    }

    #[test]
    fn a_row_straddling_the_offset_still_counts() {
        // The viewport can start one row above the row containing the offset.
        assert!(is_visible_pass(&(9..17), -1560.5));
        assert!(is_visible_pass(&(10..18), -1560.5));
        assert!(!is_visible_pass(&(0..8), -1560.5));
    }

    #[test]
    fn the_column_count_leaves_room_for_every_gap() {
        // Maximized on the 2560 x 1440 test display: 14 columns, not the 15
        // that overflowed by the 13 gaps and clipped the last one.
        assert_eq!(columns_for_width(2062.), 14);
        for width in [800., 1024., 1294., 1600., 2062., 3000.] {
            let columns = columns_for_width(width);
            let needed =
                columns as f32 * CELL_WIDTH + (columns.saturating_sub(1)) as f32 * CELL_GAP;
            let available = width - BODY_PADDING * 2.;
            assert!(
                needed <= available + 0.01,
                "{width}: {columns} columns need {needed}, only {available} available"
            );
        }
    }
}
