//! Speech gate: decides whether captured audio is worth transcribing
//! (GATE-1..GATE-6).

use crate::pre_roll::SAMPLE_RATE;

/// Title of the notice shown for [`GateVerdict::RejectWithWarning`] (GATE-5).
pub const NO_SPEECH_TITLE: &str = "No Speech Detected";

/// Message of the notice shown for [`GateVerdict::RejectWithWarning`]
/// (GATE-5).
pub const NO_SPEECH_MESSAGE: &str = "Could not detect any speech in the recording. Try speaking louder or closer to the microphone.";

/// Shortest clip worth judging: 0.25 s (GATE-2).
const MIN_SAMPLES: usize = SAMPLE_RATE / 4;
/// 20 ms frames (GATE-3).
const FRAME_SAMPLES: usize = 320;
/// A frame at or above this RMS is active (GATE-3).
const ACTIVE_FRAME_RMS: f32 = 0.006;
/// Loudest sample must reach this (GATE-3).
const MIN_PEAK: f32 = 0.015;
/// Active frames must add up to 0.08 s (GATE-3).
const MIN_ACTIVE_SAMPLES: usize = SAMPLE_RATE * 8 / 100;

/// Outcome of the speech gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    /// Run ASR on the audio.
    Accept,
    /// End the session quietly; the user plainly did not try to dictate.
    RejectSilently(SilentRejection),
    /// End the session and show [`NO_SPEECH_TITLE`] / [`NO_SPEECH_MESSAGE`]
    /// (GATE-3): long enough to count, but too quiet to hold speech.
    RejectWithWarning,
}

/// Why the gate rejected without a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilentRejection {
    /// No samples at all (GATE-1).
    Empty,
    /// Under 0.25 s of audio (GATE-2).
    TooShort,
}

/// Judges `samples` (16 kHz mono, pre-roll included) for speech.
///
/// There is deliberately no whole-clip RMS threshold (GATE-4): only the
/// peak and the total duration of active 20 ms frames count.
pub fn assess(samples: &[f32]) -> GateVerdict {
    if samples.is_empty() {
        return GateVerdict::RejectSilently(SilentRejection::Empty);
    }
    if samples.len() < MIN_SAMPLES {
        return GateVerdict::RejectSilently(SilentRejection::TooShort);
    }
    let m = measure(samples);
    if m.peak < MIN_PEAK || m.active_samples < MIN_ACTIVE_SAMPLES {
        return GateVerdict::RejectWithWarning;
    }
    GateVerdict::Accept
}

/// Judges only the first `released_sample_count` samples (GATE-6).
///
/// With a hotkey tail, pass the session's sample count at the moment the key
/// was released, so a stop cue picked up during the tail cannot pass as
/// speech. A count past the end judges the whole buffer.
pub fn assess_released(samples: &[f32], released_sample_count: usize) -> GateVerdict {
    assess(&samples[..released_sample_count.min(samples.len())])
}

#[derive(Debug)]
struct Measurements {
    peak: f32,
    active_samples: usize,
}

fn measure(samples: &[f32]) -> Measurements {
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    // a trailing partial frame is judged on its own length
    let active_samples = samples
        .chunks(FRAME_SAMPLES)
        .filter(|frame| frame_rms(frame) >= ACTIVE_FRAME_RMS)
        .map(<[f32]>::len)
        .sum();
    Measurements {
        peak,
        active_samples,
    }
}

fn frame_rms(frame: &[f32]) -> f32 {
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amplitude: f32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * 220.0 * i as f64 / SAMPLE_RATE as f64;
                amplitude * phase.sin() as f32
            })
            .collect()
    }

    fn click_in_silence() -> Vec<f32> {
        let mut samples = vec![0.0; 16_000];
        samples[8_000] = 0.5;
        samples
    }

    #[test]
    fn assess_gate_1_to_gate_4() {
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            want: GateVerdict,
        }
        let silence = vec![0.0; 29 * 8_000];
        let cases = [
            Case {
                name: "GATE-1 empty audio rejects without warning",
                samples: vec![],
                want: GateVerdict::RejectSilently(SilentRejection::Empty),
            },
            Case {
                name: "GATE-2 short audio rejects without warning",
                samples: vec![0.1; 1_000],
                want: GateVerdict::RejectSilently(SilentRejection::TooShort),
            },
            Case {
                name: "GATE-2 one sample under 0.25 s is too short",
                samples: vec![0.1; 3_999],
                want: GateVerdict::RejectSilently(SilentRejection::TooShort),
            },
            Case {
                name: "GATE-2 exactly 0.25 s of speech is judged and accepted",
                samples: vec![0.1; 4_000],
                want: GateVerdict::Accept,
            },
            Case {
                name: "GATE-3 quiet audio rejects with warning",
                samples: vec![0.001; 16_000],
                want: GateVerdict::RejectWithWarning,
            },
            Case {
                name: "GATE-3 speech-like audio is accepted",
                samples: sine(0.05, 16_000),
                want: GateVerdict::Accept,
            },
            Case {
                name: "GATE-3 lone click in silence rejects with warning",
                samples: click_in_silence(),
                want: GateVerdict::RejectWithWarning,
            },
            Case {
                name: "GATE-3 active frames but peak under 0.015 rejects",
                samples: vec![0.0149; 16_000],
                want: GateVerdict::RejectWithWarning,
            },
            Case {
                name: "GATE-3 peak exactly 0.015 is enough",
                samples: vec![0.015; 16_000],
                want: GateVerdict::Accept,
            },
            Case {
                name: "GATE-3 exactly 0.08 s of active frames is enough",
                samples: [vec![0.0; 6_400], vec![0.05; 1_280], vec![0.0; 6_400]].concat(),
                want: GateVerdict::Accept,
            },
            Case {
                name: "GATE-3 0.06 s of active frames is not enough",
                samples: [vec![0.0; 6_400], vec![0.05; 960], vec![0.0; 6_400]].concat(),
                want: GateVerdict::RejectWithWarning,
            },
            Case {
                name: "GATE-4 brief quiet speech in long silence is accepted",
                samples: [silence.clone(), sine(0.02, 16_000), silence].concat(),
                want: GateVerdict::Accept,
            },
        ];
        for tc in cases {
            assert_eq!(assess(&tc.samples), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn measure_gate_3() {
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            want_active: usize,
        }
        let cases = [
            Case {
                name: "sine is active throughout",
                samples: sine(0.05, 16_000),
                want_active: 16_000,
            },
            Case {
                name: "trailing partial frame is judged on its own RMS",
                // 20 samples at 0.01: active alone, inactive if averaged over 320
                samples: [vec![0.0; 13 * 320], vec![0.01; 20]].concat(),
                want_active: 20,
            },
            Case {
                name: "silence has no active frames",
                samples: vec![0.0; 4_000],
                want_active: 0,
            },
        ];
        for tc in cases {
            let got = measure(&tc.samples);
            assert_eq!(got.active_samples, tc.want_active, "{}", tc.name);
        }

        let got = measure(&sine(0.05, 16_000));
        assert!(got.peak > MIN_PEAK, "sine peak {}", got.peak);
    }

    #[test]
    fn assess_released_gate_6() {
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            released: usize,
            want: GateVerdict,
        }
        let quiet_then_cue = [vec![0.001; 16_000], vec![0.5; 3_200]].concat();
        let cases = [
            Case {
                name: "stop cue in the tail cannot pass as speech",
                samples: quiet_then_cue.clone(),
                released: 16_000,
                want: GateVerdict::RejectWithWarning,
            },
            Case {
                name: "the same buffer judged whole would pass",
                samples: quiet_then_cue,
                released: 19_200,
                want: GateVerdict::Accept,
            },
            Case {
                name: "release count past the end judges the whole buffer",
                samples: sine(0.05, 16_000),
                released: 20_000,
                want: GateVerdict::Accept,
            },
            Case {
                name: "release before any audio is empty",
                samples: sine(0.05, 16_000),
                released: 0,
                want: GateVerdict::RejectSilently(SilentRejection::Empty),
            },
        ];
        for tc in cases {
            assert_eq!(
                assess_released(&tc.samples, tc.released),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
