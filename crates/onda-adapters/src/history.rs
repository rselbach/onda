//! Transcript history through `onda-history`.

use std::path::PathBuf;
use std::sync::Arc;

use jiff::Timestamp;
use onda_app::seams::{HistorySink, RetentionRequest};
use onda_domain::history::{HistoryEntry, NewEntry, RecentTranscript, recent_transcripts};
use onda_history::{HistoryPreferences, HistoryStore};
use onda_settings::Settings;
use uuid::Uuid;

/// [`HistorySink`] over a [`HistoryStore`], reading current settings at the
/// start of each history write (HIS-1).
pub struct History {
    store: Arc<HistoryStore>,
    settings: Arc<Settings>,
}

impl History {
    /// Saves to `store` with preferences from `settings`.
    pub fn new(store: Arc<HistoryStore>, settings: Arc<Settings>) -> Self {
        Self { store, settings }
    }

    fn preferences(&self) -> HistoryPreferences {
        let stored = self.settings.reopen();
        HistoryPreferences {
            enabled: stored.history_enabled(),
            keep_audio: stored.history_keeps_audio(),
            retention: stored.history_retention(),
        }
    }
}

impl HistorySink for History {
    fn save(&self, entry: NewEntry, samples: &[f32]) -> Result<bool, String> {
        self.store
            .save(entry, samples, || self.preferences(), Timestamp::now())
            .map(|saved| saved.is_some())
            .map_err(|error| error.to_string())
    }

    fn recent_transcripts(&self) -> Vec<RecentTranscript> {
        match self.store.entries() {
            Ok(entries) => recent_transcripts(&entries, self.settings.reopen().history_enabled()),
            Err(error) => {
                tracing::error!(%error, "could not read history for the menu");
                Vec::new()
            }
        }
    }

    fn entries(&self) -> Result<Vec<HistoryEntry>, String> {
        self.store.entries().map_err(|error| error.to_string())
    }

    fn delete(&self, id: Uuid) -> Result<(), String> {
        self.store.delete(id).map_err(|error| error.to_string())
    }

    fn delete_all(&self) -> Result<(), String> {
        self.store.delete_all().map_err(|error| error.to_string())
    }

    fn apply_retention(&self, request: RetentionRequest, now: Timestamp) -> Result<bool, String> {
        self.store
            .apply_retention(
                || request.resolve(self.settings.reopen().history_retention()),
                now,
            )
            .map_err(|error| error.to_string())
    }

    fn audio_path(&self, id: Uuid) -> Result<PathBuf, String> {
        self.store.audio_path(id).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use onda_domain::history::{AUDIO_FILENAME, METADATA_FILENAME, entry_directory_name};
    use onda_domain::types::HistoryRetention;
    use onda_history::HistoryHooks;

    use super::*;

    const SAMPLES: &[f32] = &[0.0, 0.5, -0.5];

    struct Hooks;

    impl HistoryHooks for Hooks {
        fn history_changed(&self) {}

        fn exclude_from_backup(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn new_entry(id: u128, timestamp: Timestamp) -> NewEntry {
        NewEntry {
            id: Uuid::from_u128(id),
            timestamp,
            sample_count: SAMPLES.len(),
            model_id: "parakeet-tdt-v2".into(),
            model_name: None,
            language: "en".into(),
            transcript: "Troy Barnes studies at Greendale Community College.".into(),
            raw_transcript: None,
            was_post_processed: false,
            post_processing_error_description: None,
            app_name: None,
            app_bundle_id: None,
            audio_byte_count: None,
        }
    }

    fn history(root: &Path, settings: Arc<Settings>) -> History {
        History::new(
            Arc::new(HistoryStore::new(root.join("History"), Hooks)),
            settings,
        )
    }

    #[test]
    fn save_honors_an_independent_history_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Arc::new(Settings::open(&path));
        let history = history(dir.path(), settings.clone());
        Settings::open(&path).set_history_enabled(false).unwrap();

        assert!(
            !history
                .save(new_entry(1, Timestamp::now()), SAMPLES)
                .unwrap()
        );
        assert!(!history.store.directory().exists());
        assert!(settings.history_enabled(), "session cache stays unchanged");
    }

    #[test]
    fn recent_transcripts_follow_independent_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let history = history(dir.path(), Arc::new(Settings::open(&path)));
        assert!(
            history
                .save(new_entry(1, Timestamp::now()), SAMPLES)
                .unwrap()
        );
        assert_eq!(history.recent_transcripts().len(), 1);

        let writer = Settings::open(&path);
        writer.set_history_enabled(false).unwrap();
        assert!(history.recent_transcripts().is_empty());
        assert_eq!(history.entries().unwrap().len(), 1);

        writer.set_history_enabled(true).unwrap();
        assert_eq!(history.recent_transcripts().len(), 1);
    }

    #[test]
    fn delayed_retention_change_does_not_prune_after_the_setting_changes_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Arc::new(Settings::open(&path));
        let history = history(dir.path(), settings.clone());
        let now = Timestamp::now();
        let old = now
            .checked_sub(Duration::from_secs(8 * 24 * 60 * 60))
            .unwrap();
        assert!(history.save(new_entry(1, old), SAMPLES).unwrap());
        settings
            .set_history_retention(HistoryRetention::Week)
            .unwrap();
        Settings::open(&path)
            .set_history_retention(HistoryRetention::Forever)
            .unwrap();

        assert!(
            !history
                .apply_retention(RetentionRequest::IfCurrent(HistoryRetention::Week), now)
                .unwrap()
        );
        assert_eq!(history.entries().unwrap().len(), 1);
    }

    #[test]
    fn automatic_pruning_uses_current_retention() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Arc::new(Settings::open(&path));
        let history = history(dir.path(), settings.clone());
        let now = Timestamp::now();
        let old = now
            .checked_sub(Duration::from_secs(8 * 24 * 60 * 60))
            .unwrap();
        assert!(history.save(new_entry(1, old), SAMPLES).unwrap());
        settings
            .set_history_retention(HistoryRetention::Week)
            .unwrap();
        let writer = Settings::open(&path);

        writer
            .set_history_retention(HistoryRetention::Forever)
            .unwrap();
        assert!(
            !history
                .apply_retention(RetentionRequest::Current, now)
                .unwrap()
        );
        fs::write(&path, "{").unwrap();
        assert!(
            !history
                .apply_retention(RetentionRequest::Current, now)
                .unwrap()
        );
        assert_eq!(history.entries().unwrap().len(), 1);

        writer
            .set_history_retention(HistoryRetention::Week)
            .unwrap();
        assert!(
            history
                .apply_retention(RetentionRequest::Current, now)
                .unwrap()
        );
        assert!(history.entries().unwrap().is_empty());
    }

    #[test]
    fn save_uses_defaults_without_creating_a_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let history = history(dir.path(), Arc::new(Settings::open(&path)));
        let entry = new_entry(1, Timestamp::now());
        let folder = history
            .store
            .directory()
            .join(entry_directory_name(entry.timestamp, entry.id).as_str());

        assert!(history.save(entry, SAMPLES).unwrap());
        assert!(folder.join(METADATA_FILENAME).is_file());
        assert!(!folder.join(AUDIO_FILENAME).exists());
        assert!(!path.exists());
    }

    #[test]
    fn save_requires_opt_in_after_corrupt_settings_are_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{}").unwrap();
        let history = history(dir.path(), Arc::new(Settings::open(&path)));
        fs::write(&path, "{").unwrap();
        let now = Timestamp::now();

        assert!(!history.save(new_entry(1, now), SAMPLES).unwrap());
        assert!(!history.store.directory().exists());
        assert_eq!(fs::read(&path).unwrap(), b"{");

        let writer = Settings::open(&path);
        writer.set_history_keeps_audio(true).unwrap();
        assert!(!history.save(new_entry(2, now), SAMPLES).unwrap());
        assert!(!history.store.directory().exists());

        writer.set_history_enabled(true).unwrap();
        let entry = new_entry(3, now);
        let folder = history
            .store
            .directory()
            .join(entry_directory_name(entry.timestamp, entry.id).as_str());
        assert!(history.save(entry, SAMPLES).unwrap());
        assert!(folder.join(METADATA_FILENAME).is_file());
        assert!(folder.join(AUDIO_FILENAME).is_file());
    }

    #[test]
    fn save_honors_independent_audio_and_retention_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let writer = Settings::open(&path);
        writer.set_history_keeps_audio(true).unwrap();
        writer
            .set_history_retention(HistoryRetention::Forever)
            .unwrap();
        let settings = Arc::new(Settings::open(&path));
        let history = history(dir.path(), settings.clone());
        let now = Timestamp::now();
        let old = new_entry(
            1,
            now.checked_sub(Duration::from_secs(8 * 24 * 60 * 60))
                .unwrap(),
        );
        let old_folder = history
            .store
            .directory()
            .join(entry_directory_name(old.timestamp, old.id).as_str());
        assert!(history.save(old, SAMPLES).unwrap());
        assert!(old_folder.join(AUDIO_FILENAME).is_file());

        writer.set_history_keeps_audio(false).unwrap();
        writer
            .set_history_retention(HistoryRetention::Week)
            .unwrap();
        let new = new_entry(2, now);
        let new_folder = history
            .store
            .directory()
            .join(entry_directory_name(new.timestamp, new.id).as_str());
        assert!(history.save(new, SAMPLES).unwrap());
        assert!(new_folder.join(METADATA_FILENAME).is_file());
        assert!(!new_folder.join(AUDIO_FILENAME).exists());
        assert!(!old_folder.exists());
        assert!(
            settings.history_keeps_audio(),
            "session cache stays unchanged"
        );
        assert_eq!(settings.history_retention(), HistoryRetention::Forever);
    }
}
