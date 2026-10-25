//! The file/EXIF information window.
//!
//! It is a separate OS window with the same custom title bar as the main
//! window. The EXIF block is rendered as Markdown through a selectable,
//! scrollable `TextView`: keys and values get a table, the generation
//! parameters hidden inside a `UserComment` are pulled out into their own
//! table, and the prompt keeps its original line breaks in a code block.

use gpui_kit::base::TextSelection;
use gpui_kit::ResizeEdge;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::metadata::{ImageInfo, human_size};

use crate::strings as s;

pub struct InfoWindow {
    info: ImageInfo,
    text_view: Entity<TextViewState>,
    /// The scroll position of the EXIF body.
    ///
    /// The scroll lives in this separate handle rather than inside the text
    /// view, so dragging the scrollbar cannot be mistaken for a text-selection
    /// drag starting in the text.
    scroll: ScrollHandle,
    /// Markdown currently in the view, so it is only reset when the page changes.
    shown_markdown: String,
    /// Size from the previous frame; a resize clears any text selection.
    ///
    /// Windows' resize loop takes the mouse release, so the text view can be
    /// left thinking the left button is still down. When the pointer comes
    /// back it then extends a selection from the resize drag. Clearing on every
    /// size change resets that state before the stale drag can do anything.
    last_size: Option<(f32, f32)>,
    /// A resize or window move just ran; the next pointer move with the button
    /// physically up clears whatever selection the stale drag left behind.
    window_manipulated: bool,
    /// A guard is already waiting for the window manipulation to finish.
    selection_guard: bool,
    /// This window's id, for clearing the window-level selection snapshot.
    window_id: WindowId,
    focus: FocusHandle,
}

impl InfoWindow {
    pub fn new(info: ImageInfo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let markdown = exif_markdown(&info);
        let text_view = cx.new(|cx| TextViewState::markdown(&markdown, cx));
        Self {
            info,
            text_view,
            scroll: ScrollHandle::new(),
            shown_markdown: markdown,
            last_size: None,
            window_manipulated: false,
            selection_guard: false,
            window_id: window.window_handle().window_id(),
            focus: cx.focus_handle(),
        }
    }

    /// Clear a selection produced by the resize/move loop once it is over.
    ///
    /// The platform's modal move/resize loop swallows the mouse release, so the
    /// text view can be left mid-drag and select a line as soon as the pointer
    /// comes back. Poll the physical button from a background timer; when it is
    /// up and the pointer had a moment to settle, reset the selection.
    fn arm_selection_guard(&mut self, cx: &mut Context<Self>) {
        if self.selection_guard {
            return;
        }
        self.selection_guard = true;
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_this, cx| {
            for _ in 0..40 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                if !host_left_button_down() {
                    break;
                }
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            weak.update(cx, |this, cx| {
                this.selection_guard = false;
                this.window_manipulated = false;
                this.text_view.update(cx, |state, cx| {
                    state.clear_selection(cx);
                });
                // The blue highlight is the window-level selection snapshot,
                // not the view's local state; both have to go.
                TextSelection::clear_for_window(this.window_id, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Point the window at a new page and refresh its text.
    pub fn set_info(&mut self, info: ImageInfo, cx: &mut Context<Self>) {
        self.info = info;
        let markdown = exif_markdown(&self.info);
        if markdown != self.shown_markdown {
            self.text_view.update(cx, |state, cx| {
                state.set_text(&markdown, cx);
            });
            self.shown_markdown = markdown;
            // A new page starts at the top of its own block.
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }
}

impl Render for InfoWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = (
            f32::from(window.bounds().size.width),
            f32::from(window.bounds().size.height),
        );
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            self.window_manipulated = true;
            self.text_view.update(cx, |state, cx| {
                state.clear_selection(cx);
            });
            // The selection highlight is a window-level snapshot; clearing only
            // the view's local state does not remove it.
            TextSelection::clear(window, cx);
            self.arm_selection_guard(cx);
        }
        let theme = cx.theme().clone();
        let info = &self.info;
        let has_exif = !self.shown_markdown.is_empty();
        let title_width = (f32::from(window.bounds().size.width) - 160.).max(120.);

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_this, event: &KeyDownEvent, window, _cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            // Windows' resize/move loop takes the mouse release, which can
            // leave the text view believing a drag is still in progress. On the
            // first move after the window was manipulated, and with the button
            // physically up, drop whatever selection the stale drag produced.
            .on_mouse_move(cx.listener(|this, _event: &MouseMoveEvent, _window, cx| {
                if this.window_manipulated && !host_left_button_down() {
                    this.window_manipulated = false;
                    this.text_view.update(cx, |state, cx| {
                        state.clear_selection(cx);
                    });
                }
            }))
            // Same custom title bar as the main window, including the platform
            // window buttons on the right.
            .child(
                TitleBar::new().child(
                    div()
                        .occlude()
                        .w(px(title_width))
                        .min_w_0()
                        .h_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .overflow_hidden()
                        // Makes the bar a caption area, so Windows moves the
                        // window without a GPUI callback (the same mechanism
                        // the main window's bar uses).
                        .window_control_area(WindowControlArea::Drag)
                        // Dragging the bar moves the window. The component's
                        // own handler sits behind the occluding content, so the
                        // press is forwarded here as well.
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.window_manipulated = true;
                                this.arm_selection_guard(cx);
                                window.start_window_move();
                            }),
                        )
                        .child(
                            Icon::new(IconName::Info)
                                .size(px(15.))
                                .text_color(theme.primary),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("文件/EXIF 信息"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(info.name.clone()),
                        ),
                ),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .px_5()
                    .py_3()
                    .gap_1()
                    .child(section("文件", &theme))
                    .child(row("名称", info.name.clone(), &theme))
                    .child(row("位置", info.location.clone(), &theme))
                    .child(row("类型", info.format.clone(), &theme))
                    .child(row("大小", human_size(info.byte_size), &theme))
                    .child(row(
                        "尺寸",
                        format!("{} × {}", info.width, info.height),
                        &theme,
                    ))
                    .when(info.animated, |this| {
                        this.child(row("动画", "是".to_string(), &theme))
                    })
                    .when_some(info.decoded_orientation, |this, value| {
                        this.child(row(s::INFO_ORIENTATION, format!("EXIF {value}"), &theme))
                    })
                    .child(section("EXIF", &theme))
                    .when(has_exif, |this| {
                        this.child(
                            // The body scrolls, not the text view: scrolling
                            // lives on the inner element, so grabbing the bar
                            // cannot start a text selection in the content.
                            // The bar is a sibling overlay, painted after and
                            // above the content rather than inside the scroll
                            // area (inside it, content covered the bar and the
                            // bar did not track the scroll offset).
                            div()
                                .relative()
                                .w_full()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .id("exif-scroll")
                                        .size_full()
                                        .pr(px(12.))
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .child(
                                            TextView::new(&self.text_view)
                                                .selectable(true)
                                                .scrollable(false)
                                                .w_full()
                                                .text_sm(),
                                        ),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .right_0()
                                        .bottom_0()
                                        .w(px(10.))
                                        .child(
                                            Scrollbar::vertical(&self.scroll)
                                                .mode(ScrollbarMode::Always),
                                        ),
                                ),
                        )
                    })
                    .when(!has_exif, |this| {
                        this.child(
                            div()
                                .py_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(s::INFO_NO_EXIF),
                        )
                    }),
            )
            // The window is client-decorated, so its resize zone is very close
            // to the content. These strips turn a press on an edge into an OS
            // resize instead of letting the text view see a drag starting in
            // it — that was how resizing selected a line.
            .child(resize_strip("info-rz-left", ResizeEdge::Left, cx))
            .child(resize_strip("info-rz-right", ResizeEdge::Right, cx))
            .child(resize_strip("info-rz-top", ResizeEdge::Top, cx))
            .child(resize_strip("info-rz-bottom", ResizeEdge::Bottom, cx))
    }
}

fn resize_strip(
    id: &'static str,
    edge: ResizeEdge,
    cx: &mut Context<InfoWindow>,
) -> AnyElement {
    let strip = div().id(id).absolute().occlude();
    let strip = match edge {
        ResizeEdge::Left => strip.left_0().top_0().bottom_0().w(px(6.)),
        ResizeEdge::Right => strip.right_0().top_0().bottom_0().w(px(6.)),
        ResizeEdge::Top => strip.left_0().right_0().top_0().h(px(6.)),
        ResizeEdge::Bottom => strip.left_0().right_0().bottom_0().h(px(6.)),
        // Corners are left to the platform's own resize border.
        _ => strip,
    };
    strip
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.window_manipulated = true;
                this.arm_selection_guard(cx);
                window.start_window_resize(edge);
            }),
        )
        .into_any_element()
}

/// The EXIF block as Markdown: short fields become a Key/Value table, long
/// ones (the `UserComment` prompt) keep their line breaks in a code block.
fn exif_markdown(info: &ImageInfo) -> String {
    let Some(exif) = &info.exif else {
        return String::new();
    };

    let mut short = Vec::new();
    let mut long = Vec::new();
    let summary = [
        (s::INFO_CAMERA, exif.camera.as_deref()),
        (s::INFO_LENS, exif.lens.as_deref()),
        (s::INFO_EXPOSURE, exif.exposure.as_deref()),
        (s::INFO_APERTURE, exif.aperture.as_deref()),
        (s::INFO_ISO, exif.iso.as_deref()),
        (s::INFO_FOCAL, exif.focal.as_deref()),
        (s::INFO_TAKEN, exif.taken.as_deref()),
        ("软件", exif.software.as_deref()),
    ];
    for (label, value) in summary {
        if let Some(value) = value {
            push_field(&mut short, &mut long, label, value);
        }
    }
    for (tag, value) in &exif.fields {
        push_field(&mut short, &mut long, tag, value);
    }

    let mut out = String::new();
    if !short.is_empty() {
        out.push_str("| 字段 | 值 |\n| --- | --- |\n");
        for (key, value) in &short {
            out.push_str("| ");
            out.push_str(&escape_cell(key));
            out.push_str(" | ");
            out.push_str(&escape_cell(value));
            out.push_str(" |\n");
        }
    }

    for (key, value) in &long {
        if key == "用户注释" {
            let (prompt, negative, params) = split_comment(value);
            if !prompt.is_empty() {
                out.push_str("### 提示词\n\n```\n");
                out.push_str(&prompt);
                out.push_str("\n```\n\n");
            }
            if !negative.is_empty() {
                out.push_str("### 负面提示词\n\n```\n");
                out.push_str(&negative);
                out.push_str("\n```\n\n");
            }
            if !params.is_empty() {
                out.push_str("### 生成参数\n\n");
                for (key, value) in &params {
                    out.push_str("**");
                    out.push_str(&escape_inline(key));
                    out.push_str(":** ");
                    out.push_str(&escape_inline(value));
                    out.push_str("\n\n");
                }
            }
        } else {
            out.push_str("### ");
            out.push_str(key);
            out.push_str("\n\n```\n");
            out.push_str(value);
            out.push_str("\n```\n");
        }
    }

    out.trim_end().to_string()
}

fn push_field(
    short: &mut Vec<(String, String)>,
    long: &mut Vec<(String, String)>,
    key: &str,
    value: &str,
) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if value.contains('\n') || value.chars().count() > 120 {
        long.push((key.to_string(), value.to_string()));
    } else {
        short.push((key.to_string(), value.to_string()));
    }
}

/// Split a `UserComment` into its positive prompt, negative prompt and the
/// `Key: Value` generation parameters appended by the tool that wrote it.
fn split_comment(comment: &str) -> (String, String, Vec<(String, String)>) {
    let text = comment.replace("\r\n", "\n");
    let lines: Vec<&str> = text.lines().collect();
    let starts = |line: &str, prefix: &str| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
    };
    let negative_at = lines.iter().position(|line| starts(line, "Negative prompt:"));
    let params_at = lines.iter().position(|line| starts(line, "Steps:"));

    let prompt_end = negative_at.or(params_at).unwrap_or(lines.len());
    let prompt = lines[..prompt_end].join("\n").trim().to_string();

    let (negative, _) = if let Some(index) = negative_at {
        let marker = "Negative prompt:";
        let trimmed = lines[index].trim_start();
        let inline = trimmed[marker.len().min(trimmed.len())..].trim();
        let end = params_at.filter(|at| *at > index).unwrap_or(lines.len());
        let mut negative = String::new();
        if !inline.is_empty() {
            negative.push_str(inline);
        }
        let body = lines[index + 1..end].join("\n");
        if !body.trim().is_empty() {
            if !negative.is_empty() {
                negative.push('\n');
            }
            negative.push_str(body.trim());
        }
        (negative.trim().to_string(), end)
    } else {
        (String::new(), prompt_end)
    };

    let params = params_at
        .map(|index| parse_params(&lines[index..].join("\n")))
        .unwrap_or_default();
    (prompt, negative, params)
}

/// Parse the comma-separated `Key: Value` tail into one pair per line.
///
/// Commas inside quotes stay in the value (`Lora hashes` is one long quoted
/// value), and a comma-separated fragment without its own key is appended to
/// the previous value instead of becoming a bogus parameter.
fn parse_params(text: &str) -> Vec<(String, String)> {
    let mut params: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = Vec::new();
        let mut part = String::new();
        let mut quoted = false;
        for character in line.chars() {
            match character {
                '"' => {
                    quoted = !quoted;
                    part.push(character);
                }
                ',' if !quoted => {
                    parts.push(part.trim().to_string());
                    part.clear();
                }
                _ => part.push(character),
            }
        }
        parts.push(part.trim().to_string());

        for part in parts {
            if part.is_empty() {
                continue;
            }
            // The first colon outside quotes separates the key from the value;
            // colons inside `Lora hashes`' quoted string do not.
            let mut quoted = false;
            let mut colon = None;
            for (index, character) in part.char_indices() {
                match character {
                    '"' => quoted = !quoted,
                    ':' if !quoted => {
                        colon = Some(index);
                        break;
                    }
                    _ => {}
                }
            }
            if let Some(index) = colon {
                let key = part[..index].trim();
                if !key.is_empty() && key.chars().count() <= 48 {
                    params.push((key.to_string(), part[index + 1..].trim().to_string()));
                    continue;
                }
            }
            if let Some(last) = params.last_mut() {
                last.1.push_str(", ");
                last.1.push_str(&part);
            }
        }
    }
    params
}

/// Escape the characters that would break a Markdown table cell or inline
/// value, or be read as HTML.
fn escape_cell(value: &str) -> String {
    escape_inline(value).replace('|', "\\|")
}

fn escape_inline(value: &str) -> String {
    value.replace('<', "\\<").replace('>', "\\>")
}

/// Whether the physical left mouse button is down, independent of GPUI's
/// event state.
#[cfg(target_os = "windows")]
fn host_left_button_down() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    // SAFETY: GetAsyncKeyState takes no pointers and has no preconditions.
    unsafe { (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) != 0 }
}

#[cfg(not(target_os = "windows"))]
fn host_left_button_down() -> bool {
    true
}

fn section(title: &'static str, theme: &Theme) -> impl IntoElement {    div()
        .text_xs()
        .mt_2()
        .mb_1()
        .text_color(theme.muted_foreground)
        .child(title)
}

fn row(label: &'static str, value: String, theme: &Theme) -> impl IntoElement {
    h_flex()
        .w_full()
        .py_1()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(88.))
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().text_sm().child(value))
}
