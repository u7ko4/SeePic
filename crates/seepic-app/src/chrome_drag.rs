//! Dragging the window by our own chrome row.
//!
//! GPUI's `Window::start_window_move` is a no-op in this build, and the native
//! title bar is a system caption region, so the row we draw ourselves has no
//! way to move the window through GPUI. The standard Windows recipe is used
//! instead: hand the window a caption-button-down message, which puts it into
//! its own move loop until the button is released.
//!
//! That loop is modal and runs on this thread, so Windows keeps dispatching the
//! thread's messages while it is up — including the messages that run our
//! main-thread tasks. Entered from inside a GPUI update (as a mouse handler is),
//! one of those tasks updating the entity panics with `RefCell already borrowed`,
//! because the update on the stack still holds the app. The loop is therefore
//! entered from a task, by handle, with nothing borrowed; see
//! `ViewerApp::on_window_mouse_move`.

use std::time::Duration;

/// Restore a maximized window.
///
/// GPUI's `Window::zoom_window` only ever maximizes on Windows — it calls
/// `ShowWindow(SW_MAXIMIZE)` unconditionally — so the way back out has to be
/// asked for directly.
pub fn restore(window: &gpui_kit::Window) {
    use raw_window_handle::RawWindowHandle;
    // Asynchronous, like GPUI's own maximize: a synchronous ShowWindow
    // dispatches WM_SIZE while the event handler is still on the stack, and the
    // relayout that follows is lost, leaving the chrome laid out at the old
    // (maximized) width.
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_RESTORE, ShowWindowAsync};

    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as isize;

    // SAFETY: the handle comes from the live window.
    unsafe {
        let _ = ShowWindowAsync(hwnd, SW_RESTORE);
    }
}

/// How long two clicks may be apart and still count as one double click.
///
/// The platform works this out for itself, but GPUI never receives the result
/// inside the window (Windows reports the second press as `WM_LBUTTONDBLCLK`,
/// which it does not dispatch), so the app measures it too — and it measures it
/// the way Windows does, from the user's own setting.
pub fn double_click_interval() -> Duration {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;

    // SAFETY: a system query with no arguments and no failure mode.
    let millis = unsafe { GetDoubleClickTime() };
    // The app times two *releases*, which are always a little further apart
    // than the two presses the platform times, since the first press is held
    // for however long the user held it.
    Duration::from_millis(u64::from(millis) + 100)
}

/// The window's native handle, for work that has to run outside a GPUI update.
///
/// Returns quietly when the platform window cannot be reached, because a window
/// that will not move is a smaller problem than a panic.
pub fn native_handle(window: &gpui_kit::Window) -> Option<isize> {
    use raw_window_handle::RawWindowHandle;

    // The inherent `Window::window_handle` returns an AnyWindowHandle, so the
    // trait method is called explicitly to reach the platform handle.
    let handle = raw_window_handle::HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(win32.hwnd.get() as isize)
}

/// Whether the window is minimised, and therefore not being looked at.
///
/// The viewer keeps preparing neighbouring pages while it is idle; with the
/// window minimised there is nothing to prepare them for, so that work is
/// skipped until it comes back. GPUI has no query for this, so the platform is
/// asked directly.
pub fn is_minimized(window: &gpui_kit::Window) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsIconic;

    let Some(hwnd) = native_handle(window) else {
        return false;
    };
    // SAFETY: the handle comes from the live window; `IsIconic` only reads it.
    unsafe { IsIconic(hwnd) != 0 }
}

/// Start a native window drag by handle.
///
/// Blocking: this does not return until the user lets go of the button. Call it
/// with nothing borrowed from the app — from a task, not from an update.
pub fn start_handle(hwnd: isize) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HTCAPTION, SendMessageW, WM_NCLBUTTONDOWN,
    };

    // SAFETY: the handle came from the live window, and both messages are the
    // documented way to begin a caption drag from a client area.
    unsafe {
        ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
    }
}
