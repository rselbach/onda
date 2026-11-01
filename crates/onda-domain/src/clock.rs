//! Recording overlay clock and session limit (DICT-8, DICT-9).

use std::time::Duration;

use crate::types::MonoTime;

/// Longest a dictation session records before it stops itself (DICT-8).
pub const SESSION_LIMIT: Duration = Duration::from_secs(900);

/// How long before [`SESSION_LIMIT`] the clock becomes a warning countdown
/// (DICT-9).
pub const CLOCK_WARNING_WINDOW: Duration = Duration::from_secs(60);

/// Title of the warning posted when [`SESSION_LIMIT`] stops a recording
/// (DICT-8).
pub const RECORDING_LIMIT_TITLE: &str = "Recording Limit Reached";

/// Message of the warning posted when [`SESSION_LIMIT`] stops a recording
/// (DICT-8).
pub const RECORDING_LIMIT_MESSAGE: &str = "Onda stopped recording at the 15-minute limit and is transcribing what was captured. Start a new dictation to continue.";

/// What the overlay clock shows (DICT-9, OVL-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingClock {
    /// `m:ss` elapsed, or `Stops in m:ss` inside the warning window.
    pub text: String,
    /// True inside the warning window; the shell draws the text orange.
    pub is_warning: bool,
}

impl RecordingClock {
    /// The clock at `now` for a recording that began at `recording_started_at`.
    ///
    /// `recording_started_at` is when capture began, including any
    /// provisional Hold time, not when the overlay appeared (DICT-9).
    /// Elapsed time rounds down; the countdown rounds up so it reads `0:00`
    /// only at the stop itself.
    pub fn at(recording_started_at: MonoTime, now: MonoTime) -> Self {
        let elapsed = now.saturating_sub(recording_started_at);
        let remaining = SESSION_LIMIT.saturating_sub(elapsed);
        if remaining > CLOCK_WARNING_WINDOW {
            return Self {
                text: format_minutes(elapsed.as_secs()),
                is_warning: false,
            };
        }
        let remaining_secs = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
        Self {
            text: format!("Stops in {}", format_minutes(remaining_secs)),
            is_warning: true,
        }
    }
}

fn format_minutes(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_counts_up_then_warns_before_the_limit_dict_9() {
        struct Case {
            name: &'static str,
            started_at: f64,
            now: f64,
            want_text: &'static str,
            want_warning: bool,
        }
        let cases = [
            Case {
                name: "just started",
                started_at: 0.0,
                now: 0.0,
                want_text: "0:00",
                want_warning: false,
            },
            Case {
                name: "partial seconds round down",
                started_at: 0.0,
                now: 7.9,
                want_text: "0:07",
                want_warning: false,
            },
            Case {
                name: "over a minute",
                started_at: 0.0,
                now: 65.0,
                want_text: "1:05",
                want_warning: false,
            },
            Case {
                name: "just outside the warning window",
                started_at: 0.0,
                now: 839.9,
                want_text: "13:59",
                want_warning: false,
            },
            Case {
                name: "warning window starts",
                started_at: 0.0,
                now: 840.0,
                want_text: "Stops in 1:00",
                want_warning: true,
            },
            Case {
                name: "countdown rounds up",
                started_at: 0.0,
                now: 855.2,
                want_text: "Stops in 0:45",
                want_warning: true,
            },
            Case {
                name: "countdown shows 0:01 until the very end",
                started_at: 0.0,
                now: 899.99,
                want_text: "Stops in 0:01",
                want_warning: true,
            },
            Case {
                name: "at the limit",
                started_at: 0.0,
                now: 900.0,
                want_text: "Stops in 0:00",
                want_warning: true,
            },
            Case {
                name: "past the limit",
                started_at: 0.0,
                now: 905.0,
                want_text: "Stops in 0:00",
                want_warning: true,
            },
            Case {
                name: "measured from recording start, provisional time included",
                started_at: 100.0,
                now: 100.3 + 7.0,
                want_text: "0:07",
                want_warning: false,
            },
            Case {
                name: "a now before the start reads 0:00",
                started_at: 10.0,
                now: 9.0,
                want_text: "0:00",
                want_warning: false,
            },
        ];
        for tc in cases {
            let got = RecordingClock::at(
                Duration::from_secs_f64(tc.started_at),
                Duration::from_secs_f64(tc.now),
            );
            assert_eq!(got.text, tc.want_text, "{}", tc.name);
            assert_eq!(got.is_warning, tc.want_warning, "{}", tc.name);
        }
    }

    #[test]
    fn session_limit_is_fifteen_minutes_dict_8() {
        assert_eq!(SESSION_LIMIT, Duration::from_secs(15 * 60));
    }
}
