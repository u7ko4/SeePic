//! The standalone batch conversion window.
//!
//! It is a real OS window with the same custom title bar as the main window,
//! not an overlay: a batch is a long-running task with its own queue, preview
//! and report, and it should survive focusing the viewer.
//!
//! The layout follows the approved prototype: input on the left, settings and
//! preview on the right, and a pinned action bar at the bottom.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use seepic_core::batch::{
    self, Anchor, BatchError, BatchFormat, BatchOptions, ConflictPolicy, FailureCode,
    OutputPlacement, Resample, SizeMode,
};
use seepic_core::i18n::tr;
use seepic_core::{Container, IMAGE_EXTENSIONS};

use crate::shell_open;

/// Height of one queue row.
const ROW_HEIGHT: f32 = 46.;
/// Width of the input column.
const INPUT_WIDTH: f32 = 404.;

/// Where one source image comes from.
#[derive(Clone)]
pub enum BatchSource {
    /// A normal file on disk.
    File(PathBuf),
    /// A member of an archive the viewer already has open.
    Archive {
        container: Arc<Mutex<Container>>,
        index: usize,
        archive: PathBuf,
    },
}

impl BatchSource {
    pub fn read(&self) -> Result<Vec<u8>, String> {
        match self {
            BatchSource::File(path) => {
                std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
            }
            BatchSource::Archive {
                container, index, ..
            } => {
                let mut container = container
                    .lock()
                    .map_err(|_| "The archive is no longer available".to_string())?;
                container.read(*index).map_err(|error| error.to_string())
            }
        }
    }

    /// The path the output placement is derived from. For an archive member
    /// that is the archive itself, so the output lands beside it.
    pub fn path(&self) -> PathBuf {
        match self {
            BatchSource::File(path) => path.clone(),
            BatchSource::Archive { archive, .. } => archive.clone(),
        }
    }

    fn key(&self) -> String {
        match self {
            BatchSource::File(path) => std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.clone())
                .display()
                .to_string(),
            BatchSource::Archive { archive, index, .. } => {
                format!("{}#{index}", archive.display())
            }
        }
    }
}

/// One image handed to the window when it opens.
pub struct BatchInput {
    pub source: BatchSource,
    pub name: String,
    pub size: u64,
    pub dimensions: Option<(u32, u32)>,
    pub modified: Option<SystemTime>,
    /// Path relative to the folder the user added, for mirroring.
    pub relative: Option<PathBuf>,
}

impl BatchInput {
    pub fn file(path: PathBuf, relative: Option<PathBuf>) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let metadata = std::fs::metadata(&path).ok();
        Self {
            source: BatchSource::File(path),
            name,
            size: metadata.as_ref().map_or(0, |metadata| metadata.len()),
            dimensions: None,
            modified: metadata.and_then(|metadata| metadata.modified().ok()),
            relative,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn archive(
        container: Arc<Mutex<Container>>,
        archive: PathBuf,
        index: usize,
        name: String,
        size: u64,
        dimensions: Option<(u32, u32)>,
        modified: Option<SystemTime>,
        relative: Option<PathBuf>,
    ) -> Self {
        Self {
            source: BatchSource::Archive {
                container,
                index,
                archive,
            },
            name,
            size,
            dimensions,
            modified,
            relative,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Ready,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Clone)]
struct Failure {
    code: FailureCode,
    message: String,
}

struct Item {
    id: u64,
    input: BatchInput,
    selected: bool,
    excluded: bool,
    state: ItemState,
    output_path: Option<PathBuf>,
    output_size: u64,
    failure: Option<Failure>,
    /// The extension suggests an alpha channel. The preview refines this once
    /// the image is actually decoded.
    alpha_hint: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StageView {
    Preview,
    Result,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Template,
    Subfolder,
    LongEdge,
    BoxWidth,
    BoxHeight,
    Quality,
    Jobs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChoiceKind {
    Preset,
    Sort,
    Placement,
    Conflict,
    Format,
}

struct ChoicePopup {
    kind: ChoiceKind,
    /// Window coordinates of the click that opened the menu. The menu must
    /// stay there while the pointer moves, so this is captured once rather
    /// than read from the window every frame.
    anchor: Point<Pixels>,
    labels: Vec<&'static str>,
    selected: Option<usize>,
}

struct PreviewImage {
    render: Arc<RenderImage>,
    width: u32,
    height: u32,
}

struct PreviewState {
    before: PreviewImage,
    after: PreviewImage,
    output_width: u32,
    output_height: u32,
    estimated_bytes: u64,
}

struct Run {
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    total: usize,
    processed: usize,
    ok: usize,
    failed: usize,
    skipped: usize,
    started: Instant,
    elapsed: Option<Duration>,
    cancelled: bool,
    receiver: Receiver<RunMessage>,
}

impl Run {
    fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.
        } else {
            (self.processed as f32 / self.total as f32).clamp(0., 1.)
        }
    }
}

enum RunMessage {
    Started { id: u64 },
    Item { id: u64, outcome: ItemOutcome },
    Done { cancelled: bool },
}

enum ItemOutcome {
    Done { path: PathBuf, bytes: u64 },
    Skipped { reason: String },
    Failed { code: FailureCode, message: String },
}

/// What the result view shows.
struct RunSummary {
    ok: usize,
    failed: usize,
    skipped: usize,
    total: usize,
    input_bytes: u64,
    output_bytes: u64,
    elapsed: Duration,
    cancelled: bool,
    failures: Vec<(String, Failure)>,
}

#[derive(Clone)]
struct Job {
    id: u64,
    name: String,
    source: BatchSource,
    source_path: PathBuf,
    relative: Option<PathBuf>,
    modified: Option<SystemTime>,
    sequence: usize,
}

/// The batch converter window.
pub struct BatchWindow {
    items: Vec<Item>,
    next_id: u64,
    active: Option<u64>,
    options: BatchOptions,
    settings_collapsed: bool,
    show_more: bool,
    view: StageView,
    run: Option<Run>,
    summary: Option<RunSummary>,
    preview: Option<PreviewState>,
    preview_pending: bool,
    /// The item the last preview attempt was for. A failed decode must not be
    /// retried every frame, which is what an empty preview and an active item
    /// would otherwise ask for.
    preview_attempted: Option<u64>,
    preview_generation: u64,
    editing: Option<EditTarget>,
    edit_buffer: String,
    /// The first keystroke replaces a number instead of appending to it.
    edit_fresh: bool,
    choice: Option<ChoicePopup>,
    notice: Option<String>,
    confirm_overwrite: Option<Vec<PathBuf>>,
    /// The subset a retry asked for, kept across the overwrite confirmation.
    confirm_only: Option<HashSet<u64>>,
    mirror_root: Option<PathBuf>,
    focus: FocusHandle,
}

impl BatchWindow {
    pub fn new(
        inputs: Vec<BatchInput>,
        only: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut next_id = 1;
        let items: Vec<Item> = inputs
            .into_iter()
            .filter(|input| only.as_ref().is_none_or(|wanted| input.name == *wanted))
            .map(|input| {
                let id = next_id;
                next_id += 1;
                Item {
                    id,
                    alpha_hint: may_have_alpha(&input.name),
                    input,
                    selected: false,
                    excluded: false,
                    state: ItemState::Ready,
                    output_path: None,
                    output_size: 0,
                    failure: None,
                }
            })
            .collect();
        let active = items.first().map(|item| item.id);
        let mirror_root = items
            .first()
            .and_then(|item| item.input.source.path().parent().map(Path::to_path_buf));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        Self {
            items,
            next_id,
            active,
            options: BatchOptions::default(),
            settings_collapsed: false,
            show_more: false,
            view: StageView::Preview,
            run: None,
            summary: None,
            preview: None,
            preview_pending: false,
            preview_attempted: None,
            preview_generation: 0,
            editing: None,
            edit_buffer: String::new(),
            edit_fresh: false,
            choice: None,
            notice: None,
            confirm_overwrite: None,
            confirm_only: None,
            mirror_root,
            focus,
        }
    }

    fn running(&self) -> bool {
        self.run.is_some()
    }

    fn active_item(&self) -> Option<&Item> {
        let id = self.active?;
        self.items.iter().find(|item| item.id == id)
    }

    fn active_index(&self) -> Option<usize> {
        let id = self.active?;
        self.items.iter().position(|item| item.id == id)
    }

    fn selected_count(&self) -> usize {
        self.items.iter().filter(|item| item.selected).count()
    }

    fn runnable_count(&self) -> usize {
        self.items.iter().filter(|item| !item.excluded).count()
    }

    fn total_bytes(&self) -> u64 {
        self.items.iter().map(|item| item.input.size).sum()
    }

    fn alpha_count(&self) -> usize {
        self.items.iter().filter(|item| item.alpha_hint).count()
    }

    fn excluded_count(&self) -> usize {
        self.items.iter().filter(|item| item.excluded).count()
    }

    fn step(&self) -> usize {
        if self.items.is_empty() {
            0
        } else if self.running() {
            2
        } else {
            1
        }
    }

    fn preset(&self) -> Option<usize> {
        presets().iter().position(|preset| {
            preset.options.size == self.options.size
                && preset.options.format == self.options.format
                && preset.options.quality == self.options.quality
                && preset.options.only_shrink == self.options.only_shrink
        })
    }

    fn summary_line(&self) -> String {
        let size = match self.options.size {
            SizeMode::Original => tr("Original size").to_string(),
            SizeMode::LongEdge(edge) => format!("{} {edge}", tr("Long edge")),
            SizeMode::Fit { width, height } => format!("{} {width}×{height}", tr("Fit")),
            SizeMode::Crop { width, height, .. } => {
                format!("{} {width}×{height}", tr("Crop"))
            }
        };
        let format = tr(self.options.format.short_label());
        let quality = if self.options.format.uses_quality() {
            format!(" {}", self.options.quality)
        } else {
            String::new()
        };
        format!(
            "{size} · {format}{quality} · {}",
            tr(self.options.conflict.label())
        )
    }

    fn target_dims(&self, item: &Item) -> Option<(u32, u32)> {
        let (width, height) = item.input.dimensions?;
        let size = batch::target_size(
            seepic_core::Size::new(width as f32, height as f32),
            self.options.size,
            self.options.only_shrink,
        );
        Some((size.w.round() as u32, size.h.round() as u32))
    }

    fn planned_name(&self, item: &Item, index: usize) -> String {
        let Some(dims) = self.target_dims(item).or(item.input.dimensions) else {
            return tr("(size at conversion)").to_string();
        };
        batch::output_file_name(&self.options, &item.input.name, dims.0, dims.1, index + 1)
    }

    /// Whether every included item has a size to estimate from. Before a
    /// preview or a conversion, most rows do not: the list deliberately does
    /// not read headers, so a queue-wide estimate would be a guess.
    fn estimates_ready(&self) -> bool {
        self.items
            .iter()
            .filter(|item| !item.excluded)
            .all(|item| item.input.dimensions.is_some())
    }

    fn estimated_one(&self, item: &Item) -> u64 {
        let Some((width, height)) = self.target_dims(item) else {
            return 0;
        };
        let format = match self.options.format {
            BatchFormat::Keep => BatchFormat::from_extension(
                item.input
                    .name
                    .rsplit_once('.')
                    .map(|(_, extension)| extension)
                    .unwrap_or(""),
            )
            .unwrap_or(BatchFormat::Png),
            other => other,
        };
        batch::estimate_bytes(width, height, format, self.options.quality)
    }

    fn estimated_total(&self) -> u64 {
        self.items
            .iter()
            .filter(|item| !item.excluded)
            .map(|item| self.estimated_one(item))
            .sum()
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.confirm_overwrite = None;
        self.confirm_only = None;
        self.summary = None;
        self.view = StageView::Preview;
        // The rows describe the next run; a previous run's ticks would make
        // the queue look converted under parameters they were not made with.
        for item in &mut self.items {
            item.output_path = None;
            item.output_size = 0;
            item.failure = None;
            item.state = if item.excluded {
                ItemState::Skipped
            } else {
                ItemState::Ready
            };
        }
        self.queue_preview(cx);
        cx.notify();
    }

    // ---- list editing -----------------------------------------------------

    fn set_active(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.active == Some(id) {
            return;
        }
        self.active = Some(id);
        self.preview = None;
        self.preview_attempted = None;
        self.queue_preview(cx);
        cx.notify();
    }

    fn toggle_selected(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.selected = !item.selected;
        }
        cx.notify();
    }

    fn select_all(&mut self, selected: bool, cx: &mut Context<Self>) {
        for item in &mut self.items {
            item.selected = selected;
        }
        cx.notify();
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.select_all(false, cx);
    }

    fn move_selected(&mut self, delta: isize, cx: &mut Context<Self>) {
        if delta < 0 {
            for index in 1..self.items.len() {
                if self.items[index].selected && !self.items[index - 1].selected {
                    self.items.swap(index, index - 1);
                }
            }
        } else {
            for index in (0..self.items.len().saturating_sub(1)).rev() {
                if self.items[index].selected && !self.items[index + 1].selected {
                    self.items.swap(index, index + 1);
                }
            }
        }
        self.changed(cx);
    }

    fn move_one(&mut self, id: u64, delta: isize, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let target = (index as isize + delta).clamp(0, self.items.len() as isize - 1) as usize;
        if target != index {
            self.items.swap(index, target);
            self.changed(cx);
        }
    }

    fn toggle_excluded(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.excluded = !item.excluded;
            if item.excluded {
                item.selected = false;
            }
        }
        self.changed(cx);
    }

    fn exclude_selected(&mut self, cx: &mut Context<Self>) {
        for item in &mut self.items {
            if item.selected {
                item.excluded = true;
            }
        }
        self.clear_selection(cx);
        self.changed(cx);
    }

    fn remove_item(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        self.items.remove(index);
        if self.active == Some(id) {
            self.active = self
                .items
                .get(index.min(self.items.len().saturating_sub(1)))
                .map(|item| item.id);
            self.preview = None;
            self.queue_preview(cx);
        }
        self.changed(cx);
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let active = self.active;
        let before = self
            .items
            .iter()
            .position(|item| Some(item.id) == active)
            .unwrap_or(0);
        self.items.retain(|item| !item.selected);
        if self.items.is_empty() {
            self.active = None;
        } else if !self.items.iter().any(|item| Some(item.id) == active) {
            self.active = Some(self.items[before.min(self.items.len() - 1)].id);
        }
        self.preview = None;
        self.selected_clear();
        self.changed(cx);
    }

    fn selected_clear(&mut self) {
        for item in &mut self.items {
            item.selected = false;
        }
    }

    fn keep_selected(&mut self, cx: &mut Context<Self>) {
        let active = self.active;
        self.items.retain(|item| item.selected);
        if !self.items.iter().any(|item| Some(item.id) == active) {
            self.active = self.items.first().map(|item| item.id);
        }
        self.selected_clear();
        self.preview = None;
        self.changed(cx);
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        self.items.clear();
        self.active = None;
        self.preview = None;
        self.preview_attempted = None;
        self.summary = None;
        self.view = StageView::Preview;
        self.confirm_overwrite = None;
        self.notice = None;
        cx.notify();
    }

    fn sort_by(&mut self, kind: ChoiceKind, index: usize, cx: &mut Context<Self>) {
        let _ = kind;
        let active = self.active;
        match index {
            0 => self
                .items
                .sort_by(|a, b| seepic_core::natsort::natural_cmp(&a.input.name, &b.input.name)),
            1 => self.items.sort_by(|a, b| b.input.size.cmp(&a.input.size)),
            2 => self.items.sort_by(|a, b| {
                extension(&a.input.name)
                    .cmp(&extension(&b.input.name))
                    .then_with(|| seepic_core::natsort::natural_cmp(&a.input.name, &b.input.name))
            }),
            _ => {}
        }
        self.active = active;
        self.changed(cx);
    }

    // ---- adding files -----------------------------------------------------

    fn add_files(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        let dialog = rfd::FileDialog::new()
            .set_title(tr("Add Images"))
            .add_filter(tr("Images"), IMAGE_EXTENSIONS);
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_files(),
            |this, cx, picked| {
                let Some(paths) = picked else { return };
                let mut added = 0;
                for path in paths {
                    if this.items.len() >= 5000 {
                        break;
                    }
                    if this.push_input(BatchInput::file(path, None)) {
                        added += 1;
                    }
                }
                this.finish_adding(added, cx);
            },
        );
    }

    fn add_folder(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        let dialog = rfd::FileDialog::new().set_title(tr("Add Folder"));
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_folder(),
            |this, cx, picked| {
                let Some(root) = picked else { return };
                let mut files = Vec::new();
                collect_folder(&root, &root, &mut files, 0);
                let mut added = 0;
                for input in files {
                    if this.items.len() >= 5000 {
                        break;
                    }
                    if this.push_input(input) {
                        added += 1;
                    }
                }
                if this.mirror_root.is_none() {
                    this.mirror_root = Some(root);
                }
                this.finish_adding(added, cx);
            },
        );
    }

    fn push_input(&mut self, input: BatchInput) -> bool {
        let key = input.source.key();
        if self.items.iter().any(|item| item.input.source.key() == key) {
            return false;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Item {
            id,
            alpha_hint: may_have_alpha(&input.name),
            input,
            selected: false,
            excluded: false,
            state: ItemState::Ready,
            output_path: None,
            output_size: 0,
            failure: None,
        });
        if self.active.is_none() {
            self.active = Some(id);
        }
        true
    }

    fn finish_adding(&mut self, added: usize, cx: &mut Context<Self>) {
        if added == 0 {
            self.notice = Some(tr("No new images were added").to_string());
        } else {
            self.notice = None;
            self.summary = None;
            self.view = StageView::Preview;
        }
        self.preview = None;
        self.queue_preview(cx);
        cx.notify();
    }

    // ---- dimensions and preview -------------------------------------------

    fn queue_preview(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.active_item() else {
            self.preview = None;
            self.preview_pending = false;
            return;
        };
        let id = item.id;
        let source = item.input.source.clone();
        let options = self.options.clone();
        self.preview_attempted = Some(id);
        self.preview_generation += 1;
        let generation = self.preview_generation;
        self.preview_pending = true;
        let weak = cx.entity().downgrade();

        cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(220))
                .await;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = source.read().map_err(|message| BatchError::Read(message))?;
                    batch::preview(&bytes, &options, 420, 320)
                })
                .await;
            weak.update(cx, |this, cx| {
                if this.preview_generation != generation {
                    return;
                }
                this.preview_pending = false;
                match result {
                    Ok(preview) => {
                        if let Some(item) = this.items.iter_mut().find(|item| item.id == id) {
                            if item.input.dimensions.is_none() {
                                item.input.dimensions =
                                    Some((preview.source_width, preview.source_height));
                            }
                            item.alpha_hint = preview.had_alpha;
                        }
                        if let (Some(before), Some(after)) =
                            (render_image(&preview.before), render_image(&preview.after))
                        {
                            this.preview = Some(PreviewState {
                                before,
                                after,
                                output_width: preview.output_width,
                                output_height: preview.output_height,
                                estimated_bytes: preview.estimated_bytes,
                            });
                        }
                    }
                    Err(error) => {
                        this.preview = None;
                        this.notice = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- settings ---------------------------------------------------------

    fn set_size_mode(&mut self, mode: SizeMode, cx: &mut Context<Self>) {
        let (box_w, box_h) = self.options.size.box_size().unwrap_or((1080, 1080));
        self.options.size = match mode {
            SizeMode::Original => SizeMode::Original,
            SizeMode::LongEdge(_) => SizeMode::LongEdge(
                self.options
                    .size
                    .long_edge()
                    .unwrap_or(1600)
                    .clamp(1, 30_000),
            ),
            SizeMode::Fit { .. } => SizeMode::Fit {
                width: box_w,
                height: box_h,
            },
            SizeMode::Crop { .. } => SizeMode::Crop {
                width: box_w,
                height: box_h,
                anchor: Anchor::Center,
            },
        };
        self.changed(cx);
    }

    fn nudge(&mut self, target: EditTarget, step: i32, cx: &mut Context<Self>) {
        match target {
            EditTarget::LongEdge => {
                if let SizeMode::LongEdge(value) = self.options.size {
                    self.options.size =
                        SizeMode::LongEdge((value as i32 + step).clamp(1, 30_000) as u32);
                }
            }
            EditTarget::BoxWidth | EditTarget::BoxHeight => {
                if let Some((mut width, mut height)) = self.options.size.box_size() {
                    if target == EditTarget::BoxWidth {
                        width = (width as i32 + step).clamp(1, 30_000) as u32;
                    } else {
                        height = (height as i32 + step).clamp(1, 30_000) as u32;
                    }
                    self.options.size = self.options.size.with_box(width, height);
                }
            }
            EditTarget::Quality => {
                self.options.quality = (self.options.quality as i32 + step).clamp(1, 100) as u8;
            }
            EditTarget::Jobs => {
                let max = batch::max_jobs() as i32;
                self.options.jobs = (self.options.jobs as i32 + step).clamp(1, max) as usize;
            }
            EditTarget::Template | EditTarget::Subfolder => {}
        }
        self.changed(cx);
    }

    fn begin_edit(&mut self, target: EditTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_buffer = match target {
            EditTarget::Template => self.options.template.clone(),
            EditTarget::Subfolder => self.options.subfolder.clone(),
            EditTarget::LongEdge => self.options.size.long_edge().unwrap_or(1600).to_string(),
            EditTarget::BoxWidth => self
                .options
                .size
                .box_size()
                .unwrap_or((1080, 1080))
                .0
                .to_string(),
            EditTarget::BoxHeight => self
                .options
                .size
                .box_size()
                .unwrap_or((1080, 1080))
                .1
                .to_string(),
            EditTarget::Quality => self.options.quality.to_string(),
            EditTarget::Jobs => self.options.jobs.to_string(),
        };
        self.editing = Some(target);
        self.edit_fresh = !matches!(target, EditTarget::Template | EditTarget::Subfolder);
        let focus = self.focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    fn apply_edit(&mut self) {
        let Some(target) = self.editing else {
            return;
        };
        let buffer = self.edit_buffer.clone();
        match target {
            EditTarget::Template => self.options.template = buffer,
            EditTarget::Subfolder => {
                self.options.subfolder = buffer
                    .chars()
                    .filter(|character| {
                        !character.is_control() && !"\\/:*?\"<>|".contains(*character)
                    })
                    .collect()
            }
            EditTarget::LongEdge => {
                if let Ok(value) = buffer.parse::<u32>() {
                    self.options.size = SizeMode::LongEdge(value.clamp(1, 30_000));
                }
            }
            EditTarget::BoxWidth | EditTarget::BoxHeight => {
                if let Ok(value) = buffer.parse::<u32>() {
                    if let Some((width, height)) = self.options.size.box_size() {
                        let value = value.clamp(1, 30_000);
                        self.options.size = if target == EditTarget::BoxWidth {
                            self.options.size.with_box(value, height)
                        } else {
                            self.options.size.with_box(width, value)
                        };
                    }
                }
            }
            EditTarget::Quality => {
                if let Ok(value) = buffer.parse::<u8>() {
                    self.options.quality = value.clamp(1, 100);
                }
            }
            EditTarget::Jobs => {
                if let Ok(value) = buffer.parse::<usize>() {
                    self.options.jobs = value.clamp(1, batch::max_jobs());
                }
            }
        }
    }

    fn pick_output_folder(&mut self, cx: &mut Context<Self>) {
        let dialog = rfd::FileDialog::new().set_title(tr("Choose Output Folder"));
        Self::dialog_off_stack(
            cx,
            move || dialog.pick_folder(),
            |this, cx, picked| {
                if let Some(folder) = picked {
                    this.options.output_dir = Some(folder);
                    this.options.placement = OutputPlacement::Custom;
                    this.changed(cx);
                }
            },
        );
    }

    fn apply_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        let presets = presets();
        let Some(preset) = presets.get(index) else {
            return;
        };
        let output_dir = self.options.output_dir.clone();
        let placement = self.options.placement;
        let subfolder = self.options.subfolder.clone();
        self.options = preset.options.clone();
        // Presets never carry a folder or a file list.
        self.options.output_dir = output_dir;
        self.options.placement = placement;
        self.options.subfolder = subfolder;
        self.changed(cx);
    }

    fn apply_choice(&mut self, kind: ChoiceKind, index: usize, cx: &mut Context<Self>) {
        self.choice = None;
        match kind {
            ChoiceKind::Preset => self.apply_preset(index, cx),
            ChoiceKind::Sort => self.sort_by(kind, index, cx),
            ChoiceKind::Placement => {
                self.options.placement = OutputPlacement::ALL[index];
                self.changed(cx);
            }
            ChoiceKind::Conflict => {
                self.options.conflict = ConflictPolicy::ALL[index];
                self.changed(cx);
            }
            ChoiceKind::Format => {
                self.options.format = BatchFormat::ALL[index];
                self.changed(cx);
            }
        }
    }

    // ---- execution --------------------------------------------------------

    fn start(&mut self, cx: &mut Context<Self>) {
        self.start_with(None, false, cx);
    }

    fn start_with(&mut self, only: Option<HashSet<u64>>, confirmed: bool, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        if self.options.placement == OutputPlacement::Custom && self.options.output_dir.is_none() {
            self.notice = Some(tr("Choose an output folder first").to_string());
            cx.notify();
            return;
        }
        let runnable = self
            .items
            .iter()
            .filter(|item| !item.excluded)
            .filter(|item| only.as_ref().is_none_or(|ids| ids.contains(&item.id)))
            .count();
        if runnable == 0 {
            self.notice = Some(tr("No image to convert").to_string());
            cx.notify();
            return;
        }

        if !confirmed && self.options.conflict == ConflictPolicy::Overwrite {
            let conflicts = self.prescan_conflicts(&only);
            let unknown_size = self
                .items
                .iter()
                .filter(|item| !item.excluded)
                .filter(|item| only.as_ref().is_none_or(|ids| ids.contains(&item.id)))
                .any(|item| item.input.dimensions.is_none());
            if !conflicts.is_empty() || unknown_size {
                self.confirm_overwrite = Some(conflicts);
                self.confirm_only = only;
                cx.notify();
                return;
            }
        }
        self.confirm_overwrite = None;
        self.confirm_only = None;

        let jobs: Vec<Job> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.excluded)
            .filter(|(_, item)| only.as_ref().is_none_or(|ids| ids.contains(&item.id)))
            .map(|(index, item)| Job {
                id: item.id,
                name: item.input.name.clone(),
                source: item.input.source.clone(),
                source_path: item.input.source.path(),
                relative: item.input.relative.clone(),
                modified: item.input.modified,
                sequence: index + 1,
            })
            .collect();

        for item in &mut self.items {
            if item.excluded {
                item.state = ItemState::Skipped;
                item.output_path = None;
                item.output_size = 0;
                item.failure = Some(Failure {
                    code: FailureCode::E99,
                    message: tr("Excluded").to_string(),
                });
            } else if only.as_ref().is_none_or(|ids| ids.contains(&item.id)) {
                item.state = ItemState::Ready;
                item.failure = None;
                item.output_path = None;
                item.output_size = 0;
            }
        }

        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let options = self.options.clone();
        let mirror_root = self.mirror_root.clone();
        let worker_cancel = cancel.clone();
        let worker_paused = paused.clone();

        std::thread::spawn(move || {
            run_batch(
                jobs,
                options,
                mirror_root,
                worker_cancel,
                worker_paused,
                sender,
            );
        });

        self.run = Some(Run {
            cancel,
            paused,
            total: runnable,
            processed: 0,
            ok: 0,
            failed: 0,
            skipped: 0,
            started: Instant::now(),
            elapsed: None,
            cancelled: false,
            receiver,
        });
        self.summary = None;
        self.view = StageView::Preview;
        self.notice = None;
        self.schedule_poll(cx);
        cx.notify();
    }

    fn prescan_conflicts(&self, only: &Option<HashSet<u64>>) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            if item.excluded || only.as_ref().is_some_and(|ids| !ids.contains(&item.id)) {
                continue;
            }
            let Some((width, height)) = self.target_dims(item) else {
                continue;
            };
            if let Ok(path) = batch::planned_output_path(
                &item.input.source.path(),
                item.input.relative.as_deref(),
                &self.options,
                self.mirror_root.as_deref(),
                &item.input.name,
                width,
                height,
                index + 1,
            ) {
                if path.exists() {
                    paths.push(path);
                }
            }
        }
        paths
    }

    fn schedule_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                let running = this
                    .update(cx, |this, cx| {
                        this.poll_run(cx);
                        this.running()
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll_run(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let mut messages = Vec::new();
        loop {
            match run.receiver.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    messages.push(RunMessage::Done { cancelled: false });
                    break;
                }
            }
        }
        if messages.is_empty() {
            return;
        }

        let mut finished = false;
        for message in messages {
            match message {
                RunMessage::Started { id } => {
                    if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                        item.state = ItemState::Running;
                    }
                }
                RunMessage::Item { id, outcome } => {
                    run.processed += 1;
                    match &outcome {
                        ItemOutcome::Done { .. } => run.ok += 1,
                        ItemOutcome::Skipped { .. } => run.skipped += 1,
                        ItemOutcome::Failed { .. } => run.failed += 1,
                    }
                    if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                        match outcome {
                            ItemOutcome::Done { path, bytes } => {
                                item.state = ItemState::Done;
                                item.output_path = Some(path);
                                item.output_size = bytes;
                                item.failure = None;
                            }
                            ItemOutcome::Skipped { reason } => {
                                item.state = ItemState::Skipped;
                                item.failure = Some(Failure {
                                    code: FailureCode::E99,
                                    message: reason,
                                });
                            }
                            ItemOutcome::Failed { code, message } => {
                                item.state = ItemState::Failed;
                                item.failure = Some(Failure { code, message });
                            }
                        }
                    }
                }
                RunMessage::Done { cancelled } => {
                    finished = true;
                    run.cancelled = cancelled;
                    run.elapsed = Some(run.started.elapsed());
                    if cancelled {
                        for item in &mut self.items {
                            if item.state == ItemState::Ready && !item.excluded {
                                item.state = ItemState::Skipped;
                                item.failure = Some(Failure {
                                    code: FailureCode::E99,
                                    message: tr("Cancelled").to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        if finished {
            self.finish_run(cx);
        }
        cx.notify();
    }

    fn finish_run(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.run.take() else {
            return;
        };
        let elapsed = run.elapsed.unwrap_or_else(|| run.started.elapsed());
        let total = self.items.len();
        let skipped = self
            .items
            .iter()
            .filter(|item| item.state == ItemState::Skipped)
            .count();
        let ok = self
            .items
            .iter()
            .filter(|item| item.state == ItemState::Done)
            .count();
        let failed = self
            .items
            .iter()
            .filter(|item| item.state == ItemState::Failed)
            .count();
        let input_bytes = self
            .items
            .iter()
            .filter(|item| !item.excluded)
            .map(|item| item.input.size)
            .sum();
        let output_bytes = self.items.iter().map(|item| item.output_size).sum();
        let failures = self
            .items
            .iter()
            .filter_map(|item| {
                item.failure
                    .as_ref()
                    .filter(|_| item.state == ItemState::Failed)
                    .map(|failure| (item.input.name.clone(), failure.clone()))
            })
            .collect();

        self.summary = Some(RunSummary {
            ok,
            failed,
            skipped,
            total,
            input_bytes,
            output_bytes,
            elapsed,
            cancelled: run.cancelled,
            failures,
        });
        self.view = StageView::Result;
        cx.notify();
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &self.run {
            let paused = !run.paused.load(Ordering::Relaxed);
            run.paused.store(paused, Ordering::Relaxed);
            cx.notify();
        }
    }

    fn cancel_run(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &self.run {
            run.cancel.store(true, Ordering::Relaxed);
            run.paused.store(false, Ordering::Relaxed);
            cx.notify();
        }
    }

    fn retry_failed(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        let ids: HashSet<u64> = self
            .items
            .iter()
            .filter(|item| item.state == ItemState::Failed)
            .map(|item| item.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.start_with(Some(ids), false, cx);
    }

    fn export_csv(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<(String, String, String, String, String)> = self
            .items
            .iter()
            .map(|item| {
                let status = status_text(item).to_string();
                let (code, message) = match &item.failure {
                    Some(failure) => (failure.code.code().to_string(), failure.message.clone()),
                    None => (String::new(), String::new()),
                };
                (
                    item.input.name.clone(),
                    status,
                    code,
                    message,
                    item.output_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                )
            })
            .collect();
        let csv = batch::report_csv(&rows);
        let dialog = rfd::FileDialog::new()
            .set_title(tr("Save Report"))
            .set_file_name("batch-convert-report.csv")
            .add_filter("CSV", &["csv"]);
        Self::dialog_off_stack(
            cx,
            move || dialog.save_file(),
            move |this, cx, picked| {
                let Some(path) = picked else { return };
                match batch::write_text(&path, &csv) {
                    Ok(()) => this.notice = Some(format!("{} {}", tr("Saved"), path.display())),
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }

    fn open_output(&mut self, cx: &mut Context<Self>) {
        let folder = if self.options.placement == OutputPlacement::Custom {
            self.options.output_dir.clone()
        } else {
            self.items
                .iter()
                .find_map(|item| item.output_path.as_ref().and_then(|path| path.parent()))
                .map(Path::to_path_buf)
                .or_else(|| {
                    self.active_item()
                        .and_then(|item| item.input.source.path().parent().map(Path::to_path_buf))
                })
        };
        match folder {
            Some(folder) if shell_open::open_folder(&folder) => {}
            Some(folder) => {
                self.notice = Some(format!("{} {}", tr("Could not open"), folder.display()))
            }
            None => self.notice = Some(tr("No output folder yet").to_string()),
        }
        cx.notify();
    }

    fn back_to_preview(&mut self, cx: &mut Context<Self>) {
        self.view = StageView::Preview;
        cx.notify();
    }

    // ---- keyboard ---------------------------------------------------------

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;

        if self.choice.is_some() {
            if key == "escape" {
                self.choice = None;
                cx.notify();
            }
            return;
        }

        if self.editing.is_some() {
            match key {
                "escape" | "enter" => {
                    self.editing = None;
                    self.edit_buffer.clear();
                }
                "backspace" => {
                    if self.edit_fresh {
                        self.edit_buffer.clear();
                        self.edit_fresh = false;
                    } else {
                        self.edit_buffer.pop();
                    }
                    self.apply_edit();
                    self.changed(cx);
                }
                "space" => {
                    self.push_edit_character(' ');
                    self.changed(cx);
                }
                _ if key.chars().count() == 1 && !modifiers.control && !modifiers.alt => {
                    if let Some(character) = key.chars().next() {
                        self.push_edit_character(character);
                        self.changed(cx);
                    }
                }
                _ => {}
            }
            cx.notify();
            return;
        }

        if key == "enter" && !self.running() && !self.items.is_empty() {
            self.start(cx);
            return;
        }
        if key == "escape" {
            if self.running() {
                self.notice = Some(tr("Converting; use Cancel to stop").to_string());
                cx.notify();
            } else {
                window.remove_window();
            }
        }
    }

    fn push_edit_character(&mut self, character: char) {
        let numeric = matches!(
            self.editing,
            Some(EditTarget::LongEdge)
                | Some(EditTarget::BoxWidth)
                | Some(EditTarget::BoxHeight)
                | Some(EditTarget::Quality)
                | Some(EditTarget::Jobs)
        );
        if numeric && !character.is_ascii_digit() {
            return;
        }
        if !numeric && character.is_control() {
            return;
        }
        if self.edit_fresh {
            self.edit_buffer.clear();
            self.edit_fresh = false;
        }
        if self.edit_buffer.chars().count() >= 200 {
            return;
        }
        self.edit_buffer.push(character);
        self.apply_edit();
    }

    // ---- dialogs ----------------------------------------------------------

    fn dialog_off_stack<T: 'static>(
        cx: &mut Context<Self>,
        show: impl FnOnce() -> T + 'static,
        then: impl FnOnce(&mut Self, &mut Context<Self>, T) + 'static,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1))
                .await;
            let picked = show();
            this.update(cx, |this, cx| then(this, cx, picked)).ok();
        })
        .detach();
    }
}

impl Drop for BatchWindow {
    fn drop(&mut self) {
        if let Some(run) = &self.run {
            run.cancel.store(true, Ordering::Relaxed);
            run.paused.store(false, Ordering::Relaxed);
        }
    }
}

impl Render for BatchWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.preview.is_none()
            && !self.preview_pending
            && self.preview_attempted != self.active
            && !self.items.is_empty()
        {
            self.queue_preview(cx);
        }

        let theme = cx.theme().clone();
        let title_width = (f32::from(window.bounds().size.width) - 160.).max(160.);
        let subtitle = if self.items.is_empty() {
            tr("Convert a batch of images locally").to_string()
        } else {
            format!(
                "{} {} · {}",
                self.items.len(),
                tr("images"),
                self.summary_line()
            )
        };

        div()
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }),
            )
            .child(
                v_flex()
                    .size_full()
                    .child(
                        TitleBar::new().child(
                            div()
                                .occlude()
                                .w(px(title_width))
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .gap_2()
                                .overflow_hidden()
                                .window_control_area(WindowControlArea::Drag)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|_this, _: &MouseDownEvent, window, _cx| {
                                        window.start_window_move();
                                    }),
                                )
                                .child(
                                    Icon::new(IconName::FileImage)
                                        .size(px(15.))
                                        .text_color(theme.primary),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(tr("Batch Image Convert")),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(subtitle),
                                ),
                        ),
                    )
                    .child(self.top_bar(cx))
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(notice_bar(notice, cx))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .gap_3()
                            .px_3()
                            .pb_2()
                            .child(
                                v_flex()
                                    .w(px(INPUT_WIDTH))
                                    .flex_shrink_0()
                                    .h_full()
                                    .child(self.input_card(cx)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .gap_3()
                                    .child(self.settings_card(cx))
                                    .child(self.stage_card(cx)),
                            ),
                    )
                    .child(self.action_bar(cx)),
            )
            .when_some(self.choice.as_ref(), |this, choice| {
                this.child(choice_popup(choice, window, cx))
            })
    }
}

// ---- layout ---------------------------------------------------------------

impl BatchWindow {
    fn top_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let step = self.step();
        let steps = [(tr("1 Add"), 0), (tr("2 Settings"), 1), (tr("3 Start"), 2)];
        let preset = self
            .preset()
            .map(|index| presets()[index].name)
            .unwrap_or_else(|| tr("Custom"));

        h_flex()
            .w_full()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .child(
                h_flex()
                    .gap_1()
                    .children(steps.into_iter().map(|(label, index)| {
                        let on = step == index;
                        div()
                            .px_2p5()
                            .py_0p5()
                            .rounded_full()
                            .border_1()
                            .border_color(if on { theme.primary } else { theme.border })
                            .bg(if on {
                                theme.primary.opacity(0.12)
                            } else {
                                theme.transparent
                            })
                            .text_xs()
                            .text_color(if on {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            })
                            .child(label)
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("batch-preset")
                    .px_2p5()
                    .py_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .text_xs()
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.button_hover))
                    .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                        let index = this.preset();
                        let open = this
                            .choice
                            .as_ref()
                            .is_some_and(|choice| choice.kind == ChoiceKind::Preset);
                        this.choice = if open {
                            None
                        } else {
                            Some(ChoicePopup {
                                kind: ChoiceKind::Preset,
                                anchor: event.position(),
                                labels: presets().iter().map(|preset| preset.name).collect(),
                                selected: index,
                            })
                        };
                        cx.notify();
                    }))
                    .child(format!("{}: {}", tr("Preset"), preset)),
            )
            .into_any_element()
    }

    fn input_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let count = self.items.len();
        let selected = self.selected_count();
        let runnable = self.runnable_count();
        let alpha = self.alpha_count();
        let excluded = self.excluded_count();
        let list_enabled = !self.running();
        let active = self.active;
        let row_sizes = Rc::new(vec![size(px(100.), px(ROW_HEIGHT)); count]);
        let entity = cx.entity();

        let mut card = v_flex()
            .size_full()
            .rounded(px(10.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .bg(theme.sidebar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(tr("Input")),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .rounded_full()
                                    .bg(theme.muted)
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(count.to_string()),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("batch-sort")
                            .px_2p5()
                            .py_1()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.border)
                            .text_xs()
                            .cursor_pointer()
                            .hover(|this| this.bg(theme.button_hover))
                            .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                                let open = this
                                    .choice
                                    .as_ref()
                                    .is_some_and(|choice| choice.kind == ChoiceKind::Sort);
                                this.choice = if open {
                                    None
                                } else {
                                    Some(ChoicePopup {
                                        kind: ChoiceKind::Sort,
                                        anchor: event.position(),
                                        labels: vec![tr("By name"), tr("By size"), tr("By format")],
                                        selected: None,
                                    })
                                };
                                cx.notify();
                            }))
                            .child(tr("Sort")),
                    )
                    .child(button(tr("Add files"), false, false, cx, |this, cx| {
                        this.add_files(cx)
                    }))
                    .child(button(tr("Add folder"), false, false, cx, |this, cx| {
                        this.add_folder(cx)
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .gap_1p5()
                    .flex_wrap()
                    .child(chip(format!("{runnable} {}", tr("pending")), false, &theme))
                    .child(chip(
                        format!("{} {}", tr("Total"), batch::human_size(self.total_bytes())),
                        false,
                        &theme,
                    ))
                    .when(alpha > 0, |this| {
                        this.child(chip(
                            format!("{alpha} {}", tr("with transparency")),
                            true,
                            &theme,
                        ))
                    })
                    .when(excluded > 0, |this| {
                        this.child(chip(format!("{excluded} {}", tr("excluded")), true, &theme))
                    }),
            );

        if count == 0 {
            card = card.child(empty_state(&theme, cx));
        } else {
            if selected > 0 {
                card = card.child(selection_bar(selected, list_enabled, &theme, cx));
            }
            card = card.child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(checkbox(
                        self.items.iter().all(|item| item.selected),
                        self.selected_count(),
                        count,
                        list_enabled,
                        &theme,
                        cx,
                        |this, cx| {
                            let all = this.items.iter().all(|item| item.selected);
                            this.select_all(!all, cx);
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(tr("Name")),
                    )
                    .child(
                        div()
                            .w(px(62.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(tr("Size")),
                    )
                    .child(
                        div()
                            .w(px(62.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(tr("Status")),
                    )
                    .child(div().w(px(92.))),
            );
            card = card.child(
                div().relative().w_full().flex_1().min_h_0().child(
                    v_virtual_list(
                        entity.clone(),
                        "batch-list",
                        row_sizes,
                        move |view, range, _window, cx| {
                            let mut rows = Vec::with_capacity(range.len());
                            for index in range {
                                if index >= view.items.len() {
                                    continue;
                                }
                                rows.push(queue_row(view, index, active, list_enabled, cx));
                            }
                            rows
                        },
                    )
                    .w_full()
                    .h_full(),
                ),
            );
            card = card.child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(button(tr("Clear"), false, !list_enabled, cx, |this, cx| {
                        this.clear(cx)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_right()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(if runnable == 0 {
                                tr("All files are excluded. Restore at least one to start.")
                            } else {
                                tr("Click a row to preview · ↑↓ reorder · ⊘ exclude · ✕ remove")
                            }),
                    ),
            );
        }
        card.into_any_element()
    }

    fn settings_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let more = if self.show_more {
            Some(self.more_rows(cx))
        } else {
            None
        };
        let alpha = if self.needs_alpha_notice() {
            Some(self.alpha_notice(cx))
        } else {
            None
        };
        let mut body = v_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .when(self.settings_collapsed, |this| this.hidden())
            .child(self.size_row(cx))
            .child(self.output_row(cx))
            .child(self.conflict_row(cx))
            .child(self.format_row(cx))
            .child(self.jobs_row(cx));
        if let Some(more) = more {
            body = body.child(more);
        }
        if let Some(alpha) = alpha {
            body = body.child(alpha);
        }
        let body = div()
            .relative()
            .w_full()
            .child(body)
            .when(self.running(), |this| {
                // The queue is locked during a run; the settings must match it, or
                // the values on screen stop describing the batch being written.
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .occlude(),
                )
            });
        let card = v_flex()
            .w_full()
            .flex_shrink_0()
            .rounded(px(10.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .bg(theme.sidebar)
                    .border_b_1()
                    .border_color(theme.border)
                    .when(self.settings_collapsed, |this| this.border_b_0())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("Settings")),
                    )
                    .when(self.settings_collapsed, |this| {
                        this.child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_full()
                                .bg(theme.muted)
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(self.summary_line()),
                        )
                    })
                    .child(div().flex_1())
                    .child(button(
                        if self.settings_collapsed {
                            tr("Expand")
                        } else {
                            tr("Collapse")
                        },
                        false,
                        false,
                        cx,
                        |this, cx| {
                            this.settings_collapsed = !this.settings_collapsed;
                            cx.notify();
                        },
                    )),
            )
            .child(body);
        card.into_any_element()
    }

    fn size_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let current = self.options.size;
        let mut control = h_flex().gap_1().items_center().flex_wrap().child(
            h_flex()
                .gap_0p5()
                .p_0p5()
                .rounded(px(7.))
                .border_1()
                .border_color(theme.border)
                .child(size_button(
                    tr("Original"),
                    matches!(current, SizeMode::Original),
                    SizeMode::Original,
                    &theme,
                    cx,
                ))
                .child(size_button(
                    tr("Long edge"),
                    matches!(current, SizeMode::LongEdge(_)),
                    SizeMode::LongEdge(1600),
                    &theme,
                    cx,
                ))
                .child(size_button(
                    tr("Fit"),
                    matches!(current, SizeMode::Fit { .. }),
                    SizeMode::Fit {
                        width: 1080,
                        height: 1080,
                    },
                    &theme,
                    cx,
                ))
                .child(size_button(
                    tr("Crop"),
                    matches!(current, SizeMode::Crop { .. }),
                    SizeMode::Crop {
                        width: 1080,
                        height: 1080,
                        anchor: Anchor::Center,
                    },
                    &theme,
                    cx,
                )),
        );

        if let SizeMode::LongEdge(edge) = current {
            control = control.child(number_editor(
                edge,
                EditTarget::LongEdge,
                self.editing,
                &self.edit_buffer,
                &theme,
                cx,
            ));
            control = control.child(hint(tr("px"), &theme));
        }
        if let Some((width, height)) = current.box_size() {
            control = control
                .child(number_editor(
                    width,
                    EditTarget::BoxWidth,
                    self.editing,
                    &self.edit_buffer,
                    &theme,
                    cx,
                ))
                .child(hint("×", &theme))
                .child(number_editor(
                    height,
                    EditTarget::BoxHeight,
                    self.editing,
                    &self.edit_buffer,
                    &theme,
                    cx,
                ))
                .child(hint(tr("px"), &theme));
        }

        control = control
            .child(check_row(
                tr("Only shrink"),
                self.options.only_shrink,
                &theme,
                cx,
                |this, cx| {
                    this.options.only_shrink = !this.options.only_shrink;
                    this.changed(cx);
                },
            ))
            .child(check_row(
                tr("Auto-rotate"),
                self.options.auto_orient,
                &theme,
                cx,
                |this, cx| {
                    this.options.auto_orient = !this.options.auto_orient;
                    this.changed(cx);
                },
            ));

        setting_row(tr("Size"), control, &theme)
    }

    fn output_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let placement = self.options.placement;
        let folder_label = match placement {
            OutputPlacement::Custom => self
                .options
                .output_dir
                .as_ref()
                .map(|folder| folder.display().to_string())
                .unwrap_or_else(|| tr("Choose a folder").to_string()),
            OutputPlacement::Subfolder => format!(
                "{} / {}",
                tr("source folder"),
                if self.options.subfolder.is_empty() {
                    "output"
                } else {
                    &self.options.subfolder
                }
            ),
            OutputPlacement::Mirror => tr("Mirror source folders").to_string(),
            OutputPlacement::SameFolder => tr("Beside the source").to_string(),
        };

        let example = self
            .active_item()
            .filter(|item| !item.excluded)
            .or_else(|| self.items.iter().find(|item| !item.excluded))
            .or_else(|| self.items.first())
            .map(|item| {
                let index = self
                    .items
                    .iter()
                    .position(|candidate| candidate.id == item.id)
                    .unwrap_or(0);
                self.planned_name(item, index)
            })
            .unwrap_or_else(|| "—".to_string());

        let mut controls = h_flex().gap_1p5().items_center().flex_wrap().child(
            div()
                .id("batch-placement")
                .max_w(px(190.))
                .px_2p5()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .cursor_pointer()
                .truncate()
                .hover(|this| this.bg(theme.button_hover))
                .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                    let open = this
                        .choice
                        .as_ref()
                        .is_some_and(|choice| choice.kind == ChoiceKind::Placement);
                    this.choice = if open {
                        None
                    } else {
                        Some(ChoicePopup {
                            kind: ChoiceKind::Placement,
                            anchor: event.position(),
                            labels: OutputPlacement::ALL
                                .iter()
                                .map(|placement| tr(placement.label()))
                                .collect(),
                            selected: OutputPlacement::ALL
                                .iter()
                                .position(|value| *value == this.options.placement),
                        })
                    };
                    cx.notify();
                }))
                .child(folder_label),
        );
        if placement == OutputPlacement::Custom {
            controls = controls.child(button(tr("Choose…"), false, false, cx, |this, cx| {
                this.pick_output_folder(cx)
            }));
        }
        if placement == OutputPlacement::Subfolder {
            controls = controls
                .child(hint(tr("Folder"), &theme))
                .child(text_editor(
                    &self.options.subfolder,
                    EditTarget::Subfolder,
                    self.editing,
                    &self.edit_buffer,
                    &theme,
                    cx,
                ));
        }
        controls = controls
            .child(hint(tr("Name"), &theme))
            .child(text_editor(
                &self.options.template,
                EditTarget::Template,
                self.editing,
                &self.edit_buffer,
                &theme,
                cx,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{}: {example}", tr("Example"))),
            );

        setting_row(tr("Output"), controls, &theme)
    }

    fn conflict_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let conflict = self.options.conflict;
        let labels: Vec<&'static str> = ConflictPolicy::ALL
            .iter()
            .map(|policy| tr(policy.label()))
            .collect();
        let selected = ConflictPolicy::ALL
            .iter()
            .position(|value| *value == conflict);
        let labels_for_click = labels.clone();
        let hint_text = match conflict {
            ConflictPolicy::Rename => tr("Never loses a file"),
            ConflictPolicy::Skip => tr("Existing files count as skipped"),
            ConflictPolicy::Overwrite => tr("Asks before starting"),
            ConflictPolicy::Backup => tr("Old files move to _backup"),
        };
        let controls = h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .id("batch-conflict")
                    .max_w(px(190.))
                    .px_2p5()
                    .py_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if conflict == ConflictPolicy::Overwrite {
                        theme.danger
                    } else {
                        theme.border
                    })
                    .text_xs()
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.button_hover))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                        let open = this
                            .choice
                            .as_ref()
                            .is_some_and(|choice| choice.kind == ChoiceKind::Conflict);
                        this.choice = if open {
                            None
                        } else {
                            Some(ChoicePopup {
                                kind: ChoiceKind::Conflict,
                                anchor: event.position(),
                                labels: labels_for_click.clone(),
                                selected,
                            })
                        };
                        cx.notify();
                    }))
                    .child(tr(conflict.label())),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if conflict == ConflictPolicy::Overwrite {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(hint_text),
            );
        setting_row(tr("On conflict"), controls, &theme)
    }

    fn format_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let format = self.options.format;
        let mut controls = h_flex().gap_2().items_center().flex_wrap().child(
            div()
                .id("batch-format")
                .px_2p5()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .cursor_pointer()
                .hover(|this| this.bg(theme.button_hover))
                .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                    let open = this
                        .choice
                        .as_ref()
                        .is_some_and(|choice| choice.kind == ChoiceKind::Format);
                    this.choice = if open {
                        None
                    } else {
                        Some(ChoicePopup {
                            kind: ChoiceKind::Format,
                            anchor: event.position(),
                            labels: BatchFormat::ALL
                                .iter()
                                .map(|format| tr(format.label()))
                                .collect(),
                            selected: BatchFormat::ALL
                                .iter()
                                .position(|value| *value == this.options.format),
                        })
                    };
                    cx.notify();
                }))
                .child(tr(format.short_label())),
        );

        if format.uses_quality() {
            controls = controls.child(number_editor(
                self.options.quality as u32,
                EditTarget::Quality,
                self.editing,
                &self.edit_buffer,
                &theme,
                cx,
            ));
            controls = controls.child(hint(tr("quality"), &theme));
        } else {
            controls = controls.child(hint(
                if format == BatchFormat::WebP {
                    tr("This build writes WebP losslessly")
                } else {
                    tr("No quality setting for this format")
                },
                &theme,
            ));
        }

        controls = controls.child(button(
            if self.show_more {
                tr("Less")
            } else {
                tr("More")
            },
            false,
            false,
            cx,
            |this, cx| {
                this.show_more = !this.show_more;
                cx.notify();
            },
        ));

        setting_row(tr("Format"), controls, &theme)
    }

    fn jobs_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let max = batch::max_jobs();
        setting_row(
            tr("Concurrent conversions"),
            h_flex()
                .gap_2()
                .items_center()
                .child(number_editor(
                    self.options.jobs as u32,
                    EditTarget::Jobs,
                    self.editing,
                    &self.edit_buffer,
                    &theme,
                    cx,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} {} ({})",
                            tr("at most"),
                            max,
                            tr("logical processors")
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(tr("Lower it when the disk is slow")),
                ),
            &theme,
        )
    }

    fn more_rows(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .w_full()
            .gap_2()
            .pt_1()
            .border_t_1()
            .border_color(theme.border)
            .child(setting_row(
                tr("Resample"),
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(Resample::ALL.iter().map(|resample| {
                        let selected = self.options.resample == *resample;
                        let resample = *resample;
                        div()
                            .id(resample.label())
                            .px_2()
                            .py_0p5()
                            .rounded(px(6.))
                            .text_xs()
                            .cursor_pointer()
                            .border_1()
                            .border_color(if selected {
                                theme.primary
                            } else {
                                theme.border
                            })
                            .when(selected, |this| {
                                this.bg(theme.primary.opacity(0.12))
                                    .text_color(theme.primary)
                            })
                            .when(!selected, |this| this.text_color(theme.muted_foreground))
                            .child(tr(resample.label()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.options.resample = resample;
                                this.changed(cx);
                            }))
                    })),
                &theme,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .items_center()
                    .child(check_row(
                        tr("Keep EXIF / IPTC"),
                        self.options.keep_exif,
                        &theme,
                        cx,
                        |this, cx| {
                            this.options.keep_exif = !this.options.keep_exif;
                            this.options.keep_iptc = this.options.keep_exif;
                            this.changed(cx);
                        },
                    ))
                    .child(check_row(
                        tr("Keep colour profile"),
                        self.options.keep_color_profile,
                        &theme,
                        cx,
                        |this, cx| {
                            this.options.keep_color_profile = !this.options.keep_color_profile;
                            this.changed(cx);
                        },
                    ))
                    .child(check_row(
                        tr("Clear location"),
                        self.options.strip_location,
                        &theme,
                        cx,
                        |this, cx| {
                            this.options.strip_location = !this.options.strip_location;
                            this.changed(cx);
                        },
                    ))
                    .child(check_row(
                        tr("Keep timestamps"),
                        self.options.keep_timestamps,
                        &theme,
                        cx,
                        |this, cx| {
                            this.options.keep_timestamps = !this.options.keep_timestamps;
                            this.changed(cx);
                        },
                    )),
            )
            .child(setting_row(
                tr("Flatten background"),
                h_flex().gap_1().items_center().children(
                    [
                        ([255u8, 255, 255], tr("White")),
                        ([0u8, 0, 0], tr("Black")),
                        ([245u8, 245, 245], tr("Light grey")),
                        ([128u8, 128, 128], tr("Grey")),
                    ]
                    .into_iter()
                    .map(|(colour, label)| {
                        let selected = self.options.background == colour;
                        div()
                            .id(label)
                            .w(px(20.))
                            .h(px(20.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if selected {
                                theme.primary
                            } else {
                                theme.border
                            })
                            .cursor_pointer()
                            .bg(gpui::rgb(
                                (colour[0] as u32) << 16
                                    | (colour[1] as u32) << 8
                                    | colour[2] as u32,
                            ))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.options.background = colour;
                                this.changed(cx);
                            }))
                    }),
                ),
                &theme,
            ))
            .into_any_element()
    }

    fn needs_alpha_notice(&self) -> bool {
        self.alpha_count() > 0 && !self.options.format.supports_alpha()
    }

    fn alpha_notice(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        h_flex()
            .w_full()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(px(7.))
            .border_1()
            .border_color(theme.danger)
            .bg(theme.danger.opacity(0.08))
            .items_center()
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(px(14.))
                    .text_color(theme.danger),
            )
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(tr(
                        "The target format does not support transparency. Transparent areas are flattened onto the background colour; semi-transparent pixels blend with it.",
                    )),
            )
            .child(button(tr("Switch to PNG"), false, false, cx, |this, cx| {
                this.options.format = BatchFormat::Png;
                this.changed(cx);
            }))
            .into_any_element()
    }

    fn stage_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let title = match self.view {
            StageView::Preview => tr("Preview"),
            StageView::Result => tr("Result"),
        };
        let current = self
            .active_item()
            .map(|item| item.input.name.clone())
            .unwrap_or_else(|| "—".to_string());
        let result_ready = self.summary.is_some();

        let mut card = v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .rounded(px(10.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .bg(theme.sidebar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(current),
                    )
                    .child(tab_button(
                        tr("Preview"),
                        self.view == StageView::Preview,
                        true,
                        &theme,
                        cx,
                        |this, cx| this.back_to_preview(cx),
                    ))
                    .child(tab_button(
                        tr("Result"),
                        self.view == StageView::Result,
                        result_ready,
                        &theme,
                        cx,
                        |this, cx| {
                            this.view = StageView::Result;
                            cx.notify();
                        },
                    )),
            );

        card = card.child(match self.view {
            StageView::Preview => self.preview_body(cx),
            StageView::Result => self.result_body(cx),
        });
        card.into_any_element()
    }

    fn preview_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(item) = self.active_item() else {
            return empty_preview(&theme);
        };
        let name = self.planned_name(item, self.active_index().unwrap_or(0));
        let dims = self
            .preview
            .as_ref()
            .map(|preview| format!("{} × {}", preview.output_width, preview.output_height))
            .or_else(|| {
                self.target_dims(item)
                    .map(|(width, height)| format!("{width} × {height}"))
            })
            .unwrap_or_else(|| "—".to_string());
        let one = batch::human_size(
            self.preview
                .as_ref()
                .map(|preview| preview.estimated_bytes)
                .unwrap_or_else(|| self.estimated_one(item)),
        );
        let estimates_ready = self.estimates_ready();
        let total = if estimates_ready {
            batch::human_size(self.estimated_total())
        } else {
            "—".to_string()
        };
        let source_total = self.total_bytes();
        let estimated_total: u64 = self.estimated_total();
        let delta = if source_total == 0 || !estimates_ready {
            0
        } else {
            ((estimated_total as f64 - source_total as f64) / source_total as f64 * 100.).round()
                as i64
        };
        let delta_text = if estimates_ready {
            format!("{}{}%", if delta > 0 { "+" } else { "" }, delta)
        } else {
            "—".to_string()
        };
        let delta_color = if !estimates_ready {
            theme.muted_foreground
        } else if delta <= 0 {
            ok_color()
        } else {
            theme.danger
        };

        let (before, after) = match &self.preview {
            Some(preview) => (
                Some((
                    preview.before.width,
                    preview.before.height,
                    preview.before.render.clone(),
                )),
                Some((
                    preview.after.width,
                    preview.after.height,
                    preview.after.render.clone(),
                )),
            ),
            None => (None, None),
        };
        let before_meta = item
            .input
            .dimensions
            .map(|(width, height)| {
                format!("{width}×{height} · {}", batch::human_size(item.input.size))
            })
            .unwrap_or_else(|| batch::human_size(item.input.size));
        let after_meta = self
            .preview
            .as_ref()
            .map(|preview| format!("{}×{}", preview.output_width, preview.output_height))
            .or_else(|| {
                self.target_dims(item)
                    .map(|(width, height)| format!("{width}×{height}"))
            })
            .unwrap_or_else(|| "—".to_string());

        h_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(thumb_caption(tr("Before"), &before_meta, &theme))
                    .child(image_stage(before, &theme, self.preview_pending)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(thumb_caption(tr("After"), &after_meta, &theme))
                    .child(image_stage(after, &theme, self.preview_pending)),
            )
            .child(
                v_flex()
                    .w(px(198.))
                    .flex_shrink_0()
                    .justify_center()
                    .gap_1p5()
                    .child(kv_row(tr("Output size"), dims, &theme))
                    .child(kv_row(tr("File name"), name, &theme))
                    .child(kv_row(tr("One file"), format!("~{one}"), &theme))
                    .child(kv_row(tr("Total"), format!("~{total}"), &theme))
                    .child(kv_row_colored(
                        tr("vs source"),
                        delta_text,
                        delta_color,
                        &theme,
                    )),
            )
            .into_any_element()
    }

    fn result_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(summary) = self.summary.as_ref() else {
            return empty_preview(&theme);
        };
        let total = summary.total;
        let conserved = summary.ok + summary.failed + summary.skipped == total;
        let size_change = if summary.input_bytes == 0 {
            0
        } else {
            ((summary.output_bytes as f64 - summary.input_bytes as f64)
                / summary.input_bytes as f64
                * 100.)
                .round() as i64
        };
        let check = format!(
            "{} {} + {} {} + {} {} = {} {} {}",
            summary.ok,
            tr("success"),
            summary.failed,
            tr("failed"),
            summary.skipped,
            tr("skipped"),
            if conserved { "=" } else { "≠" },
            total,
            tr("input"),
        );

        let mut body = v_flex().w_full().flex_1().min_h_0().p_3().gap_2();

        if summary.cancelled {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(tr("The batch was cancelled after the current file.")),
            );
        }

        body = body
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(stat_box(
                        summary.ok.to_string(),
                        tr("Success"),
                        ok_color(),
                        &theme,
                    ))
                    .child(stat_box(
                        summary.failed.to_string(),
                        tr("Failed"),
                        theme.danger,
                        &theme,
                    ))
                    .child(stat_box(
                        summary.skipped.to_string(),
                        tr("Skipped"),
                        theme.muted_foreground,
                        &theme,
                    ))
                    .child(stat_box(
                        format!("{}{}%", if size_change > 0 { "+" } else { "" }, size_change),
                        tr("Volume"),
                        if size_change <= 0 {
                            ok_color()
                        } else {
                            theme.danger
                        },
                        &theme,
                    ))
                    .child(stat_box(
                        format!("{:.1}s", summary.elapsed.as_secs_f32()),
                        tr("Time"),
                        theme.foreground,
                        &theme,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .rounded(px(7.))
                    .bg(if conserved {
                        ok_color().opacity(0.10)
                    } else {
                        theme.danger.opacity(0.10)
                    })
                    .text_xs()
                    .text_color(if conserved { ok_color() } else { theme.danger })
                    .child(check),
            );

        if !summary.failures.is_empty() {
            let mut list = v_flex()
                .id("batch-failures")
                .w_full()
                .gap_1()
                .max_h(px(180.))
                .overflow_y_scroll();
            for (name, failure) in &summary.failures {
                list = list.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .bg(theme.danger.opacity(0.08))
                        .text_xs()
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.danger)
                                .child(failure.code.code()),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        .child(
                            div()
                                .flex_shrink_0()
                                .max_w(px(340.))
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(failure.message.clone()),
                        ),
                );
            }
            body = body.child(list);
        }

        body = body.child(
            h_flex()
                .w_full()
                .gap_2()
                .flex_wrap()
                .child(button(tr("Export CSV"), false, false, cx, |this, cx| {
                    this.export_csv(cx)
                }))
                .child(button(
                    tr("Retry failed"),
                    false,
                    summary.failed > 0 && !self.running(),
                    cx,
                    |this, cx| this.retry_failed(cx),
                ))
                .child(button(
                    tr("Open output folder"),
                    false,
                    false,
                    cx,
                    |this, cx| this.open_output(cx),
                ))
                .child(button(
                    tr("Back to preview"),
                    false,
                    false,
                    cx,
                    |this, cx| this.back_to_preview(cx),
                )),
        );

        body.into_any_element()
    }

    fn action_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(conflicts) = self.confirm_overwrite.clone() {
            return overwrite_strip(conflicts, &cx.theme().clone(), cx);
        }
        let theme = cx.theme().clone();
        let running = self.running();
        let paused = self
            .run
            .as_ref()
            .map(|run| run.paused.load(Ordering::Relaxed))
            .unwrap_or(false);
        let fraction = if let Some(run) = &self.run {
            run.fraction()
        } else if let Some(summary) = &self.summary {
            if summary.cancelled { 0. } else { 1. }
        } else {
            0.
        };
        let (done, ok, failed, skipped) = if let Some(run) = &self.run {
            (run.processed, run.ok, run.failed, run.skipped)
        } else if let Some(summary) = &self.summary {
            (
                summary.ok + summary.failed + summary.skipped,
                summary.ok,
                summary.failed,
                summary.skipped,
            )
        } else {
            (0, 0, 0, 0)
        };
        let total = self.run.as_ref().map(|run| run.total).unwrap_or_else(|| {
            self.summary
                .as_ref()
                .map_or_else(|| self.runnable_count(), |summary| summary.total)
        });
        let label = if running {
            if paused {
                tr("Paused")
            } else {
                tr("Processing")
            }
        } else if let Some(summary) = &self.summary {
            if summary.cancelled {
                tr("Cancelled")
            } else {
                tr("Completed")
            }
        } else {
            tr("Not started")
        };
        let label = if running {
            format!("{label} {done}/{total}")
        } else {
            label.to_string()
        };
        let start_label = if running {
            tr("Converting…")
        } else if self.summary.is_some() {
            tr("Convert again")
        } else {
            tr("Start conversion")
        };
        let can_start = !running && self.runnable_count() > 0;

        h_flex()
            .w_full()
            .flex_shrink_0()
            .min_h(px(54.))
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .bg(theme.popover)
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .w(px(130.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(6.))
                    .rounded_full()
                    .bg(theme.input)
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(relative(fraction))
                            .rounded_full()
                            .bg(theme.primary),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} {ok}", tr("Success")))
                    .child(format!("{} {failed}", tr("Failed")))
                    .child(format!("{} {skipped}", tr("Skipped"))),
            )
            .child(button(
                if paused { tr("Resume") } else { tr("Pause") },
                false,
                !running,
                cx,
                |this, cx| this.toggle_pause(cx),
            ))
            .child(button(tr("Cancel"), false, !running, cx, |this, cx| {
                this.cancel_run(cx)
            }))
            .child(
                div()
                    .id("batch-start")
                    .px_3()
                    .py_1()
                    .rounded(px(6.))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .bg(if can_start {
                        gpui::hsla(0.61, 0.78, 0.54, 1.)
                    } else {
                        theme.muted
                    })
                    .text_color(if can_start {
                        gpui::hsla(0., 0., 1., 1.)
                    } else {
                        theme.muted_foreground
                    })
                    .when(can_start, |this| {
                        this.cursor_pointer()
                            .hover(|this| this.opacity(0.9))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _window, cx| this.start(cx)),
                            )
                    })
                    .child(start_label),
            )
            .into_any_element()
    }
}

// ---- queue rows -----------------------------------------------------------

fn queue_row(
    view: &mut BatchWindow,
    index: usize,
    active: Option<u64>,
    enabled: bool,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let item = &view.items[index];
    let id = item.id;
    let is_active = active == Some(id);
    let selected = item.selected;
    let excluded = item.excluded;
    let name = item.input.name.clone();
    let mut meta_parts = Vec::new();
    if let Some(relative) = &item.input.relative {
        if let Some(parent) = relative.parent() {
            if !parent.as_os_str().is_empty() {
                meta_parts.push(parent.display().to_string());
            }
        }
    }
    meta_parts.push(extension(&name).to_uppercase());
    if item.alpha_hint {
        meta_parts.push(tr("transparent").to_string());
    }
    let meta = meta_parts.join(" · ");
    let status = status_text(item);
    let status_color = state_color(item.state, excluded, &theme);
    let size = batch::human_size(item.input.size);

    h_flex()
        .id(("batch-row", id))
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_2()
        .gap_2()
        .items_center()
        .when(selected, |this| this.bg(theme.primary.opacity(0.10)))
        .when(is_active, |this| this.bg(theme.list_active))
        .when(excluded, |this| this.opacity(0.55))
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            if enabled {
                this.set_active(id, cx);
            }
        }))
        .child(div().w(px(2.)).h_full().rounded_full().bg(if is_active {
            theme.primary
        } else {
            theme.transparent
        }))
        .child(checkbox_single(
            id,
            selected,
            enabled,
            &theme,
            cx,
            move |this, cx| this.toggle_selected(id, cx),
        ))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_xs()
                        .when(excluded, |this| this.line_through())
                        .child(name),
                )
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(meta),
                ),
        )
        .child(
            div()
                .w(px(62.))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(size),
        )
        .child(
            h_flex()
                .w(px(62.))
                .gap_1()
                .items_center()
                .child(div().size(px(6.)).rounded_full().bg(status_color))
                .child(div().text_xs().text_color(status_color).child(status)),
        )
        .child(
            h_flex()
                .w(px(92.))
                .gap_0p5()
                .justify_end()
                .when(!enabled, |this| this.opacity(0.3))
                .child(row_action(
                    id,
                    "↑",
                    enabled,
                    &theme,
                    cx,
                    move |this, cx| this.move_one(id, -1, cx),
                ))
                .child(row_action(
                    id,
                    "↓",
                    enabled,
                    &theme,
                    cx,
                    move |this, cx| this.move_one(id, 1, cx),
                ))
                .child(row_action(
                    id,
                    "⊘",
                    enabled,
                    &theme,
                    cx,
                    move |this, cx| this.toggle_excluded(id, cx),
                ))
                .child(row_action_danger(
                    id,
                    "✕",
                    enabled,
                    &theme,
                    cx,
                    move |this, cx| this.remove_item(id, cx),
                )),
        )
        .into_any_element()
}

// ---- small widgets --------------------------------------------------------

#[track_caller]
fn button(
    label: &'static str,
    primary: bool,
    disabled: bool,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    let theme = cx.theme().clone();
    div()
        .id(std::panic::Location::caller())
        .px_2p5()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(if primary { theme.primary } else { theme.border })
        .when(primary, |this| {
            this.bg(theme.primary).text_color(theme.primary_foreground)
        })
        .when(!primary, |this| this.text_color(theme.foreground))
        .text_xs()
        .opacity(if disabled { 0.45 } else { 1. })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.opacity(0.88))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
        })
        .child(label)
        .into_any_element()
}

fn tab_button(
    label: &'static str,
    selected: bool,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    div()
        .id(label)
        .px_2()
        .py_0p5()
        .rounded(px(5.))
        .text_xs()
        .when(selected, |this| {
            this.bg(theme.primary.opacity(0.14))
                .text_color(theme.primary)
        })
        .when(!selected, |this| this.text_color(theme.muted_foreground))
        .opacity(if enabled { 1. } else { 0.4 })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.list_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
        })
        .child(label)
        .into_any_element()
}

fn chip(label: String, warning: bool, theme: &Theme) -> AnyElement {
    let color = if warning {
        theme.danger
    } else {
        theme.muted_foreground
    };
    h_flex()
        .px_2()
        .py_0p5()
        .gap_1()
        .rounded_full()
        .border_1()
        .border_color(if warning {
            theme.danger.opacity(0.45)
        } else {
            theme.border
        })
        .bg(if warning {
            theme.danger.opacity(0.08)
        } else {
            theme.transparent
        })
        .text_xs()
        .text_color(color)
        .child(label)
        .into_any_element()
}

fn hint(text: &'static str, theme: &Theme) -> AnyElement {
    div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(text)
        .into_any_element()
}

fn empty_state(theme: &Theme, cx: &mut Context<BatchWindow>) -> AnyElement {
    let presets = presets();
    v_flex()
        .w_full()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .p_4()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(tr("Put the images to convert here first")),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(tr("Add files, choose a folder, or start from a preset")),
        )
        .child(v_flex().w_full().max_w(px(280.)).gap_1p5().children(
            presets.into_iter().enumerate().map(|(index, preset)| {
                let theme = theme.clone();
                h_flex()
                    .id(("batch-empty-preset", index))
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .cursor_pointer()
                    .justify_between()
                    .hover(|this| this.border_color(theme.primary))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.apply_preset(index, cx)
                    }))
                    .child(div().text_xs().child(preset.name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(preset.hint),
                    )
            }),
        ))
        .into_any_element()
}

fn empty_preview(theme: &Theme) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(Icon::new(IconName::Image).size(px(24.)))
        .child(tr("Choose a row to preview"))
        .into_any_element()
}

fn selection_bar(
    selected: usize,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    h_flex()
        .w_full()
        .flex_shrink_0()
        .px_3()
        .py_1()
        .gap_1()
        .items_center()
        .bg(theme.primary.opacity(0.08))
        .border_b_1()
        .border_color(theme.primary.opacity(0.35))
        .opacity(if enabled { 1. } else { 0.5 })
        .child(div().text_xs().text_color(theme.primary).child(format!(
            "{} {selected} {}",
            tr("Selected"),
            tr("items")
        )))
        .child(div().flex_1())
        .child(button(tr("Move up"), false, !enabled, cx, |this, cx| {
            this.move_selected(-1, cx)
        }))
        .child(button(tr("Move down"), false, !enabled, cx, |this, cx| {
            this.move_selected(1, cx)
        }))
        .child(button(tr("Exclude"), false, !enabled, cx, |this, cx| {
            this.exclude_selected(cx)
        }))
        .child(button(tr("Remove"), false, !enabled, cx, |this, cx| {
            this.remove_selected(cx)
        }))
        .child(button(tr("Keep only"), false, !enabled, cx, |this, cx| {
            this.keep_selected(cx)
        }))
        .child(button(tr("Deselect"), false, !enabled, cx, |this, cx| {
            this.clear_selection(cx)
        }))
        .into_any_element()
}

fn checkbox(
    all: bool,
    selected: usize,
    total: usize,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    let checked = all && total > 0;
    let indeterminate = selected > 0 && !all;
    div()
        .id("batch-check-all")
        .size(px(14.))
        .rounded(px(3.))
        .border_1()
        .border_color(if checked || indeterminate {
            theme.primary
        } else {
            theme.border
        })
        .when(checked || indeterminate, |this| this.bg(theme.primary))
        .flex()
        .items_center()
        .justify_center()
        .opacity(if enabled { 1. } else { 0.4 })
        .when(enabled, |this| {
            this.cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
        })
        .when(checked, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .size(px(10.))
                    .text_color(theme.primary_foreground),
            )
        })
        .when(indeterminate, |this| {
            this.child(
                div()
                    .w(px(7.))
                    .h(px(1.5))
                    .rounded_full()
                    .bg(theme.primary_foreground),
            )
        })
        .into_any_element()
}

fn checkbox_single(
    item_id: u64,
    checked: bool,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    div()
        .id(("batch-check-one", item_id))
        .size(px(14.))
        .rounded(px(3.))
        .border_1()
        .border_color(if checked { theme.primary } else { theme.border })
        .when(checked, |this| this.bg(theme.primary))
        .flex()
        .items_center()
        .justify_center()
        .opacity(if enabled { 1. } else { 0.4 })
        .when(enabled, |this| {
            this.cursor_pointer().on_click(cx.listener(
                move |this, _event: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    on_click(this, cx)
                },
            ))
        })
        .when(checked, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .size(px(10.))
                    .text_color(theme.primary_foreground),
            )
        })
        .into_any_element()
}

fn row_action(
    item_id: u64,
    label: &'static str,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    div()
        .id((label, item_id))
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_xs()
        .text_color(theme.muted_foreground)
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.button_hover).text_color(theme.foreground))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    on_click(this, cx)
                }))
        })
        .child(label)
        .into_any_element()
}

fn row_action_danger(
    item_id: u64,
    label: &'static str,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    div()
        .id((label, item_id))
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_xs()
        .text_color(theme.muted_foreground)
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.danger.opacity(0.12)).text_color(theme.danger))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    on_click(this, cx)
                }))
        })
        .child(label)
        .into_any_element()
}

fn setting_row(label: &'static str, control: impl IntoElement, theme: &Theme) -> AnyElement {
    h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .child(
            div()
                .w(px(86.))
                .flex_shrink_0()
                .pt_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control))
        .into_any_element()
}

fn size_button(
    label: &'static str,
    active: bool,
    mode: SizeMode,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    let theme = theme.clone();
    div()
        .id(std::panic::Location::caller())
        .px_2p5()
        .py_1()
        .rounded(px(5.))
        .text_xs()
        .cursor_pointer()
        .when(active, |this| {
            this.bg(theme.primary.opacity(0.16))
                .text_color(theme.primary)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!active, |this| {
            this.text_color(theme.muted_foreground)
                .hover(|this| this.bg(theme.list_hover))
        })
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _window, cx| this.set_size_mode(mode, cx)),
        )
        .child(label)
        .into_any_element()
}

fn check_row(
    label: &'static str,
    checked: bool,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
    on_click: impl Fn(&mut BatchWindow, &mut Context<BatchWindow>) + 'static,
) -> AnyElement {
    h_flex()
        .id(label)
        .gap_1p5()
        .items_center()
        .cursor_pointer()
        .text_xs()
        .text_color(theme.muted_foreground)
        .hover(|this| this.text_color(theme.foreground))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| on_click(this, cx)))
        .child(
            div()
                .size(px(13.))
                .rounded(px(3.))
                .border_1()
                .border_color(if checked { theme.primary } else { theme.border })
                .when(checked, |this| this.bg(theme.primary))
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .size(px(9.))
                            .text_color(theme.primary_foreground),
                    )
                }),
        )
        .child(label)
        .into_any_element()
}

fn text_editor(
    value: &str,
    target: EditTarget,
    editing: Option<EditTarget>,
    buffer: &str,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    let active = editing == Some(target);
    let shown = if active {
        format!("{buffer}|")
    } else {
        value.to_string()
    };
    div()
        .id(("batch-text-editor", target as usize))
        .min_w(px(170.))
        .max_w(px(280.))
        .px_2()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(if active { theme.primary } else { theme.border })
        .text_xs()
        .cursor_pointer()
        .child(if shown.is_empty() {
            tr("(empty)").to_string()
        } else {
            shown
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                let focus = this.focus.clone();
                window.focus(&focus, cx);
                this.begin_edit(target, window, cx);
                cx.stop_propagation();
            }),
        )
        .into_any_element()
}

fn number_editor(
    value: u32,
    target: EditTarget,
    editing: Option<EditTarget>,
    buffer: &str,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    let active = editing == Some(target);
    let shown = if active {
        format!("{buffer}|")
    } else {
        value.to_string()
    };
    h_flex()
        .gap_0p5()
        .items_center()
        .child(
            div()
                .id(("batch-number", target as usize))
                .min_w(px(58.))
                .px_2()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(if active { theme.primary } else { theme.border })
                .text_xs()
                .cursor_pointer()
                .child(shown)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        let focus = this.focus.clone();
                        window.focus(&focus, cx);
                        this.begin_edit(target, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(stepper_buttons(target, theme, cx))
        .into_any_element()
}

fn stepper_buttons(target: EditTarget, theme: &Theme, cx: &mut Context<BatchWindow>) -> AnyElement {
    h_flex()
        .gap_0p5()
        .items_center()
        .child(step_button("-", target, -1, theme, cx))
        .child(step_button("+", target, 1, theme, cx))
        .into_any_element()
}

fn step_button(
    label: &'static str,
    target: EditTarget,
    step: i32,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    div()
        .id((label, target as usize))
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(theme.border)
        .text_xs()
        .cursor_pointer()
        .hover(|this| this.bg(theme.button_hover))
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _window, cx| this.nudge(target, step, cx)),
        )
        .child(label)
        .into_any_element()
}

fn thumb_caption(label: &'static str, meta: &str, theme: &Theme) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .text_xs()
        .child(div().text_color(theme.muted_foreground).child(label))
        .child(
            div()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(meta.to_string()),
        )
        .into_any_element()
}

fn image_stage(
    image: Option<(u32, u32, Arc<RenderImage>)>,
    theme: &Theme,
    pending: bool,
) -> AnyElement {
    let checker = theme.muted;
    let has_image = image.is_some();
    div()
        .w_full()
        .flex_1()
        .min_h(px(110.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(checker)
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .when_some(image, |this, (width, height, render)| {
            this.child(
                img(ImageSource::Render(render))
                    .w(px(width as f32))
                    .h(px(height as f32))
                    .object_fit(ObjectFit::Contain),
            )
        })
        .when(!has_image, |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if pending {
                        tr("Loading preview…")
                    } else {
                        tr("No preview")
                    }),
            )
        })
        .into_any_element()
}

fn kv_row(label: &'static str, value: String, theme: &Theme) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_2()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .text_xs()
        .child(div().text_color(theme.muted_foreground).child(label))
        .child(div().min_w_0().truncate().text_right().child(value))
        .into_any_element()
}

fn kv_row_colored(label: &'static str, value: String, color: Hsla, theme: &Theme) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_2()
        .py_1()
        .text_xs()
        .child(div().text_color(theme.muted_foreground).child(label))
        .child(div().text_color(color).child(value))
        .into_any_element()
}

fn stat_box(value: String, label: &'static str, color: Hsla, theme: &Theme) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .gap_0p5()
        .py_2()
        .rounded(px(7.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(value),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .into_any_element()
}

fn notice_bar(text: String, cx: &mut Context<BatchWindow>) -> AnyElement {
    let theme = cx.theme().clone();
    h_flex()
        .w_full()
        .px_4()
        .py_1p5()
        .gap_2()
        .items_center()
        .bg(theme.danger.opacity(0.10))
        .border_b_1()
        .border_color(theme.danger.opacity(0.35))
        .text_xs()
        .text_color(theme.danger)
        .child(Icon::new(IconName::TriangleAlert).size(px(13.)))
        .child(div().flex_1().min_w_0().truncate().child(text))
        .child(
            div()
                .id("batch-notice-close")
                .px_1()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|this| this.bg(theme.danger.opacity(0.15)))
                .child("✕")
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.notice = None;
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn overwrite_strip(
    conflicts: Vec<PathBuf>,
    theme: &Theme,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    let examples = conflicts
        .iter()
        .take(3)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("  ·  ");
    h_flex()
        .w_full()
        .flex_shrink_0()
        .min_h(px(54.))
        .px_3()
        .py_2()
        .gap_3()
        .items_center()
        .bg(theme.popover)
        .border_t_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .items_center()
                .px_2()
                .py_1()
                .rounded(px(7.))
                .border_1()
                .border_color(theme.danger)
                .bg(theme.danger.opacity(0.08))
                .text_xs()
                .text_color(theme.danger)
                .child(Icon::new(IconName::TriangleAlert).size(px(14.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(if conflicts.is_empty() {
                            tr("Will overwrite files with the same name. This cannot be undone.")
                                .to_string()
                        } else {
                            format!(
                                "{} {} {}",
                                tr("Will overwrite"),
                                conflicts.len(),
                                tr("existing files. This cannot be undone."),
                            )
                        })
                        .when(!examples.is_empty(), |this| {
                            this.child(
                                div()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(examples),
                            )
                        }),
                ),
        )
        .child(button(tr("Cancel"), false, false, cx, |this, cx| {
            this.confirm_overwrite = None;
            this.confirm_only = None;
            cx.notify();
        }))
        .child(button(
            tr("Overwrite anyway"),
            true,
            false,
            cx,
            |this, cx| {
                let only = this.confirm_only.take();
                this.start_with(only, true, cx)
            },
        ))
        .into_any_element()
}

fn choice_popup(
    choice: &ChoicePopup,
    window: &Window,
    cx: &mut Context<BatchWindow>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let popup_width = match choice.kind {
        ChoiceKind::Preset => 236.,
        ChoiceKind::Sort => 188.,
        _ => 208.,
    };
    let kind = choice.kind;
    let selected = choice.selected;
    let labels = choice.labels.clone();

    deferred(
        anchored().child(
            div()
                .w(window.bounds().size.width)
                .h(window.bounds().size.height)
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.choice = None;
                        cx.notify();
                    }),
                )
                .child(
                    anchored()
                        .position(choice.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .child(
                            v_flex()
                                .occlude()
                                .w(px(popup_width))
                                .p_1()
                                .gap_0p5()
                                .rounded(px(8.))
                                .bg(theme.popover)
                                .text_color(theme.popover_foreground)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_2xl()
                                .text_xs()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|_this, _: &MouseDownEvent, _window, cx| {
                                        cx.stop_propagation();
                                    }),
                                )
                                .children(labels.iter().enumerate().map(|(index, label)| {
                                    let is_selected = selected == Some(index);
                                    h_flex()
                                        .id(("batch-choice", index))
                                        .h(px(26.))
                                        .flex_shrink_0()
                                        .px_2()
                                        .gap_2()
                                        .items_center()
                                        .rounded(px(5.))
                                        .cursor_pointer()
                                        .when(is_selected, |this| {
                                            this.bg(theme.primary.opacity(0.12))
                                        })
                                        .hover(|this| this.bg(theme.list_hover))
                                        .child(div().w(px(12.)).when(is_selected, |this| {
                                            this.child(
                                                Icon::new(IconName::Check)
                                                    .size(px(11.))
                                                    .text_color(theme.primary),
                                            )
                                        }))
                                        .child(div().flex_1().truncate().child(*label))
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, _window, cx| {
                                                this.apply_choice(kind, index, cx);
                                            },
                                        ))
                                })),
                        ),
                ),
        ),
    )
    .with_priority(gpui_kit::base::POPUP_PRIORITY)
    .into_any_element()
}

// ---- helpers --------------------------------------------------------------

fn presets() -> Vec<Preset> {
    vec![
        Preset {
            name: tr("Web pages"),
            hint: tr("1600 · WebP 80"),
            options: BatchOptions {
                size: SizeMode::LongEdge(1600),
                format: BatchFormat::WebP,
                quality: 80,
                only_shrink: true,
                ..BatchOptions::default()
            },
        },
        Preset {
            name: tr("Design delivery"),
            hint: tr("2560 · JPEG 90"),
            options: BatchOptions {
                size: SizeMode::LongEdge(2560),
                format: BatchFormat::Jpeg,
                quality: 90,
                only_shrink: true,
                ..BatchOptions::default()
            },
        },
        Preset {
            name: tr("Archive"),
            hint: tr("Original · PDF"),
            options: BatchOptions {
                size: SizeMode::Original,
                format: BatchFormat::Pdf,
                only_shrink: true,
                ..BatchOptions::default()
            },
        },
    ]
}

struct Preset {
    name: &'static str,
    hint: &'static str,
    options: BatchOptions,
}

fn status_text(item: &Item) -> &'static str {
    if item.excluded {
        return tr("Excluded");
    }
    match item.state {
        ItemState::Ready => tr("Ready"),
        ItemState::Running => tr("Converting"),
        ItemState::Done => tr("Success"),
        ItemState::Failed => tr("Failed"),
        ItemState::Skipped => tr("Skipped"),
    }
}

fn state_color(state: ItemState, excluded: bool, theme: &Theme) -> Hsla {
    if excluded {
        return theme.muted_foreground;
    }
    match state {
        ItemState::Ready => theme.muted_foreground,
        ItemState::Running => theme.primary,
        ItemState::Done => ok_color(),
        ItemState::Failed => theme.danger,
        ItemState::Skipped => theme.muted_foreground,
    }
}

fn ok_color() -> Hsla {
    gpui::hsla(150. / 360., 0.55, 0.46, 1.)
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

fn may_have_alpha(name: &str) -> bool {
    matches!(
        extension(name).as_str(),
        "png" | "apng" | "gif" | "webp" | "tif" | "tiff" | "avif" | "ico" | "tga"
    )
}

fn render_image((width, height, data): &(u32, u32, Vec<u8>)) -> Option<PreviewImage> {
    let buffer = image::RgbaImage::from_raw(*width, *height, data.clone())?;
    let frame = image::Frame::from_parts(buffer, 0, 0, image::Delay::from_numer_denom_ms(100, 1));
    Some(PreviewImage {
        render: Arc::new(RenderImage::new(vec![frame])),
        width: *width,
        height: *height,
    })
}

fn collect_folder(root: &Path, folder: &Path, out: &mut Vec<BatchInput>, depth: usize) {
    if depth > 32 {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<_> = read_dir.flatten().collect();
    entries.sort_by(|a, b| {
        seepic_core::natsort::natural_cmp(
            &a.file_name().to_string_lossy(),
            &b.file_name().to_string_lossy(),
        )
    });

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "output" || name == "_backup" {
            continue;
        }
        if path.is_dir() {
            collect_folder(root, &path, out, depth + 1);
        } else if seepic_core::image_formats::is_image_extension(&name) {
            let relative = path.strip_prefix(root).ok().map(Path::to_path_buf);
            out.push(BatchInput::file(path, relative));
        }
    }
}

fn run_batch(
    jobs: Vec<Job>,
    options: BatchOptions,
    mirror_root: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    sender: Sender<RunMessage>,
) {
    let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let workers = options.jobs.clamp(1, batch::max_jobs());
    let mut handles = Vec::new();

    for _ in 0..workers {
        let queue = queue.clone();
        let cancel = cancel.clone();
        let paused = paused.clone();
        let sender = sender.clone();
        let options = options.clone();
        let mirror_root = mirror_root.clone();
        handles.push(std::thread::spawn(move || {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                // Pause only between files: a file already being written is left
                // to finish, so pausing never leaves a partial output.
                while paused.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let job = {
                    let mut queue = queue
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    queue.pop_front()
                };
                let Some(job) = job else {
                    break;
                };
                let id = job.id;
                if sender.send(RunMessage::Started { id }).is_err() {
                    break;
                }
                let outcome = process_job(job, &options, mirror_root.as_deref());
                if sender.send(RunMessage::Item { id, outcome }).is_err() {
                    break;
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
    let _ = sender.send(RunMessage::Done {
        cancelled: cancel.load(Ordering::Relaxed),
    });
}

fn process_job(job: Job, options: &BatchOptions, mirror_root: Option<&Path>) -> ItemOutcome {
    let mut attempt = 0;
    loop {
        let result = (|| -> Result<ItemOutcome, BatchError> {
            let bytes = job.source.read().map_err(BatchError::Read)?;
            let conversion = batch::convert(&bytes, &job.name, options)?;
            let planned = batch::planned_output_path(
                &job.source_path,
                job.relative.as_deref(),
                options,
                mirror_root,
                &job.name,
                conversion.width,
                conversion.height,
                job.sequence,
            )?;
            let decision = batch::resolve_conflict(&planned, options.conflict);
            let target = match decision {
                batch::ConflictDecision::Skip => {
                    return Ok(ItemOutcome::Skipped {
                        reason: tr("The output file already exists").to_string(),
                    });
                }
                batch::ConflictDecision::Write(path) => path,
                batch::ConflictDecision::Backup { target, backup } => {
                    if let Some(parent) = backup.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            BatchError::Write(format!("{}: {error}", parent.display()))
                        })?;
                    }
                    if let Err(error) = std::fs::rename(&target, &backup) {
                        return Err(BatchError::Write(format!("{}: {error}", backup.display())));
                    }
                    target
                }
            };
            let bytes_written = conversion.bytes.len() as u64;
            batch::write_atomically(&target, &conversion.bytes)?;
            if options.keep_timestamps {
                if let Some(modified) = job.modified {
                    let _ = set_modified(&target, modified);
                }
            }
            Ok(ItemOutcome::Done {
                path: target,
                bytes: bytes_written,
            })
        })();

        match result {
            Ok(outcome) => return outcome,
            Err(error) => {
                let retryable = matches!(
                    error,
                    BatchError::Read(_) | BatchError::Decode(_) | BatchError::Write(_)
                );
                if retryable && attempt < 2 {
                    std::thread::sleep(batch::retry_delay(attempt));
                    attempt += 1;
                    continue;
                }
                return ItemOutcome::Failed {
                    code: error.code(),
                    message: error.to_string(),
                };
            }
        }
    }
}

fn set_modified(path: &Path, modified: SystemTime) -> std::io::Result<()> {
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
}
