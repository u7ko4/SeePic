//! The canvas context menu.
//!
//! Rows reuse the command table, so the labels, the shortcut hints and the
//! behaviour all stay in step with the menus and the keyboard.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;
use seepic_core::Point as IvPoint;

use crate::app::ViewerApp;
use crate::strings as s;

pub const MENU_WIDTH: f32 = 246.;
/// Roughly how tall the menu ends up, used to keep it on screen.
pub const MENU_HEIGHT: f32 = 700.;

/// A menu entry: a command, or a separator when `command` is `None`.
struct Item {
    label: &'static str,
    command: Option<Command>,
    disabled: bool,
}

fn separator() -> Item {
    Item {
        label: "",
        command: None,
        disabled: true,
    }
}

fn entries(view: &ViewerApp) -> Vec<Item> {
    // Opening something is the one pair of rows that makes sense on an empty
    // viewer; everything below it acts on the image that is open.
    let ready = view.has_images();
    let row = |label: &'static str, command: Command, needs_image: bool| Item {
        label,
        command: Some(command),
        disabled: needs_image && !ready,
    };

    vec![
        row(Command::OpenFile.label(), Command::OpenFile, false),
        row(Command::OpenFolder.label(), Command::OpenFolder, false),
        separator(),
        row("复制文件路径", Command::CopyPath, true),
        row("复制 EXIF 信息", Command::CopyExif, true),
        row("复制图片到剪贴板", Command::CopyImage, true),
        row("复制图像文件", Command::CopyFile, true),
        row("剪切图像文件", Command::CutFile, true),
        row("粘贴", Command::Paste, false),
        row(s::REVEAL_IN_FOLDER, Command::RevealInFolder, true),
        separator(),
        row("顺时针旋转 90°", Command::RotateCw, true),
        row("逆时针旋转 90°", Command::RotateCcw, true),
        row("水平翻转", Command::FlipHorizontal, true),
        row("垂直翻转", Command::FlipVertical, true),
        row("重置旋转与翻转", Command::ResetTransform, true),
        separator(),
        row("开始/暂停幻灯片", Command::SlideshowToggle, true),
        row("文件/EXIF 信息", Command::ToggleInfo, true),
        separator(),
        row("关闭图像", Command::CloseImage, true),
        row("删除文件", Command::DeleteFile, true),
    ]
}

/// Where the panel ends up, in content-area coordinates.
pub fn menu_origin(
    anchor: IvPoint,
    chrome: f32,
    window_width: f32,
    window_height: f32,
) -> (f32, f32) {
    let left = anchor.x.clamp(8., (window_width - MENU_WIDTH - 8.).max(8.));
    let top = (anchor.y - chrome)
        .clamp(4., (window_height - chrome - MENU_HEIGHT - 8.).max(4.));
    (left, top)
}

/// Render the menu, anchored at `anchor` (window coordinates).
///
/// `chrome` is the height of the title and menu bars, because the menu is drawn
/// inside the content area but the anchor arrives in window coordinates.
pub fn render(
    view: &ViewerApp,
    anchor: IvPoint,
    chrome: f32,
    window_width: f32,
    window_height: f32,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    let (left, top) = menu_origin(anchor, chrome, window_width, window_height);

    let mut panel = v_flex()
        .absolute()
        .id("context-menu")
        .top(px(top))
        .left(px(left))
        .w(px(MENU_WIDTH))
        .py_1()
        .rounded(px(9.))
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(theme.border)
        .shadow_2xl()
        .text_sm();

    for item in entries(view) {
        panel = match item.command {
            Some(command) => panel.child(menu_row(item.label, command, item.disabled, cx)),
            None => panel.child(separator_or_pending(item.label, item.disabled, cx)),
        };
    }

    // The panel itself is given an id so it owns a hitbox; the canvas closes
    // the menu on a left click elsewhere.
    panel
}

fn menu_row(
    label: &'static str,
    command: Command,
    disabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let shortcut = command.binding_label().unwrap_or_default();

    div()
        .id(label)
        .px_3()
        .py_1p5()
        .text_color(if disabled {
            theme.muted_foreground.opacity(0.5)
        } else {
            theme.popover_foreground
        })
        .when(!disabled, |this| {
            let hover_theme = theme.clone();
            this.cursor_pointer()
                .hover(move |this| this.bg(hover_theme.list_hover))
        })
        .child(
            h_flex()
                .w_full()
                .gap_6()
                .justify_between()
                .items_center()
                .child(label)
                .when(!shortcut.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(shortcut),
                    )
                }),
        )
        .when(!disabled, |this| {
            this.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.context_menu = None;
                this.execute(command, window, cx);
            }))
        })
}

fn separator_or_pending(
    label: &'static str,
    disabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    if label.is_empty() {
        return div()
            .h(px(1.))
            .mx_2()
            .my_1()
            .bg(theme.border)
            .into_any_element();
    }

    div()
        .id(label)
        .px_3()
        .py_1p5()
        .text_color(theme.muted_foreground.opacity(if disabled { 0.5 } else { 1. }))
        .child(label)
        .into_any_element()
}
