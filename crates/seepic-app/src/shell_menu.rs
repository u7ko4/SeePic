//! Explorer context-menu integration.
//!
//! The menu is registered through the per-user classes hive, so it needs no
//! elevation and no COM server: Explorer reads these verbs directly. Each
//! supported format gets its own `SystemFileAssociations` entry, which keeps
//! the menu off unrelated files. On Windows 11 these classic verbs live under
//! "Show more options".

use seepic_core::image_formats::{ARCHIVE_EXTENSIONS, IMAGE_EXTENSIONS};
use seepic_core::settings::Settings;
use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;

use crate::registry::{CLASSES, delete_tree, notify_shell, set_none, set_string};

const VIEW_VERB: &str = "SeePicView";
const CONVERT_VERB: &str = "SeePicConvert";

/// What the system-menu page currently asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub viewer: bool,
    pub convert: bool,
    pub convert_shift: bool,
    pub on_folder: bool,
    pub on_background: bool,
    pub icon: bool,
}

impl Config {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            enabled: settings.shell_menu_enable,
            viewer: settings.shell_menu_viewer,
            convert: settings.shell_menu_convert,
            convert_shift: settings.shell_menu_convert_shift,
            on_folder: settings.shell_menu_on_folder,
            on_background: settings.shell_menu_on_background,
            icon: settings.shell_preview_icon,
        }
    }
}

/// Replace the registered context menu with what `config` describes.
///
/// Everything is removed first, so turning the feature off also cleans up
/// entries written by an older version, and a format that left the supported
/// list cannot leave a stale verb behind.
pub fn apply(config: Config) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| format!("找不到程序路径：{error}"))?;
    if !exe.is_file() {
        return Err(format!("程序路径不存在：{}", exe.display()));
    }
    let exe = exe.to_string_lossy();
    let icon = format!("\"{exe}\",0");
    let view = format!("\"{exe}\" \"%1\"");
    let convert = format!("\"{exe}\" --convert \"%1\"");
    let view_background = format!("\"{exe}\" \"%V\"");
    let convert_background = format!("\"{exe}\" --convert \"%V\"");

    remove_all();

    if config.enabled {
        for &extension in IMAGE_EXTENSIONS.iter().chain(ARCHIVE_EXTENSIONS) {
            let shell = format!("{CLASSES}\\SystemFileAssociations\\.{extension}\\shell");
            if config.viewer {
                add(
                    &shell,
                    VIEW_VERB,
                    "用 SeePic 浏览",
                    &view,
                    false,
                    config.icon.then_some(icon.as_str()),
                )?;
            }
            if config.convert {
                add(
                    &shell,
                    CONVERT_VERB,
                    "用 SeePic 转换",
                    &convert,
                    config.convert_shift,
                    config.icon.then_some(icon.as_str()),
                )?;
            }
        }

        if config.on_folder {
            let shell = format!("{CLASSES}\\Directory\\shell");
            if config.viewer {
                add(
                    &shell,
                    VIEW_VERB,
                    "用 SeePic 浏览",
                    &view,
                    false,
                    config.icon.then_some(icon.as_str()),
                )?;
            }
            if config.convert {
                add(
                    &shell,
                    CONVERT_VERB,
                    "用 SeePic 转换",
                    &convert,
                    config.convert_shift,
                    config.icon.then_some(icon.as_str()),
                )?;
            }
        }

        if config.on_background {
            let shell = format!("{CLASSES}\\Directory\\Background\\shell");
            if config.viewer {
                add(
                    &shell,
                    VIEW_VERB,
                    "用 SeePic 浏览",
                    &view_background,
                    false,
                    config.icon.then_some(icon.as_str()),
                )?;
            }
            if config.convert {
                add(
                    &shell,
                    CONVERT_VERB,
                    "用 SeePic 转换",
                    &convert_background,
                    config.convert_shift,
                    config.icon.then_some(icon.as_str()),
                )?;
            }
        }
    }

    notify_shell();
    Ok(())
}

fn add(
    shell: &str,
    verb: &str,
    label: &str,
    command: &str,
    extended: bool,
    icon: Option<&str>,
) -> Result<(), String> {
    let key = format!("{shell}\\{verb}");
    set_string(HKEY_CURRENT_USER, &key, None, label)?;
    // A context-menu verb must not become the default action, or double-click
    // would change from the association's handler to this one.
    set_none(HKEY_CURRENT_USER, &key, Some("NeverDefault"))?;
    if extended {
        set_none(HKEY_CURRENT_USER, &key, Some("Extended"))?;
    }
    if let Some(icon) = icon {
        set_string(HKEY_CURRENT_USER, &key, Some("Icon"), icon)?;
    }
    set_string(HKEY_CURRENT_USER, &format!("{key}\\command"), None, command)?;
    Ok(())
}

fn remove_all() {
    for &extension in IMAGE_EXTENSIONS.iter().chain(ARCHIVE_EXTENSIONS) {
        let shell = format!("{CLASSES}\\SystemFileAssociations\\.{extension}\\shell");
        delete_verb(&shell, VIEW_VERB);
        delete_verb(&shell, CONVERT_VERB);
    }
    for shell in [
        format!("{CLASSES}\\Directory\\shell"),
        format!("{CLASSES}\\Directory\\Background\\shell"),
        // A hand-made or older registration may have used the all-files key.
        format!("{CLASSES}\\*\\shell"),
    ] {
        delete_verb(&shell, VIEW_VERB);
        delete_verb(&shell, CONVERT_VERB);
    }
}

fn delete_verb(shell: &str, verb: &str) {
    delete_tree(HKEY_CURRENT_USER, &format!("{shell}\\{verb}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{query, value_exists};

    #[test]
    fn the_config_carries_every_visibility_switch() {
        let mut settings = Settings::default();
        settings.shell_menu_enable = true;
        settings.shell_menu_viewer = false;
        settings.shell_menu_convert = true;
        settings.shell_menu_convert_shift = true;
        settings.shell_menu_on_folder = false;
        settings.shell_menu_on_background = true;
        settings.shell_preview_icon = true;

        let config = Config::from_settings(&settings);
        assert!(config.enabled);
        assert!(!config.viewer);
        assert!(config.convert);
        assert!(config.convert_shift);
        assert!(!config.on_folder);
        assert!(config.on_background);
        assert!(config.icon);
    }

    /// The registry write path is a manual test: it registers the real verbs,
    /// checks one file, one folder and one background entry, then removes
    /// everything again.
    #[test]
    #[ignore = "writes context-menu keys to HKCU; run explicitly"]
    fn the_menu_round_trips_through_the_registry() {
        let on = Config {
            enabled: true,
            viewer: true,
            convert: true,
            convert_shift: true,
            on_folder: true,
            on_background: true,
            icon: true,
        };
        apply(on).expect("register the menu");

        let file = format!("{CLASSES}\\SystemFileAssociations\\.jpg\\shell\\{VIEW_VERB}");
        assert_eq!(
            query(HKEY_CURRENT_USER, &file, None).as_deref(),
            Some("用 SeePic 浏览")
        );
        assert!(query(HKEY_CURRENT_USER, &file, Some("Icon")).is_some());
        assert!(query(HKEY_CURRENT_USER, &format!("{file}\\command"), None).is_some());
        assert!(
            value_exists(
                HKEY_CURRENT_USER,
                &format!("{CLASSES}\\SystemFileAssociations\\.jpg\\shell\\{CONVERT_VERB}"),
                Some("Extended")
            ),
            "the Shift-only flag must be present"
        );
        assert!(
            query(
                HKEY_CURRENT_USER,
                &format!("{CLASSES}\\Directory\\shell\\{VIEW_VERB}"),
                None
            )
            .is_some()
        );
        assert!(
            query(
                HKEY_CURRENT_USER,
                &format!("{CLASSES}\\Directory\\Background\\shell\\{CONVERT_VERB}"),
                None
            )
            .is_some()
        );

        apply(Config {
            enabled: false,
            ..on
        })
        .expect("remove the menu");
        assert!(
            query(HKEY_CURRENT_USER, &file, None).is_none(),
            "turning the feature off must remove the verb"
        );
    }
}
