//! Greedy Token-and-Duration Transducer decoding, ported from FluidAudio's
//! `TdtDecoderV3.decodeWithTimings` (the path `TdtDecoderV2` delegates to)
//! for a fresh decoder state, with the v3 script filter (ASR-12) and the
//! emission cutoff of end-aligned final windows.

use onda_asr::AsrError;

use crate::language::TokenFilter;

/// FluidAudio `TdtConfig.default`, with the model's blank.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TdtConfig {
    /// Blank token id; also primes the prediction network.
    pub blank: usize,
    /// Encoder frames to advance for each duration bin.
    pub duration_bins: Vec<usize>,
    /// Tokens emitted at one frame before the decoder is forced ahead.
    pub max_symbols_per_step: usize,
    /// Runaway guard: decoding stops after this many tokens in a window.
    pub max_tokens_per_window: usize,
    /// Consecutive blanks that end the last window's flush.
    pub consecutive_blank_limit: usize,
}

impl TdtConfig {
    /// Parakeet TDT 0.6B v2.
    pub(crate) fn v2() -> TdtConfig {
        TdtConfig::with_blank(1024)
    }

    /// Parakeet v3 and its derivatives (Ultra).
    pub(crate) fn v3() -> TdtConfig {
        TdtConfig::with_blank(8192)
    }

    /// Parakeet 0.6B Japanese.
    pub(crate) fn japanese() -> TdtConfig {
        TdtConfig::with_blank(3072)
    }

    fn with_blank(blank: usize) -> TdtConfig {
        TdtConfig {
            blank,
            duration_bins: vec![0, 1, 2, 3, 4],
            max_symbols_per_step: 10,
            max_tokens_per_window: 150,
            consecutive_blank_limit: 5,
        }
    }
}

/// The joint's top-K candidates (`JointDecisionv3`): parallel ids and raw
/// logits, not necessarily sorted.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TopK {
    pub ids: Vec<usize>,
    pub logits: Vec<f32>,
}

/// One joint network step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JointDecision {
    pub token: usize,
    pub probability: f32,
    pub duration_bin: usize,
    /// Present only when the transducer was asked for candidates.
    pub top_k: Option<TopK>,
}

/// The prediction and joint networks over one window's encoder output.
pub(crate) trait Transducer {
    /// Feeds `token` to the prediction network; later joint steps condition
    /// on it.
    fn predict(&mut self, token: usize) -> Result<(), AsrError>;
    /// The joint decision for encoder `frame` and the current prediction.
    fn joint(&mut self, frame: usize) -> Result<JointDecision, AsrError>;
}

/// An emitted token with its global encoder frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Token {
    pub id: usize,
    pub frame: i64,
    pub confidence: f32,
    pub duration: i64,
}

/// Where in the window to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodeSpan {
    /// Valid encoder frames.
    pub encoder_frames: usize,
    /// Frames holding real audio; decoding stops at the smaller of the two.
    pub audio_frames: usize,
    /// First frame to decode (skips prepended context).
    pub start_frame: usize,
    /// Global frame number of local frame 0.
    pub frame_offset: i64,
    /// The recording ends in this window: flush pending tokens at the end.
    pub is_last: bool,
    /// Tokens before this global frame are decoded but not emitted: the
    /// backfilled prefix of an end-aligned final window.
    pub emit_after_frame: Option<i64>,
}

/// One window's decode.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Hypothesis {
    pub tokens: Vec<Token>,
    /// Tokens decoded but held back by the emission cutoff.
    pub suppressed: usize,
}

impl Hypothesis {
    /// Nothing decoded at all, not even suppressed tokens (FluidAudio
    /// `isWholeWindowBlank`).
    pub(crate) fn is_blank(&self) -> bool {
        self.tokens.is_empty() && self.suppressed == 0
    }
}

/// Decodes one window greedily. `filter`, when present, applies the script
/// hint to each step of the main and blank loops (not the final flush,
/// as in FluidAudio).
pub(crate) fn decode(
    transducer: &mut impl Transducer,
    span: DecodeSpan,
    config: &TdtConfig,
    filter: Option<&TokenFilter<'_>>,
) -> Result<Hypothesis, AsrError> {
    let mut hypothesis = Hypothesis::default();
    if span.encoder_frames <= 1 {
        return Ok(hypothesis);
    }
    let effective = span.encoder_frames.min(span.audio_frames);
    let mut time = span.start_frame;
    if time >= effective {
        return Ok(hypothesis);
    }
    let last_frame = effective - 1;
    let mut safe_time = time.min(last_frame);
    let duration_of = |bin: usize| {
        config
            .duration_bins
            .get(bin)
            .copied()
            .ok_or_else(|| AsrError::TranscriptionFailed {
                detail: format!("duration bin {bin} out of range"),
            })
    };
    let emit =
        |hypothesis: &mut Hypothesis, id: usize, frame: usize, score: f32, duration: usize| {
            let frame = frame as i64 + span.frame_offset;
            if span.emit_after_frame.is_some_and(|cutoff| frame < cutoff) {
                hypothesis.suppressed += 1;
                return;
            }
            hypothesis.tokens.push(Token {
                id,
                frame,
                confidence: score,
                duration: duration as i64,
            });
        };
    // the joint's choice after the script hint
    let choose = |decision: &JointDecision| {
        let mut label = decision.token;
        let mut score = clamp_probability(decision.probability);
        if let Some(filter) = filter {
            filter.apply(&mut label, &mut score, decision.top_k.as_ref());
        }
        (label, score)
    };

    // the blank primes the prediction network
    transducer.predict(config.blank)?;

    let mut active = true;
    let mut last_emission: Option<usize> = None;
    let mut emissions_here = 0;
    let mut emitted = 0;
    while active {
        let decision = transducer.joint(safe_time)?;
        let (mut label, mut score) = choose(&decision);
        let mut duration = duration_of(decision.duration_bin)?;
        let mut blank = label == config.blank;
        if !blank && duration == 0 && last_emission == Some(time) && emissions_here >= 1 {
            duration = 1;
        }
        if blank && duration == 0 {
            duration = 1;
        }
        let mut label_frame = time;
        time += duration;
        safe_time = time.min(last_frame);
        active = time < effective;

        // blanks skip ahead without touching the prediction network
        while active && blank {
            label_frame = time;
            let decision = transducer.joint(safe_time)?;
            (label, score) = choose(&decision);
            duration = duration_of(decision.duration_bin)?;
            blank = label == config.blank;
            if blank && duration == 0 {
                duration = 1;
            }
            time += duration;
            safe_time = time.min(last_frame);
            active = time < effective;
        }

        // FluidAudio drops a token whose duration runs past the window; the
        // overlap or the last-window flush picks it up
        if active && !blank {
            emitted += 1;
            if emitted > config.max_tokens_per_window {
                break;
            }
            emit(&mut hypothesis, label, label_frame, score, duration);
            transducer.predict(label)?;
            if last_emission == Some(label_frame) {
                emissions_here += 1;
            } else {
                last_emission = Some(label_frame);
                emissions_here = 1;
            }
            if emissions_here >= config.max_symbols_per_step {
                time = (time + 1).min(last_frame);
                safe_time = time.min(last_frame);
                emissions_here = 0;
                last_emission = None;
            }
        }
        active = time < effective;
    }

    if span.is_last {
        let frame_limit = span.encoder_frames - 1;
        let mut steps = 0;
        let mut blanks = 0;
        let mut flush_time = time;
        while steps < config.max_symbols_per_step && blanks < config.consecutive_blank_limit {
            let candidates = [
                flush_time.min(frame_limit),
                last_frame.min(frame_limit),
                effective.saturating_sub(2).min(frame_limit),
            ];
            let decision = transducer.joint(candidates[steps % candidates.len()])?;
            let duration = duration_of(decision.duration_bin)?;
            if decision.token == config.blank {
                blanks += 1;
            } else {
                blanks = 0;
                let score = clamp_probability(decision.probability);
                emit(
                    &mut hypothesis,
                    decision.token,
                    flush_time.min(last_frame),
                    score,
                    duration,
                );
                transducer.predict(decision.token)?;
            }
            flush_time = (flush_time + duration.max(1)).min(effective);
            steps += 1;
        }
    }
    Ok(hypothesis)
}

fn clamp_probability(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;

    use super::*;

    const BLANK: usize = 1024;

    /// Replays joint decisions in order and records every call.
    pub(crate) struct Scripted {
        decisions: VecDeque<(usize, usize)>,
        pub calls: Vec<String>,
    }

    impl Scripted {
        /// `(token, duration_bin)` per joint call; blanks once exhausted.
        pub(crate) fn new(decisions: &[(usize, usize)]) -> Scripted {
            Scripted {
                decisions: decisions.iter().copied().collect(),
                calls: Vec::new(),
            }
        }
    }

    impl Transducer for Scripted {
        fn predict(&mut self, token: usize) -> Result<(), AsrError> {
            self.calls.push(format!("predict {token}"));
            Ok(())
        }

        fn joint(&mut self, frame: usize) -> Result<JointDecision, AsrError> {
            self.calls.push(format!("joint {frame}"));
            let (token, duration_bin) = self.decisions.pop_front().unwrap_or((BLANK, 1));
            Ok(JointDecision {
                token,
                probability: 0.9,
                duration_bin,
                top_k: None,
            })
        }
    }

    fn span(frames: usize, is_last: bool) -> DecodeSpan {
        DecodeSpan {
            encoder_frames: frames,
            audio_frames: frames,
            start_frame: 0,
            frame_offset: 0,
            is_last,
            emit_after_frame: None,
        }
    }

    fn ids_and_frames(tokens: &[Token]) -> Vec<(usize, i64)> {
        tokens.iter().map(|t| (t.id, t.frame)).collect()
    }

    #[test]
    fn decode_follows_scripted_joint() {
        struct Case {
            name: &'static str,
            span: DecodeSpan,
            script: Vec<(usize, usize)>,
            want: Vec<(usize, i64)>,
        }
        let cases = [
            Case {
                name: "blanks skip by duration, tokens keep their frame",
                span: span(10, false),
                // f0 blank +2, f2 Troy +1, f3 blank +3, f6 Barnes +0, f6 nes +1, rest blank
                script: vec![(BLANK, 2), (1, 1), (BLANK, 3), (2, 0), (3, 1)],
                want: vec![(1, 2), (2, 6), (3, 6)],
            },
            Case {
                name: "global offset and context start",
                span: DecodeSpan {
                    start_frame: 1,
                    frame_offset: 100,
                    ..span(10, false)
                },
                script: vec![(5, 1)],
                want: vec![(5, 101)],
            },
            Case {
                name: "token whose duration overruns the window is dropped",
                span: span(4, false),
                script: vec![(BLANK, 3), (7, 2)],
                want: vec![],
            },
            Case {
                name: "decoding stops at the audio frames",
                span: DecodeSpan {
                    audio_frames: 2,
                    ..span(10, false)
                },
                script: vec![(BLANK, 1), (4, 0), (6, 1)],
                want: vec![(4, 1)],
            },
            Case {
                name: "single frame decodes nothing",
                span: span(1, true),
                script: vec![(4, 1)],
                want: vec![],
            },
            Case {
                name: "last window flush recovers the dropped token",
                span: span(4, true),
                script: vec![(BLANK, 3), (7, 2), (7, 1)],
                want: vec![(7, 3)],
            },
            Case {
                name: "zero-duration repeats are forced ahead after one",
                span: span(10, false),
                script: vec![(1, 0), (2, 0), (3, 1)],
                want: vec![(1, 0), (2, 0), (3, 1)],
            },
        ];
        for tc in cases {
            let mut transducer = Scripted::new(&tc.script);
            let tokens = decode(&mut transducer, tc.span, &TdtConfig::v2(), None)
                .expect("decode")
                .tokens;
            assert_eq!(ids_and_frames(&tokens), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn decode_primes_with_blank_and_predicts_each_token() {
        let mut transducer = Scripted::new(&[(1, 1), (BLANK, 4)]);
        decode(&mut transducer, span(3, false), &TdtConfig::v2(), None).expect("decode");
        assert_eq!(
            transducer.calls,
            ["predict 1024", "joint 0", "predict 1", "joint 1"]
        );
    }

    #[test]
    fn max_symbols_per_step_forces_progress() {
        // the same token forever with duration 0: the zero-duration guard
        // advances after one repeat, and the symbol limit skips a frame
        let script = vec![(9, 0); 40];
        let mut transducer = Scripted::new(&script);
        let config = TdtConfig {
            max_symbols_per_step: 2,
            ..TdtConfig::v2()
        };
        let tokens = decode(&mut transducer, span(3, false), &config, None)
            .expect("decode")
            .tokens;
        assert_eq!(ids_and_frames(&tokens), vec![(9, 0), (9, 0), (9, 2)]);
    }

    #[test]
    fn runaway_windows_stop_at_token_limit() {
        let script = vec![(9, 1); 40];
        let mut transducer = Scripted::new(&script);
        let config = TdtConfig {
            max_tokens_per_window: 3,
            ..TdtConfig::v2()
        };
        let tokens = decode(&mut transducer, span(30, false), &config, None)
            .expect("decode")
            .tokens;
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn emission_cutoff_suppresses_the_prefix() {
        // f0 Troy +3, f3 Barnes +3, f6 Abed +1: the cutoff at global frame
        // 104 holds back the first two but still feeds them to the decoder
        let mut transducer = Scripted::new(&[(1, 3), (2, 3), (3, 1)]);
        let span = DecodeSpan {
            frame_offset: 100,
            emit_after_frame: Some(104),
            ..span(10, false)
        };
        let hypothesis = decode(&mut transducer, span, &TdtConfig::v2(), None).expect("decode");
        assert_eq!(ids_and_frames(&hypothesis.tokens), vec![(3, 106)]);
        assert_eq!(hypothesis.suppressed, 2);
        assert!(!hypothesis.is_blank());
        assert!(transducer.calls.contains(&"predict 2".to_owned()));
    }

    #[test]
    fn bad_duration_bin_fails() {
        let mut transducer = Scripted::new(&[(1, 9)]);
        let result = decode(&mut transducer, span(3, false), &TdtConfig::v2(), None);
        assert!(matches!(result, Err(AsrError::TranscriptionFailed { .. })));
    }
}
