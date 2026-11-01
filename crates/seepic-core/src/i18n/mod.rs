//! UI language selection and the translation tables.
//!
//! English is the source of truth: UI code writes English literals and passes
//! them to [`tr`], which looks the same string up in the selected language's
//! table. A missing entry falls back to the English key, so a partial table
//! still shows a usable interface. Traditional Chinese falls back to the
//! Simplified Chinese table first, keeping the script consistent. Switchable at
//! runtime: every lookup reads the current language, so the settings window can
//! change it without restarting.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

mod de;
mod es;
mod fr;
mod ja;
mod ko;
mod pt;
mod ru;
mod zh_cn;
mod zh_tw;

/// A language the interface can be shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    Chinese = 0,
    English = 1,
    TraditionalChinese = 2,
    Japanese = 3,
    Korean = 4,
    French = 5,
    German = 6,
    Spanish = 7,
    Russian = 8,
    Portuguese = 9,
}

impl Language {
    /// The languages the picker offers.
    pub const ALL: [Language; 10] = [
        Language::Chinese,
        Language::TraditionalChinese,
        Language::English,
        Language::Japanese,
        Language::Korean,
        Language::French,
        Language::German,
        Language::Spanish,
        Language::Russian,
        Language::Portuguese,
    ];

    /// The value stored in `Settings::language`.
    pub fn code(self) -> &'static str {
        match self {
            Language::Chinese => "zh-CN",
            Language::TraditionalChinese => "zh-TW",
            Language::English => "en-US",
            Language::Japanese => "ja-JP",
            Language::Korean => "ko-KR",
            Language::French => "fr-FR",
            Language::German => "de-DE",
            Language::Spanish => "es-ES",
            Language::Russian => "ru-RU",
            Language::Portuguese => "pt-BR",
        }
    }

    /// The name shown in the language picker, written in that language.
    pub fn native_label(self) -> &'static str {
        match self {
            Language::Chinese => "简体中文",
            Language::TraditionalChinese => "繁體中文",
            Language::English => "English",
            Language::Japanese => "日本語",
            Language::Korean => "한국어",
            Language::French => "Français",
            Language::German => "Deutsch",
            Language::Spanish => "Español",
            Language::Russian => "Русский",
            Language::Portuguese => "Português",
        }
    }

    /// Parse a stored code. `None` means "follow the system".
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "zh" | "zh-CN" | "zh-Hans" | "zh-SG" => Some(Language::Chinese),
            "zh-TW" | "zh-HK" | "zh-Hant" => Some(Language::TraditionalChinese),
            "en" | "en-US" | "en-GB" => Some(Language::English),
            "ja" | "ja-JP" => Some(Language::Japanese),
            "ko" | "ko-KR" => Some(Language::Korean),
            "fr" | "fr-FR" => Some(Language::French),
            "de" | "de-DE" => Some(Language::German),
            "es" | "es-ES" => Some(Language::Spanish),
            "ru" | "ru-RU" => Some(Language::Russian),
            "pt" | "pt-BR" | "pt-PT" => Some(Language::Portuguese),
            _ => None,
        }
    }
}

/// The Windows display language, or a sensible fallback elsewhere.
pub fn system_language() -> Language {
    #[cfg(windows)]
    {
        // SAFETY: the function just reads the user's UI language id.
        let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        // The low 10 bits are the primary language id; the high bits are the
        // sublanguage, which tells Simplified from Traditional Chinese.
        return match id & 0x03ff {
            0x04 if matches!(id, 0x0404 | 0x0c04 | 0x1404) => Language::TraditionalChinese,
            0x04 => Language::Chinese,
            0x09 => Language::English,
            0x11 => Language::Japanese,
            0x12 => Language::Korean,
            0x0c => Language::French,
            0x07 => Language::German,
            0x0a => Language::Spanish,
            0x19 => Language::Russian,
            0x16 => Language::Portuguese,
            _ => Language::English,
        };
    }

    #[cfg(not(windows))]
    {
        let locale = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if locale.starts_with("zh-tw") || locale.starts_with("zh-hk") {
            Language::TraditionalChinese
        } else if locale.starts_with("zh") {
            Language::Chinese
        } else if locale.starts_with("ja") {
            Language::Japanese
        } else if locale.starts_with("ko") {
            Language::Korean
        } else if locale.starts_with("fr") {
            Language::French
        } else if locale.starts_with("de") {
            Language::German
        } else if locale.starts_with("es") {
            Language::Spanish
        } else if locale.starts_with("ru") {
            Language::Russian
        } else if locale.starts_with("pt") {
            Language::Portuguese
        } else {
            Language::English
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(u8::MAX);

/// The language in force right now, detecting the system on first use.
pub fn language() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Language::Chinese,
        1 => Language::English,
        2 => Language::TraditionalChinese,
        3 => Language::Japanese,
        4 => Language::Korean,
        5 => Language::French,
        6 => Language::German,
        7 => Language::Spanish,
        8 => Language::Russian,
        9 => Language::Portuguese,
        _ => {
            let detected = system_language();
            CURRENT.store(detected as u8, Ordering::Relaxed);
            detected
        }
    }
}

pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn is_english() -> bool {
    language() == Language::English
}

/// Translate an interface string.
///
/// The key is the English text. It is returned unchanged for English or when
/// the selected table has no entry. Traditional Chinese additionally falls back
/// to the complete Simplified Chinese table before giving up.
pub fn tr(key: &'static str) -> &'static str {
    let selected = language();
    if selected == Language::English {
        return key;
    }
    if let Some(value) = table(selected).and_then(|table| table.get(key)).copied() {
        return value;
    }
    if selected == Language::TraditionalChinese {
        if let Some(value) = table(Language::Chinese)
            .and_then(|table| table.get(key))
            .copied()
        {
            return value;
        }
    }
    key
}

/// Translate a string that is only known at runtime, when it has an entry.
pub fn tr_owned(text: &str) -> Option<&'static str> {
    let selected = language();
    if selected == Language::English {
        return None;
    }
    if let Some(value) = table(selected).and_then(|table| table.get(text)).copied() {
        return Some(value);
    }
    if selected == Language::TraditionalChinese {
        return table(Language::Chinese)
            .and_then(|table| table.get(text))
            .copied();
    }
    None
}

fn table(language: Language) -> Option<&'static HashMap<&'static str, &'static str>> {
    static ZH: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static ZH_TW: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static JA: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static KO: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static FR: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static DE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static ES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static RU: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    static PT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

    macro_rules! table {
        ($cell:ident, $source:expr) => {
            Some($cell.get_or_init(|| $source.iter().copied().collect()))
        };
    }

    match language {
        Language::English => None,
        Language::Chinese => table!(ZH, zh_cn::TRANSLATIONS),
        Language::TraditionalChinese => table!(ZH_TW, zh_tw::TRANSLATIONS),
        Language::Japanese => table!(JA, ja::TRANSLATIONS),
        Language::Korean => table!(KO, ko::TRANSLATIONS),
        Language::French => table!(FR, fr::TRANSLATIONS),
        Language::German => table!(DE, de::TRANSLATIONS),
        Language::Spanish => table!(ES, es::TRANSLATIONS),
        Language::Russian => table!(RU, ru::TRANSLATIONS),
        Language::Portuguese => table!(PT, pt::TRANSLATIONS),
    }
}

/// Translate a formatted message with the same argument list.
///
/// `format!` only accepts a literal, so a runtime translation cannot be passed
/// to it directly; this macro picks the complete format literal instead. The
/// English form is the key: it is used for English and every other language
/// whose table cannot parameterise a message, while Chinese takes the Chinese
/// form.
#[macro_export]
macro_rules! trf {
    ($en:literal, $zh:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::language() == $crate::i18n::Language::Chinese {
            format!($zh $(, $arg)*)
        } else {
            format!($en $(, $arg)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The current language is global, so tests that change it must not
    /// overlap: the harness runs tests on several threads.
    static LANGUAGE_LOCK: Mutex<()> = Mutex::new(());

    fn with_language<R>(selected: Language, run: impl FnOnce() -> R) -> R {
        let _guard = LANGUAGE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = language();
        set_language(selected);
        let result = run();
        set_language(previous);
        result
    }

    #[test]
    fn every_language_round_trips_through_its_code() {
        for language in Language::ALL {
            assert_eq!(
                Language::from_code(language.code()),
                Some(language),
                "{language:?} must parse back from {}",
                language.code()
            );
            assert!(!language.native_label().is_empty());
        }
        assert_eq!(Language::from_code("system"), None);
    }

    #[test]
    fn the_detected_system_language_is_one_of_the_offered_ones() {
        assert!(Language::ALL.contains(&system_language()));
    }

    #[test]
    fn translations_use_the_selected_table_and_fall_back_to_english() {
        with_language(Language::English, || {
            assert_eq!(tr("Open File"), "Open File");
        });

        with_language(Language::Japanese, || {
            assert_eq!(tr("Open File"), "ファイルを開く");
            // A key the Japanese table lacks stays in English.
            let missing = "A string that no table translates";
            assert_eq!(tr(missing), missing);
        });

        with_language(Language::Chinese, || {
            assert_eq!(tr("Open File"), "打开文件");
        });
    }

    #[test]
    fn traditional_chinese_uses_its_table_then_english() {
        with_language(Language::TraditionalChinese, || {
            assert_eq!(tr("Open File"), "開啟檔案");

            // A key no table has stays in its English source form.
            let missing = "A string that no table translates";
            assert_eq!(tr(missing), missing);
        });
    }

    #[test]
    fn formatted_messages_follow_the_language() {
        with_language(Language::Chinese, || {
            assert_eq!(pages_message(), "3 页");
        });
        with_language(Language::Japanese, || {
            assert_eq!(pages_message(), "3 pages");
        });
    }

    fn pages_message() -> String {
        crate::trf!("{} pages", "{} 页", 3)
    }
}
