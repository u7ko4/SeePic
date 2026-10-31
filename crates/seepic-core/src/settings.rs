//! Persistent user configuration.
//!
//! Everything the settings window can change lives here, in one serialisable
//! struct. The viewer copies the fields it needs at startup and whenever a
//! setting changes, so the file is the single source of truth between runs.
//!
//! Settings live in one JSON file carrying a schema version, so a later
//! release can migrate an older configuration instead of silently
//! reinterpreting it.

use std::path::PathBuf;

use crate::command::Command;
use crate::library::{BoundaryAction, SortOrder};
use crate::slideshow::SlideshowConfig;
use crate::pages::ViewMode;
use crate::transparency::{BackgroundMode, Transparency};

/// What the mouse wheel does when there is no modifier held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WheelAction {
    /// Previous/next image.
    #[default]
    Page,
    /// Scroll the image when it is larger than the window.
    Scroll,
    /// Zoom in and out.
    Zoom,
}

impl WheelAction {
    pub const ALL: [WheelAction; 3] = [WheelAction::Page, WheelAction::Scroll, WheelAction::Zoom];

    pub fn label(self) -> &'static str {
        match self {
            WheelAction::Page => "翻页",
            WheelAction::Scroll => "滚动",
            WheelAction::Zoom => "缩放",
        }
    }
}

/// How the window's own controls are shown.
///
/// The top bar and the status bar are one decision, because every combination
/// of the five switches this replaced was either the same thing or nonsense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ChromeMode {
    /// Each bar takes a row of its own, and the image sits between them.
    #[default]
    Fixed,
    /// Both bars float over the image and appear as the pointer nears the edge
    /// they belong to, so the image gets the whole window the rest of the time.
    Auto,
    /// Both bars stay hidden. Only a way out is offered, and only while the
    /// pointer is at the top.
    Immersive,
}

impl ChromeMode {
    pub const ALL: [ChromeMode; 3] = [ChromeMode::Fixed, ChromeMode::Auto, ChromeMode::Immersive];

    pub fn label(self) -> &'static str {
        match self {
            ChromeMode::Fixed => "固定",
            ChromeMode::Auto => "自动隐藏",
            ChromeMode::Immersive => "沉浸",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ChromeMode::Fixed => "顶栏与状态栏各占一行，图像在中间",
            ChromeMode::Auto => "两条栏浮在图像上，指针靠近边缘时出现",
            ChromeMode::Immersive => "两条栏都隐藏，图像占满窗口",
        }
    }

    /// Whether the bars take their own rows, shrinking the image area.
    pub fn takes_rows(self) -> bool {
        matches!(self, ChromeMode::Fixed)
    }

    /// Whether the bars float over the image when the pointer nears an edge.
    pub fn reveals_on_hover(self) -> bool {
        matches!(self, ChromeMode::Auto)
    }
}

/// How the thumbnails are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThumbnailView {
    /// A grid over the whole window, opened on demand.
    #[default]
    Grid,
    /// The strip docked under the image.
    Strip,
    /// Not shown at all.
    Off,
}

impl ThumbnailView {
    pub const ALL: [ThumbnailView; 3] = [
        ThumbnailView::Grid,
        ThumbnailView::Strip,
        ThumbnailView::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThumbnailView::Grid => "网格浮层",
            ThumbnailView::Strip => "底部胶片条",
            ThumbnailView::Off => "不显示",
        }
    }

    /// Whether the strip takes a row under the image.
    pub fn takes_row(self) -> bool {
        matches!(self, ThumbnailView::Strip)
    }

    pub fn is_off(self) -> bool {
        matches!(self, ThumbnailView::Off)
    }
}

/// The colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "浅色",
            ThemeMode::Dark => "深色",
            ThemeMode::System => "跟随系统",
        }
    }
}

/// Which mouse button a custom action hangs off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MouseSlot {
    Left,
    DoubleLeft,
    Middle,
    Right,
    X,
    Y,
}

impl MouseSlot {
    pub const ALL: [MouseSlot; 6] = [
        MouseSlot::Left,
        MouseSlot::DoubleLeft,
        MouseSlot::Middle,
        MouseSlot::Right,
        MouseSlot::X,
        MouseSlot::Y,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MouseSlot::Left => "左键",
            MouseSlot::DoubleLeft => "双击左键",
            MouseSlot::Middle => "中键",
            MouseSlot::Right => "右键",
            MouseSlot::X => "侧键 X",
            MouseSlot::Y => "侧键 Y",
        }
    }

    /// The action bound by default.
    pub fn default_command(self) -> Option<Command> {
        match self {
            MouseSlot::DoubleLeft => Some(Command::Maximize),
            _ => None,
        }
    }
}

/// A mouse button bound to a command.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MouseBinding {
    pub slot: MouseSlot,
    pub enabled: bool,
    /// The command's [`Command::id`], so the config survives renames of the
    /// user-facing label.
    pub command: Option<String>,
}

impl MouseBinding {
    pub fn command(&self) -> Option<Command> {
        if !self.enabled {
            return None;
        }
        self.command.as_deref().and_then(Command::from_id)
    }
}

/// A keyboard shortcut the user added or rebound.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeyBinding {
    pub enabled: bool,
    /// Canonical chord, e.g. `ctrl-shift-p`.
    pub binding: String,
    pub command: String,
}

impl KeyBinding {
    pub fn command(&self) -> Option<Command> {
        if !self.enabled {
            return None;
        }
        Command::from_id(&self.command)
    }
}

/// Zoom applied when an image is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ZoomPreset {
    /// Fit the window, scaling up only if stretching is enabled.
    #[default]
    Auto,
    Actual,
    Percent100,
    Percent125,
    Percent150,
    Percent175,
    Percent200,
    Percent250,
}

impl ZoomPreset {
    pub const ALL: [ZoomPreset; 8] = [
        ZoomPreset::Auto,
        ZoomPreset::Actual,
        ZoomPreset::Percent100,
        ZoomPreset::Percent125,
        ZoomPreset::Percent150,
        ZoomPreset::Percent175,
        ZoomPreset::Percent200,
        ZoomPreset::Percent250,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ZoomPreset::Auto => "自动",
            ZoomPreset::Actual => "原始大小",
            ZoomPreset::Percent100 => "100%",
            ZoomPreset::Percent125 => "125%",
            ZoomPreset::Percent150 => "150%",
            ZoomPreset::Percent175 => "175%",
            ZoomPreset::Percent200 => "200%",
            ZoomPreset::Percent250 => "250%",
        }
    }

    /// The factor for this preset, or `None` when it means "fit the window".
    pub fn factor(self) -> Option<f32> {
        match self {
            ZoomPreset::Auto => None,
            ZoomPreset::Actual | ZoomPreset::Percent100 => Some(1.0),
            ZoomPreset::Percent125 => Some(1.25),
            ZoomPreset::Percent150 => Some(1.5),
            ZoomPreset::Percent175 => Some(1.75),
            ZoomPreset::Percent200 => Some(2.0),
            ZoomPreset::Percent250 => Some(2.5),
        }
    }
}

/// The settings groups shown in the window.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---- 常规 -----------------------------------------------------------
    pub esc_to_exit: bool,
    pub single_instance: bool,
    pub confirm_delete: bool,
    pub always_on_top: bool,
    pub snap_windows: bool,
    pub boundary_action: BoundaryAction,
    pub archive_boundary_action: BoundaryAction,

    // ---- 浏览 -----------------------------------------------------------
    /// How the top bar and the status bar are shown.
    ///
    /// One choice replaces what used to be five separate switches plus two
    /// "lock" flags, which could be combined into states that made no sense
    /// (a menu bar taking its own row while the control bar floated, say).
    pub chrome_mode: ChromeMode,
    /// Whether the icon tool group is drawn in the top bar.
    ///
    /// Off by default: the same commands are in the menus, the status bar and
    /// the keyboard, so the bar stays quiet unless the user wants the shortcuts.
    pub show_toolbar: bool,
    /// Which way the thumbnail strip is shown, if at all.
    pub thumbnail_view: ThumbnailView,
    pub show_border: bool,
    pub show_scrollbar: bool,
    pub show_side_arrows: bool,
    pub show_anim_info: bool,
    pub show_message: bool,
    /// The default template: `${folder} ${sep} ${file} ${page}/${total}`.
    pub title_format: String,
    pub theme_mode: ThemeMode,
    pub background_mode: BackgroundMode,
    pub background_color: String,
    /// A picture to tile behind the image, when the mode is Custom.
    pub background_image: Option<String>,
    pub border_color: String,
    pub zoom_preset: ZoomPreset,
    pub dpi_scale_image: bool,
    pub stretch_small: bool,

    // ---- superseded, read only to migrate ---------------------------------
    /// The lock flags that [`Settings::chrome_mode`] replaced, and the thumbnail
    /// switch that [`Settings::thumbnail_view`] replaced.
    ///
    /// Deserialize-only, so a config written before schema 6 can be brought
    /// forward instead of silently reverting to the defaults. `None` means the
    /// config predates the field entirely. Never written back out.
    #[serde(default, skip_serializing)]
    pub(crate) menu_bar_locked: Option<bool>,
    #[serde(default, skip_serializing)]
    pub(crate) control_bar_locked: Option<bool>,
    #[serde(default, skip_serializing)]
    pub(crate) show_thumbnails: Option<bool>,

    // ---- 幻灯片 ---------------------------------------------------------
    /// How the slideshow plays, saved from its panel so it survives a restart.
    pub slideshow: SlideshowConfig,

    // ---- 性能 -----------------------------------------------------------
    /// Keep decoded frames in mapped temporary files — the process's swap
    /// pages — instead of the heap.
    ///
    /// The bytes are the same either way; what changes is whose they are. On
    /// the heap they are private pages, which is what the task manager shows
    /// and what the OS cannot take back under pressure. In a mapped file they
    /// are file pages: still resident, but reclaimable, and written back to the
    /// file only when their slot is released.
    pub swap_frames: bool,
    /// How many slot files the swap store keeps. Fixed, so the number of
    /// temporary files does not grow with the size of the directory.
    pub swap_slots: u32,
    /// Capacity of one slot, in megabytes. A frame larger than this gets a
    /// file of its own for as long as it is held.
    pub swap_slot_mb: u32,
    /// Ceiling on the bytes mapped at once, in megabytes: the disk the swap
    /// store may reserve while the viewer is open.
    pub swap_capacity_mb: u32,
    /// Longest side a reduced decode may keep, in pixels.
    ///
    /// This is what bounds a single frame: at 2560 the largest one is about
    /// 25 MB, at 1920 about 14 MB, and the cost of the lower figure is
    /// sharpness when zooming to 100% (which re-decodes in full).
    pub decode_max_edge: u32,
    /// How many neighbours either side of the current image to decode ahead.
    pub prefetch_window: u32,
    /// How many decodes may run at once, the page on screen included.
    ///
    /// One keeps the visible decode alone: every neighbour waits for it, so a
    /// held key advances at one decode per page. Two lets a prefetch decode
    /// beside the visible one, which is what keeps a held key at the rate of its
    /// own decode; the price is a second decode transient at the same time (a
    /// 20-megapixel progressive file is over 100 MB on its own).
    pub decode_workers: u32,
    /// How many pages ahead of the cursor to decode while the viewer is idle.
    ///
    /// This is the depth that makes the first hold of the arrow key cheap: each
    /// prepared page is a cache hit instead of a decode. Zero turns it off.
    pub prepare_pages: u32,
    /// How many decoded pages the cache may keep.
    ///
    /// The memory budget still bounds the bytes; this bounds the count, so a
    /// folder of small pages cannot fill the cache with hundreds of thumbnails
    /// and push out the pages around the cursor. With the swap store on, the
    /// depth is also capped by its slot count — the store holds at most one
    /// frame per slot.
    pub cache_pages: u32,

    // ---- 图像处理 -------------------------------------------------------
    pub exif_rotate: bool,
    pub cache_next: bool,
    /// The total pixel budget, in megabytes.
    ///
    /// One figure covers everything the viewer holds in pixels: decoded images,
    /// the textures built from them, and the downscale pyramids. Split between
    /// the two caches by [`Self::decode_budget`] and [`Self::render_budget`].
    pub memory_budget_mb: u32,
    pub low_quality_preview: bool,
    pub use_image_icc: bool,
    pub use_monitor_icc: bool,
    pub transparency: Transparency,
    pub webtoon_gap: u32,
    pub webtoon_gap_enabled: bool,
    pub two_page_gap: u32,
    pub two_page_gap_enabled: bool,
    pub separate_cover: bool,
    pub webtoon_width: u32,
    pub clip_layers: bool,
    pub raw_jpeg_thumbnail: bool,
    pub center_if_larger: bool,
    pub view_mode: ViewMode,
    /// How the image list is ordered, saved from the sort panel.
    ///
    /// A folder or archive opened later — and the next run of the viewer —
    /// starts in the order the reader last chose, rather than reverting to
    /// name order.
    pub sort_order: SortOrder,

    // ---- 键盘 -----------------------------------------------------------
    pub arrow_scroll_only: bool,
    pub custom_keys: Vec<KeyBinding>,

    // ---- 鼠标 -----------------------------------------------------------
    pub mouse_bindings: Vec<MouseBinding>,
    pub wheel_action: WheelAction,
    pub wheel_speed: u8,
    pub wheel_bounce: bool,
    pub click_edges_to_page: bool,
    pub click_canvas_shows_chrome: bool,

    // ---- 收藏夹 ---------------------------------------------------------
    pub autobookmark: bool,
    pub share_settings: bool,
    pub max_bookmarks: u32,
    pub bookmark_folder: String,
    pub remember_archive_position: bool,
    pub remember_folder_position: bool,

    // ---- 快捷菜单（外壳） -----------------------------------------------
    pub shell_menu_enable: bool,
    pub shell_menu_viewer: bool,
    pub shell_menu_convert: bool,
    pub shell_menu_convert_shift: bool,
    pub shell_menu_on_folder: bool,
    pub shell_menu_on_background: bool,
    pub shell_preview_icon: bool,

    // ---- 图片文件夹 -----------------------------------------------------
    /// Where the copy/move commands put files.
    ///
    /// The old schema had two; the second is gone, and the first keeps its value
    /// through the alias below.
    #[serde(alias = "photo_folder_1")]
    pub photo_folder: String,

    // ---- 杂项 -----------------------------------------------------------
    /// Reopen the main window at the size it was last windowed at.
    pub remember_window_size: bool,
    /// The last windowed size, in logical pixels. Only read when
    /// [`Settings::remember_window_size`] is on.
    pub window_width: u32,
    pub window_height: u32,
    pub resize_window_to_image: bool,
    pub disable_screensaver: bool,
    pub fix_new_window_pos: bool,
    pub video_output: String,
    pub page_turn_sound: bool,
    /// How long a transient message (the toast in the corner) stays on screen.
    ///
    /// Short by default: a toast acknowledges an action, and one that lingers
    /// is in the way of the picture.
    pub toast_seconds: u32,
    pub use_user_temp_folder: bool,
    pub temp_folder: String,
    pub sync_to_folder: bool,
    pub do_not_remember_recent: bool,
    pub external_editor: String,
    pub use_image_whitelist: bool,
    pub image_whitelist: String,
    pub use_image_blacklist: bool,
    pub image_blacklist: String,

    // ---- 语言 -----------------------------------------------------------
    pub language: String,

    /// Bumped when the shape of this struct changes, so migrations can run.
    pub schema_version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            esc_to_exit: true,
            single_instance: false,
            confirm_delete: true,
            always_on_top: false,
            snap_windows: false,
            boundary_action: BoundaryAction::Wrap,
            archive_boundary_action: BoundaryAction::NextContainer,

            chrome_mode: ChromeMode::Fixed,
            show_toolbar: false,
            thumbnail_view: ThumbnailView::Grid,
            show_border: true,
            show_scrollbar: true,
            show_side_arrows: true,
            show_anim_info: true,
            show_message: true,
            title_format: "${page}/${total}  ${folder} > ${file}".to_string(),
            theme_mode: ThemeMode::Dark,
            background_mode: BackgroundMode::Custom,
            background_color: "#1a1a1a".to_string(),
            background_image: None,
            border_color: "#444444".to_string(),
            zoom_preset: ZoomPreset::Auto,
            dpi_scale_image: true,
            stretch_small: false,
            menu_bar_locked: None,
            control_bar_locked: None,
            show_thumbnails: None,

            slideshow: SlideshowConfig::default(),
            exif_rotate: true,
            cache_next: true,
            swap_frames: true,
            swap_slots: 8,
            swap_slot_mb: 16,
            swap_capacity_mb: 128,
            decode_workers: 2,
            prepare_pages: 16,
            cache_pages: 24,
            decode_max_edge: 2560,
            prefetch_window: 2,
            // Enough for the pages either side of the cursor at display
            // resolution plus their textures, which is what makes a page turn
            // instant. The split is a third for decoded pixels and two thirds
            // for the built textures a page turn actually draws.
            memory_budget_mb: 192,
            low_quality_preview: true,
            use_image_icc: true,
            use_monitor_icc: false,
            transparency: Transparency::BackgroundColor,
            webtoon_gap: 10,
            webtoon_gap_enabled: false,
            two_page_gap: 10,
            two_page_gap_enabled: false,
            separate_cover: false,
            webtoon_width: 100,
            clip_layers: false,
            raw_jpeg_thumbnail: false,
            center_if_larger: false,
            view_mode: ViewMode::FitWindow,
            sort_order: SortOrder::default(),

            arrow_scroll_only: false,
            custom_keys: Vec::new(),

            mouse_bindings: MouseSlot::ALL
                .iter()
                .map(|slot| MouseBinding {
                    slot: *slot,
                    enabled: slot.default_command().is_some(),
                    command: slot.default_command().map(|c| c.id().to_string()),
                })
                .collect(),
            wheel_action: WheelAction::Page,
            wheel_speed: 3,
            wheel_bounce: true,
            click_edges_to_page: false,
            click_canvas_shows_chrome: false,

            autobookmark: false,
            share_settings: false,
            max_bookmarks: 100,
            bookmark_folder: String::new(),
            remember_archive_position: true,
            remember_folder_position: false,

            shell_menu_enable: false,
            shell_menu_viewer: true,
            shell_menu_convert: true,
            shell_menu_convert_shift: false,
            shell_menu_on_folder: true,
            shell_menu_on_background: true,
            shell_preview_icon: false,

            photo_folder: Self::default_photo_folder(),

            resize_window_to_image: false,
            remember_window_size: true,
            window_width: DEFAULT_WINDOW_SIZE.0 as u32,
            window_height: DEFAULT_WINDOW_SIZE.1 as u32,
            disable_screensaver: false,
            fix_new_window_pos: false,
            video_output: "DirectX12".to_string(),
            page_turn_sound: false,
            toast_seconds: 1,
            use_user_temp_folder: false,
            temp_folder: String::new(),
            sync_to_folder: false,
            do_not_remember_recent: false,
            external_editor: String::new(),
            use_image_whitelist: false,
            image_whitelist: String::new(),
            use_image_blacklist: false,
            image_blacklist: String::new(),

            language: "zh-CN".to_string(),
            schema_version: SCHEMA_VERSION,
        }
    }
}

/// The current config shape.
pub const SCHEMA_VERSION: u32 = 10;

/// The window size used the first time, and whenever remembering the size is off.
pub const DEFAULT_WINDOW_SIZE: (f32, f32) = (1280., 860.);

/// The floor the main window enforces, matching the window manager's minimum.
pub const MIN_WINDOW_SIZE: (f32, f32) = (360., 400.);

/// A ceiling for a remembered size, only to reject nonsense from a hand-edited
/// config; no real display is this large.
const MAX_WINDOW_EDGE: f32 = 16_384.;

/// The smallest and largest memory budget the settings offer, in megabytes.
pub const MEMORY_BUDGET_RANGE: (u32, u32) = (64, 4096);

/// The budget options the settings page offers, in megabytes.
///
/// 192 is the default (64 MB of decoded pixels and 128 MB of textures), so it
/// is offered alongside the doublings.
pub const MEMORY_BUDGET_CHOICES: [u32; 7] = [64, 128, 192, 256, 512, 1024, 2048];

/// Ceiling on the built-texture cache, in megabytes.
///
/// About four full-size pages: enough that stepping back a page or two reuses a
/// texture, small enough that the cache's CPU side (which GPUI never releases)
/// stays in the tens of megabytes. See [`Settings::render_budget`].
pub const RENDER_CACHE_CAP_MB: u32 = 64;

/// The title format used before schema 3, so a config that never changed it can
/// be brought forward.
const TITLE_FORMAT_V2: &str = "${folder} ${sep} ${file} [${page}/${total}]";

/// The title format used before schema 8: container and file first, then the
/// counter. Only rewritten when it is still the stock string.
const TITLE_FORMAT_V7: &str = "${folder} ${sep} ${file} ${page}/${total}";

/// The command id a stored binding should now point at, if it changed.
///
/// `None` means the command no longer exists and the binding is dropped.
fn renamed_command_id(stored: &str) -> Option<String> {
    match stored {
        "CopyToPhotoFolder1" => Some("CopyToPhotoFolder".to_string()),
        "MoveToPhotoFolder1" => Some("MoveToPhotoFolder".to_string()),
        "OpenPhotoFolder1" => Some("OpenPhotoFolder".to_string()),
        // The second photo folder was removed.
        "CopyToPhotoFolder2" | "MoveToPhotoFolder2" | "OpenPhotoFolder2" => None,
        other => Some(other.to_string()),
    }
}

impl Settings {
    /// The standard folder for copied and moved pictures.
    ///
    /// `%USERPROFILE%\Pictures\SeePic`. A missing profile yields an empty
    /// string, which the commands treat as "ask the user to choose" rather
    /// than inventing a location somewhere unrelated.
    pub fn default_photo_folder() -> String {
        let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
            return String::new();
        };
        home.join("Pictures")
            .join("SeePic")
            .display()
            .to_string()
    }

    /// The folder the app keeps its own files in.
    ///
    /// `%APPDATA%\SeePic`, falling back to the working directory when the
    /// variable is missing (portable use).
    pub fn data_dir() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("SeePic")
    }

    /// Where the config file lives.
    pub fn path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    /// The size to open the main window at.
    ///
    /// A missing, zero or absurd pair falls back to the default rather than
    /// opening a window that cannot be seen or used.
    pub fn initial_window_size(&self) -> (f32, f32) {
        if !self.remember_window_size {
            return DEFAULT_WINDOW_SIZE;
        }
        let width = self.window_width as f32;
        let height = self.window_height as f32;
        if !(MIN_WINDOW_SIZE.0..=MAX_WINDOW_EDGE).contains(&width)
            || !(MIN_WINDOW_SIZE.1..=MAX_WINDOW_EDGE).contains(&height)
        {
            return DEFAULT_WINDOW_SIZE;
        }
        (width, height)
    }

    /// Load from disk, falling back to defaults for anything missing or broken.
    ///
    /// A corrupt config must never stop the app from starting, so parse errors
    /// are swallowed here and surface as defaults.
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            return Self::default();
        };
        let mut settings: Self = Self::parse(&text).unwrap_or_default();
        settings.migrate();
        settings
    }

    /// Parse a config, tolerating a byte-order mark.
    ///
    /// Editors on Windows write one by default, and a config that came from one
    /// would otherwise fail to parse and silently reset every setting — which
    /// is exactly the kind of thing a measurement run cannot afford.
    fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
    }

    /// Bring an older config forward.
    ///
    /// "A missing key means the default" works until a default changes meaning,
    /// which is why the schema version is recorded here even when there is
    /// nothing to migrate yet.
    pub fn migrate(&mut self) {
        if self.schema_version == 0 {
            self.schema_version = 1;
        }
        if self.schema_version < 4 {
            // `show_border` did nothing until schema 4 and always read false;
            // the viewer drew a border regardless. Carry that visible state
            // forward rather than making every existing config lose it.
            self.show_border = true;
        }
        if self.schema_version < 5 {
            // `cache_mb` was the decode cache alone. The budget now also covers
            // the built textures, so a config written before this schema gets a
            // total that leaves its decode cache the size it already was.
            let previous = self.memory_budget_mb.max(1);
            let doubled = previous.saturating_mul(2);
            self.memory_budget_mb = doubled.clamp(MEMORY_BUDGET_RANGE.0, MEMORY_BUDGET_RANGE.1);
        }
        if self.schema_version < 6 {
            // Five switches and two lock flags became one choice. The locks are
            // the closest thing the old shape had to "does the chrome take its
            // own space", so they decide between the two modes that have bars.
            let locked = match (self.menu_bar_locked, self.control_bar_locked) {
                (Some(a), Some(b)) => a || b,
                (Some(a), None) | (None, Some(a)) => a,
                // Neither field present: the old default was both locked.
                (None, None) => true,
            };
            self.chrome_mode = if locked {
                ChromeMode::Fixed
            } else {
                ChromeMode::Auto
            };
            // The old switch was off by default, so a config that never turned
            // it on gets no strip rather than the new grid overlay.
            self.thumbnail_view = match self.show_thumbnails {
                Some(true) => ThumbnailView::Strip,
                _ => ThumbnailView::Off,
            };
        }
        if self.schema_version < 3 && self.title_format == TITLE_FORMAT_V2 {
            // Only the stock format is rewritten; a customised one is the
            // user's text and is left alone.
            self.title_format = Self::default().title_format;
        }
        if self.schema_version < 7 {
            // The second photo folder is gone, and the first one's commands lost
            // their suffix. A binding that pointed at the second folder has
            // nothing left to run, so it is dropped; one that pointed at the
            // first is moved to the command's new id.
            for binding in &mut self.custom_keys {
                if let Some(moved) = renamed_command_id(&binding.command) {
                    binding.command = moved;
                }
            }
            for binding in &mut self.mouse_bindings {
                if let Some(id) = binding.command.as_deref() {
                    binding.command = renamed_command_id(id);
                }
            }
        }
        if self.schema_version < 8 && self.title_format == TITLE_FORMAT_V7 {
            // The counter moved to the front and the separator became ">".
            self.title_format = Self::default().title_format;
        }
        // An empty folder used to mean "not configured"; a standard location
        // is more useful, and the copy/move commands create it on first use.
        if self.photo_folder.trim().is_empty() {
            self.photo_folder = Self::default_photo_folder();
        }
        if self.schema_version < SCHEMA_VERSION {
            self.schema_version = SCHEMA_VERSION;
        }
    }

    /// The share of the budget the decoded-pixel cache may use.
    ///
    /// A third, against two thirds for the built textures. The decoded pixels
    /// are only needed again if the transform changes or the view zooms past
    /// what the texture holds, and either of those can re-decode; the built
    /// textures are what a page turn actually needs, so they get the larger
    /// share.
    /// The swap store's configuration, from the 性能 page.
    pub fn swap_config(&self) -> crate::swap::SwapConfig {
        crate::swap::SwapConfig::from_mb(
            self.swap_frames,
            self.swap_slots,
            self.swap_slot_mb,
            self.swap_capacity_mb,
        )
    }

    /// The longest side a reduced decode may keep.
    ///
    /// Clamped to something a decoder can act on, so a hand-edited config
    /// cannot ask for a 4-pixel image or an unbounded buffer.
    pub fn decode_max_edge(&self) -> u32 {
        self.decode_max_edge.clamp(512, 8192)
    }

    /// How long a toast stays on screen.
    ///
    /// Clamped so a hand-edited config cannot make a message disappear before
    /// it can be read, or stay for an hour.
    pub fn toast_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.toast_seconds.clamp(1, 30)))
    }

    /// How many decodes may run at once, clamped to something a thread pool and
    /// a memory budget can live with.
    pub fn decode_workers(&self) -> u32 {
        self.decode_workers.clamp(1, 4)
    }

    /// How many pages the idle preparation decodes ahead.
    pub fn prepare_pages(&self) -> u32 {
        self.prepare_pages.min(64)
    }

    /// How many decoded pages the cache may keep.
    pub fn cache_pages(&self) -> u32 {
        self.cache_pages.clamp(2, 64)
    }

    pub fn decode_budget(&self) -> usize {
        self.memory_budget_mb as usize * 1024 * 1024 / 3
    }

    /// How much decoded image data the cache may hold.
    ///
    /// The two backends are exclusive, and this is the figure that switches
    /// between them. With the swap store on, decoded pixels are file pages, not
    /// private memory: the cache is sized by what the store can actually hold —
    /// its usable slots, not the ceiling, which may be roomier than the slots
    /// that fit in it — and the memory budget then pays for the built textures
    /// only. With the store off, the cache is the heap and takes the budget's
    /// decode share.
    pub fn decode_cache_capacity(&self) -> usize {
        if self.swap_frames {
            let config = self.swap_config();
            config.usable_slots() * config.slot_bytes
        } else {
            self.decode_budget()
        }
    }

    /// The share of the budget the built-texture cache may use.
    ///
    /// Note that a built image also costs the sprite atlas a texture, which is
    /// larger than the image itself — a 2000x1333 image cannot share a 1024x1024
    /// atlas texture, so it takes a 2048x2048 one. The process total therefore
    /// sits above this figure; what matters is that it is bounded.
    ///
    /// Capped at [`RENDER_CACHE_CAP_MB`]. The cache exists so that coming back
    /// to a page reuses its texture identity instead of committing another one;
    /// it was sized generously when every rebuild leaked a texture, but textures
    /// are returned to the atlas properly now, and a one-page cache measured
    /// flat across sixteen ping-pong rounds with a rebuild of 0.9 ms (0.2 ms for
    /// a burst preview). Depth beyond a few pages therefore buys about a
    /// millisecond and costs tens of megabytes of private memory — each cached
    /// page keeps its CPU pixels as well as its texture, and GPUI has no way to
    /// release the pixels after upload.
    pub fn render_budget(&self) -> usize {
        let share = self.memory_budget_mb as usize * 1024 * 1024 - self.decode_budget();
        share.min(RENDER_CACHE_CAP_MB as usize * 1024 * 1024)
    }

    /// Write to disk, creating the directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }

    /// Export to a JSON file the user picked, so settings can be moved between
    /// machines: JSON plus a schema version, so the file stays readable and
    /// safe to reinterpret.
    pub fn export_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }

    /// Import a previously exported file.
    pub fn import_from(path: &std::path::Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut settings: Self = Self::parse(&text)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "配置无法解析"))?;
        settings.migrate();
        Ok(settings)
    }

    /// The custom action bound to a mouse button, if any.
    pub fn mouse_command(&self, slot: MouseSlot) -> Option<Command> {
        self.mouse_bindings
            .iter()
            .find(|binding| binding.slot == slot)
            .and_then(MouseBinding::command)
    }

    /// The binding shown for `command`: the user's custom one if there is one,
    /// otherwise the built-in default.
    pub fn effective_binding(&self, command: Command) -> Option<String> {
        self.custom_keys
            .iter()
            .find(|key| key.enabled && key.command == command.id())
            .map(|key| key.binding.clone())
            .or_else(|| command.default_binding().map(str::to_string))
    }

    /// Whether `command` has a custom binding.
    pub fn has_custom_binding(&self, command: Command) -> bool {
        self.custom_keys
            .iter()
            .any(|key| key.command == command.id())
    }

    /// Bind `binding` to `command`.
    ///
    /// Any custom binding that command had is replaced, and a binding already
    /// used by another command is taken over from it — the caller is told which
    /// command lost its key so the user can be informed.
    pub fn bind_key(&mut self, command: Command, binding: &str) -> Option<Command> {
        let displaced = self
            .custom_keys
            .iter()
            .find(|key| {
                key.command != command.id() && key.binding.eq_ignore_ascii_case(binding)
            })
            .and_then(|key| Command::from_id(&key.command));
        self.custom_keys.retain(|key| {
            key.command != command.id() && !key.binding.eq_ignore_ascii_case(binding)
        });
        self.custom_keys.push(KeyBinding {
            enabled: true,
            binding: binding.to_string(),
            command: command.id().to_string(),
        });
        displaced
    }

    /// Drop the custom binding for `command`, restoring its default.
    pub fn clear_key(&mut self, command: Command) {
        self.custom_keys.retain(|key| key.command != command.id());
    }

    /// The command a custom binding sends, if `binding` has one.
    pub fn command_for_custom_key(&self, binding: &str) -> Option<Command> {
        self.custom_keys
            .iter()
            .find(|key| key.enabled && key.binding.eq_ignore_ascii_case(binding))
            .and_then(KeyBinding::command)
    }

    /// The configured two-page gap, or zero when gaps are switched off.
    pub fn two_page_gap(&self) -> f32 {
        if self.two_page_gap_enabled {
            self.two_page_gap as f32
        } else {
            0.
        }
    }

    /// The configured webtoon gap, or zero when gaps are switched off.
    pub fn webtoon_gap(&self) -> f32 {
        if self.webtoon_gap_enabled {
            self.webtoon_gap as f32
        } else {
            0.
        }
    }

    /// Reset one group, used by the window's per-page reset button.
    pub fn reset_group(&mut self, group: Group) {
        let defaults = Self::default();
        match group {
            Group::General => {
                self.esc_to_exit = defaults.esc_to_exit;
                self.single_instance = defaults.single_instance;
                self.confirm_delete = defaults.confirm_delete;
                self.boundary_action = defaults.boundary_action;
                self.archive_boundary_action = defaults.archive_boundary_action;
            }
            // Everything about how the window looks and behaves: the chrome, the
            // display toggles, the theme and background, the window itself, and
            // the auto-rotation that decides which way up an image is shown.
            Group::Interface => {
                self.chrome_mode = defaults.chrome_mode;
                self.show_toolbar = defaults.show_toolbar;
                self.thumbnail_view = defaults.thumbnail_view;
                self.show_border = defaults.show_border;
                self.show_scrollbar = defaults.show_scrollbar;
                self.show_side_arrows = defaults.show_side_arrows;
                self.show_anim_info = defaults.show_anim_info;
                self.show_message = defaults.show_message;
                self.click_canvas_shows_chrome = defaults.click_canvas_shows_chrome;
                self.exif_rotate = defaults.exif_rotate;
                self.transparency = defaults.transparency;
                self.title_format = defaults.title_format;
                self.theme_mode = defaults.theme_mode;
                self.background_mode = defaults.background_mode;
                self.background_color = defaults.background_color;
                self.background_image = defaults.background_image;
                self.border_color = defaults.border_color;
                self.zoom_preset = defaults.zoom_preset;
                self.stretch_small = defaults.stretch_small;
                self.always_on_top = defaults.always_on_top;
                self.snap_windows = defaults.snap_windows;
                self.remember_window_size = defaults.remember_window_size;
                self.window_width = defaults.window_width;
                self.window_height = defaults.window_height;
                self.resize_window_to_image = defaults.resize_window_to_image;
                self.fix_new_window_pos = defaults.fix_new_window_pos;
                self.dpi_scale_image = defaults.dpi_scale_image;
            }
            Group::Advanced => {
                self.swap_frames = defaults.swap_frames;
                self.swap_slots = defaults.swap_slots;
                self.swap_slot_mb = defaults.swap_slot_mb;
                self.swap_capacity_mb = defaults.swap_capacity_mb;
                self.decode_max_edge = defaults.decode_max_edge;
                self.prefetch_window = defaults.prefetch_window;
                self.decode_workers = defaults.decode_workers;
                self.prepare_pages = defaults.prepare_pages;
                self.cache_pages = defaults.cache_pages;
                self.cache_next = defaults.cache_next;
                self.memory_budget_mb = defaults.memory_budget_mb;
                self.low_quality_preview = defaults.low_quality_preview;
                self.use_image_icc = defaults.use_image_icc;
                self.use_monitor_icc = defaults.use_monitor_icc;
                self.use_image_whitelist = defaults.use_image_whitelist;
                self.image_whitelist = defaults.image_whitelist;
                self.use_image_blacklist = defaults.use_image_blacklist;
                self.image_blacklist = defaults.image_blacklist;
                self.webtoon_gap = defaults.webtoon_gap;
                self.webtoon_gap_enabled = defaults.webtoon_gap_enabled;
                self.two_page_gap = defaults.two_page_gap;
                self.two_page_gap_enabled = defaults.two_page_gap_enabled;
                self.separate_cover = defaults.separate_cover;
                self.webtoon_width = defaults.webtoon_width;
                self.clip_layers = defaults.clip_layers;
                self.raw_jpeg_thumbnail = defaults.raw_jpeg_thumbnail;
                self.center_if_larger = defaults.center_if_larger;
                self.view_mode = defaults.view_mode;
            }
            Group::FileAssoc => {}
            Group::Keyboard => {
                self.arrow_scroll_only = defaults.arrow_scroll_only;
                self.custom_keys = defaults.custom_keys;
            }
            Group::Mouse => {
                self.mouse_bindings = defaults.mouse_bindings;
                self.wheel_action = defaults.wheel_action;
                self.wheel_speed = defaults.wheel_speed;
                self.wheel_bounce = defaults.wheel_bounce;
                self.click_edges_to_page = defaults.click_edges_to_page;
            }
            Group::SystemMenu => {
                self.shell_menu_enable = defaults.shell_menu_enable;
                self.shell_menu_viewer = defaults.shell_menu_viewer;
                self.shell_menu_convert = defaults.shell_menu_convert;
                self.shell_menu_convert_shift = defaults.shell_menu_convert_shift;
                self.shell_menu_on_folder = defaults.shell_menu_on_folder;
                self.shell_menu_on_background = defaults.shell_menu_on_background;
                self.shell_preview_icon = defaults.shell_preview_icon;
            }
            // The catch-all: the photo folder the copy commands use, the
            // bookmarks that had no page of their own left, and the leftovers.
            Group::Other => {
                self.photo_folder = defaults.photo_folder;
                self.autobookmark = defaults.autobookmark;
                self.share_settings = defaults.share_settings;
                self.max_bookmarks = defaults.max_bookmarks;
                self.bookmark_folder = defaults.bookmark_folder;
                self.remember_archive_position = defaults.remember_archive_position;
                self.remember_folder_position = defaults.remember_folder_position;
                self.disable_screensaver = defaults.disable_screensaver;
                self.video_output = defaults.video_output;
                self.slideshow = defaults.slideshow.clone();
                self.page_turn_sound = defaults.page_turn_sound;
                self.toast_seconds = defaults.toast_seconds;
                self.use_user_temp_folder = defaults.use_user_temp_folder;
                self.temp_folder = defaults.temp_folder;
                self.sync_to_folder = defaults.sync_to_folder;
                self.do_not_remember_recent = defaults.do_not_remember_recent;
                self.external_editor = defaults.external_editor;
            }
            Group::Language => self.language = defaults.language,
        }
    }
}

/// A page in the settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    General,
    Interface,
    FileAssoc,
    SystemMenu,
    Keyboard,
    Mouse,
    Advanced,
    Other,
    Language,
}

impl Group {
    /// All nine pages, in display order.
    pub const ALL: [Group; 9] = [
        Group::General,
        Group::Interface,
        Group::FileAssoc,
        Group::SystemMenu,
        Group::Keyboard,
        Group::Mouse,
        Group::Advanced,
        Group::Other,
        Group::Language,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::General => "基础",
            Group::Interface => "界面",
            Group::FileAssoc => "文件关联",
            Group::SystemMenu => "系统菜单",
            Group::Keyboard => "键盘快捷键",
            Group::Mouse => "鼠标操作",
            Group::Advanced => "高级",
            Group::Other => "其他",
            Group::Language => "语言",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_border_setting_is_carried_forward_by_the_migration() {
        // A config written before schema 4 has show_border=false because the
        // field was never read, not because the user turned the border off.
        let mut settings: Settings = serde_json::from_str(
            r#"{"schema_version":3,"show_border":false}"#,
        )
        .unwrap();
        settings.migrate();
        assert!(settings.show_border, "the visible state is preserved");
        assert_eq!(settings.schema_version, SCHEMA_VERSION);

        // Once migrated, turning it off sticks.
        settings.show_border = false;
        settings.migrate();
        assert!(!settings.show_border, "a later choice is not overwritten");
    }

    #[test]
    fn a_command_reports_its_default_until_it_is_rebound() {
        let mut settings = Settings::default();
        assert_eq!(
            settings.effective_binding(Command::OpenFile),
            Some("ctrl-o".to_string())
        );
        assert!(!settings.has_custom_binding(Command::OpenFile));

        settings.bind_key(Command::OpenFile, "ctrl-shift-o");
        assert_eq!(
            settings.effective_binding(Command::OpenFile),
            Some("ctrl-shift-o".to_string())
        );
        assert!(settings.has_custom_binding(Command::OpenFile));
        assert_eq!(
            settings.command_for_custom_key("ctrl-shift-o"),
            Some(Command::OpenFile)
        );

        settings.clear_key(Command::OpenFile);
        assert_eq!(
            settings.effective_binding(Command::OpenFile),
            Some("ctrl-o".to_string()),
            "clearing restores the default"
        );
        assert_eq!(settings.command_for_custom_key("ctrl-shift-o"), None);
    }

    #[test]
    fn rebinding_a_chord_takes_it_from_the_command_that_had_it() {
        let mut settings = Settings::default();
        settings.bind_key(Command::NextImage, "ctrl-n");
        let displaced = settings.bind_key(Command::PrevImage, "ctrl-n");
        assert_eq!(displaced, Some(Command::NextImage), "the old owner is reported");
        assert_eq!(
            settings.command_for_custom_key("ctrl-n"),
            Some(Command::PrevImage)
        );
        assert!(
            !settings.has_custom_binding(Command::NextImage),
            "the previous owner loses its custom key"
        );
    }

    #[test]
    fn rebinding_the_same_command_replaces_rather_than_duplicates() {
        let mut settings = Settings::default();
        settings.bind_key(Command::OpenFile, "ctrl-1");
        settings.bind_key(Command::OpenFile, "ctrl-2");
        assert_eq!(
            settings
                .custom_keys
                .iter()
                .filter(|key| key.command == Command::OpenFile.id())
                .count(),
            1
        );
        assert_eq!(settings.command_for_custom_key("ctrl-1"), None);
        assert_eq!(
            settings.command_for_custom_key("ctrl-2"),
            Some(Command::OpenFile)
        );
    }

    #[test]
    fn defaults_are_sensible() {
        let settings = Settings::default();
        assert!(settings.esc_to_exit);
        assert!(settings.exif_rotate);
        assert_eq!(settings.thumbnail_view, ThumbnailView::Grid);
        assert_eq!(settings.wheel_action, WheelAction::Page);
        assert_eq!(settings.zoom_preset, ZoomPreset::Auto);
        assert_eq!(settings.memory_budget_mb, Settings::default().memory_budget_mb);
        assert_eq!(settings.transparency, Transparency::BackgroundColor);
        assert_eq!(settings.view_mode, ViewMode::FitWindow);
    }

    #[test]
    fn round_trips_through_json() {
        let mut settings = Settings::default();
        settings.thumbnail_view = ThumbnailView::Strip;
        settings.memory_budget_mb = 512;
        settings.zoom_preset = ZoomPreset::Percent150;
        settings.wheel_action = WheelAction::Zoom;
        settings.view_mode = ViewMode::TwoPageR2L;

        let text = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn a_toast_lasts_one_second_by_default() {
        let settings = Settings::default();
        assert_eq!(settings.toast_seconds, 1);
        assert_eq!(
            settings.toast_duration(),
            std::time::Duration::from_secs(1)
        );

        // A hand-edited config is clamped into something readable.
        let mut edited = Settings::default();
        edited.toast_seconds = 0;
        assert_eq!(edited.toast_duration(), std::time::Duration::from_secs(1));
        edited.toast_seconds = 600;
        assert_eq!(edited.toast_duration(), std::time::Duration::from_secs(30));
    }

    #[test]
    fn the_sort_order_survives_a_round_trip() {
        let mut settings = Settings::default();
        settings.sort_order = SortOrder {
            key: crate::library::SortKey::Size,
            descending: true,
        };

        let text = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.sort_order, settings.sort_order);
    }

    #[test]
    fn a_config_without_a_sort_order_sorts_by_name() {
        let settings: Settings = serde_json::from_str("{\"esc_to_exit\":false}").unwrap();
        assert_eq!(
            settings.sort_order,
            SortOrder::new(crate::library::SortKey::Name)
        );
    }

    #[test]
    fn a_config_with_a_byte_order_mark_still_loads() {
        // Windows editors write one; ignoring it would reset every setting.
        let text = "\u{feff}{\"memory_budget_mb\":384}";
        let settings = Settings::parse(text).expect("a marked config parses");
        assert_eq!(settings.memory_budget_mb, 384);
    }

    #[test]
    fn the_remembered_window_size_round_trips_and_rejects_nonsense() {
        let mut settings = Settings::default();
        settings.window_width = 1024;
        settings.window_height = 700;

        let text = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.initial_window_size(), (1024., 700.));

        // Turning the preference off opens at the stock size but keeps the
        // recorded one, so turning it back on restores what was there.
        settings.remember_window_size = false;
        assert_eq!(settings.initial_window_size(), DEFAULT_WINDOW_SIZE);
        assert_eq!((settings.window_width, settings.window_height), (1024, 700));

        settings.remember_window_size = true;
        settings.window_width = 0;
        settings.window_height = 0;
        assert_eq!(
            settings.initial_window_size(),
            DEFAULT_WINDOW_SIZE,
            "a zeroed pair from a hand-edited config must not open an invisible window"
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Forward compatibility: an older config without newer keys still loads.
        let settings: Settings = serde_json::from_str("{\"esc_to_exit\":false}").unwrap();
        assert!(!settings.esc_to_exit);
        assert_eq!(settings.memory_budget_mb, Settings::default().memory_budget_mb);
        assert_eq!(settings.schema_version, SCHEMA_VERSION, "migrate() runs on load");
    }

    #[test]
    fn the_performance_defaults_bound_the_frames() {
        let settings = Settings::default();
        assert!(settings.swap_frames, "the swap store is the point of the page");
        assert_eq!(settings.prefetch_window, 2);

        let config = settings.swap_config();
        assert!(config.enabled);
        assert_eq!(config.slots, 8);
        assert_eq!(config.slot_bytes, 16 * 1024 * 1024);
        assert_eq!(config.capacity_bytes, 128 * 1024 * 1024);
        // Whatever the ceiling produces is storable: it either fits a slot or
        // is taken as an oversized frame with a file of its own.
        let edge = settings.decode_max_edge() as usize;
        let largest = edge * edge * 4;
        assert!(
            config.takes(largest),
            "a frame at the {edge}px ceiling ({} MB) must be storable",
            largest / (1024 * 1024)
        );
        assert!(largest <= config.capacity_bytes);
    }

    #[test]
    fn the_swap_store_takes_over_the_memory_budget() {
        let mut settings = Settings::default();
        // On the heap, the decoded cache is the budget's decode share.
        settings.swap_frames = false;
        assert_eq!(settings.decode_cache_capacity(), settings.decode_budget());
        // The texture share is capped, so on a large budget it is the smaller
        // of the two — and never more than the cap.
        assert!(settings.render_budget() <= RENDER_CACHE_CAP_MB as usize * 1024 * 1024);
        assert!(settings.render_budget() > 0);

        // On disk, it is what the store's slots can hold — no RAM figure at all.
        settings.swap_frames = true;
        let config = settings.swap_config();
        assert_eq!(
            settings.decode_cache_capacity(),
            config.usable_slots() * config.slot_bytes
        );
        assert_ne!(settings.decode_cache_capacity(), settings.decode_budget());
    }

    #[test]
    fn a_ceiling_too_small_for_the_slots_shrinks_the_cache_with_them() {
        let mut settings = Settings::default();
        settings.swap_frames = true;
        settings.swap_slots = 8;
        settings.swap_slot_mb = 16;
        settings.swap_capacity_mb = 64;
        // Four slots fit the ceiling, so four frames is what the cache may
        // hold — the store could not place a fifth.
        assert_eq!(settings.decode_cache_capacity(), 4 * 16 * 1024 * 1024);
    }

    #[test]
    fn the_decode_ceiling_is_clamped() {
        let mut settings = Settings::default();
        settings.decode_max_edge = 1;
        assert_eq!(settings.decode_max_edge(), 512);
        settings.decode_max_edge = u32::MAX;
        assert_eq!(settings.decode_max_edge(), 8192);
    }

    #[test]
    fn zoom_presets_map_to_factors() {
        assert_eq!(ZoomPreset::Auto.factor(), None);
        assert_eq!(ZoomPreset::Actual.factor(), Some(1.0));
        assert_eq!(ZoomPreset::Percent175.factor(), Some(1.75));
        assert_eq!(ZoomPreset::Percent250.factor(), Some(2.5));
    }

    #[test]
    fn every_group_has_a_label() {
        for group in Group::ALL {
            assert!(!group.label().is_empty(), "{group:?} has no label");
        }
    }

    #[test]
    fn the_group_list_is_in_the_order_the_panel_shows() {
        assert_eq!(Group::ALL.len(), 9);
        assert_eq!(
            Group::ALL.map(|group| group.label()),
            ["基础", "界面", "文件关联", "系统菜单", "键盘快捷键", "鼠标操作", "高级", "其他", "语言"]
        );
    }

    #[test]
    fn resetting_a_group_leaves_others_alone() {
        let mut settings = Settings::default();
        settings.thumbnail_view = ThumbnailView::Strip;
        settings.memory_budget_mb = 64;
        settings.wheel_speed = 9;

        settings.reset_group(Group::Advanced);

        assert_eq!(
            settings.thumbnail_view,
            ThumbnailView::Strip,
            "the interface group should be untouched"
        );
        assert_eq!(
            settings.memory_budget_mb,
            Settings::default().memory_budget_mb,
            "the advanced group should be reset"
        );
        assert_eq!(
            settings.swap_slots,
            Settings::default().swap_slots,
            "the swap settings belong to the advanced group too"
        );
        assert_eq!(settings.wheel_speed, 9, "mouse group should be untouched");
    }

    #[test]
    fn resetting_every_group_restores_the_defaults() {
        // A group that forgot a field would leave it modified, which this catches.
        let mut settings = Settings::default();
        settings.thumbnail_view = ThumbnailView::Strip;
        settings.memory_budget_mb = 7;
        settings.webtoon_gap = 99;
        settings.shell_menu_enable = true;
        settings.arrow_scroll_only = true;
        settings.max_bookmarks = 3;
        settings.swap_slot_mb = 3;
        settings.decode_max_edge = 999;
        settings.language = "en-US".to_string();
        settings.background_color = "#ff0000".to_string();
        settings.temp_folder = "D:/tmp".to_string();

        for group in Group::ALL {
            settings.reset_group(group);
        }

        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn mouse_bindings_resolve_to_commands() {
        let settings = Settings::default();
        assert_eq!(settings.mouse_command(MouseSlot::DoubleLeft), Some(Command::Maximize));
        assert_eq!(settings.mouse_command(MouseSlot::Left), None);

        let mut settings = settings;
        settings.mouse_bindings = vec![MouseBinding {
            slot: MouseSlot::Middle,
            enabled: true,
            command: Some(Command::NextImage.id().to_string()),
        }];
        assert_eq!(settings.mouse_command(MouseSlot::Middle), Some(Command::NextImage));

        // Disabling a slot must silence it even though a command is recorded.
        settings.mouse_bindings[0].enabled = false;
        assert_eq!(settings.mouse_command(MouseSlot::Middle), None);
    }

    #[test]
    fn every_default_mouse_binding_names_a_real_command() {
        for slot in MouseSlot::ALL {
            if let Some(command) = slot.default_command() {
                assert_eq!(Command::from_id(command.id()), Some(command));
            }
        }
    }

    #[test]
    fn gaps_are_zero_unless_switched_on() {
        let mut settings = Settings::default();
        assert_eq!(settings.two_page_gap(), 0.);
        assert_eq!(settings.webtoon_gap(), 0.);

        settings.two_page_gap_enabled = true;
        settings.webtoon_gap_enabled = true;
        assert_eq!(settings.two_page_gap(), 10.);
        assert_eq!(settings.webtoon_gap(), 10.);
    }

    #[test]
    fn exporting_and_importing_round_trips() {
        let mut settings = Settings::default();
        settings.memory_budget_mb = 42;
        let path = std::env::temp_dir().join("iv-settings-roundtrip.json");
        settings.export_to(&path).unwrap();
        let back = Settings::import_from(&path).unwrap();
        assert_eq!(back, settings);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_decode_cache_budget_becomes_a_total_budget() {
        // A config from before schema 5 held only the decode cache. Its value
        // has to become a total that leaves that cache the size it already was.
        let json = r#"{"schema_version":4,"memory_budget_mb":128}"#;
        let mut settings: Settings = serde_json::from_str(json).unwrap();
        settings.migrate();

        assert_eq!(settings.schema_version, SCHEMA_VERSION);
        assert_eq!(settings.memory_budget_mb, 256, "the cache keeps its 128 MB");
        assert_eq!(settings.decode_budget(), 256 * 1024 * 1024 / 3);
        // The texture cache is capped below its arithmetic share.
        assert_eq!(
            settings.render_budget(),
            RENDER_CACHE_CAP_MB as usize * 1024 * 1024
        );
    }

    #[test]
    fn a_migrated_budget_stays_inside_the_supported_range() {
        let json = r#"{"schema_version":4,"memory_budget_mb":4096}"#;
        let mut settings: Settings = serde_json::from_str(json).unwrap();
        settings.migrate();
        assert_eq!(settings.memory_budget_mb, MEMORY_BUDGET_RANGE.1);
    }

    #[test]
    fn the_old_lock_flags_become_a_chrome_mode() {
        // Locked chrome took its own rows, which is the fixed mode.
        let mut settings: Settings =
            serde_json::from_str(r#"{"schema_version":5,"menu_bar_locked":true,"control_bar_locked":true}"#)
                .unwrap();
        settings.migrate();
        assert_eq!(settings.chrome_mode, ChromeMode::Fixed);

        // Unlocked chrome floated, which is the automatic mode.
        let mut settings: Settings =
            serde_json::from_str(r#"{"schema_version":5,"menu_bar_locked":false,"control_bar_locked":false}"#)
                .unwrap();
        settings.migrate();
        assert_eq!(settings.chrome_mode, ChromeMode::Auto);

        // A config too old to have the fields at all keeps the old default.
        let mut settings: Settings = serde_json::from_str(r#"{"schema_version":3}"#).unwrap();
        settings.migrate();
        assert_eq!(settings.chrome_mode, ChromeMode::Fixed);
    }

    #[test]
    fn the_old_thumbnail_switch_becomes_a_view_mode() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"schema_version":5,"show_thumbnails":true}"#).unwrap();
        settings.migrate();
        assert_eq!(settings.thumbnail_view, ThumbnailView::Strip);

        let mut settings: Settings =
            serde_json::from_str(r#"{"schema_version":5,"show_thumbnails":false}"#).unwrap();
        settings.migrate();
        assert_eq!(settings.thumbnail_view, ThumbnailView::Off);

        // A fresh config gets the new default rather than the migrated one.
        assert_eq!(Settings::default().thumbnail_view, ThumbnailView::Grid);
    }

    #[test]
    fn the_stock_title_format_moves_the_counter_to_the_front() {
        // The old stock format is rewritten; a customised one is left alone.
        let json = r#"{"schema_version": 7, "title_format": "${folder} ${sep} ${file} ${page}/${total}"}"#;
        let mut settings: Settings = serde_json::from_str(json).unwrap();
        settings.migrate();
        assert_eq!(settings.title_format, "${page}/${total}  ${folder} > ${file}");

        let json = r#"{"schema_version": 7, "title_format": "${file}"}"#;
        let mut settings: Settings = serde_json::from_str(json).unwrap();
        settings.migrate();
        assert_eq!(settings.title_format, "${file}");
    }

    #[test]
    fn the_tool_group_is_hidden_by_default() {
        assert!(!Settings::default().show_toolbar);
    }

    #[test]
    fn the_photo_folder_commands_are_renamed_by_the_migration() {
        // A config written when there were two folders, with the first bound to
        // a key and the second to a mouse button.
        let json = r#"{
            "schema_version": 6,
            "custom_keys": [
                {"enabled": true, "binding": "insert", "command": "CopyToPhotoFolder1"},
                {"enabled": true, "binding": "f6", "command": "CopyImage"}
            ],
            "mouse_bindings": [
                {"slot": "Middle", "enabled": true, "command": "OpenPhotoFolder2"},
                {"slot": "X", "enabled": true, "command": "MoveToPhotoFolder1"}
            ]
        }"#;
        let mut settings: Settings = serde_json::from_str(json).unwrap();
        settings.migrate();

        assert_eq!(settings.schema_version, SCHEMA_VERSION);
        assert_eq!(settings.custom_keys[0].command, "CopyToPhotoFolder");
        assert_eq!(
            settings.custom_keys[1].command, "CopyImage",
            "an unrelated binding is left alone"
        );
        assert_eq!(
            settings.mouse_bindings[0].command, None,
            "the second folder is gone, so its binding is dropped"
        );
        assert_eq!(settings.mouse_bindings[1].command.as_deref(), Some("MoveToPhotoFolder"));
    }

    #[test]
    fn the_old_photo_folder_key_keeps_its_value() {
        let json = r#"{"schema_version": 6, "photo_folder_1": "D:/pictures", "photo_folder_2": "E:/more"}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.photo_folder, "D:/pictures");
    }

    #[test]
    fn the_photo_folder_defaults_to_the_profile_pictures_folder() {
        let folder = Settings::default_photo_folder();
        if std::env::var_os("USERPROFILE").is_some() {
            let path = std::path::Path::new(&folder);
            assert_eq!(path.file_name(), Some(std::ffi::OsStr::new("SeePic")));
            assert_eq!(
                path.parent().and_then(|parent| parent.file_name()),
                Some(std::ffi::OsStr::new("Pictures"))
            );
        }
        assert_eq!(Settings::default().photo_folder, folder);

        // A config written while the field was still empty is brought forward
        // on load, so the copy/move commands have a destination without the
        // user having to pick one first.
        let mut settings = Settings::default();
        settings.photo_folder.clear();
        settings.migrate();
        assert_eq!(settings.photo_folder, folder);
    }

    #[test]
    fn the_superseded_fields_are_not_written_back() {
        let mut settings = Settings::default();
        settings.menu_bar_locked = Some(false);
        settings.control_bar_locked = Some(false);
        settings.show_thumbnails = Some(true);

        let text = serde_json::to_string(&settings).unwrap();
        assert!(!text.contains("menu_bar_locked"), "legacy field leaked: {text}");
        assert!(!text.contains("control_bar_locked"));
        assert!(!text.contains("show_thumbnails"));
        assert!(text.contains("chrome_mode"));
        assert!(text.contains("thumbnail_view"));
    }
}
