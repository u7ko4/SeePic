//! The Image panel: view mode, filters, interpolation and per-pixel effects.
//!
//! A floating, scrollable panel rather than a dropdown: the list is long enough
//! that a menu would not fit.

use gpui_kit::assets::IconName;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;
use seepic_core::effects::{Adjustments, Channel};
use seepic_core::filters::{FilterMode, Interpolation};
use seepic_core::{BackgroundMode, Transparency, ViewMode};

use crate::app::ViewerApp;


/// The panel's contents, sized for the shared modal frame in `panels`.
pub fn body(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    div()
        .id("image-panel-body")
        .w_full()
        .px_2()
        .pb_2()
        .child(view_modes(view, cx))
        .child(section_title("基础滤镜", cx))
        .child(radio_group(
            FilterMode::ALL.map(|m| (m.label(), view.filter == m)).to_vec(),
            Move::Filter,
            cx,
        ))
        .child(section_title("插值算法", cx))
        .child(radio_group(
            Interpolation::ALL
                .map(|i| (i.label(), view.interpolation == i))
                .to_vec(),
            Move::Interpolation,
            cx,
        ))
        .child(effects_section(view, cx))
        .child(section_title("缩放", cx))
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .child(panel_button("-", Move::ZoomOut, cx))
                .child(
                    div()
                        .flex_1()
                        .text_center()
                        .text_sm()
                        .child(view.zoom_label_text()),
                )
                .child(panel_button("+", Move::ZoomIn, cx))
                .child(panel_button("重置", Move::ZoomReset, cx)),
        )
        .child(toggle(
            "固定缩放比例 (Ctrl+K)",
            view.zoom.zoom_locked,
            Move::ZoomLock,
            cx,
        ))
        .child(toggle(
            "固定滚动位置",
            view.zoom.scroll_locked,
            Move::ScrollLock,
            cx,
        ))
        .child(section_title("透明背景 (Alt+T)", cx))
        .child(radio_group(
            Transparency::ALL
                .iter()
                .map(|mode| (mode.label(), view.transparency == *mode))
                .collect(),
            Move::Transparency,
            cx,
        ))
        .child(background_section(view, cx))
}

fn section_title(title: &'static str, cx: &Context<ViewerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .mt_2()
        .mb_1()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

fn view_modes(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let stretch = view.zoom.stretch_small;
    let separate_cover = view.settings.separate_cover;
    let webtoon = view.view_mode.is_webtoon();
    let two_page = view.view_mode.is_two_page();
    let webtoon_width = view.settings.webtoon_width;

    v_flex()
        .child(section_title("浏览方式", cx))
        .child(radio_group(
            ViewMode::PANEL
                .iter()
                .map(|mode| (mode.label(), view.view_mode == *mode))
                .collect(),
            Move::View,
            cx,
        ))
        // The webtoon width is a parameter of the mode rather than part of it,
        // so it is kept when the mode changes.
        .when(webtoon, |this| {
            this.child(stepper(
                "漫画模式宽度 (%)",
                webtoon_width as i32,
                (10, 400),
                10,
                Move::WebtoonWidth,
                cx,
            ))
        })
        .when(two_page, |this| {
            this.child(toggle(
                "为智能双页使用单独的封面 (Shift+S)",
                separate_cover,
                Move::SeparateCover,
                cx,
            ))
        })
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .justify_between()
                .items_center()
                .child(div().text_sm().child("拉伸小图(Z)"))
                .child(
                    Switch::new("stretch-small").checked(stretch).on_change(
                        cx.listener(|this, value: &bool, window, cx| {
                            if this.zoom.stretch_small != *value {
                                this.execute(Command::ToggleStretchSmall, window, cx);
                            }
                        }),
                    ),
                ),
        )
}

/// Background mode, its colour, an optional picture, and the border colour.
///
/// Only the two transparency modes that show the canvas behind the image make
/// use of any of it, so the controls are hidden otherwise rather than silently
/// doing nothing.
fn background_section(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let shows_background = matches!(
        view.transparency,
        Transparency::IgnoreAlpha | Transparency::BackgroundColor
    );
    if !shows_background {
        return v_flex();
    }

    let mode = view.settings.background_mode;
    let background = view.settings.background_color.clone();
    let border = view.settings.border_color.clone();
    let has_image = view.settings.background_image.is_some();

    v_flex()
        .child(section_title("背景", cx))
        .child(radio_group(
            BackgroundMode::ALL
                .iter()
                .map(|item| (item.label(), mode == *item))
                .collect(),
            Move::Background,
            cx,
        ))
        .when(mode == BackgroundMode::Custom, |this| {
            this.child(swatches("背景色", &background, Move::BackgroundColor, cx))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(match &view.settings.background_image {
                                    Some(path) => path.clone(),
                                    None => "未使用背景图片".to_string(),
                                }),
                        )
                        .child(panel_button("选择图片…", Move::PickBackgroundImage, cx))
                        .when(has_image, |this| {
                            this.child(panel_button("清除", Move::ClearBackgroundImage, cx))
                        }),
                )
        })
        .child(section_title("边框色", cx))
        .child(swatches("边框色", &border, Move::BorderColor, cx))
}

/// A row of preset colours, with the current one ringed.
fn swatches(
    title: &'static str,
    current: &str,
    kind: Move,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let mut row = h_flex().w_full().gap_1p5().px_2().py_1().items_center();

    for (index, (hex, value)) in crate::app::BACKGROUND_SWATCHES.iter().enumerate() {
        let selected = current.eq_ignore_ascii_case(hex);
        row = row.child(
            div()
                .id((title, index))
                .size(px(20.))
                .rounded(px(4.))
                .bg(rgb(*value))
                .border_1()
                .border_color(if selected { theme.primary } else { theme.border })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    match kind {
                        Move::BackgroundColor => this.set_background_color(hex, cx),
                        Move::BorderColor => this.set_border_color(hex, cx),
                        _ => {}
                    }
                })),
        );
    }

    row.child(
        div()
            .ml_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{title} {current}")),
    )
}

fn panel_button(label: &'static str, kind: Move, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
        .id(kind.key())
        .px_2()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(theme.border)
        .text_xs()
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| match kind {
            Move::PickBackgroundImage => this.pick_background_image(cx),
            Move::ClearBackgroundImage => this.clear_background_image(cx),
            _ => {}
        }))
}

/// What a control in the panel changes.
#[derive(Clone, Copy)]
enum Move {
    View,
    ZoomOut,
    ZoomIn,
    ZoomReset,
    ZoomLock,
    ScrollLock,
    Transparency,
    Filter,
    Interpolation,
    Toggle,
    Gray,
    Sepia,
    FlipH,
    FlipV,
    Gamma,
    Brightness,
    Contrast,
    Saturation,
    WebtoonWidth,
    SeparateCover,
    Background,
    BackgroundColor,
    BorderColor,
    PickBackgroundImage,
    ClearBackgroundImage,
}

impl Move {
    /// A name unique to this control, used to build element ids.
    ///
    /// GPUI keys an element's click state by its global id, so two controls
    /// sharing one id also share the pending-press state and neither fires.
    /// The id has to be unique per control, not just per group.
    fn key(self) -> &'static str {
        match self {
            Move::View => "view",
            Move::ZoomOut => "zoom-out",
            Move::ZoomReset => "zoom-reset",
            Move::ZoomIn => "zoom-in",
            Move::ZoomLock => "zoom-lock",
            Move::ScrollLock => "scroll-lock",
            Move::Transparency => "transparency",
            Move::Filter => "filter",
            Move::Interpolation => "interpolation",
            Move::Toggle => "toggle",
            Move::Gray => "gray",
            Move::Sepia => "sepia",
            Move::FlipH => "flip-h",
            Move::FlipV => "flip-v",
            Move::Gamma => "gamma",
            Move::Brightness => "brightness",
            Move::Contrast => "contrast",
            Move::Saturation => "saturation",
            Move::WebtoonWidth => "webtoon-width",
            Move::SeparateCover => "separate-cover",
            Move::Background => "background",
            Move::BackgroundColor => "background-color",
            Move::BorderColor => "border-color",
            Move::PickBackgroundImage => "pick-background",
            Move::ClearBackgroundImage => "clear-background",
        }
    }
}

fn radio_group(
    rows: Vec<(&'static str, bool)>,
    kind: Move,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let mut list = v_flex().gap_0p5();

    for (index, (label, selected)) in rows.into_iter().enumerate() {
        list = list.child(
            h_flex()
                .id((kind.key(), index))
                .w_full()
                .gap_2p5()
                .px_2()
                .py_1p5()
                .rounded(px(6.))
                .cursor_pointer()
                .items_center()
                .hover(|this| this.bg(theme.list_hover))
                .child(
                    div()
                        .size(px(13.))
                        .rounded_full()
                        .border_1()
                        .border_color(if selected { theme.primary } else { theme.border })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |this| {
                            this.child(div().size(px(6.)).rounded_full().bg(theme.primary))
                        }),
                )
                .child(div().flex_1().text_sm().child(label))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    match kind {
                        Move::View => this.set_view_mode(ViewMode::PANEL[index], cx),
                        Move::ZoomOut => this.execute(Command::ZoomOut, window, cx),
                        Move::ZoomIn => this.execute(Command::ZoomIn, window, cx),
                        Move::ZoomReset => this.execute(Command::ZoomReset, window, cx),
                        Move::ZoomLock => this.execute(Command::ToggleZoomLock, window, cx),
                        Move::ScrollLock => this.execute(Command::ToggleScrollLock, window, cx),
                        Move::Transparency => {
                            this.set_transparency(Transparency::ALL[index], cx)
                        }
                        Move::Filter => this.set_filter(FilterMode::ALL[index], cx),
                        Move::Interpolation => {
                            this.set_interpolation(Interpolation::ALL[index], cx)
                        }
                        Move::Background => {
                            this.set_background_mode(BackgroundMode::ALL[index], cx)
                        }
                        // The remaining controls are not radio rows.
                        _ => {}
                    }
                    let _ = window;
                })),
        );
    }
    list
}

fn effects_section(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let a = view.adjustments;

    v_flex()
        .child(
            h_flex()
                .w_full()
                .mt_2()
                .mb_1()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("图片效果"),
                )
                .child(
                    div()
                        .id("effects-reset")
                        .px_2()
                        .py_0p5()
                        .rounded(px(5.))
                        .text_xs()
                        .cursor_pointer()
                        .border_1()
                        .border_color(theme.border)
                        .hover(|this| this.bg(theme.button_hover))
                        .child("重置")
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.reset_adjustments(cx);
                        })),
                ),
        )
        .child(toggle("负片 (Ctrl+I)", a.negative, Move::Toggle, cx))
        .child(stepper("伽玛值补偿 (Ctrl+G)", a.gamma, Adjustments::GAMMA_RANGE, 10, Move::Gamma, cx))
        .child(stepper("亮度", a.brightness, Adjustments::BRIGHTNESS_RANGE, 5, Move::Brightness, cx))
        .child(stepper("对比度", a.contrast, Adjustments::CONTRAST_RANGE, 5, Move::Contrast, cx))
        .child(stepper("饱和度", a.saturation, Adjustments::SATURATION_RANGE, 5, Move::Saturation, cx))
        .child(toggle("垂直翻转 (Ctrl+F)", view.orientation.flip_v, Move::FlipV, cx))
        .child(toggle("水平翻转 (Ctrl+M)", view.orientation.flip_h, Move::FlipH, cx))
        .child(toggle("灰度 (Ctrl+Y)", a.grayscale, Move::Gray, cx))
        .child(toggle("棕褐色 (Ctrl+Shift+S)", a.sepia, Move::Sepia, cx))
        .child(channel_row(a.channel, cx))
}

fn toggle(label: &'static str, checked: bool, kind: Move, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(label)
        .w_full()
        .gap_2p5()
        .px_2()
        .py_1p5()
        .rounded(px(6.))
        .cursor_pointer()
        .items_center()
        .hover(|this| this.bg(theme.list_hover))
        .child(
            div()
                .size(px(13.))
                .rounded(px(3.))
                .border_1()
                .border_color(if checked { theme.primary } else { theme.border })
                .bg(if checked { theme.primary } else { theme.transparent })
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .size(px(10.))
                            .text_color(theme.primary_foreground),
                    )
                }),
        )
        .child(div().flex_1().text_sm().child(label))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            match kind {
                Move::Toggle => {
                    this.adjustments.negative = !this.adjustments.negative;
                    this.refresh_adjustments(cx);
                }
                Move::FlipH => {
                    this.orientation.flip_horizontal();
                    this.refresh_transform(cx);
                }
                Move::FlipV => {
                    this.orientation.flip_vertical();
                    this.refresh_transform(cx);
                }
                Move::Gray => {
                    this.adjustments.grayscale = !this.adjustments.grayscale;
                    this.refresh_adjustments(cx);
                }
                Move::Sepia => {
                    this.adjustments.sepia = !this.adjustments.sepia;
                    this.refresh_adjustments(cx);
                }
                Move::SeparateCover => {
                    this.settings.separate_cover = !this.settings.separate_cover;
                    let _ = this.settings.save();
                    this.recompute_spreads();
                    cx.notify();
                }
                _ => {}
            }
            let _ = window;
        }))
}

fn stepper(
    label: &'static str,
    value: i32,
    range: (i32, i32),
    step: i32,
    kind: Move,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(label)
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .child(div().flex_1().text_sm().child(label))
        .child(step_button(kind.key(), 0, IconName::Minus, -step, range, kind, cx))
        .child(
            div()
                .min_w(px(52.))
                .text_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("{value}")),
        )
        .child(step_button(kind.key(), 1, IconName::Plus, step, range, kind, cx))
}

fn step_button(
    group: &'static str,
    side: usize,
    icon: IconName,
    step: i32,
    range: (i32, i32),
    kind: Move,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
        .id((group, side))
        .p_1()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .child(Icon::new(icon).size(px(12.)))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            let field = match kind {
                Move::Gamma => &mut this.adjustments.gamma,
                Move::Brightness => &mut this.adjustments.brightness,
                Move::Contrast => &mut this.adjustments.contrast,
                Move::Saturation => &mut this.adjustments.saturation,
                Move::WebtoonWidth => {
                    let value = (this.settings.webtoon_width as i32 + step).clamp(range.0, range.1);
                    this.settings.webtoon_width = value as u32;
                    let _ = this.settings.save();
                    cx.notify();
                    return;
                }
                _ => return,
            };
            *field = (*field + step).clamp(range.0, range.1);
            this.refresh_adjustments(cx);
        }))
}

fn channel_row(current: Channel, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let mut row = h_flex()
        .w_full()
        .px_2()
        .py_1p5()
        .gap_3()
        .items_center()
        .child(div().text_sm().child("选择 RGB 通道"));

    for channel in Channel::ALL {
        let selected = current == channel;
        row = row.child(
            h_flex()
                .id(("channel", channel as usize))
                .gap_1()
                .cursor_pointer()
                .items_center()
                .child(
                    div()
                        .size(px(12.))
                        .rounded_full()
                        .border_1()
                        .border_color(if selected { theme.primary } else { theme.border })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |this| {
                            this.child(div().size(px(5.)).rounded_full().bg(theme.primary))
                        }),
                )
                .child(div().text_xs().child(channel.label()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.adjustments.channel = channel;
                    this.refresh_adjustments(cx);
                })),
        );
    }
    row
}
