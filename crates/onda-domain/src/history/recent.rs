//! "Recent Transcripts" in the tray menu (HIS-28).

use jiff::Timestamp;
use uuid::Uuid;

use super::{HistoryEntry, RECENT_TRANSCRIPT_LIMIT};

/// A saved transcript offered under "Recent Transcripts" (HIS-28). The item
/// title is the one-line 40-character preview of `text`, built elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentTranscript {
    /// Entry id.
    pub id: Uuid,
    /// The trimmed transcript; choosing the item delivers it (DICT-27).
    pub text: String,
    /// When the entry was saved.
    pub timestamp: Timestamp,
    /// Target app, when known.
    pub app_name: Option<String>,
}

impl RecentTranscript {
    /// The app the item subtitle names after the date, or `None` when it is
    /// unknown and the subtitle is just the date. The shell writes
    /// `<abbreviated date and short time> · <app>` in the user's locale
    /// (HIS-28).
    pub fn subtitle_app(&self) -> Option<&str> {
        self.app_name
            .as_deref()
            .map(str::trim)
            .filter(|app| !app.is_empty())
    }
}

/// The newest 5 newest-first `entries` with non-empty trimmed text, or none
/// when history is off (HIS-28). The submenu is hidden when this is empty.
pub fn recent_transcripts(
    entries: &[HistoryEntry],
    history_enabled: bool,
) -> Vec<RecentTranscript> {
    if !history_enabled {
        return Vec::new();
    }
    entries
        .iter()
        .filter_map(|entry| {
            let text = entry.transcript.trim();
            if text.is_empty() {
                return None;
            }
            Some(RecentTranscript {
                id: entry.id,
                text: text.to_owned(),
                timestamp: entry.timestamp,
                app_name: entry.app_name.clone(),
            })
        })
        .take(RECENT_TRANSCRIPT_LIMIT)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_support::{at, entry};

    #[test]
    fn recent_transcripts_his_28() {
        let names = [
            "  Troy Barnes\n",
            "",
            "Abed Nadir",
            "   ",
            "Annie Edison",
            "Britta Perry",
            "Shirley Bennett",
            "Pierce Hawthorne",
        ];
        let entries: Vec<HistoryEntry> = names
            .iter()
            .enumerate()
            .map(|(i, name)| entry(i as u128, name, at(1_000 - i as i64)))
            .collect();

        struct Case {
            name: &'static str,
            entries: Vec<HistoryEntry>,
            enabled: bool,
            want: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "newest five with text, trimmed",
                entries: entries.clone(),
                enabled: true,
                want: vec![
                    "Troy Barnes",
                    "Abed Nadir",
                    "Annie Edison",
                    "Britta Perry",
                    "Shirley Bennett",
                ],
            },
            Case {
                name: "history off hides the submenu",
                entries: entries.clone(),
                enabled: false,
                want: vec![],
            },
            Case {
                name: "only blank entries hide the submenu",
                entries: vec![entry(1, "", at(1)), entry(2, " \n", at(0))],
                enabled: true,
                want: vec![],
            },
            Case {
                name: "fewer than five",
                entries: entries[..3].to_vec(),
                enabled: true,
                want: vec!["Troy Barnes", "Abed Nadir"],
            },
        ];
        for tc in cases {
            let got = recent_transcripts(&tc.entries, tc.enabled);
            let texts: Vec<&str> = got.iter().map(|r| r.text.as_str()).collect();
            assert_eq!(texts, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn recent_transcript_subtitle_app_his_28() {
        struct Case {
            name: &'static str,
            app: Option<&'static str>,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "with app",
                app: Some("Greendale Messenger"),
                want: Some("Greendale Messenger"),
            },
            Case {
                name: "app omitted when unknown",
                app: None,
                want: None,
            },
            Case {
                name: "a blank app is unknown",
                app: Some("  "),
                want: None,
            },
        ];
        for tc in cases {
            let mut entry = entry(1, "Six seasons and a movie", at(0));
            entry.app_name = tc.app.map(str::to_owned);
            let recent = recent_transcripts(std::slice::from_ref(&entry), true);
            assert_eq!(recent.len(), 1, "{}", tc.name);
            assert_eq!(recent[0].timestamp, at(0), "{}", tc.name);
            assert_eq!(recent[0].subtitle_app(), tc.want, "{}", tc.name);
        }
    }
}
