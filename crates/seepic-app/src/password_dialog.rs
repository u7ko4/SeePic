//! The password prompt for encrypted archives.
//!
//! Shown when a read fails because the archive is encrypted. The entered
//! password is handed to the container and the read is retried.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::ViewerApp;
use crate::strings as s;

/// State of an open prompt.
#[derive(Debug, Clone, Default)]
pub struct PasswordPrompt {
    /// Archive being unlocked, for the message.
    pub archive: String,
    pub input: String,
    /// True once a previous attempt was rejected.
    pub wrong: bool,
}

pub fn render(view: &ViewerApp, prompt: &PasswordPrompt, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    // Never echo the password in clear text.
    let masked = "•".repeat(prompt.input.chars().count());

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
        // Clicking outside cancels, as in the other dialogs.
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _window, cx| {
            this.cancel_password(cx);
        }))
        .child(
            v_flex()
                .w(px(420.))
                .on_mouse_down(MouseButton::Left, cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }))
                .rounded(px(12.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .track_focus(view.password_focus())
                .on_key_down(cx.listener(ViewerApp::on_password_key))
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
                                .child(Icon::new(IconName::Lock).size(px(16.)))
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(s::PASSWORD_TITLE),
                                ),
                        )
                        .child(
                            div()
                                .id("password-close")
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(14.)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.cancel_password(cx);
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
                                .child(if prompt.archive.is_empty() {
                                    s::PASSWORD_HINT.to_string()
                                } else {
                                    format!("「{}」已加密，请输入密码。", prompt.archive)
                                }),
                        )
                        .child(
                            // A plain box rather than a full text-input widget:
                            // the field only needs to accept a password and
                            // confirm, and this keeps the focus handling simple.
                            div()
                                .id("password-field")
                                .w_full()
                                .px_3()
                                .py_2()
                                .rounded(px(7.))
                                .bg(theme.input)
                                .border_1()
                                .border_color(if prompt.wrong { theme.danger } else { theme.border })
                                .text_sm()
                                .min_h(px(36.))
                                .when(prompt.input.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .text_color(theme.muted_foreground)
                                            .child("输入密码后按 Enter"),
                                    )
                                })
                                .when(!prompt.input.is_empty(), |this| this.child(masked)),
                        )
                        .when(prompt.wrong, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.danger)
                                    .child(s::PASSWORD_WRONG),
                            )
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
                        .child(
                            div()
                                .id("password-cancel")
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
                                    this.cancel_password(cx);
                                })),
                        )
                        .child(
                            div()
                                .id("password-ok")
                                .px_4()
                                .py_1()
                                .rounded(px(6.))
                                .text_sm()
                                .cursor_pointer()
                                .bg(theme.primary)
                                .text_color(theme.primary_foreground)
                                .hover(|this| this.opacity(0.9))
                                .child(s::CONFIRM)
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.submit_password(cx);
                                })),
                        ),
                ),
        )
}
