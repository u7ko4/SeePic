//! User-facing text, collected in one place so the UI has no stray literals and
//! a translation layer can be dropped in later.
//!
//! Kept as one flat table, so entries for screens that are still being built
//! (the info panel, the password prompt) sit here alongside the ones in use.
#![allow(dead_code)]

// Menus
pub const MENU_FILE: &str = "文件";
pub const MENU_VIEW: &str = "视图";
pub const MENU_MORE: &str = "更多";
pub const MENU_BOOKMARK: &str = "收藏夹";
pub const MENU_GROUP: &str = "分组";
pub const MENU_ZOOM: &str = "缩放";
pub const MENU_IMAGE: &str = "图像";
pub const MENU_SLIDESHOW: &str = "幻灯片播放";
pub const MENU_COPY_TO: &str = "复制到";
pub const THUMBNAILS: &str = "缩略图";

// File menu
pub const OPEN_FILE: &str = "打开文件…";
pub const OPEN_FOLDER: &str = "打开文件夹…";
pub const CLOSE_IMAGE: &str = "关闭图像";
pub const REVEAL_IN_FOLDER: &str = "在文件夹中显示";
pub const RENAME: &str = "重命名";
pub const DELETE: &str = "删除";
pub const SETTINGS: &str = "设置";
pub const EXIT: &str = "退出";

// Zoom menu
pub const ZOOM_AUTO: &str = "自动";
pub const ZOOM_ACTUAL: &str = "原始大小 (100%)";
pub const ZOOM_IN: &str = "放大";
pub const ZOOM_OUT: &str = "缩小";
pub const FIT_WINDOW: &str = "适应窗口";
pub const FIT_WIDTH: &str = "适应宽度";
pub const FIT_HEIGHT: &str = "适应高度";
pub const STRETCH_SMALL: &str = "拉伸小图";

// Image menu
pub const ROTATE_CW: &str = "顺时针旋转 90°";
pub const ROTATE_CCW: &str = "逆时针旋转 90°";
pub const FLIP_H: &str = "水平翻转";
pub const FLIP_V: &str = "垂直翻转";
pub const RESET_TRANSFORM: &str = "重置旋转与翻转";
pub const SHOW_INFO: &str = "文件/EXIF 信息";

// Navigation
pub const NEXT_IMAGE: &str = "下一张";
pub const PREV_IMAGE: &str = "上一张";
pub const FIRST_IMAGE: &str = "第一张";
pub const LAST_IMAGE: &str = "最后一张";
pub const NEXT_CONTAINER: &str = "下一个文件夹/压缩包";
pub const PREV_CONTAINER: &str = "上一个文件夹/压缩包";

// Slideshow
pub const SLIDESHOW_TITLE: &str = "幻灯片播放";
pub const SLIDESHOW_STOP: &str = "停止放映";
pub const SLIDESHOW_START: &str = "开始播放";
pub const SLIDESHOW_CUSTOM: &str = "自定义间隔";
pub const SLIDESHOW_INTERVAL: &str = "播放间隔";
pub const SLIDESHOW_DURATION: &str = "显示时长";
pub const SLIDESHOW_LOOP: &str = "循环";
pub const SLIDESHOW_RANDOM: &str = "随机";
pub const SLIDESHOW_PROGRESS: &str = "显示进度条";
pub const SLIDESHOW_EFFECT: &str = "翻页特效";
pub const SLIDESHOW_SETTINGS: &str = "幻灯片播放设置";
pub const SLIDESHOW_SETTINGS_HINT: &str = "设置每张幻灯片的显示时长。";
pub const SLIDESHOW_RESET: &str = "重置";
pub const MILLISECONDS: &str = "毫秒";
pub const SECONDS: &str = "秒";
pub const APPLY: &str = "确定";
pub const CANCEL: &str = "取消";

// States
pub const LOADING: &str = "正在载入…";
pub const NO_IMAGES_IN_ARCHIVE: &str = "压缩包内没有图片";
pub const PASSWORD_TITLE: &str = "需要密码";
pub const PASSWORD_HINT: &str = "此压缩包已加密，请输入密码。";
pub const PASSWORD_WRONG: &str = "密码错误";
pub const CONFIRM: &str = "确定";
pub const CLOSE: &str = "关闭";

// Info panel
pub const INFO_FILE: &str = "文件";
pub const INFO_NAME: &str = "名称";
pub const INFO_FOLDER: &str = "位置";
pub const INFO_SIZE: &str = "大小";
pub const INFO_DIMENSIONS: &str = "尺寸";
pub const INFO_MODIFIED: &str = "修改时间";
pub const INFO_TYPE: &str = "类型";
pub const INFO_EXIF: &str = "EXIF";
pub const INFO_CAMERA: &str = "相机";
pub const INFO_LENS: &str = "镜头";
pub const INFO_EXPOSURE: &str = "曝光";
pub const INFO_APERTURE: &str = "光圈";
pub const INFO_ISO: &str = "ISO";
pub const INFO_FOCAL: &str = "焦距";
pub const INFO_TAKEN: &str = "拍摄时间";
pub const INFO_ORIENTATION: &str = "方向";
pub const INFO_NO_EXIF: &str = "此图像没有 EXIF 信息";
