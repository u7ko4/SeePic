//! The slideshow panel: an interval, and the few options that change how it
//! plays.
//!
//! Everything lives on this one screen. There used to be a second screen pushed
//! from a gear, which only split the same handful of controls across two
//! places; the panel now carries the loop, random, progress and page-turn
//! choices itself, and the interval it was last set to is saved with the rest of
//! the configuration.

use gpui_kit::assets::IconName;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::{Interval, IntervalUnit, PageTurnFx};

use crate::app::ViewerApp;
use crate::strings as s;

/// The interval presets the panel offers: whole seconds first, then the three
/// sub-second steps the panel is used with most.
const PRESETS: [(&str, u64); 8] = [
    ("1 秒", 1000),
    ("2 秒", 2000),
    ("3 秒", 3000),
    ("4 秒", 4000),
    ("5 秒", 5000),
    ("300 毫秒", 300),
    ("500 毫秒", 500),
    ("800 毫秒", 800),
];

/// The panel's contents, shown inside the shared modal frame in `panels`.
pub fn body(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let config = view.slideshow.config().clone();
    v_flex()
        .id("slideshow-popup-body")
        .w_full()
        .px_1()
        .gap_0p5()
        // Two durations, each a grid of the common choices plus a custom one.
        .child(duration_section(
            s::SLIDESHOW_INTERVAL,
            "interval",
            config.interval,
            view.slideshow_interval_unit,
            ViewerApp::set_slideshow_interval,
            ViewerApp::set_slideshow_interval_unit,
            cx,
        ))
        .child(duration_section(
            s::SLIDESHOW_DURATION,
            "display",
            config.display,
            view.slideshow_display_unit,
            ViewerApp::set_slideshow_display,
            ViewerApp::set_slideshow_display_unit,
            cx,
        ))
        .child(div().h(px(1.)).my_2().bg(theme.border))
        .child(switch_row(s::SLIDESHOW_LOOP, config.loop_playback, cx, |this, value, cx| {
            this.set_slideshow_loop(value, cx);
        }))
        .child(switch_row(s::SLIDESHOW_RANDOM, config.random, cx, |this, value, cx| {
            this.set_slideshow_random(value, cx);
        }))
        .child(switch_row(
            s::SLIDESHOW_PROGRESS,
            config.show_progress,
            cx,
            |this, value, cx| {
                this.set_slideshow_progress(value, cx);
            },
        ))
        .child(effect_row(config.page_turn_fx, cx))
        // Starting is the panel's action: it plays with the settings above and
        // gets out of the way.
        .child(
            div()
                .id("slideshow-start")
                .w_full()
                .mt_2()
                .py_2()
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .bg(theme.primary)
                .text_color(theme.primary_foreground)
                .hover(|this| this.opacity(0.9))
                .child(s::SLIDESHOW_START)
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.start_slideshow(cx);
                    this.close_panel(cx);
                })),
        )
}

/// One timing setting: a heading, its grid of common choices, and a custom row.
fn duration_section(
    title: &'static str,
    key: &'static str,
    value: Interval,
    unit: IntervalUnit,
    set_value: fn(&mut ViewerApp, Interval, &mut Context<ViewerApp>),
    set_unit: fn(&mut ViewerApp, IntervalUnit, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_0p5()
        .child(section_label(title, cx))
        .child(duration_grid(key, value, set_value, cx))
        .child(custom_row(key, value, unit, set_value, set_unit, cx))
}

/// The presets as a two-row grid: the five seconds, then 5 s with the three
/// sub-second steps.
fn duration_grid(
    key: &'static str,
    current: Interval,
    set_value: fn(&mut ViewerApp, Interval, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let mut grid = v_flex().w_full().gap_1().px_1();
    for row in PRESETS.chunks(4) {
        let mut line = h_flex().w_full().gap_1().items_center();
        for (label, millis) in row {
            line = line.child(duration_chip(
                key,
                label,
                *millis,
                current.as_millis() == *millis,
                set_value,
                cx,
            ));
        }
        grid = grid.child(line);
    }
    grid
}

/// One choice in a duration grid.
fn duration_chip(
    key: &'static str,
    label: &'static str,
    millis: u64,
    selected: bool,
    set_value: fn(&mut ViewerApp, Interval, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
        .id((key, millis as usize))
        .flex_1()
        .py_1p5()
        .rounded(px(6.))
        .text_xs()
        .cursor_pointer()
        .border_1()
        .border_color(if selected { theme.primary } else { theme.border })
        .text_color(if selected {
            theme.primary_foreground
        } else {
            theme.foreground
        })
        .when(selected, |this| this.bg(theme.primary))
        // Hover only tints an unselected chip: on a selected one it would wash
        // out the label against the primary background.
        .when(!selected, |this| this.hover(|this| this.bg(theme.button_hover)))
        .child(div().w_full().text_center().child(label))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            // Selecting is all this does; 开始播放 starts the show.
            set_value(this, Interval::from_millis(millis), cx);
        }))
}

fn section_label(title: &'static str, cx: &Context<ViewerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .mt_2()
        .mb_1()
        .px_1()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}


/// The custom value for one timing setting: a unit switch, the value and two
/// step buttons.
///
/// Switching the unit is only about how the value is read and stepped; the
/// value itself stays in milliseconds, so nothing is lost on the way through.
fn custom_row(
    key: &'static str,
    current: Interval,
    unit: IntervalUnit,
    set_value: fn(&mut ViewerApp, Interval, &mut Context<ViewerApp>),
    set_unit: fn(&mut ViewerApp, IntervalUnit, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let millis = current.as_millis();
    let shown = match unit {
        // In seconds the value is shown with decimals rather than rounded, so
        // 500 ms reads as 0.5 s.
        IntervalUnit::Seconds => format!("{} {}", format_seconds(millis), s::SECONDS),
        IntervalUnit::Milliseconds => format!("{millis} {}", s::MILLISECONDS),
    };

    h_flex()
        .id(gpui::SharedString::from(format!("{key}-custom")))
        .w_full()
        .gap_2()
        .px_2()
        .py_1p5()
        .items_center()
        .child(div().flex_1().text_sm().child(s::SLIDESHOW_CUSTOM))
        .child(unit_button(
            key,
            s::SECONDS,
            IntervalUnit::Seconds,
            unit == IntervalUnit::Seconds,
            set_unit,
            cx,
        ))
        .child(unit_button(
            key,
            s::MILLISECONDS,
            IntervalUnit::Milliseconds,
            unit == IntervalUnit::Milliseconds,
            set_unit,
            cx,
        ))
        .child(step_button(
            key,
            "down",
            IconName::Minus,
            -1,
            unit,
            current,
            set_value,
            cx,
        ))
        .child(
            div()
                .min_w(px(64.))
                .text_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(shown),
        )
        .child(step_button(
            key,
            "up",
            IconName::Plus,
            1,
            unit,
            current,
            set_value,
            cx,
        ))
}

/// One of the two units a custom row can read in.
fn unit_button(
    key: &'static str,
    label: &'static str,
    unit: IntervalUnit,
    selected: bool,
    set_unit: fn(&mut ViewerApp, IntervalUnit, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
        .id((key, unit as usize))
        .px_2()
        .py_0p5()
        .rounded(px(5.))
        .text_xs()
        .cursor_pointer()
        .border_1()
        .border_color(if selected { theme.primary } else { theme.border })
        .text_color(if selected {
            theme.primary_foreground
        } else {
            theme.foreground
        })
        .when(selected, |this| this.bg(theme.primary))
        .when(!selected, |this| this.hover(|this| this.bg(theme.button_hover)))
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            set_unit(this, unit, cx);
        }))
}

/// Step one custom value: whole seconds in seconds mode, 50 ms in milliseconds
/// mode.
#[allow(clippy::too_many_arguments)]
fn step_button(
    key: &'static str,
    id: &'static str,
    icon: IconName,
    direction: i32,
    unit: IntervalUnit,
    current: Interval,
    set_value: fn(&mut ViewerApp, Interval, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let millis = current.as_millis();

    div()
        .id(gpui::SharedString::from(format!("{key}-{id}")))
        .p_1()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .child(Icon::new(icon).size(px(12.)))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            let next = match unit {
                IntervalUnit::Seconds => {
                    let next = millis as i64 + direction as i64 * 1000;
                    Interval::from_millis(next.clamp(1, Interval::MAX_MS as i64) as u64)
                }
                IntervalUnit::Milliseconds => {
                    let next = millis as i64 + direction as i64 * 50;
                    Interval::from_millis(next.clamp(1, Interval::MAX_MS as i64) as u64)
                }
            };
            set_value(this, next, cx);
        }))
}

fn switch_row(
    label: &'static str,
    checked: bool,
    cx: &mut Context<ViewerApp>,
    on_change: fn(&mut ViewerApp, bool, &mut Context<ViewerApp>),
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .justify_between()
        .items_center()
        .child(div().text_sm().child(label))
        .child(
            Switch::new(label)
                .checked(checked)
                .on_change(cx.listener(move |this, value: &bool, _window, cx| {
                    on_change(this, *value, cx);
                })),
        )
        .child(div().hidden().text_color(theme.border))
}

fn effect_row(current: PageTurnFx, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    v_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_1p5()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(s::SLIDESHOW_EFFECT),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_1()
                .children(PageTurnFx::ALL.iter().map(|fx| {
                    let fx = *fx;
                    let selected = current == fx;
                    div()
                        .id(("fx", fx as usize))
                        .px_2p5()
                        .py_1()
                        .rounded(px(6.))
                        .text_xs()
                        .cursor_pointer()
                        .border_1()
                        .border_color(if selected { theme.primary } else { theme.border })
                        .when(selected, |this| {
                            this.bg(theme.primary).text_color(theme.primary_foreground)
                        })
                        .hover(|this| this.bg(theme.button_hover))
                        .child(fx.label())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.set_slideshow_effect(fx, cx);
                        }))
                })),
        )
}


/// `3` for three seconds, `0.5` for half a second.
fn format_seconds(millis: u64) -> String {
    let seconds = millis as f64 / 1000.;
    if (seconds - seconds.round()).abs() < f64::EPSILON {
        format!("{seconds:.0}")
    } else {
        let text = format!("{seconds:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}


