//! The settings window.
//!
//! A full-window overlay with a category rail on the left. Every control writes
//! straight into `Settings`, which the viewer then mirrors and persists.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;
use seepic_core::settings::{
    ChromeMode, Group, MEMORY_BUDGET_CHOICES, MouseSlot, Settings, ThemeMode, ThumbnailView,
    WheelAction, ZoomPreset,
};
use seepic_core::Transparency;

use seepic_core::BoundaryAction;

use crate::app::{ChoicePopup, ViewerApp};
use crate::strings as s;

const NAV_WIDTH: f32 = 168.;
/// Width of the whole settings dialog.
const DIALOG_WIDTH: f32 = 820.;
/// Width of the page area to the right of the rail.
///
/// Fixed rather than `flex_1`: a scroll container laid out from its content
/// would hand the page the width of its longest line, and a long value would
/// run off the dialog instead of wrapping inside it.
const BODY_WIDTH: f32 = DIALOG_WIDTH - NAV_WIDTH;

/// Height of one entry in a settings list.
const CHOICE_ROW_HEIGHT: f32 = 30.;
/// Width of a settings list.
const CHOICE_WIDTH: f32 = 240.;

/// The wheel speeds the mouse page offers, slow to fast.
const WHEEL_SPEEDS: [u8; 3] = [1, 3, 5];

/// Longest sides a reduced decode may keep, in pixels.
///
/// The lower end trades sharpness at 100% for memory; the upper end is where a
/// single frame stops fitting a 16 MB slot.
const DECODE_EDGES: [u32; 7] = [1024, 1536, 1920, 2048, 2560, 3072, 4096];
/// How many neighbours either side to decode ahead.
const PREFETCH_WINDOWS: [u32; 5] = [0, 1, 2, 3, 4];
/// Decodes that may run at once, the visible page included.
const DECODE_WORKERS: [u32; 4] = [1, 2, 3, 4];
/// Pages decoded ahead of the cursor while the viewer is idle.
const PREPARE_PAGES: [u32; 5] = [0, 8, 16, 32, 64];
/// Decoded pages the cache may keep.
const CACHE_PAGES: [u32; 6] = [8, 16, 24, 32, 48, 64];
/// Slot files the swap store may keep.
const SWAP_SLOTS: [u32; 6] = [2, 4, 6, 8, 12, 16];
/// Capacity of one swap slot, in megabytes.
const SWAP_SLOT_MB: [u32; 6] = [8, 16, 24, 32, 48, 64];
/// Ceiling on the bytes the swap store maps, in megabytes.
const SWAP_CAPACITY_MB: [u32; 6] = [64, 128, 192, 256, 384, 512];
/// How long a toast stays on screen, in seconds.
const TOAST_SECONDS: [u32; 5] = [1, 2, 3, 5, 10];

pub fn render(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let group = view.settings_group;

    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(0x000000AA))
        .child(
            v_flex()
                .w(px(DIALOG_WIDTH))
                .h(px(640.))
                .rounded(px(14.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .overflow_hidden()
                .child(header(cx))
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .child(nav(group, cx))
                        .child(
                            div()
                                .id("settings-body")
                                .w(px(BODY_WIDTH))
                                .h_full()
                                .overflow_y_scroll()
                                .px_5()
                                .py_3()
                                .child(content(group, view, cx)),
                        ),
                )
                .child(footer(group, cx)),
        )
}

fn header(cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .w_full()
        .px_4()
        .py_3()
        .justify_between()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(IconName::Settings).size(px(16.)))
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(s::SETTINGS())),
        )
        .child(
            div()
                .id("settings-close")
                .p_1()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|this| this.bg(theme.button_hover))
                .child(Icon::new(IconName::Close).size(px(15.)))
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.show_settings = false;
                    cx.notify();
                })),
        )
}

fn nav(active: Group, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let mut rail = v_flex()
        .w(px(NAV_WIDTH))
        .h_full()
        .flex_shrink_0()
        .py_2()
        .px_2()
        .gap_0p5()
        .bg(theme.sidebar)
        .border_r_1()
        .border_color(theme.sidebar_border);

    for group in Group::ALL {
        let selected = group == active;
        rail = rail.child(
            div()
                .id(("nav", group as usize))
                .w_full()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .when(selected, |this| this.bg(theme.sidebar_accent))
                .hover(|this| this.bg(theme.sidebar_accent))
                .child(group.label())
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.settings_group = group;
                    cx.notify();
                })),
        );
    }
    rail
}

fn footer(group: Group, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .w_full()
        .px_4()
        .py_2p5()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(theme.border)
        // Associations live in the registry, not in `Settings`; there is
        // nothing for "reset this page" to reset, and the page has its own
        // explicit unregister action.
        .when(group != Group::FileAssoc, |this| {
            this.child(
                div()
                    .id("settings-reset")
                    .px_3()
                    .py_1()
                    .rounded(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .border_1()
                    .border_color(theme.border)
                    .hover(|this| this.bg(theme.button_hover))
                    .child(seepic_core::i18n::tr("Reset This Page"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.settings.reset_group(group);
                        this.settings_changed(cx);
                    })),
            )
        })
        .child(div().flex_1())
        .child(
            div()
                .id("settings-done")
                .px_4()
                .py_1()
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .bg(theme.primary)
                .text_color(theme.primary_foreground)
                .hover(|this| this.opacity(0.9))
                .child(s::CONFIRM())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.show_settings = false;
                    cx.notify();
                })),
        )
}

fn content(group: Group, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    let mut body = v_flex().w_full().gap_1();

    body = match group {
        Group::General => basic_page(body, view, cx),
        Group::Interface => interface_page(body, view, cx),
        Group::FileAssoc => assoc_page(body, view, cx),
        Group::SystemMenu => shell_page(body, view, cx),
        Group::Keyboard => keyboard_page(body, view, cx),
        Group::Mouse => mouse_page(body, view, cx),
        Group::Advanced => advanced_page(body, view, cx),
        Group::Other => other_page(body, view, cx),
        Group::Language => language_page(body, view, cx),
    };
    body
}

// ---- rows ----------------------------------------------------------------

fn page_title(title: &'static str) -> impl IntoElement {
    div()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .pb_2()
        .child(seepic_core::i18n::tr(title))
}

fn section_label(title: &'static str, cx: &Context<ViewerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .mt_2()
        .mb_1()
        .text_color(cx.theme().muted_foreground)
        .child(seepic_core::i18n::tr(title))
}

fn toggle_row(
    label: &'static str,
    checked: bool,
    cx: &mut Context<ViewerApp>,
    on_click: fn(&mut ViewerApp, &mut Context<ViewerApp>),
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(label)
        .w_full()
        .px_2()
        .py_2()
        .rounded(px(6.))
        .cursor_pointer()
        .justify_between()
        .items_center()
        .hover(|this| this.bg(theme.list_hover))
        .child(div().text_sm().child(seepic_core::i18n::tr(label)))
        .child(
            div()
                .w(px(34.))
                .h(px(18.))
                .rounded_full()
                .bg(if checked { theme.primary } else { theme.input })
                .p(px(2.))
                .flex()
                .when(checked, |this| this.justify_end())
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(if checked {
                            theme.primary_foreground
                        } else {
                            theme.muted_foreground
                        }),
                ),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            on_click(this, cx);
        }))
}

/// A row that opens a list of choices.
///
/// The list is shown at the row, with every option in it, rather than cycling
/// through the options one click at a time: a value that takes four clicks to
/// reach, with no way to see what is on the way, is not a choice.
fn choice_row(
    label: &'static str,
    value: impl Into<SharedString>,
    labels: Vec<SharedString>,
    selected: Option<usize>,
    cx: &mut Context<ViewerApp>,
    set: fn(&mut ViewerApp, usize, &mut Context<ViewerApp>),
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let value: SharedString = value.into();
    let value = seepic_core::i18n::tr_owned(&value)
        .map(SharedString::from)
        .unwrap_or(value);
    let labels: Vec<SharedString> = labels
        .into_iter()
        .map(|label| {
            seepic_core::i18n::tr_owned(&label)
                .map(SharedString::from)
                .unwrap_or(label)
        })
        .collect();
    h_flex()
        .id(label)
        .w_full()
        .px_2()
        .py_2()
        .rounded(px(6.))
        .cursor_pointer()
        .justify_between()
        .items_center()
        .hover(|this| this.bg(theme.list_hover))
        .child(
            div()
                .flex_shrink_0()
                .text_sm()
                .child(seepic_core::i18n::tr(label)),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(value),
                )
                .child(
                    Icon::new(IconName::ChevronDown)
                        .size(px(12.))
                        .text_color(theme.muted_foreground),
                ),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
            let position = event.position();
            this.open_choice(
                ChoicePopup {
                    anchor: seepic_core::Point::new(
                        f32::from(position.x),
                        f32::from(position.y),
                    ),
                    labels: labels.clone(),
                    selected,
                    set,
                },
                cx,
            );
        }))
}

/// The list of choices for the settings row that was clicked.
///
/// It hangs at the row, and the backdrop behind it swallows the next press: a
/// press outside the list closes it, and the settings panel underneath stays
/// where it was.
pub fn choice_popup(
    view: &ViewerApp,
    window: &Window,
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    let Some(choice) = view.choice.as_ref() else {
        return div().into_any_element();
    };
    let theme = cx.theme().clone();
    let width = f32::from(window.bounds().size.width);
    // The overlay is laid out inside the content area, which starts below the
    // chrome when the chrome takes its own rows.
    let chrome = view.chrome_height();
    let height = (f32::from(window.bounds().size.height) - chrome).max(120.);
    let list_height = (choice.labels.len() as f32 * CHOICE_ROW_HEIGHT + 8.).min(height - 16.);
    let left = choice
        .anchor
        .x
        .clamp(8., (width - CHOICE_WIDTH - 8.).max(8.));
    // Under the pointer when there is room, above it otherwise.
    let below = choice.anchor.y - chrome + 6.;
    let top = if below + list_height <= height - 8. {
        below
    } else {
        (choice.anchor.y - chrome - list_height - 6.).max(4.)
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_choice(cx);
            }),
        )
        .child(
            v_flex()
                .absolute()
                .top(px(top))
                .left(px(left))
                .w(px(CHOICE_WIDTH))
                .py_1()
                .rounded(px(8.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .text_sm()
                // A press inside the list belongs to the list, not to the
                // backdrop that closes it.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    }),
                )
                .children(choice.labels.iter().enumerate().map(|(index, label)| {
                    choice_item(
                        index,
                        label.clone(),
                        choice.selected == Some(index),
                        cx,
                    )
                })),
        )
        .into_any_element()
}

/// One entry in a settings list.
fn choice_item(
    index: usize,
    label: SharedString,
    selected: bool,
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(("settings-choice", index))
        .h(px(CHOICE_ROW_HEIGHT))
        .flex_shrink_0()
        .px_3()
        .gap_2()
        .items_center()
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.list_active))
        .hover(|this| this.bg(theme.list_hover))
        .child(
            div().w(px(14.)).flex_shrink_0().when(selected, |this| {
                this.child(
                    Icon::new(IconName::Check)
                        .size(px(12.))
                        .text_color(theme.primary),
                )
            }),
        )
        .child(div().flex_1().truncate().child(label))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.apply_choice(index, cx);
        }))
        .into_any_element()
}

/// One of a small set of mutually exclusive choices, shown as a radio row.
fn radio_row(
    id: (&'static str, usize),
    label: &'static str,
    selected: bool,
    action: crate::panels::Action,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    h_flex()
        .id(id)
        .w_full()
        .gap_2p5()
        .px_2()
        .py_2()
        .rounded(px(6.))
        .cursor_pointer()
        .items_center()
        .hover(|this| this.bg(theme.list_hover))
        .child(
            div()
                .size(px(14.))
                .rounded_full()
                .border_1()
                .border_color(if selected { theme.primary } else { theme.border })
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |this| {
                    this.child(div().size(px(7.)).rounded_full().bg(theme.primary))
                }),
        )
        .child(
            div()
                .flex_1()
                .text_sm()
                .child(seepic_core::i18n::tr(label)),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            crate::panels::run_action(this, action, window, cx);
        }))
}

fn info_row(label: &'static str, value: String, cx: &Context<ViewerApp>) -> impl IntoElement {
    let value = seepic_core::i18n::tr_owned(&value)
        .map(str::to_string)
        .unwrap_or(value);
    h_flex()
        .w_full()
        .px_2()
        .py_1p5()
        .gap_3()
        .items_start()
        .child(
            div()
                .flex_shrink_0()
                .pt_1()
                .text_sm()
                .child(seepic_core::i18n::tr(label)),
        )
        .child(
            // `min_w_0` lets the box shrink below the length of its own text, so
            // a long value wraps inside the dialog instead of running off the
            // edge with the end of the sentence clipped away.
            div()
                .flex_1()
                .min_w_0()
                .px_2()
                .py_1()
                .rounded(px(5.))
                .bg(cx.theme().input)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(value),
        )
}

// ---- pages ---------------------------------------------------------------

fn basic_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    body = body
        .child(page_title(seepic_core::i18n::tr("General")))
        .child(toggle_row(seepic_core::i18n::tr("Quit on ESC"), view.settings.esc_to_exit, cx, |this, cx| {
            this.settings.esc_to_exit = !this.settings.esc_to_exit;
            this.settings_changed(cx);
        }))
        .child(toggle_row(
            seepic_core::i18n::tr("Single Instance Only"),
            view.settings.single_instance,
            cx,
            |this, cx| {
                this.settings.single_instance = !this.settings.single_instance;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(seepic_core::i18n::tr("Confirm Before Deleting"), view.settings.confirm_delete, cx, |this, cx| {
            this.settings.confirm_delete = !this.settings.confirm_delete;
            this.settings_changed(cx);
        }))
        .child(section_label(seepic_core::i18n::tr("At the Boundary"), cx))
        .child(choice_row(
            seepic_core::i18n::tr("In a Folder"),
            view.settings.boundary_action.label(),
            BoundaryAction::ALL.iter().map(|a| a.label().into()).collect(),
            BoundaryAction::ALL
                .iter()
                .position(|a| *a == view.settings.boundary_action),
            cx,
            |this, index, cx| {
                this.settings.boundary_action = BoundaryAction::ALL[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("In an Archive"),
            view.settings.archive_boundary_action.label(),
            BoundaryAction::ALL.iter().map(|a| a.label().into()).collect(),
            BoundaryAction::ALL
                .iter()
                .position(|a| *a == view.settings.archive_boundary_action),
            cx,
            |this, index, cx| {
                this.settings.archive_boundary_action = BoundaryAction::ALL[index];
                this.settings_changed(cx);
            },
        ));
    body
}

fn interface_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    body = body
        .child(page_title(seepic_core::i18n::tr("Interface")))
        .child(section_label(seepic_core::i18n::tr("Chrome"), cx))
        .children(ChromeMode::ALL.into_iter().map(|mode| {
            radio_row(
                ("chrome-mode", mode as usize),
                mode.label(),
                view.settings.chrome_mode == mode,
                crate::panels::Action::ChromeMode(mode),
                cx,
            )
            .into_any_element()
        }))
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(view.settings.chrome_mode.description()),
        )
        .child(section_label(seepic_core::i18n::tr("Thumbnails"), cx))
        .children(ThumbnailView::ALL.into_iter().map(|choice| {
            radio_row(
                ("thumbnail-view", choice as usize),
                choice.label(),
                view.settings.thumbnail_view == choice,
                crate::panels::Action::ThumbnailView(choice),
                cx,
            )
            .into_any_element()
        }))
        .child(section_label(seepic_core::i18n::tr("Display"), cx))
        .child(toggle_row(
            seepic_core::i18n::tr("Top Bar Icon Toolbar"),
            view.settings.show_toolbar,
            cx,
            |this, cx| {
                this.settings.show_toolbar = !this.settings.show_toolbar;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(seepic_core::i18n::tr("Image Border"), view.settings.show_border, cx, |this, cx| {
            this.settings.show_border = !this.settings.show_border;
            this.settings_changed(cx);
        }))
        .child(toggle_row(seepic_core::i18n::tr("Page Arrows"), view.settings.show_side_arrows, cx, |this, cx| {
            this.settings.show_side_arrows = !this.settings.show_side_arrows;
            this.settings_changed(cx);
        }))
        .child(toggle_row(
            seepic_core::i18n::tr("Animation Frame Info"),
            view.settings.show_anim_info,
            cx,
            |this, cx| {
                this.settings.show_anim_info = !this.settings.show_anim_info;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(seepic_core::i18n::tr("Operation"), view.settings.show_message, cx, |this, cx| {
            this.settings.show_message = !this.settings.show_message;
            this.settings_changed(cx);
        }))
        .child(toggle_row(
            seepic_core::i18n::tr("Click Canvas to Toggle Chrome"),
            view.settings.click_canvas_shows_chrome,
            cx,
            |this, cx| {
                this.settings.click_canvas_shows_chrome = !this.settings.click_canvas_shows_chrome;
                this.settings_changed(cx);
            },
        ))
        .child(section_label(seepic_core::i18n::tr("Image Display"), cx))
        .child(toggle_row(
            seepic_core::i18n::tr("Auto-rotate by EXIF"),
            view.settings.exif_rotate,
            cx,
            |this, cx| {
                this.settings.exif_rotate = !this.settings.exif_rotate;
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Transparent Background"),
            view.settings.transparency.label(),
            Transparency::ALL.iter().map(|t| t.label().into()).collect(),
            Transparency::ALL
                .iter()
                .position(|t| *t == view.settings.transparency),
            cx,
            |this, index, cx| {
                this.settings.transparency = Transparency::ALL[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Zoom When Opened"),
            view.settings.zoom_preset.label(),
            ZoomPreset::ALL.iter().map(|z| z.label().into()).collect(),
            ZoomPreset::ALL
                .iter()
                .position(|z| *z == view.settings.zoom_preset),
            cx,
            |this, index, cx| {
                this.settings.zoom_preset = ZoomPreset::ALL[index];
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(seepic_core::i18n::tr("Stretch Small Images"), view.settings.stretch_small, cx, |this, cx| {
            this.settings.stretch_small = !this.settings.stretch_small;
            this.settings_changed(cx);
        }))
        .child(section_label(seepic_core::i18n::tr("Appearance"), cx))
        .child(choice_row(
            seepic_core::i18n::tr("Theme"),
            view.settings.theme_mode.label(),
            ThemeMode::ALL.iter().map(|t| t.label().into()).collect(),
            ThemeMode::ALL
                .iter()
                .position(|t| *t == view.settings.theme_mode),
            cx,
            |this, index, cx| {
                this.settings.theme_mode = ThemeMode::ALL[index];
                this.settings_changed(cx);
            },
        ))
        .child(section_label(seepic_core::i18n::tr("Window"), cx))
        .child(toggle_row(seepic_core::i18n::tr("Always on Top"), view.settings.always_on_top, cx, |this, cx| {
            this.settings.always_on_top = !this.settings.always_on_top;
            this.settings_changed(cx);
        }))
        .child(toggle_row(seepic_core::i18n::tr("Snap Windows"), view.settings.snap_windows, cx, |this, cx| {
            this.settings.snap_windows = !this.settings.snap_windows;
            this.settings_changed(cx);
        }))
        .child(toggle_row(
            seepic_core::i18n::tr("Remember Window Size"),
            view.settings.remember_window_size,
            cx,
            |this, cx| {
                this.settings.remember_window_size = !this.settings.remember_window_size;
                this.settings_changed(cx);
            },
        ));
    body
}

fn advanced_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    body = body
        .child(page_title(seepic_core::i18n::tr("Advanced")))
        .child(section_label(seepic_core::i18n::tr("Cache & Decode"), cx))
        .child(toggle_row(seepic_core::i18n::tr("Pre-cache Neighbours"), view.settings.cache_next, cx, |this, cx| {
            this.settings.cache_next = !this.settings.cache_next;
            this.settings_changed(cx);
        }))
        .child(choice_row(
            seepic_core::i18n::tr("Memory Budget"),
            format!("{} MB", view.settings.memory_budget_mb),
            MEMORY_BUDGET_CHOICES
                .iter()
                .map(|mb| format!("{mb} MB").into())
                .collect(),
            MEMORY_BUDGET_CHOICES
                .iter()
                .position(|mb| *mb == view.settings.memory_budget_mb),
            cx,
            |this, index, cx| {
                this.settings.memory_budget_mb = MEMORY_BUDGET_CHOICES[index];
                this.settings_changed(cx);
            },
        ))
        .child(info_row(
            seepic_core::i18n::tr("Budget Allocation"),
            if view.settings.swap_frames {
                // The decoded pixels are file pages, so the decode share is not
                // memory at all any more; it is spent on the swap store's disk.
                seepic_core::trf!("{} (decode → swap / textures {}, capped)",
                    "{}（解码 → 交换页 / 纹理 {}，缓存上限）",
                    view.settings.memory_budget_mb,
                    view.settings.render_budget() / (1024 * 1024),
                )
            } else {
                seepic_core::trf!("{} (decode {} / textures {}, capped)",
                    "{}（解码 {} / 纹理 {}，缓存上限）",
                    view.settings.memory_budget_mb,
                    view.settings.decode_budget() / (1024 * 1024),
                    view.settings.render_budget() / (1024 * 1024),
                )
            },
            cx,
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Low-quality Preview First"),
            view.settings.low_quality_preview,
            cx,
            |this, cx| {
                this.settings.low_quality_preview = !this.settings.low_quality_preview;
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Max Decode Edge"),
            format!("{} px", view.settings.decode_max_edge()),
            DECODE_EDGES
                .iter()
                .map(|edge| format!("{edge} px").into())
                .collect(),
            DECODE_EDGES
                .iter()
                .position(|edge| *edge == view.settings.decode_max_edge()),
            cx,
            |this, index, cx| {
                this.settings.decode_max_edge = DECODE_EDGES[index];
                this.settings_changed(cx);
            },
        ))
        .child(info_row(
            seepic_core::i18n::tr("Largest Frame"),
            seepic_core::trf!("About {} MB ({} × {} × 4)",
                "约 {} MB（{} × {} × 4）",
                view.settings.decode_max_edge() as usize * view.settings.decode_max_edge() as usize
                    * 4
                    / (1024 * 1024),
                view.settings.decode_max_edge(),
                view.settings.decode_max_edge()
            ),
            cx,
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Prefetch Window"),
            match view.settings.prefetch_window {
                0 => seepic_core::i18n::tr("No prefetch").to_string(),
                n => seepic_core::trf!("{} images on each side", "前后各 {} 张", n),
            },
            PREFETCH_WINDOWS
                .iter()
                .map(|n| {
                    if *n == 0 {
                        seepic_core::i18n::tr("No prefetch").to_string().into()
                    } else {
                        seepic_core::trf!("{} images on each side", "前后各 {} 张", *n).into()
                    }
                })
                .collect(),
            PREFETCH_WINDOWS
                .iter()
                .position(|n| *n == view.settings.prefetch_window),
            cx,
            |this, index, cx| {
                this.settings.prefetch_window = PREFETCH_WINDOWS[index];
                this.settings_changed(cx);
            },
        ))
        .child(section_label(seepic_core::i18n::tr("Decode Scheduling"), cx))
        .child(choice_row(
            seepic_core::i18n::tr("Parallel Decodes"),
            match view.settings.decode_workers {
                1 => seepic_core::i18n::tr("1 (serial)").to_string(),
                n => format!("{n}"),
            },
            DECODE_WORKERS
                .iter()
                .map(|n| {
                    if *n == 1 {
                        seepic_core::i18n::tr("1 (serial)").to_string().into()
                    } else {
                        format!("{n}").into()
                    }
                })
                .collect(),
            DECODE_WORKERS
                .iter()
                .position(|n| *n == view.settings.decode_workers),
            cx,
            |this, index, cx| {
                this.settings.decode_workers = DECODE_WORKERS[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Prepared Pages"),
            match view.settings.prepare_pages {
                0 => seepic_core::i18n::tr("Close").to_string(),
                n => seepic_core::trf!("{} pages", "{} 页", n),
            },
            PREPARE_PAGES
                .iter()
                .map(|n| {
                    if *n == 0 {
                        seepic_core::i18n::tr("Close").to_string().into()
                    } else {
                        seepic_core::trf!("{} pages", "{} 页", *n).into()
                    }
                })
                .collect(),
            PREPARE_PAGES
                .iter()
                .position(|n| *n == view.settings.prepare_pages),
            cx,
            |this, index, cx| {
                this.settings.prepare_pages = PREPARE_PAGES[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Decoded Cache Pages"),
            seepic_core::trf!("{} pages", "{} 页", view.settings.cache_pages),
            CACHE_PAGES
                .iter()
                .map(|n| seepic_core::trf!("{} pages", "{} 页", *n).into())
                .collect(),
            CACHE_PAGES
                .iter()
                .position(|n| *n == view.settings.cache_pages),
            cx,
            |this, index, cx| {
                this.settings.cache_pages = CACHE_PAGES[index];
                this.settings_changed(cx);
            },
        ))
        .child(info_row(
            seepic_core::i18n::tr("Decode Scheduling"),
            seepic_core::i18n::tr("Parallel decodes is the number of pages decoded at once (including the current one): 1 is serial; 2+ decodes neighbours alongside the current page, making held-arrow paging faster at the cost of 2–4 transient decoded frames (about 100 MB each for a 20 MP progressive image). Prepared pages are decoded ahead while idle, roughly the drawn size (5–8 MB each), capped at 192 MB. Decoded cache pages bounds retained decoded pages; with the swap store on, the real depth is also limited by slots.")
                .to_string(),
            cx,
        ))
        .child(section_label(seepic_core::i18n::tr("Memory Swap Pages"), cx))
        .child(toggle_row(
            seepic_core::i18n::tr("Enable Memory Swap"),
            view.settings.swap_frames,
            cx,
            |this, cx| {
                this.settings.swap_frames = !this.settings.swap_frames;
                this.settings_changed(cx);
            },
        ))
        .child(info_row(
            seepic_core::i18n::tr("Swap Store"),
            seepic_core::i18n::tr("Either this or the in-memory cache: decoded frames go into mapped temporary files (they still use physical memory, but as file pages, so private memory no longer grows with the number of images opened). The memory budget is then used for textures only; prepared pages and frames smaller than a quarter of a slot stay in memory, bounded by “Prepared pages”. When off, frames stay in memory only.")
                .to_string(),
            cx,
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Slots"),
            format!("{}", view.settings.swap_slots),
            SWAP_SLOTS.iter().map(|n| format!("{n}").into()).collect(),
            SWAP_SLOTS
                .iter()
                .position(|n| *n == view.settings.swap_slots),
            cx,
            |this, index, cx| {
                this.settings.swap_slots = SWAP_SLOTS[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Slot Size"),
            format!("{} MB", view.settings.swap_slot_mb),
            SWAP_SLOT_MB
                .iter()
                .map(|mb| format!("{mb} MB").into())
                .collect(),
            SWAP_SLOT_MB
                .iter()
                .position(|mb| *mb == view.settings.swap_slot_mb),
            cx,
            |this, index, cx| {
                this.settings.swap_slot_mb = SWAP_SLOT_MB[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Disk Reservation Cap"),
            format!("{} MB", view.settings.swap_capacity_mb),
            SWAP_CAPACITY_MB
                .iter()
                .map(|mb| format!("{mb} MB").into())
                .collect(),
            SWAP_CAPACITY_MB
                .iter()
                .position(|mb| *mb == view.settings.swap_capacity_mb),
            cx,
            |this, index, cx| {
                this.settings.swap_capacity_mb = SWAP_CAPACITY_MB[index];
                this.settings_changed(cx);
            },
        ))
        .child(swap_usage_row(view, cx))
        .child(toggle_row(
            seepic_core::i18n::tr("Use Image ICC Profile"),
            view.settings.use_image_icc,
            cx,
            |this, cx| {
                this.settings.use_image_icc = !this.settings.use_image_icc;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Use Monitor ICC Profile"),
            view.settings.use_monitor_icc,
            cx,
            |this, cx| {
                this.settings.use_monitor_icc = !this.settings.use_monitor_icc;
                this.settings_changed(cx);
            },
        ))
        .child(section_label(seepic_core::i18n::tr("Accepted Files"), cx))
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    seepic_core::i18n::tr("The whitelist and blacklist match extensions and only decide whether entries in folders and archives are listed."),
                ),
        )
        .child(toggle_row(
            seepic_core::i18n::tr("Only List Whitelist Extensions"),
            view.settings.use_image_whitelist,
            cx,
            |this, cx| {
                this.settings.use_image_whitelist = !this.settings.use_image_whitelist;
                this.settings_changed(cx);
            },
        ))
        .child(info_row(seepic_core::i18n::tr("Whitelist Extensions"), view.settings.image_whitelist.clone(), cx))
        .child(toggle_row(
            seepic_core::i18n::tr("Ignore Blacklist Extensions"),
            view.settings.use_image_blacklist,
            cx,
            |this, cx| {
                this.settings.use_image_blacklist = !this.settings.use_image_blacklist;
                this.settings_changed(cx);
            },
        ))
        .child(info_row(seepic_core::i18n::tr("Blacklist Extensions"), view.settings.image_blacklist.clone(), cx));
    body
}

fn keyboard_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    body = body
        .child(page_title(seepic_core::i18n::tr("Keyboard")))
        .child(toggle_row(
            seepic_core::i18n::tr("Arrow Keys Scroll Only"),
            view.settings.arrow_scroll_only,
            cx,
            |this, cx| {
                this.settings.arrow_scroll_only = !this.settings.arrow_scroll_only;
                this.settings_changed(cx);
            },
        ))
        .child(
            div()
                .pt_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    seepic_core::i18n::tr("Every command and its default shortcut. “Enabled” means the command can be bound; click “Record” and press a new chord to rebind it. Conflicts with another command’s default are rejected. A “custom” mark means the key was changed; “Clear” restores the default."),
                ),
        );

    // Grouped by settings page, and every command is listed with its default
    // rather than the table starting empty.
    for group in seepic_core::command::Group::ALL {
        let commands: Vec<Command> = Command::in_group(*group).collect();
        if commands.is_empty() {
            continue;
        }
        body = body.child(section_label(group.label(), cx));
        for command in commands {
            let custom = view.settings.has_custom_binding(command);
            let binding = view
                .settings
                .effective_binding(command)
                .unwrap_or_else(|| seepic_core::i18n::tr("None").to_string());
            let recording = view.recording_key == Some(command);
            let theme = cx.theme().clone();
            let is_default = command.default_binding().is_some();
            body = body.child(
                h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_3()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .w(px(28.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if is_default { "✓" } else { "" }),
                    )
                    .child(
                        div()
                            .w(px(132.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(if recording {
                                theme.primary
                            } else if custom {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .child(if recording {
                                seepic_core::i18n::tr("Press a New Key…").to_string()
                            } else if custom {
                                format!("{binding}（自定义）")
                            } else {
                                binding
                            }),
                    )
                    .child(
                        div()
                            .w(px(196.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(command.id()),
                    )
                    .child(key_action(seepic_core::i18n::tr("Record"), recording, command, true, cx))
                    .when(custom, |this| {
                        this.child(key_action(seepic_core::i18n::tr("Clear"), false, command, false, cx))
                    })
                    .child(div().flex_1().text_sm().child(command.label())),
            );
        }
    }
    body
}

/// A small per-row button on the keyboard page.
fn key_action(
    label: &'static str,
    recording: bool,
    command: Command,
    bind: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
        .id(format!("{label}-{}", command.id()))
        .px_2()
        .py_0p5()
        .rounded(px(5.))
        .border_1()
        .border_color(if recording { theme.primary } else { theme.border })
        .text_xs()
        .flex_shrink_0()
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            if bind {
                this.start_key_recording(command, cx);
            } else {
                this.clear_key_binding(command, cx);
            }
        }))
}

fn mouse_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    body = body
        .child(page_title(seepic_core::i18n::tr("Mouse")))
        .child(
            div()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(seepic_core::i18n::tr("Each key can run any command; click “Choose Command” to open the list.")),
        );

    for slot in MouseSlot::ALL {
        let binding = view
            .settings
            .mouse_bindings
            .iter()
            .find(|binding| binding.slot == slot);
        let enabled = binding.is_some_and(|binding| binding.enabled);
        let label = binding
            .and_then(|binding| binding.command.as_deref())
            .and_then(Command::from_id)
            .map(|command| command.label().to_string())
            .unwrap_or_else(|| seepic_core::i18n::tr("Do Nothing").to_string());

        body = body
            .child(toggle_row(
                slot.label(),
                enabled,
                cx,
                match slot {
                    MouseSlot::Left => |this: &mut ViewerApp, cx| {
                        set_mouse_enabled(this, MouseSlot::Left, cx)
                    },
                    MouseSlot::DoubleLeft => |this: &mut ViewerApp, cx| {
                        set_mouse_enabled(this, MouseSlot::DoubleLeft, cx)
                    },
                    MouseSlot::Middle => |this: &mut ViewerApp, cx| {
                        set_mouse_enabled(this, MouseSlot::Middle, cx)
                    },
                    MouseSlot::Right => |this: &mut ViewerApp, cx| {
                        set_mouse_enabled(this, MouseSlot::Right, cx)
                    },
                    MouseSlot::X => {
                        |this: &mut ViewerApp, cx| set_mouse_enabled(this, MouseSlot::X, cx)
                    }
                    MouseSlot::Y => {
                        |this: &mut ViewerApp, cx| set_mouse_enabled(this, MouseSlot::Y, cx)
                    }
                },
            ))
            .child(
                h_flex()
                    .w_full()
                    .pl_8()
                    .pr_2()
                    .py_0p5()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(
                        div()
                            .id(("mouse-pick", slot as usize))
                            .px_2()
                            .py_0p5()
                            .rounded(px(5.))
                            .text_xs()
                            .cursor_pointer()
                            .border_1()
                            .border_color(cx.theme().border)
                            .hover(|this| this.bg(cx.theme().button_hover))
                            .child(seepic_core::i18n::tr("Choose Command"))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.open_command_picker(Some(slot), cx);
                            })),
                    ),
            );
    }

    body = body
        .child(section_label(seepic_core::i18n::tr("Wheel"), cx))
        .child(choice_row(
            seepic_core::i18n::tr("Wheel Action"),
            view.settings.wheel_action.label(),
            WheelAction::ALL.iter().map(|a| a.label().into()).collect(),
            WheelAction::ALL
                .iter()
                .position(|a| *a == view.settings.wheel_action),
            cx,
            |this, index, cx| {
                this.settings.wheel_action = WheelAction::ALL[index];
                this.settings_changed(cx);
            },
        ))
        .child(choice_row(
            seepic_core::i18n::tr("Wheel Speed"),
            speed_label(view.settings.wheel_speed),
            WHEEL_SPEEDS
                .iter()
                .map(|speed| speed_label(*speed).into())
                .collect(),
            WHEEL_SPEEDS
                .iter()
                .position(|speed| speed_label(*speed) == speed_label(view.settings.wheel_speed)),
            cx,
            |this, index, cx| {
                this.settings.wheel_speed = WHEEL_SPEEDS[index];
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Wheel Bounce"),
            view.settings.wheel_bounce,
            cx,
            |this, cx| {
                this.settings.wheel_bounce = !this.settings.wheel_bounce;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Click Edges to Turn Pages"),
            view.settings.click_edges_to_page,
            cx,
            |this, cx| {
                this.settings.click_edges_to_page = !this.settings.click_edges_to_page;
                this.settings_changed(cx);
            },
        ));
    body
}

/// Flip one mouse slot on or off, giving it a sensible command the first time.
fn set_mouse_enabled(view: &mut ViewerApp, slot: MouseSlot, cx: &mut Context<ViewerApp>) {
    let default = slot
        .default_command()
        .map(|command| command.id().to_string());
    if let Some(binding) = view
        .settings
        .mouse_bindings
        .iter_mut()
        .find(|binding| binding.slot == slot)
    {
        binding.enabled = !binding.enabled;
        if binding.enabled && binding.command.is_none() {
            binding.command = default;
        }
    } else {
        view.settings.mouse_bindings.push(seepic_core::settings::MouseBinding {
            slot,
            enabled: true,
            command: default,
        });
    }
    view.settings_changed(cx);
}

fn speed_label(speed: u8) -> &'static str {
    match speed {
        0..=2 => seepic_core::i18n::tr("Slow"),
        3..=4 => seepic_core::i18n::tr("Normal"),
        _ => seepic_core::i18n::tr("Fast"),
    }
}

/// What the swap store is holding right now, and what it is allowed.
///
/// The two figures are worth showing together: a capacity smaller than
/// `slots × slot size` quietly lowers the number of slots, which is the kind of
/// thing that is otherwise only visible as "the setting did nothing".
fn swap_usage_row(view: &ViewerApp, cx: &Context<ViewerApp>) -> AnyElement {
    let config = view.settings.swap_config();
    let stats = view.swap_stats();
    let reserved = config.slots * config.slot_bytes / (1024 * 1024);
    let usable = config.usable_slots();
    let mut text = seepic_core::trf!("Mapped {} / {} MB · {} slot files · {} reused · {} MB reserved",
        "已映射 {} / {} MB · 槽位文件 {} · 复用 {} · 预留磁盘 {} MB",
        stats.mapped_bytes / (1024 * 1024),
        config.capacity_bytes / (1024 * 1024),
        stats.slot_files,
        stats.reused,
        reserved
    );
    if usable < config.slots {
        text.push_str(&seepic_core::trf!(" (cap fits only {} slots)",
            "（上限只够 {} 个槽位）",
            usable
        ));
    }
    if stats.refused > 0 {
        text.push_str(&seepic_core::trf!(" · {} frames left on the heap",
            " · 留在堆上 {} 张",
            stats.refused
        ));
    }
    // With the store on, the decoded cache is sized from the disk reservation
    // rather than the memory budget — but only the large frames are in slot
    // files; the previews are small enough that a slot would cost more than
    // they do.
    let cache = view.cache_usage();
    if view.settings.swap_frames {
        text.push_str(&seepic_core::trf!(" · decoded cache {} / {} MB ({} pages; large frames in swap, prepared pages in memory)",
            " · 解码缓存 {} / {} MB（{} 页，大图在交换页，预生成页在内存）",
            cache.0 / (1024 * 1024),
            cache.1 / (1024 * 1024),
            cache.2
        ));
    }
    info_row(seepic_core::i18n::tr("Swap Usage"), text, cx).into_any_element()
}

/// The format-association page.
///
/// Association is per-user registry state, so it is read straight from HKCU
/// and written back there: no elevation, and nothing to keep in sync on disk.
/// Windows 8 and later keep the user's explicit default in `UserChoice`, which
/// a program cannot overwrite; the page therefore registers SeePic as a
/// handler and points at the system page where the user confirms the default.
fn assoc_page(mut body: Div, _view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    body = body
        .child(page_title(seepic_core::i18n::tr("File Associations")))
        .child(
            div()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    seepic_core::i18n::tr("Register SeePic as a handler for the formats below. Windows 10/11 does not let a program replace an existing default; choose SeePic in Windows Settings after registering."),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .pb_1()
                .gap_2()
                .flex_wrap()
                .child(primary_button(seepic_core::i18n::tr("Register All Formats"), cx, register_all_formats))
                .child(button(seepic_core::i18n::tr("Remove All Associations"), cx, unregister_all_formats))
                .child(button(
                    seepic_core::i18n::tr("Open Default Apps"),
                    cx,
                    open_system_default_apps,
                )),
        );

    for (header, extensions) in association_table() {
        body = body.child(section_label(header, cx));
        for extension in extensions {
            body = body.child(assoc_row(extension, cx));
        }
    }

    body
}

fn all_formats() -> Vec<&'static str> {
    association_table()
        .into_iter()
        .flat_map(|(_, extensions)| extensions)
        .collect()
}

fn register_all_formats(this: &mut ViewerApp, cx: &mut Context<ViewerApp>) {
    match crate::file_assoc::register_all(&all_formats()) {
        Ok(count) => this.push_toast(
            format!("已将 SeePic 注册为 {count} 种格式的打开方式"),
            false,
            cx,
        ),
        Err(error) => this.push_toast(format!("关联失败：{error}"), true, cx),
    }
    cx.notify();
}

fn unregister_all_formats(this: &mut ViewerApp, cx: &mut Context<ViewerApp>) {
    match crate::file_assoc::unregister_all(&all_formats()) {
        Ok(count) => this.push_toast(format!("已解除 {count} 种格式的 SeePic 关联"), false, cx),
        Err(error) => this.push_toast(format!("解除关联失败：{error}"), true, cx),
    }
    cx.notify();
}

fn open_system_default_apps(this: &mut ViewerApp, cx: &mut Context<ViewerApp>) {
    if !crate::file_assoc::open_default_apps() {
        this.push_toast(seepic_core::i18n::tr("Could not open Default Apps"), true, cx);
    }
    cx.notify();
}

/// One extension, its icon, the app Windows opens it with, and the toggle.
fn assoc_row(extension: &'static str, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let state = crate::file_assoc::current(extension);
    let registered = state.registered;
    let owner = match (&state.owner, state.is_default) {
        (_, true) => "SeePic".to_string(),
        (Some(owner), _) => owner.clone(),
        (None, _) => seepic_core::i18n::tr("Not associated").to_string(),
    };
    let owner_color = if state.is_default {
        theme.primary
    } else if registered {
        theme.foreground
    } else {
        theme.muted_foreground
    };

    h_flex()
        .id(format!("assoc-row-{extension}"))
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .child(format_icon(extension, cx))
        .child(
            div()
                .w(px(56.))
                .flex_shrink_0()
                .text_sm()
                .child(format!(".{extension}")),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(owner_color)
                        .child(owner),
                )
                .when(registered && !state.is_default, |this| {
                    this.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(seepic_core::i18n::tr(" · SeePic available")),
                    )
                }),
        )
        .child(
            div()
                .id(format!("assoc-toggle-{extension}"))
                .px_2()
                .py_0p5()
                .rounded(px(4.))
                .text_xs()
                .cursor_pointer()
                .text_color(if registered {
                    theme.muted_foreground
                } else {
                    theme.primary
                })
                .hover(|this| this.bg(theme.button_hover))
                .child(if registered { seepic_core::i18n::tr("Remove") } else { seepic_core::i18n::tr("Register") })
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    let outcome = if registered {
                        crate::file_assoc::unregister(extension)
                    } else {
                        crate::file_assoc::register(extension)
                    };
                    match outcome {
                        Ok(()) => this.push_toast(
                            if registered {
                                format!(".{extension} 已解除关联")
                            } else {
                                format!(".{extension} 已关联到 SeePic")
                            },
                            false,
                            cx,
                        ),
                        Err(error) => {
                            this.push_toast(format!("操作失败：{error}"), true, cx)
                        }
                    }
                    cx.notify();
                })),
        )
}

fn format_icon(extension: &str, cx: &Context<ViewerApp>) -> AnyElement {
    if let Some(render) = crate::format_icons::render(extension) {
        return img(ImageSource::Render(render))
            .size(px(20.))
            .object_fit(ObjectFit::Contain)
            .into_any_element();
    }
    Icon::new(IconName::FileImage)
        .size(px(18.))
        .text_color(cx.theme().muted_foreground)
        .into_any_element()
}

/// A filled button for the page's main action.
fn primary_button(
    label: &'static str,
    cx: &mut Context<ViewerApp>,
    on_click: fn(&mut ViewerApp, &mut Context<ViewerApp>),
) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
        .id(label)
        .px_3()
        .py_1()
        .rounded(px(6.))
        .text_sm()
        .cursor_pointer()
        .bg(theme.primary)
        .text_color(theme.primary_foreground)
        .hover(|this| this.opacity(0.9))
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
}

/// The formats SeePic can actually open.
///
/// The list used to mirror a viewer that handles RAW and editor formats; those
/// rows registered SeePic for files it cannot decode. Keeping the table equal
/// to the decoders and archive backends is what makes every association here
/// an association that works.
pub(crate) fn association_table() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            seepic_core::i18n::tr("Common Image Formats"),
            vec![
                "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "dib", "ico",
                "tif", "tiff", "tga",
            ],
        ),
        (
            seepic_core::i18n::tr("Other Image Formats"),
            vec![
                "pnm", "pbm", "pgm", "ppm", "pam", "dds", "hdr", "exr", "qoi", "ff", "farbfeld",
            ],
        ),
        (seepic_core::i18n::tr("Comic Archives"), vec!["cbz", "cbr", "cb7", "cbt"]),
        (seepic_core::i18n::tr("Other Archives"), vec!["zip", "7z", "rar", "tar"]),
    ]
}

/// The file-manager context-menu page.
fn shell_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    let settings = &view.settings;

    // The details are hidden while the master switch is off: a sub-toggle
    // that cannot reach the registry would look like a setting that does
    // nothing.
    let mut options = v_flex().w_full();
    if settings.shell_menu_enable {
        options = options
            .child(section_label(seepic_core::i18n::tr("Context Menu Items"), cx))
            .child(toggle_row(
                seepic_core::i18n::tr("Browse Images"),
                settings.shell_menu_viewer,
                cx,
                |this, cx| {
                    this.settings.shell_menu_viewer = !this.settings.shell_menu_viewer;
                    this.settings_changed(cx);
                },
            ))
            .child(toggle_row(
                seepic_core::i18n::tr("Convert"),
                settings.shell_menu_convert,
                cx,
                |this, cx| {
                    this.settings.shell_menu_convert = !this.settings.shell_menu_convert;
                    this.settings_changed(cx);
                },
            ))
            .child(toggle_row(
                seepic_core::i18n::tr("Show Convert Only with Shift"),
                settings.shell_menu_convert_shift,
                cx,
                |this, cx| {
                    this.settings.shell_menu_convert_shift =
                        !this.settings.shell_menu_convert_shift;
                    this.settings_changed(cx);
                },
            ))
            .child(toggle_row(
                seepic_core::i18n::tr("Show on Folders"),
                settings.shell_menu_on_folder,
                cx,
                |this, cx| {
                    this.settings.shell_menu_on_folder = !this.settings.shell_menu_on_folder;
                    this.settings_changed(cx);
                },
            ))
            .child(toggle_row(
                seepic_core::i18n::tr("Show on Folder Background"),
                settings.shell_menu_on_background,
                cx,
                |this, cx| {
                    this.settings.shell_menu_on_background =
                        !this.settings.shell_menu_on_background;
                    this.settings_changed(cx);
                },
            ))
            .child(section_label(seepic_core::i18n::tr("Menu Icons"), cx))
            .child(toggle_row(
                seepic_core::i18n::tr("Show Program Icon"),
                settings.shell_preview_icon,
                cx,
                |this, cx| {
                    this.settings.shell_preview_icon = !this.settings.shell_preview_icon;
                    this.settings_changed(cx);
                },
            ));
    }

    body = body
        .child(page_title(seepic_core::i18n::tr("System Menu")))
        .child(
            div()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    seepic_core::i18n::tr("Right-click an image, a folder, or folder background in Explorer to see these items. Settings are written to the current user’s registry and apply immediately; no administrator rights are needed."),
                ),
        )
        .child(toggle_row(
            seepic_core::i18n::tr("Add to Explorer Context Menu"),
            settings.shell_menu_enable,
            cx,
            |this, cx| {
                this.settings.shell_menu_enable = !this.settings.shell_menu_enable;
                this.settings_changed(cx);
            },
        ))
        .child(options)
        .when_some(view.shell_menu_error.as_ref(), |this, error| {
            this.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(format!("无法写入右键菜单：{error}")),
            )
        })
        .child(
            div()
                .pt_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    seepic_core::i18n::tr("Windows 11 keeps classic context-menu items under “Show more options”. Explorer thumbnail previews need an extra COM component, which this build does not include."),
                ),
        );
    body
}

fn other_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    let folder = view.settings.photo_folder.clone();
    body = body
        .child(page_title(seepic_core::i18n::tr("Other")))
        .child(section_label(seepic_core::i18n::tr("Photo Folder"), cx))
        .child(info_row(
            seepic_core::i18n::tr("Path"),
            if folder.trim().is_empty() {
                seepic_core::i18n::tr("(not set)").to_string()
            } else {
                folder
            },
            cx,
        ))
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .child(button(seepic_core::i18n::tr("Choose Folder…"), cx, |this, cx| this.pick_photo_folder(cx)))
                .child(button(seepic_core::i18n::tr("Open Folder"), cx, |this, cx| this.open_photo_folder(cx))),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(seepic_core::i18n::tr("Used by copy and move; duplicate names get a suffix instead of overwriting.")),
        )
        .child(section_label(seepic_core::i18n::tr("Position Memory"), cx))
        .child(toggle_row(
            seepic_core::i18n::tr("Remember Page in Archive"),
            view.settings.remember_archive_position,
            cx,
            |this, cx| {
                this.settings.remember_archive_position = !this.settings.remember_archive_position;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Remember Page in Folder"),
            view.settings.remember_folder_position,
            cx,
            |this, cx| {
                this.settings.remember_folder_position = !this.settings.remember_folder_position;
                this.settings_changed(cx);
            },
        ))
        .child(info_row(seepic_core::i18n::tr("Bookmarks Folder"), seepic_core::i18n::tr("(not connected yet)").to_string(), cx))
        .child(info_row(seepic_core::i18n::tr("Recent"), seepic_core::i18n::tr("(not connected yet)").to_string(), cx))
        .child(section_label(seepic_core::i18n::tr("Other"), cx))
        .child(choice_row(
            seepic_core::i18n::tr("Toast Duration"),
            format!("{} 秒", view.settings.toast_seconds),
            TOAST_SECONDS
                .iter()
                .map(|seconds| format!("{seconds} 秒").into())
                .collect::<Vec<_>>(),
            TOAST_SECONDS
                .iter()
                .position(|seconds| *seconds == view.settings.toast_seconds),
            cx,
            |this, index, cx| {
                this.settings.toast_seconds = TOAST_SECONDS[index];
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Page Turn Sound"),
            view.settings.page_turn_sound,
            cx,
            |this, cx| {
                this.settings.page_turn_sound = !this.settings.page_turn_sound;
                this.settings_changed(cx);
            },
        ))
        .child(toggle_row(
            seepic_core::i18n::tr("Use Custom Temporary Folder"),
            view.settings.use_user_temp_folder,
            cx,
            |this, cx| {
                this.settings.use_user_temp_folder = !this.settings.use_user_temp_folder;
                this.settings_changed(cx);
            },
        ))
        .child(section_label(seepic_core::i18n::tr("Temporary Folder"), cx))
        .child(info_row(
            seepic_core::i18n::tr("Path"),
            {
                let value = view.settings.temp_folder.clone();
                if value.is_empty() {
                    std::env::temp_dir().display().to_string()
                } else {
                    value
                }
            },
            cx,
        ))
        .child(section_label(seepic_core::i18n::tr("Image Editor"), cx))
        .child(info_row(
            seepic_core::i18n::tr("Program"),
            if view.settings.external_editor.trim().is_empty() {
                seepic_core::i18n::tr("System Default Program").to_string()
            } else {
                view.settings.external_editor.clone()
            },
            cx,
        ))
        .child(section_label(seepic_core::i18n::tr("Configuration File"), cx))
        .child(info_row(seepic_core::i18n::tr("Path"), Settings::path().display().to_string(), cx))
        .child(info_row(
            seepic_core::i18n::tr("Config Version"),
            format!(
                "{}（当前 {}）",
                view.settings.schema_version,
                seepic_core::settings::SCHEMA_VERSION
            ),
            cx,
        ))
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .child(button(seepic_core::i18n::tr("Export Settings…"), cx, |this, cx| this.export_settings(cx)))
                .child(button(seepic_core::i18n::tr("Import Settings…"), cx, |this, cx| this.import_settings(cx))),
        )
        .child(
            div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(seepic_core::i18n::tr("Exports the complete JSON settings file for backup or moving to another computer.")),
        );
    body
}


/// A small action button.
fn button(
    label: &'static str,
    cx: &mut Context<ViewerApp>,
    on_click: fn(&mut ViewerApp, &mut Context<ViewerApp>),
) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
        .id(label)
        .px_2p5()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(theme.border)
        .text_sm()
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .child(seepic_core::i18n::tr(label))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
}

fn language_page(mut body: Div, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> Div {
    let settings = &view.settings;
    let selected = seepic_core::i18n::Language::from_code(settings.language.as_str())
        .map(|language| {
            seepic_core::i18n::Language::ALL
                .iter()
                .position(|candidate| *candidate == language)
                .unwrap_or(0)
                + 1
        })
        .unwrap_or(0);
    let current = settings.effective_language();
    let value = if selected == 0 {
        format!(
            "{} ({})",
            seepic_core::i18n::tr("Follow System"),
            current.native_label()
        )
    } else {
        current.native_label().to_string()
    };
    let labels: Vec<SharedString> = std::iter::once(seepic_core::i18n::tr("Follow System"))
        .chain(
            seepic_core::i18n::Language::ALL
                .iter()
                .map(|language| language.native_label()),
        )
        .map(SharedString::from)
        .collect();

    body = body
        .child(page_title(seepic_core::i18n::tr("Language")))
        .child(choice_row(
            seepic_core::i18n::tr("Interface Language"),
            value,
            labels,
            Some(selected),
            cx,
            |this, index, cx| {
                this.settings.language = match index {
                    0 => "system".to_string(),
                    _ => seepic_core::i18n::Language::ALL[index - 1]
                        .code()
                        .to_string(),
                };
                this.settings_changed(cx);
            },
        ))
        .child(
            div()
                .pt_3()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(seepic_core::i18n::tr("Changes take effect immediately.")),
        )
        .child(info_row(seepic_core::i18n::tr("Configuration File"), Settings::path().display().to_string(), cx));
    body
}
