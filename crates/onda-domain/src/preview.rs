//! Live preview rules: tick decisions, the stabilizer, final-decode reuse,
//! and Parakeet Unified stream bookkeeping (ASR-15..ASR-21, DICT-13).

use std::time::Duration;

use crate::level::rms_level;
use crate::pre_roll::SAMPLE_RATE;
use crate::types::MonoTime;

/// Sleep between preview ticks. A tick decodes first and sleeps afterwards,
/// so the first preview is not delayed a whole tick (ASR-15).
pub const PREVIEW_INTERVAL: Duration = Duration::from_millis(300);

/// Fewest captured samples worth a preview decode: 0.5 s (ASR-16).
pub const PREVIEW_MIN_SAMPLES: usize = 8_000;

/// How long the final decode waits for an in-flight preview decode before
/// cancelling it (DICT-13).
pub const PREVIEW_STOP_WAIT: Duration = Duration::from_secs(5);

/// Parakeet Unified encoder frame: 80 ms of 16 kHz audio (ASR-17).
pub const UNIFIED_FRAME_SAMPLES: usize = 1_280;

/// Parakeet Unified left context in encoder frames: 5.6 s (ASR-17).
pub const UNIFIED_LEFT_CONTEXT_FRAMES: usize = 70;

/// Parakeet Unified chunk in encoder frames: 0.56 s (ASR-17).
pub const UNIFIED_CHUNK_FRAMES: usize = 7;

/// Parakeet Unified look-ahead in encoder frames: 0.56 s (ASR-17).
pub const UNIFIED_LOOKAHEAD_FRAMES: usize = 7;

/// Shortest recording a preview may stand in for: 2 s (ASR-21).
const REUSE_MIN_SAMPLES: usize = SAMPLE_RATE * 2;
/// Oldest preview that may stand in for the final decode (ASR-21).
const REUSE_MAX_AGE: Duration = Duration::from_secs(3);
/// Share of the recording the preview must cover, in percent (ASR-21).
const REUSE_MIN_COVERAGE_PERCENT: usize = 88;
/// Longest uncovered tail: 1.8 s (ASR-21).
const REUSE_MAX_TAIL_SAMPLES: usize = SAMPLE_RATE * 18 / 10;
/// Loudest uncovered tail still counted as silent (ASR-21).
const REUSE_MAX_TAIL_RMS: f32 = 0.002;

/// What a preview tick should do (ASR-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickDecision {
    /// Decode the whole recording so far, then call
    /// [`PreviewLoop::decode_finished`] or [`PreviewLoop::decode_failed`].
    Decode,
    /// Fewer than [`PREVIEW_MIN_SAMPLES`] captured.
    SkipTooShort,
    /// No audio arrived since the last successful preview decode.
    SkipNoNewAudio,
    /// A preview decode is still running; slow decodes skip ticks instead of
    /// queueing.
    SkipBusy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeSlot {
    Idle,
    InFlight { sample_count: usize },
}

/// Per-session live preview state (ASR-15, ASR-16, ASR-19).
///
/// Create one per recording. Each tick, call [`PreviewLoop::tick`] with the
/// captured sample count; on [`TickDecision::Decode`], decode and report the
/// result.
#[derive(Debug, Clone)]
pub struct PreviewLoop {
    decoded_sample_count: usize,
    slot: DecodeSlot,
    stabilizer: Stabilizer,
    published: String,
}

impl Default for PreviewLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewLoop {
    /// State for a new recording.
    pub fn new() -> Self {
        Self {
            decoded_sample_count: 0,
            slot: DecodeSlot::Idle,
            stabilizer: Stabilizer::new(),
            published: String::new(),
        }
    }

    /// Decides the tick for `sample_count` captured samples. On
    /// [`TickDecision::Decode`] the decode counts as in flight until it is
    /// reported.
    pub fn tick(&mut self, sample_count: usize) -> TickDecision {
        if self.slot != DecodeSlot::Idle {
            return TickDecision::SkipBusy;
        }
        if sample_count < PREVIEW_MIN_SAMPLES {
            return TickDecision::SkipTooShort;
        }
        if sample_count <= self.decoded_sample_count {
            return TickDecision::SkipNoNewAudio;
        }
        self.slot = DecodeSlot::InFlight { sample_count };
        TickDecision::Decode
    }

    /// Reports the in-flight decode's text and returns the preview to
    /// publish, if any (ASR-19: only when non-empty and changed).
    ///
    /// `text` is the decoded text with the session's replacement rules
    /// already applied (ASR-18). A report with no decode in flight is
    /// ignored.
    pub fn decode_finished(&mut self, text: &str) -> Option<String> {
        let DecodeSlot::InFlight { sample_count } = self.slot else {
            return None;
        };
        self.slot = DecodeSlot::Idle;
        self.decoded_sample_count = sample_count;
        let preview = self.stabilizer.stabilize(text.trim());
        if preview.is_empty() || preview == self.published {
            return None;
        }
        self.published.clone_from(&preview);
        Some(preview)
    }

    /// Reports that the in-flight decode failed or was cancelled; the next
    /// tick may retry the same audio (ASR-22: the error is only logged).
    pub fn decode_failed(&mut self) {
        self.slot = DecodeSlot::Idle;
    }
}

/// Freezes the leading words two consecutive preview decodes agree on, so
/// text already on screen stops rewriting itself (ASR-19).
///
/// Preview only: the typed transcript comes from the final decode.
#[derive(Debug, Clone, Default)]
pub struct Stabilizer {
    committed: Vec<String>,
    previous: Vec<String>,
}

impl Stabilizer {
    /// A stabilizer with nothing frozen.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the text to display for the newly decoded `text`.
    ///
    /// Words are separated by spaces. Agreement ignores case and leading or
    /// trailing non-alphanumeric characters. Newly agreed words freeze in the
    /// older decode's spelling; frozen words are never rewritten or dropped,
    /// so a shorter decode still shows them.
    pub fn stabilize(&mut self, text: &str) -> String {
        let words: Vec<String> = text
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect();
        let agreed = agreement_len(&self.previous, &words);
        if agreed > self.committed.len() {
            let newly_agreed = &self.previous[self.committed.len()..agreed];
            self.committed.extend_from_slice(newly_agreed);
        }
        let tail = words
            .get(tail_start(&self.committed, &words)..)
            .unwrap_or_default();
        let shown = [self.committed.as_slice(), tail].concat().join(" ");
        self.previous = words;
        shown
    }

    /// Forgets everything, for a new recording.
    pub fn reset(&mut self) {
        self.committed.clear();
        self.previous.clear();
    }
}

// where the unfrozen tail starts in `words`. a decode that reproduces the
// frozen run starts right after it; one that inserted or dropped a word
// inside the run is re-anchored after the run's last words, nearest the
// expected position, so the tail never repeats a frozen word. (Deviation:
// Jabber always slices at the frozen length, showing "group at at Greendale"
// when a decode inserts "Nadir" into the frozen run.)
fn tail_start(committed: &[String], words: &[String]) -> usize {
    let frozen = committed.len();
    if frozen == 0 || agreement_len(committed, words) == frozen {
        return frozen;
    }
    let anchor = committed.len().min(2);
    let matches_at = |end: usize| {
        (1..=anchor).all(|back| {
            end >= back && normalized(&words[end - back]) == normalized(&committed[frozen - back])
        })
    };
    (1..=words.len())
        .filter(|&end| matches_at(end))
        .min_by_key(|&end| end.abs_diff(frozen))
        .unwrap_or(frozen)
}

fn agreement_len(lhs: &[String], rhs: &[String]) -> usize {
    lhs.iter()
        .zip(rhs)
        .take_while(|(l, r)| normalized(l) == normalized(r))
        .count()
}

fn normalized(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// The last Parakeet TDT preview decode, kept so the final decode may reuse
/// it (ASR-21).
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewSnapshot {
    /// Trimmed model output, before replacements and stabilization.
    pub text: String,
    /// Samples the preview decoded.
    pub sample_count: usize,
    /// When the preview decode finished.
    pub finished_at: MonoTime,
}

impl PreviewSnapshot {
    /// Whether the final transcript of `final_samples` at `now` may be this
    /// preview's text instead of a full decode (ASR-21).
    ///
    /// All must hold: the text is non-empty; the recording is at least 2 s;
    /// the preview is at most 3 s old; it covered at least 88% of the
    /// samples; the uncovered tail is at most 1.8 s; and that tail is empty
    /// or silent (RMS ≤ 0.002).
    pub fn can_reuse(&self, final_samples: &[f32], now: MonoTime) -> bool {
        let total = final_samples.len();
        let covered = self.sample_count.min(total);
        let tail = &final_samples[covered..];
        !self.text.trim().is_empty()
            && total >= REUSE_MIN_SAMPLES
            && now.saturating_sub(self.finished_at) <= REUSE_MAX_AGE
            && covered * 100 >= total * REUSE_MIN_COVERAGE_PERCENT
            && tail.len() <= REUSE_MAX_TAIL_SAMPLES
            && (tail.is_empty() || rms_level(tail) <= REUSE_MAX_TAIL_RMS)
    }
}

/// How to feed the Parakeet Unified streaming session (ASR-17, ASR-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFeed {
    /// Append `samples[start..]`; the session already has everything before
    /// `start`.
    Continue {
        /// First sample the session has not seen.
        start: usize,
    },
    /// Reset the session, then append all samples: there was no session, or
    /// the call has fewer samples than were streamed, so it is a new
    /// recording.
    Restart,
}

/// Tracks how much of the current recording the Parakeet Unified streaming
/// session has been given (ASR-20).
///
/// Every decode call passes the whole recording so far; this decides what
/// part is new.
#[derive(Debug, Clone, Default)]
pub struct UnifiedStream {
    streamed: Option<usize>,
}

impl UnifiedStream {
    /// A tracker whose session must reset before it takes audio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decides how to feed a call carrying `sample_count` samples of the
    /// recording, and records them as streamed.
    pub fn feed(&mut self, sample_count: usize) -> StreamFeed {
        let feed = match self.streamed {
            Some(streamed) if streamed <= sample_count => StreamFeed::Continue { start: streamed },
            _ => StreamFeed::Restart,
        };
        self.streamed = Some(sample_count);
        feed
    }

    /// Marks the session as needing a reset before it takes more audio: after
    /// the final decode, on a preview reset, or on unload.
    pub fn reset(&mut self) {
        self.streamed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_decisions_asr_16() {
        struct Case {
            name: &'static str,
            // (captured samples, decode result: None = still running,
            // Some(Ok(text)) = finished, Some(Err(())) = failed)
            ticks: Vec<(usize, Option<Result<&'static str, ()>>)>,
            want: Vec<TickDecision>,
        }
        use TickDecision::*;
        let cases = [
            Case {
                name: "under 8,000 samples skips",
                ticks: vec![(7_999, None)],
                want: vec![SkipTooShort],
            },
            Case {
                name: "8,000 samples decodes",
                ticks: vec![(8_000, None)],
                want: vec![Decode],
            },
            Case {
                name: "one decode at a time",
                ticks: vec![(8_000, None), (12_000, None)],
                want: vec![Decode, SkipBusy],
            },
            Case {
                name: "no new audio since the last preview skips",
                ticks: vec![(16_000, Some(Ok("troy"))), (16_000, None)],
                want: vec![Decode, SkipNoNewAudio],
            },
            Case {
                name: "new audio decodes again",
                ticks: vec![(16_000, Some(Ok("troy"))), (20_800, None)],
                want: vec![Decode, Decode],
            },
            Case {
                name: "a failed decode retries the same audio",
                ticks: vec![(16_000, Some(Err(()))), (16_000, None)],
                want: vec![Decode, Decode],
            },
        ];
        for tc in cases {
            let mut preview = PreviewLoop::new();
            let mut got = Vec::new();
            for (samples, result) in &tc.ticks {
                got.push(preview.tick(*samples));
                match result {
                    Some(Ok(text)) => {
                        preview.decode_finished(text);
                    }
                    Some(Err(())) => preview.decode_failed(),
                    None => {}
                }
            }
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn publish_only_non_empty_changed_text_asr_19() {
        struct Case {
            name: &'static str,
            decodes: Vec<&'static str>,
            want: Vec<Option<&'static str>>,
        }
        let cases = [
            Case {
                name: "first decode publishes",
                decodes: vec!["troy and abed"],
                want: vec![Some("troy and abed")],
            },
            Case {
                name: "silence publishes nothing",
                decodes: vec!["", "   "],
                want: vec![None, None],
            },
            Case {
                name: "unchanged text is not republished",
                decodes: vec!["troy and abed", "troy and abed"],
                want: vec![Some("troy and abed"), None],
            },
            Case {
                name: "text is trimmed",
                decodes: vec!["  greendale  "],
                want: vec![Some("greendale")],
            },
            Case {
                name: "an empty decode keeps the frozen words on screen",
                decodes: vec!["troy and", "troy and abed", ""],
                want: vec![Some("troy and"), Some("troy and abed"), Some("troy and")],
            },
        ];
        for tc in cases {
            let mut preview = PreviewLoop::new();
            let mut got = Vec::new();
            for (i, text) in tc.decodes.iter().enumerate() {
                let decision = preview.tick(PREVIEW_MIN_SAMPLES + i);
                assert_eq!(decision, TickDecision::Decode, "{}", tc.name);
                got.push(preview.decode_finished(text));
            }
            let want: Vec<Option<String>> = tc.want.iter().map(|w| w.map(str::to_owned)).collect();
            assert_eq!(got, want, "{}", tc.name);
        }
    }

    #[test]
    fn decode_report_without_decode_in_flight_is_ignored() {
        let mut preview = PreviewLoop::new();
        assert_eq!(preview.decode_finished("troy"), None);
        assert_eq!(preview.tick(PREVIEW_MIN_SAMPLES), TickDecision::Decode);
    }

    #[test]
    fn stabilizer_asr_19() {
        struct Case {
            name: &'static str,
            decodes: Vec<&'static str>,
            want_last: &'static str,
        }
        let cases = [
            Case {
                name: "first decode is shown verbatim",
                decodes: vec!["troy"],
                want_last: "troy",
            },
            Case {
                name: "unsettled tail keeps moving until two decodes agree",
                decodes: vec!["troy bahnes", "troy barnes"],
                want_last: "troy barnes",
            },
            Case {
                name: "the tail grows after agreement",
                decodes: vec!["troy bahnes", "troy barnes", "troy barnes enrolled"],
                want_last: "troy barnes enrolled",
            },
            Case {
                name: "committed prefix survives a later revision",
                decodes: vec![
                    "troy barnes",
                    "troy barnes enrolled",
                    "troy burns enrolled at",
                ],
                want_last: "troy barnes enrolled at",
            },
            Case {
                name: "agreement ignores case and punctuation, older spelling wins",
                decodes: vec!["troy and abed", "Troy, and abed in"],
                want_last: "troy and abed in",
            },
            Case {
                name: "shorter decode keeps the committed text",
                decodes: vec!["troy and abed", "troy and abed in", "troy"],
                want_last: "troy and abed",
            },
            Case {
                name: "empty decode does not blank the preview",
                decodes: vec!["troy and", "troy and abed", ""],
                want_last: "troy and",
            },
            Case {
                name: "frozen words are never rewritten by later agreement",
                // jabber re-commits from the previous decode and would show "burns"
                decodes: vec![
                    "troy barnes",
                    "troy barnes enrolled",
                    "troy burns enrolled at",
                    "troy burns enrolled at greendale",
                ],
                want_last: "troy barnes enrolled at greendale",
            },
            Case {
                name: "a word inserted into the frozen run does not repeat the tail",
                decodes: vec![
                    "yeah films the study group at green",
                    "yeah films the study group at greendale",
                    "yeah nadir films the study group at greendale community",
                ],
                want_last: "yeah films the study group at greendale community",
            },
            Case {
                name: "a word dropped from the frozen run does not swallow the tail",
                decodes: vec![
                    "troy and abed in the",
                    "troy and abed in the mor",
                    "troy abed in the morning",
                ],
                want_last: "troy and abed in the morning",
            },
            Case {
                name: "no anchor falls back to the frozen length",
                decodes: vec!["troy barnes", "troy barnes", "pierce hawthorne sings"],
                want_last: "troy barnes sings",
            },
            Case {
                name: "unicode punctuation and case",
                decodes: vec!["¿Dónde está Abed?", "¿dónde está abed? en"],
                want_last: "¿Dónde está Abed? en",
            },
            Case {
                name: "repeated spaces do not make empty words",
                decodes: vec!["troy  and", "troy and  abed"],
                want_last: "troy and abed",
            },
        ];
        for tc in cases {
            let mut stabilizer = Stabilizer::new();
            let mut got = String::new();
            for text in &tc.decodes {
                got = stabilizer.stabilize(text);
            }
            assert_eq!(got, tc.want_last, "{}", tc.name);
        }
    }

    #[test]
    fn stabilizer_reset_clears_committed_text() {
        let mut stabilizer = Stabilizer::new();
        stabilizer.stabilize("troy and abed");
        stabilizer.stabilize("troy and abed");
        stabilizer.reset();
        assert_eq!(stabilizer.stabilize("greendale"), "greendale");
    }

    #[test]
    fn preview_reuse_asr_21() {
        struct Case {
            name: &'static str,
            text: &'static str,
            final_samples: Vec<f32>,
            preview_samples: usize,
            age_ms: u64,
            want: bool,
        }
        let speech = |n: usize| vec![0.1_f32; n];
        let with_tail = |covered: usize, tail: usize, level: f32| {
            [vec![0.1_f32; covered], vec![level; tail]].concat()
        };
        let cases = [
            Case {
                name: "recent preview with complete coverage",
                text: "Troy and Abed in the morning",
                final_samples: speech(48_000),
                preview_samples: 48_000,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "uncovered tail is silent",
                text: "Troy and Abed in the morning",
                final_samples: with_tail(43_000, 5_000, 0.001),
                preview_samples: 43_000,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "uncovered tail contains speech",
                text: "Troy and Abed in the",
                final_samples: with_tail(45_000, 3_000, 0.1),
                preview_samples: 45_000,
                age_ms: 400,
                want: false,
            },
            Case {
                name: "stale preview",
                text: "Troy and Abed in the morning",
                final_samples: speech(48_000),
                preview_samples: 48_000,
                age_ms: 4_000,
                want: false,
            },
            Case {
                name: "exactly 3 s old is still fresh",
                text: "Troy and Abed in the morning",
                final_samples: speech(48_000),
                preview_samples: 48_000,
                age_ms: 3_000,
                want: true,
            },
            Case {
                name: "empty preview text",
                text: "  ",
                final_samples: speech(48_000),
                preview_samples: 48_000,
                age_ms: 400,
                want: false,
            },
            Case {
                name: "recording under 2 s",
                text: "Troy",
                final_samples: speech(31_999),
                preview_samples: 31_999,
                age_ms: 400,
                want: false,
            },
            Case {
                name: "recording of exactly 2 s",
                text: "Troy",
                final_samples: speech(32_000),
                preview_samples: 32_000,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "coverage just under 88%",
                text: "Troy",
                final_samples: with_tail(87_999, 12_001, 0.0),
                preview_samples: 87_999,
                age_ms: 400,
                want: false,
            },
            Case {
                name: "coverage of exactly 88%",
                text: "Troy",
                final_samples: with_tail(88_000, 12_000, 0.0),
                preview_samples: 88_000,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "silent tail longer than 1.8 s",
                text: "Troy",
                final_samples: with_tail(271_199, 28_801, 0.0),
                preview_samples: 271_199,
                age_ms: 400,
                want: false,
            },
            Case {
                name: "silent tail of exactly 1.8 s",
                text: "Troy",
                final_samples: with_tail(271_200, 28_800, 0.0),
                preview_samples: 271_200,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "tail RMS just under 0.002 counts as silent",
                text: "Troy",
                final_samples: with_tail(45_000, 3_000, 0.0019),
                preview_samples: 45_000,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "preview longer than the final buffer counts as full coverage",
                text: "Troy",
                final_samples: speech(48_000),
                preview_samples: 50_000,
                age_ms: 400,
                want: true,
            },
            Case {
                name: "no preview audio",
                text: "Troy",
                final_samples: speech(48_000),
                preview_samples: 0,
                age_ms: 400,
                want: false,
            },
        ];
        for tc in cases {
            let finished_at = Duration::from_secs(100);
            let snapshot = PreviewSnapshot {
                text: tc.text.to_owned(),
                sample_count: tc.preview_samples,
                finished_at,
            };
            let now = finished_at + Duration::from_millis(tc.age_ms);
            assert_eq!(
                snapshot.can_reuse(&tc.final_samples, now),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn unified_stream_feeds_only_unseen_audio_asr_20() {
        struct Case {
            name: &'static str,
            streamed: Option<usize>,
            total: usize,
            want: StreamFeed,
        }
        let cases = [
            Case {
                name: "a fresh session starts over",
                streamed: None,
                total: 16_000,
                want: StreamFeed::Restart,
            },
            Case {
                name: "a longer recording continues",
                streamed: Some(8_000),
                total: 16_000,
                want: StreamFeed::Continue { start: 8_000 },
            },
            Case {
                name: "nothing new continues at the end",
                streamed: Some(16_000),
                total: 16_000,
                want: StreamFeed::Continue { start: 16_000 },
            },
            Case {
                name: "a shorter recording is a new one",
                streamed: Some(16_000),
                total: 8_000,
                want: StreamFeed::Restart,
            },
        ];
        for tc in cases {
            let mut stream = UnifiedStream::new();
            if let Some(streamed) = tc.streamed {
                stream.feed(streamed);
            }
            assert_eq!(stream.feed(tc.total), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn unified_stream_reset_forces_restart_asr_20() {
        let mut stream = UnifiedStream::new();
        stream.feed(16_000);
        stream.reset();
        assert_eq!(stream.feed(32_000), StreamFeed::Restart);
        assert_eq!(stream.feed(40_000), StreamFeed::Continue { start: 32_000 });
    }

    #[test]
    fn unified_window_constants_asr_17() {
        let frame_ms = UNIFIED_FRAME_SAMPLES * 1_000 / SAMPLE_RATE;
        let cases = [
            ("frame is 80 ms", frame_ms, 80),
            (
                "left context is 5.6 s",
                UNIFIED_LEFT_CONTEXT_FRAMES * frame_ms,
                5_600,
            ),
            ("chunk is 0.56 s", UNIFIED_CHUNK_FRAMES * frame_ms, 560),
            (
                "look-ahead is 0.56 s",
                UNIFIED_LOOKAHEAD_FRAMES * frame_ms,
                560,
            ),
        ];
        for (name, got, want) in cases {
            assert_eq!(got, want, "{name}");
        }
    }
}
