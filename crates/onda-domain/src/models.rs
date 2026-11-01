//! Static speech-model catalog, model errors, and validation detail copy
//! (MOD-1..4).
//!
//! Download URLs and per-model file lists (DL-6) belong to the inference
//! backend (MOD-1); see `onda_asr::ModelManifest`.

use std::fmt;

/// Identifier of a model in the catalog, in catalog display order.
///
/// Stored as its string form in the `selectedModel` setting. Strings that do
/// not parse are unknown or removed ids (DL-12, ASR-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelId {
    /// `parakeet-tdt-v2`, English.
    ParakeetTdtV2,
    /// `parakeet-ultra`, 25 European languages.
    ParakeetUltra,
    /// `parakeet-ja`, Japanese.
    ParakeetJapanese,
    /// `parakeet-unified`, English with streaming decode.
    ParakeetUnified,
    /// `system-speech`, the platform engine; present only when the shell
    /// provides one (MOD-4).
    SystemSpeech,
}

impl ModelId {
    /// Every id, in catalog order.
    pub const ALL: [ModelId; 5] = [
        ModelId::ParakeetTdtV2,
        ModelId::ParakeetUltra,
        ModelId::ParakeetJapanese,
        ModelId::ParakeetUnified,
        ModelId::SystemSpeech,
    ];

    /// The stable string id used in settings and folder names.
    pub const fn as_str(self) -> &'static str {
        match self {
            ModelId::ParakeetTdtV2 => "parakeet-tdt-v2",
            ModelId::ParakeetUltra => "parakeet-ultra",
            ModelId::ParakeetJapanese => "parakeet-ja",
            ModelId::ParakeetUnified => "parakeet-unified",
            ModelId::SystemSpeech => "system-speech",
        }
    }

    /// Parses a string id. Case-sensitive; unknown ids return `None`.
    pub fn parse(id: &str) -> Option<ModelId> {
        ModelId::ALL.into_iter().find(|m| m.as_str() == id)
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Model family; decides runtime behavior such as auto-detect handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    /// Parakeet TDT: whole-buffer decode.
    ParakeetTdt,
    /// Parakeet Unified: incremental streaming decode.
    ParakeetUnified,
    /// The platform speech engine.
    SystemEngine,
}

impl ModelFamily {
    /// Family name as shown in the catalog table.
    pub const fn display_name(self) -> &'static str {
        match self {
            ModelFamily::ParakeetTdt => "Parakeet TDT",
            ModelFamily::ParakeetUnified => "Parakeet Unified (streaming)",
            ModelFamily::SystemEngine => "System engine",
        }
    }
}

/// Where a model's files come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelSource {
    /// Downloaded into `<data>/Models/<model-id>`.
    Downloaded,
    /// Provided by the platform; never downloaded or deleted, always counts as
    /// downloaded (DL-10).
    PlatformProvided,
}

/// Facts about the platform speech engine, supplied by the shell (MOD-4).
///
/// The display name is always "System Speech". License and attribution are
/// platform-specific, so the shell supplies them too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSpeechInfo {
    /// E.g. "Built-in macOS speech recognition — no download required".
    pub description: String,
    /// Language codes the engine reports (SYS-1).
    pub languages: Vec<String>,
    /// E.g. "Apple System".
    pub license_name: String,
    /// E.g. "https://www.apple.com/legal/sla/".
    pub license_url: String,
    /// E.g. "Apple Speech Framework".
    pub attribution: String,
}

/// Display name of the platform engine on every platform.
pub const SYSTEM_SPEECH_NAME: &str = "System Speech";

/// One catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo<'a> {
    /// Model id.
    pub id: ModelId,
    /// Family.
    pub family: ModelFamily,
    /// Display name, e.g. "Parakeet TDT v2".
    pub name: &'a str,
    /// One-line description.
    pub description: &'a str,
    /// Size hint, e.g. "~464MB" or "Built-in".
    pub size_hint: &'a str,
    /// Whether the model produces a live preview while recording (ASR-15).
    pub live_preview: bool,
    /// License name, e.g. "CC BY 4.0".
    pub license_name: &'a str,
    /// License URL.
    pub license_url: &'a str,
    /// Attribution line shown on About.
    pub attribution: &'a str,
    /// Downloaded or platform-provided.
    pub source: ModelSource,
    languages: LanguageSet<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanguageSet<'a> {
    Static(&'static [&'static str]),
    Shell(&'a [String]),
}

impl ModelInfo<'_> {
    /// Whether `code` is in the model's language set. `auto` is not a member;
    /// see `routing::supports` for LANG-7.
    pub fn has_language(&self, code: &str) -> bool {
        match self.languages {
            LanguageSet::Static(codes) => codes.contains(&code),
            LanguageSet::Shell(codes) => codes.iter().any(|c| c == code),
        }
    }

    /// The model's language codes.
    pub fn language_codes(&self) -> Vec<&str> {
        match self.languages {
            LanguageSet::Static(codes) => codes.to_vec(),
            LanguageSet::Shell(codes) => codes.iter().map(String::as_str).collect(),
        }
    }

    /// True for platform-provided engines.
    pub fn is_built_in(&self) -> bool {
        self.source == ModelSource::PlatformProvided
    }
}

/// Languages of NVIDIA Parakeet TDT v3, which Parakeet Ultra keeps. Wider
/// than needed on purpose: a language added later routes to Ultra for free.
pub const PARAKEET_ULTRA_LANGUAGES: &[&str] = &[
    "be", "bg", "bs", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt",
    "lv", "mt", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sr", "sv", "uk",
];

const CC_BY_4: &str = "CC BY 4.0";
const CC_BY_4_URL: &str = "https://creativecommons.org/licenses/by/4.0/";

// size hints are each download's total in decimal MB; a test in onda-asr-coreml
// checks them against the backend's manifests (MOD-2).
const STATIC_MODELS: [ModelInfo<'static>; 4] = [
    ModelInfo {
        id: ModelId::ParakeetTdtV2,
        family: ModelFamily::ParakeetTdt,
        name: "Parakeet TDT v2",
        description: "NVIDIA Parakeet TDT v2 — fast, accurate English transcription",
        size_hint: "~464MB",
        live_preview: true,
        license_name: CC_BY_4,
        license_url: CC_BY_4_URL,
        attribution: "Parakeet TDT 0.6B v2 by NVIDIA; CoreML conversion by FluidInference",
        source: ModelSource::Downloaded,
        languages: LanguageSet::Static(&["en"]),
    },
    ModelInfo {
        id: ModelId::ParakeetUltra,
        family: ModelFamily::ParakeetTdt,
        name: "Parakeet Ultra",
        description: "Moondream's retrained Parakeet v3 — 25 European languages, fewer errors at the same speed",
        size_hint: "~632MB",
        live_preview: true,
        license_name: CC_BY_4,
        license_url: CC_BY_4_URL,
        attribution: "Parakeet Ultra by Moondream, post-trained from NVIDIA Parakeet TDT 0.6B v3; CoreML conversion by FluidInference",
        source: ModelSource::Downloaded,
        languages: LanguageSet::Static(PARAKEET_ULTRA_LANGUAGES),
    },
    ModelInfo {
        id: ModelId::ParakeetJapanese,
        family: ModelFamily::ParakeetTdt,
        name: "Parakeet Japanese",
        description: "NVIDIA Parakeet 0.6B tuned for Japanese",
        size_hint: "~619MB",
        live_preview: true,
        license_name: CC_BY_4,
        license_url: CC_BY_4_URL,
        attribution: "Parakeet 0.6B Japanese by NVIDIA; CoreML conversion by FluidInference",
        source: ModelSource::Downloaded,
        languages: LanguageSet::Static(&["ja"]),
    },
    ModelInfo {
        id: ModelId::ParakeetUnified,
        family: ModelFamily::ParakeetUnified,
        name: "Parakeet Unified",
        description: "NVIDIA Parakeet Unified — English, transcribed live as you speak",
        size_hint: "~609MB",
        live_preview: true,
        license_name: "NVIDIA Open Model License",
        license_url: "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/",
        attribution: "Parakeet Unified 0.6B by NVIDIA; CoreML conversion by FluidInference",
        source: ModelSource::Downloaded,
        languages: LanguageSet::Static(&["en"]),
    },
];

/// The model catalog for this run: the static Parakeet models plus the
/// system engine when the shell provides one (MOD-3, MOD-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    system_speech: Option<SystemSpeechInfo>,
}

impl Catalog {
    /// Builds the catalog. Pass `None` when the shell has no system engine.
    pub fn new(system_speech: Option<SystemSpeechInfo>) -> Catalog {
        Catalog { system_speech }
    }

    /// Whether the shell provides a system engine.
    pub fn has_system_speech(&self) -> bool {
        self.system_speech.is_some()
    }

    /// Every model, in catalog order.
    pub fn models(&self) -> Vec<ModelInfo<'_>> {
        ModelId::ALL
            .into_iter()
            .filter_map(|id| self.model(id))
            .collect()
    }

    /// The entry for `id`, or `None` for `system-speech` without a system
    /// engine.
    pub fn model(&self, id: ModelId) -> Option<ModelInfo<'_>> {
        if let Some(info) = STATIC_MODELS.iter().find(|m| m.id == id) {
            return Some(info.clone());
        }
        let system = self.system_speech.as_ref()?;
        Some(ModelInfo {
            id: ModelId::SystemSpeech,
            family: ModelFamily::SystemEngine,
            name: SYSTEM_SPEECH_NAME,
            description: &system.description,
            size_hint: "Built-in",
            live_preview: false,
            license_name: &system.license_name,
            license_url: &system.license_url,
            attribution: &system.attribution,
            source: ModelSource::PlatformProvided,
            languages: LanguageSet::Shell(&system.languages),
        })
    }

    /// Whether `id` is in this run's catalog.
    pub fn contains(&self, id: ModelId) -> bool {
        id != ModelId::SystemSpeech || self.has_system_speech()
    }

    /// Parses a stored id against this catalog. `None` for unknown ids and
    /// for `system-speech` when the shell has no system engine.
    pub fn resolve(&self, stored: &str) -> Option<ModelId> {
        ModelId::parse(stored).filter(|id| self.contains(*id))
    }

    /// Display name for `id`; falls back to the raw id string.
    pub fn name(&self, id: ModelId) -> &str {
        match id {
            ModelId::SystemSpeech => SYSTEM_SPEECH_NAME,
            _ => self.model(id).map_or(id.as_str(), |m| m.name),
        }
    }

    /// Whether `id` is platform-provided.
    pub fn is_built_in(&self, id: ModelId) -> bool {
        id == ModelId::SystemSpeech
    }

    /// Downloaded models in catalog order: built-in engines plus the
    /// downloadable models in `on_disk` (folders that passed validation,
    /// DL-5).
    pub fn downloaded(&self, on_disk: &[ModelId]) -> Vec<ModelId> {
        ModelId::ALL
            .into_iter()
            .filter(|id| self.contains(*id))
            .filter(|id| self.is_built_in(*id) || on_disk.contains(id))
            .collect()
    }
}

/// Model management errors with their user-facing descriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// Deleting the only downloaded model while it is selected (DL-10).
    CannotDeleteActiveModel,
    /// A download finished but the folder failed validation.
    IncompleteInstallation {
        /// Model id string.
        model_id: String,
        /// Validation details, see [`FolderValidation::failure_details`].
        details: String,
    },
    /// Unknown, built-in, or not downloaded.
    NotFound {
        /// Model id string as requested.
        model_id: String,
    },
    /// The download timed out.
    DownloadTimedOut {
        /// Model id string.
        model_id: String,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::CannotDeleteActiveModel => f.write_str(
                "Cannot delete the currently active model. Please select a different model first.",
            ),
            ModelError::IncompleteInstallation { model_id, details } => {
                write!(f, "Model '{model_id}' is incomplete: {details}.")
            }
            ModelError::NotFound { model_id } => {
                write!(f, "Model '{model_id}' not found or already deleted.")
            }
            ModelError::DownloadTimedOut { model_id } => {
                write!(f, "Download timed out for model '{model_id}'.")
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// Result of checking a model folder (DL-5). The checks themselves are I/O
/// and live in the model manager; this type turns them into copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderValidation {
    /// The folder exists and is a directory.
    pub folder_exists: bool,
    /// Error text when listing the folder failed.
    pub read_error: Option<String>,
    /// Required files or bundles that are absent, in layout order.
    pub missing_files: Vec<String>,
    /// Every weight file is complete.
    pub has_weights: bool,
}

impl FolderValidation {
    /// Whether the folder counts as downloaded.
    pub fn is_complete(&self) -> bool {
        self.folder_exists
            && self.read_error.is_none()
            && self.missing_files.is_empty()
            && self.has_weights
    }

    /// Problems joined with "; ", or `None` when complete.
    pub fn failure_details(&self) -> Option<String> {
        if self.is_complete() {
            return None;
        }
        let mut problems = Vec::new();
        if !self.folder_exists {
            problems.push("folder does not exist".to_string());
        }
        if let Some(error) = &self.read_error {
            problems.push(format!("could not read folder: {error}"));
        }
        if !self.missing_files.is_empty() {
            problems.push(format!("missing files: {}", self.missing_files.join(", ")));
        }
        if !self.has_weights {
            problems.push("missing model weights".to_string());
        }
        Some(problems.join("; "))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A macOS-like system engine with Jabber's language list (SYS-1).
    pub(crate) fn mac_system_speech() -> SystemSpeechInfo {
        SystemSpeechInfo {
            description: "Built-in macOS speech recognition — no download required".to_string(),
            languages: [
                "ar", "bn", "ca", "cs", "da", "de", "el", "en", "es", "fi", "fr", "gu", "he", "hi",
                "hr", "hu", "id", "it", "ja", "kn", "ko", "ks", "mai", "ml", "mr", "ms", "nb",
                "ne", "nl", "or", "pa", "pl", "pt", "ro", "ru", "sk", "sv", "ta", "te", "th", "tr",
                "uk", "ur", "vi", "yue", "zh",
            ]
            .map(String::from)
            .to_vec(),
            license_name: "Apple System".to_string(),
            license_url: "https://www.apple.com/legal/sla/".to_string(),
            attribution: "Apple Speech Framework".to_string(),
        }
    }

    pub(crate) fn with_system() -> Catalog {
        Catalog::new(Some(mac_system_speech()))
    }

    pub(crate) fn without_system() -> Catalog {
        Catalog::new(None)
    }

    #[test]
    fn catalog_order_and_membership_mod_3_mod_4() {
        struct Case {
            name: &'static str,
            catalog: Catalog,
            want: Vec<ModelId>,
        }
        let cases = [
            Case {
                name: "with system engine",
                catalog: with_system(),
                want: ModelId::ALL.to_vec(),
            },
            Case {
                name: "without system engine",
                catalog: without_system(),
                want: ModelId::ALL[..4].to_vec(),
            },
        ];
        for tc in cases {
            let got: Vec<ModelId> = tc.catalog.models().iter().map(|m| m.id).collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn model_properties() {
        struct Case {
            name: &'static str,
            id: ModelId,
            want: (&'static str, &'static str, &'static str, ModelSource),
        }
        let cases = [
            Case {
                name: "parakeet v2",
                id: ModelId::ParakeetTdtV2,
                want: (
                    "Parakeet TDT v2",
                    "NVIDIA Parakeet TDT v2 — fast, accurate English transcription",
                    "~464MB",
                    ModelSource::Downloaded,
                ),
            },
            Case {
                name: "system speech MOD-4",
                id: ModelId::SystemSpeech,
                want: (
                    "System Speech",
                    "Built-in macOS speech recognition — no download required",
                    "Built-in",
                    ModelSource::PlatformProvided,
                ),
            },
        ];
        let catalog = with_system();
        for tc in cases {
            let m = catalog
                .model(tc.id)
                .unwrap_or_else(|| panic!("{}: missing", tc.name));
            assert_eq!(
                (m.name, m.description, m.size_hint, m.source),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    // MOD-1: the conversion's producer is credited, as Jabber does
    #[test]
    fn attributions_credit_the_conversion_mod_1() {
        let cases = [
            (
                ModelId::ParakeetTdtV2,
                "Parakeet TDT 0.6B v2 by NVIDIA; CoreML conversion by FluidInference",
            ),
            (
                ModelId::ParakeetUltra,
                "Parakeet Ultra by Moondream, post-trained from NVIDIA Parakeet TDT 0.6B v3; \
                 CoreML conversion by FluidInference",
            ),
            (
                ModelId::ParakeetJapanese,
                "Parakeet 0.6B Japanese by NVIDIA; CoreML conversion by FluidInference",
            ),
            (
                ModelId::ParakeetUnified,
                "Parakeet Unified 0.6B by NVIDIA; CoreML conversion by FluidInference",
            ),
            (ModelId::SystemSpeech, "Apple Speech Framework"),
        ];
        let catalog = with_system();
        for (id, want) in cases {
            let m = catalog
                .model(id)
                .unwrap_or_else(|| panic!("{id:?}: missing"));
            assert_eq!(m.attribution, want, "{id:?}");
        }
    }

    #[test]
    fn model_id_round_trip_and_resolve() {
        struct Case {
            name: &'static str,
            stored: &'static str,
            catalog: Catalog,
            want: Option<ModelId>,
        }
        let cases = [
            Case {
                name: "known id",
                stored: "parakeet-ultra",
                catalog: without_system(),
                want: Some(ModelId::ParakeetUltra),
            },
            Case {
                name: "unknown id",
                stored: "greendale-human-being",
                catalog: with_system(),
                want: None,
            },
            Case {
                name: "jabber apple-speech id is unknown",
                stored: "apple-speech",
                catalog: with_system(),
                want: None,
            },
            Case {
                name: "system speech with engine",
                stored: "system-speech",
                catalog: with_system(),
                want: Some(ModelId::SystemSpeech),
            },
            Case {
                name: "system speech without engine is unknown MOD-4",
                stored: "system-speech",
                catalog: without_system(),
                want: None,
            },
            Case {
                name: "case-sensitive",
                stored: "Parakeet-TDT-v2",
                catalog: with_system(),
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(tc.catalog.resolve(tc.stored), tc.want, "{}", tc.name);
        }
        for id in ModelId::ALL {
            assert_eq!(ModelId::parse(id.as_str()), Some(id), "{id}");
        }
    }

    #[test]
    fn downloaded_counts_built_in_engines_dl_10() {
        struct Case {
            name: &'static str,
            catalog: Catalog,
            on_disk: Vec<ModelId>,
            want: Vec<ModelId>,
        }
        let cases = [
            Case {
                name: "built-in always downloaded",
                catalog: with_system(),
                on_disk: vec![],
                want: vec![ModelId::SystemSpeech],
            },
            Case {
                name: "nothing without system engine",
                catalog: without_system(),
                on_disk: vec![],
                want: vec![],
            },
            Case {
                name: "catalog order",
                catalog: with_system(),
                on_disk: vec![ModelId::ParakeetUnified, ModelId::ParakeetTdtV2],
                want: vec![
                    ModelId::ParakeetTdtV2,
                    ModelId::ParakeetUnified,
                    ModelId::SystemSpeech,
                ],
            },
            Case {
                name: "system speech on disk ignored without engine",
                catalog: without_system(),
                on_disk: vec![ModelId::SystemSpeech],
                want: vec![],
            },
        ];
        for tc in cases {
            assert_eq!(tc.catalog.downloaded(&tc.on_disk), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn model_error_descriptions() {
        struct Case {
            name: &'static str,
            error: ModelError,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "cannot delete active",
                error: ModelError::CannotDeleteActiveModel,
                want: "Cannot delete the currently active model. Please select a different model first.",
            },
            Case {
                name: "download timeout",
                error: ModelError::DownloadTimedOut {
                    model_id: "base".to_string(),
                },
                want: "Download timed out for model 'base'.",
            },
            Case {
                name: "incomplete installation",
                error: ModelError::IncompleteInstallation {
                    model_id: "base".to_string(),
                    details: "missing files: vocab.json".to_string(),
                },
                want: "Model 'base' is incomplete: missing files: vocab.json.",
            },
            Case {
                name: "not found",
                error: ModelError::NotFound {
                    model_id: "nonexistent".to_string(),
                },
                want: "Model 'nonexistent' not found or already deleted.",
            },
        ];
        for tc in cases {
            assert_eq!(tc.error.to_string(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn validation_details_dl_5() {
        let complete = FolderValidation {
            folder_exists: true,
            read_error: None,
            missing_files: vec![],
            has_weights: true,
        };
        struct Case {
            name: &'static str,
            validation: FolderValidation,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "complete folder",
                validation: complete.clone(),
                want: None,
            },
            Case {
                name: "missing vocabulary",
                validation: FolderValidation {
                    missing_files: vec!["parakeet_vocab.json".to_string()],
                    ..complete.clone()
                },
                want: Some("missing files: parakeet_vocab.json"),
            },
            Case {
                name: "empty bundle",
                validation: FolderValidation {
                    has_weights: false,
                    ..complete.clone()
                },
                want: Some("missing model weights"),
            },
            Case {
                name: "missing folder",
                validation: FolderValidation {
                    folder_exists: false,
                    read_error: None,
                    missing_files: vec!["vocab.json".to_string(), "Encoder.mlmodelc".to_string()],
                    has_weights: false,
                },
                want: Some(
                    "folder does not exist; missing files: vocab.json, Encoder.mlmodelc; missing model weights",
                ),
            },
            Case {
                name: "unreadable folder",
                validation: FolderValidation {
                    read_error: Some("permission denied".to_string()),
                    ..complete.clone()
                },
                want: Some("could not read folder: permission denied"),
            },
        ];
        for tc in cases {
            assert_eq!(
                tc.validation.is_complete(),
                tc.want.is_none(),
                "{}",
                tc.name
            );
            assert_eq!(
                tc.validation.failure_details().as_deref(),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
