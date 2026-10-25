//! The "ask what to do at the end" prompt.
//!
//! The boundary setting 询问 puts up a small panel instead of guessing. Each choice carries the path it would take you to,
//! so the answer is obvious before committing to it — and the last two choices
//! turn the question off for good, which is how a user who is tired of being
//! asked gets rid of it.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::*;
use seepic_core::BoundaryAction;

use crate::app::ViewerApp;

/// What the prompt is asking about.
pub struct BoundaryPrompt {
    /// True when the user was paging forward (at the last image).
    pub forward: bool,
    /// The container being browsed, shown so the target reads in context.
    pub container: String,
    /// The image the "go to the other end" choice would land on.
    pub wrap_target: String,
    /// The neighbouring container, when there is one to move to.
    pub sibling: Option<String>,
}

impl BoundaryPrompt {
    /// The question, phrased for the direction the user was going.
    pub fn title(&self) -> &'static str {
        if self.forward {
            "已到最后一张"
        } else {
            "已到第一张"
        }
    }
}

pub fn render(prompt: &BoundaryPrompt, cx: &mut Context<ViewerApp>) -> AnyElement {
    let theme = cx.theme().clone();
    let wrap_label = if prompt.forward {
        "转到第一个文件"
    } else {
        "转到最后一个文件"
    };

    let mut choices = v_flex().w_full().gap_1().child(choice(
        "boundary-wrap",
        wrap_label,
        &prompt.wrap_target,
        Action::Apply(BoundaryAction::Wrap),
        cx,
    ));

    if let Some(sibling) = &prompt.sibling {
        choices = choices.child(choice(
            "boundary-sibling",
            "转到下一个文件夹或文件",
            sibling,
            Action::Apply(BoundaryAction::NextContainer),
            cx,
        ));
    }

    choices = choices
        .child(choice(
            "boundary-nothing",
            "无操作",
            "留在当前图像",
            Action::Apply(BoundaryAction::DoNothing),
            cx,
        ))
        .child(separator(&theme))
        // Answered once and remembered, so the user can stop being asked.
        .child(choice(
            "boundary-always-wrap",
            "以后都转到另一端（不再询问）",
            &prompt.wrap_target,
            Action::Remember(BoundaryAction::Wrap),
            cx,
        ));
    if prompt.sibling.is_some() {
        choices = choices.child(choice(
            "boundary-always-sibling",
            "以后都转到下一个文件夹或文件（不再询问）",
            prompt.sibling.as_deref().unwrap_or_default(),
            Action::Remember(BoundaryAction::NextContainer),
            cx,
        ));
    }

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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _window, cx| {
            this.dismiss_boundary_prompt(cx);
        }))
        .child(
            v_flex()
                .w(px(460.))
                .rounded(px(12.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .overflow_hidden()
                // Clicks inside must not reach the backdrop, which would
                // dismiss the prompt before a choice registered.
                .on_mouse_down(MouseButton::Left, cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }))
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
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(prompt.title()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(prompt.container.clone()),
                                ),
                        )
                        .child(
                            div()
                                .id("boundary-close")
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(14.)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.dismiss_boundary_prompt(cx);
                                })),
                        ),
                )
                .child(div().p_2().child(choices)),
        )
        .into_any_element()
}

/// What a choice does.
#[derive(Clone, Copy)]
enum Action {
    /// Do it this once.
    Apply(BoundaryAction),
    /// Do it and stop asking.
    Remember(BoundaryAction),
}

fn choice(
    id: &'static str,
    label: &str,
    target: &str,
    action: Action,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
        .id(id)
        .w_full()
        .px_3()
        .py_2()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover))
        .child(
            v_flex()
                .gap_0p5()
                .child(div().text_sm().child(label.to_string()))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(target.to_string()),
                ),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| match action {
            Action::Apply(action) => this.resolve_boundary_prompt(action, false, cx),
            Action::Remember(action) => this.resolve_boundary_prompt(action, true, cx),
        }))
}

fn separator(theme: &Theme) -> impl IntoElement {
    div().w_full().my_1().h(px(1.)).bg(theme.border)
}
