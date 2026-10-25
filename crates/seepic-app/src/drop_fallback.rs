//! Legacy `WM_DROPFILES` handling for file drops.
//!
//! GPUI registers an OLE drop target, which is the channel a shell drag uses.
//! Some drag sources still post the old `WM_DROPFILES` message instead, so this
//! hooks the window procedure, collects the paths, and queues them for the view
//! to pick up on its next frame.
//!
//! It also makes the drop path testable: an OLE drag session cannot be
//! synthesised from a script, but posting `WM_DROPFILES` can.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, GetWindowLongPtrW, SetWindowLongPtrW,
};

/// `WM_DROPFILES`, which lives in the shell headers.
const WM_DROPFILES: u32 = 0x0233;

/// Paths received since the view last looked.
static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// The previous window procedure, restored for every message we do not handle.
static ORIGINAL_PROC: AtomicIsize = AtomicIsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Hook `hwnd` once, so `WM_DROPFILES` reaches us instead of being dropped.
pub fn install(hwnd: isize) {
    if hwnd == 0 || INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    // SAFETY: `hwnd` came from the live window, and the message loop is running
    // on this thread; installing a subclass is the documented way to observe
    // messages. Every message we do not handle is forwarded verbatim.
    unsafe {
        DragAcceptFiles(hwnd as HWND, 1);
        let previous = GetWindowLongPtrW(hwnd as HWND, GWLP_WNDPROC);
        if previous == 0 {
            INSTALLED.store(false, Ordering::SeqCst);
            return;
        }
        ORIGINAL_PROC.store(previous, Ordering::SeqCst);
        SetWindowLongPtrW(hwnd as HWND, GWLP_WNDPROC, subclass_proc as *const () as isize);
    }
}

/// Drain the paths collected since the last call.
pub fn take_pending() -> Vec<PathBuf> {
    PENDING
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DROPFILES {
        // SAFETY: `wparam` is the HDROP the shell handed us for this message,
        // and each call below is the documented way to read one.
        unsafe {
            let drop = wparam as HDROP;
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            let mut paths = Vec::with_capacity(count as usize);

            for index in 0..count {
                let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0);
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length as usize + 1];
                let written =
                    DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32);
                if written == 0 {
                    continue;
                }
                buffer.truncate(written as usize);
                paths.push(PathBuf::from(String::from_utf16_lossy(&buffer)));
            }
            DragFinish(drop);

            if !paths.is_empty() {
                if let Ok(mut queue) = PENDING.lock() {
                    queue.extend(paths);
                }
                // A paint pass is what gives the view a chance to pick them up.
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
        }
        return 0;
    }

    let original = ORIGINAL_PROC.load(Ordering::SeqCst);
    // SAFETY: `ORIGINAL_PROC` only ever holds a value obtained from
    // GetWindowLongPtrW for this same slot.
    let procedure = unsafe {
        std::mem::transmute::<
            isize,
            Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
        >(original)
    };
    match procedure {
        // SAFETY: this is the procedure we replaced; forwarding to it keeps
        // GPUI's own message handling intact.
        Some(procedure) => unsafe {
            CallWindowProcW(Some(procedure), hwnd, message, wparam, lparam)
        },
        None => 0,
    }
}
