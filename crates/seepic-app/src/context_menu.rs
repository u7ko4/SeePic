//! The canvas context menu.
//!
//! Rows reuse the command table, so the labels, shortcut hints and behaviour
//! stay in step with the menus and the keyboard. The structure is six groups
//! (browse, display, adjust, output, file, program); hovering a row with a
//! submenu opens it, and moving to another row closes it again.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::Command;
use seepic_core::filters::{FilterMode, Interpolation};
use seepic_core::orientation::Rotation;
use seepic_core::{Point as IvPoint, SortKey, ViewMode};

use crate::app::ViewerApp;

pub const MENU_WIDTH: f32 = 260.;
/// Width of a nested panel. The longest label here is a navigation row plus
/// its chord, so it gets a little more room than the parent.
pub const SUBMENU_WIDTH: f32 = 270.;
/// Height of a clickable row and of a separator.
///
/// Deliberately tighter than the top menus: this menu carries twenty-seven
/// first-level rows and still has to fit a normal window.
pub(crate) const ROW_HEIGHT: f32 = 24.;
pub(crate) const SEPARATOR_HEIGHT: f32 = 9.;
/// Vertical padding of a panel (`py_1`, top and bottom).
const PANEL_PADDING: f32 = 4.;
/// Roughly how tall the main menu is, used to keep it on screen. The exact
/// height is computed for hit testing.
///
/// Nineteen first-level rows: the less common actions live one level down,
/// which keeps the menu inside a normal window instead of running off the
/// bottom.
pub const MENU_HEIGHT: f32 = 540.;

/// One row in the context menu.
enum Row {
    Action {
        label: &'static str,
        command: Command,
        disabled: bool,
    },
    Check {
        label: &'static str,
        command: Command,
        checked: bool,
        disabled: bool,
    },
    Submenu {
        label: &'static str,
        rows: Vec<Row>,
        disabled: bool,
    },
    Separator,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Separator => SEPARATOR_HEIGHT,
            _ => ROW_HEIGHT,
        }
    }
}

fn action(label: &'static str, command: Command, enabled: bool) -> Row {
    Row::Action {
        label,
        command,
        disabled: !enabled,
    }
}

fn check(label: &'static str, command: Command, checked: bool, enabled: bool) -> Row {
    Row::Check {
        label,
        command,
        checked,
        disabled: !enabled,
    }
}

fn submenu(label: &'static str, rows: Vec<Row>, enabled: bool) -> Row {
    Row::Submenu {
        label,
        rows,
        disabled: !enabled,
    }
}

fn separator() -> Row {
    Row::Separator
}

fn rows_for(view: &ViewerApp) -> Vec<Row> {
    let ready = view.has_images();
    vec![
        action(seepic_core::i18n::tr("Previous Image"), Command::PrevImage, ready),
        action(seepic_core::i18n::tr("Next Image"), Command::NextImage, ready),
        submenu(seepic_core::i18n::tr("Navigation"), navigation_rows(ready), ready),
        action(seepic_core::i18n::tr("Close"), Command::CloseImage, ready),
        separator(),
        submenu(seepic_core::i18n::tr("Zoom Tools"), zoom_rows(view, ready), ready),
        submenu(seepic_core::i18n::tr("View Mode"), view_rows(view, ready), ready),
        // Filters and interpolation used to be two rows; both are rarely
        // touched while reading, so they share one submenu.
        submenu(seepic_core::i18n::tr("Display Effects"), display_effect_rows(view, ready), ready),
        check(seepic_core::i18n::tr("EXIF Info"), Command::ToggleInfo, view.show_info, ready),
        separator(),
        // Rotation, effects and the converter are all "adjust this picture";
        // one submenu replaces three first-level rows.
        submenu(seepic_core::i18n::tr("Image Adjustments"), adjust_rows(view, ready), ready),
        separator(),
        action(seepic_core::i18n::tr("Copy to Clipboard"), Command::CopyImage, ready),
        action(seepic_core::i18n::tr("Copy to Photo Folder"), Command::CopyToPhotoFolder, ready),
        // Printing, the editor and the wallpaper setter are occasional
        // destinations rather than part of a page turn.
        submenu(seepic_core::i18n::tr("More Output"), output_rows(ready), ready),
        separator(),
        action(seepic_core::i18n::tr("Open Image"), Command::OpenFile, true),
        action(seepic_core::i18n::tr("Open Folder"), Command::OpenFolder, true),
        // Selection, reveal, the file clipboard and delete are object-level
        // actions; the group keeps the two openers and sort at the top.
        submenu(seepic_core::i18n::tr("File Operations"), file_operation_rows(ready), true),
        submenu(seepic_core::i18n::tr("Sort By"), sort_rows(view, ready), ready),
        separator(),
        action(seepic_core::i18n::tr("Settings"), Command::Settings, true),
        action(seepic_core::i18n::tr("About"), Command::About, true),
        action(seepic_core::i18n::tr("Quit"), Command::Quit, true),
    ]
}

fn navigation_rows(enabled: bool) -> Vec<Row> {
    vec![
        action(seepic_core::i18n::tr("Next Image"), Command::NextImage, enabled),
        action(seepic_core::i18n::tr("Previous Image"), Command::PrevImage, enabled),
        action(seepic_core::i18n::tr("First Image"), Command::FirstImage, enabled),
        action(seepic_core::i18n::tr("Last Image"), Command::LastImage, enabled),
        action(seepic_core::i18n::tr("Forward 10 Images"), Command::Next10, enabled),
        action(seepic_core::i18n::tr("Back 10 Images"), Command::Prev10, enabled),
        action(seepic_core::i18n::tr("Random Forward"), Command::NextRandom, enabled),
        action(seepic_core::i18n::tr("Random Back"), Command::PrevRandom, enabled),
        action(seepic_core::i18n::tr("Next Folder / Archive"), Command::NextContainer, enabled),
        action(seepic_core::i18n::tr("Previous Folder / Archive"), Command::PrevContainer, enabled),
    ]
}

fn zoom_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    vec![
        action(seepic_core::i18n::tr("Zoom In"), Command::ZoomIn, enabled),
        action(seepic_core::i18n::tr("Zoom Out"), Command::ZoomOut, enabled),
        check(seepic_core::i18n::tr("Lock Zoom"), Command::ToggleZoomLock, view.zoom.zoom_locked, enabled),
        action("100%", Command::ZoomActual, enabled),
        action(seepic_core::i18n::tr("Zoom In (1%)"), Command::ZoomIn1, enabled),
        action(seepic_core::i18n::tr("Zoom Out (1%)"), Command::ZoomOut1, enabled),
    ]
}

fn view_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    vec![
        check(
            seepic_core::i18n::tr("Stretch Small Images"),
            Command::ToggleStretchSmall,
            view.settings.stretch_small,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Actual Size (100%)"),
            Command::ViewMode100,
            view.view_mode == ViewMode::Actual,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Fit Window"),
            Command::ViewModeFitWindow,
            view.view_mode == ViewMode::FitWindow,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Fit Width"),
            Command::ViewModeFitWidth,
            view.view_mode == ViewMode::FitWidth,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Two-Page (L→R)"),
            Command::ViewModeTwoL2R,
            view.view_mode == ViewMode::TwoPageL2R,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Two-Page (R→L)"),
            Command::ViewModeTwoR2L,
            view.view_mode == ViewMode::TwoPageR2L,
            enabled,
        ),
    ]
}

fn filter_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    vec![
        check(
            seepic_core::i18n::tr("None"),
            Command::FilterNone,
            view.filter == FilterMode::None,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Soften"),
            Command::FilterSoften,
            view.filter == FilterMode::Smooth,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Soften + Sharpen"),
            Command::FilterSoftenSharpen,
            view.filter == FilterMode::SmoothSharpen,
            enabled,
        ),
    ]
}

fn interpolation_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    vec![
        check(
            seepic_core::i18n::tr("Bilinear (Fast)"),
            Command::InterpolationBilinear,
            view.interpolation == Interpolation::Bilinear,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Bicubic (High Quality)"),
            Command::InterpolationBicubic,
            view.interpolation == Interpolation::Bicubic,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Lanczos (High Quality)"),
            Command::InterpolationLanczos,
            view.interpolation == Interpolation::Lanczos,
            enabled,
        ),
    ]
}

fn rotation_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    let rotation = view.orientation.rotation;
    vec![
        check(
            seepic_core::i18n::tr("None"),
            Command::RotationNone,
            rotation == Rotation::None,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Rotate 90° CCW"),
            Command::RotationL90,
            rotation == Rotation::Cw270,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Rotate 90° CW"),
            Command::RotationR90,
            rotation == Rotation::Cw90,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Rotate 180°"),
            Command::Rotation180,
            rotation == Rotation::Cw180,
            enabled,
        ),
    ]
}

fn effect_rows(enabled: bool) -> Vec<Row> {
    vec![
        action(seepic_core::i18n::tr("Negative"), Command::ImageNegative, enabled),
        action(seepic_core::i18n::tr("Gamma"), Command::ImageGamma, enabled),
        action(seepic_core::i18n::tr("Flip Vertical"), Command::FlipVertical, enabled),
        action(seepic_core::i18n::tr("Flip Horizontal"), Command::FlipHorizontal, enabled),
    ]
}

fn sort_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    let order = view.sort_order;
    vec![
        check(
            seepic_core::i18n::tr("File Name"),
            Command::SortByName,
            order.key == SortKey::Name,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Modified"),
            Command::SortByDateModified,
            order.key == SortKey::DateModified,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Size"),
            Command::SortBySize,
            order.key == SortKey::Size,
            enabled,
        ),
        check(
            seepic_core::i18n::tr("Type"),
            Command::SortByType,
            order.key == SortKey::Type,
            enabled,
        ),
        separator(),
        check(
            seepic_core::i18n::tr("Descending"),
            Command::ToggleSortAscending,
            order.descending,
            enabled,
        ),
    ]
}

/// Filters and interpolation, which share one submenu.
fn display_effect_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    let mut rows = filter_rows(view, enabled);
    rows.push(separator());
    rows.extend(interpolation_rows(view, enabled));
    rows
}

/// Rotation, visual effects and the converter in one submenu.
fn adjust_rows(view: &ViewerApp, enabled: bool) -> Vec<Row> {
    let mut rows = rotation_rows(view, enabled);
    rows.push(separator());
    rows.extend(effect_rows(enabled));
    rows.push(separator());
    rows.push(action(seepic_core::i18n::tr("Batch Image Convert…"), Command::Convert, enabled));
    rows
}

/// The occasional destinations: editor, printer and wallpaper.
fn output_rows(enabled: bool) -> Vec<Row> {
    vec![
        action(seepic_core::i18n::tr("Open in Editor"), Command::LaunchEditor, enabled),
        action(seepic_core::i18n::tr("Print"), Command::Print, enabled),
        action(seepic_core::i18n::tr("Set as Wallpaper..."), Command::WallpaperDlg, enabled),
    ]
}

/// Selection, reveal and the file clipboard, one level below the openers.
fn file_operation_rows(enabled: bool) -> Vec<Row> {
    vec![
        action(seepic_core::i18n::tr("Select Image"), Command::SelectImage, enabled),
        action(seepic_core::i18n::tr("Reveal in Folder"), Command::RevealInFolder, enabled),
        action(seepic_core::i18n::tr("Copy Image File"), Command::CopyFile, enabled),
        action(seepic_core::i18n::tr("Cut Image File"), Command::CutFile, enabled),
        // Pasting an image from the system clipboard, or a file into the
        // photo folder, is useful even with no image open.
        action(seepic_core::i18n::tr("Paste"), Command::Paste, true),
        separator(),
        action(seepic_core::i18n::tr("Delete"), Command::DeleteFile, enabled),
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

/// The open submenu's rectangle, in content-area coordinates.
fn submenu_geometry(
    rows: &[Row],
    index: usize,
    menu_left: f32,
    menu_top: f32,
    window_width: f32,
    content_height: f32,
) -> Option<(f32, f32, f32)> {
    let Row::Submenu { rows: items, .. } = rows.get(index)? else {
        return None;
    };

    let offset = PANEL_PADDING + rows[..index].iter().map(Row::height).sum::<f32>();
    let height = PANEL_PADDING * 2. + items.iter().map(Row::height).sum::<f32>();
    let left = if menu_left + MENU_WIDTH + SUBMENU_WIDTH + 8. <= window_width {
        menu_left + MENU_WIDTH + 2.
    } else {
        (menu_left - SUBMENU_WIDTH - 2.).max(8.)
    };
    let top = (menu_top + offset).min((content_height - height - 8.).max(4.));
    Some((left, top, height))
}

/// Whether a window-space point falls inside the menu or its open submenu.
///
/// Called from the canvas press handler: a click inside either panel must not
/// dismiss the menu before the row receives it.
pub fn contains(
    view: &ViewerApp,
    anchor: IvPoint,
    chrome: f32,
    window_width: f32,
    window_height: f32,
    point: IvPoint,
) -> bool {
    let rows = rows_for(view);
    let (left, top) = menu_origin(anchor, chrome, window_width, window_height);
    let height = PANEL_PADDING * 2. + rows.iter().map(Row::height).sum::<f32>();
    let point_y = point.y - chrome;
    if point.x >= left
        && point.x <= left + MENU_WIDTH
        && point_y >= top
        && point_y <= top + height
    {
        return true;
    }

    let Some(index) = view.context_submenu else {
        return false;
    };
    let Some((sub_left, sub_top, sub_height)) = submenu_geometry(
        &rows,
        index,
        left,
        top,
        window_width,
        window_height - chrome,
    ) else {
        return false;
    };
    point.x >= sub_left
        && point.x <= sub_left + SUBMENU_WIDTH
        && point_y >= sub_top
        && point_y <= sub_top + sub_height
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
    let rows = rows_for(view);
    let (left, top) = menu_origin(anchor, chrome, window_width, window_height);
    let content_height = window_height - chrome;
    let open = view.context_submenu.and_then(|index| {
        submenu_geometry(&rows, index, left, top, window_width, content_height)
            .map(|(sub_left, sub_top, _)| (index, sub_left, sub_top))
    });

    let mut root = div()
        .absolute()
        .top(px(top))
        .left(px(left))
        .w(px(MENU_WIDTH));

    let mut panel = panel_style(cx, MENU_WIDTH).id("context-menu");
    for (index, row) in rows.iter().enumerate() {
        panel = panel.child(top_row(row, index, view, cx));
    }
    root = root.child(panel);

    if let Some((index, sub_left, sub_top)) = open {
        if let Some(Row::Submenu { rows: items, .. }) = rows.get(index) {
            let mut sub_panel = panel_style(cx, SUBMENU_WIDTH)
                .id("context-submenu")
                .absolute()
                .top(px(sub_top - top))
                .left(px(sub_left - left));
            for (sub_index, row) in items.iter().enumerate() {
                sub_panel = sub_panel.child(sub_row(row, index, sub_index, cx));
            }
            root = root.child(sub_panel);
        }
    }

    root
}

fn panel_style(cx: &Context<ViewerApp>, width: f32) -> Div {
    let theme = cx.theme().clone();
    v_flex()
        .w(px(width))
        .py_1()
        .rounded(px(9.))
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(theme.border)
        .shadow_2xl()
        .text_sm()
}

/// A top-level row, including the hover/click behaviour of a submenu parent.
fn top_row(row: &Row, index: usize, view: &ViewerApp, cx: &mut Context<ViewerApp>) -> AnyElement {
    match row {
        Row::Separator => separator_line(cx),
        Row::Submenu {
            label,
            disabled,
            ..
        } => {
            let active = view.context_submenu == Some(index);
            submenu_parent(*label, index, active, !disabled, cx)
        }
        Row::Action {
            label,
            command,
            disabled,
        } => menu_row(
            (*label).to_string(),
            *label,
            *command,
            false,
            *disabled,
            true,
            cx,
        ),
        Row::Check {
            label,
            command,
            checked,
            disabled,
        } => menu_row(
            (*label).to_string(),
            *label,
            *command,
            *checked,
            *disabled,
            true,
            cx,
        ),
    }
}

/// A row inside the open submenu. There are no nested submenus below this
/// level, so a row is either an action or a check.
fn sub_row(
    row: &Row,
    parent: usize,
    index: usize,
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    match row {
        Row::Separator => separator_line(cx),
        Row::Action {
            label,
            command,
            disabled,
        } => menu_row(
            format!("ctx-sub-{parent}-{index}"),
            *label,
            *command,
            false,
            *disabled,
            false,
            cx,
        ),
        Row::Check {
            label,
            command,
            checked,
            disabled,
        } => menu_row(
            format!("ctx-sub-{parent}-{index}"),
            *label,
            *command,
            *checked,
            *disabled,
            false,
            cx,
        ),
        Row::Submenu { .. } => separator_line(cx),
    }
}

pub(crate) fn separator_line(cx: &Context<ViewerApp>) -> AnyElement {
    // A fixed-height row with the line centred in it. Margins on the line
    // itself collapse differently inside the top menu's virtual list, which
    // pushed the line towards one side of the gap.
    div()
        .w_full()
        .h(px(SEPARATOR_HEIGHT))
        .flex()
        .items_center()
        .child(
            div()
                .mx_2()
                .flex_1()
                .h(px(1.))
                .bg(cx.theme().border),
        )
        .into_any_element()
}

fn menu_row(
    id: String,
    label: &'static str,
    command: Command,
    checked: bool,
    disabled: bool,
    close_submenu_on_hover: bool,
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let shortcut = command.binding_label().unwrap_or_default();
    let enabled = !disabled;

    div()
        .id(id)
        .flex()
        .w_full()
        .h(px(ROW_HEIGHT))
        .items_center()
        .px_3()
        .text_color(if enabled {
            theme.popover_foreground
        } else {
            theme.muted_foreground.opacity(0.5)
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.list_hover))
        })
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
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
            this.on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                if close_submenu_on_hover
                    && *hovered
                    && this.context_submenu.is_some()
                {
                    this.context_submenu = None;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.context_menu = None;
                this.context_submenu = None;
                this.execute(command, window, cx);
            }))
        })
        .into_any_element()
}

fn submenu_parent(
    label: &'static str,
    index: usize,
    active: bool,
    enabled: bool,
    cx: &mut Context<ViewerApp>,
) -> AnyElement {
    let theme = cx.theme().clone();

    div()
        .id(format!("ctx-submenu-{index}"))
        .flex()
        .w_full()
        .h(px(ROW_HEIGHT))
        .items_center()
        .px_3()
        .text_color(if enabled {
            theme.popover_foreground
        } else {
            theme.muted_foreground.opacity(0.5)
        })
        .when(active, |this| this.bg(theme.list_active))
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.list_hover))
                .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                    if *hovered && this.context_submenu != Some(index) {
                        this.context_submenu = Some(index);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    if this.context_submenu != Some(index) {
                        this.context_submenu = Some(index);
                        cx.notify();
                    }
                }))
        })
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(div().w(px(14.)).flex_shrink_0())
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .justify_between()
                        .child(div().min_w_0().truncate().child(label))
                        .child(
                            Icon::new(IconName::ChevronRight)
                                .size(px(12.))
                                .text_color(theme.muted_foreground),
                        ),
                ),
        )
        .into_any_element()
}
