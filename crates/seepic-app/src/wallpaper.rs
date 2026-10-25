//! Setting the desktop wallpaper on Windows.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
};

/// Point the desktop at `path`.
///
/// Windows takes a file path here rather than pixels, so the caller writes a
/// format it understands (JPEG) to a temporary file first.
pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call, and
    // the parameters match what SPI_SETDESKWALLPAPER expects.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            wide.as_ptr() as *mut c_void,
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    };

    if ok == 0 {
        Err("系统拒绝了壁纸设置".to_string())
    } else {
        Ok(())
    }
}
