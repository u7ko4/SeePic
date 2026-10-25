//! Opening folders and revealing files through the shell.
//!
//! Both operations go through the shell rather than by launching a program by
//! name: the shell owns the file-manager window, so asking it to show a folder
//! keeps whatever the user has configured (a different file manager, a new
//! window rather than a reused one) working.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::UI::Shell::{
    ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// A NUL-terminated UTF-16 copy of `text`, which the Win32 calls below need.
fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Show `path` in the file manager, with the file itself selected.
///
/// Returns `false` if the shell could not do it, so the caller can report it.
pub fn reveal(path: &Path) -> bool {
    let file = wide(path.as_os_str());

    // SAFETY: `file` is a NUL-terminated UTF-16 buffer that outlives both calls,
    // and the item id list it returns is freed before this function returns.
    unsafe {
        let id = ILCreateFromPathW(file.as_ptr());
        if id.is_null() {
            return false;
        }
        // No items are passed alongside the folder, which is how the shell is
        // asked to select the folder's own item.
        let result = SHOpenFolderAndSelectItems(id, 0, std::ptr::null(), 0);
        ILFree(id);
        if result >= 0 {
            return true;
        }
    }

    // The selection API needs COM; if it is unavailable, showing the containing
    // folder still answers the user's request.
    path.parent().is_some_and(open_folder)
}

/// Open a file with whatever program is associated with its type.
pub fn open_default(path: &Path) -> bool {
    let operation = wide(std::ffi::OsStr::new("open"));
    let target = wide(path.as_os_str());

    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive
    // the call, and the verb is a standard shell verb.
    let result = unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    result > 32
}

/// Show a folder in the file manager.
pub fn open_folder(folder: &Path) -> bool {
    let operation = wide(std::ffi::OsStr::new("open"));
    let target = wide(folder.as_os_str());

    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the
    // call, and the verb is a standard shell verb.
    let result = unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecute returns a value greater than 32 on success.
    result > 32
}

#[cfg(test)]
mod tests {
    /// Smoke test for the reveal path, which cannot be exercised without a
    /// desktop: it is ignored by default and run explicitly.
    #[test]
    #[ignore = "opens a file-manager window; run explicitly"]
    fn revealing_a_real_file_is_accepted_by_the_shell() {
        let file = std::env::current_exe().expect("the test binary has a path");
        assert!(
            super::reveal(&file),
            "the shell refused to show {}",
            file.display()
        );
    }

    /// Opening a folder uses the same verb the print path does, which works
    /// without a registered file type.
    #[test]
    #[ignore = "opens a file-manager window; run explicitly"]
    fn opening_a_real_folder_is_accepted_by_the_shell() {
        let dir = std::env::temp_dir();
        assert!(
            super::open_folder(&dir),
            "the shell refused to open {}",
            dir.display()
        );
    }
}
