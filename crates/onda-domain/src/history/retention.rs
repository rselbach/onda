//! Retention and budgets (HIS-12..HIS-15) and the retention-change
//! confirmation (HIS-17).

use jiff::Timestamp;
use uuid::Uuid;

use super::copy::{Confirmation, retention_label};
use super::{HistoryEntry, MAX_AUDIO_BYTES, MAX_AUDIO_RECORDINGS, MAX_ENTRIES};
use crate::types::HistoryRetention;

/// What retention removes from a newest-first list (HIS-12..HIS-15).
///
/// The storage layer applies it in this order:
/// 1. For each entry in [`delete`](Self::delete), remove its folder.
/// 2. For each entry in [`strip_audio`](Self::strip_audio), delete the audio
///    file first, then rewrite `metadata.json` from
///    [`HistoryEntry::without_audio`]. This order means a failure can leave
///    metadata pointing at a missing file, never audio over budget (HIS-14).
/// 3. Replace the in-memory list with [`kept`](Self::kept) and emit
///    `HistoryChanged` unless [`is_noop`](Self::is_noop) (HIS-10).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RetentionPlan {
    /// Entries to delete, newest first: past the retention age, or beyond
    /// the entry cap.
    pub delete: Vec<HistoryEntry>,
    /// Entries, as they are now, whose recording must be removed, newest
    /// first. Their transcripts stay.
    pub strip_audio: Vec<HistoryEntry>,
    /// The list once the plan is applied, newest first, with stripped
    /// entries already shown without audio.
    pub kept: Vec<HistoryEntry>,
}

impl RetentionPlan {
    /// True when the plan deletes nothing and strips nothing.
    pub fn is_noop(&self) -> bool {
        self.delete.is_empty() && self.strip_audio.is_empty()
    }
}

/// Plans retention for `entries` (newest first) at `now` (HIS-12..HIS-15).
///
/// Deletes entries older than the retention age and, walking newest to
/// oldest, every entry once 1,000 are kept. Then, walking the kept entries
/// newest to oldest, keeps audio until the next recording would exceed 50
/// recordings or 500,000,000 bytes; from there on every older recording
/// loses its audio, even one that would still fit. `exempt` (the entry being
/// saved) is never deleted or stripped, but it still counts toward the cap
/// and the audio budget. The keep-audio setting plays no part: turning it
/// off stops new recordings, while existing ones age out through the budget.
pub fn plan_retention(
    entries: &[HistoryEntry],
    now: Timestamp,
    retention: HistoryRetention,
    exempt: Option<Uuid>,
) -> RetentionPlan {
    let cutoff = cutoff(now, retention);
    let is_exempt = |entry: &HistoryEntry| Some(entry.id) == exempt;
    let mut plan = RetentionPlan::default();

    for entry in entries {
        let expired = cutoff.is_some_and(|cutoff| entry.timestamp < cutoff);
        let over_cap = plan.kept.len() >= MAX_ENTRIES;
        match !is_exempt(entry) && (expired || over_cap) {
            true => plan.delete.push(entry.clone()),
            false => plan.kept.push(entry.clone()),
        }
    }

    let mut recordings = 0usize;
    let mut bytes = 0u64;
    let mut budget_spent = false;
    for entry in plan.kept.iter_mut().filter(|entry| entry.has_audio()) {
        if !is_exempt(entry) {
            budget_spent = budget_spent
                || recordings >= MAX_AUDIO_RECORDINGS
                || bytes.saturating_add(entry.audio_byte_count()) > MAX_AUDIO_BYTES;
            if budget_spent {
                plan.strip_audio.push(entry.clone());
                *entry = entry.without_audio();
                continue;
            }
        }
        recordings += 1;
        bytes = bytes.saturating_add(entry.audio_byte_count());
    }

    plan
}

/// How many entries are past `retention`'s age limit at `now`, counting only
/// age, not the entry cap (HIS-17).
pub fn expiring_count(
    entries: &[HistoryEntry],
    retention: HistoryRetention,
    now: Timestamp,
) -> usize {
    let Some(cutoff) = cutoff(now, retention) else {
        return 0;
    };
    entries
        .iter()
        .filter(|entry| entry.timestamp < cutoff)
        .count()
}

/// The question to ask before switching to `new_retention`, or `None` to
/// apply it right away because nothing would be deleted (HIS-17). Cancelling
/// leaves the setting unchanged.
pub fn retention_change_confirmation(
    entries: &[HistoryEntry],
    new_retention: HistoryRetention,
    now: Timestamp,
) -> Option<Confirmation> {
    let count = expiring_count(entries, new_retention, now);
    if count == 0 {
        return None;
    }
    let noun = match count {
        1 => "transcript",
        _ => "transcripts",
    };
    let period = retention_label(new_retention).to_lowercase();
    Some(Confirmation {
        title: format!("Delete {count} {noun} older than {period}?"),
        message: "This can't be undone.",
        confirm_label: "Delete Older Transcripts",
    })
}

// entries strictly older than the cutoff expire. a cutoff before the earliest
// representable instant expires nothing.
fn cutoff(now: Timestamp, retention: HistoryRetention) -> Option<Timestamp> {
    let max_age = retention.max_age()?;
    now.checked_sub(max_age).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_support::{at, entry, with_audio};

    const DAY: i64 = 24 * 60 * 60;

    fn transcripts(entries: &[HistoryEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.transcript.as_str()).collect()
    }

    // ages in days, newest first, matching Jabber's retention fixtures.
    fn aged_entries(now: i64) -> Vec<HistoryEntry> {
        vec![
            entry(1, "yesterday", at(now - DAY)),
            entry(2, "last week", at(now - 10 * DAY)),
            entry(3, "last season", at(now - 40 * DAY)),
        ]
    }

    #[test]
    fn retention_expires_entries_by_age_his_13() {
        let now = 100 * DAY;
        struct Case {
            name: &'static str,
            retention: HistoryRetention,
            exempt: Option<u128>,
            want_kept: Vec<&'static str>,
            want_deleted: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "one week",
                retention: HistoryRetention::Week,
                exempt: None,
                want_kept: vec!["yesterday"],
                want_deleted: vec!["last week", "last season"],
            },
            Case {
                name: "one month",
                retention: HistoryRetention::Month,
                exempt: None,
                want_kept: vec!["yesterday", "last week"],
                want_deleted: vec!["last season"],
            },
            Case {
                name: "forever",
                retention: HistoryRetention::Forever,
                exempt: None,
                want_kept: vec!["yesterday", "last week", "last season"],
                want_deleted: vec![],
            },
            Case {
                name: "exempt entry survives age (HIS-15)",
                retention: HistoryRetention::Week,
                exempt: Some(3),
                want_kept: vec!["yesterday", "last season"],
                want_deleted: vec!["last week"],
            },
        ];
        for tc in cases {
            let plan = plan_retention(
                &aged_entries(now),
                at(now),
                tc.retention,
                tc.exempt.map(Uuid::from_u128),
            );
            assert_eq!(transcripts(&plan.kept), tc.want_kept, "{}", tc.name);
            assert_eq!(transcripts(&plan.delete), tc.want_deleted, "{}", tc.name);
            assert!(plan.strip_audio.is_empty(), "{}", tc.name);
        }
    }

    #[test]
    fn retention_age_boundary_his_13() {
        let now = 100 * DAY;
        struct Case {
            name: &'static str,
            age: i64,
            want_deleted: bool,
        }
        let cases = [
            Case {
                name: "exactly seven days old stays",
                age: 7 * DAY,
                want_deleted: false,
            },
            Case {
                name: "one second past seven days goes",
                age: 7 * DAY + 1,
                want_deleted: true,
            },
            Case {
                name: "future timestamp stays",
                age: -DAY,
                want_deleted: false,
            },
        ];
        for tc in cases {
            let entries = [entry(1, "Troy Barnes", at(now - tc.age))];
            let plan = plan_retention(&entries, at(now), HistoryRetention::Week, None);
            assert_eq!(!plan.delete.is_empty(), tc.want_deleted, "{}", tc.name);
        }
    }

    #[test]
    fn retention_caps_entry_count_his_12() {
        struct Case {
            name: &'static str,
            count: usize,
            exempt: Option<u128>,
            want_kept: usize,
            want_deleted: Vec<u128>,
        }
        let cases = [
            Case {
                name: "at the cap nothing goes",
                count: MAX_ENTRIES,
                exempt: None,
                want_kept: MAX_ENTRIES,
                want_deleted: vec![],
            },
            Case {
                name: "oldest beyond the cap go",
                count: MAX_ENTRIES + 2,
                exempt: None,
                want_kept: MAX_ENTRIES,
                want_deleted: vec![1_000, 1_001],
            },
            Case {
                name: "exempt oldest survives the cap and still counts (HIS-15)",
                count: MAX_ENTRIES + 2,
                exempt: Some(1_001),
                want_kept: MAX_ENTRIES + 1,
                want_deleted: vec![1_000],
            },
            Case {
                name: "exempt newest counts toward the cap (HIS-15)",
                count: MAX_ENTRIES + 1,
                exempt: Some(0),
                want_kept: MAX_ENTRIES,
                want_deleted: vec![1_000],
            },
        ];
        for tc in cases {
            // index 0 is newest.
            let entries: Vec<HistoryEntry> = (0..tc.count)
                .map(|i| entry(i as u128, "Greendale", at(1_000_000 - i as i64)))
                .collect();
            let plan = plan_retention(
                &entries,
                at(1_000_000),
                HistoryRetention::Forever,
                tc.exempt.map(Uuid::from_u128),
            );
            let deleted: Vec<u128> = plan.delete.iter().map(|e| e.id.as_u128()).collect();
            assert_eq!(plan.kept.len(), tc.want_kept, "{}", tc.name);
            assert_eq!(deleted, tc.want_deleted, "{}", tc.name);
        }
    }

    #[test]
    fn retention_age_runs_before_cap_his_12_his_13() {
        // expired entries do not take cap slots: the cap only sees survivors.
        let now = 100 * DAY;
        let mut entries: Vec<HistoryEntry> = (0..MAX_ENTRIES)
            .map(|i| entry(i as u128, "recent", at(now - i as i64)))
            .collect();
        entries.push(entry(5_000, "expired", at(now - 40 * DAY)));
        let plan = plan_retention(&entries, at(now), HistoryRetention::Month, None);
        assert_eq!(plan.kept.len(), MAX_ENTRIES, "all recent entries kept");
        assert_eq!(
            transcripts(&plan.delete),
            ["expired"],
            "only the expired entry goes"
        );
    }

    #[test]
    fn audio_budget_his_14() {
        const MB: u64 = 1_000_000;
        struct Case {
            name: &'static str,
            // byte counts newest first; None means no audio.
            audio: Vec<Option<u64>>,
            exempt: Option<usize>,
            want_audio: Vec<bool>,
        }
        let cases = [
            Case {
                name: "within budget",
                audio: vec![Some(100 * MB), Some(100 * MB), None, Some(100 * MB)],
                exempt: None,
                want_audio: vec![true, true, false, true],
            },
            Case {
                name: "exactly 500,000,000 bytes fits",
                audio: vec![Some(250 * MB), Some(250 * MB)],
                exempt: None,
                want_audio: vec![true, true],
            },
            Case {
                name: "one byte over drops the older recording",
                audio: vec![Some(250 * MB), Some(250 * MB + 1)],
                exempt: None,
                want_audio: vec![true, false],
            },
            Case {
                name: "MiB-based budget is not used",
                audio: vec![Some(500 * MB + 1)],
                exempt: None,
                want_audio: vec![false],
            },
            Case {
                name: "once spent even a small older recording loses audio",
                audio: vec![Some(400 * MB), Some(200 * MB), Some(1)],
                exempt: None,
                want_audio: vec![true, false, false],
            },
            Case {
                name: "entries without audio do not spend the budget",
                audio: vec![Some(400 * MB), None, None, Some(100 * MB)],
                exempt: None,
                want_audio: vec![true, false, false, true],
            },
            Case {
                name: "exempt oversized recording keeps audio (HIS-15)",
                audio: vec![Some(600 * MB)],
                exempt: Some(0),
                want_audio: vec![true],
            },
            Case {
                name: "exempt recording counts toward the budget (HIS-15)",
                audio: vec![Some(600 * MB), Some(1)],
                exempt: Some(0),
                want_audio: vec![true, false],
            },
            Case {
                name: "oversized recording is dropped by the next save",
                audio: vec![Some(10), Some(600 * MB)],
                exempt: Some(0),
                want_audio: vec![true, false],
            },
        ];
        for tc in cases {
            let entries: Vec<HistoryEntry> = tc
                .audio
                .iter()
                .enumerate()
                .map(|(i, bytes)| {
                    let base = entry(i as u128, "Abed Nadir", at(10_000 - i as i64));
                    match bytes {
                        Some(bytes) => with_audio(base, *bytes),
                        None => base,
                    }
                })
                .collect();
            let plan = plan_retention(
                &entries,
                at(10_000),
                HistoryRetention::Forever,
                tc.exempt.map(|i| Uuid::from_u128(i as u128)),
            );
            let got: Vec<bool> = plan.kept.iter().map(HistoryEntry::has_audio).collect();
            assert_eq!(got, tc.want_audio, "{}", tc.name);
            assert!(plan.delete.is_empty(), "{}", tc.name);
            let stripped_ids: Vec<Uuid> = plan.strip_audio.iter().map(|e| e.id).collect();
            let want_stripped: Vec<Uuid> = entries
                .iter()
                .zip(&tc.want_audio)
                .filter(|(e, keeps)| e.has_audio() && !**keeps)
                .map(|(e, _)| e.id)
                .collect();
            assert_eq!(stripped_ids, want_stripped, "{}", tc.name);
            assert!(
                plan.strip_audio.iter().all(HistoryEntry::has_audio),
                "{}: strip list holds entries as they are now",
                tc.name
            );
        }
    }

    #[test]
    fn audio_count_budget_his_14() {
        // Jabber's count test, at the real limit: 52 recordings keep the newest 50.
        struct Case {
            name: &'static str,
            count: usize,
            exempt: Option<usize>,
            want_with_audio: usize,
        }
        let cases = [
            Case {
                name: "fifty recordings fit",
                count: MAX_AUDIO_RECORDINGS,
                exempt: None,
                want_with_audio: MAX_AUDIO_RECORDINGS,
            },
            Case {
                name: "oldest beyond fifty lose audio",
                count: MAX_AUDIO_RECORDINGS + 2,
                exempt: None,
                want_with_audio: MAX_AUDIO_RECORDINGS,
            },
            Case {
                name: "exempt oldest keeps audio beyond fifty (HIS-15)",
                count: MAX_AUDIO_RECORDINGS + 2,
                exempt: Some(MAX_AUDIO_RECORDINGS + 1),
                want_with_audio: MAX_AUDIO_RECORDINGS + 1,
            },
        ];
        for tc in cases {
            let entries: Vec<HistoryEntry> = (0..tc.count)
                .map(|i| with_audio(entry(i as u128, "Britta", at(10_000 - i as i64)), 84))
                .collect();
            let plan = plan_retention(
                &entries,
                at(10_000),
                HistoryRetention::Forever,
                tc.exempt.map(|i| Uuid::from_u128(i as u128)),
            );
            let got = plan.kept.iter().filter(|e| e.has_audio()).count();
            assert_eq!(got, tc.want_with_audio, "{}", tc.name);
            assert_eq!(plan.kept.len(), tc.count, "{}: transcripts stay", tc.name);
            assert!(
                plan.kept[..MAX_AUDIO_RECORDINGS]
                    .iter()
                    .all(HistoryEntry::has_audio),
                "{}: newest keep audio",
                tc.name
            );
        }
    }

    #[test]
    fn audio_budget_ignores_deleted_entries_his_14() {
        // an expired recording neither keeps audio nor spends the budget.
        let now = 100 * DAY;
        let entries = vec![
            with_audio(entry(1, "Shirley", at(now - DAY)), 400_000_000),
            with_audio(entry(2, "Pierce", at(now - 40 * DAY)), 400_000_000),
        ];
        let plan = plan_retention(&entries, at(now), HistoryRetention::Month, None);
        assert_eq!(transcripts(&plan.kept), ["Shirley"], "kept");
        assert_eq!(transcripts(&plan.delete), ["Pierce"], "deleted");
        assert!(plan.strip_audio.is_empty(), "nothing stripped");
        assert!(plan.kept[0].has_audio(), "survivor keeps audio");
    }

    #[test]
    fn plan_noop_his_10() {
        struct Case {
            name: &'static str,
            entries: Vec<HistoryEntry>,
            want: bool,
        }
        let cases = [
            Case {
                name: "empty history",
                entries: vec![],
                want: true,
            },
            Case {
                name: "nothing to do",
                entries: vec![with_audio(entry(1, "Annie", at(100)), 84)],
                want: true,
            },
            Case {
                name: "expired entry",
                entries: vec![entry(1, "Annie", at(-40 * DAY))],
                want: false,
            },
        ];
        for tc in cases {
            let plan = plan_retention(&tc.entries, at(100), HistoryRetention::Month, None);
            assert_eq!(plan.is_noop(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn expiring_count_his_17() {
        let now = 100 * DAY;
        struct Case {
            name: &'static str,
            retention: HistoryRetention,
            want: usize,
        }
        let cases = [
            Case {
                name: "one week",
                retention: HistoryRetention::Week,
                want: 2,
            },
            Case {
                name: "one month",
                retention: HistoryRetention::Month,
                want: 1,
            },
            Case {
                name: "forever",
                retention: HistoryRetention::Forever,
                want: 0,
            },
        ];
        for tc in cases {
            let got = expiring_count(&aged_entries(now), tc.retention, at(now));
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn retention_change_confirmation_his_17() {
        let now = 100 * DAY;
        struct Case {
            name: &'static str,
            entries: Vec<HistoryEntry>,
            retention: HistoryRetention,
            want_title: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "plural",
                entries: aged_entries(now),
                retention: HistoryRetention::Week,
                want_title: Some("Delete 2 transcripts older than 1 week?"),
            },
            Case {
                name: "singular",
                entries: aged_entries(now),
                retention: HistoryRetention::Month,
                want_title: Some("Delete 1 transcript older than 1 month?"),
            },
            Case {
                name: "forever never asks",
                entries: aged_entries(now),
                retention: HistoryRetention::Forever,
                want_title: None,
            },
            Case {
                name: "nothing expires so it applies right away",
                entries: vec![entry(1, "Dean Pelton", at(now - DAY))],
                retention: HistoryRetention::Week,
                want_title: None,
            },
        ];
        for tc in cases {
            let got = retention_change_confirmation(&tc.entries, tc.retention, at(now));
            assert_eq!(
                got.as_ref().map(|c| c.title.as_str()),
                tc.want_title,
                "{}",
                tc.name
            );
            if let Some(confirmation) = got {
                assert_eq!(confirmation.message, "This can't be undone.", "{}", tc.name);
                assert_eq!(
                    confirmation.confirm_label, "Delete Older Transcripts",
                    "{}",
                    tc.name
                );
            }
        }
    }
}
