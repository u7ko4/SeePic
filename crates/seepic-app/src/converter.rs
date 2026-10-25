//! The image converter dialog (Ctrl+T).
//!
//! Batch conversion is the one feature that writes files, so the dialog shows
//! exactly where the output will land — the resolved path for the current image
//! — rather than making the user infer it from a folder and a prefix.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::convert::{
    ConvertOptions, OutputFormat, Overwrite, ResizeMode,
};
use seepic_core::orientation::Rotation;

use crate::app::ViewerApp;

/// The converter's state while its dialog is open.
pub struct ConvertDialog {
    pub options: ConvertOptions,
    /// Set once a batch starts.
    pub job: Option<ConvertProgress>,
}

/// A running batch.
pub struct ConvertProgress {
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    /// Read by the batch between files, so cancelling stops after the current
    /// one rather than mid-write.
    pub cancelled: Arc<AtomicBool>,
}

impl ConvertProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.
        } else {
            self.done as f32 / self.total as f32
        }
    }
}

impl ConvertDialog {
    pub fn new() -> Self {
        Self {
            options: ConvertOptions::default(),
            job: None,
        }
    }

    /// True while a batch is running, so the options lock and a cancel button
    /// appears.
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    pub fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

/// What a control in the dialog changes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Resize,
    Format,
    Overwrite,
    Rotation,
    Quality,
    Value,
    BoxHeight,
    SameFolder,
    DeleteSource,
}

/// The value a numeric control is editing, for the mode that is active.
pub fn value_of(options: &ConvertOptions) -> i32 {
    options.resize.value() as i32
}

/// Whether the active mode uses the numeric control at all.
fn uses_value(mode: ResizeMode) -> bool {
    !matches!(mode, ResizeMode::Original)
}

pub fn render(view: &ViewerApp, dialog: &ConvertDialog, cx: &mut Context<ViewerApp>) -> AnyElement {
    let theme = cx.theme().clone();
    let options = &dialog.options;
    let running = dialog.running();
    let target = view.convert_preview(options);

    let mut body = v_flex()
        .id("converter-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_4()
        .py_3()
        .gap_1()
        .child(section("缩放模式", cx))
        .child(radio_list(
            ResizeMode::MODES
                .iter()
                .map(|mode| {
                    (
                        mode.mode_label(),
                        std::mem::discriminant(mode) == std::mem::discriminant(&options.resize),
                    )
                })
                .collect(),
            Field::Resize,
            running,
            cx,
        ));

    if uses_value(options.resize) {
        body = body.child(stepper(
            "数值",
            value_of(options),
            1,
            20_000,
            10,
            Field::Value,
            running,
            cx,
        ));
    }
    if let ResizeMode::FitBox { height, .. } = options.resize {
        body = body.child(stepper(
            "框高",
            height as i32,
            1,
            20_000,
            10,
            Field::BoxHeight,
            running,
            cx,
        ));
    }

    body = body
        .child(section("输出格式", cx))
        .child(radio_list(
            OutputFormat::ALL
                .iter()
                .map(|format| (format.label(), *format == options.format))
                .collect(),
            Field::Format,
            running,
            cx,
        ));

    if options.format.uses_quality() {
        body = body.child(stepper(
            "质量",
            options.quality as i32,
            1,
            100,
            5,
            Field::Quality,
            running,
            cx,
        ));
    }

    body = body
        .child(section("旋转", cx))
        .child(radio_list(
            [
                (Rotation::None, "不旋转"),
                (Rotation::Cw90, "顺时针 90°"),
                (Rotation::Cw180, "180°"),
                (Rotation::Cw270, "逆时针 90°"),
            ]
            .iter()
            .map(|(rotation, label)| (*label, *rotation == options.rotation))
            .collect(),
            Field::Rotation,
            running,
            cx,
        ))
        .child(section("输出位置", cx))
        .child(toggle_row("与源文件同一目录", options.same_folder, Field::SameFolder, running, cx))
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(match &options.output_dir {
                            Some(dir) if !options.same_folder => dir.display().to_string(),
                            _ => "（同目录）".to_string(),
                        }),
                )
                .child(button("选择目录…", running, cx, |this, cx| {
                    this.pick_convert_folder(cx)
                })),
        )
        .child(section("文件名", cx))
        .child(info_row("前缀", if options.prefix.is_empty() { "（无）".to_string() } else { options.prefix.clone() }, cx))
        .child(
            div()
                .px_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("打开对话框后可直接输入前缀；退格删除。"),
        )
        .child(section("重名时", cx))
        .child(radio_list(
            Overwrite::ALL
                .iter()
                .map(|mode| (mode.label(), *mode == options.overwrite))
                .collect(),
            Field::Overwrite,
            running,
            cx,
        ))
        .child(toggle_row(
            "转换后删除源文件",
            options.delete_source,
            Field::DeleteSource,
            running,
            cx,
        ))
        .child(section("输出预览", cx))
        .child(info_row(
            "当前图像将写入",
            target.unwrap_or_else(|| "（无法确定）".to_string()),
            cx,
        ));

    if let Some(job) = &dialog.job {
        body = body.child(section("进度", cx)).child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme.input)
                        .child(
                            div()
                                .h_full()
                                .w(relative(dialog_progress_fraction(job)))
                                .rounded_full()
                                .bg(theme.primary),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} / {}（失败 {}）",
                            job.done, job.total, job.failed
                        )),
                ),
        );
    }

    let footer = if running {
        h_flex()
            .w_full()
            .px_4()
            .py_2p5()
            .gap_2()
            .justify_end()
            .border_t_1()
            .border_color(theme.border)
            .child(button("取消转换", false, cx, |this, cx| {
                this.cancel_conversion(cx)
            }))
    } else {
        h_flex()
            .w_full()
            .px_4()
            .py_2p5()
            .gap_2()
            .justify_end()
            .border_t_1()
            .border_color(theme.border)
            .child(button("关闭", false, cx, |this, cx| this.close_converter(cx)))
            .child(button("开始转换", false, cx, |this, cx| {
                this.start_conversion(cx)
            }))
    };

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
        .occlude()
        // Clicking outside closes the converter; a running batch is cancelled
        // by closing, which is what the user is asking for.
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _window, cx| {
            this.close_converter(cx);
        }))
        .child(
            v_flex()
                .w(px(520.))
                .on_mouse_down(MouseButton::Left, cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }))
                .max_h(px(720.))
                .rounded(px(12.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    h_flex()
                        .w_full()
                        .px_4()
                        .py_3()
                        .justify_between()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("图像转换器"),
                        )
                        .child(
                            div()
                                .id("converter-close")
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(14.)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.close_converter(cx);
                                })),
                        ),
                )
                .child(body)
                .child(footer),
        )
        .into_any_element()
}

/// The fraction as a `Relative`, for the progress bar's width.
fn dialog_progress_fraction(job: &ConvertProgress) -> f32 {
    job.fraction().clamp(0., 1.)
}

fn section(title: &'static str, cx: &Context<ViewerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .mt_2()
        .mb_0p5()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

fn info_row(label: &'static str, value: String, cx: &Context<ViewerApp>) -> impl IntoElement {
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_3()
        .items_center()
        .child(
            div()
                .w(px(96.))
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().flex_1().text_sm().truncate().child(value))
}

fn radio_list(
    rows: Vec<(&'static str, bool)>,
    field: Field,
    disabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let mut list = v_flex().w_full().gap_0p5();

    for (index, (label, selected)) in rows.into_iter().enumerate() {
        list = list.child(
            h_flex()
                .id((field_key(field), index))
                .w_full()
                .gap_2p5()
                .px_2()
                .py_1p5()
                .rounded(px(6.))
                .items_center()
                .when(!disabled, |this| {
                    this.cursor_pointer().hover(|this| this.bg(theme.list_hover))
                })
                .opacity(if disabled { 0.6 } else { 1. })
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
                .when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.set_convert_radio(field, index, cx)
                    }))
                }),
        );
    }
    list
}

fn toggle_row(
    label: &'static str,
    checked: bool,
    field: Field,
    disabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(field_key(field))
        .w_full()
        .px_2()
        .py_1p5()
        .rounded(px(6.))
        .justify_between()
        .items_center()
        .when(!disabled, |this| {
            this.cursor_pointer().hover(|this| this.bg(theme.list_hover))
        })
        .opacity(if disabled { 0.6 } else { 1. })
        .child(div().text_sm().child(label))
        .child(
            div()
                .w(px(34.))
                .h(px(18.))
                .rounded_full()
                .bg(if checked { theme.primary } else { theme.input })
                .p(px(2.))
                .flex()
                .when(checked, |this| this.justify_end())
                .child(div().size(px(14.)).rounded_full().bg(theme.background)),
        )
        .when(!disabled, |this| {
            this.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.toggle_convert_field(field, cx)
            }))
        })
}

#[allow(clippy::too_many_arguments)]
fn stepper(
    label: &'static str,
    value: i32,
    min: i32,
    max: i32,
    step: i32,
    field: Field,
    disabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .opacity(if disabled { 0.6 } else { 1. })
        .child(div().flex_1().text_sm().child(label))
        .child(step_button(field, 0, IconName::Minus, -step, min, max, disabled, cx))
        .child(
            div()
                .min_w(px(56.))
                .text_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(value.to_string()),
        )
        .child(step_button(field, 1, IconName::Plus, step, min, max, disabled, cx))
}

#[allow(clippy::too_many_arguments)]
fn step_button(
    field: Field,
    side: usize,
    icon: IconName,
    step: i32,
    min: i32,
    max: i32,
    disabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
        .id((field_key(field), side))
        .p_1()
        .rounded(px(5.))
        .when(!disabled, |this| {
            this.cursor_pointer().hover(|this| this.bg(theme.button_hover))
        })
        .child(Icon::new(icon).size(px(12.)))
        .when(!disabled, |this| {
            this.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.nudge_convert_value(field, step, min, max, cx)
            }))
        })
}

fn button(
    label: &'static str,
    disabled: bool,
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
        .opacity(if disabled { 0.5 } else { 1. })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.button_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
        })
        .child(label)
}

/// A stable id prefix per control, so no two rows share click state.
fn field_key(field: Field) -> &'static str {
    match field {
        Field::Resize => "convert-resize",
        Field::Format => "convert-format",
        Field::Overwrite => "convert-overwrite",
        Field::Rotation => "convert-rotation",
        Field::Quality => "convert-quality",
        Field::Value => "convert-value",
        Field::BoxHeight => "convert-box-height",
        Field::SameFolder => "convert-same-folder",
        Field::DeleteSource => "convert-delete-source",
    }
}
