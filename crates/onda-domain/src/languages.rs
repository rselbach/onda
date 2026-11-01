//! Offered languages, the `selectedLanguage` setting, and system-speech
//! locale mapping (LANG-1..4, SYS-2, SYS-3, SYS-6, 09 "Languages").

use crate::models::SYSTEM_SPEECH_NAME;
use std::fmt;

struct Entry {
    code: &'static str,
    name: &'static str,
    also: &'static [&'static str],
}

// sorted by display name: the index order is the display order (LANG-1)
const LANGUAGES: [Entry; 39] = [
    entry("ar", "Arabic", &[]),
    entry("be", "Belarusian", &[]),
    entry("bs", "Bosnian", &[]),
    entry("bg", "Bulgarian", &[]),
    entry("yue", "Cantonese", &[]),
    entry("zh", "Chinese", &["Mandarin"]),
    entry("hr", "Croatian", &[]),
    entry("cs", "Czech", &[]),
    entry("da", "Danish", &[]),
    entry("nl", "Dutch", &["Flemish"]),
    entry("en", "English", &[]),
    entry("et", "Estonian", &[]),
    entry("fi", "Finnish", &[]),
    entry("fr", "French", &[]),
    entry("de", "German", &[]),
    entry("el", "Greek", &[]),
    entry("hi", "Hindi", &[]),
    entry("hu", "Hungarian", &[]),
    entry("id", "Indonesian", &[]),
    entry("it", "Italian", &[]),
    entry("ja", "Japanese", &[]),
    entry("ko", "Korean", &[]),
    entry("lv", "Latvian", &[]),
    entry("lt", "Lithuanian", &[]),
    entry("ms", "Malay", &[]),
    entry("mt", "Maltese", &[]),
    entry("pl", "Polish", &[]),
    entry("pt", "Portuguese", &[]),
    entry("ro", "Romanian", &[]),
    entry("ru", "Russian", &[]),
    entry("sr", "Serbian", &[]),
    entry("sk", "Slovak", &[]),
    entry("sl", "Slovenian", &[]),
    entry("es", "Spanish", &["Castilian"]),
    entry("sv", "Swedish", &[]),
    entry("th", "Thai", &[]),
    entry("tr", "Turkish", &[]),
    entry("uk", "Ukrainian", &[]),
    entry("vi", "Vietnamese", &[]),
];

const fn entry(code: &'static str, name: &'static str, also: &'static [&'static str]) -> Entry {
    Entry { code, name, also }
}

/// Codes of the onboarding "Popular" languages, in order, with their flags
/// (LANG-4).
const POPULAR: [(&str, &str); 11] = [
    ("en", "🇺🇸"),
    ("es", "🇪🇸"),
    ("fr", "🇫🇷"),
    ("de", "🇩🇪"),
    ("pt", "🇧🇷"),
    ("it", "🇮🇹"),
    ("ja", "🇯🇵"),
    ("ko", "🇰🇷"),
    ("zh", "🇨🇳"),
    ("hi", "🇮🇳"),
    ("ar", "🇸🇦"),
];

/// Onboarding card flag for languages outside the popular list.
pub const DEFAULT_FLAG: &str = "🌐";

/// One of the 39 offered languages (LANG-1). `auto` is not a `Language`; see
/// [`LanguageSetting`]. Ordering follows display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Language(u8);

impl Language {
    /// Every offered language, in display order (alphabetical by name).
    pub fn all() -> impl Iterator<Item = Language> {
        (0..LANGUAGES.len()).filter_map(|i| u8::try_from(i).ok().map(Language))
    }

    /// The onboarding "Popular" languages, in LANG-4 order.
    pub fn popular() -> impl Iterator<Item = Language> {
        POPULAR
            .iter()
            .filter_map(|(code, _)| Language::from_code(code))
    }

    /// Looks up an offered language by its code. Case-sensitive.
    pub fn from_code(code: &str) -> Option<Language> {
        Language::all().find(|l| l.code() == code)
    }

    fn entry(self) -> &'static Entry {
        // indices only come from `all()`, so this is always in range
        &LANGUAGES[usize::from(self.0) % LANGUAGES.len()]
    }

    /// ISO code, e.g. `en` or `yue`.
    pub fn code(self) -> &'static str {
        self.entry().code
    }

    /// The one preferred display name, e.g. "Spanish".
    pub fn name(self) -> &'static str {
        self.entry().name
    }

    /// Alternate names that map to this code, e.g. "Castilian". Not displayed.
    pub fn alternate_names(self) -> &'static [&'static str] {
        self.entry().also
    }

    /// Whether this language is in the popular list (LANG-4).
    pub fn is_popular(self) -> bool {
        POPULAR.iter().any(|(code, _)| *code == self.code())
    }

    /// Onboarding flag: a country flag for popular languages, else 🌐.
    pub fn flag(self) -> &'static str {
        POPULAR
            .iter()
            .find(|(code, _)| *code == self.code())
            .map_or(DEFAULT_FLAG, |(_, flag)| flag)
    }

    /// Whether a search query matches the display name, an alternate name, or
    /// the code (ONB-4), case-insensitively. An empty query matches all.
    pub fn matches_search(self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        std::iter::once(self.name())
            .chain(self.alternate_names().iter().copied())
            .chain(std::iter::once(self.code()))
            .any(|text| text.to_lowercase().contains(&query))
    }
}

/// Value of the `selectedLanguage` setting: Automatic or one offered language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageSetting {
    /// `auto`: let the model detect, or use the OS locale (SYS-2).
    Auto,
    /// A specific offered language.
    Specific(Language),
}

/// Stored code for Automatic.
pub const AUTO_CODE: &str = "auto";

/// Display name for Automatic.
pub const AUTO_NAME: &str = "Automatic";

impl LanguageSetting {
    /// Parses a stored code: `auto` or an offered code. Case-sensitive.
    pub fn parse(code: &str) -> Option<LanguageSetting> {
        if code == AUTO_CODE {
            return Some(LanguageSetting::Auto);
        }
        Language::from_code(code).map(LanguageSetting::Specific)
    }

    /// The stored code.
    pub fn code(self) -> &'static str {
        match self {
            LanguageSetting::Auto => AUTO_CODE,
            LanguageSetting::Specific(language) => language.code(),
        }
    }

    /// Display name; "Automatic" for `auto`.
    pub fn display_name(self) -> &'static str {
        match self {
            LanguageSetting::Auto => AUTO_NAME,
            LanguageSetting::Specific(language) => language.name(),
        }
    }

    /// The language to pass the engine (ASR-11): `None` for `auto`.
    pub fn engine_language(self) -> Option<Language> {
        match self {
            LanguageSetting::Auto => None,
            LanguageSetting::Specific(language) => Some(language),
        }
    }
}

impl fmt::Display for LanguageSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Display name for any language code: the offered name, "Automatic" for
/// `auto`, else the code itself (e.g. an OS locale Onda does not offer).
pub fn display_name_for_code(code: &str) -> &str {
    match LanguageSetting::parse(code) {
        Some(language) => language.display_name(),
        None => code,
    }
}

/// Default language (LANG-2): the OS locale's language code if offered, else
/// `auto`. `os_language` is the ISO language code of the OS locale (e.g.
/// `pt` for `pt-BR`), or `None` when the OS reports none. Callers evaluate it
/// live until the user picks a language.
pub fn default_language(os_language: Option<&str>) -> LanguageSetting {
    os_language
        .and_then(Language::from_code)
        .map_or(LanguageSetting::Auto, LanguageSetting::Specific)
}

/// Outcome of reading a stored `selectedLanguage` value (LANG-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredLanguage {
    /// The stored code is `auto` or offered.
    Valid(LanguageSetting),
    /// The stored code is unknown. The caller saves `auto` and posts
    /// [`crate::messages::UserMessage::InvalidLanguageSetting`].
    Invalid {
        /// The rejected code.
        code: String,
    },
}

/// Validates a stored language code (LANG-3).
pub fn resolve_stored_language(stored: &str) -> StoredLanguage {
    match LanguageSetting::parse(stored) {
        Some(language) => StoredLanguage::Valid(language),
        None => StoredLanguage::Invalid {
            code: stored.to_owned(),
        },
    }
}

/// Language code to system-speech locale identifier (SYS-3).
const SYSTEM_SPEECH_LOCALES: [(&str, &str); 39] = [
    ("en", "en-US"),
    ("es", "es-ES"),
    ("fr", "fr-FR"),
    ("de", "de-DE"),
    ("it", "it-IT"),
    ("ja", "ja-JP"),
    ("ko", "ko-KR"),
    ("pt", "pt-BR"),
    ("zh", "zh-CN"),
    ("nl", "nl-NL"),
    ("ru", "ru-RU"),
    ("tr", "tr-TR"),
    ("ar", "ar-SA"),
    ("hi", "hi-IN"),
    ("th", "th-TH"),
    ("vi", "vi-VN"),
    ("pl", "pl-PL"),
    ("sv", "sv-SE"),
    ("da", "da-DK"),
    ("fi", "fi-FI"),
    ("cs", "cs-CZ"),
    ("el", "el-GR"),
    ("hu", "hu-HU"),
    ("ro", "ro-RO"),
    ("uk", "uk-UA"),
    ("id", "id-ID"),
    ("ms", "ms-MY"),
    ("yue", "yue-CN"),
    ("be", "be-BY"),
    ("bg", "bg-BG"),
    ("bs", "bs-BA"),
    ("et", "et-EE"),
    ("hr", "hr-HR"),
    ("lt", "lt-LT"),
    ("lv", "lv-LV"),
    ("mt", "mt-MT"),
    ("sk", "sk-SK"),
    ("sl", "sl-SI"),
    ("sr", "sr-RS"),
];

/// Locale identifier for a language code (SYS-3). Unmapped codes are used
/// as-is.
pub fn system_speech_locale_for_code(code: &str) -> &str {
    SYSTEM_SPEECH_LOCALES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or(code, |(_, locale)| locale)
}

/// Locale the system engine prepares at load (SYS-2, SYS-3): the OS locale
/// for `auto`, else the mapped locale.
pub fn system_speech_locale(language: LanguageSetting, os_locale: &str) -> String {
    match language {
        LanguageSetting::Auto => os_locale.to_string(),
        LanguageSetting::Specific(language) => {
            system_speech_locale_for_code(language.code()).to_string()
        }
    }
}

/// Locale for one system-engine decode (SYS-2, SYS-5): a specific language
/// wins over the locale prepared at load; `auto` keeps the prepared locale
/// rather than re-reading the OS, falling back to `os_locale` when nothing
/// was prepared.
pub fn system_speech_decode_locale(
    language: LanguageSetting,
    prepared_locale: Option<&str>,
    os_locale: &str,
) -> String {
    match (language, prepared_locale) {
        (LanguageSetting::Auto, Some(prepared)) => prepared.to_string(),
        _ => system_speech_locale(language, os_locale),
    }
}

/// Unsupported-locale error text (SYS-6). `locale_display_name` is the OS's
/// localized name for the locale, e.g. "Polish (Poland)".
pub fn system_speech_unsupported_locale(locale_display_name: &str) -> String {
    format!("{SYSTEM_SPEECH_NAME} can't transcribe {locale_display_name} on this Mac")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(code: &str) -> Language {
        Language::from_code(code).unwrap_or_else(|| panic!("{code} not offered"))
    }

    #[test]
    fn thirty_nine_languages_sorted_by_name_lang_1() {
        let names: Vec<&str> = Language::all().map(Language::name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names.len(), 39);
        assert_eq!(names, sorted);
        let mut codes: Vec<&str> = Language::all().map(Language::code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 39, "codes are unique");
    }

    #[test]
    fn preferred_name_per_code_lang_1() {
        struct Case {
            name: &'static str,
            code: &'static str,
            want: (&'static str, &'static [&'static str]),
        }
        let cases = [
            Case {
                name: "castilian maps to spanish",
                code: "es",
                want: ("Spanish", &["Castilian"]),
            },
            Case {
                name: "flemish maps to dutch",
                code: "nl",
                want: ("Dutch", &["Flemish"]),
            },
            Case {
                name: "mandarin maps to chinese",
                code: "zh",
                want: ("Chinese", &["Mandarin"]),
            },
            Case {
                name: "romanian has one name",
                code: "ro",
                want: ("Romanian", &[]),
            },
            Case {
                name: "cantonese",
                code: "yue",
                want: ("Cantonese", &[]),
            },
        ];
        for tc in cases {
            let l = lang(tc.code);
            assert_eq!((l.name(), l.alternate_names()), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn parse_language_setting() {
        struct Case {
            name: &'static str,
            code: &'static str,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "auto",
                code: "auto",
                want: Some("auto"),
            },
            Case {
                name: "english",
                code: "en",
                want: Some("en"),
            },
            Case {
                name: "chinese",
                code: "zh",
                want: Some("zh"),
            },
            Case {
                name: "arabic",
                code: "ar",
                want: Some("ar"),
            },
            Case {
                name: "unknown",
                code: "xyz",
                want: None,
            },
            Case {
                name: "empty",
                code: "",
                want: None,
            },
            Case {
                name: "uppercase is not a code",
                code: "EN",
                want: None,
            },
            Case {
                name: "alternate name is not a code",
                code: "castilian",
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(
                LanguageSetting::parse(tc.code).map(LanguageSetting::code),
                tc.want,
                "{}",
                tc.name
            );
        }
        for l in Language::all() {
            assert_eq!(
                LanguageSetting::parse(l.code()),
                Some(LanguageSetting::Specific(l)),
                "{}",
                l.code()
            );
        }
    }

    #[test]
    fn engine_language_is_none_for_auto_asr_11() {
        assert_eq!(LanguageSetting::Auto.engine_language(), None);
        assert_eq!(
            LanguageSetting::Specific(lang("ja")).engine_language(),
            Some(lang("ja"))
        );
    }

    #[test]
    fn popular_languages_lang_4() {
        let got: Vec<&str> = Language::popular().map(Language::code).collect();
        assert_eq!(
            got,
            [
                "en", "es", "fr", "de", "pt", "it", "ja", "ko", "zh", "hi", "ar"
            ]
        );
    }

    #[test]
    fn onboarding_flags() {
        struct Case {
            name: &'static str,
            code: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "english",
                code: "en",
                want: "🇺🇸",
            },
            Case {
                name: "portuguese is brazil",
                code: "pt",
                want: "🇧🇷",
            },
            Case {
                name: "arabic",
                code: "ar",
                want: "🇸🇦",
            },
            Case {
                name: "not popular",
                code: "sl",
                want: "🌐",
            },
        ];
        for tc in cases {
            assert_eq!(lang(tc.code).flag(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn search_matches_name_alternate_or_code() {
        struct Case {
            name: &'static str,
            code: &'static str,
            query: &'static str,
            want: bool,
        }
        let cases = [
            Case {
                name: "name prefix",
                code: "es",
                query: "spa",
                want: true,
            },
            Case {
                name: "alternate name",
                code: "es",
                query: "Castilian",
                want: true,
            },
            Case {
                name: "code in caps",
                code: "yue",
                query: "YUE",
                want: true,
            },
            Case {
                name: "empty query",
                code: "en",
                query: "  ",
                want: true,
            },
            Case {
                name: "no match",
                code: "en",
                query: "greendale",
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                lang(tc.code).matches_search(tc.query),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn default_language_from_os_locale_lang_2() {
        struct Case {
            name: &'static str,
            os_language: Option<&'static str>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "offered",
                os_language: Some("pt"),
                want: "pt",
            },
            Case {
                name: "not offered",
                os_language: Some("bn"),
                want: "auto",
            },
            Case {
                name: "unknown locale",
                os_language: None,
                want: "auto",
            },
        ];
        for tc in cases {
            assert_eq!(
                default_language(tc.os_language).code(),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn invalid_stored_language_resets_to_auto_lang_3() {
        struct Case {
            name: &'static str,
            stored: &'static str,
            want: StoredLanguage,
        }
        let warning = |code: &str| StoredLanguage::Invalid {
            code: code.to_owned(),
        };
        let cases = [
            Case {
                name: "auto is valid",
                stored: "auto",
                want: StoredLanguage::Valid(LanguageSetting::Auto),
            },
            Case {
                name: "offered is valid",
                stored: "de",
                want: StoredLanguage::Valid(LanguageSetting::Specific(lang("de"))),
            },
            Case {
                name: "unknown warns",
                stored: "xx",
                want: warning("xx"),
            },
            Case {
                name: "wrong case warns",
                stored: "EN",
                want: warning("EN"),
            },
        ];
        for tc in cases {
            assert_eq!(resolve_stored_language(tc.stored), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn display_name_for_code_falls_back_to_code() {
        struct Case {
            name: &'static str,
            code: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "offered",
                code: "ko",
                want: "Korean",
            },
            Case {
                name: "auto",
                code: "auto",
                want: "Automatic",
            },
            Case {
                name: "not offered",
                code: "bn",
                want: "bn",
            },
        ];
        for tc in cases {
            assert_eq!(display_name_for_code(tc.code), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn locale_map_covers_every_language_sys_3() {
        for l in Language::all() {
            assert!(
                SYSTEM_SPEECH_LOCALES.iter().any(|(c, _)| *c == l.code()),
                "{} has no locale mapping",
                l.code()
            );
        }
    }

    #[test]
    fn system_speech_locale_sys_2_sys_3() {
        struct Case {
            name: &'static str,
            language: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "english",
                language: "en",
                want: "en-US",
            },
            Case {
                name: "spanish",
                language: "es",
                want: "es-ES",
            },
            Case {
                name: "chinese",
                language: "zh",
                want: "zh-CN",
            },
            Case {
                name: "portuguese",
                language: "pt",
                want: "pt-BR",
            },
            Case {
                name: "auto uses os locale",
                language: "auto",
                want: "fr-CA",
            },
        ];
        for tc in cases {
            let language = LanguageSetting::parse(tc.language)
                .unwrap_or_else(|| panic!("{}: bad language", tc.name));
            assert_eq!(
                system_speech_locale(language, "fr-CA"),
                tc.want,
                "{}",
                tc.name
            );
        }
        assert_eq!(system_speech_locale_for_code("xx"), "xx", "unmapped as-is");
    }

    #[test]
    fn decode_locale_sys_2_sys_5() {
        struct Case {
            name: &'static str,
            language: &'static str,
            prepared: Option<&'static str>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "passed language wins over prepared locale",
                language: "de",
                prepared: Some("en-US"),
                want: "de-DE",
            },
            Case {
                name: "auto keeps prepared locale",
                language: "auto",
                prepared: Some("es-ES"),
                want: "es-ES",
            },
            Case {
                name: "auto with nothing prepared uses os locale",
                language: "auto",
                prepared: None,
                want: "it-IT",
            },
        ];
        for tc in cases {
            let language = LanguageSetting::parse(tc.language)
                .unwrap_or_else(|| panic!("{}: bad language", tc.name));
            assert_eq!(
                system_speech_decode_locale(language, tc.prepared, "it-IT"),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn unsupported_locale_message_sys_6() {
        assert_eq!(
            system_speech_unsupported_locale("Persian (Iran)"),
            "System Speech can't transcribe Persian (Iran) on this Mac"
        );
    }
}
