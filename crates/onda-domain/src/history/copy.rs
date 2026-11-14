//! User-facing History copy (HIS-17, HIS-18, HIS-20, HIS-22, HIS-24,
//! HIS-27, HIS-28) and retention option labels.

use crate::platform::Platform;
use crate::types::HistoryRetention;

/// Retention picker options, in display order (HIS-13).
pub const RETENTION_OPTIONS: [HistoryRetention; 3] = [
    HistoryRetention::Week,
    HistoryRetention::Month,
    HistoryRetention::Forever,
];

/// Picker label for a retention option: "1 Week", "1 Month", or "Forever".
pub fn retention_label(retention: HistoryRetention) -> &'static str {
    match retention {
        HistoryRetention::Week => "1 Week",
        HistoryRetention::Month => "1 Month",
        HistoryRetention::Forever => "Forever",
    }
}

/// History on/off toggle label (HIS-18).
pub const SAVE_TRANSCRIPTS_TOGGLE: &str = "Save transcripts";

/// Retention picker label; disabled when history is off (HIS-18).
pub const KEEP_FOR_PICKER: &str = "Keep for";

/// Audio toggle label; disabled when history is off (HIS-18).
pub const KEEP_AUDIO_TOGGLE: &str = "Keep audio recordings";

/// Caption under the History settings bar (HIS-18). It promises backup
/// exclusion only where the platform provides it.
pub fn settings_caption(history_enabled: bool, keeps_audio: bool, platform: Platform) -> String {
    if !history_enabled {
        return "New dictations aren't saved. Saved transcripts stay until they expire or you delete them.".to_owned();
    }
    let computer = platform.computer_term;
    let backups = match platform.excludes_from_backups {
        true => " and left out of backups",
        false => "",
    };
    let audio = match keeps_audio {
        true => " Audio keeps the newest 50 recordings, up to 500 MB.",
        false => "",
    };
    format!("Saved only on this {computer}{backups}.{audio}")
}

/// Search field prompt (HIS-19).
pub const SEARCH_PROMPT: &str = "Search transcripts";

/// Toolbar button that clears history; disabled when empty (HIS-20).
pub const CLEAR_HISTORY_BUTTON: &str = "Clear History…";

/// Tooltip of [`CLEAR_HISTORY_BUTTON`] (HIS-20).
pub const CLEAR_HISTORY_TOOLTIP: &str = "Delete all saved transcripts and recordings";

/// Row detail shown when the entry still has audio (HIS-22).
pub const RECORDING_DETAIL: &str = "Recording";

/// Placeholder for an empty transcript, shown in italics (HIS-22).
pub const NO_TEXT: &str = "No text";

/// Title of the collapsible raw-transcript section (HIS-22).
pub const ORIGINAL_SECTION: &str = "Original";

/// Accessibility label of the row's copy button (HIS-22).
pub const COPY_LABEL: &str = "Copy";

/// Accessibility label of the row's copy button for 1.5 s after copying
/// (HIS-22).
pub const COPIED_LABEL: &str = "Copied";

/// Error when an entry's recording file is gone (HIS-24).
pub const MISSING_RECORDING_ERROR: &str = "The recording for this dictation is missing.";

/// Alert title for delete, clear, and retention failures (HIS-27).
pub const HISTORY_ERROR_TITLE: &str = "History Error";

/// Tray submenu title (HIS-28).
pub const RECENT_TRANSCRIPTS_MENU: &str = "Recent Transcripts";

/// Last item of the tray submenu, after a separator (HIS-28).
pub const SHOW_ALL_HISTORY: &str = "Show All History…";

/// A destructive confirmation dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    /// Dialog title.
    pub title: String,
    /// Explanatory note under the title.
    pub message: &'static str,
    /// Label of the destructive button; the other button cancels.
    pub confirm_label: &'static str,
}

/// Confirmation for "Clear History…" with `entry_count` saved entries
/// (HIS-20). The title is plural even for one entry, as in Jabber.
pub fn clear_history_confirmation(entry_count: usize) -> Confirmation {
    Confirmation {
        title: format!("Delete all {entry_count} saved transcripts?"),
        message: "Saved recordings are deleted too. This can't be undone.",
        confirm_label: "Delete All",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy_naming::PrivacyNaming;

    #[test]
    fn retention_labels() {
        struct Case {
            name: &'static str,
            retention: HistoryRetention,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "week",
                retention: HistoryRetention::Week,
                want: "1 Week",
            },
            Case {
                name: "month",
                retention: HistoryRetention::Month,
                want: "1 Month",
            },
            Case {
                name: "forever",
                retention: HistoryRetention::Forever,
                want: "Forever",
            },
        ];
        for tc in cases {
            assert_eq!(retention_label(tc.retention), tc.want, "{}", tc.name);
        }
        let labels: Vec<&str> = RETENTION_OPTIONS.into_iter().map(retention_label).collect();
        assert_eq!(labels, ["1 Week", "1 Month", "Forever"], "picker order");
    }

    #[test]
    fn settings_caption_his_18() {
        struct Case {
            name: &'static str,
            enabled: bool,
            keeps_audio: bool,
            platform: Platform,
            want: &'static str,
        }
        let macos = Platform::macos(PrivacyNaming::Accessibility);
        let off = "New dictations aren't saved. Saved transcripts stay until they expire or you delete them.";
        let cases = [
            Case {
                name: "history off",
                enabled: false,
                keeps_audio: false,
                platform: macos,
                want: off,
            },
            Case {
                name: "history off ignores audio",
                enabled: false,
                keeps_audio: true,
                platform: macos,
                want: off,
            },
            Case {
                name: "history off on Linux",
                enabled: false,
                keeps_audio: true,
                platform: Platform::linux(),
                want: off,
            },
            Case {
                name: "on without audio",
                enabled: true,
                keeps_audio: false,
                platform: macos,
                want: "Saved only on this Mac and left out of backups.",
            },
            Case {
                name: "on with audio uses decimal 500 MB",
                enabled: true,
                keeps_audio: true,
                platform: macos,
                want: "Saved only on this Mac and left out of backups. Audio keeps the newest 50 recordings, up to 500 MB.",
            },
            Case {
                name: "Linux promises no backup exclusion",
                enabled: true,
                keeps_audio: false,
                platform: Platform::linux(),
                want: "Saved only on this computer.",
            },
            Case {
                name: "Linux with audio",
                enabled: true,
                keeps_audio: true,
                platform: Platform::linux(),
                want: "Saved only on this computer. Audio keeps the newest 50 recordings, up to 500 MB.",
            },
        ];
        for tc in cases {
            assert_eq!(
                settings_caption(tc.enabled, tc.keeps_audio, tc.platform),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn clear_history_confirmation_his_20() {
        struct Case {
            name: &'static str,
            count: usize,
            want_title: &'static str,
        }
        let cases = [
            Case {
                name: "many",
                count: 12,
                want_title: "Delete all 12 saved transcripts?",
            },
            Case {
                name: "one stays plural like Jabber",
                count: 1,
                want_title: "Delete all 1 saved transcripts?",
            },
        ];
        for tc in cases {
            let got = clear_history_confirmation(tc.count);
            assert_eq!(got.title, tc.want_title, "{}", tc.name);
            assert_eq!(
                got.message, "Saved recordings are deleted too. This can't be undone.",
                "{}",
                tc.name
            );
            assert_eq!(got.confirm_label, "Delete All", "{}", tc.name);
        }
    }
}
