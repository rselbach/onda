//! The cleanup decision flow (CLN-1, CLN-2, CLN-12..CLN-15) and its copy.
//!
//! The orchestrator drives the requests; these functions decide what happens
//! after each step:
//!
//! 1. [`start`] skips, or returns the key for request 1.
//! 2. [`after_first_attempt`] finishes, or asks for request 2.
//! 3. [`after_retry`] finishes.
//!
//! A cancelled request is not an attempt (CLN-9); the caller drops the
//! session instead of calling these.

use super::guardrails::{GuardrailViolation, check_guardrails};
use super::openrouter::{ApiKey, CleanupError};
use super::provider::CleanupProvider;
use crate::types::OutputMode;

/// History error recorded when no usable key is stored (CLN-2), e.g.
/// "OpenRouter unavailable".
pub fn unavailable_error(provider: CleanupProvider) -> String {
    format!("{} unavailable", provider.display_name())
}

/// What cleanup did to a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupOutcome {
    /// Cleanup is off or the transcript was blank: deliver the raw
    /// transcript, record nothing about cleanup (CLN-1).
    NotRun,
    /// Use the cleaned text. It may be empty: the user said "scratch that",
    /// so nothing is delivered and no warning shows (CLN-12).
    Cleaned {
        /// Trimmed cleaned text.
        text: String,
    },
    /// Deliver the raw transcript and record `error` in history (CLN-15).
    FellBack {
        /// History error description.
        error: String,
        /// Notice to show, if any. A missing key shows none (CLN-2).
        notice: Option<CleanupNotice>,
    },
}

impl CleanupOutcome {
    /// The text to store as the transcript and deliver (before replacements).
    pub fn text<'a>(&'a self, raw: &'a str) -> &'a str {
        match self {
            Self::Cleaned { text } => text,
            Self::NotRun | Self::FellBack { .. } => raw,
        }
    }

    /// Whether history marks the entry as cleaned and keeps the raw
    /// transcript (CLN-15).
    pub fn was_cleaned(&self) -> bool {
        matches!(self, Self::Cleaned { .. })
    }

    /// The history error description, when cleanup fell back (CLN-15).
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::FellBack { error, .. } => Some(error),
            Self::NotRun | Self::Cleaned { .. } => None,
        }
    }

    /// The notice to show, if any.
    pub fn notice(&self) -> Option<&CleanupNotice> {
        match self {
            Self::FellBack { notice, .. } => notice.as_ref(),
            Self::NotRun | Self::Cleaned { .. } => None,
        }
    }
}

/// A user-visible notice about a cleanup fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupNotice {
    /// Request 1 failed (CLN-13). Feedback notice, rate limited to one per
    /// 1.5 s.
    CouldNotRefine {
        /// The provider that failed.
        provider: CleanupProvider,
        /// The [`CleanupError`] message.
        error: String,
    },
    /// The guardrails rejected both attempts (CLN-14). Overlay notice only,
    /// never a click-to-dismiss dialog.
    UsedRawTranscript,
}

impl CleanupNotice {
    /// Notice title.
    pub fn title(&self) -> &'static str {
        match self {
            Self::CouldNotRefine { .. } => "Couldn't Refine Transcript",
            Self::UsedRawTranscript => "Used Raw Transcript",
        }
    }

    /// Notice message; CLN-13 names where the raw transcript went.
    pub fn message(&self, mode: OutputMode) -> String {
        match self {
            Self::CouldNotRefine { provider, error } => {
                let destination = match mode {
                    OutputMode::Type => "Typed the raw transcript instead.",
                    OutputMode::Clipboard => "Copied the raw transcript to the clipboard instead.",
                };
                let name = provider.display_name();
                format!("{name} cleanup failed ({error}). {destination}")
            }
            Self::UsedRawTranscript => "Post-processing looked wrong.".to_owned(),
        }
    }
}

/// The first decision, made before any request.
#[derive(Debug, Clone, PartialEq)]
pub enum CleanupStart {
    /// No request; this is the outcome.
    Skip(CleanupOutcome),
    /// Show "Refining..." (CLN-3) and send request 1 with this key. Use the
    /// same key for a retry (CLN-2).
    Request(ApiKey),
}

/// Decides whether cleanup runs (CLN-1, CLN-2).
///
/// `stored_key` is the credential-store value read once for this dictation;
/// pass `None` when it is absent or the read failed (log the failure).
pub fn start(
    provider: CleanupProvider,
    enabled: bool,
    raw: &str,
    stored_key: Option<&str>,
) -> CleanupStart {
    if !enabled || raw.trim().is_empty() {
        return CleanupStart::Skip(CleanupOutcome::NotRun);
    }
    match stored_key.map(ApiKey::new) {
        Some(Ok(key)) => CleanupStart::Request(key),
        Some(Err(_)) | None => CleanupStart::Skip(CleanupOutcome::FellBack {
            error: unavailable_error(provider),
            notice: None,
        }),
    }
}

/// The decision after request 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupStep {
    /// No more requests; this is the outcome.
    Done(CleanupOutcome),
    /// The guardrails rejected the output; send request 2 (CLN-14).
    Retry(GuardrailViolation),
}

/// Decides what follows request 1 (CLN-12..CLN-14).
///
/// `result` is [`super::parse_response`]'s value or the transport error.
pub fn after_first_attempt(
    provider: CleanupProvider,
    raw: &str,
    result: Result<String, CleanupError>,
) -> CleanupStep {
    let cleaned = match result {
        Ok(cleaned) => cleaned.trim().to_owned(),
        Err(error) => {
            let error = error.describe(provider);
            return CleanupStep::Done(CleanupOutcome::FellBack {
                notice: Some(CleanupNotice::CouldNotRefine {
                    provider,
                    error: error.clone(),
                }),
                error,
            });
        }
    };
    if cleaned.is_empty() {
        return CleanupStep::Done(CleanupOutcome::Cleaned { text: cleaned });
    }
    match check_guardrails(raw, &cleaned) {
        Ok(()) => CleanupStep::Done(CleanupOutcome::Cleaned { text: cleaned }),
        Err(violation) => CleanupStep::Retry(violation),
    }
}

/// Decides the outcome after request 2 (CLN-14, CLN-15).
///
/// Only non-empty output that passes the guardrails is used. An empty retry
/// is ambiguous after a rejected first pass and records the shrinkage
/// message; a failed retry records its error. Every fallback here shows
/// "Used Raw Transcript".
pub fn after_retry(
    provider: CleanupProvider,
    raw: &str,
    result: Result<String, CleanupError>,
) -> CleanupOutcome {
    let fell_back = |error: String| CleanupOutcome::FellBack {
        error,
        notice: Some(CleanupNotice::UsedRawTranscript),
    };
    let cleaned = match result {
        Ok(cleaned) => cleaned.trim().to_owned(),
        Err(error) => return fell_back(error.describe(provider)),
    };
    if cleaned.is_empty() {
        let violation = GuardrailViolation::Shrinkage {
            raw_words: raw.split_whitespace().count(),
            cleaned_words: 0,
        };
        return fell_back(violation.to_string());
    }
    match check_guardrails(raw, &cleaned) {
        Ok(()) => CleanupOutcome::Cleaned { text: cleaned },
        Err(violation) => fell_back(violation.to_string()),
    }
}

/// Sample the "Test with Sample Text" button sends, with no guardrails and
/// no app name (CLN-27).
pub const TEST_SAMPLE: &str = "um so tell troy and abed that the study group meets at three no wait four in the library thanks";

/// Alert title when the cleanup test fails; the message is the error (CLN-27).
pub const TEST_FAILED_TITLE: &str = "Test Failed";

/// First line of a finished test: `Said: <sample>` (CLN-27).
pub fn test_said_line() -> String {
    format!("Said: {TEST_SAMPLE}")
}

/// Second line of a finished test: `Result: <output>`, or a note that the
/// model removed everything (CLN-27).
pub fn test_result_line(output: &str) -> String {
    let output = output.trim();
    if output.is_empty() {
        return "Result: (nothing — the model removed it all)".to_owned();
    }
    format!("Result: {output}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREENDALE: &str = "Troy and Abed are studying at Greendale Community College for their Spanish exam with Señor Chang and they hope to pass the class this semester.";
    const GREENDALE_CLEANED: &str = "Troy and Abed are studying at Greendale Community College for their Spanish exam with Señor Chang, and they hope to pass the class this semester.";
    const SHRINKAGE_MESSAGE: &str = "Post-processing output looked too different from the transcript, so Onda used the raw transcript.";
    const MARKDOWN_MESSAGE: &str =
        "Post-processing produced unexpected Markdown formatting, so Onda used the raw transcript.";

    fn fell_back(error: &str, notice: Option<CleanupNotice>) -> CleanupOutcome {
        CleanupOutcome::FellBack {
            error: error.to_owned(),
            notice,
        }
    }

    fn cleaned(text: &str) -> CleanupOutcome {
        CleanupOutcome::Cleaned {
            text: text.to_owned(),
        }
    }

    #[test]
    fn start_cln_1_cln_2() {
        struct Case {
            name: &'static str,
            enabled: bool,
            raw: &'static str,
            key: Option<&'static str>,
            want: CleanupStart,
        }
        let unavailable = CleanupStart::Skip(fell_back("OpenRouter unavailable", None));
        let cases = [
            Case {
                name: "CLN-1 disabled never requests",
                enabled: false,
                raw: " um hello ",
                key: Some("sk-greendale"),
                want: CleanupStart::Skip(CleanupOutcome::NotRun),
            },
            Case {
                name: "CLN-1 blank transcript never requests",
                enabled: true,
                raw: "   ",
                key: Some("sk-greendale"),
                want: CleanupStart::Skip(CleanupOutcome::NotRun),
            },
            Case {
                name: "CLN-2 missing key is unavailable without a notice",
                enabled: true,
                raw: "raw only",
                key: None,
                want: unavailable.clone(),
            },
            Case {
                name: "CLN-2 blank key is unavailable without a notice",
                enabled: true,
                raw: "raw only",
                key: Some(" \n "),
                want: unavailable,
            },
            Case {
                name: "CLN-2 a stored key requests with that key",
                enabled: true,
                raw: " um hello ",
                key: Some(" sk-greendale "),
                want: CleanupStart::Request(ApiKey::new("sk-greendale").expect("key")),
            },
        ];
        for tc in cases {
            assert_eq!(
                start(CleanupProvider::OpenRouter, tc.enabled, tc.raw, tc.key),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn after_first_attempt_cln_12_to_cln_14() {
        struct Case {
            name: &'static str,
            raw: &'static str,
            result: Result<&'static str, CleanupError>,
            want: CleanupStep,
        }
        let cases = [
            Case {
                name: "passing output is used",
                raw: " um hello ",
                result: Ok("Hello."),
                want: CleanupStep::Done(cleaned("Hello.")),
            },
            Case {
                name: "output is trimmed",
                raw: " um hello ",
                result: Ok("  Hello.\n"),
                want: CleanupStep::Done(cleaned("Hello.")),
            },
            Case {
                name: "CLN-12 empty output is a valid cancel",
                raw: "troy and abed",
                result: Ok("   "),
                want: CleanupStep::Done(cleaned("")),
            },
            Case {
                name: "CLN-12 empty output of a long transcript is still valid",
                raw: GREENDALE,
                result: Ok(""),
                want: CleanupStep::Done(cleaned("")),
            },
            Case {
                name: "CLN-13 a throw is not retried and shows the error notice",
                raw: "cool cool cool",
                result: Err(CleanupError::Transport("Greendale is offline".to_owned())),
                want: CleanupStep::Done(fell_back(
                    "OpenRouter request failed: Greendale is offline",
                    Some(CleanupNotice::CouldNotRefine {
                        provider: CleanupProvider::OpenRouter,
                        error: "OpenRouter request failed: Greendale is offline".to_owned(),
                    }),
                )),
            },
            Case {
                name: "CLN-14 shrinkage triggers a retry",
                raw: GREENDALE,
                result: Ok("Studying at Greendale."),
                want: CleanupStep::Retry(GuardrailViolation::Shrinkage {
                    raw_words: 25,
                    cleaned_words: 3,
                }),
            },
            Case {
                name: "CLN-14 rogue markdown triggers a retry",
                raw: "the trojan horse is a great plan",
                result: Ok("# The Trojan Horse Is A Great Plan"),
                want: CleanupStep::Retry(GuardrailViolation::RogueMarkdown),
            },
            Case {
                name: "CLN-18 dictated header keeps markdown",
                raw: "header shopping list",
                result: Ok("# Shopping List"),
                want: CleanupStep::Done(cleaned("# Shopping List")),
            },
        ];
        for tc in cases {
            let got = after_first_attempt(
                CleanupProvider::OpenRouter,
                tc.raw,
                tc.result.map(str::to_owned),
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn after_retry_cln_14_cln_15() {
        struct Case {
            name: &'static str,
            raw: &'static str,
            result: Result<&'static str, CleanupError>,
            want: CleanupOutcome,
        }
        let used_raw = Some(CleanupNotice::UsedRawTranscript);
        let cases = [
            Case {
                name: "passing retry is used",
                raw: GREENDALE,
                result: Ok(GREENDALE_CLEANED),
                want: cleaned(GREENDALE_CLEANED),
            },
            Case {
                name: "retry still shrunk falls back with the shrinkage message",
                raw: GREENDALE,
                result: Ok("Studying at Greendale."),
                want: fell_back(SHRINKAGE_MESSAGE, used_raw.clone()),
            },
            Case {
                name: "retry still markdown falls back with the markdown message",
                raw: "the trojan horse is a great plan",
                result: Ok("# The Trojan Horse Is A Great Plan"),
                want: fell_back(MARKDOWN_MESSAGE, used_raw.clone()),
            },
            Case {
                name: "CLN-15 empty retry records the shrinkage message",
                raw: "the trojan horse is a great plan",
                result: Ok("  "),
                want: fell_back(SHRINKAGE_MESSAGE, used_raw.clone()),
            },
            Case {
                name: "retry throw stays non-disruptive",
                raw: GREENDALE,
                result: Err(CleanupError::Http {
                    status: 503,
                    message: None,
                }),
                want: fell_back("OpenRouter returned HTTP 503.", used_raw),
            },
        ];
        for tc in cases {
            let got = after_retry(
                CleanupProvider::OpenRouter,
                tc.raw,
                tc.result.map(str::to_owned),
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn outcome_history_fields_cln_15() {
        struct Case {
            name: &'static str,
            outcome: CleanupOutcome,
            want_text: &'static str,
            want_cleaned: bool,
            want_error: Option<&'static str>,
        }
        let raw = " um hello ";
        let cases = [
            Case {
                name: "not run delivers raw",
                outcome: CleanupOutcome::NotRun,
                want_text: raw,
                want_cleaned: false,
                want_error: None,
            },
            Case {
                name: "cleaned delivers the cleaned text",
                outcome: cleaned("Hello."),
                want_text: "Hello.",
                want_cleaned: true,
                want_error: None,
            },
            Case {
                name: "cleaned to nothing delivers nothing",
                outcome: cleaned(""),
                want_text: "",
                want_cleaned: true,
                want_error: None,
            },
            Case {
                name: "fallback delivers raw and records the error",
                outcome: fell_back("OpenRouter unavailable", None),
                want_text: raw,
                want_cleaned: false,
                want_error: Some("OpenRouter unavailable"),
            },
        ];
        for tc in cases {
            assert_eq!(tc.outcome.text(raw), tc.want_text, "{}", tc.name);
            assert_eq!(tc.outcome.was_cleaned(), tc.want_cleaned, "{}", tc.name);
            assert_eq!(tc.outcome.error(), tc.want_error, "{}", tc.name);
        }
    }

    #[test]
    fn notice_copy_cln_13_cln_14() {
        struct Case {
            name: &'static str,
            notice: CleanupNotice,
            mode: OutputMode,
            want_title: &'static str,
            want_message: &'static str,
        }
        let offline = || CleanupNotice::CouldNotRefine {
            provider: CleanupProvider::OpenRouter,
            error: "Greendale is offline".to_owned(),
        };
        let cases = [
            Case {
                name: "CLN-13 typing",
                notice: offline(),
                mode: OutputMode::Type,
                want_title: "Couldn't Refine Transcript",
                want_message: "OpenRouter cleanup failed (Greendale is offline). Typed the raw transcript instead.",
            },
            Case {
                name: "CLN-13 clipboard",
                notice: offline(),
                mode: OutputMode::Clipboard,
                want_title: "Couldn't Refine Transcript",
                want_message: "OpenRouter cleanup failed (Greendale is offline). Copied the raw transcript to the clipboard instead.",
            },
            Case {
                name: "CLN-14 either mode",
                notice: CleanupNotice::UsedRawTranscript,
                mode: OutputMode::Clipboard,
                want_title: "Used Raw Transcript",
                want_message: "Post-processing looked wrong.",
            },
        ];
        for tc in cases {
            assert_eq!(tc.notice.title(), tc.want_title, "{}", tc.name);
            assert_eq!(tc.notice.message(tc.mode), tc.want_message, "{}", tc.name);
        }
    }

    #[test]
    fn test_button_copy_cln_27() {
        struct Case {
            name: &'static str,
            output: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "output",
                output: "Tell Troy and Abed that the study group meets at 4 in the library. Thanks!",
                want: "Result: Tell Troy and Abed that the study group meets at 4 in the library. Thanks!",
            },
            Case {
                name: "output is trimmed",
                output: "  Thanks!\n",
                want: "Result: Thanks!",
            },
            Case {
                name: "empty output",
                output: " ",
                want: "Result: (nothing — the model removed it all)",
            },
        ];
        for tc in cases {
            assert_eq!(test_result_line(tc.output), tc.want, "{}", tc.name);
        }
        assert_eq!(
            test_said_line(),
            "Said: um so tell troy and abed that the study group meets at three no wait four in the library thanks",
            "said line"
        );
    }
}
