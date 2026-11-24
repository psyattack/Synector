mod en;
mod ru;
mod strings;

#[cfg(test)]
mod catalog;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub use strings::Strings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    Russian,
}

impl Language {
    pub fn label(&self) -> &'static str {
        match self {
            Self::English => "ENGLISH",
            Self::Russian => "РУССКИЙ",
        }
    }

    /// The interface language Windows is running in. The shipped catalogs are the only choices,
    /// so a Windows UI language without a catalog of its own (anything but Russian) falls back to
    /// the English one. A first launch adopts the result once; after that the section keeps the
    /// user's choice.
    pub fn system() -> Self {
        Self::from_primary_id(system_primary_language_id())
    }

    /// Maps a Windows primary language id — the low ten bits of a `LANGID` — onto a catalog.
    fn from_primary_id(primary: u16) -> Self {
        match primary {
            // `LANG_RUSSIAN` from winnt.h.
            0x19 => Self::Russian,
            _ => Self::English,
        }
    }
}

/// Primary language id of the current user's Windows UI language (`GetUserDefaultUILanguage`).
/// The call has no failure mode: it returns the system default when no user preference exists.
fn system_primary_language_id() -> u16 {
    /// The `LANGID` layout keeps the primary language in the low ten bits.
    const LANG_PRIMARY_MASK: u16 = 0x03FF;
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    langid & LANG_PRIMARY_MASK
}

pub fn get(lang: Language) -> &'static Strings {
    match lang {
        Language::English => &en::STRINGS,
        Language::Russian => &ru::STRINGS,
    }
}
