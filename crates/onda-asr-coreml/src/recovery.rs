//! Empty-decode recovery for the v3 family (FluidAudio #909), ported from
//! `AsrManager+Pipeline`.
//!
//! Parakeet v3 has input cuts on which the joint never beats blank, and the
//! output is on a knife edge with respect to the length inputs. When a
//! window that carries speech decodes to nothing, it is retried with
//! perturbed length inputs and the first credible result wins. A window
//! that decoded fine is never touched.

use crate::chunk::FRAME_SAMPLES;
use crate::tdt::Hypothesis;

/// How the model is told where the audio ends inside the padded input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LengthPolicy {
    /// Declare the zero padding valid to the encoder (`mel_length` = all
    /// mel frames).
    pub encoder_full: bool,
    /// Declare the zero padding valid to the preprocessor.
    pub preprocessor_full: bool,
    /// Declare the audio 0.2 s shorter and silence the trimmed tail.
    pub trimmed_tail: bool,
}

const ENCODER_FULL: LengthPolicy = LengthPolicy {
    encoder_full: true,
    preprocessor_full: false,
    trimmed_tail: false,
};
const PREPROCESSOR_FULL: LengthPolicy = LengthPolicy {
    encoder_full: false,
    preprocessor_full: true,
    trimmed_tail: false,
};
const TRIMMED_TAIL: LengthPolicy = LengthPolicy {
    encoder_full: false,
    preprocessor_full: false,
    trimmed_tail: true,
};

/// The retry ladder, in order. Trimming costs the last 0.2 s, so it comes
/// last, alone and combined.
pub(crate) const RECOVERY_POLICIES: [LengthPolicy; 5] = [
    ENCODER_FULL,
    PREPROCESSOR_FULL,
    TRIMMED_TAIL,
    LengthPolicy {
        encoder_full: true,
        ..TRIMMED_TAIL
    },
    LengthPolicy {
        preprocessor_full: true,
        ..TRIMMED_TAIL
    },
];

/// Audio needed behind an empty decode before it is retried: 2 s.
const MINIMUM_SAMPLES: usize = 32_000;
/// RMS below which a window counts as silence (about -50 dBFS).
const MINIMUM_RMS: f64 = 0.003;
/// Audio `.trimmedTail` declares absent: 0.2 s.
const TRIMMED_SAMPLES: usize = 3_200;
/// Mean token confidence and token count a recovery needs; perturbations
/// can coax hallucinations out of music or noise.
const MINIMUM_CONFIDENCE: f32 = 0.7;
const MINIMUM_TOKENS: usize = 2;

/// Whether an empty decode of `padded[..audio_length]` deserves a retry:
/// enough audio, and not silence.
pub(crate) fn should_recover(padded: &[f32], audio_length: usize) -> bool {
    let length = audio_length.min(padded.len());
    if length < MINIMUM_SAMPLES {
        return false;
    }
    let energy: f64 = padded[..length].iter().map(|&s| f64::from(s * s)).sum();
    (energy / length as f64).sqrt() >= MINIMUM_RMS
}

/// Whether a retry's hypothesis is credible enough to replace the empty
/// decode.
pub(crate) fn is_credible(hypothesis: &Hypothesis) -> bool {
    let tokens = &hypothesis.tokens;
    if tokens.len() < MINIMUM_TOKENS {
        return false;
    }
    let mean = tokens.iter().map(|t| t.confidence).sum::<f32>() / tokens.len() as f32;
    mean >= MINIMUM_CONFIDENCE
}

/// The declared length under `.trimmedTail`: 0.2 s shorter, at least 2 s,
/// on a frame boundary.
pub(crate) fn trimmed_length(audio_length: usize) -> usize {
    audio_length
        .saturating_sub(TRIMMED_SAMPLES)
        .max(MINIMUM_SAMPLES)
        / FRAME_SAMPLES
        * FRAME_SAMPLES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tdt::Token;

    #[test]
    fn retries_only_audible_windows_of_two_seconds() {
        let cases = [
            ("speech", vec![0.1f32; 40_000], 40_000, true),
            ("too short", vec![0.1f32; 40_000], 31_999, false),
            ("silence", vec![0.001f32; 40_000], 40_000, false),
            (
                "length beyond the buffer",
                vec![0.1f32; 40_000],
                90_000,
                true,
            ),
        ];
        for (name, padded, length, want) in cases {
            assert_eq!(should_recover(&padded, length), want, "{name}");
        }
    }

    #[test]
    fn credibility_needs_tokens_and_confidence() {
        let token = |confidence: f32| Token {
            id: 1,
            frame: 0,
            confidence,
            duration: 1,
        };
        let cases = [
            ("two confident tokens", vec![token(0.9), token(0.8)], true),
            ("one token", vec![token(0.99)], false),
            (
                "hallucination on noise",
                vec![token(0.6), token(0.5)],
                false,
            ),
        ];
        for (name, tokens, want) in cases {
            let hypothesis = Hypothesis {
                tokens,
                suppressed: 0,
            };
            assert_eq!(is_credible(&hypothesis), want, "{name}");
        }
    }

    #[test]
    fn trimmed_length_is_frame_aligned() {
        let cases = [(240_000, 236_800), (35_000, 32_000), (10_000, 32_000)];
        for (length, want) in cases {
            assert_eq!(trimmed_length(length), want, "{length}");
        }
    }
}
