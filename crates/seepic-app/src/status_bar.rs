//! The status bar along the bottom, plus the corner toast.
//!
//! It replaces the floating control bar. Playback, position, zoom and the
//! transient message all live on one line, which is the row the window already
//! had to spare once the title bar and the menu strip became one.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;

use crate::app::ViewerApp;

/// Width of the progress track.
const TRACK_WIDTH: f32 = 240.;

/// The narrowest the track is allowed to become.
///
/// Below its full width the track gives way so the buttons on the right stay on
/// screen; a 80px track still reads as a position and still seeks.
const TRACK_MIN_WIDTH: f32 = 80.;

/// Below this window width the image's own dimensions and file size are
/// dropped. They are the longest text on the row, and the same values are in
/// the info panel.
const FACTS_MIN_WIDTH: f32 = 960.;

/// Below this window width the zoom readout is dropped, with the separators
/// that frame it. The wheel, the shortcuts and the two buttons still zoom.
const ZOOM_MIN_WIDTH: f32 = 700.;

/// Below this window width the progress track and its counter are dropped.
///
/// The track is the widest item on the row, and the counter repeats what the
/// title bar already says ("3 / 12"); the buttons on the right are the parts a
/// narrow window cannot get at any other way.
const PROGRESS_MIN_WIDTH: f32 = 520.;

/// The bar itself.
pub fn render(view: &ViewerApp, window: &Window, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let (position, total) = view.position();
    // Floating over the image, the bar is see-through so the picture is still
    // readable behind it; taking its own row, it stays solid.
    let floating = view.settings.chrome_mode.reveals_on_hover();
    // The row is laid out left to right with a fixed part on each end, so a
    // narrow window would clip the buttons on the right rather than shrink
    // anything. Parts are given up in order of what costs the reader least.
    let width = f32::from(window.bounds().size.width);
    let facts = if width >= FACTS_MIN_WIDTH {
        view.image_facts()
    } else {
        None
    };
    let show_zoom = width >= ZOOM_MIN_WIDTH;
    let show_progress = width >= PROGRESS_MIN_WIDTH;

    h_flex()
        .id("status-bar")
        .w_full()
        .h(px(crate::app::STATUS_BAR_HEIGHT))
        .px_2()
        .gap_1()
        .items_center()
        .text_xs()
        // Floating over the image, a press on the bar belongs to the bar and
        // must not reach the canvas behind it. `occlude` would do that too,
        // but it also stops the hit test at this row, which leaves the pointer
        // without hover: no mouse-move and no mouse-up would be dispatched
        // while it is over the bar, so the progress track could be pressed but
        // never dragged. Stopping the press leaves the hover chain intact.
        .when(floating, |this| {
            this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        })
        // A release on the bar belongs to the bar, not to the image behind it:
        // two quick presses on a button must not read as a double click on the
        // image and maximise the window. Any scrub the bar started ends here.
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                this.scrubbing = false;
                cx.stop_propagation();
            }),
        )
        .bg(if floating {
            theme.title_bar.opacity(0.82)
        } else {
            theme.title_bar
        })
        .border_t_1()
        .border_color(theme.title_bar_border)
        // Left: playback, which is what the pointer reaches for most often.
        .child(icon_button(
            "status-prev",
            IconName::ChevronLeft,
            Command::PrevImage,
            view,
            cx,
        ))
        .child(icon_button(
            "status-play",
            if view.slideshow.is_running() {
                IconName::Pause
            } else {
                IconName::Play
            },
            if view.slideshow.is_running() {
                Command::SlideshowStop
            } else {
                Command::SlideshowToggle
            },
            view,
            cx,
        ))
        .child(icon_button(
            "status-next",
            IconName::ChevronRight,
            Command::NextImage,
            view,
            cx,
        ))
        .when(show_progress, |this| {
            this.child(separator(cx))
                .child(progress_track(view, position, total, cx))
                .child(
                    div()
                        .min_w(px(64.))
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(format!("{position} / {total}")),
                )
        })
        .child(div().flex_1())
        // Right: the image's own facts, then what it is being shown at.
        .when_some(facts, |this, facts| {
            this.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(facts),
            )
            .child(separator(cx))
        })
        .when(show_zoom, |this| {
            this.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(view.zoom_label(window)),
            )
            .child(separator(cx))
        })
        .child(icon_button(
            "status-zoom-out",
            IconName::ZoomOut,
            Command::ZoomOut,
            view,
            cx,
        ))
        .child(icon_button(
            "status-zoom-in",
            IconName::ZoomIn,
            Command::ZoomIn,
            view,
            cx,
        ))
        .child(icon_button(
            "status-fit",
            IconName::Expand,
            Command::ViewModeFitWindow,
            view,
            cx,
        ))
        .child(separator(cx))
        .child(icon_button(
            "status-thumbnails",
            IconName::GalleryVerticalEnd,
            Command::ToggleThumbnails,
            view,
            cx,
        ))
        .child(icon_button(
            "status-info",
            IconName::Info,
            Command::ToggleInfo,
            view,
            cx,
        ))
        .child(icon_button(
            "status-slideshow-panel",
            IconName::Timer,
            Command::ToggleSlideshowPanel,
            view,
            cx,
        ))
        .child(icon_button(
            "status-settings",
            IconName::Settings,
            Command::Settings,
            view,
            cx,
        ))
}

fn separator(cx: &Context<ViewerApp>) -> impl IntoElement {
    div()
        .w(px(1.))
        .h(px(14.))
        .mx_1()
        .bg(cx.theme().border)
}

fn icon_button(
    id: &'static str,
    icon: IconName,
    command: Command,
    view: &ViewerApp,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let active = match command {
        Command::ToggleInfo => view.show_info,
        Command::ToggleThumbnails => view.thumbnails_visible(),
        Command::SlideshowToggle | Command::SlideshowStop => view.slideshow.is_running(),
        Command::SearchFiles => view.search.is_some(),
        _ => false,
    };

    div()
        .id(id)
        .p_1()
        .rounded(px(5.))
        .cursor_pointer()
        .when(active, |this| this.bg(theme.button_active))
        .hover(|this| this.bg(theme.button_hover))
        .child(
            Icon::new(icon)
                .size(px(15.))
                .text_color(if active {
                    theme.primary
                } else {
                    theme.muted_foreground
                }),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.execute(command, window, cx);
        }))
}

/// The position slider, which doubles as the slideshow countdown.
fn progress_track(
    view: &ViewerApp,
    position: usize,
    total: usize,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let countdown = view.slideshow_progress();
    let progress = if total <= 1 {
        1.
    } else {
        (position.saturating_sub(1)) as f32 / (total - 1) as f32
    };
    let entity = cx.entity();

    // The wrapper reports the track's own rectangle: a press maps to a
    // position from the bar's left edge, not the window's.
    //
    // It takes the slack of the row up to its full width and gives width back
    // when the window narrows, so the buttons after it stay on screen. The
    // fill is a fraction of whatever width it ends up with.
    div()
        .w(px(TRACK_WIDTH))
        .flex_grow(1.)
        .flex_shrink_1()
        .min_w(px(TRACK_MIN_WIDTH))
        .max_w(px(TRACK_WIDTH))
        .h(px(6.))
        .on_children_prepainted(move |bounds: Vec<Bounds<Pixels>>, _window, cx: &mut App| {
            let Some(bounds) = bounds.first() else {
                return;
            };
            let geometry = (f32::from(bounds.origin.x), f32::from(bounds.size.width));
            entity.update(cx, |this, cx| {
                if this.seek_track != Some(geometry) {
                    this.seek_track = Some(geometry);
                    cx.notify();
                }
            });
        })
        .child(
            div()
                .id("status-track")
                .w_full()
                .h_full()
                .rounded_full()
                .bg(theme.border)
                .cursor_pointer()
                .relative()
                .child(
                    div()
                        .h_full()
                        .w(relative(progress.clamp(0., 1.)))
                        .rounded_full()
                        .bg(theme.primary),
                )
                .when_some(countdown, |this, progress| {
                    // The countdown rides on top of the position, so one bar
                    // answers both "where am I" and "how long until the next".
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .w(relative(progress.clamp(0., 1.)))
                            .rounded_full()
                            .bg(theme.primary.opacity(0.45)),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                        // A press starts a scrub: movement continues it and the
                        // bar's own mouse-up ends it.
                        this.scrubbing = true;
                        this.seek_at(f32::from(event.position.x), cx);
                    }),
                ),
        )
}

/// The transient message in the top-right corner.
pub fn toast(
    text: &SharedString,
    is_error: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let (background, foreground) = if is_error {
        (theme.danger, theme.danger_foreground)
    } else {
        (theme.secondary, theme.secondary_foreground)
    };

    div()
        .absolute()
        .top(px(52.))
        .right(px(14.))
        .max_w(px(420.))
        .px_3()
        .py_2()
        .rounded(px(8.))
        .bg(background)
        .text_color(foreground)
        .border_1()
        .border_color(theme.border)
        .shadow_md()
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Icon::new(if is_error {
                        IconName::CircleAlert
                    } else {
                        IconName::Info
                    })
                    .size(px(15.)),
                )
                .child(div().child(text.clone())),
        )
}

