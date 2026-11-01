//! Transcript history rules (prd/06): the entry metadata format, folder
//! naming and path safety, load-time cleanup, retention and audio budgets,
//! WAV encoding, the History page listing, and the tray's recent transcripts.
//!
//! Everything here is pure (ARCH-2). The storage crate reads and writes the
//! files and calls in with what it found; "now", ids, and the local time
//! zone are parameters.

mod copy;
mod entry;
mod listing;
mod recent;
mod retention;
mod wav;

pub use copy::{
    CLEAR_HISTORY_BUTTON, CLEAR_HISTORY_TOOLTIP, COPIED_LABEL, COPY_LABEL, Confirmation,
    HISTORY_ERROR_TITLE, KEEP_AUDIO_TOGGLE, KEEP_FOR_PICKER, MISSING_RECORDING_ERROR, NO_TEXT,
    ORIGINAL_SECTION, RECENT_TRANSCRIPTS_MENU, RECORDING_DETAIL, RETENTION_OPTIONS,
    SAVE_TRANSCRIPTS_TOGGLE, SEARCH_PROMPT, SHOW_ALL_HISTORY, clear_history_confirmation,
    retention_label, settings_caption,
};
pub use entry::{
    EntryAudio, FolderVerdict, HistoryEntry, MetadataError, MetadataFile, NewEntry, PathComponent,
    PruneReason, classify_entry_folder, decode_metadata, encode_metadata, entry_directory_name,
    insert_newest_first, sort_newest_first,
};
pub use listing::{
    ContextMenuItem, DaySection, DayTitle, EmptyState, EmptyStateCopy, PageContent, context_menu,
    copy_text, day_sections, filter_entries, format_duration, original_transcript, page_content,
    row_details, row_transcript,
};
pub use recent::{RecentTranscript, recent_transcripts};
pub use retention::{RetentionPlan, expiring_count, plan_retention, retention_change_confirmation};
pub use wav::{WavTooLarge, encode_wav};

/// Folder under the data dir that holds one folder per entry (HIS-5).
pub const HISTORY_DIRECTORY_NAME: &str = "History";

/// Metadata file inside each entry folder (HIS-5).
pub const METADATA_FILENAME: &str = "metadata.json";

/// Audio file inside an entry folder when audio is kept (HIS-5).
pub const AUDIO_FILENAME: &str = "audio.wav";

/// Sample rate of saved sessions and their recordings, in Hz (HIS-3, HIS-5).
pub const SAMPLE_RATE: u32 = 16_000;

/// Most entries kept; older ones beyond it are deleted (HIS-12).
pub const MAX_ENTRIES: usize = 1_000;

/// Most recordings kept by the audio budget (HIS-14).
pub const MAX_AUDIO_RECORDINGS: usize = 50;

/// Most audio bytes kept by the audio budget: 500 MB, decimal (HIS-14).
pub const MAX_AUDIO_BYTES: u64 = 500_000_000;

/// Most entries listed under "Recent Transcripts" in the tray (HIS-28).
pub const RECENT_TRANSCRIPT_LIMIT: usize = 5;

#[cfg(test)]
pub(crate) mod test_support {
    use super::{AUDIO_FILENAME, EntryAudio, HistoryEntry, PathComponent};
    use jiff::Timestamp;
    use uuid::Uuid;

    /// A Greendale entry with no audio, no app, and no cleanup.
    pub fn entry(n: u128, transcript: &str, timestamp: Timestamp) -> HistoryEntry {
        HistoryEntry {
            id: Uuid::from_u128(n),
            timestamp,
            duration: 1.0,
            sample_rate: 16_000,
            model_id: "parakeet-tdt-v2".into(),
            model_name: "Parakeet TDT v2".into(),
            language: "en".into(),
            transcript: transcript.into(),
            raw_transcript: None,
            was_post_processed: false,
            post_processing_error_description: None,
            app_name: None,
            app_bundle_id: None,
            directory_name: PathComponent::new(format!("entry-{n}")).expect("safe test name"),
            audio: None,
        }
    }

    /// `entry` with an `audio.wav` of `byte_count` bytes.
    pub fn with_audio(mut entry: HistoryEntry, byte_count: u64) -> HistoryEntry {
        entry.audio = Some(EntryAudio {
            filename: PathComponent::new(AUDIO_FILENAME).expect("safe test name"),
            byte_count,
        });
        entry
    }

    /// Seconds since the Unix epoch as a timestamp.
    pub fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(seconds).expect("valid test timestamp")
    }
}
