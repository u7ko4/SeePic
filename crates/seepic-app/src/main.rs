//! SeePic — a desktop image viewer and comic reader built with GPUI.

// Release builds are a GUI application: launching from Explorer must not open a
// console window. Debug builds keep the console subsystem so the perf traces
// (`IV_TRACE_PERF`) and panic messages stay visible in the terminal.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod boundary_prompt;
mod image_panel;
mod image_clipboard;
mod info_window;
mod settings_panel;
mod canvas;
mod chrome_drag;
mod confirm_dialog;
mod contact_sheet;
mod context_menu;
mod converter;
mod drop_fallback;
mod file_assoc;
mod format_icons;
mod menu_bar;
mod panels;
mod password_dialog;
mod perf;
mod printing;
mod registry;
mod render_cache;
mod shell_menu;
mod status_bar;
mod shell_open;
mod slideshow;
mod thumbnails;
mod trace_alloc;
mod wallpaper;
mod strings;

use gpui_kit::component::*;
use gpui_kit::*;

use app::ViewerApp;

/// Counts the bytes that pass through the allocator, so the perf trace can say
/// whether a growing process total is the viewer's own heap or the framework's
/// and the driver's.
#[global_allocator]
static ALLOCATOR: trace_alloc::Counting = trace_alloc::Counting;

use seepic_core::settings::{MIN_WINDOW_SIZE, Settings};

/// Record a panic where it can still be read after the window is gone.
///
/// Launched from Explorer there is no console: the window disappears with the
/// stack and the report is usually just "it closed". The panic and a backtrace
/// are appended to `panic.log` next to the config, and the normal hook still
/// prints to stderr for a terminal run.
fn install_panic_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;

        let backtrace = std::backtrace::Backtrace::force_capture();
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let report = format!("---- {seconds}\n{info}\n\n{backtrace}\n");
        if let Some(dir) = seepic_core::settings::Settings::path().parent() {
            let _ = std::fs::create_dir_all(dir);
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("panic.log"))
            {
                let _ = file.write_all(report.as_bytes());
            }
        }
        previous(info);
    }));
}

fn main() {
    install_panic_log();

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);

    app.run(move |cx| {
        gpui_kit::init(cx);

        // Image viewers read better dark, and a photograph is easier to judge
        // without a bright frame around it; the user can change this in the
        // settings either way.
        Theme::change(ThemeMode::Dark, None, cx);

        app::bind_keys(cx);

        // Open whatever was passed on the command line, so the app can be
        // registered as the handler for image and comic file types. The
        // Explorer context menu adds `--convert` to open the batch converter
        // on the selection instead of just browsing it.
        let mut initial = None;
        let mut start_in_converter = false;
        for argument in std::env::args_os().skip(1) {
            let text = argument.to_string_lossy();
            if text == "--convert" {
                start_in_converter = true;
            } else if !text.starts_with("--") && initial.is_none() {
                initial = Some(std::path::PathBuf::from(argument));
            }
        }

        // The window opens at the size it was last closed at, unless the user
        // turned that off; the viewer itself loads the same settings below.
        let (w, h) = Settings::load().initial_window_size();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(w), px(h)),
                cx,
            ))),
            // A viewer is useful small: the bars shed their optional parts as
            // the window narrows (see `menu_bar::MENUS_MIN_WIDTH` and the
            // status bar's widths), so the floor is what the window buttons,
            // the title and the status bar's icon row need together.
            window_min_size: Some(size(
                px(MIN_WINDOW_SIZE.0),
                px(MIN_WINDOW_SIZE.1),
            )),
            ..TitleBar::window_options()
        };

        gpui_kit::open_window(options, cx, move |_window, cx| {
            cx.new(|cx| ViewerApp::new(initial.clone(), start_in_converter, cx))
        })
        .expect("failed to open the main window");
    });
}
