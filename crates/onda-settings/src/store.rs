//! The settings store over `settings.json` (SET-1..SET-4).

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use onda_domain::cleanup::{CleanupModel, CleanupProvider};
use onda_domain::hotkey::Shortcut;
use onda_domain::language_menu::encode_recent;
use onda_domain::languages::{
    LanguageSetting, StoredLanguage, default_language, resolve_stored_language,
};
use onda_domain::messages::UserMessage;
use onda_domain::models::{Catalog, ModelId};
use onda_domain::replacements::{ReplacementRule, decode_rules};
use onda_domain::routing::default_model;
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use serde_json::{Map, Value};

use crate::Key;

type Observer = Box<dyn Fn(Key) + Send + Sync>;

/// Typed access to `<data>/settings.json`.
///
/// The file is read once at [`Settings::open`] and cached; every setter
/// rewrites it atomically (temp file plus rename) and keeps keys this
/// version does not know. Getters never write and never fail: a missing key
/// is the default, a corrupt value is the default plus a log line (SET-2),
/// and a corrupt or unreadable file reads as empty.
///
/// `Settings` is `Send + Sync`; share it behind an `Arc`.
pub struct Settings {
    path: PathBuf,
    values: Mutex<Map<String, Value>>,
    observer: Option<Observer>,
}

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// A setter could not save `settings.json`. The stored and cached values are
/// unchanged.
#[derive(Debug)]
pub enum SettingsError {
    /// Creating, writing, syncing, or renaming the file failed.
    Io(io::Error),
    /// A value could not be encoded as JSON.
    Encode(serde_json::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "couldn't write settings: {error}"),
            Self::Encode(error) => write!(f, "couldn't encode settings: {error}"),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Encode(error) => Some(error),
        }
    }
}

impl From<io::Error> for SettingsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Settings {
    /// Opens the store at `path` (normally `<data>/settings.json`).
    ///
    /// Never fails. A missing file is empty settings. An unreadable file, a
    /// file that is not JSON, or JSON that is not an object is logged and
    /// read as empty; the next setter replaces it. The parent directory is
    /// created on first write.
    pub fn open(path: impl Into<PathBuf>) -> Settings {
        let path = path.into();
        let values = load(&path);
        Settings {
            path,
            values: Mutex::new(values),
            observer: None,
        }
    }

    /// Calls `observer` with each key whose stored value a setter, reset, or
    /// [`Settings::canonicalize`] changed, after the file is saved. Setting
    /// a key to the value it already has saves nothing and reports nothing.
    /// The observer runs on the caller's thread without the store locked, so
    /// it may read settings.
    pub fn on_change(mut self, observer: impl Fn(Key) + Send + Sync + 'static) -> Settings {
        self.observer = Some(Box::new(observer));
        self
    }

    /// Whether `key` is stored, valid or not.
    pub fn is_set(&self, key: Key) -> bool {
        self.lock().contains_key(key.as_str())
    }

    /// Removes `key`, so it reads as its default again.
    pub fn remove(&self, key: Key) -> Result<(), SettingsError> {
        self.apply(vec![(key, None)])
    }

    /// Rewrites stale enum-like values to their canonical form at launch
    /// (SET-3): `outputMode`, `hotkeyActivationMode`, `historyRetention`,
    /// and `openRouterModel` (replaced slugs map to their successor, blank
    /// ones to the default; CLN-21). Unset keys stay unset. Idempotent:
    /// returns the keys it rewrote, empty once everything is canonical.
    pub fn canonicalize(&self) -> Result<Vec<Key>, SettingsError> {
        let model = self.cleanup_model(CleanupProvider::OpenRouter);
        let canonical = [
            (Key::OutputMode, self.output_mode().as_setting()),
            (
                Key::HotkeyActivationMode,
                self.activation_mode().as_setting(),
            ),
            (Key::HistoryRetention, self.history_retention().as_setting()),
            (Key::OpenRouterModel, model.slug.as_ref()),
        ];
        let changes: Vec<(Key, Option<Value>)> = canonical
            .into_iter()
            .filter(|(key, value)| self.raw(*key).is_some_and(|raw| raw != *value))
            .map(|(key, value)| (key, Some(Value::from(value))))
            .collect();
        let keys = changes.iter().map(|(key, _)| *key).collect();
        self.apply(changes)?;
        Ok(keys)
    }

    /// The stored `selectedModel` id, or the recommended model for the
    /// default language when unset (LANG-6, SET-2a).
    ///
    /// A stored id is returned as-is even when unknown or removed; pass it to
    /// `onda_domain::migration::resolve_selection` (DL-12, ASR-4).
    /// `catalog` says whether a system engine exists; `os_language` is the
    /// OS locale's ISO language code (e.g. `pt` for `pt-BR`).
    pub fn selected_model(&self, catalog: &Catalog, os_language: Option<&str>) -> String {
        if let Some(id) = self.string(Key::SelectedModel) {
            return id;
        }
        default_model(catalog, default_language(os_language))
            .as_str()
            .to_owned()
    }

    /// Stores `selectedModel`.
    pub fn set_selected_model(&self, model: ModelId) -> Result<(), SettingsError> {
        self.put(Key::SelectedModel, Value::from(model.as_str()))
    }

    /// The selected language. Unset follows `os_language` live (LANG-2,
    /// SET-2a). A stored code that is not offered reads as `auto` (LANG-3);
    /// call [`Settings::reset_invalid_language`] to store that and get the
    /// warning.
    pub fn selected_language(&self, os_language: Option<&str>) -> LanguageSetting {
        let Some(code) = self.string(Key::SelectedLanguage) else {
            return default_language(os_language);
        };
        match resolve_stored_language(&code) {
            StoredLanguage::Valid(language) => language,
            StoredLanguage::Invalid { .. } => {
                tracing::warn!(key = %Key::SelectedLanguage, "unrecognized language code; reading as auto");
                LanguageSetting::Auto
            }
        }
    }

    /// Stores `selectedLanguage`, ending the live OS-locale default.
    pub fn set_selected_language(&self, language: LanguageSetting) -> Result<(), SettingsError> {
        self.put(Key::SelectedLanguage, Value::from(language.code()))
    }

    /// LANG-3: if the stored language code is neither `auto` nor offered,
    /// stores `auto` and returns the "Invalid Language Setting" warning for
    /// the caller to post. `None` when nothing needed resetting.
    pub fn reset_invalid_language(&self) -> Result<Option<UserMessage>, SettingsError> {
        let Some(stored) = self.string(Key::SelectedLanguage) else {
            return Ok(None);
        };
        let StoredLanguage::Invalid { code } = resolve_stored_language(&stored) else {
            return Ok(None);
        };
        self.put(
            Key::SelectedLanguage,
            Value::from(LanguageSetting::Auto.code()),
        )?;
        tracing::warn!(key = %Key::SelectedLanguage, "reset an unrecognized language code to auto");
        Ok(Some(UserMessage::InvalidLanguageSetting { code }))
    }

    /// The comma-separated `recentLanguages` value (LANG-8); decode it with
    /// `onda_domain::language_menu::recent_languages`.
    pub fn recent_languages(&self) -> String {
        self.string(Key::RecentLanguages).unwrap_or_default()
    }

    /// Stores `recentLanguages`, most recent first.
    pub fn set_recent_languages(&self, languages: &[LanguageSetting]) -> Result<(), SettingsError> {
        self.put(Key::RecentLanguages, Value::from(encode_recent(languages)))
    }

    /// The input device UID; empty means the system default (AUD-2).
    pub fn input_device_uid(&self) -> String {
        self.string(Key::InputDeviceUid).unwrap_or_default()
    }

    /// Stores `inputDeviceUID`; pass `""` for the system default.
    pub fn set_input_device_uid(&self, uid: &str) -> Result<(), SettingsError> {
        self.put(Key::InputDeviceUid, Value::from(uid))
    }

    /// Where transcripts go. Default `type`.
    pub fn output_mode(&self) -> OutputMode {
        self.enum_value()
    }

    /// Stores `outputMode`.
    pub fn set_output_mode(&self, mode: OutputMode) -> Result<(), SettingsError> {
        self.put_enum(mode)
    }

    /// The dictation shortcut. Default `⌥ Space`; a stored value that does
    /// not parse or fails HOT-4 reads as the default (HOT-5).
    pub fn hotkey(&self) -> Shortcut {
        let Some(raw) = self.raw(Key::Hotkey) else {
            return Shortcut::default();
        };
        Shortcut::from_setting(&raw).unwrap_or_else(|error| {
            tracing::warn!(key = %Key::Hotkey, %error, "invalid setting; using the default");
            Shortcut::default()
        })
    }

    /// Stores `hotkey` as `{key, modifiers[]}`.
    pub fn set_hotkey(&self, shortcut: &Shortcut) -> Result<(), SettingsError> {
        let value = serde_json::to_value(shortcut).map_err(SettingsError::Encode)?;
        self.put(Key::Hotkey, value)
    }

    /// How the hotkey starts and stops recording. Default `hold`.
    pub fn activation_mode(&self) -> ActivationMode {
        self.enum_value()
    }

    /// Stores `hotkeyActivationMode`.
    pub fn set_activation_mode(&self, mode: ActivationMode) -> Result<(), SettingsError> {
        self.put_enum(mode)
    }

    /// "Pause media while recording". Default false.
    pub fn pause_media_during_recording(&self) -> bool {
        self.bool(Key::PauseMediaDuringRecording, false)
    }

    /// Stores `pauseMediaDuringRecording`.
    pub fn set_pause_media_during_recording(&self, enabled: bool) -> Result<(), SettingsError> {
        self.put(Key::PauseMediaDuringRecording, Value::from(enabled))
    }

    /// Start/stop cues. Default true.
    pub fn sound_feedback_enabled(&self) -> bool {
        self.bool(Key::SoundFeedbackEnabled, true)
    }

    /// Stores `soundFeedbackEnabled`.
    pub fn set_sound_feedback_enabled(&self, enabled: bool) -> Result<(), SettingsError> {
        self.put(Key::SoundFeedbackEnabled, Value::from(enabled))
    }

    /// Whether history is saved. Default true.
    pub fn history_enabled(&self) -> bool {
        self.bool(Key::HistoryEnabled, true)
    }

    /// Stores `historyEnabled`.
    pub fn set_history_enabled(&self, enabled: bool) -> Result<(), SettingsError> {
        self.put(Key::HistoryEnabled, Value::from(enabled))
    }

    /// Whether history keeps recordings. Default false.
    pub fn history_keeps_audio(&self) -> bool {
        self.bool(Key::HistoryKeepsAudio, false)
    }

    /// Stores `historyKeepsAudio`.
    pub fn set_history_keeps_audio(&self, enabled: bool) -> Result<(), SettingsError> {
        self.put(Key::HistoryKeepsAudio, Value::from(enabled))
    }

    /// How long history is kept. Default `month`.
    pub fn history_retention(&self) -> HistoryRetention {
        self.enum_value()
    }

    /// Stores `historyRetention`.
    pub fn set_history_retention(&self, retention: HistoryRetention) -> Result<(), SettingsError> {
        self.put_enum(retention)
    }

    /// Instant-replacement rules. A corrupt stored value is logged and reads
    /// as empty, and stays stored until [`Settings::set_replacement_rules`]
    /// replaces it (REP-7).
    pub fn replacement_rules(&self) -> Vec<ReplacementRule> {
        let raw = self.raw(Key::ReplacementEntries);
        let decoded = decode_rules(raw.as_ref());
        if let Some(error) = decoded.error {
            // serde's message can quote user text, so log only its category
            tracing::error!(
                key = %Key::ReplacementEntries,
                category = ?error.classify(),
                "corrupt replacement rules; reading as empty"
            );
        }
        decoded.rules
    }

    /// Stores `replacementEntries` exactly as given; filter with
    /// `onda_domain::replacements::rules_to_persist` first (REP-12).
    pub fn set_replacement_rules(&self, rules: &[ReplacementRule]) -> Result<(), SettingsError> {
        let value = serde_json::to_value(rules).map_err(SettingsError::Encode)?;
        self.put(Key::ReplacementEntries, value)
    }

    /// "Post-process transcripts". Default false.
    pub fn post_processing_enabled(&self) -> bool {
        self.bool(Key::PostProcessingEnabled, false)
    }

    /// Stores `postProcessingEnabled`.
    pub fn set_post_processing_enabled(&self, enabled: bool) -> Result<(), SettingsError> {
        self.put(Key::PostProcessingEnabled, Value::from(enabled))
    }

    /// `cleanupProvider`; missing or unknown ids read as the default.
    pub fn cleanup_provider(&self) -> CleanupProvider {
        let Some(id) = self.string(Key::CleanupProvider) else {
            return CleanupProvider::default();
        };
        CleanupProvider::from_id(&id).unwrap_or_else(|| {
            tracing::warn!(key = %Key::CleanupProvider, "unknown provider; reading as the default");
            CleanupProvider::default()
        })
    }

    /// Stores `cleanupProvider`.
    pub fn set_cleanup_provider(&self, provider: CleanupProvider) -> Result<(), SettingsError> {
        self.put(Key::CleanupProvider, Value::from(provider.id()))
    }

    /// The model `provider` actually uses (CLN-21): replaced slugs map to
    /// their successor, blank or corrupt values to the default, and any
    /// other slug is kept.
    pub fn cleanup_model(&self, provider: CleanupProvider) -> CleanupModel {
        let stored = model_key(provider).and_then(|key| self.string(key));
        provider.resolve_model(stored.as_deref())
    }

    /// Stores `provider`'s model slug; a provider with one fixed model
    /// stores nothing.
    pub fn set_cleanup_model(
        &self,
        provider: CleanupProvider,
        model: &CleanupModel,
    ) -> Result<(), SettingsError> {
        let Some(key) = model_key(provider) else {
            return Ok(());
        };
        self.put(key, Value::from(model.slug.as_ref()))
    }

    /// `onboardingCompleted`. Default false.
    pub fn onboarding_completed(&self) -> bool {
        self.bool(Key::OnboardingCompleted, false)
    }

    /// `didShowFirstRunSetup`. Default false.
    pub fn did_show_first_run_setup(&self) -> bool {
        self.bool(Key::DidShowFirstRunSetup, false)
    }

    /// First-run onboarding is pending while both onboarding flags are false
    /// (SET-4). This, not `onboardingCompleted` alone, drives automatic
    /// onboarding, "Finish Setup to Dictate", and the launch-time load skip.
    pub fn first_run_pending(&self) -> bool {
        !self.onboarding_completed() && !self.did_show_first_run_setup()
    }

    /// Finishing onboarding sets both flags in one write (SET-4).
    pub fn complete_onboarding(&self) -> Result<(), SettingsError> {
        self.apply(vec![
            (Key::OnboardingCompleted, Some(Value::Bool(true))),
            (Key::DidShowFirstRunSetup, Some(Value::Bool(true))),
        ])
    }

    /// Stores both onboarding flags in one write, e.g. the outcome of closing
    /// onboarding early (ONB-11).
    pub fn set_onboarding_flags(
        &self,
        onboarding_completed: bool,
        did_show_first_run_setup: bool,
    ) -> Result<(), SettingsError> {
        self.apply(vec![
            (
                Key::OnboardingCompleted,
                Some(Value::Bool(onboarding_completed)),
            ),
            (
                Key::DidShowFirstRunSetup,
                Some(Value::Bool(did_show_first_run_setup)),
            ),
        ])
    }

    /// "Run Setup Again" clears only `onboardingCompleted` (SET-4), so first
    /// run is not pending again.
    pub fn rerun_setup(&self) -> Result<(), SettingsError> {
        self.put(Key::OnboardingCompleted, Value::Bool(false))
    }

    /// The `old->new` key of the last model-migration notice shown (DL-14).
    pub fn last_model_migration_notice_key(&self) -> String {
        self.string(Key::LastModelMigrationNoticeKey)
            .unwrap_or_default()
    }

    /// Stores `lastModelMigrationNoticeKey`.
    pub fn set_last_model_migration_notice_key(&self, key: &str) -> Result<(), SettingsError> {
        self.put(Key::LastModelMigrationNoticeKey, Value::from(key))
    }

    /// The `old->new` key of the declined model-migration notice, or empty
    /// when not declined (DL-14).
    pub fn declined_model_migration_notice_key(&self) -> String {
        self.string(Key::DeclinedModelMigrationNoticeKey)
            .unwrap_or_default()
    }

    /// Stores `declinedModelMigrationNoticeKey`; pass `""` to end the
    /// declined state.
    pub fn set_declined_model_migration_notice_key(&self, key: &str) -> Result<(), SettingsError> {
        self.put(Key::DeclinedModelMigrationNoticeKey, Value::from(key))
    }

    fn lock(&self) -> MutexGuard<'_, Map<String, Value>> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn raw(&self, key: Key) -> Option<Value> {
        self.lock().get(key.as_str()).cloned()
    }

    fn string(&self, key: Key) -> Option<String> {
        match self.raw(key)? {
            Value::String(value) => Some(value),
            other => {
                wrong_type(key, &other);
                None
            }
        }
    }

    fn bool(&self, key: Key, default: bool) -> bool {
        match self.raw(key) {
            None => default,
            Some(Value::Bool(value)) => value,
            Some(other) => {
                wrong_type(key, &other);
                default
            }
        }
    }

    fn enum_value<E: SettingEnum>(&self) -> E {
        let Some(raw) = self.string(E::KEY) else {
            return E::default();
        };
        E::from_setting(&raw).unwrap_or_else(|| {
            tracing::warn!(key = %E::KEY, "unrecognized value; using the default");
            E::default()
        })
    }

    fn put(&self, key: Key, value: Value) -> Result<(), SettingsError> {
        self.apply(vec![(key, Some(value))])
    }

    fn put_enum<E: SettingEnum>(&self, value: E) -> Result<(), SettingsError> {
        self.put(E::KEY, Value::from(value.as_setting()))
    }

    // `None` removes the key. Saves only when something changed, and updates
    // the cache only after the file is saved.
    fn apply(&self, changes: Vec<(Key, Option<Value>)>) -> Result<(), SettingsError> {
        let changed = {
            let mut values = self.lock();
            let mut next = values.clone();
            let mut changed = Vec::new();
            for (key, value) in changes {
                if next.get(key.as_str()) == value.as_ref() {
                    continue;
                }
                match value {
                    Some(value) => next.insert(key.as_str().to_owned(), value),
                    None => next.remove(key.as_str()),
                };
                changed.push(key);
            }
            if changed.is_empty() {
                return Ok(());
            }
            if let Err(error) = write_atomic(&self.path, &next) {
                let keys: Vec<&str> = changed.iter().map(|key| key.as_str()).collect();
                tracing::error!(path = %self.path.display(), ?keys, %error, "couldn't save settings");
                return Err(error);
            }
            *values = next;
            changed
        };
        if let Some(observer) = &self.observer {
            changed.into_iter().for_each(observer);
        }
        Ok(())
    }
}

fn model_key(provider: CleanupProvider) -> Option<Key> {
    match provider {
        CleanupProvider::OpenRouter => Some(Key::OpenRouterModel),
        CleanupProvider::Fireworks => Some(Key::FireworksModel),
        CleanupProvider::AppleIntelligence => None,
    }
}

fn wrong_type(key: Key, value: &Value) {
    let found = match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    tracing::warn!(%key, found, "setting has the wrong type; using the default");
}

fn load(path: &Path) -> Map<String, Value> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Map::new(),
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "couldn't read settings; using defaults");
            return Map::new();
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(values)) => values,
        Ok(_) => {
            tracing::error!(path = %path.display(), "settings are not a JSON object; using defaults");
            Map::new()
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "settings are not valid JSON; using defaults");
            Map::new()
        }
    }
}

fn write_atomic(path: &Path, values: &Map<String, Value>) -> Result<(), SettingsError> {
    let bytes = serde_json::to_vec_pretty(values).map_err(SettingsError::Encode)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path);
    let result = write_and_rename(&tmp, path, &bytes);
    if result.is_err()
        && let Err(error) = fs::remove_file(&tmp)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(path = %tmp.display(), %error, "couldn't remove temporary settings file");
    }
    result.map_err(SettingsError::Io)
}

fn write_and_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

// stored string forms of the domain enums; the domain has no canonical
// mapping, so it lives here with the keys
trait SettingEnum: Copy + Default {
    const KEY: Key;

    fn as_setting(self) -> &'static str;

    fn from_setting(value: &str) -> Option<Self>;
}

impl SettingEnum for OutputMode {
    const KEY: Key = Key::OutputMode;

    fn as_setting(self) -> &'static str {
        match self {
            OutputMode::Type => "type",
            OutputMode::Clipboard => "clipboard",
        }
    }

    fn from_setting(value: &str) -> Option<Self> {
        match value {
            "type" => Some(OutputMode::Type),
            "clipboard" => Some(OutputMode::Clipboard),
            _ => None,
        }
    }
}

impl SettingEnum for ActivationMode {
    const KEY: Key = Key::HotkeyActivationMode;

    fn as_setting(self) -> &'static str {
        match self {
            ActivationMode::Hold => "hold",
            ActivationMode::Toggle => "toggle",
            ActivationMode::Automatic => "automatic",
        }
    }

    fn from_setting(value: &str) -> Option<Self> {
        match value {
            "hold" => Some(ActivationMode::Hold),
            "toggle" => Some(ActivationMode::Toggle),
            "automatic" => Some(ActivationMode::Automatic),
            _ => None,
        }
    }
}

impl SettingEnum for HistoryRetention {
    const KEY: Key = Key::HistoryRetention;

    fn as_setting(self) -> &'static str {
        match self {
            HistoryRetention::Week => "week",
            HistoryRetention::Month => "month",
            HistoryRetention::Forever => "forever",
        }
    }

    fn from_setting(value: &str) -> Option<Self> {
        match value {
            "week" => Some(HistoryRetention::Week),
            "month" => Some(HistoryRetention::Month),
            "forever" => Some(HistoryRetention::Forever),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_forms_round_trip() {
        for mode in [OutputMode::Type, OutputMode::Clipboard] {
            assert_eq!(OutputMode::from_setting(mode.as_setting()), Some(mode));
        }
        for mode in [
            ActivationMode::Hold,
            ActivationMode::Toggle,
            ActivationMode::Automatic,
        ] {
            assert_eq!(ActivationMode::from_setting(mode.as_setting()), Some(mode));
        }
        for retention in [
            HistoryRetention::Week,
            HistoryRetention::Month,
            HistoryRetention::Forever,
        ] {
            assert_eq!(
                HistoryRetention::from_setting(retention.as_setting()),
                Some(retention)
            );
        }
    }

    #[test]
    fn temp_path_is_a_sibling() {
        let got = temp_path(Path::new("/greendale/Onda/settings.json"));
        assert_eq!(got, PathBuf::from("/greendale/Onda/settings.json.tmp"));
    }
}
