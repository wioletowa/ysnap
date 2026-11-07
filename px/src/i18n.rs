//! Язык интерфейса — общий для шелла и его программ: русский (исходный), английский и
//! бразильский португальский. Пусто — по LC_ALL, LC_MESSAGES, LANG (первое непустое).

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Language {
    Russian,
    English,
    PortugueseBrazil,
}

static LANGUAGE: AtomicU8 = AtomicU8::new(Language::Russian as u8);

/// значение для настроек: «ru», «en», «pt-BR»; пусто — язык системы
pub fn normalize(tag: &str) -> &'static str {
    let tag = tag.trim().split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    match tag.split('-').next().unwrap_or("") {
        "ru" => "ru",
        "en" => "en",
        "pt" => "pt-BR",
        _ => "",
    }
}

fn resolve(explicit: &str, locales: &[&str]) -> Language {
    let source = if explicit.is_empty() {
        locales.iter().copied().find(|s| !s.is_empty()).unwrap_or("")
    } else {
        explicit
    };
    match normalize(source) {
        "ru" => Language::Russian,
        "pt-BR" => Language::PortugueseBrazil,
        _ => Language::English,
    }
}

pub fn set(lang: &str) {
    let locales: Vec<String> = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().map(|k| std::env::var(k).unwrap_or_default()).collect();
    let refs: Vec<&str> = locales.iter().map(String::as_str).collect();
    LANGUAGE.store(resolve(lang, &refs) as u8, Ordering::Relaxed);
}

pub fn en() -> bool {
    LANGUAGE.load(Ordering::Relaxed) == Language::English as u8
}

pub fn pt_br() -> bool {
    LANGUAGE.load(Ordering::Relaxed) == Language::PortugueseBrazil as u8
}

pub fn translated() -> bool {
    LANGUAGE.load(Ordering::Relaxed) != Language::Russian as u8
}

/// кнопка языка в настройках по кругу: авто, English, Русский, Português
pub fn next(lang: &str) -> &'static str {
    match normalize(lang) {
        "" => "en",
        "en" => "ru",
        "ru" => "pt-BR",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_aliases_and_settings_values() {
        for tag in ["pt-BR", "pt_BR", "pt_BR.UTF-8", "PT-br", "pt", "pt_PT.UTF-8"] {
            assert_eq!(normalize(tag), "pt-BR", "{tag}");
            assert_eq!(resolve(tag, &["ru_RU.UTF-8"]), Language::PortugueseBrazil);
        }
        assert_eq!(normalize("ru_RU.UTF-8"), "ru");
        assert_eq!(normalize("en-US"), "en");
        assert_eq!(normalize("invalid\ncommand"), "");
    }

    #[test]
    fn system_language_precedence_and_empty_variables() {
        assert_eq!(resolve("", &["", "", "pt_BR.UTF-8"]), Language::PortugueseBrazil);
        assert_eq!(resolve("", &["", "ru_RU.UTF-8", "pt_BR.UTF-8"]), Language::Russian);
        assert_eq!(resolve("", &["C.UTF-8", "pt_BR.UTF-8", "pt_BR.UTF-8"]), Language::English);
        assert_eq!(resolve("pt-BR", &["en_US.UTF-8"]), Language::PortugueseBrazil);
        assert_eq!(resolve("", &["de_DE.UTF-8"]), Language::English);
    }

    #[test]
    fn settings_cycle_includes_all_languages() {
        let mut lang = "";
        for expected in ["en", "ru", "pt-BR", ""] {
            lang = next(lang);
            assert_eq!(lang, expected);
        }
    }
}
