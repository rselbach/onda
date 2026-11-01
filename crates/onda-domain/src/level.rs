//! Input level metering, silent-microphone detection, and the overlay
//! waveform model (AUD-8, AUD-13, OVL-8).

use std::collections::VecDeque;
use std::time::Duration;

use crate::types::MonoTime;

/// Shortest gap between two level reports: at most 30 per second (AUD-8).
pub const LEVEL_REPORT_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);

/// A level below this counts as no signal, about -70 dBFS (AUD-13).
pub const SILENT_MIC_THRESHOLD: f32 = 0.0003;

/// How long the level must stay below [`SILENT_MIC_THRESHOLD`] before the
/// overlay shows the silent-microphone message (AUD-13).
pub const SILENT_MIC_DURATION: Duration = Duration::from_millis(1_500);

/// Body of the silent-microphone message (AUD-13).
pub const SILENT_MIC_BODY: &str = "Check that it's unmuted, or pick another input in Settings.";

/// Levels the waveform keeps (OVL-8).
pub const WAVEFORM_HISTORY: usize = 60;

/// Waveform bar width in points (OVL-8).
pub const BAR_WIDTH: f64 = 3.0;

/// Gap between waveform bars in points (OVL-8).
pub const BAR_SPACING: f64 = 3.0;

/// Height of a silent waveform bar in points (OVL-8).
pub const MIN_BAR_HEIGHT: f64 = 3.0;

/// RMS of one captured buffer, clamped to `[0, 1]` (AUD-8). Empty is `0`.
///
/// Jabber measures channel 0 of the device-format buffer before resampling;
/// [`SILENT_MIC_THRESHOLD`] is tuned to that.
pub fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt().min(1.0)
}

/// Rate-limits level reports to [`LEVEL_REPORT_INTERVAL`] (AUD-8).
#[derive(Debug, Clone, Default)]
pub struct LevelThrottle {
    last_report: Option<MonoTime>,
}

impl LevelThrottle {
    /// Whether a level measured at `now` should be reported; records the
    /// report when it should. The first call always reports.
    pub fn should_report(&mut self, now: MonoTime) -> bool {
        if let Some(last) = self.last_report
            && now.saturating_sub(last) < LEVEL_REPORT_INTERVAL
        {
            return false;
        }
        self.last_report = Some(now);
        true
    }
}

/// Spots a microphone that delivers no signal at all while recording
/// (AUD-13). The threshold sits far below any live microphone's noise floor,
/// so a pause to think never trips it.
#[derive(Debug, Clone, Default)]
pub struct SilentMicDetector {
    silent_since: Option<MonoTime>,
}

impl SilentMicDetector {
    /// Records `level` heard at `now`; returns whether the input has been
    /// silent for [`SILENT_MIC_DURATION`]. Any sound clears it at once.
    pub fn observe(&mut self, level: f32, now: MonoTime) -> bool {
        if level >= SILENT_MIC_THRESHOLD {
            self.silent_since = None;
            return false;
        }
        let since = *self.silent_since.get_or_insert(now);
        now.saturating_sub(since) >= SILENT_MIC_DURATION
    }

    /// Forgets any silence seen so far, for a new session.
    pub fn reset(&mut self) {
        self.silent_since = None;
    }
}

/// Title of the silent-microphone message, naming the input when known
/// (AUD-13). An empty name counts as unknown.
pub fn silent_mic_title(device_name: Option<&str>) -> String {
    match device_name {
        Some(name) if !name.is_empty() => format!("No sound from {name}"),
        _ => "No sound from the microphone".to_owned(),
    }
}

/// The last [`WAVEFORM_HISTORY`] reported levels, oldest first (OVL-8).
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformHistory {
    levels: VecDeque<f32>,
}

impl Default for WaveformHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl WaveformHistory {
    /// An empty history.
    pub fn new() -> Self {
        Self {
            levels: VecDeque::with_capacity(WAVEFORM_HISTORY),
        }
    }

    /// Adds the newest level, dropping the oldest past [`WAVEFORM_HISTORY`].
    pub fn push(&mut self, level: f32) {
        if self.levels.len() == WAVEFORM_HISTORY {
            self.levels.pop_front();
        }
        self.levels.push_back(level);
    }

    /// The kept levels, oldest first.
    pub fn levels(&self) -> Vec<f32> {
        self.levels.iter().copied().collect()
    }

    /// Clears the history for a new session (OVL-11).
    pub fn reset(&mut self) {
        self.levels.clear();
    }
}

/// Number of bars that fit in `width` points; at least one (OVL-8).
pub fn bar_count(width: f64) -> usize {
    let fit = ((width + BAR_SPACING) / (BAR_WIDTH + BAR_SPACING)).floor();
    if fit.is_finite() && fit >= 1.0 {
        fit as usize
    } else {
        1
    }
}

/// The levels to draw across `width` points, newest on the right: the most
/// recent that fit, left-padded with silence so the meter spans the full
/// width (OVL-8).
pub fn display_levels(levels: &[f32], width: f64) -> Vec<f32> {
    let count = bar_count(width);
    let recent = &levels[levels.len().saturating_sub(count)..];
    let mut bars = vec![0.0; count - recent.len()];
    bars.extend_from_slice(recent);
    bars
}

/// Height in points of a bar for `level` when `available` points are free:
/// `min + clamp(level × 10, 0, 1) × (available − min)` (OVL-8). Never below
/// [`MIN_BAR_HEIGHT`].
pub fn bar_height(level: f32, available: f64) -> f64 {
    let normalized = (f64::from(level) * 10.0).clamp(0.0, 1.0);
    MIN_BAR_HEIGHT + normalized * (available - MIN_BAR_HEIGHT).max(0.0)
}

/// The bar heights in points to draw `levels` in a `width` × `height`
/// point box, left to right with the newest on the right (OVL-8).
pub fn bar_heights(levels: &[f32], width: f64, height: f64) -> Vec<f64> {
    display_levels(levels, width)
        .into_iter()
        .map(|level| bar_height(level, height))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> MonoTime {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn rms_level_aud_8() {
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            want: f32,
        }
        let cases = [
            Case {
                name: "empty buffer is silent",
                samples: vec![],
                want: 0.0,
            },
            Case {
                name: "constant amplitude",
                samples: vec![0.5; 256],
                want: 0.5,
            },
            Case {
                name: "sign does not matter",
                samples: vec![-0.25, 0.25, -0.25, 0.25],
                want: 0.25,
            },
            Case {
                name: "clamped to 1",
                samples: vec![3.0; 16],
                want: 1.0,
            },
        ];
        for tc in cases {
            let got = rms_level(&tc.samples);
            assert!((got - tc.want).abs() < 1e-6, "{}: got {got}", tc.name);
        }
    }

    #[test]
    fn level_throttle_reports_at_most_30_hz_aud_8() {
        struct Case {
            name: &'static str,
            times: Vec<f64>,
            want: Vec<bool>,
        }
        let cases = [
            Case {
                name: "first level always reports",
                times: vec![5.0],
                want: vec![true],
            },
            Case {
                name: "levels inside the interval are dropped",
                times: vec![0.0, 0.01, 0.03, 0.034, 0.05, 0.068],
                want: vec![true, false, false, true, false, true],
            },
            Case {
                name: "dropped levels do not push the next report back",
                times: vec![0.0, 0.02, 0.04],
                want: vec![true, false, true],
            },
        ];
        for tc in cases {
            let mut throttle = LevelThrottle::default();
            let got: Vec<bool> = tc
                .times
                .iter()
                .map(|t| throttle.should_report(secs(*t)))
                .collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn silent_mic_detector_aud_13() {
        struct Case {
            name: &'static str,
            levels: Vec<(f32, f64)>,
            want: bool,
        }
        let cases = [
            Case {
                name: "silent long enough",
                levels: vec![(0.0, 0.0), (0.0001, 1.0), (0.0, 1.5)],
                want: true,
            },
            Case {
                name: "silent too briefly",
                levels: vec![(0.0, 0.0), (0.0, 1.4)],
                want: false,
            },
            Case {
                name: "room noise is not silence",
                levels: vec![(0.001, 0.0), (0.001, 2.0)],
                want: false,
            },
            Case {
                name: "the threshold itself is sound",
                levels: vec![(0.0003, 0.0), (0.0003, 2.0)],
                want: false,
            },
            Case {
                name: "sound restarts the clock",
                levels: vec![(0.0, 0.0), (0.01, 1.0), (0.0, 1.2), (0.0, 2.6)],
                want: false,
            },
            Case {
                name: "sound clears it the moment it arrives",
                levels: vec![(0.0, 0.0), (0.0, 2.0), (0.01, 2.1)],
                want: false,
            },
        ];
        for tc in cases {
            let mut detector = SilentMicDetector::default();
            let mut got = false;
            for (level, time) in &tc.levels {
                got = detector.observe(*level, secs(*time));
            }
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn silent_mic_clears_on_reset_aud_13() {
        let mut detector = SilentMicDetector::default();
        assert!(!detector.observe(0.0, secs(0.0)));
        assert!(detector.observe(0.0, secs(2.0)));
        detector.reset();
        assert!(!detector.observe(0.0, secs(2.1)));
    }

    #[test]
    fn silent_mic_title_aud_13() {
        let cases = [
            (
                "named device",
                Some("Abed's Podcast Mic"),
                "No sound from Abed's Podcast Mic",
            ),
            ("unknown device", None, "No sound from the microphone"),
            ("empty name", Some(""), "No sound from the microphone"),
        ];
        for (name, device, want) in cases {
            assert_eq!(silent_mic_title(device), want, "{name}");
        }
    }

    #[test]
    fn waveform_history_keeps_the_last_60_levels_ovl_8() {
        struct Case {
            name: &'static str,
            pushed: usize,
            want: Vec<f32>,
        }
        let cases = [
            Case {
                name: "empty",
                pushed: 0,
                want: vec![],
            },
            Case {
                name: "partly filled keeps order",
                pushed: 3,
                want: vec![0.0, 1.0, 2.0],
            },
            Case {
                name: "overflow drops the oldest",
                pushed: 70,
                want: (10..70).map(|i| i as f32).collect(),
            },
        ];
        for tc in cases {
            let mut history = WaveformHistory::new();
            for i in 0..tc.pushed {
                history.push(i as f32);
            }
            assert_eq!(history.levels(), tc.want, "{}", tc.name);
        }

        let mut history = WaveformHistory::new();
        history.push(0.5);
        history.reset();
        assert!(history.levels().is_empty(), "reset clears");
    }

    #[test]
    fn display_levels_left_pads_with_silence_ovl_8() {
        struct Case {
            name: &'static str,
            levels: Vec<f32>,
            width: f64,
            want: Vec<f32>,
        }
        let cases = [
            Case {
                name: "no levels idles as a row of dots",
                levels: vec![],
                width: 15.0,
                want: vec![0.0, 0.0, 0.0],
            },
            Case {
                name: "fewer levels than bars pad on the left",
                levels: vec![0.1, 0.2],
                width: 21.0,
                want: vec![0.0, 0.0, 0.1, 0.2],
            },
            Case {
                name: "more levels than bars keep the newest",
                levels: vec![0.1, 0.2, 0.3],
                width: 9.0,
                want: vec![0.2, 0.3],
            },
            Case {
                name: "a too-narrow meter still shows one bar",
                levels: vec![0.1, 0.2],
                width: 0.0,
                want: vec![0.2],
            },
        ];
        for tc in cases {
            assert_eq!(display_levels(&tc.levels, tc.width), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn bar_count_ovl_8() {
        let cases = [
            ("zero width", 0.0, 1),
            ("one bar", 3.0, 1),
            ("bar plus gap is not two", 8.9, 1),
            ("two bars", 9.0, 2),
            ("overlay width", 400.0, 67),
            ("negative width", -10.0, 1),
            ("not a number", f64::NAN, 1),
        ];
        for (name, width, want) in cases {
            assert_eq!(bar_count(width), want, "{name}");
        }
    }

    #[test]
    fn bar_height_ovl_8() {
        let cases = [
            ("silence is the minimum", 0.0, 24.0, 3.0),
            ("quiet audio is scaled up tenfold", 0.05, 24.0, 13.5),
            ("loud audio fills the height", 0.1, 24.0, 24.0),
            ("louder than full is clamped", 0.8, 24.0, 24.0),
            ("negative level is clamped", -0.5, 24.0, 3.0),
            ("less room than the minimum", 0.1, 2.0, 3.0),
        ];
        for (name, level, available, want) in cases {
            let got = bar_height(level, available);
            assert!((got - want).abs() < 1e-6, "{name}: got {got}");
        }
    }

    #[test]
    fn bar_heights_ovl_8() {
        let cases = [
            (
                "empty history is all minimum bars",
                vec![],
                15.0,
                24.0,
                vec![3.0, 3.0, 3.0],
            ),
            (
                "newest on the right, padded with silence",
                vec![0.1, 0.05],
                15.0,
                24.0,
                vec![3.0, 24.0, 13.5],
            ),
            (
                "only the most recent that fit",
                vec![0.1, 0.0, 0.05],
                9.0,
                24.0,
                vec![3.0, 13.5],
            ),
        ];
        for (name, levels, width, height, want) in cases {
            let got = bar_heights(&levels, width, height);
            assert_eq!(got.len(), want.len(), "{name}: got {got:?}");
            for (got, want) in got.iter().zip(&want) {
                assert!((got - want).abs() < 1e-6, "{name}: got {got}, want {want}");
            }
        }
    }
}
