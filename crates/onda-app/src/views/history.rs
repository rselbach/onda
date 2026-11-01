//! The History page (HIS-17..HIS-27).

use std::time::SystemTime;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use onda_domain::history::{
    CLEAR_HISTORY_BUTTON, CLEAR_HISTORY_TOOLTIP, COPIED_LABEL, COPY_LABEL, Confirmation,
    ContextMenuItem, DayTitle, EmptyState, HISTORY_ERROR_TITLE, HistoryEntry, KEEP_AUDIO_TOGGLE,
    KEEP_FOR_PICKER, NO_TEXT, ORIGINAL_SECTION, PageContent, RECORDING_DETAIL, RETENTION_OPTIONS,
    SAVE_TRANSCRIPTS_TOGGLE, SEARCH_PROMPT, context_menu, format_duration, original_transcript,
    page_content, retention_label, row_transcript, settings_caption,
};
use onda_domain::types::HistoryRetention;
use uuid::Uuid;

use super::{AlertView, ButtonView, ToggleView};

/// One "Keep for" option (HIS-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionOption {
    /// The option.
    pub retention: HistoryRetention,
    /// "1 Week", "1 Month", or "Forever".
    pub title: String,
}

/// A section header (HIS-21). The shell localizes dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DayHeading {
    /// "Today" or "Yesterday".
    Relative {
        /// The text.
        text: String,
    },
    /// Any other local day: weekday, month, and day within the current year
    /// ("Monday, October 27"), month, day, and year otherwise ("December 3,
    /// 2025").
    Date {
        /// Year.
        year: i32,
        /// Month, 1..=12.
        month: u8,
        /// Day of month, 1..=31.
        day: u8,
        /// The date falls in the current year: show the weekday, not the
        /// year.
        in_current_year: bool,
    },
}

/// A context menu item (HIS-24); dividers have an empty title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryMenuItem {
    /// What it does.
    pub item: ContextMenuItem,
    /// Title; empty for a divider.
    pub title: String,
}

/// One entry row (HIS-22, HIS-23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// Pass to the entry commands.
    pub id: String,
    /// When it was saved; the shell shows the localized short time.
    pub time: SystemTime,
    /// The rest of the details line after the time: "<app> · <m:ss> ·
    /// Recording", the app and "Recording" only when present.
    pub details: String,
    /// The app identifier for the row icon.
    pub app_id: Option<String>,
    /// The trimmed transcript, or `None` to show `no_text_label` in italics.
    pub transcript: Option<String>,
    /// The raw transcript for the "Original" section (HIS-23).
    pub original: Option<String>,
    /// The entry has a recording.
    pub has_audio: bool,
    /// Context menu, top to bottom.
    pub menu: Vec<HistoryMenuItem>,
}

/// One day of entries, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySection {
    /// Header.
    pub heading: DayHeading,
    /// Rows.
    pub rows: Vec<HistoryRow>,
}

/// The page body (HIS-21, HIS-26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryContent {
    /// An empty state with its copy.
    Empty {
        /// Headline.
        title: String,
        /// Description.
        message: String,
        /// "Turn On History" (`set_history_enabled(true)`), when offered.
        action: Option<String>,
    },
    /// Nothing matches: the platform's standard "no results for
    /// '<query>'" view.
    NoSearchResults {
        /// The query.
        query: String,
    },
    /// Matching entries by day.
    Sections {
        /// Sections, newest day first.
        sections: Vec<HistorySection>,
    },
}

/// A destructive confirmation (HIS-17, HIS-20). Confirm with
/// `confirm_history_dialog`, cancel with `cancel_history_dialog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryDialog {
    /// Title.
    pub title: String,
    /// Note.
    pub message: String,
    /// Destructive button.
    pub confirm_title: String,
    /// "Cancel".
    pub cancel_title: String,
}

impl From<&Confirmation> for HistoryDialog {
    fn from(confirmation: &Confirmation) -> Self {
        Self {
            title: confirmation.title.clone(),
            message: confirmation.message.to_owned(),
            confirm_title: confirmation.confirm_label.to_owned(),
            cancel_title: "Cancel".to_owned(),
        }
    }
}

/// The History page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    /// "Save transcripts".
    pub save_transcripts: ToggleView,
    /// "Keep for".
    pub retention_label: String,
    /// The options.
    pub retention_options: Vec<RetentionOption>,
    /// The selected option.
    pub retention: HistoryRetention,
    /// The picker is enabled (history on).
    pub retention_enabled: bool,
    /// "Keep audio recordings".
    pub keep_audio: ToggleView,
    /// Caption under the bar (HIS-18).
    pub caption: String,
    /// "Search transcripts".
    pub search_prompt: String,
    /// The current query.
    pub query: String,
    /// "Clear History…", disabled when empty.
    pub clear_button: ButtonView,
    /// Its tooltip.
    pub clear_tooltip: String,
    /// The body.
    pub content: HistoryContent,
    /// A confirmation to show.
    pub dialog: Option<HistoryDialog>,
    /// "History Error" alert; dismiss with `dismiss_history_error`.
    pub error: Option<AlertView>,
    /// The entry whose copy button shows the checkmark (for 1.5 s).
    pub copied_entry: Option<String>,
    /// "Copy".
    pub copy_label: String,
    /// "Copied".
    pub copied_label: String,
    /// "No text".
    pub no_text_label: String,
    /// "Original".
    pub original_label: String,
}

/// What the History page is built from.
#[derive(Debug, Clone, Copy)]
pub struct HistoryInputs<'a> {
    /// Entries, newest first.
    pub entries: &'a [HistoryEntry],
    /// Search query.
    pub query: &'a str,
    /// Save transcripts.
    pub history_enabled: bool,
    /// Keep audio recordings.
    pub keep_audio: bool,
    /// Keep for.
    pub retention: HistoryRetention,
    /// Now, for Today and Yesterday.
    pub now: Timestamp,
    /// The local time zone that decides days.
    pub tz: &'a TimeZone,
    /// A pending confirmation.
    pub dialog: Option<&'a Confirmation>,
    /// The last error message.
    pub error: Option<&'a str>,
    /// The entry just copied.
    pub copied: Option<Uuid>,
}

/// Builds the History page.
pub fn history_page(inputs: HistoryInputs<'_>) -> HistoryPage {
    let content = match page_content(
        inputs.entries,
        inputs.query,
        inputs.history_enabled,
        inputs.now,
        inputs.tz,
    ) {
        PageContent::Empty(EmptyState::NoSearchResults) => HistoryContent::NoSearchResults {
            query: inputs.query.to_owned(),
        },
        PageContent::Empty(state) => {
            let copy = state.text();
            HistoryContent::Empty {
                title: copy.map(|c| c.title.to_owned()).unwrap_or_default(),
                message: copy.map(|c| c.message.to_owned()).unwrap_or_default(),
                action: copy.and_then(|c| c.action.map(str::to_owned)),
            }
        }
        PageContent::Sections(sections) => HistoryContent::Sections {
            sections: sections
                .into_iter()
                .map(|section| HistorySection {
                    heading: heading(section.title),
                    rows: section.entries.into_iter().map(row).collect(),
                })
                .collect(),
        },
    };
    HistoryPage {
        save_transcripts: ToggleView::new(SAVE_TRANSCRIPTS_TOGGLE, inputs.history_enabled, true),
        retention_label: KEEP_FOR_PICKER.to_owned(),
        retention_options: RETENTION_OPTIONS
            .into_iter()
            .map(|retention| RetentionOption {
                retention,
                title: retention_label(retention).to_owned(),
            })
            .collect(),
        retention: inputs.retention,
        retention_enabled: inputs.history_enabled,
        keep_audio: ToggleView::new(KEEP_AUDIO_TOGGLE, inputs.keep_audio, inputs.history_enabled),
        caption: settings_caption(inputs.history_enabled, inputs.keep_audio).to_owned(),
        search_prompt: SEARCH_PROMPT.to_owned(),
        query: inputs.query.to_owned(),
        clear_button: ButtonView::new(CLEAR_HISTORY_BUTTON, !inputs.entries.is_empty()),
        clear_tooltip: CLEAR_HISTORY_TOOLTIP.to_owned(),
        content,
        dialog: inputs.dialog.map(HistoryDialog::from),
        error: inputs.error.map(|message| AlertView {
            title: HISTORY_ERROR_TITLE.to_owned(),
            message: message.to_owned(),
        }),
        copied_entry: inputs.copied.map(|id| id.to_string()),
        copy_label: COPY_LABEL.to_owned(),
        copied_label: COPIED_LABEL.to_owned(),
        no_text_label: NO_TEXT.to_owned(),
        original_label: ORIGINAL_SECTION.to_owned(),
    }
}

fn heading(title: DayTitle) -> DayHeading {
    match title {
        DayTitle::Today => DayHeading::Relative {
            text: "Today".to_owned(),
        },
        DayTitle::Yesterday => DayHeading::Relative {
            text: "Yesterday".to_owned(),
        },
        DayTitle::Date {
            date,
            in_current_year,
        } => DayHeading::Date {
            year: i32::from(date.year()),
            month: u8::try_from(date.month()).unwrap_or(1),
            day: u8::try_from(date.day()).unwrap_or(1),
            in_current_year,
        },
    }
}

fn row(entry: &HistoryEntry) -> HistoryRow {
    let duration = format_duration(entry.duration);
    let mut details: Vec<&str> = Vec::new();
    if let Some(app) = entry.app_name.as_deref() {
        details.push(app);
    }
    details.push(&duration);
    if entry.has_audio() {
        details.push(RECORDING_DETAIL);
    }
    HistoryRow {
        id: entry.id.to_string(),
        time: SystemTime::from(entry.timestamp),
        details: details.join(" · "),
        app_id: entry.app_bundle_id.clone(),
        transcript: row_transcript(entry).map(str::to_owned),
        original: original_transcript(entry).map(str::to_owned),
        has_audio: entry.has_audio(),
        menu: context_menu(entry)
            .into_iter()
            .map(|item| HistoryMenuItem {
                item,
                title: item.label().unwrap_or_default().to_owned(),
            })
            .collect(),
    }
}
