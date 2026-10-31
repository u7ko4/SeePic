//! Shared Win32 registry helpers for the per-user shell integration.
//!
//! Both the file-association page and the Explorer context menu write under
//! `HKCU\Software\Classes`, which needs no elevation. These wrappers keep the
//! unsafe pointer handling in one place.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_NONE, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS,
    REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

/// The per-user classes hive, as a path below [`HKEY_CURRENT_USER`].
pub(crate) const CLASSES: &str = "Software\\Classes";

/// Tell Explorer that associations or context menus changed.
pub(crate) fn notify_shell() {
    // SAFETY: the shell change notification takes no owned pointers.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

/// A NUL-terminated UTF-16 copy of a string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A NUL-terminated UTF-16 copy of an OS string.
pub(crate) fn wide_os(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

pub(crate) fn open(root: HKEY, subkey: &str, access: REG_SAM_FLAGS) -> Option<HKEY> {
    let subkey = wide(subkey);
    let mut key: HKEY = 0;

    // SAFETY: `subkey` is a NUL-terminated buffer that outlives the call, and
    // `key` is only used when the call reports success.
    let status = unsafe { RegOpenKeyExW(root, subkey.as_ptr(), 0, access, &mut key) };
    (status == ERROR_SUCCESS).then_some(key)
}

/// Whether a key exists, without handing the caller a handle to close.
pub(crate) fn key_exists(root: HKEY, subkey: &str) -> bool {
    let Some(key) = open(root, subkey, KEY_READ) else {
        return false;
    };
    // SAFETY: `key` was opened by `open` and is not used afterwards.
    unsafe { RegCloseKey(key) };
    true
}

/// Whether a value exists, whatever its type.
///
/// Flag values such as `Extended` and `NeverDefault` are empty and typed
/// `REG_NONE`, so [`query`] (which decodes strings) cannot see them. Only the
/// registry round-trip test needs this today.
#[cfg(test)]
pub(crate) fn value_exists(root: HKEY, subkey: &str, value: Option<&str>) -> bool {
    let Some(key) = open(root, subkey, KEY_READ) else {
        return false;
    };
    let value_name = value.map(wide);
    let value_ptr = value_name
        .as_ref()
        .map_or(std::ptr::null(), |name| name.as_ptr());
    let mut kind: REG_VALUE_TYPE = 0;
    let mut bytes: u32 = 0;

    // SAFETY: the value name is a NUL-terminated buffer, and a null data
    // pointer asks for the size alone.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_ptr,
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    // SAFETY: `key` is not used afterwards.
    unsafe { RegCloseKey(key) };
    status == ERROR_SUCCESS
}

fn create(root: HKEY, subkey: &str) -> Result<HKEY, String> {
    let name = wide(subkey);
    let mut key: HKEY = 0;

    // SAFETY: every pointer argument either points at a live local or is
    // explicitly null, which the API accepts for the optional outputs.
    let status = unsafe {
        RegCreateKeyExW(
            root,
            name.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(format!("无法创建注册表项 {subkey}（代码 {status}）"))
    }
}

pub(crate) fn query(root: HKEY, subkey: &str, value: Option<&str>) -> Option<String> {
    let key = open(root, subkey, KEY_READ)?;
    let value_name = value.map(wide);
    let value_ptr = value_name
        .as_ref()
        .map_or(std::ptr::null(), |name| name.as_ptr());

    let mut kind: REG_VALUE_TYPE = 0;
    let mut bytes: u32 = 0;

    // First ask for the size. A value that does not exist fails here.
    // SAFETY: the value name is a NUL-terminated buffer, and a null data
    // pointer is how the API is asked for the size alone.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_ptr,
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
        // SAFETY: `key` was opened by `open` and is not used afterwards.
        unsafe { RegCloseKey(key) };
        return None;
    }
    if bytes == 0 {
        // SAFETY: as above.
        unsafe { RegCloseKey(key) };
        return Some(String::new());
    }

    let mut buffer = vec![0u16; bytes.div_ceil(2) as usize + 1];
    let mut read = (buffer.len() * 2) as u32;

    // SAFETY: the buffer is large enough for the size just reported, and its
    // length is passed in bytes.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_ptr,
            std::ptr::null(),
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut read,
        )
    };
    // SAFETY: `key` is no longer used on either path.
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        return None;
    }

    // String values are NUL-terminated; a hand-written value may not be.
    let mut units = (read as usize / 2).min(buffer.len());
    if units > 0 && buffer[units - 1] == 0 {
        units -= 1;
    }
    Some(String::from_utf16_lossy(&buffer[..units]))
}

pub(crate) fn set_string(
    root: HKEY,
    subkey: &str,
    value: Option<&str>,
    data: &str,
) -> Result<(), String> {
    let key = create(root, subkey)?;
    let value_name = value.map(wide);
    let value_ptr = value_name
        .as_ref()
        .map_or(std::ptr::null(), |name| name.as_ptr());
    let data: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: the name and data buffers outlive the call, and the byte length
    // matches the buffer.
    let status = unsafe {
        RegSetValueExW(
            key,
            value_ptr,
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    // SAFETY: `key` belongs to this function and is not used afterwards.
    unsafe { RegCloseKey(key) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("无法写入注册表项 {subkey}（代码 {status}）"))
    }
}

pub(crate) fn set_none(root: HKEY, subkey: &str, value: Option<&str>) -> Result<(), String> {
    let key = create(root, subkey)?;
    let value_name = value.map(wide);
    let value_ptr = value_name
        .as_ref()
        .map_or(std::ptr::null(), |name| name.as_ptr());

    // SAFETY: the name buffer outlives the call; REG_NONE with no data has the
    // required empty payload.
    let status = unsafe { RegSetValueExW(key, value_ptr, 0, REG_NONE, std::ptr::null(), 0) };
    // SAFETY: `key` belongs to this function and is not used afterwards.
    unsafe { RegCloseKey(key) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("无法写入注册表项 {subkey}（代码 {status}）"))
    }
}

pub(crate) fn delete_value(root: HKEY, subkey: &str, value: Option<&str>) -> Result<(), String> {
    let Some(key) = open(root, subkey, KEY_WRITE) else {
        // No key means there is nothing to delete.
        return Ok(());
    };
    let value_name = value.map(wide);
    let value_ptr = value_name
        .as_ref()
        .map_or(std::ptr::null(), |name| name.as_ptr());

    // SAFETY: the name buffer outlives the call.
    let status = unsafe { RegDeleteValueW(key, value_ptr) };
    // SAFETY: `key` belongs to this function and is not used afterwards.
    unsafe { RegCloseKey(key) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("无法删除注册表值 {subkey}（代码 {status}）"))
    }
}

/// Remove a key and everything under it. Missing keys are not an error: the
/// path is also used to clean up registrations written by an older version.
pub(crate) fn delete_tree(root: HKEY, subkey: &str) {
    let name = wide_os(OsStr::new(subkey));

    // SAFETY: `name` is a NUL-terminated buffer that outlives both calls.
    // `RegDeleteTree` leaves the key itself, so `RegDeleteKey` removes it.
    unsafe {
        RegDeleteTreeW(root, name.as_ptr());
        RegDeleteKeyW(root, name.as_ptr());
    }
}
