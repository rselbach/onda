//! The settings store over `settings.json` (SET-1..SET-4).

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use onda_domain::cleanup::{CleanupModel, CleanupPrompt, CleanupProvider};
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
/// The file is read at [`Settings::open`] and cached. Every setter takes an
/// exclusive lock on the sibling `settings.json.lock`, rereads the file,
/// applies only its own keys, and replaces the file atomically through a
/// uniquely named temp file. Other instances and processes may write the same
/// file: a write never undoes theirs, keys this version does not know are
/// kept, and the cache picks up the file's latest contents on each write.
/// Getters never write and never fail: a missing key is the default, a
/// corrupt value is the default plus a log line (SET-2), and a corrupt or
/// unreadable file reads as empty.
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

/// A setter could not save `settings.json`. The stored values are
/// unchanged.
#[derive(Debug)]
pub enum SettingsError {
    /// Locking, reading, creating, writing, syncing, or renaming the file
    /// failed. An unreadable file is never replaced.
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
    /// read as empty. The next setter copies a corrupt file to
    /// `settings.json.corrupt-<random>` before replacing it, and fails rather than
    /// replace a file it cannot read. The parent directory is created on
    /// first write.
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
    /// a key to the value the file already has saves nothing and reports
    /// nothing; keys other writers changed are not reported. The observer
    /// runs on the caller's thread without the store or file locked, so it
    /// may read and write settings.
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
        self.apply(|| vec![(key, None)]).map(drop)
    }

    /// Rewrites stale enum-like values to their canonical form at launch
    /// (SET-3): `outputMode`, `hotkeyActivationMode`, `historyRetention`,
    /// and `openRouterModel` (replaced slugs map to their successor, blank
    /// ones to the default; CLN-21). Unset keys stay unset. Idempotent:
    /// returns the keys it rewrote, empty once everything is canonical.
    pub fn canonicalize(&self) -> Result<Vec<Key>, SettingsError> {
        self.apply(|| {
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
            canonical
                .into_iter()
                .filter(|(key, value)| self.raw(*key).is_some_and(|raw| raw != *value))
                .map(|(key, value)| (key, Some(Value::from(value))))
                .collect()
        })
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
        let mut invalid = None;
        self.apply(|| {
            let stored = self.string(Key::SelectedLanguage);
            let Some(StoredLanguage::Invalid { code }) =
                stored.as_deref().map(resolve_stored_language)
            else {
                return Vec::new();
            };
            invalid = Some(code);
            let auto = Value::from(LanguageSetting::Auto.code());
            vec![(Key::SelectedLanguage, Some(auto))]
        })?;
        let Some(code) = invalid else {
            return Ok(None);
        };
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

    /// The dictation shortcut. `default` is the platform's (HOT-2); a
    /// stored value that does not parse or fails HOT-4 reads as it too
    /// (HOT-5).
    pub fn hotkey(&self, default: &Shortcut) -> Shortcut {
        let Some(raw) = self.raw(Key::Hotkey) else {
            return default.clone();
        };
        Shortcut::from_setting(&raw).unwrap_or_else(|error| {
            tracing::warn!(key = %Key::Hotkey, %error, "invalid setting; using the default");
            default.clone()
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

    /// How long history is kept. Default `month`. A stored value this build
    /// does not recognize reads as `forever`: it may come from a newer build,
    /// and a guessed shorter limit would delete history.
    pub fn history_retention(&self) -> HistoryRetention {
        if self.raw(Key::HistoryRetention).is_none() {
            return HistoryRetention::default();
        }
        self.string(Key::HistoryRetention)
            .and_then(|raw| HistoryRetention::from_setting(&raw))
            .unwrap_or_else(|| {
                tracing::warn!(key = %Key::HistoryRetention, "unrecognized value; keeping all history");
                HistoryRetention::Forever
            })
    }

    /// Stores `historyRetention`.
    pub fn set_history_retention(&self, retention: HistoryRetention) -> Result<(), SettingsError> {
        self.put_enum(retention)
    }

    /// Instant-replacement rules. Corrupt rows are logged and left out; a
    /// value that is not a list reads as empty. The stored value stays as
    /// it is until [`Settings::set_replacement_rules`] replaces it (REP-7).
    pub fn replacement_rules(&self) -> Vec<ReplacementRule> {
        let raw = self.raw(Key::ReplacementEntries);
        let decoded = decode_rules(raw.as_ref());
        if let Some(error) = decoded.error {
            // serde's message can quote user text, so log only its category
            tracing::error!(
                key = %Key::ReplacementEntries,
                category = ?error.classify(),
                "corrupt replacement rules; reading the rows that decode"
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

    /// The cleanup prompt in use, for every provider and model: the stored
    /// `cleanupPrompt`, or Onda's default when it is unset, blank, or
    /// corrupt.
    pub fn cleanup_prompt(&self) -> CleanupPrompt {
        let Some(text) = self.string(Key::CleanupPrompt) else {
            return CleanupPrompt::default();
        };
        CleanupPrompt::from_text(&text).unwrap_or_else(|_blank| {
            tracing::warn!(key = %Key::CleanupPrompt, "blank prompt; using the default");
            CleanupPrompt::default()
        })
    }

    /// Stores a custom prompt as `cleanupPrompt`. The default removes the
    /// key, so a later version's default is adopted.
    pub fn set_cleanup_prompt(&self, prompt: &CleanupPrompt) -> Result<(), SettingsError> {
        let value = prompt.custom().map(Value::from);
        self.apply(|| vec![(Key::CleanupPrompt, value)]).map(drop)
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
        self.apply(|| {
            vec![
                (Key::OnboardingCompleted, Some(Value::Bool(true))),
                (Key::DidShowFirstRunSetup, Some(Value::Bool(true))),
            ]
        })
        .map(drop)
    }

    /// Stores both onboarding flags in one write, e.g. the outcome of closing
    /// onboarding early (ONB-11).
    pub fn set_onboarding_flags(
        &self,
        onboarding_completed: bool,
        did_show_first_run_setup: bool,
    ) -> Result<(), SettingsError> {
        self.apply(|| {
            vec![
                (
                    Key::OnboardingCompleted,
                    Some(Value::Bool(onboarding_completed)),
                ),
                (
                    Key::DidShowFirstRunSetup,
                    Some(Value::Bool(did_show_first_run_setup)),
                ),
            ]
        })
        .map(drop)
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
        self.apply(|| vec![(key, Some(value))]).map(drop)
    }

    fn put_enum<E: SettingEnum>(&self, value: E) -> Result<(), SettingsError> {
        self.put(E::KEY, Value::from(value.as_setting()))
    }

    // Under the file lock: reread the file into the cache, ask `changes` for
    // this write's keys (it may call getters, which now see the file), and
    // save only if one differs from the file. `None` removes the key. Returns
    // the changed keys, which are reported after both locks are released.
    fn apply(
        &self,
        changes: impl FnOnce() -> Vec<(Key, Option<Value>)>,
    ) -> Result<Vec<Key>, SettingsError> {
        let changed = {
            let _file_lock = lock_file(&self.path)?;
            let stored = reload(&self.path)?;
            let corrupt = stored.is_none();
            let mut next = stored.unwrap_or_default();
            *self.lock() = next.clone();
            let mut changed = Vec::new();
            for (key, value) in changes() {
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
                return Ok(changed);
            }
            if let Err(error) = save(&self.path, &next, corrupt) {
                let keys: Vec<&str> = changed.iter().map(|key| key.as_str()).collect();
                tracing::error!(path = %self.path.display(), ?keys, %error, "couldn't save settings");
                return Err(error);
            }
            *self.lock() = next;
            changed
        };
        if let Some(observer) = &self.observer {
            changed.iter().copied().for_each(observer);
        }
        Ok(changed)
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
    match read(path) {
        Ok(Ok(values)) => values,
        Ok(Err(problem)) => {
            tracing::error!(path = %path.display(), problem, "corrupt settings; using defaults");
            Map::new()
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "couldn't read settings; using defaults");
            Map::new()
        }
    }
}

// the file's settings for a write: `None` when it is corrupt, an error when
// it cannot be read, since replacing it would lose settings we never saw
fn reload(path: &Path) -> Result<Option<Map<String, Value>>, SettingsError> {
    match read(path) {
        Ok(Ok(values)) => Ok(Some(values)),
        Ok(Err(problem)) => {
            tracing::error!(path = %path.display(), problem, "corrupt settings; replacing them");
            Ok(None)
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "couldn't reread settings; not saving");
            Err(SettingsError::Io(error))
        }
    }
}

// the outer error is I/O; the inner one says why the contents are corrupt
fn read(path: &Path) -> io::Result<Result<Map<String, Value>, &'static str>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Ok(Map::new())),
        Err(error) => return Err(error),
    };
    Ok(match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(values)) => Ok(values),
        Ok(_) => Err("not a JSON object"),
        Err(_) => Err("not valid JSON"),
    })
}

// an exclusive lock on `settings.json.lock`, held until the file is dropped;
// the lock file is never removed, so every writer locks the same inode
fn lock_file(path: &Path) -> Result<File, SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = sibling(path, ".lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    file.lock()?;
    Ok(file)
}

// replaces the file through a uniquely named sibling temp file, first copying
// corrupt contents to a unique backup without moving the original
fn save(path: &Path, values: &Map<String, Value>, corrupt: bool) -> Result<(), SettingsError> {
    let bytes = serde_json::to_vec_pretty(values).map_err(SettingsError::Encode)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let prefix = sibling(path, ".");
    let mut tmp = tempfile::Builder::new()
        .prefix(prefix.file_name().unwrap_or_default())
        .suffix(".tmp")
        .tempfile_in(parent)?;
    tmp.write_all(&bytes)?;
    tmp.as_file().sync_all()?;
    if corrupt {
        let backup_prefix = sibling(path, ".corrupt-");
        let mut backup = tempfile::Builder::new()
            .prefix(backup_prefix.file_name().unwrap_or_default())
            .tempfile_in(parent)?;
        let mut original = File::open(path)?;
        io::copy(&mut original, &mut backup)?;
        backup.as_file().sync_all()?;
        let (_, backup_path) = backup.keep().map_err(|error| error.error)?;
        tracing::warn!(path = %backup_path.display(), "preserved corrupt settings");
    }
    tmp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
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
    fn sibling_shares_the_folder() {
        let got = sibling(Path::new("/greendale/Onda/settings.json"), ".lock");
        assert_eq!(got, PathBuf::from("/greendale/Onda/settings.json.lock"));
    }
}
