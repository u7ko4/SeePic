//! Printing the image that is on screen.
//!
//! A full print dialog (printer, rotation, fit, margins, copies) would mean
//! owning page setup, which is a
//! large amount of code for something Windows already does: writing the image
//! out and handing it to the shell's `print` verb gets the real, installed
//! printer list and the OS's own options, and prints at the image's own
//! resolution rather than the screen's.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use image::{DynamicImage, RgbaImage};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Write `bytes` (BGRA) to `path` as a PNG and ask the shell to print it.
///
/// PNG rather than JPEG: printing a JPEG would re-compress an image that has
/// already been compressed once.
pub fn print_bgra(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    if width == 0 || height == 0 || bgra.len() != (width as usize) * (height as usize) * 4 {
        return Err("图像数据无效".to_string());
    }

    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let Some(buffer) = RgbaImage::from_raw(width, height, rgba) else {
        return Err("图像数据无效".to_string());
    };
    DynamicImage::ImageRgba8(buffer)
        .save(path)
        .map_err(|error| format!("写入临时文件失败: {error}"))?;

    shell_print(path)
}

/// Hand `path` to the shell's print verb.
///
/// This hands the file to the shell's print verb, so the system's own print
/// dialog supplies the printer, the copy count and the page layout.
fn shell_print(path: &Path) -> Result<(), String> {
    let wide = |text: &std::ffi::OsStr| {
        text.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>()
    };
    let operation = wide(std::ffi::OsStr::new("print"));
    let file = wide(path.as_os_str());

    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the
    // call, and the verb is a standard shell verb.
    let result = unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecute returns a value greater than 32 on success; anything else is
    // an error code, and the common one here is "no association".
    if result > 32 {
        Ok(())
    } else {
        Err("系统没有可用的打印程序".to_string())
    }
}
