//! The tray menu's Language submenu: recent languages and the model choice
//! when switching language from it (LANG-8..10).

use crate::languages::LanguageSetting;
use crate::models::{Catalog, ModelId};
use crate::routing::{routes, supports};

/// Most languages the submenu remembers (LANG-8).
pub const RECENT_LIMIT: usize = 5;

/// Title of the item after the separator; opens Speech settings (LANG-9).
pub const OTHER_LANGUAGE_TITLE: &str = "Other Language…";

/// `current` first, then the stored codes in recency order, without
/// duplicates or codes no longer offered, capped at [`RECENT_LIMIT`]
/// (LANG-8). `stored` is the comma-separated `recentLanguages` setting.
pub fn recent_languages(current: LanguageSetting, stored: &str) -> Vec<LanguageSetting> {
    let mut recents = vec![current];
    for language in stored.split(',').filter_map(LanguageSetting::parse) {
        if recents.len() == RECENT_LIMIT {
            break;
        }
        if !recents.contains(&language) {
            recents.push(language);
        }
    }
    recents
}

/// Encodes recents as the comma-separated setting value.
pub fn encode_recent(languages: &[LanguageSetting]) -> String {
    languages
        .iter()
        .map(|l| l.code())
        .collect::<Vec<_>>()
        .join(",")
}

/// One submenu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItem {
    /// The language the item selects.
    pub language: LanguageSetting,
    /// Display name ("Automatic" for `auto`).
    pub title: &'static str,
    /// Checkmark on the current language.
    pub is_selected: bool,
}

/// Items for the recent languages, sorted by display name so the order does
/// not shuffle as recency changes (LANG-9).
pub fn menu_items(recents: &[LanguageSetting], current: LanguageSetting) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = recents
        .iter()
        .map(|&language| MenuItem {
            language,
            title: language.display_name(),
            is_selected: language == current,
        })
        .collect();
    items.sort_by_key(|item| item.title.to_lowercase());
    items
}

/// Subtitle of the Language menu item: the current language's name (LANG-9).
pub fn subtitle(current: LanguageSetting) -> &'static str {
    current.display_name()
}

/// The model to use after switching language from the menu (LANG-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuModelChoice {
    /// Select this model. It may be the current one; the attempt still
    /// clears any pending "Download & Use" (DL-8).
    Use(ModelId),
    /// No downloaded model transcribes the language: open Speech settings.
    OpenSpeechSettings,
}

/// Picks the model for `language` (LANG-10): the current model if it is
/// downloaded and supports the language; else the first downloaded model
/// among the language's routes, recommended first; else open Speech settings.
/// `current_model` is the raw `selectedModel` setting; `downloaded` comes from
/// [`Catalog::downloaded`].
pub fn model_for_language(
    catalog: &Catalog,
    language: LanguageSetting,
    current_model: &str,
    downloaded: &[ModelId],
) -> MenuModelChoice {
    if let Some(current) = catalog.resolve(current_model)
        && downloaded.contains(&current)
        && supports(catalog, language, current)
    {
        return MenuModelChoice::Use(current);
    }
    let mut candidates = routes(catalog, language);
    // stable sort: recommended first, display order otherwise
    candidates.sort_by_key(|r| !r.is_recommended);
    candidates
        .into_iter()
        .find(|r| downloaded.contains(&r.model))
        .map_or(MenuModelChoice::OpenSpeechSettings, |r| {
            MenuModelChoice::Use(r.model)
        })
}

/// What to do when the user picks a language from the menu (LANG-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuLanguageSwitch {
    /// Save as `selectedLanguage`.
    pub language: LanguageSetting,
    /// New `recentLanguages` value.
    pub recent_languages: String,
    /// Model to select, or open Speech settings.
    pub model: MenuModelChoice,
}

/// Plans a menu language switch. `None` while a session runs: the items are
/// disabled, since switching would change the model under the session.
pub fn switch_language_from_menu(
    catalog: &Catalog,
    language: LanguageSetting,
    session_idle: bool,
    stored_recent: &str,
    current_model: &str,
    downloaded: &[ModelId],
) -> Option<MenuLanguageSwitch> {
    if !session_idle {
        return None;
    }
    Some(MenuLanguageSwitch {
        language,
        recent_languages: encode_recent(&recent_languages(language, stored_recent)),
        model: model_for_language(catalog, language, current_model, downloaded),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::{with_system, without_system};

    fn lang(code: &str) -> LanguageSetting {
        LanguageSetting::parse(code).unwrap_or_else(|| panic!("{code} not offered"))
    }

    fn codes(languages: &[LanguageSetting]) -> Vec<&'static str> {
        languages.iter().map(|l| l.code()).collect()
    }

    #[test]
    fn recent_languages_lang_8() {
        struct Case {
            name: &'static str,
            current: &'static str,
            stored: &'static str,
            want: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "nothing stored",
                current: "en",
                stored: "",
                want: vec!["en"],
            },
            Case {
                name: "current moves to the front",
                current: "pt",
                stored: "en,pt,fr",
                want: vec!["pt", "en", "fr"],
            },
            Case {
                name: "capped",
                current: "ja",
                stored: "en,pt,fr,de,es",
                want: vec!["ja", "en", "pt", "fr", "de"],
            },
            Case {
                name: "unknown codes dropped",
                current: "en",
                stored: "xx,pt,,auto",
                want: vec!["en", "pt", "auto"],
            },
            Case {
                name: "duplicates dropped before the cap",
                current: "en",
                stored: "en,de,de,fr,es,it,ja",
                want: vec!["en", "de", "fr", "es", "it"],
            },
        ];
        for tc in cases {
            let got = recent_languages(lang(tc.current), tc.stored);
            assert_eq!(codes(&got), tc.want, "{}", tc.name);
        }
        assert_eq!(
            encode_recent(&[lang("pt"), LanguageSetting::Auto]),
            "pt,auto"
        );
    }

    #[test]
    fn items_sorted_by_name_with_checkmark_lang_9() {
        struct Case {
            name: &'static str,
            recents: Vec<&'static str>,
            current: &'static str,
            want: Vec<(&'static str, &'static str, bool)>,
        }
        let cases = [
            Case {
                name: "sorted with auto first and current checked",
                recents: vec!["pt", "auto", "en"],
                current: "pt",
                want: vec![
                    ("auto", "Automatic", false),
                    ("en", "English", false),
                    ("pt", "Portuguese", true),
                ],
            },
            Case {
                name: "order ignores recency",
                recents: vec!["sv", "de", "ar"],
                current: "de",
                want: vec![
                    ("ar", "Arabic", false),
                    ("de", "German", true),
                    ("sv", "Swedish", false),
                ],
            },
        ];
        for tc in cases {
            let recents: Vec<LanguageSetting> = tc.recents.iter().map(|c| lang(c)).collect();
            let got: Vec<(&str, &str, bool)> = menu_items(&recents, lang(tc.current))
                .into_iter()
                .map(|i| (i.language.code(), i.title, i.is_selected))
                .collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
        assert_eq!(subtitle(LanguageSetting::Auto), "Automatic");
        assert_eq!(subtitle(lang("ja")), "Japanese");
    }

    #[test]
    fn model_for_language_lang_10() {
        use ModelId::*;
        struct Case {
            name: &'static str,
            system: bool,
            language: &'static str,
            current: &'static str,
            on_disk: Vec<ModelId>,
            want: MenuModelChoice,
        }
        let cases = [
            Case {
                name: "current model covers it",
                system: true,
                language: "en",
                current: "parakeet-ultra",
                on_disk: vec![ParakeetTdtV2, ParakeetUltra],
                want: MenuModelChoice::Use(ParakeetUltra),
            },
            Case {
                name: "switches to the recommended model",
                system: true,
                language: "pt",
                current: "parakeet-tdt-v2",
                on_disk: vec![ParakeetTdtV2, ParakeetUltra],
                want: MenuModelChoice::Use(ParakeetUltra),
            },
            Case {
                name: "falls back to a downloaded alternative",
                system: true,
                language: "pt",
                current: "parakeet-tdt-v2",
                on_disk: vec![ParakeetTdtV2],
                want: MenuModelChoice::Use(SystemSpeech),
            },
            Case {
                name: "nothing downloaded can",
                system: false,
                language: "ja",
                current: "parakeet-tdt-v2",
                on_disk: vec![ParakeetTdtV2],
                want: MenuModelChoice::OpenSpeechSettings,
            },
            Case {
                name: "current not downloaded",
                system: false,
                language: "en",
                current: "parakeet-ultra",
                on_disk: vec![ParakeetTdtV2],
                want: MenuModelChoice::Use(ParakeetTdtV2),
            },
            Case {
                name: "auto picks the recommended downloaded model",
                system: true,
                language: "auto",
                current: "parakeet-ja",
                on_disk: vec![ParakeetUnified],
                want: MenuModelChoice::Use(SystemSpeech),
            },
            Case {
                name: "auto keeps any downloaded current model",
                system: true,
                language: "auto",
                current: "parakeet-ja",
                on_disk: vec![ParakeetJapanese],
                want: MenuModelChoice::Use(ParakeetJapanese),
            },
            Case {
                name: "unknown current model is not kept",
                system: false,
                language: "en",
                current: "greendale-human-being",
                on_disk: vec![ParakeetUnified],
                want: MenuModelChoice::Use(ParakeetUnified),
            },
            Case {
                name: "LANG-5 system-only language without system engine",
                system: false,
                language: "ko",
                current: "parakeet-ultra",
                on_disk: vec![ParakeetUltra, ParakeetTdtV2],
                want: MenuModelChoice::OpenSpeechSettings,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            let downloaded = catalog.downloaded(&tc.on_disk);
            assert_eq!(
                model_for_language(&catalog, lang(tc.language), tc.current, &downloaded),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn switch_from_menu_only_when_idle_lang_10() {
        let catalog = with_system();
        let downloaded = catalog.downloaded(&[]);
        struct Case {
            name: &'static str,
            idle: bool,
            want: Option<MenuLanguageSwitch>,
        }
        let cases = [
            Case {
                name: "idle saves language, recents, and model",
                idle: true,
                want: Some(MenuLanguageSwitch {
                    language: lang("de"),
                    recent_languages: "de,en".to_string(),
                    model: MenuModelChoice::Use(ModelId::SystemSpeech),
                }),
            },
            Case {
                name: "busy session ignores the choice",
                idle: false,
                want: None,
            },
        ];
        for tc in cases {
            let got = switch_language_from_menu(
                &catalog,
                lang("de"),
                tc.idle,
                "en,de",
                "parakeet-tdt-v2",
                &downloaded,
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }
}
