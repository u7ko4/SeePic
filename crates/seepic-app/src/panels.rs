//! The right-hand panels, the mini title, the zoom control and the file
//! navigator.
//!
//! Eight panels are docked at the top right and one is shown at a time; the
//! same shell is used for all of them so they behave identically — a title,
//! a close button, a scrollable body, and dismissal when another panel opens.

use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;

use crate::app::{SidePanel, ViewerApp};

/// Height of the navigator window.
const NAVIGATOR_HEIGHT: f32 = 460.;

/// Render whichever panel is open.
///
/// Every panel is a centred dialog: the same frame, a title, a close button and
/// a scrollable body. They are modal because that is what they are — nothing
/// behind one can be used until it is dismissed.
pub fn render(
    view: &ViewerApp,
    panel: SidePanel,
    window: &Window,
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    let _ = window;

    let width = match panel {
        SidePanel::Image | SidePanel::Bookmark => 440.,
        SidePanel::Sort => 400.,
        SidePanel::Slideshow => 380.,
    };

    // The body is built first: `modal` needs the context too, and one can only
    // be borrowed mutably once at a time.
    let body: AnyElement = match panel {
        SidePanel::Image => crate::image_panel::body(view, cx).into_any_element(),
        SidePanel::Sort => sort_body(view, cx).into_any_element(),
        SidePanel::Bookmark => bookmark_body(view, cx).into_any_element(),
        SidePanel::Slideshow => crate::slideshow::body(view, cx).into_any_element(),
    };

    modal("panel", panel.title(), width, body, ViewerApp::close_panel, cx)
}

/// The shared dialog frame: a dim backdrop, a title row with a close button,
/// and a scrollable body.
///
/// `close` is what the close button and a press on the backdrop run — a panel
/// clears the panel stack, while the information overlay clears its own flag.
pub fn modal(
    key: &'static str,
    title: impl Into<SharedString>,
    width: f32,
    body: impl IntoElement,
    close: fn(&mut ViewerApp, &mut Context<ViewerApp>),
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let title = title.into();

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
        // A press outside dismisses, like the settings window.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                close(this, cx);
            }),
        )
        .child(
            v_flex()
                .w(px(width))
                .max_h(px(640.))
                .rounded(px(14.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .overflow_hidden()
                // Swallows the press so it reaches neither the backdrop nor the
                // canvas behind it.
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_4()
                        .py_3()
                        .gap_2()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex_1()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .id(gpui::SharedString::from(format!("{key}-panel-close")))
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(16.)))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    close(this, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("{key}-panel-body")))
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_4()
                        .py_3()
                        .child(body),
                ),
        )
        .into_any_element()
}

fn section(title: &'static str, cx: &Context<ViewerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .mt_2()
        .mb_1()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

/// A radio row: a dot plus a label, mutually exclusive within its group.
fn radio_row(
    id: (&'static str, usize),
    label: &'static str,
    selected: bool,
    action: Action,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(id)
        .w_full()
        .gap_2p5()
        .px_2()
        .py_1p5()
        .rounded(px(6.))
        .cursor_pointer()
        .items_center()
        .when(selected, |this| this.bg(theme.list_active))
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
            run(this, action, window, cx);
        }))
}


/// A row that runs a command.
fn command_row(
    id: &'static str,
    label: &'static str,
    hint: Option<String>,
    command: Command,
    enabled: bool,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .id(id)
        .w_full()
        .px_2()
        .py_1p5()
        .rounded(px(6.))
        .gap_3()
        .justify_between()
        .items_center()
        .when(enabled, |this| this.cursor_pointer())
        .when(enabled, |this| this.hover(|this| this.bg(theme.list_hover)))
        .child(
            div()
                .text_sm()
                .when(!enabled, |this| this.text_color(theme.muted_foreground))
                .child(label),
        )
        .when_some(hint, |this, hint| {
            this.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(hint),
            )
        })
        .when(enabled, |this| {
            this.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.execute(command, window, cx);
            }))
        })
}


/// What a panel control does, so the rows can stay declarative.
#[derive(Clone, Copy)]
pub enum Action {
    ChromeMode(seepic_core::settings::ChromeMode),
    ThumbnailView(seepic_core::settings::ThumbnailView),
    Sort(seepic_core::SortKey),
    /// Set the direction outright, for the 升序 / 降序 rows.
    SortDirection(bool),
    BookmarkOpen(usize),
    BookmarkRemove(usize),
    BookmarkClear,
}

/// Run one of the panel actions, for callers outside this module.
pub fn run_action(
    view: &mut ViewerApp,
    action: Action,
    window: &mut Window,
    cx: &mut Context<ViewerApp>,
) {
    run(view, action, window, cx);
}

fn run(view: &mut ViewerApp, action: Action, window: &mut Window, cx: &mut Context<ViewerApp>) {
    match action {
        Action::ChromeMode(mode) => view.set_chrome_mode(mode, cx),
        Action::ThumbnailView(choice) => {
            view.settings.thumbnail_view = choice;
            view.contact_sheet = choice == seepic_core::settings::ThumbnailView::Grid
                && view.contact_sheet;
            view.settings_changed(cx);
        }
        Action::Sort(key) => view.apply_sort(key, cx),
        Action::SortDirection(descending) => {
            if view.sort_order.descending != descending {
                view.execute(Command::ToggleSortAscending, window, cx);
            }
        }
        Action::BookmarkOpen(index) => view.open_bookmark(index, cx),
        Action::BookmarkRemove(index) => view.remove_bookmark(index, cx),
        Action::BookmarkClear => view.clear_bookmarks(cx),
    }
}

// ---- Zoom --------------------------------------------------------------------


// ---- Sort --------------------------------------------------------------------

fn sort_body(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let order = view.sort_order;

    v_flex()
        .child(section("文件排序方式", cx))
        .children(seepic_core::SortKey::ALL.into_iter().enumerate().map(
            |(index, key)| {
                radio_row(
                    ("sort-key", index),
                    key.label(),
                    order.key == key,
                    Action::Sort(key),
                    cx,
                )
                .into_any_element()
            },
        ))
        .child(section("排序方向", cx))
        .child(radio_row(
            ("sort-dir", 0),
            "升序",
            !order.descending,
            Action::SortDirection(false),
            cx,
        ))
        .child(radio_row(
            ("sort-dir", 1),
            "降序",
            order.descending,
            Action::SortDirection(true),
            cx,
        ))
}

// ---- Bookmark ----------------------------------------------------------------

fn bookmark_body(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let mut body = v_flex()
        .child(section("收藏夹", cx))
        .child(command_row(
            "bm-add",
            "＋ 添加到收藏夹",
            Some("B".to_string()),
            Command::BookmarkAdd,
            view.has_images(),
            cx,
        ))
        .child(command_row(
            "bm-recent",
            "打开最近添加的收藏",
            Some("F3".to_string()),
            Command::BookmarkLoadRecent,
            !view.bookmarks.is_empty(),
            cx,
        ))
        .child(
            h_flex()
                .id("bm-clear")
                .w_full()
                .px_2()
                .py_1p5()
                .rounded(px(6.))
                .gap_3()
                .justify_between()
                .items_center()
                .when(!view.bookmarks.is_empty(), |this| {
                    this.cursor_pointer()
                        .hover(|this| this.bg(theme.list_hover))
                })
                .child(
                    div()
                        .text_sm()
                        .when(view.bookmarks.is_empty(), |this| {
                            this.text_color(theme.muted_foreground)
                        })
                        .child("清空收藏夹"),
                )
                .when(!view.bookmarks.is_empty(), |this| {
                    this.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        run(this, Action::BookmarkClear, window, cx);
                    }))
                }),
        );

    if view.bookmarks.is_empty() {
        body = body.child(
            div()
                .px_2()
                .py_6()
                .text_sm()
                .text_center()
                .text_color(theme.muted_foreground)
                .child("收藏夹为空"),
        );
        return body;
    }

    for (index, bookmark) in view.bookmarks.iter().enumerate() {
        let name = bookmark
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| bookmark.path.display().to_string());
        let size = seepic_core::metadata::human_size(bookmark.size);
        body = body.child(
            h_flex()
                .id(("bookmark", index))
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .hover(|this| this.bg(theme.list_hover))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .child(name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(size),
                )
                .child(
                    div()
                        .id(("bookmark-open", index))
                        .p_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.button_hover))
                        .child(Icon::new(IconName::FolderOpen).size(px(12.)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            run(this, Action::BookmarkOpen(index), window, cx);
                        })),
                )
                .child(
                    div()
                        .id(("bookmark-remove", index))
                        .p_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.button_hover))
                        .child(Icon::new(IconName::Trash).size(px(12.)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            run(this, Action::BookmarkRemove(index), window, cx);
                        })),
                ),
        );
    }
    body
}


/// The file navigator: a list of the container's images, sorted as the container
/// is. `Enter` opens it; a click selects, a double click opens.
pub fn navigator(
    view: &ViewerApp,
    _window: &Window,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = view.entry_count();
    let cursor = view.cursor_position();
    let selection = view.navigator_selection;

    let mut list = v_flex().w_full();
    // A long comic would otherwise build thousands of rows every frame.
    let first = selection.saturating_sub(40);
    let last = (first + 80).min(count);
    for index in first..last {
        let selected = index == cursor;
        let highlighted = index == selection;
        list = list.child(
            h_flex()
                .id(("nav-row", index))
                .w_full()
                .px_3()
                .py_1()
                .gap_3()
                .items_center()
                .when(selected, |this| this.bg(theme.list_active))
                .when(highlighted && !selected, |this| this.bg(theme.list_hover))
                .hover(|this| this.bg(theme.list_hover))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .child(view.entry_name(index)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{}", index + 1)),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                    this.navigator_selection = index;
                    if event.click_count() > 1 {
                        this.show_navigator = false;
                        this.select_position(index, cx);
                    } else {
                        cx.notify();
                    }
                })),
        );
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
            this.show_navigator = false;
            cx.notify();
        }))
        .child(
            v_flex()
                .w(px(520.))
                .h(px(NAVIGATOR_HEIGHT))
                .rounded(px(12.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                    // Clicks inside the dialog must not reach the backdrop and
                    // dismiss it.
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
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("选择图像"),
                        )
                        .child(
                            div()
                                .id("navigator-close")
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(14.)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.show_navigator = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("navigator-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(list),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_4()
                        .py_2()
                        .gap_2()
                        .justify_end()
                        .border_t_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .id("navigator-open")
                                .px_4()
                                .py_1()
                                .rounded(px(6.))
                                .text_sm()
                                .cursor_pointer()
                                .bg(theme.primary)
                                .text_color(theme.primary_foreground)
                                .child("打开")
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    let index = this.navigator_selection;
                                    this.show_navigator = false;
                                    this.select_position(index, cx);
                                })),
                        ),
                ),
        )
}

/// The file search: a filter over the open folder or archive.
pub fn file_search(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> AnyElement {
    let theme = cx.theme().clone();
    let Some(search) = view.search.as_ref() else {
        return div().into_any_element();
    };
    let total = view.entry_count();
    let count = search.matches.len();
    let column = view.search_col_width;

    let mut list = v_flex()
        .id("search-results")
        .w_full()
        .gap_0p5()
        // The results are the only part of the panel that can outgrow it; the
        // bar appears while it scrolls and withdraws again (the component's
        // own idle animation).
        .max_h(px(420.))
        .overflow_y_scroll()
        .track_scroll(&view.search_scroll)
        .vertical_scrollbar(&view.search_scroll);
    for (index, position) in search.matches.iter().enumerate() {
        let position = *position;
        let selected = index == search.selection;
        let name = view.entry_name(position);
        list = list.child(
            h_flex()
                .id(("search-row", position))
                .w_full()
                .px_2()
                .py_1p5()
                .rounded(px(6.))
                .gap_3()
                .items_center()
                .cursor_pointer()
                .when(selected, |this| this.bg(theme.list_active))
                .hover(|this| this.bg(theme.list_hover))
                .child(
                    div()
                        .w(px(column))
                        .flex_shrink_0()
                        .text_xs()
                        .text_right()
                        .text_color(theme.muted_foreground)
                        .child(format!("{}/{}", position + 1, total)),
                )
                .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.search = None;
                    this.search_col_drag = None;
                    this.contact_sheet = false;
                    this.contact_filter.clear();
                    this.contact_matches.clear();
                    this.select_position(position, cx);
                    cx.notify();
                })),
        );
    }
    if count == 0 {
        list = list.child(
            div()
                .px_2()
                .py_4()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("没有匹配的文件"),
        );
    }

    let header = h_flex()
        .w_full()
        .px_2()
        .py_1()
        .items_center()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(
            div()
                .w(px(column))
                .flex_shrink_0()
                .text_right()
                .child("序号"),
        )
        // The divider is the resize handle: a wide hit area around a hairline,
        // with the familiar east-west cursor.
        .child(
            div()
                .id("search-col-resize")
                .w(px(9.))
                .h(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::ResizeLeftRight)
                .child(div().w(px(1.)).h(px(12.)).bg(theme.border))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                        this.search_col_drag =
                            Some((f32::from(event.position.x), this.search_col_width));
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(div().flex_1().min_w_0().pl_1().child("文件名"));

    let body = v_flex()
        .w_full()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .bg(theme.input)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .when(search.filter.is_empty(), |this| {
                            this.text_color(theme.muted_foreground)
                        })
                        .child(if search.filter.is_empty() {
                            "输入文件名以搜索…".to_string()
                        } else {
                            search.filter.clone()
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if search.filter.is_empty() {
                            format!("共 {total} 项")
                        } else {
                            format!("{count} 项")
                        }),
                ),
        )
        .child(header)
        .child(list)
        .child(
            div()
                .px_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("↑↓ 选择，回车打开，Esc 关闭"),
        )
        .on_mouse_move(cx.listener(
            |this, event: &MouseMoveEvent, _window, cx| {
                let Some((start_x, start_width)) = this.search_col_drag else {
                    return;
                };
                this.search_col_width = (start_width + f32::from(event.position.x) - start_x)
                    .clamp(crate::app::SEARCH_COL_MIN, crate::app::SEARCH_COL_MAX);
                cx.notify();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                if this.search_col_drag.take().is_some() {
                    cx.notify();
                }
            }),
        )
        .into_any_element();

    modal("search", "搜索文件", 560., body, ViewerApp::close_file_search, cx)
}

/// The command list, used to bind a mouse button to any command in the table.
pub fn command_picker(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> AnyElement {
    let theme = cx.theme().clone();
    let Some(picker) = view.picker.as_ref() else {
        return div().into_any_element();
    };
    let matches = picker.matches();
    let title = match picker.slot {
        Some(slot) => format!("为「{}」选择指令", slot.label()),
        // Opened from 更多 → 快捷键一览, this is the reference list.
        None => "指令与快捷键".to_string(),
    };

    let mut list = v_flex().w_full();
    for command in matches.iter().take(400) {
        let command = *command;
        list = list.child(
            h_flex()
                .id(command.id())
                .w_full()
                .px_3()
                .py_1()
                .gap_3()
                .items_center()
                .hover(|this| this.bg(theme.list_hover))
                .child(
                    div()
                        .w(px(104.))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(command.group().label()),
                )
                .child(div().flex_1().text_sm().child(command.label()))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(command.binding_label().unwrap_or_default()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.pick_command(command, cx);
                })),
        );
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
            this.close_command_picker(cx);
        }))
        .child(
            v_flex()
                .w(px(600.))
                .h(px(520.))
                .rounded(px(12.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .shadow_2xl()
                .overflow_hidden()
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
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .id("picker-close")
                                .p_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|this| this.bg(theme.button_hover))
                                .child(Icon::new(IconName::Close).size(px(14.)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.close_command_picker(cx);
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_4()
                        .py_2()
                        .gap_2()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("筛选"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .px_2()
                                .py_1()
                                .rounded(px(5.))
                                .bg(theme.input)
                                .text_sm()
                                .child(if picker.filter.is_empty() {
                                    "输入以筛选指令…".to_string()
                                } else {
                                    picker.filter.clone()
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("{} 项", matches.len())),
                        ),
                )
                .child(
                    div()
                        .id("picker-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(list),
                ),
        )
        .into_any_element()
}
