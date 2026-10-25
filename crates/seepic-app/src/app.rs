//! The application view: state, command execution and layout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::*;
use gpui_kit::{ClipboardItem, Priority, ScrollStrategy};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::command::{canonical_binding, Command};
use seepic_core::cache::{CacheKey, CachedImage, Detail, ImageCache};
use seepic_core::decode::{self, DecodedImage};
use seepic_core::effects::Adjustments;
use seepic_core::metadata::ImageInfo;
use seepic_core::settings::{ChromeMode, Group, MouseSlot, Settings, ThumbnailView, WheelAction};
use seepic_core::{
    BackgroundMode, Container, ContainerKind, ContainerRef, Navigator, Orientation, Point as IvPoint,
    IntervalUnit, Size as IvSize, Slideshow, SortKey, SortOrder,
    Transparency, ViewMode, ZoomState,
};

use seepic_core::filters::{FilterMode, Interpolation};
use crate::menu_bar::Menu;use crate::strings as s;
use crate::confirm_dialog::DialogState;
use crate::drop_fallback;
use crate::image_clipboard;
use crate::password_dialog::PasswordPrompt;
use crate::render_cache::{FramePick, RenderCache, RenderKey, Transform};
use crate::{
    canvas, confirm_dialog, contact_sheet, context_menu, menu_bar, panels,
    password_dialog, settings_panel, status_bar, thumbnails, wallpaper,
};
use crate::info_window::InfoWindow;

/// Where the configuration file lives.
pub fn config_path() -> PathBuf {
    seepic_core::settings::Settings::path()
}

/// Where the header cache lives.
pub fn meta_cache_path() -> PathBuf {
    seepic_core::settings::Settings::data_dir().join("meta-cache.json")
}

/// Height of the single-row top bar.
/// Height of the top bar, which is the platform title bar's own height
/// (`gpui-component`'s `TITLE_BAR_HEIGHT`). The layout does not depend on it -
/// the bar takes a row and the canvas gets what is left - but the context menu
/// is placed below the chrome by it.
pub const TOP_BAR_HEIGHT: f32 = 34.;
/// Height of the status bar along the bottom.
pub const STATUS_BAR_HEIGHT: f32 = 30.;
/// How far two releases may drift apart and still count as one double click.
/// How long they may be apart is the system's own double-click time.
const DOUBLE_CLICK_SLOP: f32 = 5.;
/// How near the top edge the pointer has to be to reveal the bars.
const TOP_REVEAL: f32 = 48.;
/// Above this many decoded bytes, an animation is played a frame at a time.
///
/// Handing a long animation to GPUI whole makes the built image hold every
/// frame — a second full copy beside the decoded one — and a 100-frame 800x600
/// GIF is 190 MB of copy on its own. Below this figure that copy is small
/// enough to keep the simpler whole-animation path, with GPUI's own timing.
const ANIMATION_SELF_DRIVEN_BYTES: usize = 32 * 1024 * 1024;
/// Frames decoded per hand-off while an animation streams.
///
/// Four frames are about a tenth of a second of decode work and roughly four
/// frames of playback: enough for the viewer to stay ahead of the clock without
/// making the first frame wait for the whole animation.
const ANIMATION_CHUNK_FRAMES: usize = 4;
/// Shortest time one frame of a self-driven animation is held, whatever the
/// file's delay says; a zero delay would otherwise spin the timer.
const MIN_ANIM_FRAME_MS: u32 = 20;
/// How long the bottom animation bar stays after the pointer stops.
///
/// Long enough to reach for the pause button after moving the mouse, short
/// enough that the controls are not part of the picture.
const ANIM_BAR_IDLE_MS: u64 = 2500;
/// Default width of the search results' page-number column.
///
/// Wide enough for `9999/9999`; the divider in the column header changes it.
pub const SEARCH_COL_DEFAULT: f32 = 64.;
pub const SEARCH_COL_MIN: f32 = 40.;
pub const SEARCH_COL_MAX: f32 = 240.;
/// Target box for thumbnails in the strip and the grid.
///
/// Both sides are bounded: scaling only the width turns a tall webtoon page into
/// a 112 x thousands tile, which is the opposite of what a thumbnail is for.
const THUMBNAIL_WIDTH: u32 = 112;
const THUMBNAIL_HEIGHT: u32 = 116;
/// Thumbnail decodes allowed to run at once.
///
/// The grid asks for every visible cell on every frame. Without a cap a fast
/// scroll starts a full decode per cell — each one transiently holding the
/// source pixels — so the peak memory tracked the scroll speed rather than the
/// number of thumbnails kept. Cells that miss the cap are re-asked on the next
/// frame, so the pool naturally follows the viewport.
const THUMBNAIL_JOBS: usize = 4;
/// How much of a page is read to learn its dimensions.
///
/// Every format's dimensions live within the first few kilobytes, and EXIF
/// orientation within the first APP segment; 256 KB leaves ample room and still
/// saves decompressing the rest of a multi-megabyte page once per page in
/// two-page and webtoon modes.
const PROBE_PREFIX_BYTES: usize = 256 * 1024;
/// Longest side of the Blur background's source image. Small enough that the
/// GPU's magnification of it reads as a blur.
const BLUR_BACKDROP_SIDE: u32 = 32;
/// Never decode below this, so a tiny window still shows a usable image.
const MIN_DECODE_SIDE: u32 = 512;
/// Never decode above this, whatever the window size.

/// How much smaller than its own texture an image must be drawn before a
/// downscale pyramid is worth building for it.
///
/// The decode headroom keeps a fitted image at about 0.7 of its texture, so the
/// threshold has to sit below that or every page turn builds a pyramid it does
/// not need — five more textures per image and a background resize to go with
/// them. Below this the draw really is small enough for the GPU's four-texel
/// bilinear sampling to alias visibly.
const MIP_TRIGGER_SCALE: f32 = 0.6;
/// Colour swatches offered for the background and border.
pub const BACKGROUND_SWATCHES: [(&str, u32); 8] = [
    ("#1a1a1a", 0x1a1a1a),
    ("#000000", 0x000000),
    ("#3c3c3c", 0x3c3c3c),
    ("#5a5a5a", 0x5a5a5a),
    ("#808080", 0x808080),
    ("#c0c0c0", 0xc0c0c0),
    ("#f2f2f2", 0xf2f2f2),
    ("#ffffff", 0xffffff),
];

/// Everything that decides what a rendered page looks like beyond its pixels.
///
/// Cached renders of pages other than the current one are keyed by position
/// alone, so they also carry this: a rotation or a filter change leaves the key
/// untouched but makes the pixels wrong.
type TransformSignature = (Orientation, Adjustments, FilterMode, bool);

/// A downscaled copy of the current image.
pub struct MipLevel {
    render: Arc<RenderImage>,
    /// Pixel size of this level, in image pixels.
    size: IvSize,
}

/// The pixels a mip pyramid is built from.
///
/// The chain has to match what is on screen, so it is normally built from the
/// texture's own bytes. When nothing has been done to the image since it was
/// decoded, the shared decode is the same pixels and can be used without
/// copying the texture.
enum MipSource {
    Decoded(Arc<DecodedImage>),
    Pixels(Vec<u8>),
}

impl MipSource {
    fn pixels(&self) -> Option<&[u8]> {
        match self {
            MipSource::Decoded(image) => Some(image.frames.first()?.bgra.as_slice()),
            MipSource::Pixels(bytes) => Some(bytes.as_slice()),
        }
    }
}

/// One of the right-hand panels. Only one is open at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanel {
    Image,
    Sort,
    Slideshow,
    Bookmark,
}

impl SidePanel {
    /// The heading a pushed screen shows. The base screen has none: its button
    /// already labels it.
    pub fn title(self) -> &'static str {
        match self {
            SidePanel::Image => "图像",
            SidePanel::Sort => "排序",
            SidePanel::Slideshow => "幻灯片播放",
            SidePanel::Bookmark => "收藏夹",
        }
    }
}

/// A saved location.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub path: PathBuf,
    pub size: u64,
}

/// One of the Image panel's effect checkboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Negative,
    Gamma,
    Brightness,
    Contrast,
    Saturation,
    Gray,
    Sepia,
}

/// The file search over the open container's entries.
#[derive(Debug, Clone, Default)]
pub struct FileSearch {
    pub filter: String,
    /// Index into `matches`.
    pub selection: usize,
    /// Display positions whose names match the filter.
    pub matches: Vec<usize>,
}

/// The command list opened from the mouse settings page.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandPicker {
    /// The mouse slot the chosen command will be bound to, or `None` when the
    /// list is being browsed rather than assigned.
    pub slot: Option<MouseSlot>,
    /// Substring filter over the command's label and id.
    pub filter: String,
}

impl CommandPicker {
    /// Commands matching the filter, in table order.
    pub fn matches(&self) -> Vec<Command> {
        let needle = self.filter.trim().to_lowercase();
        Command::ALL
            .iter()
            .copied()
            .filter(|command| {
                needle.is_empty()
                    || command.label().to_lowercase().contains(&needle)
                    || command.id().to_lowercase().contains(&needle)
            })
            .collect()
    }
}

/// A settings dropdown that is open.
///
/// The settings panel scrolls, so a list anchored inside it would be cut off at
/// the viewport's edge. The list is drawn at the top level instead, at the row
/// that was clicked, and the choice is applied by the function it carries.
pub struct ChoicePopup {
    /// Where the list hangs, in window coordinates.
    pub anchor: IvPoint,
    /// Every entry, in order.
    pub labels: Vec<SharedString>,
    /// Which entry is the current value, when the current value is one of them.
    pub selected: Option<usize>,
    /// How to apply the entry that is picked.
    pub set: fn(&mut ViewerApp, usize, &mut Context<ViewerApp>),
}

/// Why a load failed, so the password case can open the prompt instead of a toast.
enum LoadFailure {
    Password,
    Other(String),
}

/// What the background produced before the viewer decides how to show it.
enum Opening {
    /// An animated WebP to be decoded frame by frame: the viewer can show what
    /// has arrived and keep feeding it.
    Stream(decode::WebPAnimation),
    /// Bytes for the ordinary decoder, either already complete or not an
    /// animation.
    Still(Vec<u8>),
    /// It looked like an animated WebP but would not open; the ordinary decoder
    /// cannot help because the bytes went with the failed attempt.
    Failed(String),
}

/// An image that is currently loaded and drawable.
pub struct CurrentImage {
    pub index: usize,
    /// Built on the main thread, because `RenderImage` is not `Send`.
    pub render: Arc<RenderImage>,
    /// Whether the render cache owns a reference to `render`.
    ///
    /// A page shown only in passing — one a fast pager has already moved past —
    /// is built without being cached, so it has to be handed back here when it
    /// leaves the screen. Anything else would leave its texture in the atlas
    /// with nobody left to release it.
    pub render_cached: bool,
    /// Size after the user's rotation and flips are applied.
    pub size: IvSize,
    /// Kept so rotating does not need to re-read and re-decode the source.
    pub decoded: Arc<DecodedImage>,
    /// Frames the file declares.
    ///
    /// A streaming animation starts with only a few frames decoded, but the
    /// frame counter and the slider already know the whole figure. Every other
    /// image has this equal to `decoded.frames.len()`.
    pub total_frames: usize,
    pub name: SharedString,
    pub byte_size: u64,
    /// Halved copies, built in the background on first use, for crisp display
    /// when zoomed out.
    pub mips: Option<Vec<MipLevel>>,
    /// Whether a pyramid has been asked for and not yet arrived, so the request
    /// is not repeated on every frame while it is being built.
    pub mips_requested: bool,
    /// How much detail this image holds; `None` is the full resolution.
    pub detail: Detail,
    /// The image's average colour, for the Auto background.
    pub average_color: u32,
    /// A tiny copy of the image, drawn stretched for the Blur background.
    pub blur: Option<Arc<RenderImage>>,
    /// Whether this animation is played a frame at a time by the viewer.
    ///
    /// A short animation is handed to GPUI whole, which cycles its frames on
    /// the render thread. A long one is not: the built image would hold a second
    /// copy of every frame on top of the decoded ones, so the viewer keeps only
    /// the frame on screen and builds the next when its time comes.
    pub self_driven: bool,
}

/// A transient message shown in the corner.
pub struct Toast {
    pub text: SharedString,
    pub is_error: bool,
    pub shown_at: Instant,
}

impl Toast {
    /// Whether the message has been up for `lifetime`, the user's setting.
    fn expired(&self, lifetime: Duration) -> bool {
        self.shown_at.elapsed() > lifetime
    }
}

pub struct ViewerApp {
    focus: FocusHandle,

    /// Shared with the background decode task, which needs to read from it.
    container: Option<Arc<Mutex<Container>>>,
    pub navigator: Navigator,
    /// Folders and archives next to the current one, for `[` and `]`.
    siblings: Vec<ContainerRef>,
    sibling_index: Option<usize>,

    pub current: Option<CurrentImage>,
    pub zoom: ZoomState,
    pub orientation: Orientation,
    pub loading: bool,
    pub error: Option<SharedString>,
    pub toast: Option<Toast>,

    pub show_info: bool,
    /// Whether the thumbnail grid overlay is open.
    ///
    /// Distinct from the setting, which chooses between the grid, the strip and
    /// nothing: this is the on/off of the overlay the setting selects.
    pub contact_sheet: bool,
    /// Thumbnails by display position, filled in as the strip scrolls.
    pub thumbnails: HashMap<usize, Arc<RenderImage>>,
    /// Thumbnails that finished while the grid was scrolling.
    ///
    /// Installing them straight away would upload a texture and evict another
    /// in the middle of the wheel animation, which is exactly the kind of
    /// per-frame work that makes the grid stutter. They are held here and
    /// installed in one batch once the grid stops.
    deferred_thumbnails: Vec<(usize, Arc<RenderImage>)>,
    /// Positions already being decoded, so scrolling does not re-request them.
    thumbnails_requested: HashSet<usize>,
    /// Thumbnails the last grid frame drew (plus its prefetch rows).
    ///
    /// These must never be evicted: a large window can show more tiles than
    /// [`Self::THUMBNAIL_KEEP`], and because the prefetch rows are requested
    /// after the visible ones they have newer LRU ticks. Without this set the
    /// prune removed on-screen tiles, the next frame re-requested them, and the
    /// grid decoded the same handful of files forever (30k decodes for a
    /// 1152-file folder).
    pub thumbnail_protected: HashSet<usize>,
    /// Decodes currently running, capped by [`Self::THUMBNAIL_JOBS`].
    thumbnails_in_flight: usize,
    /// Last frame each thumbnail was asked for, for LRU eviction.
    ///
    /// Evicting by distance to the open page was wrong for the grid: the pages
    /// on screen can be thousands of entries away from the cursor, so their
    /// thumbnails were dropped as soon as they arrived and re-decoded on the
    /// next frame. A use counter keeps what is actually being looked at.
    thumbnail_use: HashMap<usize, u64>,
    thumbnail_tick: u64,
    /// Incremented on a container or order change, so a decode that was already
    /// running cannot install its thumbnail under the new listing.
    thumbnail_generation: u64,
    /// Scale factor the resident thumbnails were decoded for, in physical
    /// pixels per logical point.
    thumbnail_scale: f32,
    /// Owned so the strip can be scrolled programmatically.
    pub strip_scroll: VirtualListScrollHandle,
    /// Index currently at the strip's left edge, used for wheel scrolling.
    strip_first: usize,
    /// True while the pointer is in the top strip, which reveals the bars in
    /// the automatic mode.
    pointer_at_top: bool,
    /// True while the pointer is in the bottom strip.
    pointer_at_bottom: bool,
    /// Where each menu title sits, by `Menu` index, so a dropdown opens under
    /// the button that was clicked rather than at the start of the bar.
    pub menu_anchors: [Option<(f32, f32)>; crate::menu_bar::MENU_SLOTS],
    /// Recomputed on every pointer move, so the reveal thresholds and the
    /// repaint trigger can never disagree.
    arrows_visible: bool,
    /// Measured size of the canvas, so fit modes use the real viewport rather
    /// than a guess based on chrome heights.
    canvas_bounds: Option<IvSize>,
    /// The menu row the keyboard is on, as an index into the open menu's rows.
    menu_index: Option<usize>,
    /// True between a left press on the chrome and the first movement, so a
    /// drag that starts on a button still moves the window.
    pub chrome_drag_armed: bool,
    /// The command waiting for a new key chord, while the recorder is armed.
    pub recording_key: Option<Command>,
    /// The open panel, and any screens pushed on top of it.
    ///
    /// The first entry is the screen the menu button opened; a screen pushed
    /// from inside it is shown at the same anchor and offers a way back, so
    /// the stack is the whole navigation model.
    pub panel_stack: Vec<SidePanel>,
    /// The "what should happen at the end?" prompt, when the boundary setting
    /// asks for one.
    pub boundary_prompt: Option<crate::boundary_prompt::BoundaryPrompt>,
    /// The open menu's scroll position, so the highlight can be brought into
    /// view when it is driven from the keyboard.
    pub menu_scroll: VirtualListScrollHandle,
    pub fullscreen: bool,
    pub always_on_top: bool,
    /// The GPUI theme last applied, so the settings are not re-pushed every
    /// frame.
    applied_theme: Option<ThemeMode>,

    pub slideshow: Slideshow,
    /// Whether each custom duration row reads in seconds or milliseconds.
    pub slideshow_interval_unit: IntervalUnit,
    pub slideshow_display_unit: IntervalUnit,
    slideshow_started: Option<Instant>,
    slideshow_tick: Option<Task<()>>,

    /// Which menu dropdown is open, if any.
    pub open_menu: Option<Menu>,
    pub adjustments: Adjustments,
    /// Shown in the info panel; EXIF arrives asynchronously.
    pub info: Option<ImageInfo>,
    /// The EXIF read for the page on screen has not started yet.
    ///
    /// A burst of page turns skips it, because it is a second read of the file
    /// competing with the decodes; the page the reader stops on picks it up
    /// once the burst ends.
    info_exif_pending: bool,
    /// An EXIF read is running, so a second one is not started.
    info_exif_in_flight: bool,
    /// The separate window showing the current file and its EXIF block.
    info_window: Option<(AnyWindowHandle, Entity<InfoWindow>)>,
    /// Clears `show_info` when the info window is closed by its own title bar.
    info_window_subscription: Option<Subscription>,
    /// Persistent configuration and the settings overlay state.
    pub settings: Settings,
    /// Decoded images, so navigating back and forth does not re-decode.
    cache: ImageCache,
    /// Where decoded frames live: heap pages, or the mapped slot files the
    /// swap store keeps. Shared with the decode threads, which are the ones
    /// that put a frame there.
    swap: seepic_core::swap::FrameStore,
    /// Built, drawable images, so coming back to a page reuses its texture.
    ///
    /// The decode cache above saves the pixel work; this one saves the GPU
    /// texture, which is what makes a page turn stop committing new memory.
    render_cache: RenderCache,
    pub show_settings: bool,
    pub settings_group: Group,
    /// Where the canvas context menu is open, in window coordinates.
    pub context_menu: Option<IvPoint>,
    /// Open password prompt for an encrypted archive.
    pub password_prompt: Option<PasswordPrompt>,
    password_focus: FocusHandle,
    pub dialog: Option<DialogState>,
    dialog_focus: FocusHandle,
    /// The image converter dialog, with its batch progress.
    pub convert: Option<crate::converter::ConvertDialog>,
    /// Page-turn animation in progress, with when it started.
    page_turn: Option<(seepic_core::PageTurnFx, Instant)>,

    /// Whether a password has already been submitted for this container.
    password_tried: bool,
    pub filter: FilterMode,
    pub interpolation: Interpolation,

    /// How the container is laid out: single page, two page or webtoon.
    pub view_mode: ViewMode,
    /// What sits behind images that carry an alpha channel.
    pub transparency: Transparency,
    /// Page dimensions by display position, filled in by a background probe.
    /// Two-page and webtoon layout cannot be computed without all of them.
    pub page_sizes: Vec<Option<IvSize>>,
    /// For each page, the page that begins its spread.
    pub spread_starts: Vec<usize>,
    /// Bumped whenever a probe run starts, so a stale run can be discarded.
    probe_generation: u64,
    /// True while page dimensions are being read.
    pub probing: bool,
    /// Header reads recorded across runs, so reopening a container is instant.
    meta_cache: seepic_core::metacache::MetaCache,
    /// Where reading stopped in each container, across runs.
    positions: seepic_core::positions::PositionStore,
    /// When the position was last written, so paging does not write per page.
    position_saved_at: Option<Instant>,
    /// The container key the position store is using for the open container.
    position_key: Option<String>,
    /// Entry count at the last flush, so writes happen in batches.
    meta_cache_flushed_at: usize,
    /// When the header cache was last written.
    ///
    /// Probing a large container writes in batches, and each write serializes
    /// the whole cache; without a time floor the total cost grows with the
    /// square of the entry count.
    meta_cache_flushed_when: Option<Instant>,
    /// Bumped per load, so a slow decode cannot replace a newer image.
    load_generation: u64,
    /// Keys with a prefetch decode in flight.
    ///
    /// A page turn re-runs the whole prefetch window and the same neighbour
    /// stays in range for several turns, so without this each turn would start
    /// another decode of the same page.
    prefetching: HashSet<CacheKey>,
    /// The navigator position on screen, for deciding whether a decode that
    /// finished late is still worth showing.
    displayed_cursor: Option<usize>,
    /// When the page was last turned, so a burst of turns can be spotted and
    /// decoded at a lower detail.
    last_step_at: Option<Instant>,
    /// Whether a delayed full-detail decode is already waiting for the burst to
    /// end, so a second one is not started on every page turn.
    upgrade_scheduled: bool,
    /// Whether a full-detail decode has been asked for and not yet arrived.
    ///
    /// The render pass asks for one whenever the page on screen is a preview,
    /// which is every frame until the decode lands; without this the request
    /// would start a fresh decode per frame.
    upgrade_in_flight: bool,

    /// The detail the canvas can show, recomputed every frame.
    ///
    /// The render pass is the only place that knows the window's scale factor
    /// and the current zoom, so it records the figure here and the decode paths
    /// read it rather than each needing a window.
    decode_target: Option<u32>,
    /// The longest side the current image is drawn at, in physical pixels.
    ///
    /// Recorded by the render pass alongside the decode target, and compared
    /// against the texture size to decide whether a downscale pyramid is worth
    /// building.
    drawn_longest: f32,
    /// Whether a frame has run, and so whether the layout is known.
    layout_measured: bool,
    /// A load that arrived before the first frame, waiting for one to size it.
    load_pending: bool,
    /// How long the last decode took, for the status readout.
    pub last_load_ms: Option<f32>,
    /// Webtoon's vertical scroll position.
    pub webtoon_scroll: VirtualListScrollHandle,
    /// A webtoon scroll to apply once the list has laid itself out.
    pub webtoon_pending_scroll: Option<usize>,
    /// Decoded webtoon pages, by display position.
    pub webtoon_renders: HashMap<usize, Arc<RenderImage>>,
    /// Pages already being decoded, so the visible range does not re-request.
    webtoon_requested: HashSet<usize>,
    /// The other page of the current spread, by display position.
    ///
    /// A spread shows two pages but only the cursor's page is decoded for the
    /// main view, so the partner is decoded here. Each entry records the
    /// transform it was built with, because a rotation or filter change makes
    /// it wrong without changing its key.
    spread_renders: HashMap<usize, (TransformSignature, Arc<RenderImage>)>,
    /// Partner pages already being decoded, with the transform they asked for.
    spread_requested: HashMap<usize, TransformSignature>,
    /// The transparency checkerboard, rebuilt only when the canvas resizes.
    checker: Option<(IvSize, Arc<RenderImage>)>,
    /// The background picture and the path it came from.
    background_image: Option<(String, Arc<RenderImage>)>,
    /// True while the configured background picture is being read.
    background_image_loading: bool,
    /// The animation frame being held, or `None` when the image plays itself.
    pub anim_frame: Option<usize>,
    /// Whether an animation is advancing on its own.
    pub anim_playing: bool,
    /// Bumped whenever a self-driven animation must stop, so a timer that was
    /// already waiting for its frame does not advance a page it no longer owns.
    anim_generation: u64,
    /// Whether the bottom animation bar is on screen.
    ///
    /// It is shown when an animated image opens or the pointer moves, and hides
    /// itself once the pointer has been still for a moment — the controls are
    /// in the way of the picture otherwise.
    anim_bar_visible: bool,
    /// When the bar was last revealed or kept alive, for the idle countdown.
    anim_bar_revealed_at: Instant,
    /// Whether the pointer is over the bar, which keeps it up regardless.
    anim_bar_hovered: bool,
    /// A countdown task is already waiting, so activity only moves the deadline.
    anim_bar_timer_pending: bool,
    /// Whether the window is minimised, refreshed each frame.
    ///
    /// Nothing that is prepared for the next page turn is worth doing while the
    /// window is out of sight, so the background work checks this before it
    /// spends a decode on it.
    window_minimized: bool,
    /// The order the navigator sorts by.
    pub sort_order: SortOrder,
    /// Saved locations.
    pub bookmarks: Vec<Bookmark>,
    /// The row highlighted in the file navigator.
    pub navigator_selection: usize,
    /// Whether the file navigator is open.
    pub show_navigator: bool,
    /// Textures evicted since the last frame, waiting for a window to be
    /// released through.
    ///
    /// GPUI's `drop_image` takes the current window explicitly, because the app
    /// has taken it out of its map for the duration of an update — and an
    /// eviction always happens inside one. Passing `None` there silently leaves
    /// the atlas entry, and with it the texture, alive: every page turn then
    /// commits a new texture of its own size and the process grows by roughly
    /// the size of the image per page. The queue is drained at the start of the
    /// next frame, where the window is in hand.
    pending_texture_drops: Vec<Arc<RenderImage>>,
    /// The settings dropdown that is open, if any.
    pub choice: Option<ChoicePopup>,
    /// The command list opened from the mouse settings page.
    pub picker: Option<CommandPicker>,
    /// The file search over the open folder or archive.
    pub search: Option<FileSearch>,
    /// Scroll position of the search results.
    ///
    /// Held by the app rather than the modal so the highlighted row can be
    /// scrolled into view, and so the list can carry an auto-hiding scrollbar.
    pub search_scroll: ScrollHandle,
    /// Width of the page-number column in the search results, in pixels.
    pub search_col_width: f32,
    /// Where a column-width drag started: the pointer x and the width then.
    pub search_col_drag: Option<(f32, f32)>,
    /// The thumbnail grid's name filter.
    pub contact_filter: String,
    /// Entry positions matching `contact_filter`, in display order.
    pub contact_matches: Vec<usize>,
    /// First match index for each A–Z slot (26 is `#`), cached for the index
    /// bar so it does not rescan the whole match list every frame.
    pub contact_index_first: [Option<usize>; 27],
    /// The grid's vertical scroll position. The virtual list only knows which
    /// rows to draw from this handle, and the body only knows how far it can
    /// scroll from the list's full height.
    pub contact_scroll: VirtualListScrollHandle,
    /// The first row at the top of the grid, in match order.
    ///
    /// Kept in step with the pixel offset for the selection and index-bar paths;
    /// the wheel itself moves [`Self::contact_scroll_target`].
    pub contact_top_row: usize,
    /// Where the grid's eased wheel scroll is heading, in pixels (<= 0).
    ///
    /// The wheel only moves this target; a per-frame loop walks the list's
    /// offset towards it, so one notch glides instead of teleporting a whole
    /// row and a quick series of notches accumulates into one motion.
    contact_scroll_target: f32,
    contact_scroll_animating: bool,
    /// When the last easing step ran, for a frame-rate independent speed.
    contact_scroll_last: Option<Instant>,
    /// The progress track's rectangle in window coordinates (left, width),
    /// recorded at prepaint so a press maps to the position under the pointer.
    pub seek_track: Option<(f32, f32)>,
    /// True between a press on the progress track and the button's release.
    pub scrubbing: bool,

    /// Last known cursor position, for drag-to-pan.
    last_mouse: Option<IvPoint>,
    /// Where and when the left button was last released, for the double click.
    last_click_up: Option<(Instant, IvPoint)>,
    /// Latest pointer position, used to reveal the edge controls.
    cursor: Option<IvPoint>,
}

/// A saved duration reads in whole seconds when it is one, and in milliseconds
/// otherwise.
fn unit_for(interval: &seepic_core::Interval) -> IntervalUnit {
    if interval.as_millis() % 1000 == 0 {
        IntervalUnit::Seconds
    } else {
        IntervalUnit::Milliseconds
    }
}

impl ViewerApp {
    pub fn new(initial: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        // Loaded first: the slideshow starts from the interval it was left at.
        let settings = Settings::load();
        // Read before the settings are moved into the app.
        let swap_config = settings.swap_config();
        let sort_order = settings.sort_order;
        let mut app = Self {
            focus: cx.focus_handle(),
            container: None,
            navigator: Navigator::new(),
            siblings: Vec::new(),
            sibling_index: None,
            current: None,
            displayed_cursor: None,
            last_step_at: None,
            upgrade_scheduled: false,
            upgrade_in_flight: false,
            zoom: ZoomState::default(),
            orientation: Orientation::default(),
            loading: false,
            error: None,
            toast: None,
            show_info: false,
            contact_sheet: false,
            thumbnails: HashMap::new(),
            thumbnails_requested: HashSet::new(),
            thumbnail_protected: HashSet::new(),
            deferred_thumbnails: Vec::new(),
            thumbnails_in_flight: 0,
            thumbnail_use: HashMap::new(),
            thumbnail_tick: 0,
            thumbnail_generation: 0,
            thumbnail_scale: 0.,
            strip_scroll: VirtualListScrollHandle::new(),
            strip_first: 0,
            pointer_at_top: false,
            pointer_at_bottom: false,
            menu_anchors: [None; crate::menu_bar::MENU_SLOTS],
            arrows_visible: false,
            canvas_bounds: None,
            menu_index: None,
            boundary_prompt: None,
            menu_scroll: VirtualListScrollHandle::new(),
            fullscreen: false,
            always_on_top: false,
            applied_theme: None,
            slideshow: Slideshow::new(settings.slideshow.clone()),
            // A saved sub-second duration reads in milliseconds; a whole one
            // reads in seconds.
            slideshow_interval_unit: unit_for(&settings.slideshow.interval),
            slideshow_display_unit: unit_for(&settings.slideshow.display),
            slideshow_started: None,
            slideshow_tick: None,
            open_menu: None,
            panel_stack: Vec::new(),
            chrome_drag_armed: false,
            recording_key: None,
            adjustments: Adjustments::default(),
            info: None,
            info_exif_pending: false,
            info_exif_in_flight: false,
            info_window: None,
            info_window_subscription: None,
            settings,
            cache: ImageCache::new(600 * 1024 * 1024),
            swap: seepic_core::swap::FrameStore::new(swap_config),
            render_cache: RenderCache::new(600 * 1024 * 1024),
            show_settings: false,
            settings_group: Group::General,
            context_menu: None,
            password_prompt: None,
            password_focus: cx.focus_handle(),
            dialog: None,
            convert: None,
            page_turn: None,
            dialog_focus: cx.focus_handle(),
            password_tried: false,
            filter: FilterMode::default(),
            interpolation: Interpolation::default(),
            view_mode: ViewMode::FitWindow,
            transparency: Transparency::BackgroundColor,
            page_sizes: Vec::new(),
            spread_starts: Vec::new(),
            probe_generation: 0,
            probing: false,
            meta_cache: seepic_core::metacache::MetaCache::open(
                seepic_core::settings::Settings::data_dir().join("meta-cache.json"),
            ),
            meta_cache_flushed_at: 0,
            meta_cache_flushed_when: None,
            positions: seepic_core::positions::PositionStore::open(
                seepic_core::settings::Settings::data_dir().join("positions.json"),
            ),
            position_saved_at: None,
            position_key: None,
            load_generation: 0,
            prefetching: HashSet::new(),
            decode_target: None,
            drawn_longest: 0.,
            layout_measured: false,
            load_pending: false,
            last_load_ms: None,
            webtoon_scroll: VirtualListScrollHandle::new(),
            webtoon_pending_scroll: None,
            webtoon_renders: HashMap::new(),
            webtoon_requested: HashSet::new(),
            spread_renders: HashMap::new(),
            spread_requested: HashMap::new(),
            checker: None,
            background_image: None,
            background_image_loading: false,
            anim_frame: None,
            anim_playing: true,
            anim_generation: 0,
            anim_bar_visible: false,
            anim_bar_revealed_at: Instant::now(),
            anim_bar_hovered: false,
            anim_bar_timer_pending: false,
            window_minimized: false,
            sort_order,
            bookmarks: Vec::new(),
            navigator_selection: 0,
            show_navigator: false,
            pending_texture_drops: Vec::new(),
            choice: None,
            picker: None,
            search: None,
            search_scroll: ScrollHandle::new(),
            search_col_width: SEARCH_COL_DEFAULT,
            search_col_drag: None,
            contact_filter: String::new(),
            contact_matches: Vec::new(),
            contact_index_first: [None; 27],
            contact_scroll: VirtualListScrollHandle::new(),
            contact_top_row: 0,
            contact_scroll_target: 0.,
            contact_scroll_animating: false,
            contact_scroll_last: None,
            seek_track: None,
            scrubbing: false,
            last_mouse: None,
            last_click_up: None,
            cursor: None,
        };

        app.apply_settings();

        if let Some(path) = initial {
            app.open_path(&path, cx);
        }

        app
    }

    // ---- state accessors used by the view modules -------------------------

    /// The name to show in the title bar.
    /// The window title, built from the `title_format` template.
    ///
    /// The default reads left to right as container, image, position: the order
    /// that answers "where am I?" without needing a second, separate counter.
    pub fn title(&self) -> SharedString {
        let Some(image) = &self.current else {
            return "SeePic".into();
        };
        let (position, total) = self.position();
        let folder = self.container_name().unwrap_or_default();
        let text = self
            .settings
            .title_format
            .replace("${folder}", &folder)
            .replace("${file}", &image.name)
            .replace("${sep}", "·")
            .replace("${page}", &position.to_string())
            .replace("${total}", &total.to_string());
        text.into()
    }

    /// The container's own name, e.g. the comic archive's file name.
    pub fn container_name(&self) -> Option<String> {
        self.container
            .as_ref()
            .and_then(|c| c.lock().ok().map(|c| c.display_name()))
    }

    pub fn container_kind(&self) -> Option<ContainerKind> {
        self.container.as_ref().and_then(|c| c.lock().ok().map(|c| c.kind()))
    }

    /// The open container's path, which is the first half of a cache key.
    pub fn container_path(&self) -> Option<PathBuf> {
        self.container
            .as_ref()
            .and_then(|c| c.lock().ok().map(|c| c.path().to_path_buf()))
    }

    pub fn position(&self) -> (usize, usize) {
        if self.navigator.is_empty() {
            (0, 0)
        } else {
            (self.navigator.cursor() + 1, self.navigator.len())
        }
    }

    pub fn zoom_percent(&self, window: &Window) -> f32 {
        let Some(image) = &self.current else {
            return 100.;
        };
        self.zoom
            .resolve(image.size, self.viewport(window), window.scale_factor())
            .percent(window.scale_factor())
    }

    /// The current image's pixel size and file size, for the status bar.
    ///
    /// `info` is filled in when a page is adopted, so this is available as soon
    /// as the image is.
    pub fn image_facts(&self) -> Option<String> {
        let info = self.info.as_ref()?;
        Some(format!(
            "{} × {} · {}",
            info.width,
            info.height,
            seepic_core::metadata::human_size(info.byte_size)
        ))
    }

    /// The zoom percentage against the canvas the view last measured.
    ///
    /// Panels render without a window, so they cannot ask the layout for the
    /// scale factor; this is the same figure the status bar shows.
    pub fn zoom_label_text(&self) -> String {
        let Some(image) = &self.current else {
            return "—".to_string();
        };
        let bounds = self.canvas_bounds.unwrap_or(image.size);
        let percent = self.zoom.resolve(image.size, bounds, 1.).percent(1.);
        format!("{percent:.0}%")
    }

    /// The area the image is drawn into: the window minus our own chrome.
    pub fn viewport(&self, window: &Window) -> IvSize {
        // Prefer what the canvas actually got during layout; the constant-based
        // estimate is only a first-frame fallback.
        if let Some(size) = self.canvas_bounds {
            if !size.is_empty() {
                return size;
            }
        }
        let bounds = window.bounds().size;
        let chrome = self.chrome_height();
        IvSize::new(
            f32::from(bounds.width).max(1.),
            (f32::from(bounds.height) - chrome).max(1.),
        )
    }

    pub fn has_images(&self) -> bool {
        !self.navigator.is_empty()
    }

    /// The menu row the keyboard is on, if any.
    pub fn highlighted_row(&self) -> Option<usize> {
        self.menu_index
    }

    /// Move the menu highlight by one row, wrapping nowhere.
    ///
    /// The highlight walks the rows the keyboard can act on, so separators are
    /// stepped over rather than landing on nothing.
    pub fn move_menu_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(menu) = self.open_menu else {
            return;
        };
        let rows = menu_bar::actionable_rows(menu);
        if rows.is_empty() {
            return;
        }
        let position = self
            .menu_index
            .and_then(|index| rows.iter().position(|row| *row == index));
        let next = match position {
            Some(position) => {
                (position as isize + delta).clamp(0, rows.len() as isize - 1) as usize
            }
            // First press: start at the top going down, at the bottom going up.
            None => {
                if delta > 0 {
                    0
                } else {
                    rows.len() - 1
                }
            }
        };
        self.menu_index = Some(rows[next]);
        if std::env::var_os("IV_TRACE_KEYS").is_some() {
            eprintln!("menu highlight -> row {}", rows[next]);
        }
        cx.notify();
    }

    /// Run the highlighted menu row: a command runs, a submenu opens.
    pub fn activate_menu_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(menu), Some(index)) = (self.open_menu, self.menu_index) else {
            return;
        };
        let items = menu_bar::menu_items(menu);
        if std::env::var_os("IV_TRACE_KEYS").is_some() {
            eprintln!("menu activate -> row {index}");
        }
        match items.get(index) {
            // A labelled row runs the same command; only its text differs.
            Some(menu_bar::Item::Command(command))
            | Some(menu_bar::Item::Labeled(_, command)) => {
                let command = *command;
                self.close_menu();
                self.execute(command, window, cx);
            }
            Some(menu_bar::Item::Submenu(_, submenu)) => {
                let submenu = *submenu;
                self.open_submenu(submenu, cx);
            }
            _ => {}
        }
    }

    /// Open the list for a settings row.
    pub fn open_choice(&mut self, popup: ChoicePopup, cx: &mut Context<Self>) {
        self.choice = Some(popup);
        cx.notify();
    }

    /// Close the list without choosing anything.
    pub fn close_choice(&mut self, cx: &mut Context<Self>) {
        self.choice = None;
        cx.notify();
    }

    /// Apply the entry at `index` and close the list.
    pub fn apply_choice(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(popup) = self.choice.take() else {
            return;
        };
        (popup.set)(self, index, cx);
        cx.notify();
    }

    /// Open a submenu of the menu that is currently open.
    ///
    /// A submenu has no button of its own to be measured, so it takes over the
    /// anchor of the menu it hangs from: it appears under the same button and
    /// at the same vertical edge as every other dropdown.
    pub fn open_submenu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        if menu.is_submenu()
            && let Some(parent) = self.open_menu
            && let Some(anchor) = self.menu_anchors.get(parent as usize).copied().flatten()
            && let Some(slot) = self.menu_anchors.get_mut(menu as usize)
        {
            *slot = Some(anchor);
        }
        self.open_menu = Some(menu);
        // The keyboard starts fresh at the top of the new list.
        self.menu_index = None;
        self.menu_scroll = VirtualListScrollHandle::new();
        cx.notify();
    }

    /// Open a menu, or close it when it is already the open one.
    ///
    /// Opening also dismisses a docked panel, because the panel is outside the
    /// menu the user just asked for.
    pub fn toggle_menu(&mut self, menu: Menu) {
        if self.open_menu == Some(menu) {
            self.close_menu();
            return;
        }
        self.open_menu = Some(menu);
        // The keyboard starts fresh at the top of the new list.
        self.menu_index = None;
        self.menu_scroll = VirtualListScrollHandle::new();
        self.panel_stack.clear();
    }

    /// Close the menu and forget where the keyboard was.
    pub fn close_menu(&mut self) {
        self.open_menu = None;
        self.menu_index = None;
        self.menu_scroll = VirtualListScrollHandle::new();
    }

    /// Where a panel should appear, relative to the content area.
    ///
    /// `x` is the left edge of the button that opened it; `y` is the strip's
    /// bottom edge, converted to content coordinates (the content area starts
    /// below the chrome when the chrome is locked in place).
    /// Whether the current container came from an archive, so file operations
    /// that only make sense on disk can be disabled.
    pub fn in_archive(&self) -> bool {
        self.container_kind().is_some_and(|k| k.is_archive())
    }

    pub fn slideshow_progress(&self) -> Option<f32> {
        if !self.slideshow.is_running() || !self.slideshow.config().show_progress {
            return None;
        }
        let elapsed = self.slideshow_started.map(|t| t.elapsed())?;
        Some(self.slideshow.progress(elapsed))
    }

    // ---- opening ----------------------------------------------------------

    /// Open a file or folder, and load its first image.
    pub fn open_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else if seepic_core::is_archive_extension(&path.to_string_lossy()) {
            path.to_path_buf()
        } else {
            // A single image: browse the folder it lives in.
            path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf())
        };

        let open_started = Instant::now();
        match Container::open(&target) {
            Ok(container) => {
                crate::perf!(
                    "open container={} entries={} in {:.1}ms",
                    container.display_name(),
                    container.len(),
                    crate::perf::millis_of(open_started)
                );
                // The header cache is keyed by container, and the batch that
                // measured the one being left may not have been written yet.
                self.flush_meta_cache(true);
                // Opening a loose image should land on that image, not on the
                // first file in its folder.
                let wanted = if path.is_file() && !seepic_core::is_archive_extension(&path.to_string_lossy()) {
                    path.file_name().map(|n| n.to_string_lossy().into_owned())
                } else {
                    None
                };

                self.siblings = target
                    .parent()
                    .map(seepic_core::list_containers)
                    .unwrap_or_default();
                self.sibling_index = self
                    .siblings
                    .iter()
                    .position(|c| c.path() == target.as_path());

                let mut navigator =
                    Navigator::with_order(self.sort_order, self.boundary_action());
                navigator.set_entries(container.entries().to_vec());
                if let Some(name) = wanted {
                    if let Some(index) = navigator
                        .entries()
                        .iter()
                        .position(|e| e.name == name)
                    {
                        navigator.select(index);
                    }
                }

                // Reopen where the user stopped reading, if they asked for
                // that and this container has been read before.
                self.position_key =
                    seepic_core::metacache::file_key(&target);
                let remembering = if container.kind().is_archive() {
                    self.settings.remember_archive_position
                } else {
                    self.settings.remember_folder_position
                };
                if remembering {
                    if let Some(key) = self.position_key.clone() {
                        if let Some(place) = self.positions.recall(&key) {
                            // Prefer the name: sorting may have moved the page.
                            let by_name = navigator
                                .entries()
                                .iter()
                                .position(|entry| entry.file_name() == place.entry);
                            if let Some(index) = by_name.or_else(|| {
                                (place.index < navigator.len()).then_some(place.index)
                            }) {
                                navigator.select(index);
                            }
                        }
                    }
                }

                self.navigator = navigator;
                self.release_render_maps();
                self.cache.clear();
                self.thumbnails_requested.clear();
                self.webtoon_requested.clear();
                self.spread_requested.clear();
                self.strip_first = 0;
                // The grid belongs to the container that was open; a new one
                // starts with no stale filter or match list.
                self.contact_sheet = false;
                self.contact_filter.clear();
                self.contact_matches.clear();
                self.container = Some(Arc::new(Mutex::new(container)));
                self.orientation.reset();
                self.zoom.set_auto(seepic_core::FitAxis::Window);
                self.error = None;
                // The previous container's facts must not stand in for the one
                // about to be loaded.
                self.info = None;
                self.anim_frame = None;
                self.anim_playing = true;
                self.page_sizes.clear();
                self.spread_starts.clear();
                self.probe_generation += 1;
                self.probing = false;
                self.navigator_selection = self.navigator.cursor();

                // The layout modes need every page's dimensions, so the probe
                // starts with the container rather than on the first draw.
                if self.view_mode.is_two_page() || self.view_mode.is_webtoon() {
                    self.ensure_page_sizes(cx);
                }

                // Written now so the previous container's place is safe.
                let _ = self.positions.flush();
                self.position_saved_at = None;
                self.load_current(cx);
            }
            Err(error) => {
                self.error = Some(error.to_string().into());
                self.current = None;
                self.info = None;
                self.container = None;
                cx.notify();
            }
        }
    }

    /// The metadata-cache key for the page at `position`, if it can be built.
    ///
    /// A loose file is keyed on its own path, size and time; an archive member
    /// on the archive's, plus the member's name, because an archive does not
    /// always record per-member times.
    fn page_meta_key(&self, position: usize) -> Option<String> {
        let container = self.container.as_ref()?.lock().ok()?;
        let source = self.navigator.source_index(position)?;
        match container.source(source)? {
            seepic_core::Source::File(path) => seepic_core::metacache::file_key(&path),
            seepic_core::Source::ArchiveMember { archive, member } => {
                // The member index stands in for its name: the container keeps
                // the list stable for as long as the archive is unchanged, and
                // the archive's own size and time are in the key too.
                seepic_core::metacache::member_key(&archive, &member.to_string())
            }
        }
    }

    /// The longest side the current image is drawn at, in physical pixels.
    ///
    /// The render pass is the only place that knows the window's scale factor
    /// and the live zoom, so this is only meaningful after a frame has run.
    ///
    /// It is resolved against the *file's* size rather than the decode's. A
    /// reduced decode smaller than the window is not enlarged by "fit", so
    /// resolving against it would make each frame ask for the size the last
    /// decode already produced: the target would crawl up one step per frame
    /// and decode the same image three or four times before settling.
    fn drawn_longest(&self, window: &Window) -> f32 {
        let viewport = self.viewport(window);
        let scale = window.scale_factor();

        // With nothing loaded there is no image to fit, and the window itself
        // is the best estimate of what will be drawn.
        let Some(image) = self.current.as_ref() else {
            return viewport.w.max(viewport.h) * scale;
        };

        let (source_w, source_h) = image.decoded.source_size();
        let decoded_longest = image.size.w.max(image.size.h);
        let factor = if decoded_longest > 0. {
            (source_w.max(source_h)) as f32 / decoded_longest
        } else {
            1.
        };
        // The decoded size carries the rotation, so scaling it up keeps the
        // orientation and aspect the fit has to work with.
        let full = IvSize::new(image.size.w * factor, image.size.h * factor);

        // Webtoon lays every page out at the canvas width and scrolls, so the
        // zoom state does not describe the draw at all: the page is drawn at
        // whatever width the canvas gives it, however far the reader has
        // scrolled and whatever the last single-page zoom was. Sizing the decode
        // from the zoom there decodes several times the pixels that are shown,
        // for every page the scrolling window keeps ready.
        if self.view_mode.is_webtoon() {
            let width = viewport.w * scale;
            let height = width * full.h / full.w.max(1.);
            return width.max(height);
        }

        let drawn = self.zoom.resolve(full, viewport, scale).drawn;
        drawn.w.max(drawn.h) * scale
    }

    /// How close together two page turns have to be to count as a burst.
    ///
    /// Short enough that the page sharpens soon after the key comes up, long
    /// enough for a held key: Windows repeats at about 31 ms.
    const BURST_GAP: Duration = Duration::from_millis(120);

    /// Whether a background decode may start right now.
    ///
    /// The visible page always may; the neighbours, the prepared pages and the
    /// prefetch share what is left of the worker count. One worker means the
    /// old behaviour — nothing but the visible decode runs — and two lets one
    /// neighbour decode beside it, which is what keeps a held key at the rate
    /// of its own decode instead of one page per decode.
    fn decode_slot_free(&self) -> bool {
        let workers = self.settings.decode_workers() as usize;
        self.prefetching.len() + usize::from(self.loading) < workers
    }

    /// Whether pages are being turned in a burst.
    pub fn paging_fast(&self) -> bool {
        self.last_step_at.is_some_and(|at| at.elapsed() < Self::BURST_GAP)
    }

    /// The target to decode at: the size the page is drawn at.
    ///
    /// A burst used to decode at a quarter of that, to keep up with a held key,
    /// and then decode the page the reader stopped on a second time. Measured
    /// over 4000–6000 px photographs, the quarter-size decode saved only about
    /// 10 ms a page (35 ms against 45 ms — the entropy stage reads the same file
    /// either way) and cost a page that was visibly soft until it was replaced
    /// under the reader's eyes. Sharp from the first frame is worth those ten
    /// milliseconds, so there is one target and it is the display's.
    fn decode_target_now(&self) -> Option<u32> {
        self.decode_target
    }

    /// Decode the pages a burst will run into, while the viewer is idle.
    ///
    /// The first hold of the arrow key is otherwise one decode per page, with
    /// the twenty-megapixel progressive files costing a third of a second each.
    /// Preparing a few pages ahead — at the size they will be drawn, not at a
    /// blurry thumbnail — is what makes that first pass as smooth as the second.
    fn prepare_previews(&mut self, cx: &mut Context<Self>) {
        if !self.settings.cache_next || self.loading || self.paging_fast() || self.window_minimized
        {
            return;
        }
        if !self.decode_slot_free() {
            return;
        }
        // Full resolution is asked for only when the reader has zoomed past what
        // a reduced decode can give; preparing whole originals is not worth it.
        let Some(target) = self.decode_target else {
            return;
        };
        let Some(container) = self.container.clone() else {
            return;
        };
        let Some(path) = container.lock().ok().map(|c| c.path().to_path_buf()) else {
            return;
        };
        let cursor = self.navigator.cursor();
        // A prepared page is the size of the drawn one — 5–8 MB of an ordinary
        // photo — so the page count is also a memory figure, and the byte cap
        // below keeps a large setting from turning into a large heap.
        const PREPARE_BYTES: usize = 192 * 1024 * 1024;
        let per_page = (target as usize).pow(2) * 4;
        // Do not prepare more pages than the cache can keep. Anything beyond
        // that is decoded, evicted, and decoded again on the next pass — measured
        // as 281 decodes in twelve idle seconds before this bound.
        const PREFETCH_RESERVE: usize = 3;
        let room = self
            .cache
            .capacity()
            .saturating_div(per_page.max(1))
            .saturating_sub(PREFETCH_RESERVE);
        let ahead = (self.settings.prepare_pages() as usize)
            .min(PREPARE_BYTES / per_page.max(1))
            .min(room)
            .max(1);
        // The cache also holds the page on screen and the prefetched
        // neighbours; stop while the window is still inside the room left.
        if self.cache.len() >= ahead {
            return;
        }
        for step in 1..=ahead {
            let Some(position) = cursor.checked_add(step) else {
                break;
            };
            if position >= self.navigator.len() {
                break;
            }
            let Some(source) = self.navigator.source_index(position) else {
                continue;
            };
            let key = CacheKey::new(&path, source);
            if self.cache.get_for_detail(&key, target).is_some() {
                continue;
            }
            if !self.prefetching.insert(key.clone()) {
                continue;
            }
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn_with_priority(Priority::Low, async move {
                        let bytes = container.lock().ok()?.read(source).ok()?;
                        // An animated WebP is streamed when it is actually
                        // shown; decoding one whole just because it is nearby
                        // would spend seconds and a gigabyte of pixels.
                        if decode::is_animated_webp(&bytes) {
                            return None;
                        }
                        let byte_size = bytes.len() as u64;
                        // Deliberately not `move_to_swap`: a prepared page is
                        // the size of the view, so it would take a whole slot
                        // and leave the visible decode without one. The
                        // preparation budget bounds them instead.
                        decode::decode_for_display(&bytes, true, Some(target))
                            .ok()
                            .map(|image| (Arc::new(image), byte_size))
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.prefetching.remove(&key);
                    if let Some((image, byte_size)) = decoded {
                        let detail = image.detail();
                        this.cache.insert(key, CachedImage { image, byte_size, detail });
                        crate::perf!("prep keep target={target} cursor={}", this.navigator.cursor());
                    } else {
                        crate::perf!("prep drop target={target}");
                    }
                    // The preparation runs one page at a time, and an idle
                    // window draws no frames of its own — without asking for one
                    // here it would stop after the first pages and never catch
                    // up.
                    cx.notify();
                })
                .ok();
            })
            .detach();
            // One at a time: the visible decode still has priority.
            return;
        }
    }

    /// How much detail the canvas can show, in physical pixels.
    ///
    /// This is the true requirement, not a wish: the decoder is free to keep a
    /// little more than it is asked for — and the JPEG path keeps up to twice as
    /// much, because that is the next step of its DCT ladder — which is what
    /// gives a modest zoom-in its headroom. Asking for more than the view needs
    /// would only make the cache miss on the extra detail.
    fn compute_decode_target(&self, window: &Window) -> Option<u32> {
        // Only when the user asked for previews first; otherwise the full
        // resolution is decoded and the mip pyramid does the downscaling.
        if !self.settings.low_quality_preview {
            return None;
        }
        // An animation keeps every frame, so reducing multiplies the saving but
        // also the visible cost; those are usually small and left alone.
        if self.current.as_ref().is_some_and(|image| image.decoded.animated) {
            return None;
        }

        let longest = self.drawn_longest(window);
        // The ceiling is the setting's, and it is never below the floor, so the
        // clamp below cannot be handed a reversed pair.
        Some((longest.round() as u32).clamp(MIN_DECODE_SIDE, self.settings.decode_max_edge()))
    }

    /// Whether the image on screen holds less detail than the view now needs.
    ///
    /// True after zooming into a reduced decode, which is the signal to decode
    /// again at full resolution.
    pub fn needs_more_detail(&self) -> bool {
        let Some(image) = self.current.as_ref() else {
            return false;
        };
        // Already full resolution, so nothing to upgrade.
        let Some(detail) = image.detail else {
            return false;
        };
        match self.decode_target {
            // Full resolution asked for, so a reduced decode is short of it.
            None => true,
            // A margin stops a borderline zoom from upgrading; the `None` the
            // upgrade produces ends the loop.
            Some(target) => target > detail + detail / 8,
        }
    }

    /// Decode the current page at full detail once the reader has stopped.
    ///
    /// A page shown during a burst is a preview, and nothing else asks for
    /// another frame after the burst ends. A single timer can fire inside the
    /// burst window, be skipped, and leave a blurry page on screen for good, so
    /// this keeps checking for as long as the reader keeps flipping: the page
    /// they finally stop on is the one that has to sharpen. It also cannot stop
    /// just because a request was made once — a page can be adopted from the
    /// cache after the request, and that adopt is another preview.
    fn schedule_upgrade(&mut self, cx: &mut Context<Self>) {
        if self.upgrade_scheduled || !self.needs_more_detail() {
            return;
        }
        self.upgrade_scheduled = true;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            // About five seconds, so a decode that keeps failing cannot leave a
            // timer running for the rest of the session.
            for _ in 0..40 {
                executor.timer(Self::BURST_GAP).await;
                let settled = this
                    .update(cx, |this, cx| {
                        if !this.needs_more_detail() {
                            return true;
                        }
                        if !this.paging_fast() && !this.loading {
                            this.upgrade_detail(cx);
                        }
                        false
                    })
                    .unwrap_or(true);
                if settled {
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.upgrade_scheduled = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Free a swap slot for the page about to be decoded, if the store is full.
    ///
    /// Without this the store saturates: every slot is held by a cached frame,
    /// the cache only evicts when something is inserted, and an insert needs a
    /// slot — so the cache freezes on whatever it last held and every new page
    /// is shown once and forgotten. Measured on a held key: `frames=0/1`,
    /// `mapped=128MB` for every page after the first eight.
    fn make_room_for_decode(&mut self, cursor: usize) {
        let config = self.settings.swap_config();
        if !config.enabled || config.slot_bytes == 0 {
            return;
        }
        let stats = self.swap.stats();
        if stats.mapped_bytes + config.slot_bytes <= config.capacity_bytes {
            return;
        }
        let Some(source) = self.navigator.source_index(cursor) else {
            return;
        };
        self.cache.evict_farthest_from(source);
    }

    /// Re-decode the current image at full resolution.
    pub fn upgrade_detail(&mut self, cx: &mut Context<Self>) {
        // During a burst the page on screen is about to be replaced; re-decoding
        // it at full size would only compete with the flip that is coming. A
        // minimised window has nobody to sharpen for either.
        if self.paging_fast()
            || self.window_minimized
            || !self.needs_more_detail()
            || self.upgrade_in_flight
        {
            return;
        }
        crate::perf!(
            "upgrade index={} detail={:?}",
            self.navigator.cursor(),
            self.current.as_ref().and_then(|i| i.detail)
        );
        // The claim on full detail is made when the decoded image is adopted,
        // not here: a request that never lands — one folded into a newer load,
        // say — must not leave the page marked as sharp while it is not.
        self.load_current(cx);
        // `load_current` either started a load, folded this request into one, or
        // served the page from the cache; in the first two cases a decode is
        // running and the flag keeps the next frame from asking again.
        self.upgrade_in_flight = self.loading;
    }

    /// Re-read and decode the image at the navigator's cursor.
    pub fn load_current(&mut self, cx: &mut Context<Self>) {
        let Some(container) = self.container.clone() else {
            return;
        };
        // Before the first frame there is no layout to size the decode against,
        // so anything decoded now would be sized by guesswork — either thrown
        // away and decoded again, or more pixels than the window can show. The
        // render pass calls back here once it has measured the window.
        if !self.layout_measured {
            self.load_pending = true;
            return;
        }
        // The cursor is a position in the sorted list; the container indexes its
        // own order, so translate before reading.
        let Some(source) = self.navigator.current_source() else {
            return;
        };
        let name: SharedString = self
            .navigator
            .current()
            .map(|entry| entry.file_name().to_string())
            .unwrap_or_default()
            .into();

        let Some(container_path) = container.lock().ok().map(|c| c.path().to_path_buf()) else {
            return;
        };
        let key = CacheKey::new(&container_path, source);
        let target = self.decode_target_now();
        let needed = target.unwrap_or(u32::MAX);

        // A cached decode skips the read and all of the pixel work, as long as
        // it holds enough detail for what is on screen right now.
        if let Some(cached) = self.cache.get_for_detail(&key, needed) {
            self.loading = false;
            self.error = None;
            crate::perf!(
                "load position={} cache=hit detail={:?}",
                self.navigator.cursor(),
                cached.detail
            );
            let position = self.navigator.cursor();
            let frames = cached.image.frames.len();
            self.adopt_decoded(
                key,
                position,
                name,
                cached.image,
                cached.byte_size,
                cached.detail,
                true,
                frames,
                cx,
            );
            // A page served from the cache can be one of the prepared burst
            // previews, which is too small for the window it is drawn in. The
            // burst may still be running, so the upgrade waits for it.
            self.schedule_upgrade(cx);
            self.prefetch_around(cx);
            cx.notify();
            return;
        }

        // A page turn while a decode is in flight does not start a second one.
        // Holding the arrow key would otherwise run one decode per press —
        // several full-size decodes at once, every one of them discarded on
        // arrival but each holding its own buffers — and that is what makes
        // fast paging cost hundreds of megabytes. The newest page wins: the
        // in-flight result is installed for the cache and a fresh load starts
        // as soon as it is done.
        if self.loading {
            self.load_pending = true;
            return;
        }

        self.loading = true;
        self.error = None;
        // A full swap store cannot take another frame, and the cache cannot
        // evict to make room because it only evicts on insert. Free the page
        // farthest from the cursor here, so the reaper has the decode's own
        // runtime to release the slot.
        self.make_room_for_decode(self.navigator.cursor());
        // Rapid paging starts a load per press; only the newest one may install
        // its image, or a slow decode could land on top of a newer page.
        self.load_generation += 1;
        let generation = self.load_generation;
        let cursor = self.navigator.cursor();
        crate::perf!("load position={cursor} cache=miss target={target:?}");
        cx.notify();

        let swap = self.swap.clone();
        cx.spawn(async move |this, cx| {
            // Read first, on the background thread: a stream needs to own the
            // bytes, and the read is part of the time to the first frame.
            let read = cx
                .background_executor()
                .spawn(async move {
                    let read_started = Instant::now();
                    container
                        .lock()
                        .map_err(|_| LoadFailure::Other("压缩包状态已损坏".to_string()))
                        .and_then(|mut c| {
                            c.read(source).map_err(|error| match error {
                                seepic_core::ContainerError::Archive(
                                    seepic_core::ArchiveError::PasswordRequired
                                    | seepic_core::ArchiveError::WrongPassword,
                                ) => LoadFailure::Password,
                                other => LoadFailure::Other(other.to_string()),
                            })
                        })
                        .map(|bytes| (bytes, crate::perf::millis_of(read_started)))
                })
                .await;
            let (bytes, read_ms) = match read {
                Ok(read) => read,
                Err(failure) => {
                    this.update(cx, |this, cx| this.load_failed(failure, cursor, generation, cx))
                        .ok();
                    return;
                }
            };
            let byte_size = bytes.len() as u64;
            // Everything from here to the first adopt is what the reader waits
            // for; the read stays separate in the trace.
            let decode_started = Instant::now();

            // An animated WebP streams: the first chunk is shown and playing
            // while the rest is still being decoded. Everything else decodes
            // whole, as before.
            let opening = cx
                .background_executor()
                .spawn(async move {
                    if !decode::is_animated_webp(&bytes) {
                        return Opening::Still(bytes);
                    }
                    match decode::WebPAnimation::new(bytes) {
                        Some(stream) => Opening::Stream(stream),
                        None => Opening::Failed("WebP 动画无法解码".to_string()),
                    }
                })
                .await;

            let mut stream = match opening {
                Opening::Stream(stream) => stream,
                Opening::Still(bytes) => {
                    let started = Instant::now();
                    let decoded = cx
                        .background_executor()
                        .spawn(async move {
                            let peak_before = crate::trace_alloc::peak_bytes();
                            decode::decode_for_display(&bytes, true, target)
                                .map(|mut image| {
                                    let moved = image.move_to_swap(&swap);
                                    crate::perf!(
                                        "swap position={cursor} frames={moved}/{} mapped={}MB heap={}MB                                  peak={}MB (+{}MB by this decode)",
                                        image.frames.len(),
                                        swap.stats().mapped_bytes / (1024 * 1024),
                                        crate::trace_alloc::live_bytes() / (1024 * 1024),
                                        crate::trace_alloc::peak_bytes() / (1024 * 1024),
                                        crate::trace_alloc::peak_bytes()
                                            .saturating_sub(peak_before)
                                            / (1024 * 1024)
                                    );
                                    Arc::new(image)
                                })
                                .map_err(|e| LoadFailure::Other(e.to_string()))
                        })
                        .await;

                    this.update(cx, |this, cx| {
                        this.finish_whole_load(
                            decoded, key, cursor, name, generation, started, read_ms, byte_size,
                            cx,
                        );
                    })
                    .ok();
                    return;
                }
                Opening::Failed(message) => {
                    this.update(cx, |this, cx| {
                        this.load_failed(LoadFailure::Other(message), cursor, generation, cx)
                    })
                    .ok();
                    return;
                }
            };

            // The first chunk is enough to show the image and start its clock.
            let store = swap.clone();
            let (opened, first) = cx
                .background_executor()
                .spawn(async move {
                    let result = stream.decode_chunk(ANIMATION_CHUNK_FRAMES, Some(&store));
                    (stream, result)
                })
                .await;
            stream = opened;
            if let Err(error) = first {
                this.update(cx, |this, cx| {
                    this.load_failed(LoadFailure::Other(error.to_string()), cursor, generation, cx)
                })
                .ok();
                return;
            }

            let total = stream.total_frames();
            let snapshot = Arc::new(stream.snapshot());
            let shown = this
                .update(cx, |this, cx| {
                    let fresh = this.load_generation == generation;
                    if fresh {
                        this.loading = false;
                    }
                    let cursor_now = this.navigator.cursor();
                    let on_cursor = cursor_now == cursor;
                    let show = fresh
                        && this.displayed_cursor.is_none_or(|shown| {
                            cursor.abs_diff(cursor_now) <= shown.abs_diff(cursor_now)
                        });
                    // A turn taken while this load ran was held back; now that
                    // the slot is free, start it here.
                    if this.load_pending {
                        this.load_pending = false;
                        this.load_current(cx);
                    }
                    let first_ms = crate::perf::millis_of(decode_started);
                    crate::perf!(
                        "stream position={cursor} read={read_ms:.1}ms first={first_ms:.1}ms \
                         {}x{} frames={}/{} bytes={byte_size}",
                        snapshot.width,
                        snapshot.height,
                        snapshot.frames.len(),
                        total
                    );
                    if show {
                        this.last_load_ms = Some(first_ms);
                        let detail = snapshot.detail();
                        this.adopt_decoded(
                            key.clone(),
                            cursor,
                            name.clone(),
                            snapshot,
                            byte_size,
                            detail,
                            on_cursor,
                            total,
                            cx,
                        );
                        this.prefetch_around(cx);
                        this.schedule_upgrade(cx);
                    }
                    show
                })
                .unwrap_or(false);
            if !shown {
                // Nobody is looking at it: decoding another 1.2 GB in the
                // background would only compete with the page that is.
                return;
            }

            while !stream.is_done() {
                let store = swap.clone();
                let (opened, result) = cx
                    .background_executor()
                    .spawn(async move {
                        let result = stream.decode_chunk(ANIMATION_CHUNK_FRAMES, Some(&store));
                        (stream, result)
                    })
                    .await;
                stream = opened;
                if let Err(error) = result {
                    // Keep the frames that did arrive; a broken frame is not a
                    // reason to take the picture away.
                    this.update(cx, |this, cx| {
                        if this.load_generation == generation {
                            this.push_toast(format!("动画解码中断：{error}"), true, cx);
                        }
                    })
                    .ok();
                    break;
                }

                let snapshot = Arc::new(stream.snapshot());
                let total = stream.total_frames();
                let done = stream.is_done();
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.load_generation != generation {
                            return false;
                        }
                        // The page may have been replaced by a cached one
                        // without the generation changing; frames of the page
                        // that is no longer on screen must not be drawn there.
                        if !this.apply_stream_frames(key.index, snapshot.clone(), total, cx) {
                            return false;
                        }
                        crate::perf!(
                            "stream position={cursor} frames={}/{} mapped={}MB heap={}MB",
                            snapshot.frames.len(),
                            total,
                            this.swap.stats().mapped_bytes / (1024 * 1024),
                            crate::trace_alloc::live_bytes() / (1024 * 1024)
                        );
                        // Now that the whole animation is here it can be cached
                        // like any other page.
                        if done && this.cacheable(&snapshot) {
                            this.cache.insert(
                                key.clone(),
                                CachedImage {
                                    image: snapshot.clone(),
                                    byte_size,
                                    detail: snapshot.detail(),
                                },
                            );
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Install the result of a whole-image decode.
    #[allow(clippy::too_many_arguments)]
    fn finish_whole_load(
        &mut self,
        decoded: Result<Arc<DecodedImage>, LoadFailure>,
        key: CacheKey,
        cursor: usize,
        name: SharedString,
        generation: u64,
        started: Instant,
        read_ms: f32,
        byte_size: u64,
        cx: &mut Context<Self>,
    ) {
        // A newer load owns the view; this result still goes into the cache,
        // because the user may well turn back to it.
        let fresh = self.load_generation == generation;
        if fresh {
            self.loading = false;
        }
        // The cursor may have moved on while this decode ran. Showing the
        // result is still right when it is closer to where the user is now than
        // what is on screen: holding the arrow key then flips through the pages
        // as they decode rather than freezing on one image until the key is
        // released. Results further away than the page already shown are left
        // in the cache.
        let cursor_now = self.navigator.cursor();
        let on_cursor = cursor_now == cursor;
        let show = fresh
            && self
                .displayed_cursor
                .is_none_or(|shown| cursor.abs_diff(cursor_now) <= shown.abs_diff(cursor_now));
        // A turn taken while this decode ran was held back; now that the slot
        // is free, start it here rather than waiting for the next frame.
        if self.load_pending {
            self.load_pending = false;
            self.load_current(cx);
        }
        match decoded {
            Ok(image) => {
                let decode_ms = crate::perf::millis_of(started);
                if show {
                    self.last_load_ms = Some(decode_ms);
                }
                crate::perf!(
                    "decode position={cursor} read={read_ms:.1}ms decode={decode_ms:.1}ms \
                     {}x{} bytes={byte_size}",
                    image.width,
                    image.height
                );
                // The actual reduction, not the requested one: an image small
                // enough to keep in full holds full detail.
                let detail = image.detail();
                // With the swap store on, a frame it would not take is left on
                // the heap; caching it would mean holding the image twice over,
                // in two different backends. Those are shown once and not kept.
                if self.cacheable(&image) {
                    self.cache.insert(
                        key.clone(),
                        CachedImage {
                            image: image.clone(),
                            byte_size,
                            detail,
                        },
                    );
                }
                if show {
                    // A page the user has already paged past is shown without
                    // claiming a place in the texture cache.
                    let total = image.frames.len();
                    self.adopt_decoded(
                        key, cursor, name, image, byte_size, detail, on_cursor, total, cx,
                    );
                    self.prefetch_around(cx);
                    self.schedule_upgrade(cx);
                }
            }
            Err(failure) => self.load_failed(failure, cursor, generation, cx),
        }
        cx.notify();
    }

    /// Put the viewer into the state a failed load leaves behind.
    fn load_failed(
        &mut self,
        failure: LoadFailure,
        cursor: usize,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        // A newer load owns the view; this failure must not clear its flag.
        if self.load_generation == generation {
            self.loading = false;
        }
        self.upgrade_in_flight = false;
        match failure {
            LoadFailure::Password => {
                self.error = None;
                self.current = None;
                self.info = None;
                // A second failure means the previous password was wrong.
                let wrong = self.password_tried;
                self.password_prompt = Some(PasswordPrompt {
                    archive: self.container_name().unwrap_or_default(),
                    input: String::new(),
                    wrong,
                });
            }
            LoadFailure::Other(message) => {
                // An error for a page the user has already left is not worth a
                // toast; the page they are on gets its own try.
                if self.navigator.cursor() == cursor {
                    self.error = Some(message.clone().into());
                    self.current = None;
                    self.info = None;
                    self.push_toast(message, true, cx);
                }
            }
        }
        // A turn taken while this load ran was held back; start it now that
        // this one is over, or it would wait for the next unrelated event.
        if self.load_pending {
            self.load_pending = false;
            self.load_current(cx);
        }
        cx.notify();
    }

    /// Install a newer chunk of a streaming animation.
    ///
    /// Returns whether the chunk belonged to the page on screen; the caller
    /// stops streaming when it did not.
    fn apply_stream_frames(
        &mut self,
        expected: usize,
        decoded: Arc<DecodedImage>,
        total: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        if current.index != expected {
            return false;
        }
        let before = current.decoded.frames.len();
        current.decoded = decoded;
        current.total_frames = total.max(current.decoded.frames.len());
        // A frame the user asked for — or one playback was waiting on — may
        // just have arrived. Show it now rather than at the next timer tick.
        let waiting = self.anim_frame.is_some_and(|index| index >= before);
        if waiting && !self.window_minimized {
            self.refresh_transform(cx);
        }
        cx.notify();
        true
    }

    /// Install a freshly decoded image and reset the view to fit it.
    #[allow(clippy::too_many_arguments)]
    fn adopt_decoded(
        &mut self,
        key: CacheKey,
        position: usize,
        name: SharedString,
        decoded: Arc<DecodedImage>,
        byte_size: u64,
        detail: Detail,
        keep_render: bool,
        total_frames: usize,
        cx: &mut Context<Self>,
    ) {
        let index = key.index;
        self.displayed_cursor = Some(position);
        // A new image starts playing from the top, whatever the last one did.
        self.anim_generation += 1;
        self.anim_frame = None;
        self.anim_playing = true;
        // A long animation is not handed to GPUI whole: the built image would
        // hold a copy of every frame. It starts on its first frame, and the
        // viewer advances it (see `schedule_anim_frame`). A streaming animation
        // is always driven that way, because only the frames decoded so far
        // exist and more are on the way.
        let streaming = total_frames > decoded.frames.len();
        let self_driven = streaming
            || (decoded.frames.len() > 1
                && decoded.frames.iter().map(|frame| frame.bgra.len()).sum::<usize>()
                    > ANIMATION_SELF_DRIVEN_BYTES);
        let pick = if self_driven {
            FramePick::One(0)
        } else {
            FramePick::All
        };
        // The image being replaced is about to leave the screen. A cached
        // build belongs to the render cache, which releases it when it is
        // evicted; one built for a page that was only passing through is ours
        // to hand back, which `release_current_textures` does.
        self.release_current_textures();

        // Whatever was in flight has arrived; the next frame decides afresh
        // whether this page still needs more detail.
        self.upgrade_in_flight = false;
        // Eviction keeps the pages around the reader, not the ones just read.
        self.cache
            .set_focus(self.navigator.source_index(position));
        let started = Instant::now();
        let Some((render, _decoded_size, render_cached)) = self.render_for(
            key.clone(),
            pick,
            &decoded,
            detail,
            keep_render,
        ) else {
            self.error = Some("图像数据无效".into());
            return;
        };

        self.password_tried = false;
        self.orientation.reset();
        self.reset_zoom_for_view_mode();
        // How large the page is on screen is the file's own size, not the
        // decode's. A decode reduced for the window — or made half-size while
        // the reader is flipping through — must not change the page's size, or
        // fit-to-window and 100% would move with every decode.
        let source = decoded.source_size();
        let size = self.orientation.applied_size(IvSize::new(
            source.0 as f32,
            source.1 as f32,
        ));
        let decoded_size = (decoded.width, decoded.height);
        let average_color = decoded.average_color();
        let blur = decoded
            .blur_source(BLUR_BACKDROP_SIDE)
            .and_then(|(width, height, bytes)| render_image(width, height, bytes));
        let total_frames = total_frames.max(decoded.frames.len());
        self.current = Some(CurrentImage {
            index,
            render,
            render_cached,
            size,
            decoded,
            total_frames,
            name,
            byte_size,
            mips: None,
            mips_requested: false,
            detail,
            average_color,
            blur,
            self_driven,
        });
        if self_driven {
            self.anim_frame = Some(0);
        }
        // The image on screen must not be evicted while it is drawn.
        self.render_cache
            .pin(Some(RenderKey::new(key, self.render_transform(), pick)));
        // The status bar's facts come from the image just adopted, so they are
        // current on every page turn; only the EXIF read waits for the burst
        // to end (see `refresh_info`).
        self.refresh_info(cx);
        if self_driven {
            self.schedule_anim_frame(cx);
        }
        // A new animated page brings its controls up; they withdraw on their
        // own once the pointer has been still.
        self.anim_bar_hovered = false;
        self.reveal_anim_bar(cx);
        let (cache_mb, held_mb) = self.pixel_megabytes();
        let swap = self.swap.stats();
        crate::perf!(
            "adopt index={index} {}x{} detail={detail:?} build={:.1}ms cache={cache_mb:.0}MB held={held_mb:.0}MB              swap={}MB slots={} mapped={} heap={}MB peak={}MB",
            decoded_size.0,
            decoded_size.1,
            crate::perf::millis_of(started),
            swap.mapped_bytes / (1024 * 1024),
            swap.slot_files,
            swap.live_frames,
            crate::trace_alloc::live_bytes() / (1024 * 1024),
            crate::trace_alloc::peak_bytes() / (1024 * 1024)
        );
        cx.notify();
    }

    /// The transform the current settings ask for.
    ///
    /// Everything here changes the built pixels rather than the source, so it
    /// belongs in the render cache's key.
    fn render_transform(&self) -> Transform {
        Transform {
            orientation: self.orientation,
            adjustments: self.adjustments,
            filter: self.filter,
            flatten_alpha: self.transparency.flattens_alpha(),
        }
    }

    /// A built, drawable image for `source`, reusing a cached one when it fits.
    ///
    /// This is what keeps a page turn from committing a fresh GPU texture: the
    /// same key hands back the same `RenderImage`, so GPUI's sprite atlas finds
    /// the entry it already has instead of allocating another one.
    fn render_for(
        &mut self,
        source: CacheKey,
        frame: FramePick,
        decoded: &Arc<DecodedImage>,
        detail: Detail,
        keep: bool,
    ) -> Option<(Arc<RenderImage>, IvSize, bool)> {
        let key = RenderKey::new(source, self.render_transform(), frame);
        if let Some((render, size)) = self.render_cache.get_if_covers(&key, detail) {
            return Some((render, size, true));
        }

        let held = match frame {
            FramePick::All => None,
            FramePick::One(index) => Some(index),
        };
        let (render, size) = build_render(
            decoded,
            &self.orientation,
            &self.adjustments,
            self.filter,
            held,
            self.transparency.flattens_alpha(),
        )?;
        if !keep {
            // Shown once and gone: caching it would evict a page the user may
            // come back to, for a texture that is about to be thrown away.
            return Some((render, size, false));
        }
        let evicted = self.render_cache.insert(key.clone(), render.clone(), size, detail);
        self.release_textures(evicted);
        // An image the budget cannot hold — a full-size 22-megapixel page, say
        // — is refused rather than cached, so whoever shows it has to release
        // it; asking the cache is the only way to tell the two apart.
        let cached = self.render_cache.get_if_covers(&key, detail).is_some();
        Some((render, size, cached))
    }

    /// Put the zoom where the current browse mode wants it.
    pub fn reset_zoom_for_view_mode(&mut self) {
        // The layout modes own their own scaling, so the ordinary zoom state
        // stays out of the way.
        if !self.view_mode.is_single_page() {
            self.zoom.set_auto(seepic_core::FitAxis::Window);
            return;
        }
        // A pinned zoom from the settings wins; Auto means fit to the window.
        match self.settings.zoom_preset.factor() {
            Some(factor) => {
                self.zoom.set_factor(factor);
                self.zoom.center();
            }
            None => {
                let axis = self.view_mode.fit_axis().unwrap_or(seepic_core::FitAxis::Window);
                self.zoom.set_auto(axis);
            }
        }
    }

    /// Rebuild the drawable image after a rotation or flip.
    ///
    /// The build goes through the render cache, so rotating a full turn and
    /// coming back to a transform that was already built is instant and reuses
    /// the texture rather than committing another one.
    pub fn refresh_transform(&mut self, cx: &mut Context<Self>) {
        let Some(image) = &self.current else { return };
        let decoded = image.decoded.clone();
        let index = image.index;
        let name = image.name.clone();
        let byte_size = image.byte_size;
        // None of these follow the transform, so they are carried over.
        let average_color = image.average_color;
        let blur = image.blur.clone();
        let detail = image.detail;
        let self_driven = image.self_driven;
        let total_frames = image.total_frames;

        let Some(container_path) = self.container_path() else {
            return;
        };
        let source = CacheKey::new(&container_path, index);
        let frame = match self.anim_frame {
            Some(index) => FramePick::One(index),
            None => FramePick::All,
        };
        let Some((render, _decoded_size, render_cached)) =
            self.render_for(source.clone(), frame, &decoded, detail, true)
        else {
            return;
        };
        let source_size = decoded.source_size();
        let size = self
            .orientation
            .applied_size(IvSize::new(source_size.0 as f32, source_size.1 as f32));
        // The transform changed the pixels, so the old pyramid is no longer
        // drawn. The blur backdrop does not follow the transform and is carried
        // over (so it must not be released here), and a built image the cache
        // owns is released when it is evicted; one the cache refused is handed
        // back here.
        if let Some(old) = self.current.take() {
            let mut textures: Vec<Arc<RenderImage>> = old
                .mips
                .into_iter()
                .flatten()
                .map(|level| level.render)
                .collect();
            if !old.render_cached {
                textures.push(old.render);
            }
            self.release_textures(textures);
        }
        self.current = Some(CurrentImage {
            index,
            render,
            render_cached,
            size,
            decoded,
            total_frames,
            name,
            byte_size,
            mips: None,
            mips_requested: false,
            detail,
            average_color,
            blur,
            self_driven,
        });
        self.render_cache
            .pin(Some(RenderKey::new(source, self.render_transform(), frame)));
        cx.notify();
    }

    /// Build the downscale pyramid the view needs, off the UI thread.
    ///
    /// Sampling a large texture straight down to a fitted draw with the GPU's
    /// bilinear filter aliases, because it only reads four texels; drawing a
    /// level already close to the on-screen size avoids that. Building the whole
    /// chain costs tens of milliseconds, though, and doing it here stalled every
    /// page turn. It now runs in the background and the full-resolution image is
    /// drawn for the frame or two until it lands.
    ///
    /// The chain is also only worth having when the image is drawn well below
    /// its own size. With the default decode headroom the texture is already
    /// close to the draw, so most page turns skip this entirely.
    pub fn ensure_mips(&mut self, cx: &mut Context<Self>) {
        if self.window_minimized {
            return;
        }
        let Some(image) = self.current.as_ref() else {
            return;
        };
        if image.mips.is_some() || image.mips_requested {
            return;
        }

        // Frame sizes can differ inside an animation, so a shared pyramid would
        // be wrong there; animated images simply draw at full resolution.
        if image.decoded.animated {
            if let Some(image) = self.current.as_mut() {
                image.mips = Some(Vec::new());
            }
            return;
        }

        let texture = image.size.w.max(image.size.h);
        if texture <= 0. || self.drawn_longest >= texture * MIP_TRIGGER_SCALE {
            return;
        }

        // The pyramid must match what is drawn, which is the decoded pixels
        // after the user's rotation and adjustments. When none of those are in
        // play the render *is* the decode, so the shared copy can be used
        // instead of reading and copying the whole texture.
        let untouched = self.orientation.is_identity()
            && self.adjustments.is_neutral()
            && self.filter.is_identity()
            && !self.transparency.flattens_alpha();
        let source = if untouched {
            MipSource::Decoded(image.decoded.clone())
        } else {
            match image.render.as_bytes(0) {
                Some(bytes) => MipSource::Pixels(bytes.to_vec()),
                None => return,
            }
        };

        let (width, height) = (image.size.w as u32, image.size.h as u32);
        let kernel = self.interpolation;
        if let Some(image) = self.current.as_mut() {
            image.mips_requested = true;
        }

        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let levels = cx
                .background_executor()
                .spawn(async move {
                    let pixels = source.pixels();
                    let Some(pixels) = pixels else {
                        return Vec::new();
                    };
                    decode::mip_chain(width, height, pixels, kernel)
                })
                .await;

            this.update(cx, |this, cx| {
                let Some(image) = this.current.as_mut() else {
                    return;
                };
                if image.mips.is_some() {
                    return;
                }
                // `render_image` only moves the buffer, so wrapping each level
                // into a texture here costs nothing measurable; the upload
                // happens on the next paint.
                image.mips = Some(
                    levels
                        .into_iter()
                        .filter_map(|(w, h, bgra)| {
                            Some(MipLevel {
                                render: render_image(w, h, bgra)?,
                                size: IvSize::new(w as f32, h as f32),
                            })
                        })
                        .collect(),
                );
                crate::perf!(
                    "mips ready {}x{} levels={} {:.1}ms after request",
                    width,
                    height,
                    image.mips.as_ref().map(Vec::len).unwrap_or(0),
                    crate::perf::millis_of(started)
                );
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What to draw at `scale` logical pixels per image pixel, and the pixel size
    /// of that texture.
    pub fn render_for_scale(
        &self,
        scale: f32,
        window_scale_factor: f32,
    ) -> Option<(&Arc<RenderImage>, IvSize)> {
        let image = self.current.as_ref()?;
        let Some(levels) = image.mips.as_ref() else {
            return Some((&image.render, image.size));
        };
        if levels.is_empty() {
            return Some((&image.render, image.size));
        }

        // Levels run large to small, so the last one still at least as wide as
        // the draw is the sharpest one we can use without sampling up.
        let wanted = image.size.w * scale * window_scale_factor;
        let mut chosen = (&image.render, image.size);
        for level in levels {
            if level.size.w >= wanted {
                chosen = (&level.render, level.size);
            } else {
                break;
            }
        }
        Some(chosen)
    }

    /// Select the image at a display position.
    /// How many images the current container holds.
    pub fn entry_count(&self) -> usize {
        self.navigator.len()
    }

    /// Which display position the cursor is on.
    pub fn cursor_position(&self) -> usize {
        self.navigator.cursor()
    }

    /// The display name at `position`, without any directory part.
    pub fn entry_name(&self, position: usize) -> String {
        self.navigator
            .entries()
            .get(position)
            .map(|entry| entry.file_name().to_string())
            .unwrap_or_default()
    }

    /// Scroll the filmstrip when the wheel is used over it.
    ///
    /// GPUI locks a horizontal list to its own axis, so a plain vertical wheel
    /// would otherwise do nothing there; this translates each notch into one
    /// cell of horizontal movement.
    pub fn scroll_strip_by_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y * 40.,
        };
        if delta == 0. {
            return;
        }
        let count = self.navigator.len();
        if count == 0 {
            return;
        }

        // Wheel up moves towards the start of the strip.
        let notches = (delta / 40.).round();
        let steps = if notches == 0. {
            if delta > 0. { -1 } else { 1 }
        } else {
            -(notches as isize)
        };

        let next = (self.strip_first as isize + steps).clamp(0, count as isize - 1) as usize;
        if next != self.strip_first {
            self.strip_first = next;
            self.strip_scroll
                .scroll_to_item(next, ScrollStrategy::Top);
            cx.notify();
        }
    }

    /// The fit axis when the zoom is following the window, if it is.
    /// Side arrows appear only while the pointer is near an edge.
    /// Height of the title and menu bars above the canvas.
    /// Fraction of the current page-turn animation, if one is running.
    pub fn page_turn_progress(&self) -> Option<(seepic_core::PageTurnFx, f32)> {
        let (effect, started) = self.page_turn.as_ref()?;
        if *effect == seepic_core::PageTurnFx::None {
            return None;
        }
        // The gap is the transition's time, so the animation ends exactly as
        // the next slide arrives; outside a slideshow it keeps its old pace.
        let duration = if self.slideshow.is_running() {
            self.slideshow.interval().as_secs_f32().max(0.06)
        } else {
            0.24
        };
        let elapsed = started.elapsed().as_secs_f32();
        Some((*effect, (elapsed / duration).clamp(0., 1.)))
    }

    /// (opacity, x offset, width scale) for the image being drawn.
    /// GPUI has no 3D transform, so a page turn is approximated by squashing
    /// the width and unfolding it again.
    pub fn page_turn_transform(&self, viewport_width: f32) -> (f32, f32, f32) {

        let Some((effect, progress)) = self.page_turn_progress() else {
            return (1., 0., 1.);
        };
        let eased = 1. - (1. - progress).powi(3);

        match effect {
            seepic_core::PageTurnFx::None => (1., 0., 1.),
            seepic_core::PageTurnFx::Fade => (eased.max(0.02), 0., 1.),
            seepic_core::PageTurnFx::Slide => (1., (1. - eased) * viewport_width * 0.14, 1.),
            seepic_core::PageTurnFx::Flip => (0.55 + 0.45 * eased, 0., 0.04 + 0.96 * eased),
        }
    }

    /// Keep GPUI's theme in step with the 主题 setting.
    ///
    /// "跟随系统" needs the window's own appearance, which only exists during a
    /// render pass, so the setting is applied here rather than where the rest of
    /// the settings are.
    fn sync_theme(&mut self, window: &Window, cx: &mut Context<Self>) {
        let wanted = match self.settings.theme_mode {
            seepic_core::settings::ThemeMode::Light => ThemeMode::Light,
            seepic_core::settings::ThemeMode::Dark => ThemeMode::Dark,
            seepic_core::settings::ThemeMode::System => match window.appearance() {
                WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
                _ => ThemeMode::Dark,
            },
        };
        if self.applied_theme != Some(wanted) {
            Theme::change(wanted, None, cx);
            self.applied_theme = Some(wanted);
        }
    }

    /// How much of the window the bars take out of the image area.
    ///
    /// Only the fixed mode takes any: the other two float, so the canvas gets
    /// the whole window and the bars are drawn over it.
    pub fn chrome_height(&self) -> f32 {
        if self.settings.chrome_mode.takes_rows() {
            TOP_BAR_HEIGHT + STATUS_BAR_HEIGHT
        } else {
            0.
        }
    }

    pub fn paging_arrows_visible(&self, _window: &Window) -> bool {
        self.settings.show_side_arrows && self.arrows_visible
    }

    /// Whether the top bar is drawn at all.
    pub fn top_bar_visible(&self) -> bool {
        match self.settings.chrome_mode {
            ChromeMode::Fixed => true,
            ChromeMode::Auto => self.pointer_at_top,
            ChromeMode::Immersive => false,
        }
    }

    /// Whether the status bar is drawn at all.
    pub fn status_bar_visible(&self) -> bool {
        match self.settings.chrome_mode {
            ChromeMode::Fixed => true,
            ChromeMode::Auto => self.pointer_at_bottom,
            ChromeMode::Immersive => false,
        }
    }

    /// Change 界面模式, keeping the pointer's reveal state sensible.
    pub fn set_chrome_mode(&mut self, mode: ChromeMode, cx: &mut Context<Self>) {
        self.settings.chrome_mode = mode;
        self.pointer_at_top = true;
        self.settings_changed(cx);
    }

    /// What the image is being shown at, for the status bar.
    pub fn zoom_label(&self, window: &Window) -> String {
        match self.zoom.mode() {
            seepic_core::ZoomMode::Auto => match self.view_mode {
                ViewMode::FitWidth => "适合宽度".to_string(),
                ViewMode::FitHeight => "适合高度".to_string(),
                _ => "适合窗口".to_string(),
            },
            seepic_core::ZoomMode::Factor(_) => {
                format!("{:.0}%", self.zoom_percent(window))
            }
        }
    }

    /// Move the grid's selection, without leaving the grid.
    pub fn move_contact_sheet_selection(
        &mut self,
        delta: isize,
        columns: usize,
        cx: &mut Context<Self>,
    ) {
        if self.contact_matches.is_empty() {
            return;
        }
        let current = self
            .contact_matches
            .iter()
            .position(|position| *position == self.navigator_selection)
            .unwrap_or(0);
        let target = (current as isize + delta)
            .clamp(0, self.contact_matches.len() as isize - 1) as usize;
        self.navigator_selection = self.contact_matches[target];
        // A keyboard move takes over from an easing wheel: stop the animation
        // and let the list place the row itself.
        self.contact_scroll_animating = false;
        self.contact_scroll_last = None;
        // Keep the selected cell's row on screen; the wheel resumes from it.
        self.contact_top_row = target / columns.max(1);
        self.contact_scroll
            .scroll_to_item(self.contact_top_row, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Rebuild the thumbnail grid's filtered list.
    ///
    /// The grid draws from this list rather than from every entry, so typing
    /// narrows both what is shown and what the arrows walk through.
    pub fn refresh_contact_matches(&mut self) {
        let needle = self.contact_filter.trim().to_lowercase();
        let mut matches = Vec::new();
        for position in 0..self.navigator.len() {
            let name = self.entry_name(position);
            if needle.is_empty() || name.to_lowercase().contains(&needle) {
                matches.push(position);
            }
        }
        self.contact_matches = matches;
        // One pass here instead of one per rendered frame: the index bar needs
        // the first entry of each letter, and a big folder made that scan the
        // most expensive thing in the grid's render pass.
        let mut first = [None; 27];
        for (index, position) in self.contact_matches.iter().enumerate() {
            let slot = crate::contact_sheet::letter_slot(&self.entry_name(*position));
            if first[slot].is_none() {
                first[slot] = Some(index);
            }
        }
        self.contact_index_first = first;
        if self
            .contact_matches
            .binary_search(&self.navigator_selection)
            .is_err()
        {
            if let Some(&first) = self.contact_matches.first() {
                self.navigator_selection = first;
            }
        }
        // A shorter list starts at its first row; keeping the old offset would
        // show a blank area below the results. Setting the offset directly is
        // what actually moves the list — `scroll_to_item(.., Top)` only nudges
        // an item that is out of view.
        self.contact_top_row = 0;
        self.contact_scroll_target = 0.;
        self.contact_scroll_animating = false;
        self.contact_scroll_last = None;
        self.contact_scroll.set_offset(point(px(0.), px(0.)));
    }

    /// The thumbnail grid's keyboard: filtering, selection and dismissal.
    ///
    /// Returns whether the event was consumed. It runs before the shortcut
    /// table so typing a name filters instead of firing single-key commands,
    /// and before the app-wide Escape chain so Esc never quits the viewer from
    /// the grid.
    fn on_contact_sheet_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let columns = crate::contact_sheet::columns(window);
        match key {
            "escape" => {
                cx.stop_propagation();
                if self.contact_filter.is_empty() {
                    self.contact_sheet = false;
                } else {
                    self.contact_filter.clear();
                    self.refresh_contact_matches();
                }
            }
            "enter" => {
                cx.stop_propagation();
                let index = self.navigator_selection;
                self.contact_sheet = false;
                self.contact_filter.clear();
                self.contact_matches.clear();
                self.select_position(index, cx);
            }
            "backspace" => {
                cx.stop_propagation();
                if self.contact_filter.pop().is_some() {
                    self.refresh_contact_matches();
                }
            }
            "left" => {
                cx.stop_propagation();
                self.move_contact_sheet_selection(-1, columns, cx);
            }
            "right" => {
                cx.stop_propagation();
                self.move_contact_sheet_selection(1, columns, cx);
            }
            "up" => {
                cx.stop_propagation();
                self.move_contact_sheet_selection(-(columns as isize), columns, cx);
            }
            "down" => {
                cx.stop_propagation();
                self.move_contact_sheet_selection(columns as isize, columns, cx);
            }
            _ => {
                // Modified chords stay available to the global shortcuts.
                if modifiers.control || modifiers.alt {
                    return false;
                }
                let typed = event
                    .keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| key.to_string());
                if typed.chars().count() != 1 {
                    return false;
                }
                cx.stop_propagation();
                self.contact_filter.push_str(&typed);
                self.refresh_contact_matches();
                // Typing starts at the best match.
                if let Some(&first) = self.contact_matches.first() {
                    self.navigator_selection = first;
                }
            }
        }
        cx.notify();
        true
    }

    /// Jump to the page a click on the progress track lands on.
    pub fn seek_to_fraction(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let total = self.navigator.len();
        if total == 0 {
            return;
        }
        let target = (fraction.clamp(0., 1.) * (total - 1) as f32).round() as usize;
        self.select(target, cx);
    }

    /// Seek from a pointer x, using the track's recorded rectangle.
    pub fn seek_at(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some((left, width)) = self.seek_track else {
            return;
        };
        self.seek_to_fraction((x - left) / width.max(1.), cx);
    }

    /// Open the command palette.
    pub fn open_palette(&mut self, cx: &mut Context<Self>) {
        self.picker = Some(CommandPicker {
            slot: None,
            filter: String::new(),
        });
        cx.notify();
    }

    /// `(used bytes, capacity bytes, entry count)` for the info panel.
    pub fn cache_usage(&self) -> (usize, usize, usize) {
        (self.cache.used(), self.cache.capacity(), self.cache.len())
    }

    /// Whether a freshly decoded image may be kept in the cache.
    ///
    /// The two backends are exclusive: with the swap store on, every frame has
    /// to be in a slot file, and an image the store would not take is not
    /// cached at all. With it off, the cache is the heap and takes everything.
    fn cacheable(&self, image: &DecodedImage) -> bool {
        if !self.settings.swap_frames {
            return true;
        }
        // The store leaves small frames on the heap by design: a burst preview
        // costs a megabyte, and a slot is reserved and charged whole. Those are
        // exactly the frames caching must keep, or every pass over the same
        // pages decodes them again.
        let config = self.settings.swap_config();
        image
            .frames
            .iter()
            .all(|frame| frame.bgra.is_mapped() || !config.maps(frame.bgra.len()))
    }

    /// What the swap store is holding, and what it may hold.
    pub fn swap_stats(&self) -> seepic_core::swap::SwapStats {
        self.swap.stats()
    }

    /// Redraw the image after a colour adjustment changed.
    /// The EXIF block as plain text, for the clipboard.
    fn exif_text(&self) -> String {
        let Some(info) = &self.info else {
            return String::new();
        };
        let mut lines = vec![
            format!("File: {}", info.name),
            format!("Size: {} x {}", info.width, info.height),
        ];
        if let Some(exif) = &info.exif {
            for (label, value) in [
                ("Camera", exif.camera.as_ref()),
                ("Lens", exif.lens.as_ref()),
                ("Exposure", exif.exposure.as_ref()),
                ("Aperture", exif.aperture.as_ref()),
                ("ISO", exif.iso.as_ref()),
                ("Focal length", exif.focal.as_ref()),
                ("Taken", exif.taken.as_ref()),
                ("Software", exif.software.as_ref()),
            ] {
                if let Some(value) = value {
                    lines.push(format!("{label}: {value}"));
                }
            }
            for (tag, value) in &exif.fields {
                lines.push(format!("{tag}: {value}"));
            }
        }
        lines.join("
")
    }

    /// Rebuild the info panel: file facts now, EXIF once the worker returns.
    /// Push edits from the settings window into the live view flags and save.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        self.apply_settings();
        // Persisting can fail (read-only profile, disk full); the app keeps
        // running with the in-memory values, which is the useful behaviour.
        let _ = self.settings.save();
        cx.notify();
    }

    /// Copy configuration into the fields the viewer reads every frame.
    pub fn apply_settings(&mut self) {
        self.zoom.stretch_small = self.settings.stretch_small;
        // Turning the swap store on, off, or to another size takes effect for
        // the next frame decoded; frames already in slot files keep working.
        self.swap.reconfigure(self.settings.swap_config());
        self.always_on_top = self.settings.always_on_top;
        self.view_mode = self.settings.view_mode;
        self.transparency = self.settings.transparency;
        // One budget figure covers decoded pixels and built textures alike —
        // unless the swap store is on, in which case the decoded cache is
        // disk-sized and the budget pays for the textures only.
        self.cache.set_capacity(self.settings.decode_cache_capacity());
        let evicted = self.render_cache.set_budget(self.settings.render_budget());
        self.release_textures(evicted);
        // The byte budget still applies; this bounds the count as well, so small
        // pages cannot crowd out the ones around the cursor.
        self.cache
            .set_max_entries(self.settings.cache_pages() as usize);
    }

    /// Change the browse mode, probing page dimensions when the new mode needs
    /// to know how big every page is.
    pub fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.settings.view_mode = mode;
        self.view_mode = mode;
        self.anim_frame = None;
        let _ = self.settings.save();

        if mode.is_two_page() || mode.is_webtoon() {
            self.ensure_page_sizes(cx);
        } else if let Some(axis) = mode.fit_axis() {
            self.zoom.set_auto(axis);
        } else if mode == ViewMode::Actual {
            self.zoom.zoom_actual_size();
        }
        // Entering webtoon keeps the reader where they were, which needs the
        // list's own layout, so the scroll waits for the next render.
        if mode.is_webtoon() {
            self.webtoon_pending_scroll = Some(self.navigator.cursor());
        }
        self.push_toast(format!("浏览方式：{}", mode.label()), false, cx);
    }

    /// Read every page's header so two-page pairing and webtoon heights can be
    /// worked out.
    ///
    /// Header reads are microseconds where decoding is milliseconds, which is
    /// why doing this for a whole comic is affordable. The run is
    /// cancellable by bumping the generation, so switching modes mid-probe does
    /// not leave a stale result behind.
    pub fn ensure_page_sizes(&mut self, cx: &mut Context<Self>) {
        if self.window_minimized {
            return;
        }
        let count = self.navigator.len();
        if count == 0 {
            self.page_sizes.clear();
            self.spread_starts.clear();
            return;
        }
        if self.page_sizes.len() == count && self.page_sizes.iter().all(Option::is_some) {
            return;
        }

        self.page_sizes = vec![None; count];
        self.spread_starts = seepic_core::spread_starts(&vec![IvSize::default(); count], false);
        self.probing = true;
        self.probe_generation += 1;
        let generation = self.probe_generation;

        let Some(container) = self.container.clone() else {
            return;
        };
        let sources: Vec<Option<usize>> = (0..count)
            .map(|position| self.navigator.source_index(position))
            .collect();

        // A second visit to the same container costs no reads at all: the
        // headers were recorded the first time round.
        let keys: Vec<Option<String>> = (0..count)
            .map(|position| self.page_meta_key(position))
            .collect();
        let cached: Vec<Option<seepic_core::metacache::MetaEntry>> = keys
            .iter()
            .map(|key| key.as_deref().and_then(|key| self.meta_cache.lookup(key)))
            .collect();
        let from_cache = cached.iter().filter(|entry| entry.is_some()).count();
        let cached_hits = from_cache > 0;
        for (position, entry) in cached.iter().enumerate() {
            if let (Some(entry), Some(slot)) = (entry, self.page_sizes.get_mut(position)) {
                *slot = Some(IvSize::new(entry.width as f32, entry.height as f32));
            }
        }
        if cached_hits {
            self.recompute_spreads();
            cx.notify();
        }

        cx.spawn(async move |this, cx| {
            // Batched so a 2000-page comic costs a couple of hundred task
            // round-trips rather than two thousand.
            const BATCH: usize = 32;
            let mut done = 0;
            while done < count {
                let end = (done + BATCH).min(count);
                let slice = sources[done..end].to_vec();
                // Only the pages the cache did not answer are read.
                let wanted: Vec<usize> = (done..end)
                    .filter(|position| cached[*position].is_none())
                    .map(|position| position - done)
                    .collect();
                let batch: Vec<Option<usize>> = wanted
                    .iter()
                    .map(|offset| slice[*offset])
                    .collect();
                let probed = cx
                    .background_executor()
                    .spawn({
                        let container = container.clone();
                        async move {
                            batch
                                .into_iter()
                                .map(|source| {
                                    let source = source?;
                                    let mut container = container.lock().ok()?;
                                    let prefix = container
                                        .read_prefix(source, PROBE_PREFIX_BYTES)
                                        .ok()?;
                                    if let Some(details) = decode::probe_details(&prefix.bytes) {
                                        return Some(details);
                                    }
                                    // A header that did not fit in the prefix
                                    // (or a format the prefix reader cannot
                                    // shorten) falls back to the whole member.
                                    if prefix.complete {
                                        return None;
                                    }
                                    let bytes = container.read(source).ok()?;
                                    decode::probe_details(&bytes)
                                })
                                .collect::<Vec<Option<(IvSize, Option<u8>)>>>()
                        }
                    })
                    .await;

                let stale = this
                    .update(cx, |this, cx| {
                        if this.probe_generation != generation {
                            return true;
                        }
                        for (offset, probed) in wanted.iter().zip(probed) {
                            let position = done + offset;
                            let Some((size, orientation)) = probed else {
                                continue;
                            };
                            if let Some(slot) = this.page_sizes.get_mut(position) {
                                *slot = Some(size);
                            }
                            // Recorded so the next open of this container skips
                            // the read entirely.
                            if let Some(key) = &keys[position] {
                                this.meta_cache.store(
                                    key,
                                    seepic_core::metacache::MetaEntry {
                                        width: size.w.max(0.) as u32,
                                        height: size.h.max(0.) as u32,
                                        orientation,
                                    },
                                );
                            }
                        }
                        this.recompute_spreads();
                        cx.notify();
                        false
                    })
                    .unwrap_or(true);
                if stale {
                    return;
                }
                done = end;
            }

            this.update(cx, |this, cx| {
                this.flush_meta_cache(false);
                cx.notify();
            })
            .ok();

            this.update(cx, |this, cx| {
                if this.probe_generation == generation {
                    this.probing = false;
                    this.recompute_spreads();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Recompute the two-page pairing from whatever dimensions are known.
    ///
    /// Unknown pages are treated as portrait, so the layout is usable while the
    /// probe is still running and settles as results arrive.
    pub fn recompute_spreads(&mut self) {
        let sizes: Vec<IvSize> = self
            .page_sizes
            .iter()
            .map(|size| size.unwrap_or(IvSize::new(1000., 1500.)))
            .collect();
        self.spread_starts = seepic_core::spread_starts(&sizes, self.settings.separate_cover);
    }

    /// The spread containing the cursor, for two-page mode.
    pub fn current_spread(&self) -> seepic_core::Spread {
        let sizes: Vec<IvSize> = self
            .page_sizes
            .iter()
            .map(|size| size.unwrap_or(IvSize::new(1000., 1500.)))
            .collect();
        let cursor = self.navigator.cursor();
        seepic_core::spread(
            &sizes,
            &self.spread_starts,
            cursor,
            self.view_mode.direction(),
            self.settings.two_page_gap(),
        )
    }

    /// The drawn size of every page in webtoon mode, at the current viewport.
    pub fn webtoon_item_sizes(&self, viewport_width: f32) -> Vec<IvSize> {
        let sizes: Vec<IvSize> = self
            .page_sizes
            .iter()
            .map(|size| size.unwrap_or(IvSize::new(1000., 1500.)))
            .collect();
        let width = seepic_core::webtoon_width(viewport_width, self.settings.webtoon_width, None);
        seepic_core::webtoon_sizes(&sizes, width, self.settings.webtoon_gap())
    }

    /// Step the two-page cursor by whole spreads, so paging never lands in the
    /// middle of a pair.
    pub fn step_spread(&mut self, forward: bool, cx: &mut Context<Self>) {
        let cursor = self.navigator.cursor();
        let Some(&start) = self.spread_starts.get(cursor) else {
            self.step(if forward { 1 } else { -1 }, cx);
            return;
        };
        // How many pages the spread holds.
        let len = if self
            .spread_starts
            .get(start + 1)
            .is_some_and(|next| *next == start)
        {
            2
        } else {
            1
        };
        let target: Option<usize> = if forward {
            Some(start + len)
        } else {
            start.checked_sub(1).map(|previous| {
                // Land on the first page of the previous spread.
                self.spread_starts
                    .get(previous)
                    .copied()
                    .unwrap_or(previous)
            })
        };
        match target {
            Some(target) if target < self.navigator.len() => self.select(target, cx),
            _ => self.step(if forward { 1 } else { -1 }, cx),
        }
    }

    /// Decode the neighbouring pages in the background so paging is instant.
    ///
    /// Forward first, because that is the direction a viewer is mostly used in,
    /// then back: a page turn is usually followed by another, and a viewer who
    /// turns back should not pay for a decode they already triggered once.
    ///
    /// The decodes run at a lower priority than the image on screen. GPUI's
    /// background executor is the Windows thread pool, so these genuinely run
    /// alongside the visible decode rather than queueing behind it, and the low
    /// priority keeps them from competing with it for a core.
    fn prefetch_around(&mut self, cx: &mut Context<Self>) {
        if !self.settings.cache_next || self.window_minimized {
            return;
        }
        // A decode of a full-size photo peaks at tens of megabytes, so how many
        // run at once is what fast paging actually costs in memory: one
        // prefetch, and normally none while the page on screen is being
        // decoded.
        //
        if !self.decode_slot_free() {
            return;
        }
        let cursor = self.navigator.cursor();
        // Nearest first, and both directions, so a fast pager still finds the
        // next page decoded when the window is only one deep.
        let window = self.settings.prefetch_window.min(8) as isize;
        for distance in 1..=window {
            for offset in [distance, -distance] {
                let Some(position) = cursor.checked_add_signed(offset) else {
                    continue;
                };
                if position >= self.navigator.len() {
                    continue;
                }
                if !self.decode_slot_free() {
                    break;
                }
                self.prefetch(position, cx);
            }
        }
    }

    /// Decode one page into the cache, unless it is already there or on its way.
    fn prefetch(&mut self, position: usize, cx: &mut Context<Self>) {
        let Some(container) = self.container.clone() else {
            return;
        };
        let Some(source) = self.navigator.source_index(position) else {
            return;
        };
        let Some(path) = container.lock().ok().map(|c| c.path().to_path_buf()) else {
            return;
        };
        let key = CacheKey::new(&path, source);
        // Prefetching at the same detail the view will ask for means the next
        // page is a cache hit rather than a re-decode. During a burst that is
        // the reduced target, which is also what the next turn will ask for.
        let target = self.decode_target_now();
        if self.cache.get_for_detail(&key, target.unwrap_or(u32::MAX)).is_some() {
            return;
        }
        // Every page turn re-runs this for the whole window, so without this
        // the same neighbour would be decoded again for each of the four turns
        // that keep it in range.
        if !self.prefetching.insert(key.clone()) {
            return;
        }

        let swap = self.swap.clone();
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn_with_priority(
                    Priority::Low,
                    async move {
                        let bytes = container.lock().ok()?.read(source).ok()?;
                        // Streaming handles these when they reach the screen;
                        // pre-decoding one whole is far too expensive for a
                        // page the reader may never turn to.
                        if decode::is_animated_webp(&bytes) {
                            return None;
                        }
                        let byte_size = bytes.len() as u64;
                        decode::decode_for_display(&bytes, true, target)
                            .ok()
                            .map(|mut image| {
                                image.move_to_swap(&swap);
                                (Arc::new(image), byte_size)
                            })
                    },
                )
                .await;

            this.update(cx, |this, cx| {
                this.prefetching.remove(&key);
                // Ask for a frame: the idle preparation is driven from `render`,
                // and once the prefetches have stopped there is nothing else to
                // draw, so without this the preparation chain never starts.
                cx.notify();
                let Some((image, byte_size)) = decoded else {
                    return;
                };
                // The user may have paged away while this ran. Keeping the
                // result would evict something the new window needs, so a
                // frame from out of reach is simply dropped.
                let cursor = this.navigator.cursor();
                // A burst skips pages, so the one that was two ahead when the
                // decode started is one behind by the time it lands — keep it,
                // it is what the next presses run into. Only a decode from far
                // out of reach is dropped.
                let window = if this.paging_fast() {
                    6
                } else {
                    this.settings.prefetch_window.max(1) as usize
                };
                if position.abs_diff(cursor) > window {
                    crate::perf!("prefetch drop position={position} cursor={cursor}");
                    return;
                }
                crate::perf!(
                    "prefetch keep position={position} cursor={cursor} detail={:?}",
                    image.detail()
                );
                if !this.cacheable(&image) {
                    return;
                }
                let detail = image.detail();
                this.cache.insert(
                    key,
                    CachedImage {
                        image,
                        byte_size,
                        detail,
                    },
                );
            })
            .ok();
        })
        .detach();
    }

    /// Put the visible image on the clipboard as a picture.
    fn copy_image_to_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some((width, height, bgra)) = self.current_bgra() else {
            self.push_toast("没有可复制的图像", true, cx);
            return;
        };

        match image_clipboard::copy_bgra(width, height, &bgra) {
            Ok(()) => self.push_toast("已复制图片到剪贴板", false, cx),
            Err(error) => self.push_toast(format!("复制失败: {error}"), true, cx),
        }
    }

    /// The displayed image as raw BGRA, ready for a DIB.
    fn current_bgra(&self) -> Option<(u32, u32, Vec<u8>)> {
        let image = self.current.as_ref()?;
        let bytes = image.render.as_bytes(0)?;
        let (width, height) = (image.size.w as u32, image.size.h as u32);
        if width == 0 || height == 0 || bytes.len() != (width as usize) * (height as usize) * 4 {
            return None;
        }
        Some((width, height, bytes.to_vec()))
    }

    /// Copy or move the current image into the configured photo folder.
    fn store_in_photo_folder(&mut self, move_it: bool, cx: &mut Context<Self>) {
        let folder = self.settings.photo_folder.clone();
        if folder.trim().is_empty() {
            self.push_toast("请先在「设置 → 其他」指定图片文件夹", true, cx);
            return;
        }
        let target_dir = PathBuf::from(folder);
        if !target_dir.is_dir() {
            self.push_toast(
                format!("目标文件夹不存在: {}", target_dir.display()),
                true,
                cx,
            );
            return;
        }

        let position = self.navigator.cursor();
        let Some(source_index) = self.navigator.source_index(position) else {
            self.push_toast("没有可处理的图像", true, cx);
            return;
        };
        let name = self.entry_name(position);
        let source = self
            .container
            .as_ref()
            .and_then(|container| container.lock().ok().and_then(|c| c.source(source_index)));
        let Some(source) = source else {
            self.push_toast("没有可处理的图像", true, cx);
            return;
        };

        let destination = unique_path(&target_dir, &name);
        let from_archive = matches!(source, seepic_core::Source::ArchiveMember { .. });

        let result = match &source {
            seepic_core::Source::File(path) if move_it => std::fs::rename(path, &destination)
                .or_else(|_| {
                    // Across volumes rename fails; copy then delete instead.
                    std::fs::copy(path, &destination).and_then(|_| std::fs::remove_file(path))
                })
                .map(|_| ()),
            seepic_core::Source::File(path) => std::fs::copy(path, &destination).map(|_| ()),
            seepic_core::Source::ArchiveMember { .. } => {
                // A page inside an archive has to be extracted; it cannot be
                // moved out of the archive itself.
                let bytes = self.container.as_ref().and_then(|container| {
                    container.lock().ok().and_then(|mut c| c.read(source_index).ok())
                });
                match bytes {
                    Some(bytes) => std::fs::write(&destination, bytes),
                    None => Err(std::io::Error::other("无法读取压缩包内图像")),
                }
            }
        };

        match result {
            Ok(()) => {
                let verb = if from_archive {
                    "已提取到"
                } else if move_it {
                    "已移动到"
                } else {
                    "已复制到"
                };
                self.push_toast(format!("{verb} {}", target_dir.display()), false, cx);
                if move_it && !from_archive {
                    let cursor = self.navigator.cursor();
                    self.rescan_after_change(cursor, cx);
                }
            }
            Err(error) => self.push_toast(format!("操作失败: {error}"), true, cx),
        }
    }

    /// The dialog's focus handle, so the prompt can take the keyboard.
    pub fn dialog_focus(&self) -> &FocusHandle {
        &self.dialog_focus
    }

    /// Typing in a dialog: rename edits its field, Enter confirms.
    pub fn on_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => return self.confirm_dialog(cx),
            "escape" => return self.cancel_dialog(cx),
            "backspace" => {
                if let Some(dialog) = self.dialog.as_mut() {
                    if dialog.is_rename() {
                        dialog.input.pop();
                    }
                }
            }
            _ => {
                if event.keystroke.modifiers.control || event.keystroke.modifiers.alt {
                    return;
                }
                let typed = event
                    .keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| key.to_string());
                if typed.chars().count() != 1 {
                    return;
                }
                if let Some(dialog) = self.dialog.as_mut() {
                    if dialog.is_rename() {
                        dialog.input.push_str(&typed);
                        dialog.error = None;
                    }
                }
            }
        }
        cx.notify();
    }

    /// Run the dialog's action.
    pub fn confirm_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let cursor = self.navigator.cursor();
        let kind = dialog.kind.clone();

        match kind {
            confirm_dialog::DialogKind::DeleteFile { path }
            | confirm_dialog::DialogKind::DeleteArchive { path } => {
                self.delete_file_now(&path, cursor, cx);
            }
            // Nothing to do: the About box is dismissed by closing it.
            confirm_dialog::DialogKind::About => {}
            confirm_dialog::DialogKind::Rename { path } => {
                match seepic_core::fileops::rename_file(&path, &dialog.input) {
                    Ok(new_path) => {
                        let name = new_path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.push_toast(format!("已重命名为 {name}"), false, cx);
                        self.rescan_after_change(cursor, cx);
                    }
                    Err(error) => {
                        // Keep the dialog open so the user can correct the name.
                        self.dialog = Some(confirm_dialog::DialogState {
                            error: Some(error.to_string()),
                            ..dialog
                        });
                        cx.notify();
                    }
                }
            }
        }
    }

    pub fn cancel_dialog(&mut self, cx: &mut Context<Self>) {
        self.dialog = None;
        cx.notify();
    }

    /// Move the file to the recycle bin, then refresh the list.
    fn delete_file_now(&mut self, path: &Path, cursor: usize, cx: &mut Context<Self>) {
        match seepic_core::fileops::delete_to_recycle_bin(path) {
            Ok(()) => {
                self.push_toast("已移到回收站", false, cx);
                self.rescan_after_change(cursor, cx);
            }
            Err(error) => self.push_toast(format!("删除失败: {error}"), true, cx),
        }
    }

    /// Reload the container after a file changed, keeping the cursor near it.
    fn rescan_after_change(&mut self, cursor: usize, cx: &mut Context<Self>) {
        let Some(path) = self
            .container
            .as_ref()
            .and_then(|c| c.lock().ok().map(|c| c.path().to_path_buf()))
        else {
            return;
        };
        self.open_path(&path, cx);
        if !self.navigator.is_empty() {
            let index = cursor.min(self.navigator.len() - 1);
            if self.navigator.select(index).moved() {
                self.load_current(cx);
            }
        }
    }

    /// Open the first of a set of dropped paths.
    pub fn accept_dropped_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if let Some(path) = paths.into_iter().next() {
            self.open_path(&path, cx);
        }
    }

    pub fn password_focus(&self) -> &FocusHandle {
        &self.password_focus
    }

    /// Typing in the password field: characters append, Enter submits.
    pub fn on_password_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The prompt owns the keyboard while it is open.
        cx.stop_propagation();

        let key = event.keystroke.key.as_str();
        match key {
            "enter" => return self.submit_password(cx),
            "escape" => return self.cancel_password(cx),
            "backspace" => {
                if let Some(prompt) = self.password_prompt.as_mut() {
                    prompt.input.pop();
                }
            }
            _ => {
                if event.keystroke.modifiers.control || event.keystroke.modifiers.alt {
                    return;
                }
                let typed = event
                    .keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| key.to_string());
                if typed.chars().count() != 1 {
                    return;
                }
                if let Some(prompt) = self.password_prompt.as_mut() {
                    prompt.input.push_str(&typed);
                    prompt.wrong = false;
                }
            }
        }
        cx.notify();
    }

    /// Hand the password to the container and retry the read.
    pub fn submit_password(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.password_prompt.take() else {
            return;
        };
        let Some(container) = self.container.clone() else {
            return;
        };

        if let Ok(mut container) = container.lock() {
            container.set_password(&prompt.input);
        }
        self.password_tried = true;
        // Anything cached was decoded without (or with a different) password.
        self.cache.clear();
        self.load_current(cx);
    }

    /// Close the prompt without trying.
    pub fn cancel_password(&mut self, cx: &mut Context<Self>) {
        self.password_prompt = None;
        self.password_tried = false;
        cx.notify();
    }

    /// Open the information window, or close it when it is already open.
    pub fn toggle_info_window(&mut self, cx: &mut Context<Self>) {
        if self.info_window_open(cx) {
            self.close_info_window(cx);
        } else {
            self.open_info_window(cx);
        }
    }

    /// Open the file/EXIF information in its own window.
    pub fn open_info_window(&mut self, cx: &mut Context<Self>) {
        if self.info_window_open(cx) {
            return;
        }
        let Some(info) = self.info.clone() else {
            return;
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(560.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(380.), px(320.))),
            // Same custom title bar as the main window, so the two windows do
            // not look like they come from different applications.
            ..TitleBar::window_options()
        };
        match gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| InfoWindow::new(info, window, cx))
        }) {
            Ok((handle, view)) => {
                let window_id = handle.window_id();
                // The native title bar's X closes the window without calling
                // back into this view, so observe it and clear the button state.
                let weak = cx.entity().downgrade();
                self.info_window_subscription = Some(cx.on_window_closed(move |cx, closed| {
                    if closed == window_id {
                        weak.update(cx, |this, cx| {
                            this.info_window = None;
                            this.show_info = false;
                            cx.notify();
                        })
                        .ok();
                    }
                }));
                self.info_window = Some((handle, view));
                self.show_info = true;
                self.panel_stack.clear();
                cx.notify();
            }
            Err(error) => {
                self.push_toast(format!("无法打开信息窗口：{error}"), true, cx);
            }
        }
    }

    /// Whether the information window is still open.
    ///
    /// Closing it with the title bar's X or the system menu leaves no callback
    /// here, so this checks the window and clears the state that keeps the
    /// button highlighted.
    pub fn info_window_open(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((handle, _)) = self.info_window.clone() else {
            return false;
        };
        if handle.update(cx, |_, _, _| ()).is_ok() {
            true
        } else {
            self.info_window = None;
            self.show_info = false;
            false
        }
    }

    pub fn close_info_window(&mut self, cx: &mut Context<Self>) {
        if let Some((handle, _)) = self.info_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        self.show_info = false;
        cx.notify();
    }

    /// Push the current page's facts to the window, if it is open.
    fn sync_info_window(&mut self, cx: &mut Context<Self>) {
        let Some((handle, view)) = self.info_window.clone() else {
            return;
        };
        if handle.update(cx, |_, _, _| ()).is_err() {
            self.info_window = None;
            self.show_info = false;
            return;
        }
        let Some(info) = self.info.clone() else {
            return;
        };
        view.update(cx, |view, cx| view.set_info(info, cx));
        // The window may be in the background while the user pages through the
        // main window; notify alone can leave it waiting for its next frame.
        let _ = handle.update(cx, |_, window, _| window.refresh());
    }

    /// Set the facts for the page now on screen, and start its EXIF read.
    ///
    /// The pixel and file sizes come from the image that was just adopted, so
    /// they are current on every page turn — a format, not a second read of the
    /// file. The EXIF block is that second read, so it waits until the pages
    /// stop turning instead of running once per flipped page.
    pub fn refresh_info(&mut self, cx: &mut Context<Self>) {
        let Some(image) = self.current.as_ref() else {
            return;
        };

        let info = ImageInfo {
            name: image.name.to_string(),
            location: self.container_name().unwrap_or_default(),
            format: self
                .container_kind()
                .map(|kind| kind.label().to_string())
                .unwrap_or_default(),
            byte_size: image.byte_size,
            // The file's own size: a display-size reduction must not change
            // what the panel says the image is.
            width: image.decoded.source_size().0,
            height: image.decoded.source_size().1,
            animated: image.decoded.animated,
            decoded_orientation: image.decoded.exif_orientation,
            exif: None,
        };
        self.info = Some(info);
        self.info_exif_pending = true;
        self.sync_info_window(cx);
        self.fetch_exif(cx);
    }

    /// Read the current page's EXIF block, unless a burst is running.
    ///
    /// Called from `refresh_info` and once per frame after it, so a page
    /// adopted during a burst gets its EXIF as soon as the reader stops.
    fn fetch_exif(&mut self, cx: &mut Context<Self>) {
        if !self.info_exif_pending
            || self.info_exif_in_flight
            || self.paging_fast()
            || self.loading
        {
            return;
        }
        let Some(container) = self.container.clone() else {
            return;
        };
        let Some(source) = self.current.as_ref().map(|image| image.index) else {
            return;
        };
        let Some(info) = self.info.as_ref() else {
            return;
        };
        // The read lands later, and the reader may have paged on by then. The
        // page it belongs to is pinned by what was on screen when it started.
        let name = info.name.clone();
        let byte_size = info.byte_size;
        self.info_exif_pending = false;
        self.info_exif_in_flight = true;

        cx.spawn(async move |this, cx| {
            let exif = cx
                .background_executor()
                .spawn(async move {
                    let bytes = container.lock().ok()?.read(source).ok()?;
                    seepic_core::metadata::exif_from_bytes(&bytes)
                })
                .await;

            this.update(cx, |this, cx| {
                this.info_exif_in_flight = false;
                if let Some(info) = &mut this.info
                    && info.name == name
                    && info.byte_size == byte_size
                {
                    info.exif = exif;
                    this.info_exif_pending = false;
                }
                // The window may have opened while the read was in flight.
                this.sync_info_window(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn refresh_adjustments(&mut self, cx: &mut Context<Self>) {
        self.refresh_transform(cx);
    }

    /// Clear both the colour effects and the flips.
    pub fn reset_adjustments(&mut self, cx: &mut Context<Self>) {
        self.adjustments.reset();
        self.orientation.reset();
        self.refresh_transform(cx);
    }

    pub fn select_position(&mut self, position: usize, cx: &mut Context<Self>) {
        self.select(position, cx);
    }

    /// Whether the thumbnail UI the button controls is on screen.
    ///
    /// The grid is an on-demand overlay, so its setting being `Grid` does not
    /// mean it is visible; the strip takes a row whenever it is selected.
    pub fn thumbnails_visible(&self) -> bool {
        match self.settings.thumbnail_view {
            ThumbnailView::Grid => self.contact_sheet,
            ThumbnailView::Strip => true,
            ThumbnailView::Off => false,
        }
    }

    /// Whether the grid is being scrolled right now.
    ///
    /// Decoding and texture uploads are paused for the duration: the visible
    /// cells keep whatever they already had (or their placeholder) and the
    /// work happens once per stop instead of once per frame.
    pub fn contact_scrolling(&self) -> bool {
        self.contact_sheet && self.contact_scroll_animating
    }

    /// Start decoding the thumbnail at `position`, unless it is already
    /// available or on its way.
    ///
    /// `scale` is the window's scale factor. Thumbnails are stored in physical
    /// pixels: a 112-point cell on a 125% display covers 140 pixels, and a
    /// 112-pixel texture stretched over them is visibly soft.
    pub fn ensure_thumbnail(&mut self, position: usize, scale: f32, cx: &mut Context<Self>) {
        if self.window_minimized {
            return;
        }
        let scale = if scale.is_finite() && scale > 0. {
            scale
        } else {
            1.
        };
        if (self.thumbnail_scale - scale).abs() > 0.01 {
            // Moving to a monitor with another scale invalidates every tile:
            // the grid would draw them at a different physical size. Drop them
            // and decode again at the new scale.
            self.thumbnail_scale = scale;
            self.thumbnail_generation = self.thumbnail_generation.wrapping_add(1);
            let dropped: Vec<Arc<RenderImage>> =
                self.thumbnails.drain().map(|(_, render)| render).collect();
            self.thumbnails_requested.clear();
            self.thumbnail_use.clear();
            self.release_textures(dropped);
        }
        // Every visible cell asks on every frame. Touching the use counter here
        // is what makes "still on screen" the criterion for keeping a tile.
        self.thumbnail_tick += 1;
        let tick = self.thumbnail_tick;
        if self.thumbnails.contains_key(&position) {
            self.thumbnail_use.insert(position, tick);
            return;
        }
        // The visible cells ask again next frame, so declining while the pool
        // is full keeps the running decodes following the viewport instead of
        // the scroll history.
        if self.thumbnails_in_flight >= THUMBNAIL_JOBS {
            return;
        }
        let Some(container) = self.container.clone() else {
            return;
        };
        let Some(source) = self.navigator.source_index(position) else {
            return;
        };
        if !self.thumbnails_requested.insert(position) {
            return;
        }
        self.thumbnail_use.insert(position, tick);
        self.thumbnails_in_flight += 1;
        crate::perf!(
            "thumb start position={} in_flight={} sheet={} top={} strip={} view={:?} prot={} protected={}",
            position,
            self.thumbnails_in_flight,
            self.contact_sheet,
            self.contact_top_row,
            self.strip_first,
            self.settings.thumbnail_view,
            self.thumbnail_protected.contains(&position),
            self.thumbnail_protected.len()
        );
        let generation = self.thumbnail_generation;

        let thumb_w = (THUMBNAIL_WIDTH as f32 * scale).round().max(1.) as u32;
        let thumb_h = (THUMBNAIL_HEIGHT as f32 * scale).round().max(1.) as u32;
        cx.spawn(async move |this, cx| {
            let (thumb, millis) = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    let raw = container.lock().ok().and_then(|mut c| c.read(source).ok());
                    let thumb =
                        raw.and_then(|raw| decode::thumbnail_from_bytes(&raw, thumb_w, thumb_h));
                    (thumb, started.elapsed().as_secs_f32() * 1000.)
                })
                .await;

            let size = thumb.as_ref().map(|(w, h, _)| (*w, *h));
            this.update(cx, |this, cx| {
                this.thumbnails_in_flight = this.thumbnails_in_flight.saturating_sub(1);
                // The listing changed while this decode ran, so the position
                // now means a different file and the result belongs to nothing.
                if this.thumbnail_generation == generation {
                    if let Some(render) = thumb.and_then(|(w, h, bgra)| render_image(w, h, bgra)) {
                        if this.contact_scrolling() {
                            // Upload it after the wheel settles instead of
                            // during the animation; the cell keeps its
                            // placeholder until then.
                            this.deferred_thumbnails.push((position, render));
                        } else {
                            this.thumbnails.insert(position, render);
                            this.prune_thumbnails();
                        }
                    }
                    // A failure stays in `requested` so a corrupt file is not
                    // retried on every frame; the row keeps its placeholder.
                }
                crate::perf!(
                    "thumb done position={} in_flight={} resident={} size={:?} decode={millis:.1}ms",
                    position,
                    this.thumbnails_in_flight,
                    this.thumbnails.len(),
                    size
                );
                // A slot is free: ask the next render to pick up the cells that
                // were declined while the pool was full.
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn push_toast(&mut self, text: impl Into<SharedString>, is_error: bool, cx: &mut Context<Self>) {
        self.toast = Some(Toast {
            text: text.into(),
            is_error,
            shown_at: Instant::now(),
        });
        cx.notify();
    }

    // ---- two-page spreads -------------------------------------------------

    /// What the current image looks like once the transform is applied.
    fn transform_signature(&self) -> TransformSignature {
        (
            self.orientation,
            self.adjustments,
            self.filter,
            self.transparency.flattens_alpha(),
        )
    }

    /// The partner page of a spread, once it has been built.
    ///
    /// Returns `None` for a render made under a transform the user has since
    /// changed, so the caller falls back to the placeholder rather than showing
    /// an image that no longer matches the one beside it.
    pub fn spread_render(&self, position: usize) -> Option<Arc<RenderImage>> {
        let signature = self.transform_signature();
        self.spread_renders
            .get(&position)
            .filter(|(built, _)| *built == signature)
            .map(|(_, render)| render.clone())
    }

    /// Make sure every page of the current spread is drawable.
    ///
    /// The cursor's own page is already decoded for the main view; the partner
    /// is decoded here. Without this a spread draws one page and a placeholder,
    /// which is not what two-page browsing means.
    pub fn ensure_spread_renders(&mut self, cx: &mut Context<Self>) {
        if self.window_minimized {
            return;
        }
        let signature = self.transform_signature();
        let pages = self.current_spread().pages;
        let current = self.current.as_ref().map(|image| image.index);

        for position in pages {
            let source = self.navigator.source_index(position);
            // The cursor's page is the main view's image, already drawn.
            if source.is_some() && source == current {
                continue;
            }
            if self.spread_render(position).is_some() {
                continue;
            }
            if self.spread_requested.get(&position) == Some(&signature) {
                continue;
            }
            self.request_spread_page(position, signature, cx);
        }
    }

    /// Decode and build one partner page, reusing the decode cache if it is
    /// already there.
    fn request_spread_page(
        &mut self,
        position: usize,
        signature: TransformSignature,
        cx: &mut Context<Self>,
    ) {
        let Some(container) = self.container.clone() else {
            return;
        };
        let Some(source) = self.navigator.source_index(position) else {
            return;
        };
        let Some(container_path) = container.lock().ok().map(|c| c.path().to_path_buf()) else {
            return;
        };
        let key = CacheKey::new(&container_path, source);
        self.spread_requested.insert(position, signature);

        let (orientation, adjustments, filter, flatten) = signature;
        // At the same detail as the main view: a partner page is drawn beside
        // the current one, so it never needs more than the current one has.
        let target = self.decode_target;
        let needed = target.unwrap_or(u32::MAX);
        if let Some(cached) = self.cache.get_for_detail(&key, needed) {
            if let Some((render, _)) =
                build_render(&cached.image, &orientation, &adjustments, filter, None, flatten)
            {
                self.spread_renders.insert(position, (signature, render));
                self.prune_spread_renders();
            }
            return;
        }

        let swap = self.swap.clone();
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move {
                    let bytes = container.lock().ok()?.read(source).ok()?;
                    let byte_size = bytes.len() as u64;
                    decode::decode_for_display(&bytes, true, target)
                        .ok()
                        .map(|mut image| {
                            image.move_to_swap(&swap);
                            (Arc::new(image), byte_size)
                        })
                })
                .await;

            let Some((image, byte_size)) = decoded else {
                return;
            };
            this.update(cx, |this, cx| {
                if !this.cacheable(&image) {
                    return;
                }
                let detail = image.detail();
                this.cache.insert(
                    key,
                    CachedImage {
                        image: image.clone(),
                        byte_size,
                        detail,
                    },
                );
                if let Some((render, _)) =
                    build_render(&image, &orientation, &adjustments, filter, None, flatten)
                {
                    this.spread_renders.insert(position, (signature, render));
                }
                this.prune_spread_renders();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- webtoon ----------------------------------------------------------

    /// Hand textures back to the sprite atlas.
    ///
    /// A `RenderImage` becomes an atlas tile the first time it is painted, and
    /// the atlas has no eviction of its own — dropping the `Arc` leaves the
    /// pixels resident. Without this, every page ever visited keeps its texture
    /// (and one more per mip level) for the life of the process, which on a
    /// folder of photographs is hundreds of megabytes.
    fn release_textures<I>(&mut self, images: I)
    where
        I: IntoIterator<Item = Arc<RenderImage>>,
    {
        self.pending_texture_drops.extend(images);
    }

    /// Everything the viewer is holding in pixel buffers, in megabytes.
    ///
    /// The working set on its own cannot tell the viewer's own buffers from the
    /// renderer's textures and the platform's, and those need very different
    /// fixes. Printing this next to the working set says which one to work on.
    fn pixel_megabytes(&self) -> (f32, f32) {
        let cache = self.cache.used() as f32 / 1e6;

        let mut held = 0usize;
        if let Some(image) = self.current.as_ref() {
            held += image.blur.as_ref().and_then(|blur| blur.as_bytes(0)).map_or(0, <[u8]>::len);
            if let Some(mips) = image.mips.as_ref() {
                held += mips
                    .iter()
                    .map(|level| level.render.as_bytes(0).map_or(0, <[u8]>::len))
                    .sum::<usize>();
            }
        }
        // The current image's own texture is counted by the render cache, which
        // owns every built image.
        held += self.render_cache.used();
        held += self
            .thumbnails
            .values()
            .chain(self.webtoon_renders.values())
            .chain(self.spread_renders.values().map(|(_, render)| render))
            .map(|render| render.as_bytes(0).map_or(0, <[u8]>::len))
            .sum::<usize>();

        (cache, held as f32 / 1e6)
    }

    /// Release the textures belonging to the image on screen.
    ///
    /// Its built image is not released: the render cache owns it, and a page
    /// turn back to it has to find the same texture still there.
    fn release_current_textures(&mut self) {
        let Some(image) = self.current.take() else {
            return;
        };
        let mut textures = Vec::new();
        textures.extend(image.blur);
        if let Some(mips) = image.mips {
            textures.extend(mips.into_iter().map(|level| level.render));
        }
        // A build the render cache refused is not in the cache, so nothing else
        // will ever hand it back. This matters for frame-by-frame animation: a
        // frame too large for the budget would otherwise leave one texture per
        // frame in the atlas.
        if !image.render_cached {
            textures.push(image.render);
        }
        self.release_textures(textures);
    }

    /// Release every texture in the render maps, for a container change or a
    /// re-sort.
    fn release_render_maps(&mut self) {
        // A thumbnail decode already in flight was reading the old container;
        // it must not install its result into the new listing.
        self.thumbnail_generation = self.thumbnail_generation.wrapping_add(1);
        self.thumbnail_use.clear();
        self.thumbnail_protected.clear();
        // The current image is dropped first, so the render cache is the sole
        // owner of its texture by the time the cache hands everything back.
        self.release_current_textures();
        let mut textures: Vec<Arc<RenderImage>> = Vec::new();
        textures.extend(self.thumbnails.drain().map(|(_, render)| render));
        textures.extend(
            self.deferred_thumbnails
                .drain(..)
                .map(|(_, render)| render),
        );
        textures.extend(self.webtoon_renders.drain().map(|(_, render)| render));
        textures.extend(self.spread_renders.drain().map(|(_, (_, render))| render));
        textures.extend(self.render_cache.clear());
        self.release_textures(textures);
    }

    /// How many pages either side of the cursor keep their drawable texture.
    ///
    /// A texture is the decode's bytes again in CPU memory plus the GPU copy,
    /// so an unbounded map over a folder of photographs costs hundreds of
    /// megabytes. Pages outside this window are dropped; coming back to one
    /// re-draws it from the render cache, which is where the memory that can be
    /// reused lives. The window is deliberately narrow: these textures sit
    /// outside the memory budget.
    const WEBTOON_KEEP: usize = 1;

    /// How many spreads either side of the cursor keep their partner built.
    const SPREAD_KEEP: usize = 1;

    /// How many filmstrip thumbnails stay resident.
    const THUMBNAIL_KEEP: usize = 128;

    /// Drop webtoon pages that have scrolled out of reach.
    fn prune_webtoon_renders(&mut self) {
        let cursor = self.navigator.cursor();
        let mut dropped = Vec::new();
        self.webtoon_renders.retain(|position, render| {
            let keep = position.abs_diff(cursor) <= Self::WEBTOON_KEEP;
            if !keep {
                dropped.push(render.clone());
            }
            keep
        });
        self.webtoon_requested
            .retain(|position| position.abs_diff(cursor) <= Self::WEBTOON_KEEP);
        self.release_textures(dropped);
    }

    /// Drop spread partners that are no longer near the visible spread.
    fn prune_spread_renders(&mut self) {
        let cursor = self.navigator.cursor();
        let mut dropped = Vec::new();
        self.spread_renders.retain(|position, (_, render)| {
            let keep = position.abs_diff(cursor) <= Self::SPREAD_KEEP;
            if !keep {
                dropped.push(render.clone());
            }
            keep
        });
        self.spread_requested
            .retain(|position, _| position.abs_diff(cursor) <= Self::SPREAD_KEEP);
        self.release_textures(dropped);
    }

    /// Install thumbnails that finished while the grid was scrolling.
    ///
    /// One insert and one prune for the batch, instead of a texture upload and
    /// an eviction on every frame of the wheel animation.
    fn install_deferred_thumbnails(&mut self) {
        if self.deferred_thumbnails.is_empty() {
            return;
        }
        for (position, render) in self.deferred_thumbnails.drain(..) {
            self.thumbnails.insert(position, render);
        }
        self.prune_thumbnails();
    }

    /// Drop the least recently shown thumbnails.
    ///
    /// Each one is tiny — at most the 112 x 116 box — but a container can hold
    /// tens of thousands of entries, and both the grid and the strip are
    /// scrolled through rather than clicked in. Recency is the request order,
    /// which the render pass refreshes for every visible cell, so what is on
    /// screen survives and what scrolled away goes first.
    fn prune_thumbnails(&mut self) {
        if self.thumbnails.len() <= Self::THUMBNAIL_KEEP {
            return;
        }
        // Only tiles that are off screen may go. Evicting a visible one is
        // self-defeating: the next frame asks for it again, and on a large
        // window the prefetch row has a newer tick than the row above it, so
        // the on-screen tiles were exactly the ones being dropped.
        let mut by_use: Vec<(u64, usize)> = self
            .thumbnails
            .keys()
            .filter(|position| !self.thumbnail_protected.contains(position))
            .map(|position| (self.thumbnail_use.get(position).copied().unwrap_or(0), *position))
            .collect();
        by_use.sort_unstable();

        let excess = by_use.len().min(self.thumbnails.len() - Self::THUMBNAIL_KEEP);
        let stale: HashSet<usize> = by_use
            .into_iter()
            .take(excess)
            .map(|(_, position)| position)
            .collect();
        if stale.is_empty() {
            return;
        }
        let mut dropped = Vec::new();
        self.thumbnails.retain(|position, render| {
            let keep = !stale.contains(position);
            if !keep {
                dropped.push(render.clone());
            }
            keep
        });
        self.thumbnails_requested
            .retain(|position| !stale.contains(position));
        self.thumbnail_use
            .retain(|position, _| !stale.contains(position));
        self.release_textures(dropped);
        crate::perf!(
            "thumbnails resident={} limit={} protected={} dropped={} first={:?}",
            self.thumbnails.len(),
            Self::THUMBNAIL_KEEP,
            self.thumbnail_protected.len(),
            stale.len(),
            stale.iter().next()
        );
    }

    /// The drawable image for a webtoon page, once it has been built.
    pub fn webtoon_render(&self, position: usize) -> Option<Arc<RenderImage>> {
        self.webtoon_renders.get(&position).cloned()
    }

    /// Make sure the page at `position` is decoded and drawable.
    ///
    /// Continuous scrolling needs the neighbouring pages ready before they come
    /// into view, otherwise the scroll tears. Requests are made from the render
    /// pass because that is the only place that knows which pages are visible.
    pub fn ensure_webtoon_page(&mut self, position: usize, cx: &mut Context<Self>) {
        if self.webtoon_renders.contains_key(&position) || !self.webtoon_requested.insert(position) {
            return;
        }
        let Some(container) = self.container.clone() else {
            return;
        };
        let Some(source) = self.navigator.source_index(position) else {
            return;
        };
        let Some(container_path) = container.lock().ok().map(|c| c.path().to_path_buf()) else {
            return;
        };
        let key = CacheKey::new(&container_path, source);
        let orientation = self.orientation;
        let adjustments = self.adjustments;
        let filter = self.filter;
        let flatten = self.transparency.flattens_alpha();

        // At the same detail as the main view: a scrolling page is drawn no
        // larger than a fitted one, so full resolution would only be a bigger
        // texture to upload.
        let target = self.decode_target;
        let needed = target.unwrap_or(u32::MAX);

        // A page already decoded for the main view costs nothing to draw.
        if let Some(cached) = self.cache.get_for_detail(&key, needed) {
            if let Some((render, _)) =
                build_render(&cached.image, &orientation, &adjustments, filter, None, flatten)
            {
                self.webtoon_renders.insert(position, render);
                self.prune_webtoon_renders();
            }
            return;
        }

        let swap = self.swap.clone();
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move {
                    let bytes = container.lock().ok()?.read(source).ok()?;
                    let byte_size = bytes.len() as u64;
                    decode::decode_for_display(&bytes, true, target)
                        .ok()
                        .map(|mut image| {
                            image.move_to_swap(&swap);
                            (Arc::new(image), byte_size)
                        })
                })
                .await;

            let Some((image, byte_size)) = decoded else {
                return;
            };
            this.update(cx, |this, cx| {
                if !this.cacheable(&image) {
                    return;
                }
                let detail = image.detail();
                this.cache.insert(
                    key,
                    CachedImage {
                        image: image.clone(),
                        byte_size,
                        detail,
                    },
                );
                if let Some((render, _)) =
                    build_render(&image, &orientation, &adjustments, filter, None, flatten)
                {
                    this.webtoon_renders.insert(position, render);
                }
                this.prune_webtoon_renders();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Build the transparency checkerboard for the canvas, once per size.
    pub fn ensure_checker(&mut self, size: IvSize) {
        if size.is_empty() {
            return;
        }
        if self
            .checker
            .as_ref()
            .is_some_and(|(cached, _)| *cached == size)
        {
            return;
        }
        let (width, height) = (size.w.ceil() as u32, size.h.ceil() as u32);
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return;
        }

        let cell = seepic_core::transparency::CHECKER_SIZE as u32;
        let mut bytes = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let dark = ((x / cell) + (y / cell)) % 2 == 1;
                let [r, g, b] = if dark {
                    [0x4d, 0x4d, 0x4d]
                } else {
                    [0x66, 0x66, 0x66]
                };
                bytes.extend_from_slice(&[b, g, r, 0xff]);
            }
        }
        if let Some(render) = render_image(width, height, bytes) {
            // A resized window builds a new checkerboard, and the old tile
            // would otherwise stay in the atlas.
            if let Some((_, previous)) = self.checker.replace((size, render)) {
                self.release_textures([previous]);
            }
        }
    }

    /// The checkerboard to draw behind the image, if the mode calls for one.
    pub fn checker_render(&self) -> Option<Arc<RenderImage>> {
        if self.transparency != Transparency::Grid {
            return None;
        }
        self.checker.as_ref().map(|(_, render)| render.clone())
    }

    // ---- navigation -------------------------------------------------------

    /// The boundary behaviour for the container that is open.
    ///
    /// Folders and archives are kept apart, because "the end of a comic" and
    /// "the end of a folder" are different situations.
    pub fn boundary_action(&self) -> seepic_core::BoundaryAction {
        if self.in_archive() {
            self.settings.archive_boundary_action
        } else {
            self.settings.boundary_action
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.navigator.is_empty() {
            return;
        }
        match self.navigator.step(delta) {
            seepic_core::Step::Moved => {
                self.note_step();
                self.remember_position(cx);
                self.load_current(cx);
            }
            // The end of the list: what happens now is the user's choice.
            seepic_core::Step::Blocked => self.at_boundary(delta, cx),
        }
    }

    /// Record a page turn, so a burst of turns can be recognised.
    fn note_step(&mut self) {
        self.last_step_at = Some(Instant::now());
    }

    /// Handle a step that ran off the end of the list.
    fn at_boundary(&mut self, delta: isize, cx: &mut Context<Self>) {
        let forward = delta > 0;
        match self.boundary_action() {
            seepic_core::BoundaryAction::DoNothing => {}
            seepic_core::BoundaryAction::Wrap => {
                // Wrapping is still a page turn, so a held key keeps counting
                // as a burst after it comes round the end of the folder.
                self.note_step();
                self.navigator.wrap_to_end(forward);
                self.load_current(cx);
            }
            seepic_core::BoundaryAction::NextContainer => self.go_to_sibling(delta.signum(), cx),
            seepic_core::BoundaryAction::Ask => self.ask_at_boundary(forward, cx),
        }
    }

    /// Print the image as it is displayed.
    ///
    /// The pixels come from the drawn image, so rotation, flips, filters and the
    /// channel selection are all included — "print what you see", not the file
    /// as it sits on disk.
    fn print_current(&mut self, cx: &mut Context<Self>) {
        let Some((width, height, bgra)) = self.current_bgra() else {
            self.push_toast("没有可打印的图像", true, cx);
            return;
        };
        if self.in_archive() {
            // A member has no file of its own, so the temporary copy is the
            // only way to hand it to a printer.
            self.push_toast("正在准备压缩包内的图像以便打印…", false, cx);
        }

        let dir = self.temp_dir();
        let name = self
            .navigator
            .current()
            .map(|entry| {
                Path::new(entry.file_name())
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_string())
            })
            .unwrap_or_else(|| "image".to_string());
        let path = dir.join(format!("{name}-print.png"));

        match crate::printing::print_bgra(&path, width, height, &bgra) {
            Ok(()) => self.push_toast("已发送到打印程序", false, cx),
            Err(error) => self.push_toast(format!("打印失败: {error}"), true, cx),
        }
    }

    // ---- key recording -----------------------------------------------------

    /// Start listening for a new chord for `command`.
    pub fn start_key_recording(&mut self, command: Command, cx: &mut Context<Self>) {
        self.recording_key = Some(command);
        self.push_toast(
            format!("按下新的快捷键绑定「{}」（Esc 取消）", command.label()),
            false,
            cx,
        );
    }

    /// Finish a recording with the chord the user pressed.
    pub fn bind_recorded_key(&mut self, command: Command, binding: &str, cx: &mut Context<Self>) {
        // A chord that is another command's built-in key would shadow it with
        // no way back, so it is refused rather than silently taken.
        if let Some(other) = Command::for_binding(binding) {
            if other != command {
                self.push_toast(
                    format!("「{binding}」已是「{}」的默认键", other.label()),
                    true,
                    cx,
                );
                return;
            }
        }

        let displaced = self.settings.bind_key(command, binding);
        let _ = self.settings.save();
        self.recording_key = None;
        match displaced {
            Some(previous) => self.push_toast(
                format!(
                    "「{binding}」已从「{}」改绑给「{}」",
                    previous.label(),
                    command.label()
                ),
                false,
                cx,
            ),
            None => self.push_toast(
                format!("「{}」→ {binding}", command.label()),
                false,
                cx,
            ),
        }
    }

    /// Drop a command's custom chord, restoring its default.
    pub fn clear_key_binding(&mut self, command: Command, cx: &mut Context<Self>) {
        self.settings.clear_key(command);
        let _ = self.settings.save();
        self.push_toast(format!("「{}」已恢复默认键位", command.label()), false, cx);
    }

    /// Show a native dialog with nothing borrowed from the app.
    ///
    /// `rfd`'s dialogs run their own modal message loop, exactly like the
    /// caption drag loop: entered from inside an update they keep dispatching
    /// the thread's messages, and a task that updates the entity then panics
    /// with "RefCell already borrowed" because the update on the stack still
    /// holds the app. The dialog therefore runs in a task that waits for the
    /// current update to return, and `then` runs when it closes.
    fn dialog_off_stack<T: 'static>(
        cx: &mut Context<Self>,
        show: impl FnOnce() -> T + 'static,
        then: impl FnOnce(&mut Self, &mut Context<Self>, T) + 'static,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1))
                .await;
            let picked = show();
            this.update(cx, |this, cx| then(this, cx, picked)).ok();
        })
        .detach();
    }

    /// Save exactly what the canvas is showing.
    ///
    /// The pixels come from the drawn image, so rotation, flips, filters and the
    /// channel selection are all included — this is "save as seen", not "copy
    /// the file".
    pub fn save_as_seen(&mut self, cx: &mut Context<Self>) {
        use seepic_core::convert::{OutputFormat, encode_bgra};

        let Some((width, height, bgra)) = self.current_bgra() else {
            self.push_toast("没有可保存的图像", true, cx);
            return;
        };
        let stem = self
            .navigator
            .current()
            .map(|entry| {
                Path::new(entry.file_name())
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_string())
            })
            .unwrap_or_else(|| "image".to_string());

        let dialog = rfd::FileDialog::new()
            .set_title("另存为所见图像")
            .set_file_name(format!("{stem}-as-seen.png"))
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("WebP", &["webp"])
            .add_filter("BMP", &["bmp"]);
        Self::dialog_off_stack(
            cx,
            move || dialog.save_file(),
            move |this, cx, picked| {
                let Some(path) = picked else { return };
                let format = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .and_then(seepic_core::convert::format_from_extension)
                    .unwrap_or(OutputFormat::Png);

                match encode_bgra(width, height, &bgra, format, 92) {
                    Ok(bytes) => match std::fs::write(&path, bytes) {
                        Ok(()) => {
                            this.push_toast(format!("已保存：{}", path.display()), false, cx)
                        }
                        Err(error) => this.push_toast(format!("写入失败: {error}"), true, cx),
                    },
                    Err(error) => this.push_toast(format!("编码失败: {error}"), true, cx),
                }
            },
        );
    }

    /// Open the recent list.
    pub fn open_recent(&mut self, cx: &mut Context<Self>) {
        self.push_toast("最近打开：列表即将接入", false, cx);
    }

    /// Open another window of the viewer.
    pub fn open_new_window(&mut self, cx: &mut Context<Self>) {
        let Ok(exe) = std::env::current_exe() else {
            self.push_toast("找不到程序自身", true, cx);
            return;
        };
        if let Err(error) = std::process::Command::new(exe).spawn() {
            self.push_toast(format!("无法新建窗口: {error}"), true, cx);
        }
    }

    // ---- image converter --------------------------------------------------

    /// Open the converter dialog.
    pub fn open_converter(&mut self, cx: &mut Context<Self>) {
        if !self.has_images() {
            self.push_toast(s::NO_IMAGES_IN_ARCHIVE, true, cx);
            return;
        }
        self.convert = Some(crate::converter::ConvertDialog::new());
        cx.notify();
    }

    pub fn close_converter(&mut self, cx: &mut Context<Self>) {
        // Closing mid-batch would leave it running with nowhere to report, so
        // the batch is cancelled first.
        if let Some(dialog) = &self.convert {
            dialog.cancel();
        }
        self.convert = None;
        cx.notify();
    }

    /// Where the current image's conversion would be written, for the preview.
    pub fn convert_preview(&self, options: &seepic_core::convert::ConvertOptions) -> Option<String> {
        let path = self.current_path()?;
        seepic_core::convert::output_path(&path, options)
            .ok()
            .map(|path| path.display().to_string())
    }

    /// Choose the output folder.
    pub fn pick_convert_folder(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new().set_title("选择输出文件夹");
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_folder(),
            |this, cx, picked| {
                let Some(folder) = picked else { return };
                if let Some(dialog) = this.convert.as_mut() {
                    dialog.options.output_dir = Some(folder);
                    dialog.options.same_folder = false;
                }
                cx.notify();
            },
        );
    }

    /// Pick one of the dialog's radio rows.
    pub fn set_convert_radio(
        &mut self,
        field: crate::converter::Field,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        use seepic_core::convert::{OutputFormat, Overwrite, ResizeMode};
        let Some(dialog) = self.convert.as_mut() else {
            return;
        };
        match field {
            crate::converter::Field::Resize => {
                let mode = ResizeMode::MODES[index];
                // Switching modes keeps the number the user already typed.
                let value = dialog.options.resize.value();
                let box_height = dialog.options.resize.box_height();
                dialog.options.resize = mode.with_value(value, box_height);
            }
            crate::converter::Field::Format => {
                dialog.options.format = OutputFormat::ALL[index];
            }
            crate::converter::Field::Overwrite => {
                dialog.options.overwrite = Overwrite::ALL[index];
            }
            crate::converter::Field::Rotation => {
                dialog.options.rotation = [
                    seepic_core::orientation::Rotation::None,
                    seepic_core::orientation::Rotation::Cw90,
                    seepic_core::orientation::Rotation::Cw180,
                    seepic_core::orientation::Rotation::Cw270,
                ][index];
            }
            _ => {}
        }
        cx.notify();
    }

    /// Flip one of the dialog's toggles.
    pub fn toggle_convert_field(&mut self, field: crate::converter::Field, cx: &mut Context<Self>) {
        let Some(dialog) = self.convert.as_mut() else {
            return;
        };
        match field {
            crate::converter::Field::SameFolder => {
                dialog.options.same_folder = !dialog.options.same_folder;
            }
            crate::converter::Field::DeleteSource => {
                dialog.options.delete_source = !dialog.options.delete_source;
            }
            _ => {}
        }
        cx.notify();
    }

    /// Step one of the dialog's numbers.
    pub fn nudge_convert_value(
        &mut self,
        field: crate::converter::Field,
        step: i32,
        min: i32,
        max: i32,
        cx: &mut Context<Self>,
    ) {
        use seepic_core::convert::ResizeMode;
        let Some(dialog) = self.convert.as_mut() else {
            return;
        };
        let clamp = |value: i32| value.clamp(min, max);

        match field {
            crate::converter::Field::Quality => {
                dialog.options.quality = clamp(dialog.options.quality as i32 + step) as u8;
            }
            crate::converter::Field::Value => {
                let value = clamp(dialog.options.resize.value() as i32 + step) as u32;
                let box_height = dialog.options.resize.box_height();
                dialog.options.resize = dialog.options.resize.with_value(value, box_height);
            }
            crate::converter::Field::BoxHeight => {
                if let ResizeMode::FitBox { width, height } = dialog.options.resize {
                    dialog.options.resize = ResizeMode::FitBox {
                        width,
                        height: clamp(height as i32 + step) as u32,
                    };
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Type into the prefix field.
    pub fn on_converter_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(dialog) = self.convert.as_mut() else {
            return;
        };
        let key = event.keystroke.key.clone();
        if key == "backspace" {
            dialog.options.prefix.pop();
        } else if key == "escape" {
            self.convert = None;
        } else if key.chars().count() == 1 {
            if let Some(character) = key.chars().next() {
                // Only characters that are legal in a file name: a prefix with
                // a slash or a colon would name a path that cannot exist.
                if !character.is_control() && !"/\\:*?\"<>|".contains(character) {
                    dialog.options.prefix.push(character);
                }
            }
        }
        cx.notify();
    }

    /// Convert every image in the container.
    pub fn start_conversion(&mut self, cx: &mut Context<Self>) {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let Some(dialog) = self.convert.as_mut() else {
            return;
        };
        if dialog.running() {
            return;
        }
        let Some(container) = self.container.clone() else {
            return;
        };

        let options = dialog.options.clone();
        // The output folder must exist before anything is written, or every
        // file would fail with the same error.
        if !options.same_folder {
            let Some(dir) = options.output_dir.clone() else {
                self.push_toast("请先选择输出目录", true, cx);
                return;
            };
            if let Err(error) = std::fs::create_dir_all(&dir) {
                self.push_toast(format!("无法创建输出目录: {error}"), true, cx);
                return;
            }
        }

        let total = self.navigator.len();
        if total == 0 {
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        dialog.job = Some(crate::converter::ConvertProgress {
            done: 0,
            total,
            failed: 0,
            cancelled: cancelled.clone(),
        });
        cx.notify();

        // Names come from the navigator, not from the container: an archive
        // member's backend index is not its entry index, and building the
        // output name from the wrong one silently renames the output.
        let jobs: Vec<(usize, String)> = (0..total)
            .filter_map(|position| {
                let source = self.navigator.source_index(position)?;
                let name = self.navigator.entries().get(position)?.file_name().to_string();
                Some((source, name))
            })
            .collect();

        cx.spawn(async move |this, cx| {
            for (source, name) in jobs {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let container = container.clone();
                let options = options.clone();
                let outcome = cx
                    .background_executor()
                    .spawn(async move { convert_one(&container, source, &name, &options) })
                    .await;

                this.update(cx, |this, cx| {
                    if let Some(job) = this.convert.as_mut().and_then(|dialog| dialog.job.as_mut()) {
                        job.done += 1;
                        // Skipping an existing file is the user's own setting,
                        // not a failure.
                        if let Err(error) = &outcome {
                            if !matches!(error, seepic_core::convert::ConvertError::Skipped) {
                                job.failed += 1;
                            }
                        }
                    }
                    cx.notify();
                })
                .ok();
            }

            // Reported once rather than per file.
            let (done, failed, was_cancelled) = this
                .update(cx, |this, cx| {
                    let summary = this
                        .convert
                        .as_mut()
                        .and_then(|dialog| dialog.job.take())
                        .map(|job| (job.done, job.failed, job.cancelled.load(Ordering::Relaxed)))
                        .unwrap_or((0, 0, false));
                    this.convert = None;
                    cx.notify();
                    summary
                })
                .unwrap_or((0, 0, false));

            let message = if was_cancelled {
                format!("已取消，完成 {done} 个")
            } else if failed == 0 {
                format!("转换完成：{done} 个文件")
            } else {
                format!("转换完成：成功 {} 个，失败 {failed} 个", done.saturating_sub(failed))
            };
            this.update(cx, |this, cx| {
                this.push_toast(message, failed > 0, cx);
                // The folder may now hold files that were not there before.
                this.refresh_after_conversion(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Stop the running batch after the file it is on.
    pub fn cancel_conversion(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = &self.convert {
            dialog.cancel();
        }
        cx.notify();
    }

    /// Re-read the container after a batch, keeping the current image.
    ///
    /// Only a folder can gain entries, and re-opening an archive would throw
    /// away the decode cache for nothing.
    fn refresh_after_conversion(&mut self, cx: &mut Context<Self>) {
        if self.in_archive() {
            return;
        }
        let Some(path) = self.current_path() else {
            return;
        };
        let Some(container) = self.container.clone() else {
            return;
        };
        let name = self
            .navigator
            .current()
            .map(|entry| entry.file_name().to_string());
        if let Ok(container) = container.lock() {
            if let Ok(reopened) = seepic_core::Container::open(container.path()) {
                let mut navigator = Navigator::with_order(self.sort_order, self.boundary_action());
                navigator.set_entries(reopened.entries().to_vec());
                if let Some(name) = &name {
                    if let Some(index) = navigator.entries().iter().position(|e| &e.name == name) {
                        navigator.select(index);
                    }
                }
                self.navigator = navigator;
                self.container = Some(Arc::new(Mutex::new(reopened)));
                self.page_sizes.clear();
                self.spread_starts.clear();
                self.probe_generation += 1;
                if self.view_mode.is_two_page() || self.view_mode.is_webtoon() {
                    self.ensure_page_sizes(cx);
                }
            }
        }
        let _ = path;
        cx.notify();
    }

    /// Enter or leave fullscreen.
    ///
    /// Fullscreen does not change what the chrome shows: the pin decides that,
    /// exactly as it does in a window. Locked keeps the title and menu bars in
    /// place; unlocked lets them hide until the pointer reaches the top edge.
    pub fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fullscreen = !self.fullscreen;
        window.toggle_fullscreen();
        cx.notify();
    }

    /// Whether the chrome (title bar and menu bar) is on screen right now.
    pub fn chrome_visible(&self) -> bool {
        self.top_bar_visible() || self.status_bar_visible()
    }

    /// Put up the prompt, with the paths each answer would take you to.
    fn ask_at_boundary(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.navigator.len();
        if count == 0 {
            return;
        }
        let wrap_position = if forward { 0 } else { count - 1 };
        let wrap_target = self
            .navigator
            .entries()
            .get(wrap_position)
            .map(|entry| entry.file_name().to_string())
            .unwrap_or_default();

        // The sibling in the direction the user was going, so the choice reads
        // as "the next thing", not "some other folder".
        let sibling = self
            .sibling_at(if forward { 1 } else { -1 })
            .map(|name| name);

        self.boundary_prompt = Some(crate::boundary_prompt::BoundaryPrompt {
            forward,
            container: self.container_name().unwrap_or_default(),
            wrap_target,
            sibling,
        });
        cx.notify();
    }

    /// The name of the neighbouring container in the given direction.
    fn sibling_at(&self, delta: isize) -> Option<String> {
        if self.siblings.is_empty() {
            return None;
        }
        let current = self.sibling_index? as isize;
        let count = self.siblings.len() as isize;
        let next = (current + delta).rem_euclid(count) as usize;
        let path = self.siblings.get(next)?.path();
        Some(
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        )
    }

    /// Answer the prompt.
    pub fn resolve_boundary_prompt(
        &mut self,
        action: seepic_core::BoundaryAction,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.boundary_prompt.take() else {
            return;
        };
        let forward = prompt.forward;

        if remember {
            if self.in_archive() {
                self.settings.archive_boundary_action = action;
            } else {
                self.settings.boundary_action = action;
            }
            let _ = self.settings.save();
            self.navigator.set_boundary(action);
            self.push_toast(
                format!("已记住：{}", action.label()),
                false,
                cx,
            );
        }

        match action {
            seepic_core::BoundaryAction::Wrap => {
                // Wrapping is still a page turn, so a held key keeps counting
                // as a burst after it comes round the end of the folder.
                self.note_step();
                self.navigator.wrap_to_end(forward);
                self.load_current(cx);
            }
            seepic_core::BoundaryAction::NextContainer => {
                self.go_to_sibling(if forward { 1 } else { -1 }, cx);
            }
            seepic_core::BoundaryAction::DoNothing | seepic_core::BoundaryAction::Ask => {
                cx.notify();
            }
        }
    }

    /// Close the prompt without doing anything.
    pub fn dismiss_boundary_prompt(&mut self, cx: &mut Context<Self>) {
        self.boundary_prompt = None;
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.navigator.select(index).moved() {
            self.note_step();
            self.follow_cursor_in_strip();
            self.remember_position(cx);
            self.load_current(cx);
        }
    }

    /// Record the current page, at most once every couple of seconds.
    ///
    /// Writing on every page turn would put a file write in the middle of
    /// browsing a comic; the throttle means at worst the last couple of pages
    /// are forgotten if the app is killed.
    fn remember_position(&mut self, _cx: &mut Context<Self>) {
        let Some(key) = self.position_key.clone() else {
            return;
        };
        let Some(entry) = self.navigator.current() else {
            return;
        };
        let name = entry.file_name().to_string();
        let index = self.navigator.cursor();

        const WRITE_INTERVAL: Duration = Duration::from_secs(2);
        let due = self
            .position_saved_at
            .is_none_or(|saved| saved.elapsed() >= WRITE_INTERVAL);
        if !due {
            // Still remembered in memory, just not written yet.
            self.positions.remember(&key, &name, index);
            return;
        }
        self.positions.remember(&key, &name, index);
        let _ = self.positions.flush();
        self.position_saved_at = Some(Instant::now());
    }

    /// Write the header cache when it has grown and enough time has passed.
    ///
    /// A flush serializes the whole cache, and probing a large container
    /// finishes a batch every 32 pages, so writing on every batch would re-encode
    /// the entire cache hundreds of times while a comic is measured. `force`
    /// skips the throttle, for the points where the run is ending or the
    /// container is about to change.
    fn flush_meta_cache(&mut self, force: bool) {
        if self.meta_cache.len() <= self.meta_cache_flushed_at {
            return;
        }
        const WRITE_INTERVAL: Duration = Duration::from_secs(3);
        let due = force
            || self
                .meta_cache_flushed_when
                .is_none_or(|written| written.elapsed() >= WRITE_INTERVAL);
        if !due {
            return;
        }
        let _ = self.meta_cache.flush();
        self.meta_cache_flushed_at = self.meta_cache.len();
        self.meta_cache_flushed_when = Some(Instant::now());
    }

    /// Keep the current image inside the filmstrip's visible window.
    ///
    /// The list owns its scroll position, so this both scrolls it and records
    /// the approximate first visible cell for the wheel handler.
    fn follow_cursor_in_strip(&mut self) {
        let count = self.navigator.len();
        if count == 0 {
            return;
        }

        let cursor = self.navigator.cursor();
        let width = self.canvas_bounds.map(|size| size.w).unwrap_or(1200.);
        let visible = ((width / crate::thumbnails::CELL_WIDTH).floor() as usize).max(1);

        if cursor < self.strip_first || cursor >= self.strip_first + visible {
            let target = cursor.saturating_sub(visible / 2);
            self.strip_first = target;
            self.strip_scroll
                .scroll_to_item(target, ScrollStrategy::Nearest);
        }
    }

    fn go_to_sibling(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.siblings.is_empty() {
            return;
        }
        let current = self.sibling_index.unwrap_or(0) as isize;
        let count = self.siblings.len() as isize;
        let next = (current + delta).rem_euclid(count) as usize;

        let target = self.siblings[next].path().to_path_buf();
        self.sibling_index = Some(next);
        self.open_path(&target, cx);
    }

    // ---- animation --------------------------------------------------------

    /// How many frames the current image carries.
    ///
    /// This is the file's figure, which for a streaming animation is known
    /// before every frame has been decoded — the counter and the frame buttons
    /// should not grow as the decoder catches up. Playback uses the decoded
    /// count separately, so it can wait for the tail.
    pub fn frame_count(&self) -> usize {
        self.current
            .as_ref()
            .map(|image| image.total_frames.max(image.decoded.frames.len()))
            .unwrap_or(0)
    }

    pub fn is_animated(&self) -> bool {
        self.frame_count() > 1
    }

    /// The frame the animation is held on, or `None` while it plays itself.
    pub fn held_frame(&self) -> Option<usize> {
        self.anim_frame
    }

    pub fn anim_playing(&self) -> bool {
        self.anim_playing
    }

    /// Hold a neighbouring frame, which stops the animation where it lands.
    pub fn anim_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.frame_count();
        if count == 0 {
            return;
        }
        let current = self.anim_frame.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count as isize) as usize;
        self.anim_frame = Some(next);
        self.anim_playing = false;
        // A self-driven timer may already be waiting for its frame.
        self.anim_generation += 1;
        self.refresh_transform(cx);
        self.reveal_anim_bar(cx);
    }

    pub fn anim_first(&mut self, cx: &mut Context<Self>) {
        if self.frame_count() == 0 {
            return;
        }
        self.anim_frame = Some(0);
        self.anim_playing = false;
        self.anim_generation += 1;
        self.refresh_transform(cx);
        self.reveal_anim_bar(cx);
    }

    pub fn anim_last(&mut self, cx: &mut Context<Self>) {
        let count = self.frame_count();
        if count == 0 {
            return;
        }
        self.anim_frame = Some(count - 1);
        self.anim_playing = false;
        self.anim_generation += 1;
        self.refresh_transform(cx);
        self.reveal_anim_bar(cx);
    }

    /// Play or pause. The frame on screen is held when pausing, and playback
    /// resumes from it.
    pub fn toggle_anim_play(&mut self, cx: &mut Context<Self>) {
        if !self.is_animated() {
            return;
        }
        let self_driven = self
            .current
            .as_ref()
            .is_some_and(|image| image.self_driven);
        if self.anim_playing {
            self.anim_playing = false;
            // GPUI's driver does not report where it is, so that path can only
            // hold the first frame; the self-driven one knows its position.
            if !self_driven {
                self.anim_frame = Some(0);
            }
        } else {
            self.anim_playing = true;
            if self_driven {
                self.anim_frame = Some(self.anim_frame.unwrap_or(0));
            } else {
                self.anim_frame = None;
            }
        }
        self.anim_generation += 1;
        self.refresh_transform(cx);
        self.schedule_anim_frame(cx);
        self.reveal_anim_bar(cx);
    }

    /// Advance a self-driven animation, one frame per delay.
    ///
    /// Used only for animations too large to hand to GPUI whole (see
    /// `CurrentImage::self_driven`). The task is not stored: the generation
    /// counter invalidates a timer that was already waiting when the page or
    /// the play state changed, and the checks inside the update stop it.
    fn schedule_anim_frame(&mut self, cx: &mut Context<Self>) {
        if !self.anim_playing {
            return;
        }
        let Some(image) = self.current.as_ref() else {
            return;
        };
        if !image.self_driven {
            return;
        }
        // The clock runs on the file's frame count, but only frames that have
        // been decoded can be shown. A stream that is still running may be
        // ahead of, or behind, the viewer; both are handled below.
        let count = image.total_frames.max(image.decoded.frames.len());
        if count < 2 || image.decoded.frames.is_empty() {
            return;
        }
        let index = self.anim_frame.unwrap_or(0).min(count - 1);
        let delay = image
            .decoded
            .frames
            .get(index)
            .map(|frame| frame.delay_ms)
            .unwrap_or(MIN_ANIM_FRAME_MS)
            .max(MIN_ANIM_FRAME_MS);
        let generation = self.anim_generation;
        let executor = cx.background_executor().clone();

        cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(u64::from(delay))).await;
            this.update(cx, |this, cx| {
                if this.anim_generation != generation || !this.anim_playing {
                    return;
                }
                let Some(image) = this.current.as_ref() else {
                    return;
                };
                let count = image.total_frames.max(image.decoded.frames.len());
                let available = image.decoded.frames.len();
                if count < 2 {
                    return;
                }
                let current = this.anim_frame.unwrap_or(0).min(count - 1);
                if current < available {
                    let next = (current + 1) % count;
                    if next < available {
                        this.anim_frame = Some(next);
                        // Minimised there is nobody to show the frame to: the
                        // index still advances so playback resumes where it
                        // would have been, but the rebuild — a full copy of the
                        // frame — is skipped.
                        if !this.window_minimized {
                            this.refresh_transform(cx);
                        }
                    }
                    // Otherwise the next frame is still being decoded. Hold the
                    // one on screen and look again after its own delay; that is
                    // what paces playback to a stream.
                }
                this.schedule_anim_frame(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Whether the bottom animation bar should be drawn right now.
    pub fn anim_bar_shown(&self) -> bool {
        self.settings.show_anim_info && self.is_animated() && self.anim_bar_visible
    }

    /// Show the bar and restart its idle clock.
    ///
    /// Called when an animated image opens, when the pointer moves over the
    /// canvas, and when an animation command runs — the three moments the
    /// controls are worth seeing.
    pub fn reveal_anim_bar(&mut self, cx: &mut Context<Self>) {
        if !self.settings.show_anim_info || !self.is_animated() {
            return;
        }
        self.anim_bar_revealed_at = Instant::now();
        // The bar may still be marked visible from a page that is no longer on
        // screen; the countdown is restarted either way so it cannot be left
        // with no timer.
        if self.anim_bar_visible {
            self.ensure_anim_bar_timer(cx);
            return;
        }
        self.anim_bar_visible = true;
        self.ensure_anim_bar_timer(cx);
        cx.notify();
    }

    /// The pointer entered or left the bar itself.
    ///
    /// Hovering keeps it up however long the pointer rests there; leaving starts
    /// the countdown from that moment.
    pub fn set_anim_bar_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.anim_bar_hovered = hovered;
        if hovered {
            self.reveal_anim_bar(cx);
        } else {
            self.anim_bar_revealed_at = Instant::now();
        }
    }

    /// Start the idle countdown, unless a task is already waiting on it.
    ///
    /// One task per period of visibility: activity only moves the deadline it
    /// checks, so a mouse dragging across the canvas does not spawn a task per
    /// event.
    fn ensure_anim_bar_timer(&mut self, cx: &mut Context<Self>) {
        if self.anim_bar_timer_pending {
            return;
        }
        self.anim_bar_timer_pending = true;
        let idle = Duration::from_millis(ANIM_BAR_IDLE_MS);
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                let wait = this
                    .update(cx, |this, cx| {
                        if !this.settings.show_anim_info
                            || !this.is_animated()
                            || !this.anim_bar_visible
                        {
                            this.anim_bar_timer_pending = false;
                            return None;
                        }
                        if this.anim_bar_hovered {
                            // Resting on the controls keeps them up; leaving
                            // resets the deadline, so poll for that.
                            return Some(Duration::from_millis(250));
                        }
                        let elapsed = this.anim_bar_revealed_at.elapsed();
                        if elapsed < idle {
                            return Some(idle - elapsed);
                        }
                        this.anim_bar_visible = false;
                        this.anim_bar_timer_pending = false;
                        cx.notify();
                        None
                    })
                    .ok()
                    .flatten();
                match wait {
                    Some(wait) => executor.timer(wait).await,
                    None => break,
                }
            }
        })
        .detach();
    }

    // ---- transparent background -------------------------------------------

    pub fn set_transparency(&mut self, mode: Transparency, cx: &mut Context<Self>) {
        let flatten_changed = mode.flattens_alpha() != self.transparency.flattens_alpha();
        self.settings.transparency = mode;
        self.transparency = mode;
        let _ = self.settings.save();
        if flatten_changed {
            self.refresh_transform(cx);
        }
        self.push_toast(format!("透明背景：{}", mode.label()), false, cx);
    }

    // ---- canvas background and border -------------------------------------

    /// The flat colour the canvas is filled with.
    ///
    /// The transparent-background setting wins where it names a colour, because
    /// that is what the user asked to see behind a see-through image; otherwise
    /// the background setting decides.
    pub fn backdrop_color(&self, cx: &Context<Self>) -> Hsla {
        match self.transparency {
            Transparency::White => rgb(0xffffff).into(),
            Transparency::Black => rgb(0x000000).into(),
            Transparency::Grid => rgb(crate::canvas::CHECKER_BASE).into(),
            Transparency::IgnoreAlpha | Transparency::BackgroundColor => {
                match self.settings.background_mode {
                    BackgroundMode::Custom => self
                        .background_rgb()
                        .map(|value| rgb(value).into())
                        .unwrap_or_else(|| self.canvas_fallback(cx)),
                    // Derived from the image, so it moves with the picture.
                    BackgroundMode::Auto => self
                        .current
                        .as_ref()
                        .map(|image| rgb(seepic_core::auto_background(image.average_color)).into())
                        .unwrap_or_else(|| self.canvas_fallback(cx)),
                    // The blurred copy is drawn on top of this, which only shows
                    // where the picture does not cover.
                    BackgroundMode::Blur => self.canvas_fallback(cx),
                }
            }
        }
    }

    /// The picture drawn behind the image, for the Custom and Blur modes.
    pub fn backdrop_image(&self) -> Option<Arc<RenderImage>> {
        if !matches!(
            self.transparency,
            Transparency::IgnoreAlpha | Transparency::BackgroundColor
        ) {
            return None;
        }
        match self.settings.background_mode {
            BackgroundMode::Custom => self.background_image_render(),
            BackgroundMode::Auto => None,
            BackgroundMode::Blur => self.current.as_ref().and_then(|image| image.blur.clone()),
        }
    }

    /// The plain canvas colour, used where no setting names one.
    fn canvas_fallback(&self, cx: &Context<Self>) -> Hsla {
        if cx.theme().mode.is_dark() {
            rgb(0x1a1a1a).into()
        } else {
            rgb(0xf2f2f2).into()
        }
    }

    /// The user's background colour, if it parses.
    pub fn background_rgb(&self) -> Option<u32> {
        seepic_core::parse_hex_color(&self.settings.background_color)
    }

    /// The border colour, if a border is wanted and the colour parses.
    pub fn border_rgb(&self) -> Option<u32> {
        if !self.settings.show_border {
            return None;
        }
        seepic_core::parse_hex_color(&self.settings.border_color)
    }

    /// The chosen background picture, decoded once and kept.
    fn background_image_render(&self) -> Option<Arc<RenderImage>> {
        self.background_image
            .as_ref()
            .map(|(_, render)| render.clone())
    }

    pub fn set_background_mode(&mut self, mode: BackgroundMode, cx: &mut Context<Self>) {
        self.settings.background_mode = mode;
        let _ = self.settings.save();
        self.push_toast(format!("背景：{}", mode.label()), false, cx);
    }

    pub fn set_background_color(&mut self, hex: &str, cx: &mut Context<Self>) {
        self.settings.background_color = hex.to_string();
        // A colour only shows in the modes that use one.
        if self.settings.background_mode == BackgroundMode::Auto {
            self.settings.background_mode = BackgroundMode::Custom;
        }
        self.settings.background_image = None;
        self.background_image = None;
        let _ = self.settings.save();
        self.push_toast(format!("背景色：{hex}"), false, cx);
    }

    pub fn set_border_color(&mut self, hex: &str, cx: &mut Context<Self>) {
        self.settings.border_color = hex.to_string();
        let _ = self.settings.save();
        self.push_toast(format!("边框色：{hex}"), false, cx);
    }

    /// Use a picture file as the background, decoding it off the main thread.
    pub fn set_background_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let key = path.to_string_lossy().to_string();
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move {
                    let bytes = std::fs::read(&path).ok()?;
                    decode::decode(&bytes, false).ok()
                })
                .await;
            let Some(image) = decoded else {
                this.update(cx, |this, cx| {
                    // Forgotten rather than retried: a path that does not work
                    // will not start working on the next frame.
                    this.background_image_loading = false;
                    this.settings.background_image = None;
                    let _ = this.settings.save();
                    this.push_toast("无法读取背景图片".to_string(), true, cx);
                })
                .ok();
                return;
            };
            this.update(cx, |this, cx| {
                // Scaled to a screenful: the backdrop is drawn stretched, so a
                // full-size copy would only cost memory.
                let render = image
                    .blur_source(BLUR_BACKDROP_SIDE * 8)
                    .and_then(|(width, height, bytes)| render_image(width, height, bytes));
                this.background_image_loading = false;
                if let Some(render) = render {
                    this.background_image = Some((key.clone(), render));
                    this.settings.background_image = Some(key.clone());
                    this.settings.background_mode = BackgroundMode::Custom;
                    let _ = this.settings.save();
                    this.push_toast(format!("背景图片：{key}"), false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Put the background back to the plain theme colour.
    pub fn clear_background_image(&mut self, cx: &mut Context<Self>) {
        self.background_image = None;
        self.settings.background_image = None;
        let _ = self.settings.save();
        cx.notify();
    }

    /// Load the configured background picture, once, on the first frame that
    /// needs it.
    ///
    /// Deferred rather than done at start-up so a broken path in the config
    /// cannot delay the window appearing.
    pub fn ensure_background_image(&mut self, cx: &mut Context<Self>) {
        if self.background_image.is_some() || self.background_image_loading {
            return;
        }
        let Some(path) = self.settings.background_image.clone() else {
            return;
        };
        if self.settings.background_mode != BackgroundMode::Custom {
            return;
        }
        self.background_image_loading = true;
        self.set_background_image(PathBuf::from(path), cx);
    }

    // ---- panels -----------------------------------------------------------

    /// Open a panel, or close it when it is already the open one.
    /// The screen currently showing, which is the top of the stack.
    pub fn current_panel(&self) -> Option<SidePanel> {
        self.panel_stack.last().copied()
    }

    /// A menu button was clicked: open its panel, or close everything when it
    /// is already the one showing.
    pub fn toggle_panel(&mut self, panel: SidePanel, cx: &mut Context<Self>) {
        if self.panel_stack.first() == Some(&panel) {
            self.panel_stack.clear();
        } else {
            self.panel_stack = vec![panel];
        }
        self.open_menu = None;
        cx.notify();
    }

    /// Open a screen from a button inside the current one.
    ///
    /// It shows at the same anchor, with a title bar and a way back; the screen
    /// underneath stays on the stack so returning is exact.
    /// Close the panel and any screens pushed on it.
    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.panel_stack.clear();
        cx.notify();
    }

    /// Whether `panel` is the screen a menu button owns, for its highlight.
    pub fn panel_is(&self, panel: SidePanel) -> bool {
        self.panel_stack.first() == Some(&panel)
    }


    // ---- command picker ---------------------------------------------------

    // ---- file search ------------------------------------------------------

    /// Open the file search over the current folder or archive.
    pub fn open_file_search(&mut self, cx: &mut Context<Self>) {
        if self.navigator.is_empty() {
            self.push_toast(s::NO_IMAGES_IN_ARCHIVE, true, cx);
            return;
        }
        // One modal at a time.
        self.panel_stack.clear();
        self.picker = None;
        self.context_menu = None;
        self.search = Some(FileSearch::default());
        // A new search starts at the top; the column width is the reader's and
        // is kept between openings.
        self.search_scroll = ScrollHandle::new();
        self.search_col_drag = None;
        self.refresh_file_search();
        self.open_menu = None;
        cx.notify();
    }

    pub fn close_file_search(&mut self, cx: &mut Context<Self>) {
        self.search = None;
        self.search_col_drag = None;
        cx.notify();
    }

    /// Rebuild the match list for the current filter.
    fn refresh_file_search(&mut self) {
        let count = self.navigator.len();
        let needle = self
            .search
            .as_ref()
            .map(|search| search.filter.trim().to_lowercase())
            .unwrap_or_default();
        let mut matches = Vec::new();
        for position in 0..count {
            let name = self.entry_name(position);
            if needle.is_empty() || name.to_lowercase().contains(&needle) {
                matches.push(position);
                if matches.len() >= 500 {
                    break;
                }
            }
        }
        if let Some(search) = self.search.as_mut() {
            search.matches = matches;
            search.selection = search.selection.min(search.matches.len().saturating_sub(1));
        }
    }

    /// Move the highlighted match.
    pub fn move_file_search(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let count = search.matches.len();
        if count == 0 {
            return;
        }
        let target = (search.selection as isize + delta).clamp(0, count as isize - 1);
        search.selection = target as usize;
        // Keep the highlighted row in view when the list is longer than the
        // panel.
        self.search_scroll.scroll_to_item(target as usize);
        cx.notify();
    }

    /// Jump to the highlighted match.
    pub fn accept_file_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let Some(&position) = search.matches.get(search.selection) else {
            return;
        };
        self.search = None;
        self.search_col_drag = None;
        // A search started from the thumbnail grid jumps to the image, so the
        // grid has to step aside.
        self.contact_sheet = false;
        self.contact_filter.clear();
        self.contact_matches.clear();
        self.select_position(position, cx);
        cx.notify();
    }

    pub fn on_search_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => return self.close_file_search(cx),
            "enter" => return self.accept_file_search(cx),
            "up" => return self.move_file_search(-1, cx),
            "down" => return self.move_file_search(1, cx),
            "backspace" => {
                if let Some(search) = self.search.as_mut() {
                    search.filter.pop();
                }
                self.refresh_file_search();
            }
            _ => {
                if event.keystroke.modifiers.control || event.keystroke.modifiers.alt {
                    return;
                }
                let typed = event
                    .keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| key.to_string());
                if typed.chars().count() != 1 {
                    return;
                }
                if let Some(search) = self.search.as_mut() {
                    search.filter.push_str(&typed);
                    // Typing starts the highlight at the best match.
                    search.selection = 0;
                }
                self.refresh_file_search();
            }
        }
        cx.notify();
    }

    /// Open the command list, optionally to assign to a mouse slot.
    pub fn open_command_picker(&mut self, slot: Option<MouseSlot>, cx: &mut Context<Self>) {
        self.picker = Some(CommandPicker {
            slot,
            filter: String::new(),
        });
        cx.notify();
    }

    pub fn close_command_picker(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        cx.notify();
    }

    /// Bind the picked command to the slot that opened the list.
    pub fn pick_command(&mut self, command: Command, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.clone() else {
            return;
        };
        let Some(slot) = picker.slot else {
            self.close_command_picker(cx);
            return;
        };

        if let Some(binding) = self
            .settings
            .mouse_bindings
            .iter_mut()
            .find(|binding| binding.slot == slot)
        {
            binding.command = Some(command.id().to_string());
            binding.enabled = true;
        } else {
            self.settings
                .mouse_bindings
                .push(seepic_core::settings::MouseBinding {
                    slot,
                    enabled: true,
                    command: Some(command.id().to_string()),
                });
        }
        self.picker = None;
        self.push_toast(
            format!("{} → {}", slot.label(), command.label()),
            false,
            cx,
        );
        self.settings_changed(cx);
    }

    /// Typing in the command list's filter box.
    pub fn on_picker_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => return self.close_command_picker(cx),
            "backspace" => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.filter.pop();
                }
            }
            _ => {
                if event.keystroke.modifiers.control || event.keystroke.modifiers.alt {
                    return;
                }
                let typed = event
                    .keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| key.to_string());
                if typed.chars().count() != 1 {
                    return;
                }
                if let Some(picker) = self.picker.as_mut() {
                    picker.filter.push_str(&typed);
                }
            }
        }
        cx.notify();
    }

    // ---- sorting ----------------------------------------------------------

    /// Select a sort field, re-sorting the container and keeping the current
    /// image selected.
    pub fn apply_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let mut order = self.sort_order;
        order.toggle(key);
        self.set_sort_order(order, cx);
    }

    /// Put the viewer in `order`: re-sort, save the choice and reload the page.
    ///
    /// The order lives in the settings, not just in the app, so a folder or
    /// archive opened later — in this run or the next — starts sorted the way
    /// the reader last chose instead of reverting to name order.
    fn set_sort_order(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        let anchor = self
            .navigator
            .current()
            .map(|entry| entry.name.clone());

        self.sort_order = order;
        self.settings.sort_order = order;
        let _ = self.settings.save();

        self.navigator.set_order(order);
        self.release_render_maps();
        self.thumbnails_requested.clear();
        self.webtoon_requested.clear();
        self.spread_requested.clear();
        self.strip_first = 0;
        self.page_sizes.clear();
        self.spread_starts.clear();
        self.probe_generation += 1;

        // Sorting moves the anchor, so re-find the file the user was looking at
        // rather than jumping to whatever now sits at the old position.
        if let Some(name) = anchor {
            if let Some(index) = self
                .navigator
                .entries()
                .iter()
                .position(|entry| entry.name == name)
            {
                self.navigator.select(index);
            }
        }

        if self.view_mode.is_two_page() || self.view_mode.is_webtoon() {
            self.ensure_page_sizes(cx);
        }
        self.load_current(cx);
        self.push_toast(
            format!(
                "排序：{}（{}）",
                order.key.label(),
                if order.descending { "降序" } else { "升序" }
            ),
            false,
            cx,
        );
    }

    // ---- bookmarks --------------------------------------------------------

    /// Remember the current image.
    pub fn add_bookmark(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.current_path() else {
            self.push_toast("没有可添加的文件", true, cx);
            return;
        };
        if self.bookmarks.iter().any(|bookmark| bookmark.path == path) {
            self.push_toast("该文件已在收藏夹中", false, cx);
            return;
        }
        if self.bookmarks.len() >= self.settings.max_bookmarks as usize {
            self.push_toast(
                format!("收藏数量已达上限（{}）", self.settings.max_bookmarks),
                true,
                cx,
            );
            return;
        }
        let size = self.current.as_ref().map(|image| image.byte_size).unwrap_or(0);
        self.bookmarks.push(Bookmark { path, size });
        let count = self.bookmarks.len();
        self.push_toast(format!("已添加到收藏夹（{count} 个）"), false, cx);
    }

    pub fn remove_bookmark(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.bookmarks.len() {
            self.bookmarks.remove(index);
            cx.notify();
        }
    }

    pub fn clear_bookmarks(&mut self, cx: &mut Context<Self>) {
        self.bookmarks.clear();
        self.push_toast("已清空收藏夹", false, cx);
    }

    /// Open a bookmark, whether it is a file or a folder.
    pub fn open_bookmark(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(bookmark) = self.bookmarks.get(index).cloned() else {
            return;
        };
        if !bookmark.path.exists() {
            self.push_toast(
                format!("收藏已失效：{}", bookmark.path.display()),
                true,
                cx,
            );
            return;
        }
        self.panel_stack.clear();
        self.open_path(&bookmark.path, cx);
    }

    /// The most recently added bookmark.
    pub fn open_recent_bookmark(&mut self, cx: &mut Context<Self>) {
        match self.bookmarks.last().cloned() {
            Some(_) => self.open_bookmark(self.bookmarks.len() - 1, cx),
            None => self.push_toast("收藏夹为空", true, cx),
        }
    }

    // ---- external tools ---------------------------------------------------

    /// Hand the current image to the configured editor.
    ///
    /// A page inside an archive has no path of its own, so it is extracted to
    /// the temp folder first, which is why the temp directory is configurable.
    fn open_in_editor(&mut self, cx: &mut Context<Self>) {
        // An unset editor means the system decides what opens the file, which
        // is both a useful default and one that names no particular program.
        let editor = self.settings.external_editor.trim().to_string();
        let use_system_default = editor.is_empty();

        let Some(position) = Some(self.navigator.cursor()) else {
            return;
        };
        let Some(source_index) = self.navigator.source_index(position) else {
            self.push_toast("没有可打开的图像", true, cx);
            return;
        };
        let name = self.entry_name(position);

        let target = match self
            .container
            .as_ref()
            .and_then(|container| container.lock().ok().and_then(|c| c.source(source_index)))
        {
            Some(seepic_core::Source::File(path)) => Ok(path),
            Some(seepic_core::Source::ArchiveMember { .. }) => {
                let bytes = self.container.as_ref().and_then(|container| {
                    container.lock().ok().and_then(|mut c| c.read(source_index).ok())
                });
                match bytes {
                    Some(bytes) => self
                        .temp_path(&name)
                        .ok_or_else(|| std::io::Error::other("无法创建临时文件"))
                        .and_then(|path| std::fs::write(&path, bytes).map(|_| path)),
                    None => Err(std::io::Error::other("无法读取压缩包内图像")),
                }
            }
            None => Err(std::io::Error::other("没有可打开的图像")),
        };

        match target {
            Ok(path) if use_system_default => {
                if crate::shell_open::open_default(&path) {
                    let verb = if self.in_archive() { "已解压并用" } else { "已用" };
                    self.push_toast(format!("{verb}系统默认程序打开"), false, cx);
                } else {
                    self.push_toast("没有可用来打开该文件的程序", true, cx);
                }
            }
            Ok(path) => match std::process::Command::new(&editor).arg(&path).spawn() {
                Ok(_) => {
                    let verb = if self.in_archive() { "已解压并用" } else { "已用" };
                    self.push_toast(format!("{verb}{editor} 打开"), false, cx);
                }
                Err(error) => self.push_toast(
                    format!("无法启动 {editor}: {error}"),
                    true,
                    cx,
                ),
            },
            Err(error) => self.push_toast(format!("打开失败: {error}"), true, cx),
        }
    }

    /// A path in the configured temp folder, unique to this process.
    fn temp_path(&self, name: &str) -> Option<PathBuf> {
        let dir = self.temp_dir();
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir.join(format!("iv-{}-{}", std::process::id(), name)))
    }

    /// The temp folder to extract into.
    pub fn temp_dir(&self) -> PathBuf {
        if self.settings.use_user_temp_folder && !self.settings.temp_folder.trim().is_empty() {
            return PathBuf::from(self.settings.temp_folder.trim());
        }
        std::env::temp_dir()
    }

    /// Set the current image as the desktop wallpaper.
    ///
    /// Windows takes a *file path* rather than pixels, so the displayed image is
    /// encoded to a temporary JPEG first. What gets encoded is what is on
    /// screen — rotation, effects and all — which is what "set as wallpaper"
    /// means to a user.
    fn set_as_wallpaper(&mut self, cx: &mut Context<Self>) {
        let Some((width, height, bgra)) = self.current_bgra() else {
            self.push_toast("没有可设为壁纸的图像", true, cx);
            return;
        };
        let name = self
            .current
            .as_ref()
            .map(|image| image.name.to_string())
            .unwrap_or_else(|| "wallpaper.jpg".to_string());
        let stem = Path::new(&name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "wallpaper".to_string());
        let Some(path) = self.temp_path(&format!("{stem}.jpg")) else {
            self.push_toast("无法创建临时文件", true, cx);
            return;
        };

        // `image` wants RGB, and BGRA is what the canvas holds.
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for pixel in bgra.chunks_exact(4) {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        let Some(buffer) = image::RgbImage::from_raw(width, height, rgb) else {
            self.push_toast("图像数据无效", true, cx);
            return;
        };
        if let Err(error) = image::DynamicImage::ImageRgb8(buffer).save(&path) {
            self.push_toast(format!("无法写入壁纸文件: {error}"), true, cx);
            return;
        }

        match wallpaper::set_wallpaper(&path) {
            Ok(()) => self.push_toast("已设为桌面壁纸", false, cx),
            Err(error) => self.push_toast(format!("设置壁纸失败: {error}"), true, cx),
        }
    }

    /// Ask for the photo folder and remember the answer.
    pub fn pick_photo_folder(&mut self, cx: &mut Context<Self>) {
        let start = {
            let configured = self.settings.photo_folder.clone();
            if configured.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(configured))
            }
        };
        let mut dialog = rfd::FileDialog::new().set_title("选择图片文件夹");
        if let Some(start) = start.filter(|path| path.is_dir()) {
            dialog = dialog.set_directory(start);
        }
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_folder(),
            |this, cx, picked| {
                if let Some(picked) = picked {
                    this.settings.photo_folder = picked.display().to_string();
                    this.settings_changed(cx);
                }
            },
        );
    }

    /// Open the photo folder.
    pub fn open_photo_folder(&mut self, cx: &mut Context<Self>) {
        let folder = self.settings.photo_folder.clone();
        if folder.trim().is_empty() {
            self.push_toast("请先在「设置 → 其他」指定图片文件夹", true, cx);
            return;
        }
        if !Path::new(&folder).is_dir() {
            self.push_toast(format!("文件夹不存在: {folder}"), true, cx);
            return;
        }
        if !crate::shell_open::open_folder(Path::new(&folder)) {
            self.push_toast("无法打开文件夹", true, cx);
        }
    }

    // ---- clipboard --------------------------------------------------------

    /// Copy what is on screen, rather than the original bitmap.
    fn copy_resized_to_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some((width, height, bgra)) = self.current_bgra() else {
            self.push_toast("没有可复制的图像", true, cx);
            return;
        };
        match image_clipboard::copy_bgra(width, height, &bgra) {
            Ok(()) => self.push_toast("已复制当前显示的图像", false, cx),
            Err(error) => self.push_toast(format!("复制失败: {error}"), true, cx),
        }
    }

    /// Take whatever the clipboard holds and show it.
    ///
    /// An image is written to the temp folder and opened as a file, which makes
    /// it behave exactly like any other image — paging, zoom and the info panel
    /// all work without a special "pasted image" source.
    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            self.push_toast("剪贴板为空", true, cx);
            return;
        };

        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    let extension = match image.format {
                        ImageFormat::Png => "png",
                        ImageFormat::Jpeg => "jpg",
                        ImageFormat::Webp => "webp",
                        ImageFormat::Gif => "gif",
                        ImageFormat::Svg => "svg",
                        ImageFormat::Bmp => "bmp",
                        ImageFormat::Tiff => "tiff",
                        _ => "png",
                    };
                    let Some(path) = self.temp_path(&format!("clipboard.{extension}")) else {
                        self.push_toast("无法创建临时文件", true, cx);
                        return;
                    };
                    match std::fs::write(&path, image.bytes.to_vec()) {
                        Ok(()) => {
                            self.push_toast("已从剪贴板粘贴图像", false, cx);
                            self.open_path(&path, cx);
                        }
                        Err(error) => {
                            self.push_toast(format!("写入临时文件失败: {error}"), true, cx)
                        }
                    }
                    return;
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    if let Some(path) = paths.paths().first() {
                        let path = path.clone();
                        self.push_toast("已从剪贴板粘贴文件", false, cx);
                        self.open_path(&path, cx);
                        return;
                    }
                }
                ClipboardEntry::String(text) => {
                    let trimmed = text.text.trim().trim_matches('"');
                    let candidate = PathBuf::from(trimmed);
                    if candidate.exists() {
                        self.push_toast("已从剪贴板粘贴文件", false, cx);
                        self.open_path(&candidate, cx);
                        return;
                    }
                }
            }
        }

        self.push_toast("剪贴板里没有可显示的图像", true, cx);
    }

    // ---- commands ---------------------------------------------------------

    /// Run a command. This is the single entry point shared by the keyboard,
    /// the menus and the control bar.
    pub fn execute(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::Quit => {
                cx.quit();
                return;
            }
            Command::OpenFile => self.pick_file(cx),
            Command::OpenFolder => self.pick_folder(cx),
            Command::CloseImage => {
                self.container = None;
                self.current = None;
                self.info = None;
                self.navigator = Navigator::new();
                self.slideshow.stop();
            }
            Command::RevealInFolder => self.reveal_in_folder(cx),
            Command::Rename => {
                if self.in_archive() {
                    self.push_toast("压缩包内的文件不能重命名", true, cx);
                } else if let Some(path) = self.current_path() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.dialog = Some(DialogState::rename(path, &name));
                    cx.notify();
                }
            }
            Command::DeleteFile => {
                let Some(path) = self.current_path() else {
                    self.push_toast("没有可删除的文件", true, cx);
                    return;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if self.settings.confirm_delete {
                    self.dialog = Some(if self.in_archive() {
                        DialogState::delete_archive(path, &name)
                    } else {
                        DialogState::delete_file(path, &name)
                    });
                    cx.notify();
                } else {
                    let cursor = self.navigator.cursor();
                    self.delete_file_now(&path, cursor, cx);
                }
            }
            Command::Settings => {
                self.show_settings = !self.show_settings;
                if self.show_settings {
                    self.close_panel(cx);
                }
            }
            Command::About => {
                self.dialog = Some(DialogState::about(env!("CARGO_PKG_VERSION")));
                let handle = self.dialog_focus();
                handle.focus(window, cx);
                cx.notify();
            }

            Command::Minimize => window.minimize_window(),
            // "Maximize" is the title bar's zoom button, and it toggles:
            // GPUI's own zoom only maximizes, so a second double-click would
            // otherwise do nothing.
            // In fullscreen there is nothing to maximize: the double click
            // that reaches this command means "give me my window back", which
            // is the same restore the fullscreen toggle performs.
            Command::Maximize => {
                if self.fullscreen {
                    self.toggle_fullscreen(window, cx);
                } else if window.is_maximized() {
                    crate::chrome_drag::restore(window);
                } else {
                    window.zoom_window();
                }
            }
            // The window is sized to the image, in logical pixels, which is
            // what "适应图像大小" means here.
            Command::ResizeWindowToImage => {
                if self.current.is_some() {
                    let image = self.image_size();
                    let chrome = self.chrome_height();
                    window.resize(gpui::size(
                        px(image.w.max(320.)),
                        px((image.h + chrome).max(240.)),
                    ));
                }
            }

            Command::NextImage => {
                if self.view_mode.is_two_page() {
                    self.step_spread(true, cx);
                } else {
                    self.step(1, cx);
                }
            }
            Command::PrevImage => {
                if self.view_mode.is_two_page() {
                    self.step_spread(false, cx);
                } else {
                    self.step(-1, cx);
                }
            }
            Command::FirstImage => self.select(0, cx),
            Command::LastImage => {
                let last = self.navigator.len().saturating_sub(1);
                self.select(last, cx);
            }
            Command::Next5 => self.step(5, cx),
            Command::Prev5 => self.step(-5, cx),
            Command::Next10 => self.step(10, cx),
            Command::Prev10 => self.step(-10, cx),
            Command::NextRandom | Command::PrevRandom => {
                if self.navigator.len() > 1 {
                    let len = self.navigator.len();
                    let current = self.navigator.cursor();
                    let offset = 1 + (Instant::now().elapsed().subsec_nanos() as usize) % (len - 1);
                    let target = (current + offset) % len;
                    self.select(target, cx);
                }
            }
            Command::NextContainer => self.go_to_sibling(1, cx),
            Command::PrevContainer => self.go_to_sibling(-1, cx),

            Command::SortByName => self.apply_sort(SortKey::Name, cx),
            Command::SortByDateModified => self.apply_sort(SortKey::DateModified, cx),
            Command::SortBySize => self.apply_sort(SortKey::Size, cx),
            Command::SortByType => self.apply_sort(SortKey::Type, cx),
            Command::SortByDateTaken => self.apply_sort(SortKey::DateTaken, cx),
            Command::SortByDateCreated => self.apply_sort(SortKey::DateCreated, cx),
            // The platform supplies its own comparison here rather than a locale
            // collation being used to approximate it.
            Command::SortByLogical => self.apply_sort(SortKey::Logical, cx),
            Command::ToggleSortAscending => {
                let mut order = self.sort_order;
                order.descending = !order.descending;
                self.set_sort_order(order, cx);
            }

            Command::ViewMode100 => self.set_view_mode(ViewMode::Actual, cx),
            Command::ViewModeFitWindow => self.set_view_mode(ViewMode::FitWindow, cx),
            Command::ViewModeFitWidth => self.set_view_mode(ViewMode::FitWidth, cx),
            Command::ViewModeFitHeight => self.set_view_mode(ViewMode::FitHeight, cx),
            Command::ViewModeTwoL2R => self.set_view_mode(ViewMode::TwoPageL2R, cx),
            Command::ViewModeTwoR2L => self.set_view_mode(ViewMode::TwoPageR2L, cx),
            Command::ViewModeWebtoon => self.set_view_mode(ViewMode::Webtoon, cx),
            Command::ViewModeNext => {
                let next = self.view_mode.next();
                self.set_view_mode(next, cx);
            }
            Command::ToggleSeparateCover => {
                self.settings.separate_cover = !self.settings.separate_cover;
                let on = self.settings.separate_cover;
                self.recompute_spreads();
                let _ = self.settings.save();
                self.push_toast(
                    if on {
                        "智能双页：使用单独的封面"
                    } else {
                        "智能双页：不使用单独的封面"
                    },
                    false,
                    cx,
                );
            }
            Command::ToggleTransparentBg => {
                let next = self.transparency.next();
                self.set_transparency(next, cx);
            }
            Command::TransparentGrid => self.set_transparency(Transparency::Grid, cx),
            Command::TransparentWhite => self.set_transparency(Transparency::White, cx),
            Command::TransparentBlack => self.set_transparency(Transparency::Black, cx),
            Command::TransparentIgnoreAlpha => {
                self.set_transparency(Transparency::IgnoreAlpha, cx)
            }
            Command::TransparentBackgroundColor => {
                self.set_transparency(Transparency::BackgroundColor, cx)
            }

            Command::ToggleThumbnails => {
                // The grid is an overlay rather than a row, so when it is the
                // chosen view the command opens and closes it; the strip is a
                // row, so there it turns the row on and off.
                match self.settings.thumbnail_view {
                    ThumbnailView::Grid => {
                        self.contact_sheet = !self.contact_sheet;
                        if self.contact_sheet {
                            self.navigator_selection = self.navigator.cursor();
                            self.refresh_contact_matches();
                        } else {
                            self.contact_filter.clear();
                            self.contact_matches.clear();
                        }
                        cx.notify();
                    }
                    ThumbnailView::Strip => {
                        self.settings.thumbnail_view = ThumbnailView::Off;
                        self.settings_changed(cx);
                        cx.notify();
                    }
                    ThumbnailView::Off => {
                        self.settings.thumbnail_view = ThumbnailView::Grid;
                        self.contact_sheet = true;
                        self.navigator_selection = self.navigator.cursor();
                        self.refresh_contact_matches();
                        self.settings_changed(cx);
                        cx.notify();
                    }
                }
            }
            Command::ToggleInfo => self.toggle_info_window(cx),
            Command::ToggleImagePanel => self.toggle_panel(SidePanel::Image, cx),
            Command::ToggleSlideshowPanel => self.toggle_panel(SidePanel::Slideshow, cx),
            Command::OpenCommandPalette => self.open_palette(cx),
            Command::SearchFiles => {
                // The toolbar button and the command both toggle: a second
                // press on the search button closes the panel it opened.
                if self.search.is_some() {
                    self.close_file_search(cx);
                } else {
                    self.open_file_search(cx);
                }
            }
            Command::ToggleBookmarkPanel => self.toggle_panel(SidePanel::Bookmark, cx),
            Command::ToggleSortPanel => self.toggle_panel(SidePanel::Sort, cx),

            Command::ZoomIn => self.zoom.zoom_in(self.image_size(), self.viewport(window)),
            Command::ZoomOut => self.zoom.zoom_out(self.image_size(), self.viewport(window)),
            Command::ZoomIn1 => self.zoom_by_percent(1., window),
            Command::ZoomOut1 => self.zoom_by_percent(-1., window),
            Command::ZoomActual => self.zoom.zoom_actual_size(),
            Command::ZoomReset => {
                self.reset_zoom_for_view_mode();
                self.zoom.center();
            }

            Command::ScrollUp => self.pan(0., 60., window),
            Command::ScrollDown => self.pan(0., -60., window),
            Command::ScrollLeft => self.pan(60., 0., window),
            Command::ScrollRight => self.pan(-60., 0., window),
            Command::ScrollCenter => self.zoom.center(),
            Command::ScrollTopLeft => {
                self.scroll_to_corner(0., 0., window);
            }
            Command::ScrollTopRight => {
                self.scroll_to_corner(1., 0., window);
            }
            Command::ScrollBottomLeft => {
                self.scroll_to_corner(0., 1., window);
            }
            Command::ScrollBottomRight => {
                self.scroll_to_corner(1., 1., window);
            }
            // Space scrolls first, and pages forward once the bottom of the
            // image is in view.
            Command::ScrollPageDown => {
                let viewport = self.viewport(window);
                let layout = self
                    .zoom
                    .resolve(self.image_size(), viewport, window.scale_factor());
                let slack = (layout.drawn.h - viewport.h) / 2.;
                let at_bottom = slack <= 0. || self.zoom.offset().y <= -slack + 1.;
                if at_bottom {
                    self.step(1, cx);
                } else {
                    self.pan(0., -viewport.h * 0.9, window);
                }
            }

            Command::RotateCw => {
                self.orientation.rotate_cw();
                self.refresh_transform(cx);
                return;
            }
            Command::RotateCcw => {
                self.orientation.rotate_ccw();
                self.refresh_transform(cx);
                return;
            }
            Command::RotationNone => {
                self.orientation.set_quarter_turns(0);
                self.refresh_transform(cx);
                return;
            }
            Command::RotationL90 => {
                self.orientation.set_quarter_turns(3);
                self.refresh_transform(cx);
                return;
            }
            Command::RotationR90 => {
                self.orientation.set_quarter_turns(1);
                self.refresh_transform(cx);
                return;
            }
            Command::Rotation180 => {
                self.orientation.set_quarter_turns(2);
                self.refresh_transform(cx);
                return;
            }
            Command::FlipHorizontal => {
                self.orientation.flip_horizontal();
                self.refresh_transform(cx);
                return;
            }
            Command::FlipVertical => {
                self.orientation.flip_vertical();
                self.refresh_transform(cx);
                return;
            }
            Command::ResetTransform => {
                self.orientation.reset();
                self.refresh_transform(cx);
                return;
            }

            Command::FilterNone => self.set_filter(FilterMode::None, cx),
            Command::FilterSoften => self.set_filter(FilterMode::Smooth, cx),
            Command::FilterSoftenSharpen => self.set_filter(FilterMode::SmoothSharpen, cx),
            Command::FilterNext => {
                let next = match self.filter {
                    FilterMode::None => FilterMode::Smooth,
                    FilterMode::Smooth => FilterMode::SmoothSharpen,
                    FilterMode::SmoothSharpen => FilterMode::None,
                };
                self.set_filter(next, cx);
            }
            Command::InterpolationBilinear => self.set_interpolation(Interpolation::Bilinear, cx),
            Command::InterpolationBicubic => self.set_interpolation(Interpolation::Bicubic, cx),
            Command::InterpolationLanczos => self.set_interpolation(Interpolation::Lanczos, cx),
            Command::InterpolationNext => {
                let next = match self.interpolation {
                    Interpolation::Bilinear => Interpolation::Bicubic,
                    Interpolation::Bicubic => Interpolation::Lanczos,
                    Interpolation::Lanczos => Interpolation::Bilinear,
                };
                self.set_interpolation(next, cx);
            }

            Command::ImageNegative => self.toggle_effect(Effect::Negative, cx),
            Command::ImageGamma => self.toggle_effect(Effect::Gamma, cx),
            Command::ImageBrightness => self.toggle_effect(Effect::Brightness, cx),
            Command::ImageContrast => self.toggle_effect(Effect::Contrast, cx),
            Command::ImageSaturation => self.toggle_effect(Effect::Saturation, cx),
            Command::ImageGray => self.toggle_effect(Effect::Gray, cx),
            Command::ImageSepia => self.toggle_effect(Effect::Sepia, cx),
            Command::ChannelAll => self.set_channel(seepic_core::effects::Channel::All, cx),
            Command::ChannelR => self.set_channel(seepic_core::effects::Channel::Red, cx),
            Command::ChannelG => self.set_channel(seepic_core::effects::Channel::Green, cx),
            Command::ChannelB => self.set_channel(seepic_core::effects::Channel::Blue, cx),
            Command::ChannelA => self.set_channel(seepic_core::effects::Channel::Alpha, cx),
            Command::ChannelNext => {
                let next = self.adjustments.channel.next();
                self.set_channel(next, cx);
            }
            Command::ResetImageProc => self.reset_adjustments(cx),

            Command::CopyImage => self.copy_image_to_clipboard(cx),
            Command::CopyResized => self.copy_resized_to_clipboard(cx),
            Command::Paste => self.paste_from_clipboard(cx),
            Command::CopyToPhotoFolder => self.store_in_photo_folder(false, cx),
            Command::MoveToPhotoFolder => self.store_in_photo_folder(true, cx),
            Command::OpenPhotoFolder => self.open_photo_folder(cx),

            Command::BookmarkAdd => self.add_bookmark(cx),
            Command::BookmarkLoadRecent => self.open_recent_bookmark(cx),
            Command::ToggleBookmarkWindow => self.toggle_panel(SidePanel::Bookmark, cx),

            Command::AnimNext => self.anim_step(1, cx),
            Command::AnimPrev => self.anim_step(-1, cx),
            Command::AnimHome => self.anim_first(cx),
            Command::AnimEnd => self.anim_last(cx),
            Command::AnimPlayPause => self.toggle_anim_play(cx),
            Command::ToggleAnimInfo => {
                self.settings.show_anim_info = !self.settings.show_anim_info;
                if self.settings.show_anim_info {
                    self.reveal_anim_bar(cx);
                } else {
                    self.anim_bar_visible = false;
                    self.anim_bar_hovered = false;
                }
                self.settings_changed(cx);
            }

            Command::LaunchEditor => self.open_in_editor(cx),
            Command::WallpaperDlg => self.set_as_wallpaper(cx),
            Command::PopupMeta => self.open_info_window(cx),

            // Open a file/folder picker, or report that the feature is a stub.
            Command::SelectImage => {
                self.show_navigator = !self.show_navigator;
                self.navigator_selection = self.navigator.cursor();
            }
            Command::Convert => self.open_converter(cx),
            Command::Print => self.print_current(cx),
            Command::SaveResized => self.save_as_seen(cx),
            Command::NewWindow => self.open_new_window(cx),
            Command::ToggleBorder => {
                self.settings.show_border = !self.settings.show_border;
                self.settings_changed(cx);
                self.push_toast(
                    format!(
                        "图像边框：{}",
                        if self.settings.show_border { "显示" } else { "隐藏" }
                    ),
                    false,
                    cx,
                );
            }
            Command::OpenRecent => self.open_recent(cx),

            Command::ToggleFullscreen => self.toggle_fullscreen(window, cx),
            // 界面模式: the pin and the two bound commands all land here.
            Command::ToggleChromeRows => {
                self.settings.chrome_mode = match self.settings.chrome_mode {
                    ChromeMode::Fixed => ChromeMode::Auto,
                    ChromeMode::Auto | ChromeMode::Immersive => ChromeMode::Fixed,
                };
                // Keep the bars revealed while the pointer is still up there;
                // otherwise the control would vanish from under the cursor.
                self.pointer_at_top = true;
                self.settings_changed(cx);
            }
            Command::ChromeFixed => self.set_chrome_mode(ChromeMode::Fixed, cx),
            Command::ChromeAuto => self.set_chrome_mode(ChromeMode::Auto, cx),
            Command::ChromeImmersive => self.set_chrome_mode(ChromeMode::Immersive, cx),
            Command::ToggleZoomLock => {
                self.zoom.zoom_locked = !self.zoom.zoom_locked;
                self.push_toast(
                    if self.zoom.zoom_locked {
                        "已固定缩放比例"
                    } else {
                        "已取消固定缩放比例"
                    },
                    false,
                    cx,
                );
            }
            Command::ToggleScrollLock => {
                self.zoom.scroll_locked = !self.zoom.scroll_locked;
                self.push_toast(
                    if self.zoom.scroll_locked {
                        "已固定滚动位置"
                    } else {
                        "已取消固定滚动位置"
                    },
                    false,
                    cx,
                );
            }
            Command::AlwaysOnTop => {
                self.settings.always_on_top = !self.settings.always_on_top;
                self.settings_changed(cx);
            }
            Command::ToggleStretchSmall => {
                self.settings.stretch_small = !self.settings.stretch_small;
                self.settings_changed(cx);
            }
            Command::CopyPath => match self.current_path() {
                Some(path) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        path.display().to_string(),
                    ));
                    self.push_toast("已复制文件路径", false, cx);
                }
                None => self.push_toast("没有可复制的路径", true, cx),
            },
            Command::CopyExif => {
                let text = self.exif_text();
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.push_toast("已复制 EXIF 信息", false, cx);
            }
            Command::CopyFile => match self.current_path() {
                Some(path) => {
                    let item = ClipboardItem {
                        entries: vec![ClipboardEntry::ExternalPaths(
                            gpui::ExternalPaths(vec![path].into()),
                        )],
                    };
                    cx.write_to_clipboard(item);
                    self.push_toast("已复制图像文件", false, cx);
                }
                None => self.push_toast("没有可复制的文件", true, cx),
            },

            Command::SlideshowStop => self.stop_slideshow(cx),
            Command::SlideshowToggle => {
                if self.slideshow.is_running() {
                    self.stop_slideshow(cx);
                } else {
                    self.start_slideshow(cx);
                }
            }
            Command::SlideshowCustom => self.toggle_panel(SidePanel::Slideshow, cx),

            other => {
                if let Some(seconds) = other.slideshow_seconds() {
                    self.set_slideshow_interval(seepic_core::Interval::from_preset(seconds), cx);
                }
            }
        }
        cx.notify();
    }

    /// Zoom by a relative percentage, which is what the `±1%` commands do.
    fn zoom_by_percent(&mut self, delta: f32, window: &Window) {
        if self.zoom.zoom_locked {
            return;
        }
        let current = self
            .zoom
            .effective_factor(self.image_size(), self.viewport(window));
        self.zoom.set_factor(current + delta / 100.);
    }

    pub fn set_filter(&mut self, filter: FilterMode, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.refresh_transform(cx);
    }

    pub fn set_interpolation(&mut self, interpolation: Interpolation, cx: &mut Context<Self>) {
        if self.interpolation == interpolation {
            return;
        }
        self.interpolation = interpolation;
        // The mip pyramid was built with the old kernel.
        if let Some(image) = self.current.as_mut() {
            image.mips = None;
        }
        cx.notify();
    }

    /// Turn one image effect on or off.
    ///
    /// The sliders have no separate on/off control, so the checkbox flips
    /// between neutral and a visible starting value.
    fn toggle_effect(&mut self, effect: Effect, cx: &mut Context<Self>) {
        match effect {
            Effect::Negative => self.adjustments.negative = !self.adjustments.negative,
            Effect::Gray => self.adjustments.grayscale = !self.adjustments.grayscale,
            Effect::Sepia => self.adjustments.sepia = !self.adjustments.sepia,
            Effect::Brightness => {
                self.adjustments.brightness = if self.adjustments.brightness != 0 {
                    0
                } else {
                    20
                }
            }
            Effect::Contrast => {
                self.adjustments.contrast = if self.adjustments.contrast != 0 {
                    0
                } else {
                    20
                }
            }
            Effect::Saturation => {
                self.adjustments.saturation = if self.adjustments.saturation != 0 {
                    0
                } else {
                    30
                }
            }
            Effect::Gamma => {
                self.adjustments.gamma = if self.adjustments.gamma != 100 {
                    100
                } else {
                    120
                }
            }
        }
        self.refresh_transform(cx);
    }

    fn set_channel(&mut self, channel: seepic_core::effects::Channel, cx: &mut Context<Self>) {
        self.adjustments.channel = channel;
        self.refresh_transform(cx);
    }

    /// Native "open file" dialog.
    ///
    /// The dialog is modal, so blocking the UI thread while it is up is what the
    /// user experiences anyway.
    fn pick_file(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new()
            .set_title("打开图片或压缩包")
            .add_filter(
                "图片与压缩包",
                &[
                    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "webp", "bmp", "ico", "tif",
                    "tiff", "tga", "pnm", "dds", "hdr", "exr", "qoi", "zip", "cbz", "7z", "cb7",
                    "rar", "cbr", "tar", "cbt",
                ],
            );
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_file(),
            |this, cx, picked| {
                if let Some(path) = picked {
                    this.open_path(&path, cx);
                }
            },
        );
    }

    /// Native "open folder" dialog.
    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new().set_title("打开文件夹");
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_folder(),
            |this, cx, picked| {
                if let Some(path) = picked {
                    this.open_path(&path, cx);
                }
            },
        );
    }

    /// Write the whole configuration to a file the user picks.
    pub fn export_settings(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new()
            .set_title("导出设置")
            .set_file_name("seepic-settings.json")
            .add_filter("JSON", &["json"]);
        Self::dialog_off_stack(
            cx,
            move || dialog.save_file(),
            |this, cx, picked| {
                let Some(path) = picked else { return };
                match this.settings.export_to(&path) {
                    Ok(()) => this.push_toast(
                        format!("已导出设置：{}", path.display()),
                        false,
                        cx,
                    ),
                    Err(error) => this.push_toast(format!("导出失败: {error}"), true, cx),
                }
            },
        );
    }

    /// Replace the configuration from a file the user picks, then apply it.
    pub fn import_settings(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new()
            .set_title("导入设置")
            .add_filter("JSON", &["json"]);
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_file(),
            |this, cx, picked| {
                let Some(path) = picked else { return };
                match seepic_core::settings::Settings::import_from(&path) {
                    Ok(settings) => {
                        this.settings = settings;
                        // Written straight back, so the imported file becomes
                        // the live config rather than being reverted at the next
                        // change.
                        let _ = this.settings.save();
                        this.apply_settings();
                        // The sort order is app state as well as a setting, so
                        // an imported one has to go through the setter: it
                        // re-sorts, drops stale thumbnails, and reloads the page.
                        let order = this.settings.sort_order;
                        this.set_sort_order(order, cx);
                        this.push_toast(
                            format!("已导入设置：{}", path.display()),
                            false,
                            cx,
                        );
                    }
                    Err(error) => this.push_toast(format!("导入失败: {error}"), true, cx),
                }
            },
        );
    }

    /// Native picker for the picture drawn behind the image.
    pub fn pick_background_image(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new()
            .set_title("选择背景图片")
            .add_filter("图片", &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"]);
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_file(),
            |this, cx, picked| {
                if let Some(path) = picked {
                    this.set_background_image(path, cx);
                }
            },
        );
    }

    fn image_size(&self) -> IvSize {
        self.current.as_ref().map(|i| i.size).unwrap_or_default()
    }

    fn pan(&mut self, dx: f32, dy: f32, window: &Window) {
        self.zoom.pan(
            IvPoint::new(dx, dy),
            self.image_size(),
            self.viewport(window),
            window.scale_factor(),
        );
    }

    /// Move the view to one of the four corners of the image.
    fn scroll_to_corner(&mut self, fx: f32, fy: f32, window: &Window) {
        self.zoom.scroll_to_fraction(
            fx,
            fy,
            self.image_size(),
            self.viewport(window),
            window.scale_factor(),
        );
    }

    /// True when an open overlay is under the pointer and therefore owns the
    /// wheel.
    ///
    /// GPUI delivers a scroll to every hitbox under the pointer, not just the
    /// topmost, so the canvas has to opt out by geometry. The rectangles here
    /// mirror the ones the overlays lay themselves out with.
    fn overlay_owns_pointer(&self, point: IvPoint, window: &Window) -> bool {
        let width = f32::from(window.bounds().size.width);
        let height = f32::from(window.bounds().size.height);
        let inside = |left: f32, top: f32, w: f32, h: f32| {
            point.x >= left && point.x <= left + w && point.y >= top && point.y <= top + h
        };

        // The settings window covers everything.
        if self.show_settings {
            return true;
        }
        // The navigator and the command picker are centred modals.
        if self.show_navigator && inside((width - 520.) / 2., (height - 460.) / 2., 520., 460.) {
            return true;
        }
        if self.search.is_some() {
            return true;
        }
        if self.picker.is_some() && inside((width - 600.) / 2., (height - 520.) / 2., 600., 520.) {
            return true;
        }

        // A panel is a centred dialog with a full-window backdrop, so it owns
        // the pointer wherever it is.
        self.current_panel().is_some()
    }

    /// Move the thumbnail grid's scroll target by one wheel delta.
    ///
    /// The grid is an overlay that lets scroll through to the canvas; the
    /// canvas handler routes the delta here. The wheel changes a target, not
    /// the drawn offset: [`Self::advance_contact_scroll`] eases the list
    /// towards it over a few frames, which is what makes a notch glide instead
    /// of jumping 100 px and a burst of notches accumulate into one motion.
    pub fn scroll_contact_sheet(&mut self, dy: f32, columns: usize, cx: &mut Context<Self>) {
        let rows = self.contact_matches.len().div_ceil(columns.max(1));
        if rows == 0 || dy == 0. {
            return;
        }
        // Measure from where the previous wheel was heading while an animation
        // is running; the drawn offset lags behind the target.
        let base = if self.contact_scroll_animating {
            self.contact_scroll_target
        } else {
            f32::from(self.contact_scroll.offset().y)
        };
        // Compute the range from the rows rather than `max_offset`: the list
        // owns its offset and only refreshes the handle's maximum during
        // prepaint, which made the first wheel events after opening a no-op.
        let viewport = f32::from(self.contact_scroll.bounds().size.height);
        let content = rows as f32 * crate::contact_sheet::CELL_HEIGHT;
        let max = -(content - viewport).max(0.);
        let target = (base + dy).clamp(max, 0.);
        crate::perf!(
            "grid scroll dy={dy} base={base:.1} target={target:.1} max={max:.1} viewport={viewport:.1}"
        );
        if (target - self.contact_scroll_target).abs() < 0.01 {
            // Already at the top or bottom edge.
            return;
        }
        self.contact_scroll_target = target;
        self.contact_scroll_animating = true;
        self.contact_scroll_last = Some(Instant::now());
        cx.notify();
    }

    /// Walk the grid's offset one frame towards the wheel target.
    ///
    /// Exponential ease-out with a speed floor: the first step covers about a
    /// quarter of the remaining distance, and the floor keeps the last few
    /// pixels from crawling. At 60 Hz a 100 px notch settles in about 110 ms.
    fn advance_contact_scroll(&mut self, window: &Window) {
        if !self.contact_scroll_animating {
            return;
        }
        if !self.contact_sheet || self.window_minimized {
            self.contact_scroll_animating = false;
            self.contact_scroll_last = None;
            return;
        }
        let now = Instant::now();
        let dt = self
            .contact_scroll_last
            .map_or(1. / 60., |last| now.duration_since(last).as_secs_f32());
        self.contact_scroll_last = Some(now);
        // A long stall (the window was dragged, a decode blocked the frame)
        // must not turn into one huge jump.
        let dt = dt.clamp(1. / 240., 1. / 30.);

        let current = f32::from(self.contact_scroll.offset().y);
        let delta = self.contact_scroll_target - current;
        if delta.abs() < 0.5 {
            self.contact_scroll.set_offset(point(px(0.), px(self.contact_scroll_target)));
            self.contact_scroll_animating = false;
            self.contact_scroll_last = None;
            self.contact_top_row =
                ((-self.contact_scroll_target) / crate::contact_sheet::CELL_HEIGHT).floor() as usize;
            return;
        }

        let blend = 1. - (-18. * dt).exp();
        let step = (delta.abs() * blend).max(700. * dt).min(delta.abs());
        let next = current + step.copysign(delta);
        self.contact_scroll.set_offset(point(px(0.), px(next)));
        self.contact_top_row =
            ((-next) / crate::contact_sheet::CELL_HEIGHT).floor() as usize;
        crate::perf!(
            "grid ease offset={next:.1} target={:.1}",
            self.contact_scroll_target
        );
        window.request_animation_frame();
    }

    /// Jump the grid to the first item of one index section.
    ///
    /// `match_index` is the position in `contact_matches`, so the row follows
    /// the current filter rather than the unfiltered container.
    pub fn jump_contact_index(
        &mut self,
        match_index: usize,
        columns: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(&position) = self.contact_matches.get(match_index) else {
            return;
        };
        self.navigator_selection = position;
        let row = match_index / columns.max(1);
        self.contact_top_row = row;
        // Same reason as `refresh_contact_matches`: put the row at the top by
        // setting the pixel offset rather than asking for a nudge.
        let offset = -(row as f32) * crate::contact_sheet::CELL_HEIGHT;
        self.contact_scroll_target = offset;
        self.contact_scroll_animating = false;
        self.contact_scroll_last = None;
        self.contact_scroll.set_offset(point(px(0.), px(offset)));
        cx.notify();
    }

    /// Scroll the image by the wheel, and bounce to the neighbouring image when
    /// the scroll runs out of room.
    ///
    /// "Bounce" means a wheel with nothing left to scroll turns the page
    /// instead of stopping dead. Without
    /// it an image taller than the window traps the wheel at its edges.
    fn wheel_scroll(&mut self, dy: f32, window: &Window, cx: &mut Context<Self>) {
        // Speed 3 is the neutral setting, so it scales the delta 1:1.
        let speed = self.settings.wheel_speed.clamp(1, 9) as f32 / 3.;
        let dy = dy * speed;
        if dy == 0. {
            return;
        }

        let viewport = self.viewport(window);
        let layout = self
            .zoom
            .resolve(self.image_size(), viewport, window.scale_factor());
        let scrollable = layout.drawn.h > viewport.h + 0.5 || layout.drawn.w > viewport.w + 0.5;
        if !scrollable {
            if self.settings.wheel_bounce {
                self.step(if dy > 0. { -1 } else { 1 }, cx);
            }
            return;
        }

        let before = self.zoom.offset();
        self.pan(0., dy, window);
        if self.zoom.offset() == before && self.settings.wheel_bounce {
            self.step(if dy > 0. { -1 } else { 1 }, cx);
        }
    }

    fn reveal_in_folder(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.current_path() else {
            self.push_toast("没有可显示的文件", true, cx);
            return;
        };
        if !crate::shell_open::reveal(&path) {
            self.push_toast("无法打开文件夹", true, cx);
        }
    }

    /// The on-disk path of the current image, if it is a loose file.
    pub fn current_path(&self) -> Option<PathBuf> {
        let container = self.container.as_ref()?.lock().ok()?;
        match container.source(self.navigator.cursor())? {
            seepic_core::Source::File(path) => Some(path),
            seepic_core::Source::ArchiveMember { archive, .. } => Some(archive),
        }
    }

    // ---- slideshow --------------------------------------------------------

    pub fn start_slideshow(&mut self, cx: &mut Context<Self>) {
        if !self.has_images() {
            self.push_toast(s::NO_IMAGES_IN_ARCHIVE, true, cx);
            return;
        }
        self.slideshow.start();
        self.schedule_tick(cx);
        cx.notify();
    }

    pub fn stop_slideshow(&mut self, cx: &mut Context<Self>) {
        self.slideshow.stop();
        self.slideshow_tick = None;
        self.slideshow_started = None;
        cx.notify();
    }

    /// Step the slideshow interval by a sensible amount for its magnitude.
    ///
    /// Under a second the interesting steps are tens of milliseconds; above it
    /// they are whole seconds, so one stepper serves both.

    pub fn set_slideshow_interval(&mut self, interval: seepic_core::Interval, cx: &mut Context<Self>) {
        self.slideshow.set_interval(interval);
        if self.slideshow.is_running() {
            // Restart the pending timer so the new interval takes effect now
            // rather than after the old one elapses.
            self.schedule_tick(cx);
        }
        self.remember_slideshow(cx);
    }

    /// How long each slide is shown, before the gap.
    pub fn set_slideshow_display(&mut self, display: seepic_core::Interval, cx: &mut Context<Self>) {
        self.slideshow.set_display(display);
        if self.slideshow.is_running() {
            // Restart the cycle so the new timing applies at once.
            self.schedule_tick(cx);
        }
        self.remember_slideshow(cx);
    }

    /// Which unit each custom duration row reads in.
    pub fn set_slideshow_interval_unit(&mut self, unit: IntervalUnit, cx: &mut Context<Self>) {
        self.slideshow_interval_unit = unit;
        cx.notify();
    }

    pub fn set_slideshow_display_unit(&mut self, unit: IntervalUnit, cx: &mut Context<Self>) {
        self.slideshow_display_unit = unit;
        cx.notify();
    }

    pub fn set_slideshow_loop(&mut self, value: bool, cx: &mut Context<Self>) {
        self.slideshow.set_loop_playback(value);
        self.remember_slideshow(cx);
    }

    pub fn set_slideshow_random(&mut self, value: bool, cx: &mut Context<Self>) {
        self.slideshow.set_random(value);
        self.remember_slideshow(cx);
    }

    pub fn set_slideshow_progress(&mut self, value: bool, cx: &mut Context<Self>) {
        self.slideshow.set_show_progress(value);
        self.remember_slideshow(cx);
    }

    pub fn set_slideshow_effect(&mut self, fx: seepic_core::PageTurnFx, cx: &mut Context<Self>) {
        self.slideshow.set_page_turn_fx(fx);
        self.remember_slideshow(cx);
    }

    /// Save the slideshow's settings so the panel opens where it was left.
    fn remember_slideshow(&mut self, cx: &mut Context<Self>) {
        self.settings.slideshow = self.slideshow.config().clone();
        self.settings_changed(cx);
    }

    /// Arm the next advance. Dropping the previous task cancels it.
    fn schedule_tick(&mut self, cx: &mut Context<Self>) {
        let display = self.slideshow.display();
        let gap = self.slideshow.interval();
        let effect = self.slideshow.config().page_turn_fx;
        self.slideshow_started = Some(Instant::now());

        self.slideshow_tick = Some(cx.spawn(async move |this, cx| {
            // The slide sits still for its display time.
            cx.background_executor().timer(display).await;
            this.update(cx, |this, cx| {
                if !this.slideshow.is_running() {
                    return;
                }
                // Then the transition plays across the gap, ending exactly as
                // the next slide arrives.
                if effect != seepic_core::PageTurnFx::None {
                    this.page_turn = Some((effect, Instant::now()));
                    cx.notify();
                }
            })
            .ok();

            cx.background_executor().timer(gap).await;
            this.update(cx, |this, cx| {
                if !this.slideshow.is_running() {
                    return;
                }
                let total = this.navigator.len();
                let current = this.navigator.cursor();
                match this.slideshow.advance(current, total) {
                    seepic_core::Advance::To(index) => {
                        this.slideshow_started = Some(Instant::now());
                        this.select(index, cx);
                        // `select` only loads when the index actually changes;
                        // a one-image container must still keep ticking.
                        this.schedule_tick(cx);
                    }
                    seepic_core::Advance::Stop => this.stop_slideshow(cx),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    // ---- keyboard ---------------------------------------------------------

    pub fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;

        // A modal owns the keyboard while it is open, so its typing does not
        // also trigger shortcuts.
        if self.search.is_some() {
            return self.on_search_key(event, window, cx);
        }
        if self.picker.is_some() {
            return self.on_picker_key(event, window, cx);
        }
        if self.convert.is_some() {
            return self.on_converter_key(event, cx);
        }
        if self.contact_sheet && self.on_contact_sheet_key(event, window, cx) {
            return;
        }

        // Escape unwinds the interface one layer at a time, and only quits when
        // there is nothing left to close and the user asked for that.
        if keystroke.key == "escape" {
            cx.stop_propagation();
            if self.choice.is_some() {
                self.close_choice(cx);
            } else if self.picker.is_some() {
                self.close_command_picker(cx);
            } else if self.dialog.is_some() {
                self.cancel_dialog(cx);
            } else if self.password_prompt.is_some() {
                self.cancel_password(cx);
            } else if self.open_menu.is_some() {
                self.close_menu();
                cx.notify();
            } else if self.show_settings {
                self.show_settings = false;
                cx.notify();
            } else if self.show_navigator {
                self.show_navigator = false;
                cx.notify();
            } else if self.show_info {
                self.close_info_window(cx);
            } else if !self.panel_stack.is_empty() {
                self.close_panel(cx);
            } else if self.fullscreen {
                self.toggle_fullscreen(window, cx);
            } else if self.settings.esc_to_exit {
                cx.quit();
            }
            return;
        }

        // An open menu takes the arrows and Enter before any shortcut does.
        if self.open_menu.is_some() {
            match keystroke.key.as_str() {
                "down" => {
                    cx.stop_propagation();
                    self.move_menu_highlight(1, cx);
                    return;
                }
                "up" => {
                    cx.stop_propagation();
                    self.move_menu_highlight(-1, cx);
                    return;
                }
                "enter" => {
                    cx.stop_propagation();
                    self.activate_menu_row(window, cx);
                    return;
                }
                _ => {}
            }
        }

        let binding = canonical_binding(
            &keystroke.key,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
        );

        // The recorder takes the next chord, whatever it is.
        if let Some(command) = self.recording_key {
            cx.stop_propagation();
            if keystroke.key == "escape" {
                self.recording_key = None;
                self.push_toast("已取消录制", false, cx);
            } else {
                self.bind_recorded_key(command, &binding, cx);
            }
            return;
        }

        // A custom chord wins over the built-in table.
        if let Some(command) = self.settings.command_for_custom_key(&binding) {
            cx.stop_propagation();
            self.execute(command, window, cx);
            return;
        }

        if let Some(command) = Command::for_binding(&binding) {
            // Claim the key so GPUI does not also act on it. Tab in particular
            // would otherwise be consumed by focus traversal.
            cx.stop_propagation();
            if std::env::var_os("IV_TRACE_KEYS").is_some() {
                eprintln!("key {binding:?} -> {}", command.id());
            }
            self.execute(command, window, cx);
        } else if std::env::var_os("IV_TRACE_KEYS").is_some() {
            eprintln!("key {binding:?} -> (unbound)");
        }
    }

    pub fn on_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let dy = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y * 40.,
        };
        crate::perf!("wheel raw={:?} dy={dy} pos=({:.0},{:.0})", event.delta, f32::from(event.position.x), f32::from(event.position.y));

        // A scrollable panel sits over the canvas and both receive the wheel,
        // so the canvas stands down while the pointer is on the panel —
        // otherwise scrolling a panel would turn pages behind it.
        let pointer = IvPoint::new(f32::from(event.position.x), f32::from(event.position.y));
        if self.overlay_owns_pointer(pointer, window) {
            return;
        }

        // GPUI reports wheel-down as negative y (`Lines(-3)` for a notch), the
        // same direction the scroll target uses: down moves the offset
        // negative. Passing it straight through is what keeps the wheel from
        // fighting the animation.
        if self.contact_sheet {
            let columns = crate::contact_sheet::columns(window);
            self.scroll_contact_sheet(dy, columns, cx);
            cx.stop_propagation();
            return;
        }
        // The webtoon's virtual list scrolls itself; turning a page there would
        // fight the scroll the user is actually performing.
        if self.view_mode.is_webtoon() && !event.modifiers.control {
            return;
        }

        if event.modifiers.control {
            // Wheel zoom is anchored at the cursor, so the pixel under the
            // pointer stays put.
            let anchor = IvPoint::new(f32::from(event.position.x), f32::from(event.position.y));
            let step = if dy > 0. {
                seepic_core::ZoomStep::In
            } else {
                seepic_core::ZoomStep::Out
            };
            self.zoom.zoom_at(
                anchor,
                self.image_size(),
                self.viewport(window),
                step,
            );
        } else {
            match self.settings.wheel_action {
                WheelAction::Zoom => {
                    let anchor = IvPoint::new(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                    );
                    let step = if dy > 0. {
                        seepic_core::ZoomStep::In
                    } else {
                        seepic_core::ZoomStep::Out
                    };
                    self.zoom.zoom_at(anchor, self.image_size(), self.viewport(window), step);
                }
                WheelAction::Scroll => self.wheel_scroll(dy, window, cx),
                WheelAction::Page => {
                    // Page, unless the image is taller than the viewport, where
                    // scrolling is what the user wants instead.
                    let viewport = self.viewport(window);
                    let layout =
                        self.zoom
                            .resolve(self.image_size(), viewport, window.scale_factor());
                    if layout.drawn.h > viewport.h + 0.5 {
                        self.wheel_scroll(dy, window, cx);
                    } else if dy > 0. {
                        self.step(-1, cx);
                    } else if dy < 0. {
                        self.step(1, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    /// The top of the canvas in window coordinates.
    ///
    /// The side arrows are positioned inside the canvas, so the band that
    /// reveals them has to be measured from the same place.
    fn canvas_top(&self) -> f32 {
        if self.settings.chrome_mode.takes_rows() {
            TOP_BAR_HEIGHT
        } else {
            0.
        }
    }

    /// Refresh the hover-reveal state from a pointer position.
    ///
    /// Called both from the canvas and from the window root, so moving over the
    /// chrome or the bottom bar keeps the state current instead of freezing it.
    fn update_reveal(&mut self, position: IvPoint, window: &Window, cx: &mut Context<Self>) {
        let width = f32::from(window.bounds().size.width);
        let height = f32::from(window.bounds().size.height);
        const EDGE_REVEAL: f32 = 110.;
        const BOTTOM_REVEAL: f32 = 64.;

        // The arrows sit in a band as tall as the button they draw, centred in
        // the canvas. Only that band at the edge reveals them; the rest of the
        // edge stays a place to press or drag the image.
        let band_top = self.canvas_top()
            + ((self.viewport(window).h - crate::canvas::ARROW_HEIGHT) / 2.).max(0.);
        let in_band = position.y >= band_top
            && position.y <= band_top + crate::canvas::ARROW_HEIGHT;
        let near_edge = position.x < EDGE_REVEAL || position.x > width - EDGE_REVEAL;

        self.pointer_at_top = position.y < TOP_REVEAL;
        self.pointer_at_bottom = position.y > height - BOTTOM_REVEAL;
        self.arrows_visible = near_edge && in_band;
        self.cursor = Some(position);
        cx.notify();
    }

    /// A mouse button press, checked against the configured mouse bindings.
    ///
    /// Dispatch happens at the window root rather than on the canvas, because
    /// mouse bindings are global: any button can send any command from
    /// anywhere.
    pub fn on_root_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = match event.button {
            MouseButton::Left => MouseSlot::Left,
            MouseButton::Middle => MouseSlot::Middle,
            MouseButton::Right => MouseSlot::Right,
            MouseButton::Navigate(NavigationDirection::Back) => MouseSlot::X,
            MouseButton::Navigate(NavigationDirection::Forward) => MouseSlot::Y,
        };
        if let Some(command) = self.settings.mouse_command(slot) {
            self.execute(command, window, cx);
        }
    }

    /// Whether this release is the second half of a double click, recording the
    /// release as it goes.
    ///
    /// The platform never reports a double click inside the window: Windows
    /// turns the second press into `WM_LBUTTONDBLCLK`, which GPUI does not
    /// dispatch, so a double click arrives as one press and *two* releases — the
    /// click count it does report therefore never gets past one. Two releases
    /// close together in time and place are the double click, timed against the
    /// user's own double-click setting.
    pub fn register_click_up(&mut self, x: f32, y: f32) -> bool {
        let point = IvPoint::new(x, y);
        let doubled = self.last_click_up.is_some_and(|(at, last)| {
            at.elapsed() < crate::chrome_drag::double_click_interval()
                && (point.x - last.x).abs() <= DOUBLE_CLICK_SLOP
                && (point.y - last.y).abs() <= DOUBLE_CLICK_SLOP
        });
        self.last_click_up = if doubled {
            None
        } else {
            Some((Instant::now(), point))
        };
        doubled
    }

    /// Double-click detection for the `DoubleLeft` slot.
    pub fn on_root_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let doubled =
            self.register_click_up(f32::from(event.position.x), f32::from(event.position.y));
        if doubled && let Some(command) = self.settings.mouse_command(MouseSlot::DoubleLeft) {
            self.execute(command, window, cx);
        }
    }

    /// Track the pointer anywhere in the window, not just over the canvas.
    pub fn on_window_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A press on the chrome row arms a window drag; the first movement
        // starts it. This lives on the root because the pointer leaves the
        // chrome row as soon as the drag begins.
        if self.chrome_drag_armed && event.pressed_button == Some(MouseButton::Left) {
            self.chrome_drag_armed = false;
            let Some(hwnd) = crate::chrome_drag::native_handle(window) else {
                return;
            };
            // The move loop is modal and dispatches this thread's messages, our
            // main-thread tasks among them. Entered from inside an update — as
            // this handler is — those tasks call `Entity::update` while the
            // update on the stack still holds the app, and panic with "RefCell
            // already borrowed". So the loop is entered from a task instead,
            // once this update has returned.
            cx.spawn_in(window, async move |this, cx| {
                // One turn of the message loop: the update that got us here is
                // done and has released the app.
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(1))
                    .await;
                crate::chrome_drag::start_handle(hwnd);
                // The loop runs until the button comes up. Dragging a maximized
                // window restores it, and that resize arrives while the loop is
                // still running: the window counts as drawing, so the resize it
                // reports is dropped, and the chrome keeps the maximized layout
                // afterwards, with the right-hand buttons outside the window and
                // the status bar below it. Reading the bounds back once the loop
                // is over is what puts the layout right.
                this.update_in(cx, |_, window, cx| {
                    window.bounds_changed(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
        // Dragging the progress track keeps seeking while the button is held,
        // even once the pointer has left the bar.
        if self.scrubbing && event.pressed_button == Some(MouseButton::Left) {
            self.seek_at(f32::from(event.position.x), cx);
        }
        self.update_reveal(
            IvPoint::new(f32::from(event.position.x), f32::from(event.position.y)),
            window,
            cx,
        );
    }

    pub fn on_canvas_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Clicking the image returns keyboard control. An open context menu only
        // closes when the click lands outside it — closing on every press would
        // unmount the menu before a row could receive its click.
        window.focus(&self.focus, cx);
        if let Some(anchor) = self.context_menu {
            let chrome = self.chrome_height();
            let (left, top) = crate::context_menu::menu_origin(
                anchor,
                chrome,
                f32::from(window.bounds().size.width),
                f32::from(window.bounds().size.height),
            );
            let inside = (f32::from(event.position.x) - left).abs()
                <= crate::context_menu::MENU_WIDTH
                && f32::from(event.position.x) >= left
                && f32::from(event.position.y) - chrome >= top
                && f32::from(event.position.y) - chrome <= top + crate::context_menu::MENU_HEIGHT;
            if !inside {
                self.context_menu = None;
            }
        }
        self.pointer_at_top = f32::from(event.position.y) < TOP_REVEAL;
        self.last_mouse = Some(IvPoint::new(
            f32::from(event.position.x),
            f32::from(event.position.y),
        ));
        // A docked panel is a popover: clicking the image dismisses it, along
        // with any screens pushed on top, and so does an open menu.
        self.close_panel(cx);
        if self.open_menu.is_some() {
            self.close_menu();
        }
        // Dragging the image pans it when it is larger than the canvas; when
        // there is nothing to pan the same gesture moves the window instead,
        // so no part of the window is dead.
        let viewport = self.viewport(window);
        let layout = self
            .zoom
            .resolve(self.image_size(), viewport, window.scale_factor());
        // A fullscreen window already covers its display, so dragging it would
        // only slide it off the screen.
        if !self.fullscreen
            && layout.drawn.w <= viewport.w + 0.5
            && layout.drawn.h <= viewport.h + 0.5
        {
            self.chrome_drag_armed = true;
        }
        cx.notify();
    }

    /// Right click opens the canvas context menu at the pointer.
    pub fn on_canvas_context_menu(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.context_menu = Some(IvPoint::new(
            f32::from(event.position.x),
            f32::from(event.position.y),
        ));
        cx.notify();
    }

    pub fn on_canvas_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.last_mouse = None;
        cx.notify();
    }

    pub fn on_canvas_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = IvPoint::new(f32::from(event.position.x), f32::from(event.position.y));
        self.update_reveal(position, window, cx);
        self.reveal_anim_bar(cx);
        let previous = self.last_mouse.replace(position);

        if event.pressed_button == Some(MouseButton::Left) {
            if let Some(previous) = previous {
                self.zoom.pan(
                    IvPoint::new(position.x - previous.x, position.y - previous.y),
                    self.image_size(),
                    self.viewport(window),
                    window.scale_factor(),
                );
                cx.notify();
            }
        }
    }
}

/// Convert one container entry and write the result.
///
/// Runs on the background executor: it reads from the container, converts in
/// memory and writes one file, and reports only what the caller needs to count.
fn convert_one(
    container: &Arc<Mutex<seepic_core::Container>>,
    source: usize,
    name: &str,
    options: &seepic_core::convert::ConvertOptions,
) -> Result<PathBuf, seepic_core::convert::ConvertError> {
    use seepic_core::convert::ConvertError;
    use seepic_core::convert::{Overwrite, free_path, output_path};

    let (bytes, source_path, disk_path) = {
        let mut guard = container
            .lock()
            .map_err(|_| ConvertError::Read("容器状态已损坏".to_string()))?;
        let bytes = guard
            .read(source)
            .map_err(|error| ConvertError::Read(error.to_string()))?;
        match guard.source(source) {
            Some(seepic_core::Source::File(path)) => {
                let disk = path.clone();
                (bytes, path, Some(disk))
            }
            Some(seepic_core::Source::ArchiveMember { archive, .. }) => {
                // A member has no path of its own, so its output is named after
                // it and placed beside the archive — which is where a reader
                // would look for it.
                let parent = archive.parent().map(Path::to_path_buf).unwrap_or_default();
                (bytes, parent.join(name), None)
            }
            None => return Err(ConvertError::Read("找不到该条目".to_string())),
        }
    };

    let encoded = seepic_core::convert::convert_bytes(&bytes, options)?;
    let wanted = output_path(&source_path, options)?;
    let target = match options.overwrite {
        Overwrite::Overwrite => wanted.clone(),
        Overwrite::Skip if wanted.exists() => return Err(ConvertError::Skipped),
        Overwrite::Rename if wanted.exists() => free_path(&wanted),
        _ => wanted.clone(),
    };

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ConvertError::Write(error.to_string()))?;
    }
    std::fs::write(&target, &encoded).map_err(|error| ConvertError::Write(error.to_string()))?;

    // Deleting is only ever done for a real file on disk, and only after the
    // new file is safely written.
    if options.delete_source {
        if let Some(disk) = disk_path {
            let _ = seepic_core::fileops::delete_to_recycle_bin(&disk);
        }
    }
    Ok(target)
}

/// Build the drawable image, applying the user's rotation and flips.
///
/// `frame` selects a single animation frame to hold; `None` keeps every frame so
/// GPUI advances the animation itself. `flatten_alpha` forces the alpha channel
/// opaque, which is what the "ignore alpha" transparent-background treatment
/// means: transparent pixels are drawn as whatever colour they carry.
fn build_render(
    decoded: &DecodedImage,
    orientation: &Orientation,
    adjustments: &Adjustments,
    filter: FilterMode,
    frame: Option<usize>,
    flatten_alpha: bool,
) -> Option<(Arc<RenderImage>, IvSize)> {
    let selected: Vec<&decode::DecodedFrame> = match frame {
        Some(index) => decoded.frames.get(index).into_iter().collect(),
        None => decoded.frames.iter().collect(),
    };
    let mut frames = Vec::with_capacity(selected.len());

    for frame in selected {
        // An owned copy either way: the effects and filters below rewrite the
        // pixels in place, and a frame may well be living in a mapped slot,
        // which is read-only as far as this path is concerned.
        let (width, height, pixels) = if orientation.is_identity() {
            (decoded.width, decoded.height, frame.bgra.to_vec())
        } else {
            decode::transform_bgra(decoded.width, decoded.height, &frame.bgra, orientation)?
        };

        let mut pixels = pixels;
        seepic_core::effects::apply_bgra(&mut pixels, adjustments);
        if !filter.is_identity() {
            pixels = seepic_core::filters::apply_filter(filter, width, height, &pixels);
        }
        if flatten_alpha {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 0xff;
            }
        }
        let buffer = image::RgbaImage::from_raw(width, height, pixels)?;
        let delay = if frame.delay_ms > 0 {
            image::Delay::from_numer_denom_ms(frame.delay_ms, 1)
        } else {
            image::Delay::from_numer_denom_ms(100, 1)
        };
        frames.push(image::Frame::from_parts(buffer, 0, 0, delay));
    }

    if frames.is_empty() {
        return None;
    }

    let size = orientation.applied_size(IvSize::new(decoded.width as f32, decoded.height as f32));
    Some((Arc::new(RenderImage::new(frames)), size))
}

/// Wrap a single BGRA buffer as a drawable image.
fn render_image(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    let frame = image::Frame::from_parts(
        buffer,
        0,
        0,
        image::Delay::from_numer_denom_ms(100, 1),
    );
    Some(Arc::new(RenderImage::new(vec![frame])))
}

impl Render for ViewerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The info window can be closed with its own title bar buttons, which
        // leaves no callback in this view; reconcile the flag before the bars
        // are built so the button is not left lit.
        let _ = self.info_window_open(cx);
        // File drops arrive either through GPUI's OLE target or the legacy
        // WM_DROPFILES message; the latter also makes the path script-testable.
        let dropped = drop_fallback::take_pending();
        if !dropped.is_empty() {
            self.accept_dropped_paths(dropped, cx);
        }
        if let Some(hwnd) = native_window_handle(window) {
            drop_fallback::install(hwnd);
        }
        // Apply one easing step of the grid's wheel scroll, after this frame's
        // deltas have been added to the target and before the list is built.
        self.advance_contact_scroll(window);
        // Protection belongs to the grid's current viewport; with it closed,
        // the normal LRU bound applies again.
        if !self.contact_sheet && !self.thumbnail_protected.is_empty() {
            self.thumbnail_protected.clear();
        }
        // The wheel has settled: the batch that finished during the animation
        // can be uploaded now, and this frame's cells will show it.
        if !self.contact_scrolling() {
            self.install_deferred_thumbnails();
        }

        // Hand the textures evicted since the last frame back to the atlas.
        // This has to happen here rather than where the eviction happened:
        // `App::drop_image` needs the window, and during an update the app has
        // taken the window out of its own map. Without it the atlas keeps the
        // texture alive for every page turned.
        for image in std::mem::take(&mut self.pending_texture_drops) {
            cx.drop_image(image, Some(window));
        }

        // Asked once per frame, before the background work below decides
        // whether to spend a decode on a window nobody is looking at.
        self.window_minimized = crate::chrome_drag::is_minimized(window);

        // What the image is drawn at, and how much detail that needs, are both
        // decided here: this is the only place that knows the window's scale
        // factor and the live zoom.
        self.drawn_longest = self.drawn_longest(window);
        self.decode_target = self.compute_decode_target(window);
        self.layout_measured = true;
        // A load that arrived before this frame was waiting for a size.
        if std::mem::take(&mut self.load_pending) {
            self.load_current(cx);
        }
        self.upgrade_detail(cx);
        // A page adopted during a burst got its facts at once but not its EXIF;
        // once the pages stop turning, fetch it for the page that stayed.
        self.fetch_exif(cx);
        // Asked for, not built: the pyramid is assembled in the background so a
        // page turn never blocks on it.
        self.ensure_mips(cx);
        // And the small previews the next burst will run into, while nothing
        // else is decoding.
        self.prepare_previews(cx);
        // The checkerboard is only needed when the user asked for it, and only
        // changes when the canvas resizes.
        if self.transparency == Transparency::Grid {
            self.ensure_checker(self.viewport(window));
        }

        self.sync_theme(window, cx);

        // A window too narrow for the menu titles has no button left for an
        // open dropdown to hang from, so the dropdown goes with them.
        if self.open_menu.is_some() && !menu_bar::menus_fit(window) {
            self.close_menu();
        }

        // Keep the progress bar and animation frames moving.
        if self.slideshow.is_running() && self.slideshow.config().show_progress {
            window.request_animation_frame();
        }
        // Keep animating while a page turn is in flight, then settle.
        if self.page_turn_progress().is_some_and(|(_, progress)| progress < 1.) {
            window.request_animation_frame();
        } else {
            self.page_turn = None;
        }

        if let Some(toast) = &self.toast {
            if toast.expired(self.settings.toast_duration()) {
                self.toast = None;
            } else {
                window.request_animation_frame();
            }
        }

        // The root view owns keyboard focus so shortcuts work without the user
        // having to click the canvas first.
        if !self.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
        // A modal takes the keyboard while it is open.
        if self.dialog.is_some() && !self.dialog_focus.is_focused(window) {
            self.dialog_focus.focus(window, cx);
        } else if self.password_prompt.is_some() && !self.password_focus.is_focused(window) {
            self.password_focus.focus(window, cx);
        }

        let mut root = v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(Self::on_window_mouse_move))
            // An open menu is dismissed by a press anywhere outside it; the
            // dropdown stops the press from reaching here.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    if this.open_menu.is_some() {
                        this.close_menu();
                        cx.notify();
                    }
                }),
            )
            // A press that never moves is a click, so the drag must not stay
            // armed for the next unrelated drag (a slider, say).
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, _cx| {
                    this.chrome_drag_armed = false;
                    this.scrubbing = false;
                    let _ = event;
                }),
            )
            .on_any_mouse_down(cx.listener(Self::on_root_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_root_mouse_up));

        // In the fixed mode the bars take their own rows, so the image starts
        // below the top bar and above the status bar. The other two modes draw
        // them as overlays further down, over the image.
        let rows = self.settings.chrome_mode.takes_rows();
        if rows {
            root = root.child(menu_bar::render(self, window, cx));
        }

        // The decode size was already decided above from the canvas bounds the
        // last layout produced, and the chrome rows added here do not change
        // them within this frame, so there is nothing to recompute before the
        // canvas is built.

        let entity = cx.entity();
        let checker = self.checker_render();

        let content = div()
                .flex_1()
                .w_full()
                .min_h_0()
                .relative()
                .child(
                    v_flex()
                        .size_full()
                        .child(
                            div()
                                .flex_1()
                                .w_full()
                                .min_h_0()
                                .relative()
                                // The checkerboard sits under the image so a
                                // transparent PNG reads as transparent.
                                .when_some(checker, |this, checker| {
                                    this.child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .right_0()
                                            .bottom_0()
                                            .child(
                                                img(gpui::ImageSource::Render(checker))
                                                    .size_full()
                                                    .object_fit(ObjectFit::Fill),
                                            ),
                                    )
                                })
                                .child(canvas::render(self, window, cx)),
                        )
                        .when(self.settings.thumbnail_view.takes_row(), |this| {
                            this.child(thumbnails::render(self, entity.clone(), cx))
                        }),
                )
                // The floating modes draw the bars over the image: the automatic
                // one when the pointer nears the edge they belong to, and the
                // immersive one never.
                .when(!rows && self.top_bar_visible(), |this| {
                    this.child(v_flex().absolute().top_0().left_0().right_0().child(
                        menu_bar::render(self, window, cx),
                    ))
                })
                .when(!rows && self.status_bar_visible(), |this| {
                    this.child(
                        v_flex()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .child(status_bar::render(self, window, cx)),
                    )
                })
                .when(self.contact_sheet, |this| {
                    this.child(contact_sheet::render(self, window, cx))
                })
                .when(self.show_settings, |this| {
                    this.child(settings_panel::render(self, cx))
                })
                // Above the settings panel, which it belongs to: the panel
                // scrolls, so the list cannot live inside it.
                .when(self.choice.is_some(), |this| {
                    this.child(settings_panel::choice_popup(self, window, cx))
                })
                .when(self.show_navigator, |this| {
                    this.child(panels::navigator(self, window, cx))
                })
                .when_some(self.current_panel(), |this, panel| {
                    this.child(panels::render(self, panel, window, cx))
                })
                // The dropdown has to be drawn here, above the content area:
                // the menu strip is a flow row before it, so a dropdown nested
                // in the strip would be painted under the image.
                .when_some(
                    menu_bar::dropdown_overlay(self, window, cx),
                    |this, dropdown| this.child(dropdown),
                )
                // With the chrome hidden in fullscreen there is no button to
                // press, so the way out floats.
                .when(self.fullscreen && !self.chrome_visible(), |this| {
                    this.child(menu_bar::fullscreen_exit_overlay(self, cx))
                })
                .when_some(self.boundary_prompt.as_ref(), |this, prompt| {
                    this.child(crate::boundary_prompt::render(prompt, cx))
                })
                .when_some(self.convert.as_ref(), |this, dialog| {
                    this.child(crate::converter::render(self, dialog, cx))
                })
                .when(self.picker.is_some(), |this| {
                    this.child(panels::command_picker(self, cx))
                })
                .when(self.search.is_some(), |this| {
                    this.child(panels::file_search(self, cx))
                })
                .when(self.dialog.is_some(), |this| {
                    this.child(confirm_dialog::render(
                        self,
                        self.dialog.as_ref().unwrap(),
                        cx,
                    ))
                })
                .when(self.password_prompt.is_some(), |this| {
                    this.child(password_dialog::render(
                        self,
                        self.password_prompt.as_ref().unwrap(),
                        cx,
                    ))
                })
                .when_some(self.context_menu, |this, anchor| {
                    this.child(context_menu::render(
                        self,
                        anchor,
                        self.chrome_height(),
                        f32::from(window.bounds().size.width),
                        f32::from(window.bounds().size.height),
                        cx,
                    ))
                })
                .when(self.settings.show_message, |this| {
                    this.when_some(
                        self.toast.as_ref().map(|t| (t.text.clone(), t.is_error)),
                        |this, (text, is_error)| {
                            this.child(status_bar::toast(&text, is_error, cx))
                        },
                    )
                });

        root = root.child(content);

        // The status bar takes its own row in the fixed mode, which is what the
        // chrome height above accounts for; the floating modes drew it over the
        // image instead.
        if rows {
            root = root.child(status_bar::render(self, window, cx));
        }

        root
    }
}

/// A free name in `dir`, appending a counter when the name is already taken.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    };

    for counter in 1..1000 {
        let candidate = match extension {
            Some(extension) => dir.join(format!("{stem} ({counter}).{extension}")),
            None => dir.join(format!("{stem} ({counter})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

/// The platform window handle, for the WM_DROPFILES fallback.
fn native_window_handle(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // The inherent `window_handle` returns GPUI own handle, so the trait method
    // has to be named explicitly.
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Register the default key bindings. The command table owns the bindings; this
/// only makes them visible to GPUI's key dispatch for menus to display.
pub fn bind_keys(cx: &mut App) {
    let _ = cx;
}
