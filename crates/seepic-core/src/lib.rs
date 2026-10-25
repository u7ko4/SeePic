//! Pure-logic core for the image viewer.
//!
//! Nothing in this crate touches GPUI or any windowing API, so everything here
//! is unit-testable without a display.

pub mod archive;
pub mod command;
pub mod cache;
pub mod container;
pub mod convert;
pub mod decode;
pub mod effects;
pub mod fileops;
pub mod filters;
pub mod geom;
pub mod image_formats;
pub mod library;
pub mod metacache;
pub mod metadata;
pub mod natsort;
pub mod pages;
pub mod positions;
pub mod resample;
pub mod settings;
pub mod orientation;
pub mod slideshow;
pub mod swap;
pub mod transparency;
pub mod zoom;

pub use archive::{ArchiveBackend, ArchiveError, ArchiveFormat, Member};
pub use command::Command;
pub use container::{
    Container, ContainerError, ContainerKind, ContainerRef, Source, list_containers,
};
pub use geom::{Point, Size};
pub use image_formats::{
    ARCHIVE_EXTENSIONS, IMAGE_EXTENSIONS, is_archive_extension, is_image_extension,
};
pub use library::{BoundaryAction, Entry, Navigator, SortKey, SortOrder, Step};
pub use orientation::{Orientation, Rotation};
pub use pages::{ReadingDirection, Spread, ViewMode, spread, spread_starts, webtoon_sizes, webtoon_width};
pub use slideshow::{Advance, Interval, IntervalUnit, PageTurnFx, Slideshow, SlideshowConfig};
pub use transparency::{BackgroundMode, Transparency, auto_background, checker_tile, parse_hex_color};
pub use zoom::{FitAxis, ViewLayout, ZoomMode, ZoomState, ZoomStep};
