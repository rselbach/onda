//! Transcript history through `onda-history`.

use std::path::PathBuf;
use std::sync::Arc;

use jiff::Timestamp;
use onda_app::seams::HistorySink;
use onda_domain::history::{HistoryEntry, NewEntry, RecentTranscript, recent_transcripts};
use onda_domain::types::HistoryRetention;
use onda_history::{HistoryPreferences, HistoryStore};
use onda_settings::Settings;
use uuid::Uuid;

/// [`HistorySink`] over a [`HistoryStore`], reading preferences at save
/// time (HIS-1).
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
        HistoryPreferences {
            enabled: self.settings.history_enabled(),
            keep_audio: self.settings.history_keeps_audio(),
            retention: self.settings.history_retention(),
        }
    }
}

impl HistorySink for History {
    fn save(&self, entry: NewEntry, audio: Option<Arc<[f32]>>) -> Result<(), String> {
        let samples = audio.as_deref().unwrap_or(&[]);
        self.store
            .save(entry, samples, self.preferences(), Timestamp::now())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn recent_transcripts(&self) -> Vec<RecentTranscript> {
        match self.store.entries() {
            Ok(entries) => recent_transcripts(&entries, self.settings.history_enabled()),
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

    fn apply_retention(&self, retention: HistoryRetention, now: Timestamp) -> Result<bool, String> {
        self.store
            .apply_retention(retention, now)
            .map_err(|error| error.to_string())
    }

    fn audio_path(&self, id: Uuid) -> Result<PathBuf, String> {
        self.store.audio_path(id).map_err(|error| error.to_string())
    }
}
