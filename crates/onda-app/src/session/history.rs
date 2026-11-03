//! The History page inside the session actor (HIS-16..HIS-27): loading
//! with retention, search, the retention and clear confirmations, copying,
//! recordings, deletes, and errors. Days follow the system time zone.

use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use onda_domain::history::{
    Confirmation, HistoryEntry, clear_history_confirmation, copy_text, original_transcript,
    retention_change_confirmation,
};
use onda_domain::messages::UserMessage;
use onda_domain::types::HistoryRetention;
use tokio::task::spawn_blocking;
use uuid::Uuid;

use super::{Actor, Msg, Task, UiMsg};
use crate::events::Event;
use crate::ports::{CopyOutcome, Delivery};
use crate::seams::{HistorySink, SettingChange};
use crate::views::{HistoryInputs, HistoryPage, MainWindowSection, history_page};

/// How long a row's copy button shows the checkmark (HIS-22).
pub(crate) const COPIED_DURATION: Duration = Duration::from_millis(1500);

/// A confirmation the History page is asking.
pub(super) enum HistoryDialog {
    /// Shortening retention would delete entries (HIS-17).
    Retention(HistoryRetention, Confirmation),
    /// "Clear History…" (HIS-20).
    Clear(Confirmation),
}

impl HistoryDialog {
    fn confirmation(&self) -> &Confirmation {
        match self {
            Self::Retention(_, confirmation) | Self::Clear(confirmation) => confirmation,
        }
    }
}

// everything the page view depends on besides settings, to skip rebuilding
// a long list when nothing changed
#[derive(Clone, PartialEq)]
struct ViewKey {
    entries: u64,
    query: String,
    enabled: bool,
    keep_audio: bool,
    retention: HistoryRetention,
    dialog: Option<String>,
    error: Option<String>,
    copied: Option<Uuid>,
    today: Date,
}

/// History page state.
#[derive(Default)]
pub(super) struct HistoryState {
    entries: Vec<HistoryEntry>,
    entries_version: u64,
    // bumped when a mutation starts or finishes, so a read begun earlier
    // cannot overwrite the page or the menu with what it removed
    pub(super) generation: u64,
    // the latest page read; an earlier one landing after it is dropped
    page_read: u64,
    pub(super) query: String,
    pub(super) dialog: Option<HistoryDialog>,
    pub(super) error: Option<String>,
    copied: Option<Uuid>,
    copied_generation: u64,
    copied_timer: Option<Task>,
    built: Option<(ViewKey, HistoryPage)>,
}

impl Actor {
    /// Every history change: the menu and a visible History page re-read
    /// (HIS-10).
    pub(super) fn history_changed(&mut self) {
        self.ui.history.generation += 1;
        self.events.emit(Event::HistoryChanged);
        self.refresh_recent();
        if self.ui.is_visible(MainWindowSection::History) {
            self.reload_history(false);
        }
    }

    // HIS-16: retention also applies when the page appears
    pub(super) fn reload_history(&mut self, apply_retention: bool) {
        let history = self.ports.history.clone();
        let retention = self.settings.history_retention;
        let messages = self.messages.clone();
        let generation = self.ui.history.generation;
        self.ui.history.page_read += 1;
        let read = self.ui.history.page_read;
        tokio::spawn(async move {
            let loaded = spawn_blocking(move || {
                let retained = match apply_retention {
                    true => history.apply_retention(retention, Timestamp::now()),
                    false => Ok(false),
                };
                (retained, history.entries())
            })
            .await;
            let (retained, entries) = match loaded {
                Ok(loaded) => loaded,
                Err(error) => (Ok(false), Err(error.to_string())),
            };
            match retained {
                Ok(false) => {}
                // HIS-10: what retention removed leaves the menu too
                Ok(true) => {
                    let _ = messages.send(Msg::Ui(UiMsg::HistoryOpDone(Ok(()))));
                }
                Err(error) => {
                    let _ = messages.send(Msg::Ui(UiMsg::HistoryOpDone(Err(error))));
                }
            }
            let _ = messages.send(Msg::Ui(UiMsg::HistoryLoaded {
                generation,
                read,
                result: entries,
            }));
        });
    }

    pub(super) fn history_loaded(
        &mut self,
        generation: u64,
        read: u64,
        result: Result<Vec<HistoryEntry>, String>,
    ) {
        if generation != self.ui.history.generation || read != self.ui.history.page_read {
            return;
        }
        match result {
            Ok(entries) => {
                self.ui.history.entries = entries;
                self.ui.history.entries_version += 1;
            }
            Err(error) => {
                tracing::error!(%error, "could not read history");
                self.ui.history.error = Some(error);
            }
        }
    }

    fn history_op(
        &mut self,
        op: impl FnOnce(&dyn HistorySink) -> Result<(), String> + Send + 'static,
    ) {
        self.ui.history.generation += 1;
        let history = self.ports.history.clone();
        let messages = self.messages.clone();
        tokio::spawn(async move {
            let result = spawn_blocking(move || op(history.as_ref()))
                .await
                .unwrap_or_else(|error| Err(error.to_string()));
            let _ = messages.send(Msg::Ui(UiMsg::HistoryOpDone(result)));
        });
    }

    // HIS-27: errors show as "History Error"
    pub(super) fn history_op_done(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            tracing::error!(%error, "history operation failed");
            self.ui.history.error = Some(error);
        }
        self.history_changed();
    }

    // HIS-17: ask before a shorter retention deletes entries
    pub(super) fn set_history_retention(&mut self, retention: HistoryRetention) {
        if retention == self.settings.history_retention {
            return;
        }
        match retention_change_confirmation(&self.ui.history.entries, retention, Timestamp::now()) {
            Some(confirmation) => {
                self.ui.history.dialog = Some(HistoryDialog::Retention(retention, confirmation));
            }
            None => self.apply_retention_setting(retention),
        }
    }

    fn apply_retention_setting(&mut self, retention: HistoryRetention) {
        self.apply_setting(SettingChange::HistoryRetention(retention));
        self.history_op(move |history| {
            history
                .apply_retention(retention, Timestamp::now())
                .map(|_| ())
        });
    }

    pub(super) fn request_clear_history(&mut self) {
        let count = self.ui.history.entries.len();
        if count == 0 {
            return;
        }
        self.ui.history.dialog = Some(HistoryDialog::Clear(clear_history_confirmation(count)));
    }

    pub(super) fn confirm_history_dialog(&mut self) {
        match self.ui.history.dialog.take() {
            Some(HistoryDialog::Retention(retention, _)) => self.apply_retention_setting(retention),
            Some(HistoryDialog::Clear(_)) => self.history_op(|history| history.delete_all()),
            None => {}
        }
    }

    pub(super) fn delete_entry(&mut self, id: Uuid) {
        self.history_op(move |history| history.delete(id));
    }

    // HIS-25: every copy path trims
    pub(super) fn copy_entry(&mut self, id: Uuid, original: bool) {
        let Some(entry) = self.ui.history.entries.iter().find(|entry| entry.id == id) else {
            return;
        };
        let text = match original {
            true => original_transcript(entry),
            false => Some(copy_text(entry)),
        };
        let Some(text) = text.map(str::to_owned) else {
            return;
        };
        let output = self.ports.text_output.clone();
        let history = &mut self.ui.history;
        history.copied = None;
        history.copied_timer = None;
        history.copied_generation += 1;
        let generation = history.copied_generation;
        let messages = self.messages.clone();
        // a history copy belongs to no session, so nothing cancels it
        tokio::spawn(async move {
            let outcome = spawn_blocking(move || output.copy(&text, &Delivery::new()))
                .await
                .unwrap_or(CopyOutcome::Failed);
            let _ = messages.send(Msg::Ui(UiMsg::HistoryCopied {
                id,
                generation,
                outcome,
            }));
        });
    }

    pub(super) fn history_copied(&mut self, id: Uuid, generation: u64, outcome: CopyOutcome) {
        if generation != self.ui.history.copied_generation {
            return;
        }
        match outcome {
            CopyOutcome::Copied => {}
            CopyOutcome::Cancelled => return,
            CopyOutcome::Failed => {
                tracing::warn!("could not copy a history entry");
                self.ui.history.error = Some(
                    UserMessage::CopyFailed.body(&self.ports.tray_term, self.ports.privacy_naming),
                );
                return;
            }
        }
        let history = &mut self.ui.history;
        history.copied = Some(id);
        self.ui.history.copied_timer =
            Some(self.timer(COPIED_DURATION, Msg::Ui(UiMsg::CopiedElapsed(generation))));
    }

    pub(super) fn copied_elapsed(&mut self, generation: u64) {
        if generation == self.ui.history.copied_generation {
            self.ui.history.copied = None;
            self.ui.history.copied_timer = None;
        }
    }

    // HIS-24: play with the default app or reveal in the file manager
    pub(super) fn open_recording(&mut self, id: Uuid, reveal: bool) {
        let history = self.ports.history.clone();
        let files = self.ports.files.clone();
        let messages = self.messages.clone();
        tokio::spawn(async move {
            let opened = spawn_blocking(move || {
                let path = history.audio_path(id)?;
                match reveal {
                    true => files.reveal(&path),
                    false => files.open(&path),
                }
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            if let Err(error) = opened {
                let _ = messages.send(Msg::Ui(UiMsg::HistoryOpDone(Err(error))));
            }
        });
    }

    /// The page view, or `None` when nothing it shows changed since the
    /// last one published.
    pub(super) fn history_view(&mut self) -> Option<HistoryPage> {
        let tz = TimeZone::system();
        let now = Timestamp::now();
        let history = &self.ui.history;
        let key = ViewKey {
            entries: history.entries_version,
            query: history.query.clone(),
            enabled: self.settings.history_enabled,
            keep_audio: self.settings.keep_audio,
            retention: self.settings.history_retention,
            dialog: history
                .dialog
                .as_ref()
                .map(|d| d.confirmation().title.clone()),
            error: history.error.clone(),
            copied: history.copied,
            today: now.to_zoned(tz.clone()).date(),
        };
        if history
            .built
            .as_ref()
            .is_some_and(|(built, _)| *built == key)
        {
            return None;
        }
        let view = history_page(HistoryInputs {
            entries: &history.entries,
            query: &history.query,
            history_enabled: self.settings.history_enabled,
            keep_audio: self.settings.keep_audio,
            retention: self.settings.history_retention,
            now,
            tz: &tz,
            dialog: history.dialog.as_ref().map(HistoryDialog::confirmation),
            error: history.error.as_deref(),
            copied: history.copied,
        });
        self.ui.history.built = Some((key, view.clone()));
        Some(view)
    }

    /// Forgets the last built view so the next publish sends a fresh one.
    pub(super) fn forget_history_view(&mut self) {
        self.ui.history.built = None;
    }
}
