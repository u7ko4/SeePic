//! The command table.
//!
//! One definition per user-facing action, carrying its label, its group and its
//! default key binding. The menu bar, the panels, the context menu, the keyboard
//! handler and the custom-shortcut settings page all read from here, so the five
//! can never drift apart.
//!
//! The inventory covers what a reader expects from a desktop image viewer;
//! where a conventional shortcut is already claimed by GPUI (notably `Tab`,
//! which the focus system swallows) the binding differs and is rebindable from
//! the settings page.

/// Which settings page a command belongs under, grouped the way the shortcut
/// table presents them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    File,
    Window,
    Navigation,
    View,
    Zoom,
    Scroll,
    Rotate,
    ImageProc,
    Slideshow,
    Animation,
    PhotoFolder,
    Bookmark,
    Other,
}

impl Group {
    /// Every group, in the order the settings page lists them.
    pub const ALL: &'static [Group] = &[
        Group::File,
        Group::Window,
        Group::Navigation,
        Group::View,
        Group::Zoom,
        Group::Scroll,
        Group::Rotate,
        Group::ImageProc,
        Group::Slideshow,
        Group::Animation,
        Group::PhotoFolder,
        Group::Bookmark,
        Group::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::File => "文件",
            Group::Window => "窗口",
            Group::Navigation => "导航·排序",
            Group::View => "视图",
            Group::Zoom => "缩放",
            Group::Scroll => "滚动",
            Group::Rotate => "旋转·翻转",
            Group::ImageProc => "图像处理",
            Group::Slideshow => "幻灯片",
            Group::Animation => "动画",
            Group::PhotoFolder => "照片文件夹",
            Group::Bookmark => "收藏夹",
            Group::Other => "其它",
        }
    }
}

/// Everything the user can invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    // ---- File -----------------------------------------------------------
    OpenFile,
    OpenFolder,
    OpenRecent,
    NewWindow,
    CloseImage,
    RevealInFolder,
    /// The file navigator list.
    SelectImage,
    Rename,
    /// Move the file to the recycle bin.
    DeleteFile,
    Print,
    Convert,
    /// Save what is on screen, with zoom and effects applied.
    SaveResized,
    Settings,
    About,
    Quit,

    // ---- Window ---------------------------------------------------------
    ToggleFullscreen,
    Minimize,
    Maximize,
    AlwaysOnTop,
    /// Toggle whether the top bar and status bar keep their own rows.
    ToggleChromeRows,
    /// The three 界面模式 choices, as explicit commands so each can be bound.
    ChromeFixed,
    ChromeAuto,
    ChromeImmersive,
    ToggleBorder,
    ResizeWindowToImage,

    // ---- Navigation and sorting ----------------------------------------
    NextImage,
    PrevImage,
    FirstImage,
    LastImage,
    Next5,
    Prev5,
    Next10,
    Prev10,
    NextRandom,
    PrevRandom,
    NextContainer,
    PrevContainer,
    SortByName,
    SortByDateModified,
    SortBySize,
    SortByType,
    SortByDateTaken,
    SortByDateCreated,
    SortByLogical,
    ToggleSortAscending,
    /// Find a file by name inside the open folder or archive.
    SearchFiles,

    // ---- View -----------------------------------------------------------
    ViewMode100,
    ViewModeFitWindow,
    ViewModeFitWidth,
    ViewModeFitHeight,
    ViewModeTwoL2R,
    ViewModeTwoR2L,
    ViewModeWebtoon,
    /// Step through the view modes in order.
    ViewModeNext,
    ToggleStretchSmall,
    ToggleSeparateCover,
    ToggleZoomLock,
    ToggleScrollLock,
    /// Cycle the five transparent-background treatments.
    ToggleTransparentBg,
    TransparentGrid,
    TransparentWhite,
    TransparentBlack,
    TransparentIgnoreAlpha,
    TransparentBackgroundColor,
    ToggleThumbnails,
    ToggleInfo,
    ToggleImagePanel,
    /// The slideshow panel, which is only its interval and playback controls.
    ToggleSlideshowPanel,
    /// The command palette: the one entry point for every command, and the
    /// shortcut reference, since each row carries its binding.
    OpenCommandPalette,
    ToggleBookmarkPanel,
    /// The sort panel, which also previews the resulting order.
    ToggleSortPanel,

    // ---- Zoom -----------------------------------------------------------
    ZoomIn,
    ZoomOut,
    ZoomIn1,
    ZoomOut1,
    ZoomActual,
    ZoomReset,

    // ---- Scroll ---------------------------------------------------------
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    ScrollCenter,
    ScrollTopLeft,
    ScrollTopRight,
    ScrollBottomLeft,
    ScrollBottomRight,
    /// Space: scroll down, or page forward once the image bottom is reached.
    ScrollPageDown,

    // ---- Rotate and flip ------------------------------------------------
    RotateCw,
    RotateCcw,
    RotationNone,
    RotationL90,
    RotationR90,
    Rotation180,
    FlipHorizontal,
    FlipVertical,
    ResetTransform,

    // ---- Image processing ----------------------------------------------
    FilterNone,
    FilterSoften,
    FilterSoftenSharpen,
    FilterNext,
    InterpolationBilinear,
    InterpolationBicubic,
    InterpolationLanczos,
    InterpolationNext,
    ImageNegative,
    ImageGamma,
    ImageBrightness,
    ImageContrast,
    ImageSaturation,
    ImageGray,
    ImageSepia,
    /// Every channel, undoing a channel isolation.
    ChannelAll,
    ChannelR,
    ChannelG,
    ChannelB,
    ChannelA,
    ChannelNext,
    ResetImageProc,

    // ---- Clipboard and photo folders ------------------------------------
    CopyImage,
    /// Copy what is on screen rather than the original bitmap.
    CopyResized,
    CopyPath,
    CopyFile,
    /// Put the file on the clipboard marked for a move.
    CutFile,
    CopyExif,
    Paste,
    CopyToPhotoFolder,
    MoveToPhotoFolder,
    OpenPhotoFolder,

    // ---- Bookmark -------------------------------------------------------
    BookmarkAdd,
    BookmarkLoadRecent,
    ToggleBookmarkWindow,

    // ---- Slideshow ------------------------------------------------------
    SlideshowStop,
    Slideshow1,
    Slideshow2,
    Slideshow3,
    Slideshow4,
    Slideshow5,
    Slideshow6,
    Slideshow7,
    Slideshow8,
    Slideshow9,
    SlideshowCustom,
    SlideshowToggle,

    // ---- Animation ------------------------------------------------------
    AnimNext,
    AnimPrev,
    AnimPlayPause,
    AnimHome,
    AnimEnd,
    ToggleAnimInfo,

    // ---- Other ----------------------------------------------------------
    LaunchEditor,
    WallpaperDlg,
    PopupMeta,
}

impl Command {
    /// Every command, in the order the settings page should list them.
    pub const ALL: &'static [Command] = &[
        Command::OpenFile,
        Command::OpenFolder,
        Command::OpenRecent,
        Command::NewWindow,
        Command::CloseImage,
        Command::RevealInFolder,
        Command::SelectImage,
        Command::Rename,
        Command::DeleteFile,
        Command::Print,
        Command::Convert,
        Command::SaveResized,
        Command::Settings,
        Command::About,
        Command::Quit,
        Command::ToggleFullscreen,
        Command::Minimize,
        Command::Maximize,
        Command::AlwaysOnTop,
        Command::ToggleChromeRows,
        Command::ChromeFixed,
        Command::ChromeAuto,
        Command::ChromeImmersive,
        Command::ToggleBorder,
        Command::ResizeWindowToImage,
        Command::NextImage,
        Command::PrevImage,
        Command::FirstImage,
        Command::LastImage,
        Command::Next5,
        Command::Prev5,
        Command::Next10,
        Command::Prev10,
        Command::NextRandom,
        Command::PrevRandom,
        Command::NextContainer,
        Command::PrevContainer,
        Command::SortByName,
        Command::SortByDateModified,
        Command::SortBySize,
        Command::SortByType,
        Command::SortByDateTaken,
        Command::SortByDateCreated,
        Command::SortByLogical,
        Command::ToggleSortAscending,
        Command::SearchFiles,
        Command::ViewMode100,
        Command::ViewModeFitWindow,
        Command::ViewModeFitWidth,
        Command::ViewModeFitHeight,
        Command::ViewModeTwoL2R,
        Command::ViewModeTwoR2L,
        Command::ViewModeWebtoon,
        Command::ViewModeNext,
        Command::ToggleStretchSmall,
        Command::ToggleSeparateCover,
        Command::ToggleZoomLock,
        Command::ToggleScrollLock,
        Command::ToggleTransparentBg,
        Command::TransparentGrid,
        Command::TransparentWhite,
        Command::TransparentBlack,
        Command::TransparentIgnoreAlpha,
        Command::TransparentBackgroundColor,
        Command::ToggleThumbnails,
        Command::ToggleInfo,
        Command::OpenCommandPalette,
        Command::ToggleImagePanel,
        Command::ToggleSlideshowPanel,
        Command::ToggleBookmarkPanel,
        Command::ToggleSortPanel,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::ZoomIn1,
        Command::ZoomOut1,
        Command::ZoomActual,
        Command::ZoomReset,
        Command::ScrollUp,
        Command::ScrollDown,
        Command::ScrollLeft,
        Command::ScrollRight,
        Command::ScrollCenter,
        Command::ScrollTopLeft,
        Command::ScrollTopRight,
        Command::ScrollBottomLeft,
        Command::ScrollBottomRight,
        Command::ScrollPageDown,
        Command::RotateCw,
        Command::RotateCcw,
        Command::RotationNone,
        Command::RotationL90,
        Command::RotationR90,
        Command::Rotation180,
        Command::FlipHorizontal,
        Command::FlipVertical,
        Command::ResetTransform,
        Command::FilterNone,
        Command::FilterSoften,
        Command::FilterSoftenSharpen,
        Command::FilterNext,
        Command::InterpolationBilinear,
        Command::InterpolationBicubic,
        Command::InterpolationLanczos,
        Command::InterpolationNext,
        Command::ImageNegative,
        Command::ImageGamma,
        Command::ImageBrightness,
        Command::ImageContrast,
        Command::ImageSaturation,
        Command::ImageGray,
        Command::ImageSepia,
        Command::ChannelAll,
        Command::ChannelR,
        Command::ChannelG,
        Command::ChannelB,
        Command::ChannelA,
        Command::ChannelNext,
        Command::ResetImageProc,
        Command::CopyImage,
        Command::CopyResized,
        Command::CopyPath,
        Command::CopyFile,
        Command::CutFile,
        Command::CopyExif,
        Command::Paste,
        Command::CopyToPhotoFolder,
        Command::MoveToPhotoFolder,
        Command::OpenPhotoFolder,
        Command::BookmarkAdd,
        Command::BookmarkLoadRecent,
        Command::ToggleBookmarkWindow,
        Command::SlideshowStop,
        Command::Slideshow1,
        Command::Slideshow2,
        Command::Slideshow3,
        Command::Slideshow4,
        Command::Slideshow5,
        Command::Slideshow6,
        Command::Slideshow7,
        Command::Slideshow8,
        Command::Slideshow9,
        Command::SlideshowCustom,
        Command::SlideshowToggle,
        Command::AnimNext,
        Command::AnimPrev,
        Command::AnimPlayPause,
        Command::AnimHome,
        Command::AnimEnd,
        Command::ToggleAnimInfo,
        Command::LaunchEditor,
        Command::WallpaperDlg,
        Command::PopupMeta,
    ];

    /// The settings-page group this command is listed under.
    pub fn group(self) -> Group {
        use Command::*;
        match self {
            OpenFile | OpenFolder | OpenRecent | NewWindow | CloseImage | RevealInFolder
            | SelectImage | Rename | DeleteFile | Print | Convert | SaveResized
            | Settings | About | Quit => Group::File,

            ToggleFullscreen | Minimize | Maximize | AlwaysOnTop | ToggleChromeRows
            | ChromeFixed | ChromeAuto | ChromeImmersive
            | ToggleBorder | ResizeWindowToImage => Group::Window,

            NextImage | PrevImage | FirstImage | LastImage | Next5 | Prev5 | Next10 | Prev10
            | NextRandom | PrevRandom | NextContainer | PrevContainer | SortByName
            | SortByDateModified | SortBySize | SortByType | SortByDateTaken | SortByDateCreated
            | SortByLogical | ToggleSortAscending | SearchFiles => Group::Navigation,

            ViewMode100 | ViewModeFitWindow | ViewModeFitWidth | ViewModeFitHeight
            | ViewModeTwoL2R | ViewModeTwoR2L | ViewModeWebtoon | ViewModeNext
            | ToggleStretchSmall | ToggleSeparateCover | ToggleZoomLock | ToggleScrollLock
            | ToggleTransparentBg | TransparentGrid | TransparentWhite
            | TransparentBlack | TransparentIgnoreAlpha | TransparentBackgroundColor
            | ToggleThumbnails | ToggleInfo | OpenCommandPalette | ToggleImagePanel
            | ToggleSlideshowPanel | ToggleBookmarkPanel | ToggleSortPanel => Group::View,

            ZoomIn | ZoomOut | ZoomIn1 | ZoomOut1 | ZoomActual | ZoomReset => Group::Zoom,

            ScrollUp | ScrollDown | ScrollLeft | ScrollRight | ScrollCenter | ScrollTopLeft
            | ScrollTopRight | ScrollBottomLeft | ScrollBottomRight | ScrollPageDown => {
                Group::Scroll
            }

            RotateCw | RotateCcw | RotationNone | RotationL90 | RotationR90 | Rotation180
            | FlipHorizontal | FlipVertical | ResetTransform => Group::Rotate,

            FilterNone | FilterSoften | FilterSoftenSharpen | FilterNext
            | InterpolationBilinear | InterpolationBicubic | InterpolationLanczos
            | InterpolationNext | ImageNegative | ImageGamma | ImageBrightness | ImageContrast
            | ImageSaturation | ImageGray | ImageSepia | ChannelAll | ChannelR | ChannelG
            | ChannelB | ChannelA | ChannelNext | ResetImageProc => Group::ImageProc,

            SlideshowStop | Slideshow1 | Slideshow2 | Slideshow3 | Slideshow4 | Slideshow5
            | Slideshow6 | Slideshow7 | Slideshow8 | Slideshow9 | SlideshowCustom
            | SlideshowToggle => Group::Slideshow,

            AnimNext | AnimPrev | AnimPlayPause | AnimHome | AnimEnd | ToggleAnimInfo => {
                Group::Animation
            }

            CopyToPhotoFolder | MoveToPhotoFolder | OpenPhotoFolder => Group::PhotoFolder,

            BookmarkAdd | BookmarkLoadRecent | ToggleBookmarkWindow => {
                Group::Bookmark
            }

            CopyImage | CopyResized | CopyPath | CopyFile | CutFile | CopyExif | Paste
            | LaunchEditor | WallpaperDlg | PopupMeta => {
                Group::Other
            }
        }
    }

    pub fn label(self) -> &'static str {
        use Command::*;
        crate::i18n::tr(match self {
            OpenFile => "Open File",
            OpenFolder => "Open Folder",
            OpenRecent => "Recent",
            NewWindow => "New Window",
            CloseImage => "Close Image",
            RevealInFolder => "Reveal in Folder",
            SelectImage => "Select Image",
            Rename => "Rename",
            DeleteFile => "Delete File",
            Print => "Print",
            Convert => "Convert",
            SaveResized => "Save As Seen",
            Settings => "Settings",
            About => "About",
            Quit => "Quit",

            ToggleFullscreen => "Fullscreen",
            Minimize => "Minimize",
            Maximize => "Maximize / Restore",
            AlwaysOnTop => "Always on Top",
            ToggleChromeRows => "Chrome Rows / Overlay",
            ChromeFixed => "Chrome: Fixed",
            ChromeAuto => "Chrome: Auto-hide",
            ChromeImmersive => "Chrome: Immersive",
            ToggleBorder => "Show Window Border",
            ResizeWindowToImage => "Resize Window to Image",

            NextImage => "Next Image",
            PrevImage => "Previous Image",
            FirstImage => "First Image",
            LastImage => "Last Image",
            Next5 => "Forward 5",
            Prev5 => "Back 5",
            Next10 => "Forward 10",
            Prev10 => "Back 10",
            NextRandom => "Random Next",
            PrevRandom => "Random Previous",
            NextContainer => "Next Folder / Archive",
            PrevContainer => "Previous Folder / Archive",
            SortByName => "Sort by Name",
            SortByDateModified => "Sort by Modified",
            SortBySize => "Sort by Size",
            SortByType => "Sort by Type",
            SortByDateTaken => "Sort by Taken",
            SortByDateCreated => "Sort by Created",
            SortByLogical => "Natural Order",
            ToggleSortAscending => "Toggle Ascending / Descending",
            SearchFiles => "Search Files",

            ViewMode100 => "Actual Size (100%)",
            ViewModeFitWindow => "Fit Window",
            ViewModeFitWidth => "Fit Width",
            ViewModeFitHeight => "Fit Height",
            ViewModeTwoL2R => "Two-Page (L→R)",
            ViewModeTwoR2L => "Two-Page (R→L)",
            ViewModeWebtoon => "Webtoon",
            ViewModeNext => "Cycle View Mode",
            ToggleStretchSmall => "Stretch Small Images",
            ToggleSeparateCover => "Separate Cover",
            ToggleZoomLock => "Lock Zoom",
            ToggleScrollLock => "Lock Scroll",
            ToggleTransparentBg => "Transparent Background (Cycle)",
            TransparentGrid => "Transparent: Grid",
            TransparentWhite => "Transparent: White",
            TransparentBlack => "Transparent: Black",
            TransparentIgnoreAlpha => "Transparent: Ignore Alpha",
            TransparentBackgroundColor => "Transparent: Background Color",
            ToggleThumbnails => "Thumbnail Strip",
            ToggleInfo => "File / EXIF Info",
            ToggleImagePanel => "Image Panel",
            ToggleSlideshowPanel => "Slideshow Panel",
            OpenCommandPalette => "Command Palette",
            ToggleBookmarkPanel => "Bookmarks",
            ToggleSortPanel => "Sort Panel",

            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ZoomIn1 => "Zoom In 1%",
            ZoomOut1 => "Zoom Out 1%",
            ZoomActual => "Actual Size (100%)",
            ZoomReset => "Reset Zoom",

            ScrollUp => "Scroll Up",
            ScrollDown => "Scroll Down",
            ScrollLeft => "Scroll Left",
            ScrollRight => "Scroll Right",
            ScrollCenter => "Center",
            ScrollTopLeft => "Scroll to Top Left",
            ScrollTopRight => "Scroll to Top Right",
            ScrollBottomLeft => "Scroll to Bottom Left",
            ScrollBottomRight => "Scroll to Bottom Right",
            ScrollPageDown => "Scroll Page Down",

            RotateCw => "Rotate 90° CW",
            RotateCcw => "Rotate 90° CCW",
            RotationNone => "Rotation: None",
            RotationL90 => "Rotation: 90° CCW",
            RotationR90 => "Rotation: 90° CW",
            Rotation180 => "Rotation: 180°",
            FlipHorizontal => "Flip Horizontal",
            FlipVertical => "Flip Vertical",
            ResetTransform => "Reset Rotation & Flip",

            FilterNone => "Filter: None",
            FilterSoften => "Filter: Soften",
            FilterSoftenSharpen => "Filter: Soften + Sharpen",
            FilterNext => "Cycle Filter",
            InterpolationBilinear => "Interpolation: Bilinear",
            InterpolationBicubic => "Interpolation: Bicubic",
            InterpolationLanczos => "Interpolation: Lanczos",
            InterpolationNext => "Cycle Interpolation",
            ImageNegative => "Negative",
            ImageGamma => "Gamma",
            ImageBrightness => "Brightness",
            ImageContrast => "Contrast",
            ImageSaturation => "Saturation",
            ImageGray => "Grayscale",
            ImageSepia => "Sepia",
            ChannelAll => "RGB Channel: All",
            ChannelR => "RGB Channel: Red",
            ChannelG => "RGB Channel: Green",
            ChannelB => "RGB Channel: Blue",
            ChannelA => "RGB Channel: Alpha",
            ChannelNext => "Cycle RGB Channel",
            ResetImageProc => "Reset Image Effects",

            CopyImage => "Copy to Clipboard",
            CopyResized => "Copy Displayed Image",
            CopyPath => "Copy File Path",
            CopyFile => "Copy Image File",
            CutFile => "Cut Image File",
            CopyExif => "Copy EXIF Info",
            Paste => "Paste",
            CopyToPhotoFolder => "Copy to Photo Folder",
            MoveToPhotoFolder => "Move to Photo Folder",
            OpenPhotoFolder => "Open Photo Folder",

            BookmarkAdd => "Add Bookmark",
            BookmarkLoadRecent => "Open Recent Bookmark",
            ToggleBookmarkWindow => "Bookmarks",

            SlideshowStop => "Stop Slideshow",
            Slideshow1 => "Slideshow 1 s",
            Slideshow2 => "Slideshow 2 s",
            Slideshow3 => "Slideshow 3 s",
            Slideshow4 => "Slideshow 4 s",
            Slideshow5 => "Slideshow 5 s",
            Slideshow6 => "Slideshow 6 s",
            Slideshow7 => "Slideshow 7 s",
            Slideshow8 => "Slideshow 8 s",
            Slideshow9 => "Slideshow 9 s",
            SlideshowCustom => "Custom Slideshow Interval",
            SlideshowToggle => "Start / Pause Slideshow",

            AnimNext => "Next Frame",
            AnimPrev => "Previous Frame",
            AnimPlayPause => "Play / Pause Animation",
            AnimHome => "First Frame",
            AnimEnd => "Last Frame",
            ToggleAnimInfo => "Animation Info",

            LaunchEditor => "Open in Editor",
            WallpaperDlg => "Set as Wallpaper",
            PopupMeta => "Metadata Popup",
        })
    }

    /// The preset second count for the `SlideshowN` commands.
    pub fn slideshow_seconds(self) -> Option<u64> {
        use Command::*;
        match self {
            Slideshow1 => Some(1),
            Slideshow2 => Some(2),
            Slideshow3 => Some(3),
            Slideshow4 => Some(4),
            Slideshow5 => Some(5),
            Slideshow6 => Some(6),
            Slideshow7 => Some(7),
            Slideshow8 => Some(8),
            Slideshow9 => Some(9),
            _ => None,
        }
    }

    /// Default binding in canonical form, or `None` when unbound.
    ///
    /// `Ctrl+0`..`Ctrl+9` belong to the slideshow presets, so the zoom and fit
    /// commands use the bare digit keys instead — the convention most image
    /// viewers follow.
    pub fn default_binding(self) -> Option<&'static str> {
        use Command::*;
        let binding = match self {
            OpenFile => "ctrl-o",
            OpenFolder => "f",
            CloseImage => "ctrl-w",
            RevealInFolder => "ctrl-enter",
            SelectImage => "enter",
            Rename => "f2",
            DeleteFile => "delete",
            Print => "ctrl-p",
            Convert => "ctrl-t",
            Settings => "f5",
            About => "f1",
            Quit => "x",
            OpenRecent | NewWindow | SaveResized => return None,

            ToggleFullscreen => "alt-enter",
            AlwaysOnTop => "ctrl-a",
            ToggleChromeRows => "ctrl-shift-m",
            ChromeAuto => "ctrl-shift-b",
            Minimize | Maximize | ChromeFixed | ChromeImmersive | ToggleBorder
            | ResizeWindowToImage => return None,

            NextImage => "right",
            PrevImage => "left",
            FirstImage => "home",
            LastImage => "end",
            Next10 => "ctrl-pagedown",
            Prev10 => "ctrl-pageup",
            NextRandom => "ctrl-alt-pagedown",
            PrevRandom => "ctrl-alt-pageup",
            NextContainer => "]",
            PrevContainer => "[",
            SortByName => "alt-s",
            SearchFiles => "ctrl-shift-f",
            Next5 | Prev5 | SortByDateModified | SortBySize | SortByType | SortByDateTaken
            | SortByDateCreated | SortByLogical | ToggleSortAscending => {
                return None
            }

            ViewMode100 => "1",
            ViewModeFitWindow => "2",
            ViewModeFitWidth => "3",
            ViewModeFitHeight => "4",
            ViewModeTwoL2R => "z",
            ViewModeWebtoon => "5",
            ToggleStretchSmall => "ctrl-shift-z",
            ToggleZoomLock => "ctrl-k",
            ToggleTransparentBg => "alt-t",
            ToggleInfo => "ctrl-i",
            ToggleImagePanel => "alt-i",
            ToggleSlideshowPanel => "alt-p",
            OpenCommandPalette => "ctrl-shift-p",
            ViewModeTwoR2L | ViewModeNext | ToggleSeparateCover | ToggleScrollLock
            | TransparentGrid | TransparentWhite | TransparentBlack
            | TransparentIgnoreAlpha | TransparentBackgroundColor
            | ToggleSortPanel | ChannelAll => return None,

            ZoomIn => "ctrl-=",
            ZoomOut => "ctrl--",
            ZoomActual => "0",
            ZoomReset => "alt-0",
            ZoomIn1 | ZoomOut1 => return None,

            ScrollUp => "up",
            ScrollDown => "down",
            // The arrow keys navigate; scrolling with the keyboard is available
            // by rebinding in the settings page.
            ScrollLeft | ScrollRight => return None,
            ScrollCenter => "ctrl-home",
            ScrollTopLeft => "ctrl-shift-home",
            // The remaining corners have no key of their own; they are there to
            // be bound in the settings page.
            ScrollTopRight | ScrollBottomLeft => return None,
            ScrollBottomRight => "ctrl-shift-end",
            ScrollPageDown => "space",

            RotateCw => "ctrl-r",
            RotateCcw => "ctrl-shift-r",
            RotationNone => "alt-up",
            RotationL90 => "alt-left",
            RotationR90 => "alt-right",
            Rotation180 => "alt-down",
            FlipHorizontal => "ctrl-m",
            FlipVertical => "ctrl-f",
            ResetTransform => "ctrl-shift-0",

            FilterNone | FilterSoften | FilterSoftenSharpen | FilterNext
            | InterpolationBilinear | InterpolationBicubic | InterpolationLanczos
            | InterpolationNext | ImageBrightness | ImageContrast | ImageSaturation
            | ChannelR | ChannelG | ChannelB | ChannelA | ChannelNext | ResetImageProc => {
                return None
            }
            // `Ctrl+I` is the EXIF panel here, because GPUI's focus traversal
            // swallows `Tab`; the negative shortcut moves to the shift chord
            // rather than becoming unreachable.
            ImageNegative => "ctrl-shift-i",
            ImageGamma => "ctrl-g",
            ImageGray => "ctrl-y",
            ImageSepia => "ctrl-shift-s",

            CopyImage => "ctrl-c",
            CopyExif => "ctrl-shift-c",
            CutFile => "ctrl-x",
            Paste => "ctrl-v",
            CopyToPhotoFolder => "insert",
            MoveToPhotoFolder => "ctrl-insert",
            CopyResized | CopyPath | CopyFile | OpenPhotoFolder => return None,

            BookmarkAdd => "b",
            BookmarkLoadRecent => "f3",
            ToggleBookmarkPanel => "alt-b",
            ToggleBookmarkWindow => return None,

            SlideshowStop => "ctrl-0",
            Slideshow1 => "ctrl-1",
            Slideshow2 => "ctrl-2",
            Slideshow3 => "ctrl-3",
            Slideshow4 => "ctrl-4",
            Slideshow5 => "ctrl-5",
            Slideshow6 => "ctrl-6",
            Slideshow7 => "ctrl-7",
            Slideshow8 => "ctrl-8",
            Slideshow9 => "ctrl-9",
            SlideshowCustom => "alt-w",
            SlideshowToggle => return None,

            ToggleAnimInfo => "alt-a",
            AnimNext | AnimPrev | AnimPlayPause | AnimHome | AnimEnd => return None,

            LaunchEditor => "ctrl-e",
            ToggleThumbnails => "f4",
            WallpaperDlg | PopupMeta => return None,
        };
        Some(binding)
    }

    /// Extra bindings accepted in addition to the default one.
    ///
    /// Used where a conventional key is worth honouring even though it is not
    /// the primary binding — `Tab` for the EXIF panel being the main case.
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Command::ToggleInfo => &["tab"],
            // The palette replaced the read-only shortcut list, so it answers
            // to the key that list used.
            Command::OpenCommandPalette => &["f9"],
            _ => &[],
        }
    }

    /// A stable identifier, written into the config file when a command is
    /// bound to a key or a mouse button.
    ///
    /// Deliberately independent of [`Command::label`], which is user-facing
    /// text and may be reworded or translated.
    pub fn id(self) -> &'static str {
        macro_rules! ids {
            ($($variant:ident),* $(,)?) => {
                match self {
                    $(Command::$variant => stringify!($variant),)*
                }
            };
        }
        ids!(
            OpenFile, OpenFolder, OpenRecent, NewWindow, CloseImage, RevealInFolder, SelectImage,
            Rename, DeleteFile, Print, Convert, SaveResized, Settings, About, Quit,
            ToggleFullscreen, Minimize, Maximize, AlwaysOnTop, ToggleChromeRows, ChromeFixed,
            ChromeAuto, ChromeImmersive, ToggleBorder, ResizeWindowToImage,
            NextImage, PrevImage, FirstImage, LastImage, Next5, Prev5, Next10, Prev10, NextRandom,
            PrevRandom, NextContainer, PrevContainer, SortByName, SortByDateModified, SortBySize,
            SortByType, SortByDateTaken, SortByDateCreated, SortByLogical,
            ToggleSortAscending, SearchFiles,
            ViewMode100, ViewModeFitWindow, ViewModeFitWidth, ViewModeFitHeight, ViewModeTwoL2R,
            ViewModeTwoR2L, ViewModeWebtoon, ViewModeNext, ToggleStretchSmall, ToggleSeparateCover,
            ToggleZoomLock, ToggleScrollLock, ToggleTransparentBg,
            TransparentGrid, TransparentWhite, TransparentBlack, TransparentIgnoreAlpha,
            TransparentBackgroundColor, ToggleThumbnails, ToggleInfo, OpenCommandPalette,
            ToggleImagePanel,
            ToggleSlideshowPanel, ToggleBookmarkPanel, ToggleSortPanel,
            ZoomIn, ZoomOut, ZoomIn1, ZoomOut1, ZoomActual, ZoomReset,
            ScrollUp, ScrollDown, ScrollLeft, ScrollRight, ScrollCenter, ScrollTopLeft,
            ScrollTopRight, ScrollBottomLeft, ScrollBottomRight, ScrollPageDown,
            RotateCw, RotateCcw, RotationNone, RotationL90, RotationR90, Rotation180,
            FlipHorizontal, FlipVertical, ResetTransform,
            FilterNone, FilterSoften, FilterSoftenSharpen, FilterNext, InterpolationBilinear,
            InterpolationBicubic, InterpolationLanczos, InterpolationNext, ImageNegative,
            ImageGamma, ImageBrightness, ImageContrast, ImageSaturation, ImageGray, ImageSepia,
            ChannelAll, ChannelR, ChannelG, ChannelB, ChannelA, ChannelNext, ResetImageProc,
            CopyImage, CopyResized, CopyPath, CopyFile, CutFile, CopyExif, Paste,
            CopyToPhotoFolder, MoveToPhotoFolder, OpenPhotoFolder,
            BookmarkAdd, BookmarkLoadRecent, ToggleBookmarkWindow,
            SlideshowStop, Slideshow1, Slideshow2, Slideshow3, Slideshow4, Slideshow5, Slideshow6,
            Slideshow7, Slideshow8, Slideshow9, SlideshowCustom, SlideshowToggle,
            AnimNext, AnimPrev, AnimPlayPause, AnimHome, AnimEnd, ToggleAnimInfo,
            LaunchEditor, WallpaperDlg, PopupMeta,
        )
    }

    /// Look a command up by the identifier [`Command::id`] produced.
    pub fn from_id(id: &str) -> Option<Command> {
        Self::ALL
            .iter()
            .copied()
            .find(|command| command.id().eq_ignore_ascii_case(id))
    }

    /// Human-readable form of the default binding, for menus.
    pub fn binding_label(self) -> Option<String> {
        self.default_binding().map(pretty_binding)
    }

    /// The command bound to `binding`, if any.
    pub fn for_binding(binding: &str) -> Option<Command> {
        Self::ALL
            .iter()
            .copied()
            .find(|command| {
                command.default_binding() == Some(binding)
                    || command.aliases().contains(&binding)
            })
    }

    /// Every command in the given group, in table order.
    pub fn in_group(group: Group) -> impl Iterator<Item = Command> {
        Self::ALL
            .iter()
            .copied()
            .filter(move |command| command.group() == group)
    }

    /// The command that selects the given sort key, so the sort menu and the
    /// settings page share one mapping.
    pub fn for_sort_key(key: crate::library::SortKey) -> Command {
        use crate::library::SortKey;
        match key {
            SortKey::Name => Command::SortByName,
            SortKey::DateModified => Command::SortByDateModified,
            SortKey::DateCreated => Command::SortByDateCreated,
            SortKey::Size => Command::SortBySize,
            SortKey::Type => Command::SortByType,
            SortKey::DateTaken => Command::SortByDateTaken,
            SortKey::Logical => Command::SortByLogical,
        }
    }
}

/// Build the canonical binding string from a key and its modifiers.
///
/// Modifier order is fixed so that two spellings of the same chord compare
/// equal. The key is lowercased because holding Shift changes the reported
/// character, and that must not change which command matches.
pub fn canonical_binding(key: &str, ctrl: bool, alt: bool, shift: bool) -> String {
    let mut out = String::new();
    if ctrl {
        out.push_str("ctrl-");
    }
    if alt {
        out.push_str("alt-");
    }
    if shift {
        out.push_str("shift-");
    }
    out.push_str(&key.to_lowercase());
    out
}

/// Turn a canonical binding into something worth showing in a menu, e.g.
/// `ctrl-shift-o` becomes `Ctrl+Shift+O`.
pub fn pretty_binding(binding: &str) -> String {
    let mut parts: Vec<&str> = binding.split('-').collect();
    let key = parts.pop().unwrap_or_default();

    let mut out = String::new();
    for modifier in parts {
        let name = match modifier {
            "ctrl" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            other => other,
        };
        out.push_str(name);
        out.push('+');
    }

    let key = match key {
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "enter" => "Enter".to_string(),
        "delete" => "Del".to_string(),
        "space" => "Space".to_string(),
        "tab" => "Tab".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "=" => "+".to_string(),
        other => other.to_uppercase(),
    };
    out.push_str(&key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_binding_is_order_independent() {
        let a = canonical_binding("O", true, false, true);
        let b = canonical_binding("o", true, false, true);
        assert_eq!(a, "ctrl-shift-o");
        assert_eq!(a, b, "shift must not change which command matches");
    }

    #[test]
    fn canonical_binding_lists_modifiers_in_a_fixed_order() {
        assert_eq!(canonical_binding("pagedown", true, true, false), "ctrl-alt-pagedown");
        assert_eq!(canonical_binding("right", false, false, false), "right");
    }

    #[test]
    fn every_default_binding_resolves_back_to_its_command() {
        for command in Command::ALL {
            let Some(binding) = command.default_binding() else {
                continue;
            };
            assert_eq!(
                Command::for_binding(binding),
                Some(*command),
                "{binding} did not resolve back to {command:?}"
            );
        }
    }

    #[test]
    fn every_alias_resolves_back_to_its_command() {
        for command in Command::ALL {
            for alias in command.aliases() {
                assert_eq!(
                    Command::for_binding(alias),
                    Some(*command),
                    "alias {alias} did not resolve back to {command:?}"
                );
            }
        }
    }

    #[test]
    fn default_bindings_and_aliases_are_unique() {
        // A duplicate would make one of the two commands unreachable by key.
        let mut seen: Vec<&str> = Vec::new();
        for command in Command::ALL {
            let bindings = command
                .default_binding()
                .into_iter()
                .chain(command.aliases().iter().copied());
            for binding in bindings {
                assert!(
                    !seen.contains(&binding),
                    "{binding} is bound to both {command:?} and another command"
                );
                seen.push(binding);
            }
        }
    }

    #[test]
    fn presets_report_their_seconds() {
        assert_eq!(Command::Slideshow1.slideshow_seconds(), Some(1));
        assert_eq!(Command::Slideshow9.slideshow_seconds(), Some(9));
        assert_eq!(Command::SlideshowStop.slideshow_seconds(), None);
        assert_eq!(Command::NextImage.slideshow_seconds(), None);
    }

    #[test]
    fn every_command_has_a_label() {
        for command in Command::ALL {
            assert!(!command.label().is_empty(), "{command:?} has no label");
        }
    }

    #[test]
    fn every_command_belongs_to_a_group_and_every_group_is_used() {
        for command in Command::ALL {
            assert!(
                Group::ALL.contains(&command.group()),
                "{command:?} has an unknown group"
            );
        }
        for group in Group::ALL {
            assert!(
                Command::in_group(*group).next().is_some(),
                "{group:?} lists no commands"
            );
        }
    }

    #[test]
    fn groups_partition_the_table() {
        let total: usize = Group::ALL
            .iter()
            .map(|group| Command::in_group(*group).count())
            .sum();
        assert_eq!(total, Command::ALL.len(), "a command is in two groups or none");
    }

    #[test]
    fn the_table_has_no_duplicate_entries() {
        let mut seen: Vec<Command> = Vec::new();
        for command in Command::ALL {
            assert!(!seen.contains(command), "{command:?} is listed twice");
            seen.push(*command);
        }
    }

    #[test]
    fn sort_commands_cover_every_sort_key() {
        for key in crate::library::SortKey::ALL {
            let command = Command::for_sort_key(key);
            assert!(
                Command::ALL.contains(&command),
                "{key:?} maps to {command:?}, which is not in the table"
            );
        }
    }

    #[test]
    fn pretty_binding_reads_like_a_shortcut() {
        assert_eq!(pretty_binding("ctrl-o"), "Ctrl+O");
        assert_eq!(pretty_binding("ctrl-shift-o"), "Ctrl+Shift+O");
        assert_eq!(pretty_binding("right"), "→");
        assert_eq!(pretty_binding("f11"), "F11");
        assert_eq!(pretty_binding("ctrl-="), "Ctrl++");
    }

    #[test]
    fn ids_round_trip_and_are_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for command in Command::ALL {
            let id = command.id();
            assert!(!id.is_empty(), "{command:?} has no id");
            assert!(!seen.contains(&id), "{id} is used by two commands");
            seen.push(id);
            assert_eq!(
                Command::from_id(id),
                Some(*command),
                "{id} did not resolve back to {command:?}"
            );
        }
    }

    #[test]
    fn ids_are_case_insensitive_on_lookup() {
        assert_eq!(Command::from_id("openfile"), Some(Command::OpenFile));
        assert_eq!(Command::from_id("OPENFILE"), Some(Command::OpenFile));
        assert_eq!(Command::from_id("no-such-command"), None);
    }
}
