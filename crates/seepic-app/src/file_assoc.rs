//! Windows file associations for the settings page.
//!
//! Associations live in the per-user `HKCU\Software\Classes` hive, which is
//! what makes this possible without elevation. For every format SeePic takes,
//! the registration consists of a ProgID (`SeePic.jpg`) that names the
//! executable, an `OpenWithProgids` entry on the extension so the app appears
//! in "Open with", and the extension's default value.
//!
//! Windows 8 and later keep the user's explicit choice in
//! `...\Explorer\FileExts\.ext\UserChoice`, which a program cannot overwrite.
//! Writing the extension's default still registers the handler and takes
//! effect where no `UserChoice` exists; elsewhere the page points the user at
//! the system Default apps page, where the registration makes SeePic
//! available as a choice.

use std::ffi::OsStr;
use std::path::Path;

use windows_sys::Win32::System::Registry::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::registry::{
    CLASSES, delete_tree, delete_value, key_exists, notify_shell, query, set_none, set_string,
    wide_os,
};

/// The prefix every ProgID this app writes uses.
const PROGID_PREFIX: &str = "SeePic.";

/// The value that keeps the extension's previous default for [`unregister`].
///
/// A user may have pointed `.jpg` at a different viewer before registering
/// here; unregistering should put it back rather than leave the extension
/// pointing at a ProgID that no longer exists.
const BACKUP_VALUE: &str = "SeePic_bak";

const APP_KEY: &str = "Software\\Classes\\Applications\\seepic.exe";
const CAPABILITIES: &str = "Software\\SeePic\\Capabilities";
const REGISTERED_APPS: &str = "Software\\RegisteredApplications";

/// What Windows currently does with one extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Association {
    /// The app that opens the extension, as a friendly name when one exists.
    pub owner: Option<String>,
    /// SeePic is registered as a handler, whether or not it is the default.
    pub registered: bool,
    /// The effective default is this app.
    pub is_default: bool,
}

/// Read the current association for `extension` (lowercase, without the dot).
///
/// This runs in the settings render pass, so it only uses direct registry
/// reads; the previous implementation spawned a `cmd /C assoc` per row, which
/// blocked the UI thread for every format on every frame.
pub fn current(extension: &str) -> Association {
    let user_choice = query(
        HKEY_CURRENT_USER,
        &user_choice_key(extension),
        Some("ProgId"),
    );
    let local = query(HKEY_CURRENT_USER, &extension_key(extension), None);
    let effective = non_empty(user_choice)
        .or_else(|| non_empty(local))
        .or_else(|| non_empty(query(HKEY_CLASSES_ROOT, &format!(".{extension}"), None)));

    let registered = query(
        HKEY_CURRENT_USER,
        &format!("{}\\{}\\shell\\open\\command", CLASSES, progid(extension)),
        None,
    )
    .is_some();
    let is_default = effective.as_deref().is_some_and(is_our_progid);

    Association {
        owner: effective.map(|progid| friendly_name(&progid)),
        registered,
        is_default,
    }
}

/// Register SeePic as a handler for `extension`.
pub fn register(extension: &str) -> Result<(), String> {
    let (exe, command) = executable()?;
    register_app(&exe, &command)?;
    register_one(extension, &exe, &command)?;
    notify_shell();
    Ok(())
}

/// Remove SeePic's handler for `extension`, restoring the previous default.
pub fn unregister(extension: &str) -> Result<(), String> {
    unregister_one(extension)?;
    notify_shell();
    Ok(())
}

/// Register SeePic as a handler for every extension in `extensions`.
pub fn register_all(extensions: &[&str]) -> Result<usize, String> {
    let (exe, command) = executable()?;
    register_app(&exe, &command)?;
    for extension in extensions {
        register_one(extension, &exe, &command)?;
    }
    notify_shell();
    Ok(extensions.len())
}

/// Remove SeePic's handler for every extension in `extensions`.
pub fn unregister_all(extensions: &[&str]) -> Result<usize, String> {
    for extension in extensions {
        unregister_one(extension)?;
    }
    notify_shell();
    Ok(extensions.len())
}

/// Open Windows' Default apps page, where the user confirms the default.
///
/// Returns `false` when the shell refuses the URI, which only happens on
/// versions that predate the Settings app.
pub fn open_default_apps() -> bool {
    let verb = wide_os(OsStr::new("open"));
    let uri = wide_os(OsStr::new("ms-settings:defaultapps"));

    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the
    // call, and the verb and URI are standard shell arguments.
    let result = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    result > 32
}

// ---- registration ---------------------------------------------------------

fn executable() -> Result<(String, String), String> {
    let exe = std::env::current_exe().map_err(|error| format!("找不到程序路径：{error}"))?;
    if !exe.is_file() {
        return Err(format!("程序路径不存在：{}", exe.display()));
    }
    let exe = exe.to_string_lossy().into_owned();
    let command = format!("\"{exe}\" \"%1\"");
    Ok((exe, command))
}

fn register_app(exe: &str, command: &str) -> Result<(), String> {
    set_string(HKEY_CURRENT_USER, APP_KEY, None, "SeePic")?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{APP_KEY}\\DefaultIcon"),
        None,
        &format!("\"{exe}\",0"),
    )?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{APP_KEY}\\shell\\open"),
        Some("FriendlyAppName"),
        "SeePic",
    )?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{APP_KEY}\\shell\\open\\command"),
        None,
        command,
    )?;

    set_string(
        HKEY_CURRENT_USER,
        CAPABILITIES,
        Some("ApplicationName"),
        "SeePic",
    )?;
    set_string(
        HKEY_CURRENT_USER,
        CAPABILITIES,
        Some("ApplicationDescription"),
        "SeePic 图片查看器",
    )?;
    set_string(
        HKEY_CURRENT_USER,
        CAPABILITIES,
        Some("ApplicationIcon"),
        &format!("\"{exe}\",0"),
    )?;
    // Makes SeePic show up as an application in Settings → Default apps on
    // versions that read the per-user list.
    set_string(
        HKEY_CURRENT_USER,
        REGISTERED_APPS,
        Some("SeePic"),
        CAPABILITIES,
    )?;
    Ok(())
}

fn register_one(extension: &str, exe: &str, command: &str) -> Result<(), String> {
    let id = progid(extension);
    let icon = crate::format_icons::ensure_icon_file(extension)
        .map(|path| icon_value(&path))
        .unwrap_or_else(|| format!("\"{exe}\",0"));

    let prog = format!("{CLASSES}\\{id}");
    set_string(
        HKEY_CURRENT_USER,
        &prog,
        None,
        &format!("{} 图片", extension.to_ascii_uppercase()),
    )?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{prog}\\DefaultIcon"),
        None,
        &icon,
    )?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{prog}\\shell\\open"),
        Some("FriendlyAppName"),
        "SeePic",
    )?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{prog}\\shell\\open\\command"),
        None,
        command,
    )?;

    // Point the extension at the ProgID, keeping what was there so that
    // unregistering can put it back. The backup is written only once, so
    // registering again does not overwrite the original with our own ProgID.
    let key = extension_key(extension);
    let existing = query(HKEY_CURRENT_USER, &key, None).unwrap_or_default();
    if existing != id && query(HKEY_CURRENT_USER, &key, Some(BACKUP_VALUE)).is_none() {
        set_string(HKEY_CURRENT_USER, &key, Some(BACKUP_VALUE), &existing)?;
    }
    set_string(HKEY_CURRENT_USER, &key, None, &id)?;
    set_none(
        HKEY_CURRENT_USER,
        &format!("{key}\\OpenWithProgids"),
        Some(&id),
    )?;

    // The Applications entry is what the "Open with" menu lists, and its
    // SupportedTypes values limit it to the formats registered here.
    set_none(
        HKEY_CURRENT_USER,
        &format!("{APP_KEY}\\SupportedTypes"),
        Some(&format!(".{extension}")),
    )?;
    set_string(
        HKEY_CURRENT_USER,
        &format!("{CAPABILITIES}\\FileAssociations"),
        Some(&format!(".{extension}")),
        &id,
    )?;
    Ok(())
}

fn unregister_one(extension: &str) -> Result<(), String> {
    let id = progid(extension);
    let key = extension_key(extension);

    // If SeePic was chosen as the default through the system Settings page,
    // the choice is recorded in UserChoice and would otherwise outlive the
    // ProgID deleted below, leaving the extension pointing at nothing.
    let choice = user_choice_key(extension);
    if query(HKEY_CURRENT_USER, &choice, Some("ProgId"))
        .as_deref()
        .is_some_and(is_our_progid)
    {
        delete_tree(HKEY_CURRENT_USER, &choice);
    }

    if query(HKEY_CURRENT_USER, &key, None).as_deref() == Some(id.as_str()) {
        match query(HKEY_CURRENT_USER, &key, Some(BACKUP_VALUE)) {
            // Only restore a handler that still exists; another uninstall may
            // have removed the ProgID since it was recorded.
            Some(previous) if !previous.is_empty() && progid_exists(&previous) => {
                set_string(HKEY_CURRENT_USER, &key, None, &previous)?;
            }
            _ => delete_value(HKEY_CURRENT_USER, &key, None)?,
        }
    }
    delete_value(HKEY_CURRENT_USER, &key, Some(BACKUP_VALUE))?;
    delete_value(
        HKEY_CURRENT_USER,
        &format!("{key}\\OpenWithProgids"),
        Some(&id),
    )?;
    delete_tree(HKEY_CURRENT_USER, &format!("{CLASSES}\\{id}"));
    delete_value(
        HKEY_CURRENT_USER,
        &format!("{APP_KEY}\\SupportedTypes"),
        Some(&format!(".{extension}")),
    )?;
    delete_value(
        HKEY_CURRENT_USER,
        &format!("{CAPABILITIES}\\FileAssociations"),
        Some(&format!(".{extension}")),
    )?;
    Ok(())
}

// ---- state ----------------------------------------------------------------

fn extension_key(extension: &str) -> String {
    format!("{CLASSES}\\.{extension}")
}

fn user_choice_key(extension: &str) -> String {
    format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.{extension}\\UserChoice"
    )
}

fn progid(extension: &str) -> String {
    format!("{PROGID_PREFIX}{extension}")
}

fn is_our_progid(progid: &str) -> bool {
    progid
        .get(..PROGID_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PROGID_PREFIX))
}

fn progid_exists(progid: &str) -> bool {
    key_exists(HKEY_CLASSES_ROOT, progid)
}

/// A human name for a ProgID: the app's own name when the shell knows it,
/// otherwise the type description, otherwise the ProgID itself.
fn friendly_name(progid: &str) -> String {
    if is_our_progid(progid) {
        return "SeePic".to_string();
    }
    if let Some(name) = non_empty(query(
        HKEY_CLASSES_ROOT,
        &format!("{progid}\\shell\\open"),
        Some("FriendlyAppName"),
    )) {
        return name;
    }
    if let Some(name) = non_empty(query(HKEY_CLASSES_ROOT, progid, None)) {
        return name;
    }
    if progid.starts_with("AppX") || progid.starts_with("MSEdge") {
        return "Windows 应用".to_string();
    }
    progid.to_string()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn icon_value(path: &Path) -> String {
    format!("\"{}\",0", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_progids_are_recognised_case_insensitively() {
        assert!(is_our_progid("SeePic.jpg"));
        assert!(is_our_progid("seepic.cbz"));
        assert!(!is_our_progid("Honeyview.jpg"));
        assert!(!is_our_progid("SeePic"));
        assert!(!is_our_progid(""));
    }

    #[test]
    fn the_extension_key_is_under_the_user_classes_hive() {
        assert_eq!(extension_key("jpg"), "Software\\Classes\\.jpg");
        assert_eq!(progid("cbz"), "SeePic.cbz");
    }

    /// The association page and the decoder must agree on what SeePic opens:
    /// a missing row cannot be associated, and an extra row would register a
    /// handler for a format that then fails to open.
    #[test]
    fn the_settings_table_matches_the_supported_formats() {
        let table = crate::settings_panel::association_table();
        let listed: std::collections::HashSet<&str> = table
            .iter()
            .flat_map(|(_, extensions)| extensions.iter().copied())
            .collect();
        let supported: std::collections::HashSet<&str> =
            seepic_core::image_formats::IMAGE_EXTENSIONS
                .iter()
                .chain(seepic_core::image_formats::ARCHIVE_EXTENSIONS)
                .copied()
                .collect();

        for extension in &supported {
            assert!(
                listed.contains(extension),
                ".{extension} can be opened but is not offered for association"
            );
        }
        for extension in &listed {
            assert!(
                supported.contains(extension),
                ".{extension} is offered for association but cannot be opened"
            );
        }
        assert_eq!(
            listed.len(),
            table
                .iter()
                .map(|(_, extensions)| extensions.len())
                .sum::<usize>(),
            "a duplicate row would write the same registry keys twice"
        );
    }

    /// The write path cannot be exercised without touching HKCU, so it is a
    /// manual test: it registers a throwaway extension, checks what the page
    /// would read back, and removes it again.
    #[test]
    #[ignore = "writes a throwaway extension to HKCU; run explicitly"]
    fn registration_round_trips_and_restores_the_previous_default() {
        let extension = "seepicassoctest";
        let key = extension_key(extension);
        let previous = "Honeyview.jpg";

        unregister_one(extension).expect("clean slate");
        set_string(HKEY_CURRENT_USER, &key, None, previous).expect("seed a previous default");

        let exe = r"C:\Program Files\SeePic\seepic.exe";
        register_one(extension, exe, &format!("\"{exe}\" \"%1\"")).expect("register");
        let state = current(extension);
        assert!(state.registered, "the handler must be visible to the page");
        assert!(
            state.is_default,
            "with no UserChoice, the default must be ours"
        );
        assert_eq!(
            query(HKEY_CURRENT_USER, &key, None).as_deref(),
            Some(progid(extension).as_str())
        );

        unregister_one(extension).expect("unregister");
        let state = current(extension);
        assert!(!state.registered, "the handler must be gone");
        assert_eq!(
            query(HKEY_CURRENT_USER, &key, None).as_deref(),
            Some(previous),
            "the previous default must be restored"
        );

        // Leave the user's registry as it was before the test.
        delete_value(HKEY_CURRENT_USER, &key, None).expect("remove the seed");
        delete_tree(HKEY_CURRENT_USER, &key);
    }
}
