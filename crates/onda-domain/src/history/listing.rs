//! The History page listing: search (HIS-19), day sections (HIS-21), rows
//! (HIS-22, HIS-23), context menu (HIS-24), copying (HIS-25), and empty
//! states (HIS-26).

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::HistoryEntry;
use super::copy::{COPY_LABEL, RECORDING_DETAIL};

/// Entries matching `query`, in their original order (HIS-19). An entry
/// matches when every whitespace-separated word of the query appears in its
/// transcript, raw transcript, or app name, ignoring case and diacritics. A
/// blank query matches everything.
pub fn filter_entries<'a>(entries: &'a [HistoryEntry], query: &str) -> Vec<&'a HistoryEntry> {
    let words: Vec<String> = query.split_whitespace().map(fold).collect();
    entries
        .iter()
        .filter(|entry| {
            if words.is_empty() {
                return true;
            }
            let haystack = [
                Some(entry.transcript.as_str()),
                entry.raw_transcript.as_deref(),
                entry.app_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .map(fold)
            .collect::<Vec<_>>()
            .join(" ");
            words.iter().all(|word| haystack.contains(word.as_str()))
        })
        .collect()
}

// lowercase, decompose, and drop combining marks so "Señor" matches "senor".
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect()
}

/// Title of a day section; the shell localizes the text (HIS-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayTitle {
    /// "Today".
    Today,
    /// "Yesterday".
    Yesterday,
    /// Any other day. Within the current year the shell shows weekday,
    /// month, and day ("Monday, October 27"); otherwise month, day, and year
    /// ("December 3, 2025").
    Date {
        /// The local calendar day.
        date: Date,
        /// Whether `date` falls in the same year as today.
        in_current_year: bool,
    },
}

/// Entries dictated on one local calendar day (HIS-21).
#[derive(Debug, Clone, PartialEq)]
pub struct DaySection<'a> {
    /// The local calendar day.
    pub date: Date,
    /// What the section header says.
    pub title: DayTitle,
    /// The day's entries, newest first.
    pub entries: Vec<&'a HistoryEntry>,
}

/// Groups newest-first entries into day sections, newest day first, using
/// `tz` for local days and `now` for Today and Yesterday (HIS-21).
pub fn day_sections<'a>(
    entries: impl IntoIterator<Item = &'a HistoryEntry>,
    now: Timestamp,
    tz: &TimeZone,
) -> Vec<DaySection<'a>> {
    let today = tz.to_datetime(now).date();
    let yesterday = today.yesterday().ok();
    let mut sections: Vec<DaySection<'a>> = Vec::new();
    for entry in entries {
        let date = tz.to_datetime(entry.timestamp).date();
        match sections.last_mut() {
            Some(section) if section.date == date => section.entries.push(entry),
            _ => sections.push(DaySection {
                date,
                title: day_title(date, today, yesterday),
                entries: vec![entry],
            }),
        }
    }
    sections
}

fn day_title(date: Date, today: Date, yesterday: Option<Date>) -> DayTitle {
    if date == today {
        return DayTitle::Today;
    }
    if Some(date) == yesterday {
        return DayTitle::Yesterday;
    }
    DayTitle::Date {
        date,
        in_current_year: date.year() == today.year(),
    }
}

/// Recording length as `m:ss`, rounding to the nearest second with ties to
/// even, like Jabber's minute-second duration style (HIS-22).
pub fn format_duration(seconds: f64) -> String {
    let total = match seconds.is_finite() && seconds > 0.0 {
        true => seconds.round_ties_even() as u64,
        false => 0,
    };
    format!("{}:{:02}", total / 60, total % 60)
}

/// A row's details line, `<time> · <app> · <m:ss> · Recording`, with the
/// app and "Recording" only when present (HIS-22). `time_text` is the
/// shell's localized short time for the entry.
pub fn row_details(entry: &HistoryEntry, time_text: &str) -> String {
    let duration = format_duration(entry.duration);
    let mut parts = vec![time_text];
    if let Some(app) = entry.app_name.as_deref() {
        parts.push(app);
    }
    parts.push(&duration);
    if entry.has_audio() {
        parts.push(RECORDING_DETAIL);
    }
    parts.join(" · ")
}

/// The trimmed transcript a row shows, or `None` to show [`NO_TEXT`] in
/// italics (HIS-22).
///
/// [`NO_TEXT`]: super::NO_TEXT
pub fn row_transcript(entry: &HistoryEntry) -> Option<&str> {
    let text = entry.transcript.trim();
    match text.is_empty() {
        true => None,
        false => Some(text),
    }
}

/// The trimmed raw transcript, when cleanup was applied and changed the
/// text: it is non-empty and differs from the trimmed final transcript
/// (HIS-23). Drives the "Original" section and "Copy Original".
pub fn original_transcript(entry: &HistoryEntry) -> Option<&str> {
    if !entry.was_post_processed {
        return None;
    }
    let raw = entry.raw_transcript.as_deref()?.trim();
    match raw.is_empty() || raw == entry.transcript.trim() {
        true => None,
        false => Some(raw),
    }
}

/// What every transcript copy path puts on the clipboard: the trimmed
/// transcript (HIS-25). "Copy Original" uses [`original_transcript`], which
/// is already trimmed.
pub fn copy_text(entry: &HistoryEntry) -> &str {
    entry.transcript.trim()
}

/// An item of a row's context menu (HIS-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    /// Copy the transcript ([`copy_text`]).
    Copy,
    /// Copy the original ([`original_transcript`]).
    CopyOriginal,
    /// Open the recording with the default app.
    PlayRecording,
    /// Reveal the recording in the file manager.
    ShowRecording,
    /// A separator.
    Divider,
    /// Delete the entry.
    Delete,
}

impl ContextMenuItem {
    /// The item's title, or `None` for [`ContextMenuItem::Divider`].
    /// "Show Recording in Finder" names the macOS file manager; other
    /// shells substitute theirs.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Copy => Some(COPY_LABEL),
            Self::CopyOriginal => Some("Copy Original"),
            Self::PlayRecording => Some("Play Recording"),
            Self::ShowRecording => Some("Show Recording in Finder"),
            Self::Divider => None,
            Self::Delete => Some("Delete"),
        }
    }
}

/// A row's context menu, top to bottom (HIS-24): "Copy", "Copy Original"
/// when an original exists, the recording items when audio exists, a
/// divider, and "Delete".
pub fn context_menu(entry: &HistoryEntry) -> Vec<ContextMenuItem> {
    let mut items = vec![ContextMenuItem::Copy];
    if original_transcript(entry).is_some() {
        items.push(ContextMenuItem::CopyOriginal);
    }
    if entry.has_audio() {
        items.push(ContextMenuItem::PlayRecording);
        items.push(ContextMenuItem::ShowRecording);
    }
    items.push(ContextMenuItem::Divider);
    items.push(ContextMenuItem::Delete);
    items
}

/// Why the History page shows no list (HIS-26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyState {
    /// History is on and there are no entries.
    NoDictations,
    /// History is off and there are no entries.
    HistoryOff,
    /// Entries exist but none match the search; the shell shows its
    /// platform's standard `no results for '<query>'` view.
    NoSearchResults,
}

/// Text of an empty state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyStateCopy {
    /// Headline.
    pub title: &'static str,
    /// Description under the headline.
    pub message: &'static str,
    /// Button label, when the state offers one.
    pub action: Option<&'static str>,
}

impl EmptyState {
    /// The state's text, or `None` for
    /// [`EmptyState::NoSearchResults`], which uses the platform's standard
    /// search view.
    pub fn text(self) -> Option<EmptyStateCopy> {
        match self {
            Self::NoDictations => Some(EmptyStateCopy {
                title: "No Dictations Yet",
                message: "Your transcripts appear here after you dictate.",
                action: None,
            }),
            Self::HistoryOff => Some(EmptyStateCopy {
                title: "History Is Off",
                message: "Turn it on to keep your transcripts on this computer.",
                action: Some("Turn On History"),
            }),
            Self::NoSearchResults => None,
        }
    }
}

/// What the History page body shows (HIS-21, HIS-26).
#[derive(Debug, Clone, PartialEq)]
pub enum PageContent<'a> {
    /// No list; show the empty state.
    Empty(EmptyState),
    /// Matching entries grouped by day.
    Sections(Vec<DaySection<'a>>),
}

/// The History page body for newest-first `entries` and the search `query`
/// (HIS-19, HIS-21, HIS-26).
pub fn page_content<'a>(
    entries: &'a [HistoryEntry],
    query: &str,
    history_enabled: bool,
    now: Timestamp,
    tz: &TimeZone,
) -> PageContent<'a> {
    if entries.is_empty() {
        return match history_enabled {
            true => PageContent::Empty(EmptyState::NoDictations),
            false => PageContent::Empty(EmptyState::HistoryOff),
        };
    }
    let matches = filter_entries(entries, query);
    if matches.is_empty() {
        return PageContent::Empty(EmptyState::NoSearchResults);
    }
    PageContent::Sections(day_sections(matches, now, tz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_support::{at, entry, with_audio};
    use jiff::civil::date;

    fn transcripts<'a>(entries: impl IntoIterator<Item = &'a HistoryEntry>) -> Vec<&'a str> {
        entries.into_iter().map(|e| e.transcript.as_str()).collect()
    }

    fn cleaned(transcript: &str, raw: Option<&str>, post_processed: bool) -> HistoryEntry {
        let mut e = entry(1, transcript, at(0));
        e.raw_transcript = raw.map(Into::into);
        e.was_post_processed = post_processed;
        e
    }

    #[test]
    fn filter_matches_every_word_across_text_and_app_his_19() {
        let mut morning = entry(1, "Troy and Abed in the morning", at(0));
        morning.app_name = Some("Greendale Messenger".into());
        let spanish = cleaned(
            "Señor Chang's Spanish class",
            Some("senor changs spanish class um"),
            true,
        );
        let movie = entry(3, "Six seasons and a movie", at(0));
        let mut cafe = entry(4, "meet at the cafe", at(0));
        cafe.app_name = Some("CAFÉ Notes".into());
        let entries = vec![morning, spanish, movie, cafe];

        struct Case<'a> {
            name: &'static str,
            query: &'static str,
            want: Vec<&'a str>,
        }
        let cases = [
            Case {
                name: "empty query keeps everything",
                query: "  ",
                want: transcripts(&entries),
            },
            Case {
                name: "case and accents are ignored",
                query: "senor CHANG",
                want: vec!["Señor Chang's Spanish class"],
            },
            Case {
                name: "accented query matches plain text",
                query: "café",
                want: vec!["meet at the cafe"],
            },
            Case {
                name: "original transcript counts",
                query: "um",
                want: vec!["Señor Chang's Spanish class"],
            },
            Case {
                name: "app name counts",
                query: "messenger morning",
                want: vec!["Troy and Abed in the morning"],
            },
            Case {
                name: "every word must match",
                query: "movie morning",
                want: vec![],
            },
            Case {
                name: "words split on any whitespace",
                query: "troy\tabed\n",
                want: vec!["Troy and Abed in the morning"],
            },
            Case {
                name: "substrings match",
                query: "seas",
                want: vec!["Six seasons and a movie"],
            },
        ];
        for tc in cases {
            let got = transcripts(filter_entries(&entries, tc.query));
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn sections_group_by_day_newest_first_his_21() {
        let day = 24 * 60 * 60;
        let now = 1_759_150_800; // Monday, September 29, 2025, 13:00 UTC
        let entries = vec![
            entry(1, "this afternoon", at(now - 60)),
            entry(2, "this morning", at(now - 12 * 60 * 60)),
            entry(3, "last night", at(now - day)),
            entry(4, "last week", at(now - 7 * day)),
            entry(5, "last year", at(now - 365 * day)),
        ];
        let sections = day_sections(&entries, at(now), &TimeZone::UTC);
        let titles: Vec<DayTitle> = sections.iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            [
                DayTitle::Today,
                DayTitle::Yesterday,
                DayTitle::Date {
                    date: date(2025, 9, 22),
                    in_current_year: true
                },
                DayTitle::Date {
                    date: date(2024, 9, 29),
                    in_current_year: false
                },
            ],
            "titles"
        );
        let grouped: Vec<Vec<&str>> = sections
            .iter()
            .map(|s| transcripts(s.entries.iter().copied()))
            .collect();
        assert_eq!(
            grouped,
            [
                vec!["this afternoon", "this morning"],
                vec!["last night"],
                vec!["last week"],
                vec!["last year"],
            ],
            "entries"
        );
    }

    #[test]
    fn sections_use_the_local_time_zone_his_21() {
        let tz = TimeZone::get("America/New_York").expect("tzdb has New York");
        struct Case {
            name: &'static str,
            now: &'static str,
            entry: &'static str,
            want: DayTitle,
        }
        let cases = [
            Case {
                name: "late evening local is still today though UTC rolled over",
                now: "2025-10-28T03:30:00Z",
                entry: "2025-10-28T00:30:00Z",
                want: DayTitle::Today,
            },
            Case {
                name: "just before local midnight is yesterday",
                now: "2025-10-28T13:00:00Z",
                entry: "2025-10-28T03:59:00Z",
                want: DayTitle::Yesterday,
            },
            Case {
                name: "yesterday across a DST change",
                now: "2025-11-03T12:00:00Z",
                entry: "2025-11-02T05:30:00Z",
                want: DayTitle::Yesterday,
            },
            Case {
                name: "new year's eve seen on january first",
                now: "2026-01-02T12:00:00Z",
                entry: "2025-12-31T12:00:00Z",
                want: DayTitle::Date {
                    date: date(2025, 12, 31),
                    in_current_year: false,
                },
            },
        ];
        for tc in cases {
            let now: Timestamp = tc.now.parse().expect("valid now");
            let entries = [entry(
                1,
                "Greendale",
                tc.entry.parse().expect("valid entry"),
            )];
            let sections = day_sections(&entries, now, &tz);
            assert_eq!(sections.len(), 1, "{}", tc.name);
            assert_eq!(sections[0].title, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn format_duration_his_22() {
        struct Case {
            name: &'static str,
            seconds: f64,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "zero",
                seconds: 0.0,
                want: "0:00",
            },
            Case {
                name: "rounds down",
                seconds: 4.4,
                want: "0:04",
            },
            Case {
                name: "rounds up",
                seconds: 59.6,
                want: "1:00",
            },
            Case {
                name: "half rounds to even, down",
                seconds: 2.5,
                want: "0:02",
            },
            Case {
                name: "half rounds to even, up",
                seconds: 3.5,
                want: "0:04",
            },
            Case {
                name: "minutes are not padded",
                seconds: 65.0,
                want: "1:05",
            },
            Case {
                name: "session limit",
                seconds: 900.0,
                want: "15:00",
            },
            Case {
                name: "over an hour stays in minutes",
                seconds: 3_725.0,
                want: "62:05",
            },
            Case {
                name: "invalid becomes zero",
                seconds: f64::NAN,
                want: "0:00",
            },
            Case {
                name: "negative becomes zero",
                seconds: -3.0,
                want: "0:00",
            },
        ];
        for tc in cases {
            assert_eq!(format_duration(tc.seconds), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn row_details_his_22() {
        let base = entry(1, "Troy Barnes", at(0));
        let mut with_app = base.clone();
        with_app.app_name = Some("Greendale Messenger".into());
        let mut long = with_audio(with_app.clone(), 84);
        long.duration = 75.0;

        struct Case {
            name: &'static str,
            entry: HistoryEntry,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "time and duration only",
                entry: base.clone(),
                want: "9:41 AM · 0:01",
            },
            Case {
                name: "with app",
                entry: with_app,
                want: "9:41 AM · Greendale Messenger · 0:01",
            },
            Case {
                name: "with app and recording",
                entry: long,
                want: "9:41 AM · Greendale Messenger · 1:15 · Recording",
            },
            Case {
                name: "recording without app",
                entry: with_audio(base, 84),
                want: "9:41 AM · 0:01 · Recording",
            },
        ];
        for tc in cases {
            assert_eq!(row_details(&tc.entry, "9:41 AM"), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn row_transcript_and_copy_trim_his_22_his_25() {
        struct Case {
            name: &'static str,
            transcript: &'static str,
            want_row: Option<&'static str>,
            want_copy: &'static str,
        }
        let cases = [
            Case {
                name: "plain",
                transcript: "Cool. Cool cool cool.",
                want_row: Some("Cool. Cool cool cool."),
                want_copy: "Cool. Cool cool cool.",
            },
            Case {
                name: "surrounding whitespace trimmed",
                transcript: "  streets ahead\n",
                want_row: Some("streets ahead"),
                want_copy: "streets ahead",
            },
            Case {
                name: "empty shows No text",
                transcript: "",
                want_row: None,
                want_copy: "",
            },
            Case {
                name: "blank shows No text",
                transcript: " \n\t ",
                want_row: None,
                want_copy: "",
            },
        ];
        for tc in cases {
            let e = entry(1, tc.transcript, at(0));
            assert_eq!(row_transcript(&e), tc.want_row, "{}", tc.name);
            assert_eq!(copy_text(&e), tc.want_copy, "{}", tc.name);
        }
    }

    #[test]
    fn original_transcript_only_when_cleanup_changed_it_his_23() {
        struct Case {
            name: &'static str,
            entry: HistoryEntry,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "cleaned up",
                entry: cleaned("Buy water.", Some(" um buy milk no wait buy water "), true),
                want: Some("um buy milk no wait buy water"),
            },
            Case {
                name: "cleanup changed nothing",
                entry: cleaned("Buy water.", Some("Buy water."), true),
                want: None,
            },
            Case {
                name: "only whitespace differs",
                entry: cleaned(" Buy water. ", Some("Buy water.\n"), true),
                want: None,
            },
            Case {
                name: "not post-processed",
                entry: cleaned("buy water", None, false),
                want: None,
            },
            Case {
                name: "raw kept but cleanup not applied",
                entry: cleaned("Buy water.", Some("buy water"), false),
                want: None,
            },
            Case {
                name: "empty original",
                entry: cleaned("Buy water.", Some("  "), true),
                want: None,
            },
            Case {
                name: "scratch that emptied the final text",
                entry: cleaned("", Some("buy water scratch that"), true),
                want: Some("buy water scratch that"),
            },
        ];
        for tc in cases {
            assert_eq!(original_transcript(&tc.entry), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn context_menu_his_24() {
        use ContextMenuItem::*;
        let plain = entry(1, "Abed", at(0));
        let with_original = cleaned("Hello.", Some("um hello"), true);
        struct Case {
            name: &'static str,
            entry: HistoryEntry,
            want: Vec<ContextMenuItem>,
        }
        let cases = [
            Case {
                name: "plain entry",
                entry: plain.clone(),
                want: vec![Copy, Divider, Delete],
            },
            Case {
                name: "with original",
                entry: with_original.clone(),
                want: vec![Copy, CopyOriginal, Divider, Delete],
            },
            Case {
                name: "with recording",
                entry: with_audio(plain, 84),
                want: vec![Copy, PlayRecording, ShowRecording, Divider, Delete],
            },
            Case {
                name: "with both",
                entry: with_audio(with_original, 84),
                want: vec![
                    Copy,
                    CopyOriginal,
                    PlayRecording,
                    ShowRecording,
                    Divider,
                    Delete,
                ],
            },
        ];
        for tc in cases {
            assert_eq!(context_menu(&tc.entry), tc.want, "{}", tc.name);
        }
        let labels: Vec<Option<&str>> = [
            Copy,
            CopyOriginal,
            PlayRecording,
            ShowRecording,
            Divider,
            Delete,
        ]
        .into_iter()
        .map(ContextMenuItem::label)
        .collect();
        assert_eq!(
            labels,
            [
                Some("Copy"),
                Some("Copy Original"),
                Some("Play Recording"),
                Some("Show Recording in Finder"),
                None,
                Some("Delete"),
            ],
            "labels"
        );
    }

    #[test]
    fn page_content_his_26() {
        let now = at(1_759_150_800);
        let entries = vec![entry(1, "Troy and Abed in the morning", at(1_759_150_000))];
        struct Case {
            name: &'static str,
            entries: Vec<HistoryEntry>,
            query: &'static str,
            enabled: bool,
            want_empty: Option<EmptyState>,
        }
        let cases = [
            Case {
                name: "history on and empty",
                entries: vec![],
                query: "",
                enabled: true,
                want_empty: Some(EmptyState::NoDictations),
            },
            Case {
                name: "history off and empty",
                entries: vec![],
                query: "troy",
                enabled: false,
                want_empty: Some(EmptyState::HistoryOff),
            },
            Case {
                name: "no search matches",
                entries: entries.clone(),
                query: "pierce",
                enabled: true,
                want_empty: Some(EmptyState::NoSearchResults),
            },
            Case {
                name: "history off still lists saved entries",
                entries: entries.clone(),
                query: "",
                enabled: false,
                want_empty: None,
            },
            Case {
                name: "matches are listed",
                entries,
                query: "abed",
                enabled: true,
                want_empty: None,
            },
        ];
        for tc in cases {
            let got = page_content(&tc.entries, tc.query, tc.enabled, now, &TimeZone::UTC);
            match (got, tc.want_empty) {
                (PageContent::Empty(state), Some(want)) => {
                    assert_eq!(state, want, "{}", tc.name)
                }
                (PageContent::Sections(sections), None) => {
                    assert_eq!(sections.len(), 1, "{}", tc.name)
                }
                (got, want) => panic!("{}: got {got:?}, want {want:?}", tc.name),
            }
        }
    }

    #[test]
    fn empty_state_text_his_26() {
        struct Case {
            name: &'static str,
            state: EmptyState,
            want: Option<EmptyStateCopy>,
        }
        let cases = [
            Case {
                name: "no dictations",
                state: EmptyState::NoDictations,
                want: Some(EmptyStateCopy {
                    title: "No Dictations Yet",
                    message: "Your transcripts appear here after you dictate.",
                    action: None,
                }),
            },
            Case {
                name: "history off",
                state: EmptyState::HistoryOff,
                want: Some(EmptyStateCopy {
                    title: "History Is Off",
                    message: "Turn it on to keep your transcripts on this computer.",
                    action: Some("Turn On History"),
                }),
            },
            Case {
                name: "search uses the platform view",
                state: EmptyState::NoSearchResults,
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(tc.state.text(), tc.want, "{}", tc.name);
        }
    }
}
