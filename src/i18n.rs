//! App-owned text follows the persisted language, including worker threads and
//! native menus. User filenames, game names and external diagnostics stay intact.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    De,
    En,
}

impl Language {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "de" => Some(Self::De),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn text(self, de: &'static str, en: &'static str) -> &'static str {
        match self { Self::De => de, Self::En => en }
    }
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    LANGUAGE.store(u8::from(language == Language::En), Ordering::Relaxed);
}

pub fn language() -> Language {
    if LANGUAGE.load(Ordering::Relaxed) == 1 { Language::En } else { Language::De }
}

pub fn text(de: &'static str, en: &'static str) -> &'static str {
    language().text(de, en)
}

/// Both translations are format literals, so the compiler checks their arguments.
#[macro_export]
macro_rules! tr_format {
    ($de:literal, $en:literal $(, $($args:tt)*)?) => {
        if $crate::i18n::language() == $crate::i18n::Language::En {
            format!($en $(, $($args)*)?)
        } else {
            format!($de $(, $($args)*)?)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids_are_stable_and_unsupported_values_are_rejected() {
        assert_eq!(Language::parse("de"), Some(Language::De));
        assert_eq!(Language::parse("en"), Some(Language::En));
        assert_eq!(Language::parse("fr"), None);
        assert_eq!(serde_json::to_string(&Language::En).unwrap(), "\"en\"");
        assert_eq!(Language::De.text("Öffnen", "Open"), "Öffnen");
        assert_eq!(Language::En.text("Öffnen", "Open"), "Open");
    }
}
