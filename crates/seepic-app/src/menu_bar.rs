//! The single-row top bar: title, menus, tool group and the "more" menu.
//!
//! The title bar, the menu strip and the old floating toolbar used to be three
//! separate rows. They are one row here, which gives the image back the height
//! the other two took and puts everything that acts on the current image within
//! one reach of the pointer.
//!
//! The dropdowns are drawn by hand rather than with an action-based popup
//! component, so every row can run an [`seepic_core::Command`] directly and the
//! shortcut hints come straight from the command table.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;
use seepic_core::settings::{ChromeMode, MouseSlot};

use crate::app::{SidePanel, ViewerApp};
use crate::strings as s;

/// Which dropdown is open, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    View,
    Image,
    More,
    /// Only ever a submenu of 更多.
    Clipboard,
}

/// The menus the bar itself carries, left to right.
pub const MENU_COUNT: usize = 4;

/// Every dropdown the bar can show, counting the submenus, so the app can size
/// its anchor array. Only `MENUS` hang off the bar and are measured; a submenu
/// takes over the anchor of the menu it hangs from.
pub const MENU_SLOTS: usize = 5;

impl Menu {
    /// Whether this menu hangs off another one rather than off the bar.
    pub fn is_submenu(self) -> bool {
        matches!(self, Menu::Clipboard)
    }
}

const MENUS: [(Menu, &str); MENU_COUNT] = [
    (Menu::File, s::MENU_FILE),
    (Menu::View, s::MENU_VIEW),
    (Menu::Image, s::MENU_IMAGE),
    (Menu::More, s::MENU_MORE),
];

/// One row in a dropdown.
pub enum Item {
    Command(Command),
    /// A command shown under a label of its own.
    ///
    /// Used where the row should read like an action ("按名称排序") while doing
    /// something broader (opening the sort panel).
    Labeled(&'static str, Command),
    /// A toggle or one of a radio group: the label, the command, and whether it
    /// is currently on. A radio group is a run of these with one check between
    /// them.
    Checked(&'static str, Command, bool),
    /// A submenu, opened by clicking the row.
    Submenu(&'static str, Menu),
    Separator,
}

impl Item {
    /// How tall this row is, in logical pixels.
    pub fn height(&self) -> f32 {
        match self {
            Item::Separator => SEPARATOR_HEIGHT,
            _ => ROW_HEIGHT,
        }
    }
}

/// Height of a clickable menu row, and of a separator.
const ROW_HEIGHT: f32 = 32.;
const SEPARATOR_HEIGHT: f32 = 9.;

/// The rows the keyboard can land on: everything but the separators.
pub fn actionable_rows(menu: Menu) -> Vec<usize> {
    menu_items(menu)
        .iter()
        .enumerate()
        .filter(|(_, item)| !matches!(item, Item::Separator))
        .map(|(index, _)| index)
        .collect()
}

fn cmd(command: Command) -> Item {
    Item::Command(command)
}

fn sub(label: &'static str, menu: Menu) -> Item {
    Item::Submenu(label, menu)
}

fn check(label: &'static str, command: Command, on: bool) -> Item {
    Item::Checked(label, command, on)
}

/// Height of the bar, matching the platform title bar the windowed mode uses.
const BAR_HEIGHT: f32 = 34.;
/// Left inset of the bar's content, matching the platform title bar's.
const BAR_LEFT_PADDING: f32 = 12.;
/// Below this window width the icon group is dropped.
///
/// It is the widest part of the bar and the only part whose commands all remain
/// reachable elsewhere — the menus and the status bar both carry them. Dropping
/// it keeps the title, the menus and the platform's window buttons on screen
/// when the window is narrowed.
const TOOLBAR_MIN_WIDTH: f32 = 980.;
/// Below this window width the menu titles are dropped as well.
///
/// The title and the window buttons are what a window this narrow still needs;
/// every menu command remains on the keyboard and the status bar's buttons.
const MENUS_MIN_WIDTH: f32 = 600.;

/// The whole bar: one row across the top of the window.
///
/// In a window the platform's own title bar carries the window buttons: they
/// are real non-client areas, so Windows gives them its hover treatment and the
/// snap-layout popup the maximise button has on Windows 11. In fullscreen the
/// platform drops that hit test (`handle_hit_test_msg` returns early while the
/// window is fullscreen), which leaves those buttons drawn but inert, so the
/// bar is drawn here instead, with buttons of its own.
pub fn render(view: &ViewerApp, window: &Window, cx: &mut Context<ViewerApp>) -> AnyElement {
    if view.fullscreen {
        return fullscreen_bar(view, window, cx).into_any_element();
    }

    let theme = cx.theme().clone();
    // Floating over the image, the bar is see-through so the picture is still
    // readable behind it; taking its own row, it stays solid.
    let floating = view.settings.chrome_mode.reveals_on_hover();

    TitleBar::new()
        .when(floating, |this| this.bg(theme.title_bar.opacity(0.82)))
        .child(
            // `occlude` keeps a press on the bar's empty space from reaching the
            // canvas behind it, which would turn the page. It is also what makes
            // the bar clickable at all: the platform title bar hands its content
            // a caption area to drag the window by, and an occluder in front of
            // it is what keeps the platform from taking every press on the bar
            // as a caption press and swallowing the release (see `bar_row`).
            //
            // `max_w` is what keeps the platform's window buttons on screen:
            // a flex item's content still counts towards its minimum width
            // whatever its own `min_w_0` says, so the width left after the
            // buttons is imposed here and the overflow is clipped. Without it a
            // narrow window lays the buttons out past the right edge, where
            // they can be neither seen nor clicked.
            div()
                .occlude()
                .w_full()
                .max_w(px(bar_content_width(window)))
                .min_w_0()
                .overflow_hidden()
                .child(bar_row(view, window, cx)),
        )
        .into_any_element()
}

/// The bar for fullscreen: the same row, with the window buttons drawn by us.
fn fullscreen_bar(
    view: &ViewerApp,
    window: &Window,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    div().flex_shrink_0().child(
        div()
            .id("title-bar")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(BAR_HEIGHT))
            .pl(px(BAR_LEFT_PADDING))
            .border_b_1()
            .border_color(theme.title_bar_border)
            .bg(title_bar_background(&theme))
            .child(
                div()
                    .occlude()
                    .h_full()
                    .flex_1()
                    // The same guard as the windowed bar: the controls after
                    // this row must stay inside the window.
                    .max_w(px(bar_content_width(window)))
                    .min_w_0()
                    .overflow_hidden()
                    .child(bar_row(view, window, cx)),
            )
            .child(window_controls(cx)),
    )
}

/// The platform title bar's background: the bar colour mixed a little towards
/// the window background at the top, so the bar reads as a raised surface.
fn title_bar_background(theme: &Theme) -> Background {
    let bar = theme.title_bar.to_rgb();
    let background = theme.background.to_rgb();
    let mixed = Hsla::from(Rgba {
        r: bar.r * 0.55 + background.r * 0.45,
        g: bar.g * 0.55 + background.g * 0.45,
        b: bar.b * 0.55 + background.b * 0.45,
        a: bar.a * 0.55 + background.a * 0.45,
    });
    linear_gradient(
        180.,
        linear_color_stop(mixed, 0.),
        linear_color_stop(theme.title_bar, 1.),
    )
}

/// Minimise, leave fullscreen, close: the three cells the platform title bar
/// would have handled itself.
fn window_controls(cx: &mut Context<ViewerApp>) -> impl IntoElement {
    h_flex()
        .h_full()
        .flex_shrink_0()
        .child(control_cell(
            "chrome-minimize",
            IconName::WindowMinimize,
            false,
            cx,
            |_this, window, _cx| window.minimize_window(),
        ))
        .child(control_cell(
            "chrome-restore",
            IconName::WindowRestore,
            false,
            cx,
            |this, window, cx| this.toggle_fullscreen(window, cx),
        ))
        .child(control_cell(
            "chrome-close",
            IconName::WindowClose,
            true,
            cx,
            |_this, window, _cx| window.remove_window(),
        ))
}

/// One window button. The width and the hover colours are the platform title
/// bar's, so the bar does not change shape between the two modes.
fn control_cell(
    id: &'static str,
    icon: IconName,
    danger: bool,
    cx: &mut Context<ViewerApp>,
    on_click: fn(&mut ViewerApp, &mut Window, &mut Context<ViewerApp>),
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let hover_bg = if danger { theme.danger } else { theme.secondary_hover };
    let hover_fg = if danger {
        theme.danger_foreground
    } else {
        theme.secondary_foreground
    };

    div()
        .id(id)
        .flex()
        .w(px(BAR_HEIGHT))
        .h_full()
        .flex_shrink_0()
        .justify_center()
        .items_center()
        .text_color(theme.foreground)
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg).text_color(hover_fg))
        .active(move |this| this.bg(hover_bg).text_color(hover_fg))
        .child(Icon::new(icon).small())
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            on_click(this, window, cx);
        }))
}

/// The width left for the bar's own content once the platform's window buttons
/// and the bar's left inset are accounted for.
fn bar_content_width(window: &Window) -> f32 {
    (f32::from(window.bounds().size.width) - BAR_LEFT_PADDING - window_controls_width(window))
        .max(0.)
}

/// How much of the title bar the platform's window buttons take.
///
/// The platform draws them itself, to the right of the row we fill. macOS puts
/// its traffic lights in the left padding, and a server-side-decorated Linux
/// window draws none of our buttons, so nothing is reserved there.
fn window_controls_width(window: &Window) -> f32 {
    if cfg!(target_os = "macos") || cfg!(target_family = "wasm") {
        return 0.;
    }
    #[cfg(target_os = "linux")]
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return 0.;
    }
    let supported = window.window_controls();
    let cells = 1 + usize::from(supported.minimize) + usize::from(supported.maximize);
    cells as f32 * BAR_HEIGHT
}

/// Whether the window is wide enough for the menu titles.
pub fn menus_fit(window: &Window) -> bool {
    f32::from(window.bounds().size.width) >= MENUS_MIN_WIDTH
}

/// The row itself: title, menus, optional icon group, and the two window-wide
/// toggles. Both bars share it, so they cannot drift apart.
fn bar_row(view: &ViewerApp, window: &Window, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    // The bar gives parts up as the window narrows, in the order that costs the
    // reader least: the icon group (its commands live in the menus and the
    // status bar), then the menu titles themselves (the keyboard still has every
    // command), leaving the title and the window buttons.
    let width = f32::from(window.bounds().size.width);
    let show_tools = view.settings.show_toolbar && width >= TOOLBAR_MIN_WIDTH;
    let show_menus = menus_fit(window);
    // The menu buttons are direct children, not a nested row, so their
    // prepaint indices line up with `MENUS`.
    let mut row = h_flex()
        .w_full()
        .min_w_0()
        .pr_3()
        .gap_2()
        .items_center()
        .text_sm()
        .child(title_content(view, cx));
    if show_menus {
        row = row.child(div().flex_shrink_0().w(px(1.)).h(px(18.)).bg(theme.border));
        for (menu, label) in MENUS {
            row = row.child(menu_button(menu, label, view, cx));
        }
    }
    // The icon group is optional: the same commands are in the menus, the
    // status bar and the keyboard.
    row = row.child(div().flex_1());
    if show_tools {
        row = row.child(tools(view, cx)).child(div().flex_1());
    }
    row.child(icon_button(
        "chrome-fullscreen",
        if view.fullscreen {
            IconName::Minimize
        } else {
            IconName::Fullscreen
        },
        Command::ToggleFullscreen,
        view,
        cx,
    ))
    .child(pin_button(view, cx))
    // The bar is our own chrome, so the platform has no caption area to drag
    // by. A press arms the drag; the first movement starts it, and a press that
    // does not move still reaches the buttons underneath. A fullscreen window
    // covers its display already, so there is nowhere to drag it to.
    .when(!view.fullscreen, |this| {
        this.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, _cx| {
                this.chrome_drag_armed = true;
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _window, _cx| {
                this.chrome_drag_armed = false;
            }),
        )
    })
    // Fullscreen, the bar is the window's caption, and the platform has no
    // caption behaviour there. The root never sees these releases either: the
    // bar's occluder is what keeps it clickable, and it stops the hit test
    // before the root, so the double click is recognised on the bar itself.
    .when(view.fullscreen, |this| {
        this.on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, event: &MouseUpEvent, window, cx| {
                let doubled = this.register_click_up(
                    f32::from(event.position.x),
                    f32::from(event.position.y),
                );
                if doubled && let Some(command) = this.settings.mouse_command(MouseSlot::DoubleLeft)
                {
                    this.execute(command, window, cx);
                }
            }),
        )
    })
    // Record where each menu title landed: the dropdown hangs below the button
    // that was clicked. The anchors are in the same layout pixels the overlay
    // positions itself in.
    .on_children_prepainted({
        let entity = cx.entity();
        move |bounds: Vec<Bounds<Pixels>>, _window, cx: &mut App| {
            entity.update(cx, |this, cx| {
                let mut changed = false;
                // Children in order, when the menus are drawn: the title
                // content, the divider, then the menu titles. With the menus
                // hidden there is no button for a dropdown to hang from.
                for (index, (menu, _)) in MENUS.iter().enumerate() {
                    let slot = &mut this.menu_anchors[*menu as usize];
                    let wanted = if show_menus {
                        bounds.get(index + 2).map(|bounds| {
                            (
                                f32::from(bounds.origin.x),
                                f32::from(bounds.origin.y + bounds.size.height),
                            )
                        })
                    } else {
                        None
                    };
                    if *slot != wanted {
                        *slot = wanted;
                        changed = true;
                    }
                }
                if changed {
                    cx.notify();
                }
            });
        }
    })
}

/// The icon, the file name and the position, in the order that answers
/// "where am I?".
fn title_content(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();

    h_flex()
        .gap_2()
        .items_center()
        // The title gives up width before anything else does: truncated with an
        // ellipsis it still says which file is open, while the menus and the
        // window buttons have no shorter form.
        .min_w_0()
        .child(
            Icon::new(IconName::Image)
                .size(px(16.))
                .flex_shrink_0()
                .text_color(theme.primary),
        )
        .child(
            div()
                .max_w(px(420.))
                .min_w_0()
                .truncate()
                .child(view.title()),
        )
        .when(view.is_animated(), |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "第 {} / {} 帧",
                        view.held_frame().unwrap_or(0) + 1,
                        view.frame_count()
                    )),
            )
        })
}

/// The menu titles.
/// The tool group: the actions worth one click rather than two.
fn tools(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    h_flex()
        .gap_0p5()
        .items_center()
        .flex_shrink_0()
        .child(icon_button(
            "tool-zoom-out",
            IconName::ZoomOut,
            Command::ZoomOut,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-zoom-in",
            IconName::ZoomIn,
            Command::ZoomIn,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-fit",
            IconName::Expand,
            Command::ViewModeFitWindow,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-rotate",
            IconName::RotateCw,
            Command::RotateCw,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-prev",
            IconName::ChevronLeft,
            Command::PrevImage,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-next",
            IconName::ChevronRight,
            Command::NextImage,
            view,
            cx,
        ))
        // The same timer button, with the same effect, as the status bar's.
        .child(icon_button(
            "tool-slideshow",
            IconName::Timer,
            Command::ToggleSlideshowPanel,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-thumbnails",
            IconName::GalleryVerticalEnd,
            Command::ToggleThumbnails,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-info",
            IconName::Info,
            Command::ToggleInfo,
            view,
            cx,
        ))
        .child(icon_button(
            "tool-settings",
            IconName::Settings,
            Command::Settings,
            view,
            cx,
        ))
}

/// A small icon button that runs a command.
fn icon_button(
    id: &'static str,
    icon: IconName,
    command: Command,
    view: &ViewerApp,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let enabled = command_is_available(command, view);
    let active = match command {
        Command::SearchFiles => view.search.is_some(),
        Command::ToggleThumbnails => view.thumbnails_visible(),
        Command::ToggleInfo => view.show_info,
        Command::SlideshowToggle | Command::SlideshowStop => view.slideshow.is_running(),
        _ => false,
    };

    div()
        .id(id)
        .p_1p5()
        .rounded(px(5.))
        .when(active, |this| this.bg(theme.button_active))
        .when(enabled, |this| this.cursor_pointer())
        .when(enabled, |this| this.hover(|this| this.bg(theme.button_hover)))
        .child(
            Icon::new(icon)
                .size(px(15.))
                .text_color(if active {
                    theme.primary
                } else if enabled {
                    theme.muted_foreground
                } else {
                    theme.muted_foreground.opacity(0.4)
                }),
        )
        .when(enabled, |this| {
            this.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.execute(command, window, cx);
            }))
        })
}

/// The pin: whether the bars keep their own space or float over the image.
///
/// It is the one-click version of the 界面模式 choice in the 视图 menu.
fn pin_button(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let fixed = view.settings.chrome_mode == ChromeMode::Fixed;

    div()
        .id("chrome-pin")
        .p_1p5()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .child(
            Icon::new(if fixed { IconName::Pin } else { IconName::PinOff })
                .size(px(15.))
                .text_color(if fixed {
                    theme.foreground
                } else {
                    theme.muted_foreground
                }),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.execute(Command::ToggleChromeRows, window, cx);
        }))
}

/// A menu title, with its dropdown hanging below it when open.
fn menu_button(
    menu: Menu,
    label: &'static str,
    view: &ViewerApp,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let is_open = view.open_menu == Some(menu);

    div()
        .relative()
        .flex_shrink_0()
        .child(
            div()
                .id(("menu", menu as usize))
                .px_2p5()
                .py_1()
                .rounded(px(5.))
                .cursor_pointer()
                .when(is_open, |this| this.bg(theme.button_active))
                .hover(|this| this.bg(theme.button_hover))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_menu(menu);
                    cx.notify();
                })),
        )
}

/// The way out when the chrome is hidden.
///
/// A mode that hides both bars takes the toggles with it, so this floats the
/// fullscreen button in the corner while the pointer is at the top.
pub fn fullscreen_exit_overlay(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
        .absolute()
        .top(px(8.))
        .right(px(12.))
        .rounded(px(6.))
        .bg(theme.title_bar.opacity(0.75))
        .border_1()
        .border_color(theme.border)
        .occlude()
        .child(icon_button(
            "chrome-fullscreen-exit",
            IconName::Minimize,
            Command::ToggleFullscreen,
            view,
            cx,
        ))
}

/// The open dropdown, drawn as a root-level overlay.
///
/// It has to be here rather than inside the bar: GPUI paints in tree order, and
/// the bar is a flow row *above* the canvas, so a dropdown nested in it would be
/// covered by the image. The vertical position is the clicked button's bottom;
/// the left edge is shared by every dropdown (see `common_x` below).
pub fn dropdown_overlay(
    view: &ViewerApp,
    window: &Window,
    cx: &mut Context<ViewerApp>,
) -> Option<AnyElement> {
    let menu = view.open_menu?;
    let width = f32::from(window.bounds().size.width);
    // Every panel shares one vertical edge, taken from the first menu title.
    // Anchoring each dropdown to its own button made the list jump sideways
    // when the user moved between 文件/视图/图像/更多, and the submenu added a
    // further 18 px step to the right. A submenu copies its parent's anchor,
    // so the same edge covers it too.
    let common_x = MENUS
        .iter()
        .filter_map(|(menu, _)| view.menu_anchors.get(*menu as usize).copied().flatten())
        .map(|(x, _)| x)
        .fold(f32::MAX, f32::min);
    let anchor_bottom = view
        .menu_anchors
        .get(menu as usize)
        .copied()
        .flatten()
        .map_or(0., |(_, bottom)| bottom);
    let left = if common_x.is_finite() { common_x } else { 8. }
        .clamp(8., (width - 288.).max(8.));
    // The overlay is laid out inside the content area, which itself starts
    // below the chrome when the chrome takes its own rows. Subtracting that
    // offset puts the dropdown directly under the button in both layouts.
    let top = (anchor_bottom - view.chrome_height()).max(0.);

    // The File menu is long enough to run past the bottom of a small window, so
    // it is bounded and scrolls rather than hiding its last items.
    let max_height =
        (f32::from(window.bounds().size.height) - top - view.chrome_height() - 8.).max(120.);

    // The keyboard highlight may be off-screen; the list owns its own scroll
    // position, so bringing the row into view happens here rather than by
    // computing an offset.
    if let Some(row) = view.highlighted_row() {
        view.menu_scroll.scroll_to_item(row, ScrollStrategy::Nearest);
    }

    Some(
        div()
            .absolute()
            .top(px(top))
            .left(px(left))
            .max_h(px(max_height))
            .flex()
            .flex_col()
            // Swallows clicks and the wheel, so a menu never leaks through to
            // the image behind it, and keeps its own presses from reaching the
            // root's dismiss-on-outside-click handler.
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(dropdown(menu, view, max_height - 8., cx))
            .into_any_element(),
    )
}

fn dropdown(
    menu: Menu,
    view: &ViewerApp,
    available: f32,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    // The check marks depend on the view's current state, which `menu_items`
    // does not have; they are filled in here.
    let mut items = menu_items(menu);
    mark_current(view, &mut items);
    let heights: Rc<Vec<gpui::Size<Pixels>>> = Rc::new(
        items
            .iter()
            .map(|item| gpui::size(px(288.), px(item.height())))
            .collect(),
    );
    // A virtual list needs a definite height: it is a scroll viewport, not a box
    // that grows with its content. Natural height when the menu fits, and the
    // available space when it does not, which is when it scrolls.
    let natural: f32 = items.iter().map(|item| item.height()).sum();
    let height = natural.min(available).max(ROW_HEIGHT);
    let highlighted = view.highlighted_row();
    let entity = cx.entity();

    div()
        .id("menu-dropdown")
        .w(px(288.))
        .py_1()
        .rounded(px(8.))
        .overflow_hidden()
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .text_sm()
        .child(
            v_virtual_list(
                entity,
                "menu-rows",
                heights,
                move |view, range, _window, cx| {
                    range
                        .map(|index| {
                            let item = &items[index];
                            let selected = highlighted == Some(index);
                            match item {
                                Item::Command(command) => {
                                    menu_row(*command, command.label(), selected, false, view, cx)
                                        .into_any_element()
                                }
                                Item::Labeled(label, command) => {
                                    menu_row(*command, label, selected, false, view, cx)
                                        .into_any_element()
                                }
                                Item::Checked(label, command, on) => {
                                    menu_row(*command, label, selected, *on, view, cx)
                                        .into_any_element()
                                }
                                Item::Submenu(label, menu) => {
                                    submenu_row(label, *menu, selected, view, cx).into_any_element()
                                }
                                Item::Separator => div()
                                    .mx_2()
                                    .my_1()
                                    .h(px(1.))
                                    .bg(cx.theme().border)
                                    .into_any_element(),
                            }
                        })
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&view.menu_scroll)
            .h(px(height)),
        )
}

/// A menu row, optionally carrying a check.
fn menu_row(
    command: Command,
    label: &'static str,
    selected: bool,
    checked: bool,
    view: &ViewerApp,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let shortcut = command.binding_label().unwrap_or_default();
    // A row's availability comes from the view the caller already holds; reading
    // the entity back out of the context here would re-enter it while it is
    // being rendered.
    let enabled = command_is_available(command, view);

    div()
        .id(label)
        .flex()
        .w_full()
        .h(px(ROW_HEIGHT))
        .items_center()
        .px_3()
        .when(selected, |this| this.bg(theme.list_active))
        .when(enabled, |this| this.cursor_pointer())
        .text_color(if enabled {
            theme.popover_foreground
        } else {
            theme.muted_foreground.opacity(0.6)
        })
        .when(enabled, |this| this.hover(|this| this.bg(theme.list_hover)))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                // A fixed-width check column, so labels line up whether or not
                // they are checked.
                .child(
                    div()
                        .w(px(14.))
                        .flex_shrink_0()
                        .when(checked, |this| {
                            this.child(
                                Icon::new(IconName::Check)
                                    .size(px(13.))
                                    .text_color(theme.primary),
                            )
                        }),
                )
                // The label and its shortcut share a text baseline; centring
                // their boxes instead would leave the smaller shortcut sitting
                // a couple of pixels higher than the label.
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_baseline()
                        .justify_between()
                        .child(div().min_w_0().truncate().child(label))
                        .when(!shortcut.is_empty(), |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .pl_2()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(shortcut),
                            )
                        }),
                ),
        )
        .when(enabled, |this| {
            this.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_menu = None;
                this.execute(command, window, cx);
            }))
        })
}

/// A row that opens another dropdown, shown as a nested panel.
fn submenu_row(
    label: &'static str,
    menu: Menu,
    selected: bool,
    _view: &ViewerApp,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
        .id(label)
        .flex()
        .w_full()
        .h(px(ROW_HEIGHT))
        .items_center()
        .px_3()
        .when(selected, |this| this.bg(theme.list_active))
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .justify_between()
                // Same empty check column as the command rows, so the labels
                // line up down the whole menu.
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(14.)).flex_shrink_0())
                        .child(label),
                )
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size(px(12.))
                        .text_color(theme.muted_foreground),
                ),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.open_submenu(menu, cx);
        }))
}

/// Whether a command makes sense right now. Greying out the impossible ones
/// keeps the menu honest.
fn command_is_available(command: Command, view: &ViewerApp) -> bool {
    use Command::*;
    match command {
        CloseImage | SaveResized | CopyImage | CopyResized | CopyPath | CopyFile | CutFile
        | CopyExif | Paste | BookmarkAdd | LaunchEditor | WallpaperDlg | PopupMeta | Print
        | Convert | Rename | DeleteFile | SelectImage => {
            // These need something to act on, except Paste and the pickers.
            matches!(command, Paste | SelectImage | Rename | DeleteFile)
                || view.has_images()
        }
        CopyToPhotoFolder | MoveToPhotoFolder => view.has_images(),
        ZoomIn | ZoomOut | ZoomActual | ZoomReset | ToggleZoomLock => view.has_images(),
        ViewMode100 | ViewModeFitWindow | ViewModeFitWidth | ViewModeFitHeight | ViewModeTwoL2R
        | ViewModeTwoR2L | ViewModeWebtoon | ViewModeNext | ToggleStretchSmall
        | ToggleSeparateCover | ToggleTransparentBg | TransparentGrid | TransparentWhite
        | TransparentBlack | TransparentIgnoreAlpha | TransparentBackgroundColor | ToggleInfo
        | ToggleImagePanel | ToggleSortPanel | ToggleThumbnails | ToggleSlideshowPanel
        | FilterNone | FilterSoften | FilterSoftenSharpen
        | InterpolationBilinear | InterpolationBicubic | InterpolationLanczos | ImageNegative
        | ImageGamma | ImageBrightness | ImageContrast | ImageSaturation | ImageGray
        | ImageSepia | ChannelR | ChannelG | ChannelB | ChannelA | ChannelNext
        | ResetImageProc | NextImage | PrevImage | FirstImage | LastImage | NextContainer
        | PrevContainer => view.has_images(),
        _ => true,
    }
}

/// The rows of a menu, in order.
pub fn menu_items(menu: Menu) -> Vec<Item> {
    match menu {
        Menu::File => vec![
            cmd(Command::OpenFile),
            cmd(Command::OpenFolder),
            cmd(Command::NewWindow),
            cmd(Command::OpenRecent),
            Item::Separator,
            cmd(Command::CloseImage),
            cmd(Command::RevealInFolder),
            cmd(Command::Rename),
            cmd(Command::DeleteFile),
            Item::Separator,
            cmd(Command::Print),
            cmd(Command::Convert),
            Item::Labeled("另存为当前视图", Command::SaveResized),
            Item::Separator,
            cmd(Command::Settings),
            cmd(Command::About),
            cmd(Command::Quit),
        ],
        Menu::View => vec![
            check("100%", Command::ViewMode100, false),
            check("适合窗口", Command::ViewModeFitWindow, false),
            check("适合宽度", Command::ViewModeFitWidth, false),
            check("适合高度", Command::ViewModeFitHeight, false),
            check("双页（左→右）", Command::ViewModeTwoL2R, false),
            check("双页（右→左）", Command::ViewModeTwoR2L, false),
            check("Webtoon", Command::ViewModeWebtoon, false),
            Item::Separator,
            cmd(Command::ZoomIn),
            cmd(Command::ZoomOut),
            cmd(Command::ZoomActual),
            cmd(Command::ZoomReset),
            Item::Separator,
            cmd(Command::ToggleStretchSmall),
            cmd(Command::ToggleSeparateCover),
            cmd(Command::ToggleZoomLock),
            cmd(Command::ToggleScrollLock),
            Item::Separator,
            cmd(Command::ToggleTransparentBg),
            cmd(Command::ToggleThumbnails),
            cmd(Command::ToggleBorder),
            cmd(Command::ToggleAnimInfo),
            Item::Separator,
            // 界面模式: one choice instead of the switches and locks it replaced.
            check("固定界面", Command::ChromeFixed, false),
            check("自动隐藏界面", Command::ChromeAuto, false),
            check("沉浸界面", Command::ChromeImmersive, false),
            Item::Separator,
            cmd(Command::ToggleFullscreen),
            cmd(Command::AlwaysOnTop),
        ],
        Menu::Image => vec![
            check("图像与缩放面板", Command::ToggleImagePanel, false),
            Item::Separator,
            cmd(Command::RotationNone),
            cmd(Command::RotationL90),
            cmd(Command::RotationR90),
            cmd(Command::Rotation180),
            cmd(Command::RotateCw),
            cmd(Command::RotateCcw),
            cmd(Command::FlipHorizontal),
            cmd(Command::FlipVertical),
            cmd(Command::ResetTransform),
            Item::Separator,
            check("无滤镜", Command::FilterNone, false),
            check("柔化", Command::FilterSoften, false),
            check("柔化 + 锐化", Command::FilterSoftenSharpen, false),
            Item::Separator,
            check("双线性插值", Command::InterpolationBilinear, false),
            check("双三次插值", Command::InterpolationBicubic, false),
            check("Lanczos 插值", Command::InterpolationLanczos, false),
            Item::Separator,
            cmd(Command::ImageNegative),
            cmd(Command::ImageGamma),
            cmd(Command::ImageBrightness),
            cmd(Command::ImageContrast),
            cmd(Command::ImageSaturation),
            cmd(Command::ImageGray),
            cmd(Command::ImageSepia),
            Item::Separator,
            check("全部通道", Command::ChannelAll, false),
            check("红通道", Command::ChannelR, false),
            check("绿通道", Command::ChannelG, false),
            check("蓝通道", Command::ChannelB, false),
            check("Alpha 通道", Command::ChannelA, false),
            Item::Separator,
            cmd(Command::ResetImageProc),
        ],
        Menu::More => vec![
            sub("复制到剪贴板", Menu::Clipboard),
            cmd(Command::CutFile),
            cmd(Command::Paste),
            cmd(Command::CopyToPhotoFolder),
            cmd(Command::MoveToPhotoFolder),
            cmd(Command::OpenPhotoFolder),
            Item::Separator,
            // The panels that used to have a menu of their own.
            Item::Labeled("文件/EXIF信息", Command::ToggleInfo),
            check("收藏夹", Command::ToggleBookmarkPanel, false),
            check("幻灯片播放", Command::ToggleSlideshowPanel, false),
            // The sort menu and the sort panel were the same thing twice; the
            // panel is the one that also shows the direction and a preview.
            Item::Labeled("排序…", Command::ToggleSortPanel),
            Item::Separator,
            cmd(Command::LaunchEditor),
            cmd(Command::WallpaperDlg),
            cmd(Command::BookmarkAdd),
            Item::Separator,
            // The palette lists every command with its binding, so it is the
            // shortcut reference; the row keeps the name it always had.
            Item::Labeled("快捷键一览", Command::OpenCommandPalette),
        ],
        Menu::Clipboard => vec![
            cmd(Command::CopyImage),
            cmd(Command::CopyResized),
            cmd(Command::CopyPath),
            cmd(Command::CopyFile),
            cmd(Command::CopyExif),
        ],
    }
}

/// Which view mode is currently in force, for the check marks.
///
/// The menu rows are built without the view in hand — `menu_items` is also used
/// for keyboard navigation, where there is no view — so the checks for the
/// view-mode and 界面模式 groups are filled in here, once, by the caller that
/// does have it.
pub fn mark_current(view: &ViewerApp, items: &mut [Item]) {
    for item in items.iter_mut() {
        let Item::Checked(_, command, on) = item else {
            continue;
        };
        *on = match command {
            Command::ViewMode100 => view.view_mode == seepic_core::ViewMode::Actual,
            Command::ViewModeFitWindow => view.view_mode == seepic_core::ViewMode::FitWindow,
            Command::ViewModeFitWidth => view.view_mode == seepic_core::ViewMode::FitWidth,
            Command::ViewModeFitHeight => view.view_mode == seepic_core::ViewMode::FitHeight,
            Command::ViewModeTwoL2R => view.view_mode == seepic_core::ViewMode::TwoPageL2R,
            Command::ViewModeTwoR2L => view.view_mode == seepic_core::ViewMode::TwoPageR2L,
            Command::ViewModeWebtoon => view.view_mode == seepic_core::ViewMode::Webtoon,
            Command::ChromeFixed => view.settings.chrome_mode == ChromeMode::Fixed,
            Command::ChromeAuto => view.settings.chrome_mode == ChromeMode::Auto,
            Command::ChromeImmersive => view.settings.chrome_mode == ChromeMode::Immersive,
            Command::ToggleInfo => view.show_info,
            Command::ToggleImagePanel => view.panel_is(SidePanel::Image),
            Command::ToggleSortPanel => view.panel_is(SidePanel::Sort),
            Command::ToggleBookmarkPanel => view.panel_is(SidePanel::Bookmark),
            Command::ToggleSlideshowPanel => view.panel_is(SidePanel::Slideshow),
            Command::ToggleThumbnails => view.thumbnails_visible(),
            Command::FilterNone => view.filter == seepic_core::filters::FilterMode::None,
            Command::FilterSoften => view.filter == seepic_core::filters::FilterMode::Smooth,
            Command::FilterSoftenSharpen => {
                view.filter == seepic_core::filters::FilterMode::SmoothSharpen
            }
            Command::InterpolationBilinear => {
                view.interpolation == seepic_core::filters::Interpolation::Bilinear
            }
            Command::InterpolationBicubic => {
                view.interpolation == seepic_core::filters::Interpolation::Bicubic
            }
            Command::InterpolationLanczos => {
                view.interpolation == seepic_core::filters::Interpolation::Lanczos
            }
            Command::ChannelAll => view.adjustments.channel == seepic_core::effects::Channel::All,
            Command::ChannelR => view.adjustments.channel == seepic_core::effects::Channel::Red,
            Command::ChannelG => view.adjustments.channel == seepic_core::effects::Channel::Green,
            Command::ChannelB => view.adjustments.channel == seepic_core::effects::Channel::Blue,
            Command::ChannelA => view.adjustments.channel == seepic_core::effects::Channel::Alpha,
            _ => *on,
        };
    }
}
