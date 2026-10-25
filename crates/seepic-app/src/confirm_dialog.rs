//! Confirmation and rename dialogs for file operations.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::ViewerApp;
use crate::strings as s;

/// What the dialog will do when confirmed.
#[derive(Debug, Clone)]
pub enum DialogKind {
    DeleteFile { path: PathBuf },
    /// Deleting one page of an archive means deleting the whole file.
    DeleteArchive { path: PathBuf },
    Rename { path: PathBuf },
    /// Purely informational: nothing to confirm.
    About,
}

/// An open dialog.
#[derive(Debug, Clone)]
pub struct DialogState {
    pub kind: DialogKind,
    pub title: String,
    pub message: String,
    /// Editable text, only for rename.
    pub input: String,
    pub error: Option<String>,
}

impl DialogState {
    pub fn is_rename(&self) -> bool {
        matches!(self.kind, DialogKind::Rename { .. })
    }

    /// Whether the dialog only has something to say.
    pub fn is_about(&self) -> bool {
        matches!(self.kind, DialogKind::About)
    }

    /// The About box: what this is, and where its files live.
    pub fn about(version: &str) -> Self {
        Self {
            kind: DialogKind::About,
            title: "关于".to_string(),
            message: format!(
                "SeePic {version}\n\
                 一个用 Rust + GPUI 编写的桌面图片查看器与漫画阅读器。\n\
                 \n\
                 支持 JPEG / PNG / GIF / WebP / BMP / TIFF / TGA / PNM / DDS / HDR / EXR / QOI，\n\
                 以及 ZIP / CBZ、7Z / CB7、RAR / CBR、TAR / CBT 压缩包直接浏览。\n\
                 \n\
                 配置文件：{}\n\
                 信息缓存：{}",
                crate::app::config_path().display(),
                crate::app::meta_cache_path().display()
            ),
            input: String::new(),
            error: None,
        }
    }

    pub fn delete_file(path: PathBuf, name: &str) -> Self {
        Self {
            kind: DialogKind::DeleteFile { path },
            title: "删除文件".to_string(),
            message: format!("确定要把「{name}」移到回收站吗？"),
            input: String::new(),
            error: None,
        }
    }

    pub fn delete_archive(path: PathBuf, name: &str) -> Self {
        Self {
            kind: DialogKind::DeleteArchive { path },
            title: "删除压缩包".to_string(),
            message: format!("当前图像是压缩包的一部分。要删除整个「{name}」吗？"),
            input: String::new(),
            error: None,
        }
    }

    pub fn rename(path: PathBuf, current: &str) -> Self {
        Self {
            kind: DialogKind::Rename { path },
            title: "重命名".to_string(),
            message: "输入新的文件名。".to_string(),
            input: current.to_string(),
            error: None,
        }
    }
}

pub fn render(view: &ViewerApp, dialog: &DialogState, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let danger = !dialog.is_rename() && !dialog.is_about();

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
        // Clicking outside cancels; the dialog itself stops the click so a
        // press on a button is not also a dismissal.
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _window, cx| {
            this.cancel_dialog(cx);
        }))
        .child(
            v_flex()
                .w(px(440.))
                .on_mouse_down(MouseButton::Left, cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }))
                .rounded(px(12.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .track_focus(view.dialog_focus())
                .on_key_down(cx.listener(ViewerApp::on_dialog_key))
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
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(if dialog.is_about() {
                                        IconName::Info
                                    } else if danger {
                                        IconName::TriangleAlert
                                    } else {
                                        IconName::FilePen
                                    })
                                    .size(px(16.))
                                    .text_color(if danger { theme.danger } else { theme.foreground }),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(dialog.title.clone()),
                                ),
                        )
                        .child(
                            div()
                                .id("dialog-close")
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(14.)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.cancel_dialog(cx);
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .px_4()
                        .py_3()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(dialog.message.clone()),
                        )
                        .when(dialog.is_rename(), |this| {
                            this.child(
                                div()
                                    .id("dialog-field")
                                    .w_full()
                                    .px_3()
                                    .py_2()
                                    .rounded(px(7.))
                                    .bg(theme.input)
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_sm()
                                    .min_h(px(36.))
                                    .child(dialog.input.clone()),
                            )
                        })
                        .when_some(dialog.error.clone(), |this, error| {
                            this.child(div().text_xs().text_color(theme.danger).child(error))
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_4()
                        .py_2p5()
                        .gap_2()
                        .justify_end()
                        .border_t_1()
                        .border_color(theme.border)
                        .when(!dialog.is_about(), |this| {
                            this.child(
                            div()
                                .id("dialog-cancel")
                                .px_3()
                                .py_1()
                                .rounded(px(6.))
                                .text_sm()
                                .cursor_pointer()
                                .border_1()
                                .border_color(theme.border)
                                .hover(|this| this.bg(theme.button_hover))
                                .child(s::CANCEL)
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.cancel_dialog(cx);
                                })),
                            )
                        })
                        .child(
                            div()
                                .id("dialog-ok")
                                .px_4()
                                .py_1()
                                .rounded(px(6.))
                                .text_sm()
                                .cursor_pointer()
                                .bg(if danger { theme.danger } else { theme.primary })
                                .text_color(if danger {
                                    theme.danger_foreground
                                } else {
                                    theme.primary_foreground
                                })
                                .hover(|this| this.opacity(0.9))
                                .child(if dialog.is_about() {
                                    s::CLOSE
                                } else {
                                    s::CONFIRM
                                })
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.confirm_dialog(cx);
                                })),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: the parent imports gpui's `test` attribute
    // macro, which would shadow the built-in one.
    use super::DialogState;

    /// The About box is the one place the application states its own name, so a
    /// rename that misses it would only show up to a user who opened it.
    #[test]
    fn the_about_box_names_the_application() {
        let about = DialogState::about("9.9.9");
        assert!(
            about.message.contains("SeePic 9.9.9"),
            "the About box must carry the product name and version: {}",
            about.message
        );
    }

    /// The About box tells the user where their files are, so the path it
    /// prints has to be the one the app actually uses.
    #[test]
    fn the_about_box_reports_the_real_config_path() {
        let about = DialogState::about("1.0.0");
        let config = crate::app::config_path();
        assert!(
            about.message.contains(&config.display().to_string()),
            "About should print {}: {}",
            config.display(),
            about.message
        );
        assert!(
            config.to_string_lossy().contains("SeePic"),
            "the config must live under the product's own directory: {}",
            config.display()
        );
    }
}
