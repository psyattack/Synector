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
}

pub fn get(lang: Language) -> &'static Strings {
    match lang {
        Language::English => &en::STRINGS,
        Language::Russian => &ru::STRINGS,
    }
}
