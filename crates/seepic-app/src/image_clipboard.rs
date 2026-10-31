//! Clipboard integration: pictures as a device-independent bitmap, and files
//! as an Explorer drop list.
//!
//! GPUI's own clipboard image path writes a private "PNG" clipboard format,
//! which apps like Paint and Word do not read, and its writer ignores
//! `ExternalPaths` entirely — a file copy through it silently did nothing.
//! Writing `CF_DIB` and `CF_HDROP` directly is what makes both commands real.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock,
};
use windows_sys::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};

/// Clipboard format id for a device-independent bitmap.
const CF_DIB: u32 = 8;
/// Clipboard format id for a list of files.
const CF_HDROP: u32 = 15;
/// The standard clipboard format that tells a paste whether to copy or move.
const PREFERRED_DROP_EFFECT: &str = "Preferred DropEffect";
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

/// Open the clipboard, retrying briefly while another program holds it.
fn open_clipboard_with_retry() -> Result<(), String> {
    for _ in 0..10 {
        if unsafe { OpenClipboard(0 as HWND) } != 0 {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Err("剪贴板被其他程序占用".to_string())
}

/// Copy a BGRA8 image to the clipboard as a 24-bit DIB.
///
/// 24-bit is deliberate: with a 32-bit DIB several consumers (including .NET)
/// treat the spare byte as alpha and render nothing, while 24-bit is understood
/// everywhere. Rows are bottom-up, which is what a plain `CF_DIB` consumer
/// expects.
pub fn copy_bgra(width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    if width == 0 || height == 0 || bgra.len() != (width as usize) * (height as usize) * 4 {
        return Err("图像数据无效".to_string());
    }

    // Each 24-bit row is padded to a four-byte boundary.
    let row_bytes = ((width as usize * 3).div_ceil(4)) * 4;
    let pixel_bytes = row_bytes * height as usize;
    let header_bytes = std::mem::size_of::<BITMAPINFOHEADER>();
    let total = header_bytes + pixel_bytes;

    // SAFETY: the block is allocated for exactly the header plus the padded
    // rows written below, and ownership passes to the clipboard on success.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total);
        if memory == 0 {
            return Err("无法分配剪贴板内存".to_string());
        }

        let base = GlobalLock(memory) as *mut u8;
        if base.is_null() {
            return Err("无法锁定剪贴板内存".to_string());
        }

        let header = base as *mut BITMAPINFOHEADER;
        (*header).biSize = header_bytes as u32;
        (*header).biWidth = width as i32;
        (*header).biHeight = height as i32;
        (*header).biPlanes = 1;
        (*header).biBitCount = 24;
        (*header).biCompression = BI_RGB as u32;
        (*header).biSizeImage = pixel_bytes as u32;

        let pixels = base.add(header_bytes);
        for y in 0..height as usize {
            // Bottom-up: the last source row is written first.
            let destination_row = height as usize - 1 - y;
            let mut out = pixels.add(destination_row * row_bytes);
            let mut source = y * width as usize * 4;
            for _ in 0..width as usize {
                *out = bgra[source]; // blue
                *out.add(1) = bgra[source + 1]; // green
                *out.add(2) = bgra[source + 2]; // red
                out = out.add(3);
                source += 4;
            }
        }

        GlobalUnlock(memory);

        if let Err(error) = open_clipboard_with_retry() {
            GlobalFree(memory);
            return Err(error);
        }

        EmptyClipboard();
        let copied = SetClipboardData(CF_DIB, memory);
        CloseClipboard();

        if copied == 0 {
            GlobalFree(memory);
            return Err("写入剪贴板失败".to_string());
        }
        Ok(())
    }
}

/// Put files on the clipboard as an Explorer drop list.
///
/// `cut` writes the standard `Preferred DropEffect` marker, so a paste in
/// Explorer or SeePic moves the files instead of copying them. The marker is
/// also how [`read_files`] tells the two apart.
pub fn copy_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有可复制的文件".to_string());
    }

    // DROPFILES followed by a double-NUL-terminated wide path list.
    let mut list: Vec<u16> = Vec::new();
    for path in paths {
        list.extend(path.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);

    let header_bytes = std::mem::size_of::<DROPFILES>();
    let drop = DROPFILES {
        pFiles: header_bytes as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: 0,
        fWide: 1,
    };
    let total = header_bytes + list.len() * 2;

    // SAFETY: the block is sized for the header and the list, and ownership
    // passes to the clipboard only when `SetClipboardData` succeeds.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total);
        if memory == 0 {
            return Err("无法分配剪贴板内存".to_string());
        }
        let base = GlobalLock(memory) as *mut u8;
        if base.is_null() {
            GlobalFree(memory);
            return Err("无法锁定剪贴板内存".to_string());
        }
        std::ptr::write_unaligned(base.cast::<DROPFILES>(), drop);
        std::ptr::copy_nonoverlapping(
            list.as_ptr().cast::<u8>(),
            base.add(header_bytes),
            list.len() * 2,
        );
        GlobalUnlock(memory);

        let effect = preferred_effect_handle(cut);
        if cut && effect.is_none() {
            GlobalFree(memory);
            return Err("无法标记剪切".to_string());
        }

        if let Err(error) = open_clipboard_with_retry() {
            GlobalFree(memory);
            if let Some((_, memory)) = effect {
                GlobalFree(memory);
            }
            return Err(error);
        }

        EmptyClipboard();
        let dropped = SetClipboardData(CF_HDROP, memory);
        if dropped == 0 {
            GlobalFree(memory);
        }
        let mut effect_written = true;
        if let Some((format, memory)) = effect {
            if SetClipboardData(format, memory) == 0 {
                effect_written = false;
                GlobalFree(memory);
            }
        }
        CloseClipboard();

        if dropped == 0 {
            return Err("写入剪贴板失败".to_string());
        }
        if cut && !effect_written {
            return Err("无法标记剪切".to_string());
        }
    }
    Ok(())
}

/// The files currently on the clipboard, and whether they were cut.
///
/// This is read directly rather than through GPUI because the preferred drop
/// effect is what distinguishes a cut from a copy, and GPUI's reader only
/// returns the paths.
pub fn read_files() -> Option<(Vec<PathBuf>, bool)> {
    open_clipboard_with_retry().ok()?;
    let mut result = None;

    // SAFETY: the clipboard is open; `GetClipboardData` returns a handle owned
    // by the clipboard, which must not be freed here.
    unsafe {
        let handle = GetClipboardData(CF_HDROP);
        if handle != 0 {
            let drop = handle as HDROP;
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0);
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length as usize + 1];
                let written = DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32);
                if written > 0 {
                    paths.push(PathBuf::from(String::from_utf16_lossy(
                        &buffer[..written as usize],
                    )));
                }
            }
            if !paths.is_empty() {
                result = Some((paths, preferred_effect_is_move()));
            }
        }
        CloseClipboard();
    }

    result
}

/// Remove the clipboard contents, used after a cut has been pasted completely.
pub fn clear_clipboard() {
    if open_clipboard_with_retry().is_ok() {
        // SAFETY: the clipboard was opened by the call above.
        unsafe {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

/// Build the small `Preferred DropEffect` block that marks a copy or a cut.
fn preferred_effect_handle(cut: bool) -> Option<(u32, isize)> {
    let format = register_format(PREFERRED_DROP_EFFECT);
    if format == 0 {
        return None;
    }

    // SAFETY: the block is four bytes, and ownership passes to the clipboard
    // when `SetClipboardData` succeeds.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, 4);
        if memory == 0 {
            return None;
        }
        let ptr = GlobalLock(memory) as *mut u32;
        if ptr.is_null() {
            GlobalFree(memory);
            return None;
        }
        ptr.write(if cut {
            DROPEFFECT_MOVE
        } else {
            DROPEFFECT_COPY
        });
        GlobalUnlock(memory);
        Some((format, memory))
    }
}

/// Whether the clipboard's files were cut rather than copied.
///
/// Call while the clipboard is open. A missing marker means copy.
fn preferred_effect_is_move() -> bool {
    let format = register_format(PREFERRED_DROP_EFFECT);
    if format == 0 {
        return false;
    }

    // SAFETY: the clipboard is open, and the handle belongs to the clipboard.
    unsafe {
        let handle = GetClipboardData(format);
        if handle == 0 {
            return false;
        }
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            return false;
        }
        let effect = *(ptr as *const u32);
        GlobalUnlock(handle);
        effect & DROPEFFECT_MOVE != 0
    }
}

fn register_format(name: &str) -> u32 {
    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the name is a NUL-terminated wide string.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "replaces the system clipboard; run explicitly"]
    fn files_round_trip_as_a_drop_list() {
        let directory = std::env::temp_dir().join("seepic-clipboard-test");
        std::fs::create_dir_all(&directory).expect("test folder");
        let path = directory.join("copy.txt");
        std::fs::write(&path, b"clipboard").expect("test file");

        copy_files(std::slice::from_ref(&path), false).expect("copy");
        let (paths, cut) = read_files().expect("read the copied files");
        assert_eq!(paths, vec![path.clone()]);
        assert!(!cut, "a copy must not be marked for moving");

        copy_files(std::slice::from_ref(&path), true).expect("cut");
        let (paths, cut) = read_files().expect("read the cut files");
        assert_eq!(paths, vec![path.clone()]);
        assert!(cut, "a cut must carry the move marker");

        clear_clipboard();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&directory);
    }
}
