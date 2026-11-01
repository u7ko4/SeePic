//! User-facing text, collected in one place so the UI has no stray literals and
//! every entry goes through the i18n layer.
//!
//! Kept as one flat table, so entries for screens that are still being built
//! (the info panel, the password prompt) sit here alongside the ones in use.
#![allow(dead_code, non_snake_case)]

// Menus
pub fn MENU_FILE() -> &'static str { seepic_core::i18n::tr("File") }
pub fn MENU_VIEW() -> &'static str { seepic_core::i18n::tr("View") }
pub fn MENU_MORE() -> &'static str { seepic_core::i18n::tr("More") }
pub fn MENU_BOOKMARK() -> &'static str { seepic_core::i18n::tr("Bookmarks") }
pub fn MENU_GROUP() -> &'static str { seepic_core::i18n::tr("Group") }
pub fn MENU_ZOOM() -> &'static str { seepic_core::i18n::tr("Zoom") }
pub fn MENU_IMAGE() -> &'static str { seepic_core::i18n::tr("Image") }
pub fn MENU_SLIDESHOW() -> &'static str { seepic_core::i18n::tr("Slideshow") }
pub fn MENU_COPY_TO() -> &'static str { seepic_core::i18n::tr("Copy to") }
pub fn THUMBNAILS() -> &'static str { seepic_core::i18n::tr("Thumbnails") }

// File menu
pub fn OPEN_FILE() -> &'static str { seepic_core::i18n::tr("Open File…") }
pub fn OPEN_FOLDER() -> &'static str { seepic_core::i18n::tr("Open Folder…") }
pub fn CLOSE_IMAGE() -> &'static str { seepic_core::i18n::tr("Close Image") }
pub fn REVEAL_IN_FOLDER() -> &'static str { seepic_core::i18n::tr("Reveal in Folder") }
pub fn RENAME() -> &'static str { seepic_core::i18n::tr("Rename") }
pub fn DELETE() -> &'static str { seepic_core::i18n::tr("Delete") }
pub fn SETTINGS() -> &'static str { seepic_core::i18n::tr("Settings") }
pub fn EXIT() -> &'static str { seepic_core::i18n::tr("Quit") }

// Zoom menu
pub fn ZOOM_AUTO() -> &'static str { seepic_core::i18n::tr("Auto") }
pub fn ZOOM_ACTUAL() -> &'static str { seepic_core::i18n::tr("Actual Size (100%)") }
pub fn ZOOM_IN() -> &'static str { seepic_core::i18n::tr("Zoom In") }
pub fn ZOOM_OUT() -> &'static str { seepic_core::i18n::tr("Zoom Out") }
pub fn FIT_WINDOW() -> &'static str { seepic_core::i18n::tr("Fit Window") }
pub fn FIT_WIDTH() -> &'static str { seepic_core::i18n::tr("Fit Width") }
pub fn FIT_HEIGHT() -> &'static str { seepic_core::i18n::tr("Fit Height") }
pub fn STRETCH_SMALL() -> &'static str { seepic_core::i18n::tr("Stretch Small Images") }

// Image menu
pub fn ROTATE_CW() -> &'static str { seepic_core::i18n::tr("Rotate 90° CW") }
pub fn ROTATE_CCW() -> &'static str { seepic_core::i18n::tr("Rotate 90° CCW") }
pub fn FLIP_H() -> &'static str { seepic_core::i18n::tr("Flip Horizontal") }
pub fn FLIP_V() -> &'static str { seepic_core::i18n::tr("Flip Vertical") }
pub fn RESET_TRANSFORM() -> &'static str { seepic_core::i18n::tr("Reset Rotation & Flip") }
pub fn SHOW_INFO() -> &'static str { seepic_core::i18n::tr("File / EXIF Info") }

// Navigation
pub fn NEXT_IMAGE() -> &'static str { seepic_core::i18n::tr("Next Image") }
pub fn PREV_IMAGE() -> &'static str { seepic_core::i18n::tr("Previous Image") }
pub fn FIRST_IMAGE() -> &'static str { seepic_core::i18n::tr("First Image") }
pub fn LAST_IMAGE() -> &'static str { seepic_core::i18n::tr("Last Image") }
pub fn NEXT_CONTAINER() -> &'static str { seepic_core::i18n::tr("Next Folder / Archive") }
pub fn PREV_CONTAINER() -> &'static str { seepic_core::i18n::tr("Previous Folder / Archive") }

// Slideshow
pub fn SLIDESHOW_TITLE() -> &'static str { seepic_core::i18n::tr("Slideshow") }
pub fn SLIDESHOW_STOP() -> &'static str { seepic_core::i18n::tr("Stop Slideshow") }
pub fn SLIDESHOW_START() -> &'static str { seepic_core::i18n::tr("Start Slideshow") }
pub fn SLIDESHOW_CUSTOM() -> &'static str { seepic_core::i18n::tr("Custom Interval") }
pub fn SLIDESHOW_INTERVAL() -> &'static str { seepic_core::i18n::tr("Interval") }
pub fn SLIDESHOW_DURATION() -> &'static str { seepic_core::i18n::tr("Duration") }
pub fn SLIDESHOW_LOOP() -> &'static str { seepic_core::i18n::tr("Loop") }
pub fn SLIDESHOW_RANDOM() -> &'static str { seepic_core::i18n::tr("Random") }
pub fn SLIDESHOW_PROGRESS() -> &'static str { seepic_core::i18n::tr("Show Progress") }
pub fn SLIDESHOW_EFFECT() -> &'static str { seepic_core::i18n::tr("Transition") }
pub fn SLIDESHOW_SETTINGS() -> &'static str { seepic_core::i18n::tr("Slideshow Settings") }
pub fn SLIDESHOW_SETTINGS_HINT() -> &'static str { seepic_core::i18n::tr("Set how long each slide is shown.") }
pub fn SLIDESHOW_RESET() -> &'static str { seepic_core::i18n::tr("Reset") }
pub fn MILLISECONDS() -> &'static str { seepic_core::i18n::tr("ms") }
pub fn SECONDS() -> &'static str { seepic_core::i18n::tr("s") }
pub fn APPLY() -> &'static str { seepic_core::i18n::tr("OK") }
pub fn CANCEL() -> &'static str { seepic_core::i18n::tr("Cancel") }

// States
pub fn LOADING() -> &'static str { seepic_core::i18n::tr("Loading…") }
pub fn NO_IMAGES_IN_ARCHIVE() -> &'static str { seepic_core::i18n::tr("No images in this archive") }
pub fn PASSWORD_TITLE() -> &'static str { seepic_core::i18n::tr("Password Required") }
pub fn PASSWORD_HINT() -> &'static str { seepic_core::i18n::tr("This archive is encrypted. Enter its password.") }
pub fn PASSWORD_WRONG() -> &'static str { seepic_core::i18n::tr("Incorrect password") }
pub fn CONFIRM() -> &'static str { seepic_core::i18n::tr("OK") }
pub fn CLOSE() -> &'static str { seepic_core::i18n::tr("Close") }

// Info panel
pub fn INFO_FILE() -> &'static str { seepic_core::i18n::tr("File") }
pub fn INFO_NAME() -> &'static str { seepic_core::i18n::tr("Name") }
pub fn INFO_FOLDER() -> &'static str { seepic_core::i18n::tr("Location") }
pub fn INFO_SIZE() -> &'static str { seepic_core::i18n::tr("Size") }
pub fn INFO_DIMENSIONS() -> &'static str { seepic_core::i18n::tr("Dimensions") }
pub fn INFO_MODIFIED() -> &'static str { seepic_core::i18n::tr("Modified") }
pub fn INFO_TYPE() -> &'static str { seepic_core::i18n::tr("Type") }
pub fn INFO_EXIF() -> &'static str { seepic_core::i18n::tr("EXIF") }
pub fn INFO_CAMERA() -> &'static str { seepic_core::i18n::tr("Camera") }
pub fn INFO_LENS() -> &'static str { seepic_core::i18n::tr("Lens") }
pub fn INFO_EXPOSURE() -> &'static str { seepic_core::i18n::tr("Exposure") }
pub fn INFO_APERTURE() -> &'static str { seepic_core::i18n::tr("Aperture") }
pub fn INFO_ISO() -> &'static str { seepic_core::i18n::tr("ISO") }
pub fn INFO_FOCAL() -> &'static str { seepic_core::i18n::tr("Focal Length") }
pub fn INFO_TAKEN() -> &'static str { seepic_core::i18n::tr("Taken") }
pub fn INFO_ORIENTATION() -> &'static str { seepic_core::i18n::tr("Orientation") }
pub fn INFO_NO_EXIF() -> &'static str { seepic_core::i18n::tr("This image has no EXIF data") }
