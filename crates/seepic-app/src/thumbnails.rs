//! The thumbnail filmstrip along the bottom.
//!
//! Scrolls horizontally and is virtualized, so a thousand-page comic only builds
//! the cells on screen. Thumbnails are decoded on a background thread and cached
//! per display position; until one is ready the cell shows a placeholder rather
//! than a spinner, which keeps the strip from flickering while scrolling.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::ViewerApp;

/// Height of the whole strip, including the name line.
pub const STRIP_HEIGHT: f32 = 128.;
/// Width of one cell.
pub const CELL_WIDTH: f32 = 92.;
const THUMB_WIDTH: f32 = 72.;
const THUMB_HEIGHT: f32 = 84.;

pub fn render(
    view: &ViewerApp,
    entity: Entity<ViewerApp>,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = view.entry_count();
    let cell_sizes = Rc::new(vec![size(px(CELL_WIDTH), px(STRIP_HEIGHT)); count]);

    div()
        .w_full()
        .h(px(STRIP_HEIGHT))
        .flex_shrink_0()
        .bg(theme.sidebar)
        .border_t_1()
        .border_color(theme.sidebar_border)
        .child(
            h_virtual_list(
                entity,
                "thumbnail-strip",
                cell_sizes,
                move |view, range, window, cx| {
                    let scale = window.scale_factor();
                    let mut cells = Vec::with_capacity(range.len());
                    for index in range {
                        // The render pass is the only place that knows which
                        // cells are visible, so requests are made from here.
                        view.ensure_thumbnail(index, scale, cx);
                        cells.push(cell(view, index, cx).into_any_element());
                    }
                    cells
                },
            )
            .with_scroll_handle(&view.strip_scroll)
            .h_full(),
        )
        // GPUI locks the list to its own axis, so a vertical wheel would be
        // ignored there; translate it here instead.
        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
            this.scroll_strip_by_wheel(event, cx);
        }))
}

fn cell(view: &ViewerApp, index: usize, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let selected = view.cursor_position() == index;
    let name = view.entry_name(index);
    let thumb = view.thumbnails.get(&index).cloned();
    let has_thumb = thumb.is_some();

    div()
        .id(("thumb", index))
        .w(px(CELL_WIDTH))
        .h(px(STRIP_HEIGHT))
        .px_1()
        .py_1p5()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.list_active))
        .hover(|this| this.bg(theme.list_hover))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
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
                            .size(px(16.))
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
