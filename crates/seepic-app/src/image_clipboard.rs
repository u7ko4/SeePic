//! Putting an image on the Windows clipboard as a device-independent bitmap.
//!
//! GPUI's own clipboard image path writes a private "PNG" clipboard format,
//! which apps like Paint and Word do not read. Writing `CF_DIB` directly is what
//! makes "copy image" actually pasteable.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};

/// Clipboard format id for a device-independent bitmap.
const CF_DIB: u32 = 8;

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

        // Another app can hold the clipboard briefly; a few retries is the
        // standard remedy.
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(0 as HWND) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            return Err("剪贴板被其他程序占用".to_string());
        }

        EmptyClipboard();
        let copied = SetClipboardData(CF_DIB, memory);
        CloseClipboard();

        if copied == 0 {
            return Err("写入剪贴板失败".to_string());
        }
        Ok(())
    }
}
