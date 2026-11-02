//! Which model for which language (LANG-4..7) and the Speech page
//! compatibility note.

use crate::languages::{Language, LanguageSetting, display_name_for_code};
use crate::models::{Catalog, ModelFamily, ModelId, PARAKEET_ULTRA_LANGUAGES};

/// One model offered for a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// The model.
    pub model: ModelId,
    /// Whether this is the language's recommended model. At most one route is.
    pub is_recommended: bool,
}

const fn route(model: ModelId, is_recommended: bool) -> Route {
    Route {
        model,
        is_recommended,
    }
}

/// Models for `language` in display order.
///
/// Without a system engine, the system routes disappear and the first
/// remaining Parakeet route becomes the recommendation (LANG-5). A language
/// only the system engine covers then has no routes.
pub fn routes(catalog: &Catalog, language: LanguageSetting) -> Vec<Route> {
    let mut routes = available_routes(catalog, language);
    routes.retain(|route| catalog.contains(route.model));
    if !routes.iter().any(|route| route.is_recommended)
        && let Some(first) = routes.first_mut()
    {
        first.is_recommended = true;
    }
    routes
}

fn available_routes(catalog: &Catalog, language: LanguageSetting) -> Vec<Route> {
    let system = catalog.model(ModelId::SystemSpeech);
    let code = match language {
        LanguageSetting::Auto => {
            let mut parakeet = vec![
                route(ModelId::ParakeetUltra, system.is_none()),
                route(ModelId::ParakeetTdtV2, false),
                route(ModelId::ParakeetUnified, false),
            ];
            if system.is_some() {
                parakeet.insert(0, route(ModelId::SystemSpeech, true));
            }
            return parakeet;
        }
        LanguageSetting::Specific(language) => language.code(),
    };
    let system_supports = system.is_some_and(|s| s.has_language(code));
    let system_route = |is_recommended| {
        system_supports
            .then_some(route(ModelId::SystemSpeech, is_recommended))
            .into_iter()
    };

    let parakeet: Vec<Route> = match code {
        "en" => vec![
            route(ModelId::ParakeetTdtV2, true),
            route(ModelId::ParakeetUltra, false),
            route(ModelId::ParakeetUnified, false),
        ],
        _ if PARAKEET_ULTRA_LANGUAGES.contains(&code) => vec![route(ModelId::ParakeetUltra, true)],
        "ja" => vec![route(ModelId::ParakeetJapanese, true)],
        _ => return system_route(true).collect(),
    };
    parakeet.into_iter().chain(system_route(false)).collect()
}

/// The recommended model for `language` (LANG-5), or `None` when no model in
/// this catalog transcribes it (only possible without a system engine, or
/// when the system engine does not report the language).
pub fn recommended_model(catalog: &Catalog, language: LanguageSetting) -> Option<ModelId> {
    routes(catalog, language)
        .into_iter()
        .find(|r| r.is_recommended)
        .map(|r| r.model)
}

/// Whether `model` can transcribe `language` (LANG-7). `auto` is supported by
/// every model in the catalog; models not in the catalog support nothing.
pub fn supports(catalog: &Catalog, language: LanguageSetting, model: ModelId) -> bool {
    let Some(info) = catalog.model(model) else {
        return false;
    };
    match language {
        LanguageSetting::Auto => true,
        LanguageSetting::Specific(language) => info.has_language(language.code()),
    }
}

/// Default model (LANG-6): the recommended model for the default language.
///
/// When the default language has no recommendation (a system-only language
/// on a shell without a system engine), falls back to the recommendation for
/// `auto`, which always exists.
pub fn default_model(catalog: &Catalog, default_language: LanguageSetting) -> ModelId {
    recommended_model(catalog, default_language)
        .or_else(|| recommended_model(catalog, LanguageSetting::Auto))
        .unwrap_or(ModelId::ParakeetUltra)
}

/// What a model does with the selected language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    /// The model transcribes the language.
    Supported,
    /// The model cannot transcribe the language. `recommended` is the model
    /// that can, or `None` when no model in the catalog can (LANG-5).
    Unsupported {
        /// The language's recommended model.
        recommended: Option<ModelId>,
    },
    /// `auto` with a multilingual model that detects the spoken language.
    DetectsLanguage,
    /// `auto` with a single-language model; always transcribes this language.
    AssumesLanguage(Language),
    /// `auto` with the system engine, which uses the OS language (SYS-2).
    /// `None` when the OS language is unknown.
    UsesSystemLanguage(Option<String>),
}

/// Resolves what `model` does with `language`. `system_language` is the OS
/// locale's language code. `None` for models not in the catalog.
pub fn compatibility(
    catalog: &Catalog,
    language: LanguageSetting,
    model: ModelId,
    system_language: Option<&str>,
) -> Option<Compatibility> {
    let info = catalog.model(model)?;
    if language != LanguageSetting::Auto {
        if supports(catalog, language, model) {
            return Some(Compatibility::Supported);
        }
        return Some(Compatibility::Unsupported {
            recommended: recommended_model(catalog, language),
        });
    }
    if info.family == ModelFamily::SystemEngine {
        return Some(Compatibility::UsesSystemLanguage(
            system_language.map(str::to_string),
        ));
    }
    match info.language_codes().as_slice() {
        [only] => Some(Language::from_code(only).map_or(
            Compatibility::DetectsLanguage,
            Compatibility::AssumesLanguage,
        )),
        _ => Some(Compatibility::DetectsLanguage),
    }
}

/// Availability of a model, as the model manager reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelAvailability {
    /// Downloaded or built-in.
    Downloaded,
    /// A download is running; `progress` in `[0, 1]`.
    Downloading {
        /// Download fraction.
        progress: f64,
    },
    /// Not on disk and not downloading.
    NotDownloaded,
}

/// The fix button under an "unsupported" compatibility note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteAction {
    /// The recommended model the button acts on.
    pub model: ModelId,
    /// What pressing it does.
    pub kind: NoteActionKind,
    /// Button label.
    pub label: String,
    /// False while the model downloads.
    pub enabled: bool,
}

/// What the compatibility note button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteActionKind {
    /// Select the downloaded model (DL-7).
    Use,
    /// Start "Download & Use" (DL-8).
    DownloadAndUse,
    /// Disabled while the download runs.
    Downloading,
}

/// The note shown under the Speech page language picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityNote {
    /// The parsed compatibility.
    pub compatibility: Compatibility,
    /// Caption text.
    pub text: String,
    /// True for the orange "can't transcribe" warning.
    pub is_warning: bool,
    /// Fix button, present only for an unsupported pair with a recommendation.
    pub action: Option<NoteAction>,
}

/// Builds the compatibility note (WIN-18).
/// `availability` reports the recommended model's download state.
pub fn compatibility_note(
    catalog: &Catalog,
    language: LanguageSetting,
    model: ModelId,
    system_language: Option<&str>,
    availability: impl Fn(ModelId) -> ModelAvailability,
) -> Option<CompatibilityNote> {
    let compatibility = compatibility(catalog, language, model, system_language)?;
    let model_name = catalog.name(model);
    let language_name = language.display_name();
    let (text, action) = match &compatibility {
        Compatibility::Unsupported { recommended } => (
            format!(
                "{model_name} can't transcribe {language_name}, so dictation will come out garbled."
            ),
            recommended.map(|r| note_action(catalog, r, availability(r))),
        ),
        Compatibility::Supported => (format!("{model_name} supports {language_name}."), None),
        Compatibility::DetectsLanguage => (
            format!("{model_name} detects which of its supported languages you're speaking."),
            None,
        ),
        Compatibility::AssumesLanguage(assumed) => (
            format!(
                "{model_name} can't detect the language. It always transcribes {}.",
                assumed.name()
            ),
            None,
        ),
        Compatibility::UsesSystemLanguage(Some(code)) => (
            format!(
                "{model_name} can't detect the language. It uses the system language, {}.",
                display_name_for_code(code)
            ),
            None,
        ),
        Compatibility::UsesSystemLanguage(None) => (
            format!("{model_name} can't detect the language. It uses the system language."),
            None,
        ),
    };
    Some(CompatibilityNote {
        is_warning: matches!(compatibility, Compatibility::Unsupported { .. }),
        compatibility,
        text,
        action,
    })
}

fn note_action(catalog: &Catalog, model: ModelId, availability: ModelAvailability) -> NoteAction {
    let name = catalog.name(model);
    let (kind, label, enabled) = match availability {
        ModelAvailability::Downloaded => (NoteActionKind::Use, format!("Use {name}"), true),
        ModelAvailability::NotDownloaded => {
            let size = catalog.model(model).map_or("", |m| m.size_hint);
            (
                NoteActionKind::DownloadAndUse,
                format!("Download & Use {name} ({size})"),
                true,
            )
        }
        ModelAvailability::Downloading { progress } => (
            NoteActionKind::Downloading,
            format!("Downloading {name}… {}%", percent(progress)),
            false,
        ),
    };
    NoteAction {
        model,
        kind,
        label,
        enabled,
    }
}

/// Whole percent for a progress fraction, truncated and clamped to 0..=100.
pub fn percent(progress: f64) -> u8 {
    // the clamp keeps the cast in range; NaN casts to 0
    (progress.clamp(0.0, 1.0) * 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::{with_system, without_system};

    const V2: ModelId = ModelId::ParakeetTdtV2;
    const ULTRA: ModelId = ModelId::ParakeetUltra;
    const JA: ModelId = ModelId::ParakeetJapanese;
    const UNIFIED: ModelId = ModelId::ParakeetUnified;
    const SYS: ModelId = ModelId::SystemSpeech;

    fn lang(code: &str) -> LanguageSetting {
        LanguageSetting::parse(code).unwrap_or_else(|| panic!("{code} not offered"))
    }

    fn l(code: &str) -> Language {
        Language::from_code(code).unwrap_or_else(|| panic!("{code} not offered"))
    }

    fn all_settings() -> Vec<LanguageSetting> {
        std::iter::once(LanguageSetting::Auto)
            .chain(Language::all().map(LanguageSetting::Specific))
            .collect()
    }

    #[test]
    fn routes_table() {
        struct Case {
            name: &'static str,
            system: bool,
            language: &'static str,
            want: Vec<(ModelId, bool)>,
        }
        let cases = [
            Case {
                name: "auto recommends system speech",
                system: true,
                language: "auto",
                want: vec![(SYS, true), (ULTRA, false), (V2, false), (UNIFIED, false)],
            },
            Case {
                name: "english recommends parakeet v2",
                system: true,
                language: "en",
                want: vec![(V2, true), (ULTRA, false), (UNIFIED, false), (SYS, false)],
            },
            Case {
                name: "german recommends ultra then system",
                system: true,
                language: "de",
                want: vec![(ULTRA, true), (SYS, false)],
            },
            Case {
                name: "russian cyrillic recommends ultra",
                system: true,
                language: "ru",
                want: vec![(ULTRA, true), (SYS, false)],
            },
            Case {
                name: "greek recommends ultra",
                system: true,
                language: "el",
                want: vec![(ULTRA, true), (SYS, false)],
            },
            Case {
                name: "belarusian has no system route",
                system: true,
                language: "be",
                want: vec![(ULTRA, true)],
            },
            Case {
                name: "japanese recommends parakeet japanese",
                system: true,
                language: "ja",
                want: vec![(JA, true), (SYS, false)],
            },
            Case {
                name: "chinese only offers system speech",
                system: true,
                language: "zh",
                want: vec![(SYS, true)],
            },
            Case {
                name: "LANG-5 auto without system recommends ultra",
                system: false,
                language: "auto",
                want: vec![(ULTRA, true), (V2, false), (UNIFIED, false)],
            },
            Case {
                name: "LANG-5 english without system",
                system: false,
                language: "en",
                want: vec![(V2, true), (ULTRA, false), (UNIFIED, false)],
            },
            Case {
                name: "LANG-5 german without system",
                system: false,
                language: "de",
                want: vec![(ULTRA, true)],
            },
            Case {
                name: "LANG-5 japanese without system",
                system: false,
                language: "ja",
                want: vec![(JA, true)],
            },
            Case {
                name: "LANG-5 system-only language without system has no routes",
                system: false,
                language: "ko",
                want: vec![],
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            let got: Vec<(ModelId, bool)> = routes(&catalog, lang(tc.language))
                .into_iter()
                .map(|r| (r.model, r.is_recommended))
                .collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn system_routes_follow_shell_languages_sys_1() {
        // the shell, not Jabber's list, decides what the system engine covers
        let mut info = crate::models::tests::mac_system_speech();
        info.languages = vec!["be".to_string()];
        let catalog = Catalog::new(Some(info));
        struct Case {
            name: &'static str,
            language: &'static str,
            want: Vec<(ModelId, bool)>,
        }
        let cases = [
            Case {
                name: "reported language gains a system route",
                language: "be",
                want: vec![(ULTRA, true), (SYS, false)],
            },
            Case {
                name: "unreported language loses it",
                language: "de",
                want: vec![(ULTRA, true)],
            },
            Case {
                name: "unreported system-only language has no routes",
                language: "ar",
                want: vec![],
            },
        ];
        for tc in cases {
            let got: Vec<(ModelId, bool)> = routes(&catalog, lang(tc.language))
                .into_iter()
                .map(|r| (r.model, r.is_recommended))
                .collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn at_most_one_recommended_route() {
        for catalog in [with_system(), without_system()] {
            for language in all_settings() {
                let count = routes(&catalog, language)
                    .iter()
                    .filter(|r| r.is_recommended)
                    .count();
                assert!(count <= 1, "{language}: {count} recommended");
            }
        }
    }

    #[test]
    fn recommended_model_lang_5() {
        struct Case {
            name: &'static str,
            system: bool,
            language: &'static str,
            want: Option<ModelId>,
        }
        let cases = [
            Case {
                name: "chinese recommends system speech",
                system: true,
                language: "zh",
                want: Some(SYS),
            },
            Case {
                name: "portuguese recommends ultra",
                system: true,
                language: "pt",
                want: Some(ULTRA),
            },
            Case {
                name: "auto with system",
                system: true,
                language: "auto",
                want: Some(SYS),
            },
            Case {
                name: "auto without system recommends ultra",
                system: false,
                language: "auto",
                want: Some(ULTRA),
            },
            Case {
                name: "unsupported language without system has no recommendation",
                system: false,
                language: "zh",
                want: None,
            },
            Case {
                name: "japanese without system",
                system: false,
                language: "ja",
                want: Some(JA),
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            assert_eq!(
                recommended_model(&catalog, lang(tc.language)),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn every_ultra_language_recommends_ultra() {
        // guards the pairing: a language ultra handles never recommends system speech
        for catalog in [with_system(), without_system()] {
            for l in Language::all() {
                if l.code() == "en" || !PARAKEET_ULTRA_LANGUAGES.contains(&l.code()) {
                    continue;
                }
                assert_eq!(
                    recommended_model(&catalog, LanguageSetting::Specific(l)),
                    Some(ULTRA),
                    "{}",
                    l.code()
                );
            }
        }
    }

    #[test]
    fn recommended_model_supports_its_language() {
        // a recommendation that cannot transcribe would load a useless model
        for catalog in [with_system(), without_system()] {
            for language in all_settings() {
                if let Some(model) = recommended_model(&catalog, language) {
                    assert!(supports(&catalog, language, model), "{language} on {model}");
                }
            }
        }
    }

    #[test]
    fn every_language_has_a_model_with_system_speech() {
        let catalog = with_system();
        for language in all_settings() {
            assert!(
                recommended_model(&catalog, language).is_some(),
                "{language} has no model"
            );
        }
    }

    #[test]
    fn system_only_languages_lack_a_model_without_system_engine() {
        // the ten system-only languages
        let catalog = without_system();
        let none: Vec<&str> = Language::all()
            .filter(|l| recommended_model(&catalog, LanguageSetting::Specific(*l)).is_none())
            .map(Language::code)
            .collect();
        assert_eq!(
            none,
            ["ar", "yue", "zh", "hi", "id", "ko", "ms", "th", "tr", "vi"]
        );
    }

    #[test]
    fn system_speech_only_where_it_transcribes() {
        let catalog = with_system();
        struct Case {
            name: &'static str,
            languages: &'static [&'static str],
            want_system: bool,
            want_only_system: bool,
        }
        let cases = [
            Case {
                name: "parakeet languages apple has no model for",
                languages: &["be", "bg", "bs", "et", "lt", "lv", "mt", "sl", "sr"],
                want_system: false,
                want_only_system: false,
            },
            Case {
                name: "only the system engine covers these",
                languages: &["ar", "id", "ms", "th", "tr", "vi"],
                want_system: true,
                want_only_system: true,
            },
        ];
        for tc in cases {
            for code in tc.languages {
                let models: Vec<ModelId> = routes(&catalog, lang(code))
                    .into_iter()
                    .map(|r| r.model)
                    .collect();
                assert_eq!(models.contains(&SYS), tc.want_system, "{}: {code}", tc.name);
                assert_eq!(models == [SYS], tc.want_only_system, "{}: {code}", tc.name);
            }
        }
    }

    #[test]
    fn supports_lang_7() {
        struct Case {
            name: &'static str,
            system: bool,
            language: &'static str,
            model: ModelId,
            want: bool,
        }
        let cases = [
            Case {
                name: "ultra english",
                system: true,
                language: "en",
                model: ULTRA,
                want: true,
            },
            Case {
                name: "ultra polish",
                system: true,
                language: "pl",
                model: ULTRA,
                want: true,
            },
            Case {
                name: "ultra japanese",
                system: true,
                language: "ja",
                model: ULTRA,
                want: false,
            },
            Case {
                name: "ultra chinese",
                system: true,
                language: "zh",
                model: ULTRA,
                want: false,
            },
            Case {
                name: "ultra hindi",
                system: true,
                language: "hi",
                model: ULTRA,
                want: false,
            },
            Case {
                name: "v2 english",
                system: true,
                language: "en",
                model: V2,
                want: true,
            },
            Case {
                name: "v2 german",
                system: true,
                language: "de",
                model: V2,
                want: false,
            },
            Case {
                name: "unified english",
                system: true,
                language: "en",
                model: UNIFIED,
                want: true,
            },
            Case {
                name: "unified german",
                system: true,
                language: "de",
                model: UNIFIED,
                want: false,
            },
            Case {
                name: "japanese model rejects english",
                system: true,
                language: "en",
                model: JA,
                want: false,
            },
            Case {
                name: "system speech korean",
                system: true,
                language: "ko",
                model: SYS,
                want: true,
            },
            Case {
                name: "system speech absent supports nothing",
                system: false,
                language: "ko",
                model: SYS,
                want: false,
            },
            Case {
                name: "system speech absent rejects auto",
                system: false,
                language: "auto",
                model: SYS,
                want: false,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            assert_eq!(
                supports(&catalog, lang(tc.language), tc.model),
                tc.want,
                "{}",
                tc.name
            );
        }
        let catalog = with_system();
        for m in catalog.models() {
            assert!(
                supports(&catalog, LanguageSetting::Auto, m.id),
                "auto on {}",
                m.id
            );
        }
    }

    #[test]
    fn default_model_lang_6() {
        struct Case {
            name: &'static str,
            system: bool,
            default_language: &'static str,
            want: ModelId,
        }
        let cases = [
            Case {
                name: "english",
                system: true,
                default_language: "en",
                want: V2,
            },
            Case {
                name: "auto with system",
                system: true,
                default_language: "auto",
                want: SYS,
            },
            Case {
                name: "korean with system",
                system: true,
                default_language: "ko",
                want: SYS,
            },
            Case {
                name: "auto without system",
                system: false,
                default_language: "auto",
                want: ULTRA,
            },
            Case {
                name: "korean without system falls back to the auto recommendation",
                system: false,
                default_language: "ko",
                want: ULTRA,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            assert_eq!(
                default_model(&catalog, lang(tc.default_language)),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn compatibility_describes_what_the_model_will_do() {
        use Compatibility::*;
        struct Case {
            name: &'static str,
            system: bool,
            language: &'static str,
            model: ModelId,
            system_language: Option<&'static str>,
            want: Option<Compatibility>,
        }
        let case = |name, language, model, system_language, want| Case {
            name,
            system: true,
            language,
            model,
            system_language,
            want,
        };
        let pt = Some("pt");
        let cases = [
            case("english on v2 is supported", "en", V2, pt, Some(Supported)),
            case(
                "english on ultra is supported",
                "en",
                ULTRA,
                pt,
                Some(Supported),
            ),
            case(
                "english on unified is supported",
                "en",
                UNIFIED,
                pt,
                Some(Supported),
            ),
            case(
                "english on system is supported",
                "en",
                SYS,
                pt,
                Some(Supported),
            ),
            case(
                "english on japanese recommends v2",
                "en",
                JA,
                pt,
                Some(Unsupported {
                    recommended: Some(V2),
                }),
            ),
            case(
                "german on v2 recommends ultra",
                "de",
                V2,
                pt,
                Some(Unsupported {
                    recommended: Some(ULTRA),
                }),
            ),
            case(
                "german on unified recommends ultra",
                "de",
                UNIFIED,
                pt,
                Some(Unsupported {
                    recommended: Some(ULTRA),
                }),
            ),
            case(
                "german on japanese recommends ultra",
                "de",
                JA,
                pt,
                Some(Unsupported {
                    recommended: Some(ULTRA),
                }),
            ),
            case(
                "german on ultra is supported",
                "de",
                ULTRA,
                pt,
                Some(Supported),
            ),
            case(
                "german on system is supported",
                "de",
                SYS,
                pt,
                Some(Supported),
            ),
            case(
                "japanese on v2 recommends japanese",
                "ja",
                V2,
                pt,
                Some(Unsupported {
                    recommended: Some(JA),
                }),
            ),
            case(
                "japanese on ultra recommends japanese",
                "ja",
                ULTRA,
                pt,
                Some(Unsupported {
                    recommended: Some(JA),
                }),
            ),
            case(
                "japanese on japanese is supported",
                "ja",
                JA,
                pt,
                Some(Supported),
            ),
            case(
                "korean on v2 recommends system",
                "ko",
                V2,
                pt,
                Some(Unsupported {
                    recommended: Some(SYS),
                }),
            ),
            case(
                "korean on ultra recommends system",
                "ko",
                ULTRA,
                pt,
                Some(Unsupported {
                    recommended: Some(SYS),
                }),
            ),
            case(
                "korean on japanese recommends system",
                "ko",
                JA,
                pt,
                Some(Unsupported {
                    recommended: Some(SYS),
                }),
            ),
            case(
                "korean on system is supported",
                "ko",
                SYS,
                pt,
                Some(Supported),
            ),
            case(
                "belarusian on system recommends ultra",
                "be",
                SYS,
                pt,
                Some(Unsupported {
                    recommended: Some(ULTRA),
                }),
            ),
            case(
                "auto on ultra detects",
                "auto",
                ULTRA,
                pt,
                Some(DetectsLanguage),
            ),
            case(
                "auto on v2 assumes english",
                "auto",
                V2,
                pt,
                Some(AssumesLanguage(l("en"))),
            ),
            case(
                "auto on unified assumes english",
                "auto",
                UNIFIED,
                pt,
                Some(AssumesLanguage(l("en"))),
            ),
            case(
                "auto on japanese assumes japanese",
                "auto",
                JA,
                pt,
                Some(AssumesLanguage(l("ja"))),
            ),
            case(
                "auto on system uses the system language",
                "auto",
                SYS,
                pt,
                Some(UsesSystemLanguage(Some("pt".to_string()))),
            ),
            case(
                "auto on system without a system language",
                "auto",
                SYS,
                None,
                Some(UsesSystemLanguage(None)),
            ),
            case(
                "auto on v2 ignores the system language",
                "auto",
                V2,
                Some("de"),
                Some(AssumesLanguage(l("en"))),
            ),
            Case {
                name: "LANG-5 korean on ultra without system has no recommendation",
                system: false,
                language: "ko",
                model: ULTRA,
                system_language: pt,
                want: Some(Unsupported { recommended: None }),
            },
            Case {
                name: "system speech absent has no answer",
                system: false,
                language: "en",
                model: SYS,
                system_language: pt,
                want: None,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            assert_eq!(
                compatibility(&catalog, lang(tc.language), tc.model, tc.system_language),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn compatibility_recommends_a_working_model_for_every_combination() {
        // the fix button must never offer the selected model, nor one that also fails
        for catalog in [with_system(), without_system()] {
            for model in catalog.models() {
                for language in all_settings() {
                    let name = format!("{language} on {}", model.id);
                    match compatibility(&catalog, language, model.id, Some("en")) {
                        Some(Compatibility::Unsupported {
                            recommended: Some(r),
                        }) => {
                            assert_ne!(r, model.id, "{name}");
                            assert!(supports(&catalog, language, r), "{name}");
                        }
                        Some(_) => {}
                        None => panic!("no compatibility for {name}"),
                    }
                }
            }
        }
    }

    #[test]
    fn compatibility_note_copy() {
        struct Case {
            name: &'static str,
            system: bool,
            language: &'static str,
            model: ModelId,
            system_language: Option<&'static str>,
            availability: ModelAvailability,
            want_text: &'static str,
            want_action: Option<(&'static str, NoteActionKind, bool)>,
        }
        let cases = [
            Case {
                name: "unsupported, recommended not downloaded",
                system: true,
                language: "de",
                model: V2,
                system_language: None,
                availability: ModelAvailability::NotDownloaded,
                want_text: "Parakeet TDT v2 can't transcribe German, so dictation will come out garbled.",
                want_action: Some((
                    "Download & Use Parakeet Ultra (~632MB)",
                    NoteActionKind::DownloadAndUse,
                    true,
                )),
            },
            Case {
                name: "unsupported, recommended downloaded",
                system: true,
                language: "ja",
                model: ULTRA,
                system_language: None,
                availability: ModelAvailability::Downloaded,
                want_text: "Parakeet Ultra can't transcribe Japanese, so dictation will come out garbled.",
                want_action: Some(("Use Parakeet Japanese", NoteActionKind::Use, true)),
            },
            Case {
                name: "unsupported, recommended downloading",
                system: true,
                language: "de",
                model: JA,
                system_language: None,
                availability: ModelAvailability::Downloading { progress: 0.427 },
                want_text: "Parakeet Japanese can't transcribe German, so dictation will come out garbled.",
                want_action: Some((
                    "Downloading Parakeet Ultra… 42%",
                    NoteActionKind::Downloading,
                    false,
                )),
            },
            Case {
                name: "unsupported, system speech recommended and built in",
                system: true,
                language: "ko",
                model: V2,
                system_language: None,
                availability: ModelAvailability::Downloaded,
                want_text: "Parakeet TDT v2 can't transcribe Korean, so dictation will come out garbled.",
                want_action: Some(("Use System Speech", NoteActionKind::Use, true)),
            },
            Case {
                name: "LANG-5 unsupported without any model has no button",
                system: false,
                language: "ko",
                model: V2,
                system_language: None,
                availability: ModelAvailability::NotDownloaded,
                want_text: "Parakeet TDT v2 can't transcribe Korean, so dictation will come out garbled.",
                want_action: None,
            },
            Case {
                name: "supported",
                system: true,
                language: "en",
                model: V2,
                system_language: None,
                availability: ModelAvailability::Downloaded,
                want_text: "Parakeet TDT v2 supports English.",
                want_action: None,
            },
            Case {
                name: "auto detects",
                system: true,
                language: "auto",
                model: ULTRA,
                system_language: None,
                availability: ModelAvailability::Downloaded,
                want_text: "Parakeet Ultra detects which of its supported languages you're speaking.",
                want_action: None,
            },
            Case {
                name: "auto single-language",
                system: true,
                language: "auto",
                model: JA,
                system_language: None,
                availability: ModelAvailability::Downloaded,
                want_text: "Parakeet Japanese can't detect the language. It always transcribes Japanese.",
                want_action: None,
            },
            Case {
                name: "auto system with known language",
                system: true,
                language: "auto",
                model: SYS,
                system_language: Some("pt"),
                availability: ModelAvailability::Downloaded,
                want_text: "System Speech can't detect the language. It uses the system language, Portuguese.",
                want_action: None,
            },
            Case {
                name: "auto system with unoffered language shows the code",
                system: true,
                language: "auto",
                model: SYS,
                system_language: Some("bn"),
                availability: ModelAvailability::Downloaded,
                want_text: "System Speech can't detect the language. It uses the system language, bn.",
                want_action: None,
            },
            Case {
                name: "auto system with unknown language",
                system: true,
                language: "auto",
                model: SYS,
                system_language: None,
                availability: ModelAvailability::Downloaded,
                want_text: "System Speech can't detect the language. It uses the system language.",
                want_action: None,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            let note = compatibility_note(
                &catalog,
                lang(tc.language),
                tc.model,
                tc.system_language,
                |_| tc.availability,
            )
            .unwrap_or_else(|| panic!("{}: no note", tc.name));
            assert_eq!(note.text, tc.want_text, "{}", tc.name);
            assert_eq!(
                note.is_warning,
                tc.want_text.contains("garbled"),
                "{}",
                tc.name
            );
            let got = note
                .action
                .as_ref()
                .map(|a| (a.label.as_str(), a.kind, a.enabled));
            assert_eq!(got, tc.want_action, "{}", tc.name);
        }
    }

    #[test]
    fn percent_truncates_and_clamps() {
        struct Case {
            name: &'static str,
            progress: f64,
            want: u8,
        }
        let cases = [
            Case {
                name: "truncates",
                progress: 0.429,
                want: 42,
            },
            Case {
                name: "full",
                progress: 1.0,
                want: 100,
            },
            Case {
                name: "over",
                progress: 1.5,
                want: 100,
            },
            Case {
                name: "negative",
                progress: -0.2,
                want: 0,
            },
            Case {
                name: "nan",
                progress: f64::NAN,
                want: 0,
            },
        ];
        for tc in cases {
            assert_eq!(percent(tc.progress), tc.want, "{}", tc.name);
        }
    }
}
