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

            CopyImage | CopyResized | CopyPath | CopyFile | CopyExif | Paste | LaunchEditor
            | WallpaperDlg | PopupMeta => {
                Group::Other
            }
        }
    }

    pub fn label(self) -> &'static str {
        use Command::*;
        match self {
            OpenFile => "打开文件",
            OpenFolder => "打开文件夹",
            OpenRecent => "最近打开",
            NewWindow => "新建窗口",
            CloseImage => "关闭图像",
            RevealInFolder => "在文件夹中显示",
            SelectImage => "选择图像",
            Rename => "重命名",
            DeleteFile => "删除文件",
            Print => "打印",
            Convert => "转换图片",
            SaveResized => "另存为所见图像",
            Settings => "设置",
            About => "关于",
            Quit => "退出",

            ToggleFullscreen => "全屏",
            Minimize => "最小化",
            Maximize => "最大化/还原",
            AlwaysOnTop => "窗口置顶",
            ToggleChromeRows => "界面占位/浮出",
            ChromeFixed => "界面模式：固定",
            ChromeAuto => "界面模式：自动隐藏",
            ChromeImmersive => "界面模式：沉浸",
            ToggleBorder => "显示窗口边框",
            ResizeWindowToImage => "窗口适应图片大小",

            NextImage => "下一张",
            PrevImage => "上一张",
            FirstImage => "第一张",
            LastImage => "最后一张",
            Next5 => "后第 5 张",
            Prev5 => "前第 5 张",
            Next10 => "后第 10 张",
            Prev10 => "前第 10 张",
            NextRandom => "随机下一张",
            PrevRandom => "随机上一张",
            NextContainer => "下一个文件夹或压缩包",
            PrevContainer => "上一个文件夹或压缩包",
            SortByName => "按名称排序",
            SortByDateModified => "按修改时间排序",
            SortBySize => "按大小排序",
            SortByType => "按类型排序",
            SortByDateTaken => "按拍摄时间排序",
            SortByDateCreated => "按创建时间排序",
            SortByLogical => "逻辑顺序（数字按数值）",
            ToggleSortAscending => "切换升序/降序",
            SearchFiles => "搜索文件",

            ViewMode100 => "原始大小 (100%)",
            ViewModeFitWindow => "适合窗口",
            ViewModeFitWidth => "适合宽度",
            ViewModeFitHeight => "适合高度",
            ViewModeTwoL2R => "智能双页浏览 (左→右)",
            ViewModeTwoR2L => "智能双页浏览 (左←右)",
            ViewModeWebtoon => "漫画模式",
            ViewModeNext => "切换浏览方式",
            ToggleStretchSmall => "拉伸小图",
            ToggleSeparateCover => "为智能双页使用单独的封面",
            ToggleZoomLock => "固定缩放比例",
            ToggleScrollLock => "固定滚动位置",
            ToggleTransparentBg => "透明背景（循环切换）",
            TransparentGrid => "透明背景：网格",
            TransparentWhite => "透明背景：白色",
            TransparentBlack => "透明背景：黑色",
            TransparentIgnoreAlpha => "透明背景：忽略 Alpha",
            TransparentBackgroundColor => "透明背景：使用背景色",
            ToggleThumbnails => "缩略图栏",
            ToggleInfo => "文件/EXIF 信息",
            ToggleImagePanel => "图像面板",
            ToggleSlideshowPanel => "幻灯片播放面板",
            OpenCommandPalette => "指令面板",
            ToggleBookmarkPanel => "收藏夹",
            ToggleSortPanel => "排序面板",

            ZoomIn => "放大",
            ZoomOut => "缩小",
            ZoomIn1 => "放大 1%",
            ZoomOut1 => "缩小 1%",
            ZoomActual => "原始大小 (100%)",
            ZoomReset => "重置缩放",

            ScrollUp => "向上滚动",
            ScrollDown => "向下滚动",
            ScrollLeft => "向左滚动",
            ScrollRight => "向右滚动",
            ScrollCenter => "滚动到中心",
            ScrollTopLeft => "滚动到左上角",
            ScrollTopRight => "滚动到右上角",
            ScrollBottomLeft => "滚动到左下角",
            ScrollBottomRight => "滚动到右下角",
            ScrollPageDown => "向下滚动/翻页",

            RotateCw => "顺时针旋转 90°",
            RotateCcw => "逆时针旋转 90°",
            RotationNone => "旋转：不旋转",
            RotationL90 => "旋转：左 90°",
            RotationR90 => "旋转：右 90°",
            Rotation180 => "旋转：180°",
            FlipHorizontal => "水平翻转",
            FlipVertical => "垂直翻转",
            ResetTransform => "重置旋转与翻转",

            FilterNone => "滤镜：不使用",
            FilterSoften => "滤镜：减淡",
            FilterSoftenSharpen => "滤镜：减淡+锐化",
            FilterNext => "切换滤镜",
            InterpolationBilinear => "插值：Bilinear",
            InterpolationBicubic => "插值：Bicubic",
            InterpolationLanczos => "插值：Lanczos",
            InterpolationNext => "切换插值算法",
            ImageNegative => "负片",
            ImageGamma => "伽马值补偿",
            ImageBrightness => "亮度",
            ImageContrast => "对比度",
            ImageSaturation => "饱和度",
            ImageGray => "灰度",
            ImageSepia => "棕褐色",
            ChannelAll => "RGB 通道：全部",
            ChannelR => "RGB 通道：R",
            ChannelG => "RGB 通道：G",
            ChannelB => "RGB 通道：B",
            ChannelA => "RGB 通道：A",
            ChannelNext => "切换 RGB 通道",
            ResetImageProc => "重置图像效果",

            CopyImage => "复制到剪贴板",
            CopyResized => "复制当前显示的图像",
            CopyPath => "复制文件路径",
            CopyFile => "复制图像文件",
            CopyExif => "复制 EXIF 信息",
            Paste => "从剪贴板粘贴",
            CopyToPhotoFolder => "复制到图片文件夹",
            MoveToPhotoFolder => "移动到图片文件夹",
            OpenPhotoFolder => "打开图片文件夹",

            BookmarkAdd => "添加到收藏夹",
            BookmarkLoadRecent => "打开最近添加的收藏",
            ToggleBookmarkWindow => "收藏夹",

            SlideshowStop => "停止放映",
            Slideshow1 => "幻灯片 1 秒",
            Slideshow2 => "幻灯片 2 秒",
            Slideshow3 => "幻灯片 3 秒",
            Slideshow4 => "幻灯片 4 秒",
            Slideshow5 => "幻灯片 5 秒",
            Slideshow6 => "幻灯片 6 秒",
            Slideshow7 => "幻灯片 7 秒",
            Slideshow8 => "幻灯片 8 秒",
            Slideshow9 => "幻灯片 9 秒",
            SlideshowCustom => "幻灯片自定义间隔",
            SlideshowToggle => "开始/暂停放映",

            AnimNext => "下一动画帧",
            AnimPrev => "上一动画帧",
            AnimPlayPause => "播放/暂停动画",
            AnimHome => "第一个动画帧",
            AnimEnd => "最后一个动画帧",
            ToggleAnimInfo => "动画信息控件",

            LaunchEditor => "用图像编辑器打开",
            WallpaperDlg => "设为桌面壁纸",
            PopupMeta => "元信息弹窗",
        }
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
            CopyImage, CopyResized, CopyPath, CopyFile, CopyExif, Paste, CopyToPhotoFolder,
            MoveToPhotoFolder, OpenPhotoFolder,
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
