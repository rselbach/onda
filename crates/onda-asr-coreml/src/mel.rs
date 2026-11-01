//! Log-mel features for Parakeet Unified, ported from FluidAudio's
//! `UnifiedMelExtractor` and `AudioMelSpectrogram`: NeMo's
//! `AudioToMelSpectrogramPreprocessor` as configured for
//! `nvidia/parakeet-unified-en-0.6b` (512-point FFT, 25 ms symmetric Hann
//! window, 10 ms hop, 128 Slaney mel bins, 0.97 pre-emphasis, `log(x +
//! 2^-24)`, centered zero padding) with `per_feature` normalization. The
//! conversion ships no CoreML preprocessor.

use std::f64::consts::PI;

/// Mel bins.
pub(crate) const N_MELS: usize = 128;
const N_FFT: usize = 512;
const BINS: usize = N_FFT / 2 + 1;
const HOP: usize = 160;
const WIN: usize = 400;
const PREEMPHASIS: f32 = 0.97;
const LOG_GUARD: f32 = 1.0 / 16_777_216.0;
const SAMPLE_RATE: f64 = 16_000.0;

/// Mel features for fixed-size encoder windows.
pub(crate) struct MelExtractor {
    window_samples: usize,
    total_frames: usize,
    hann: Vec<f32>,
    /// Per mel bin: first FFT bin with a nonzero weight, and the weights.
    filters: Vec<(usize, Vec<f32>)>,
    fft: Fft,
}

impl MelExtractor {
    /// An extractor for windows of `window_samples` samples.
    pub(crate) fn new(window_samples: usize) -> MelExtractor {
        let hann = (0..WIN)
            .map(|i| (0.5 * (1.0 - (2.0 * PI * i as f64 / (WIN - 1) as f64).cos())) as f32)
            .collect();
        MelExtractor {
            window_samples,
            total_frames: window_samples / HOP + 1,
            hann,
            filters: slaney_filters(),
            fft: Fft::new(),
        }
    }

    /// Mel frames per window (`window_samples / hop + 1`), the encoder's
    /// fixed input length.
    pub(crate) fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Features of a window whose first `audio.len()` samples are real and
    /// the rest zeros: `[N_MELS, total_frames]` row-major, and the valid
    /// frame count (`len / hop`, NeMo `get_seq_len`; the tensor's extra
    /// frame is padding). Frames past the valid count are zero.
    pub(crate) fn features(&self, audio: &[f32]) -> (Vec<f32>, usize) {
        let mut mel = vec![0.0f32; N_MELS * self.total_frames];
        let audio = &audio[..audio.len().min(self.window_samples)];
        let valid = (audio.len() / HOP).min(self.total_frames);
        if valid == 0 {
            return (mel, 0);
        }

        // pre-emphasis over the zero-padded window, centered by n_fft / 2
        let pad = N_FFT / 2;
        let mut padded = vec![0.0f32; self.window_samples + 2 * pad];
        let mut previous = 0.0;
        for (n, slot) in padded[pad..pad + self.window_samples]
            .iter_mut()
            .enumerate()
        {
            let sample = audio.get(n).copied().unwrap_or(0.0);
            *slot = sample - PREEMPHASIS * previous;
            previous = sample;
        }

        // only valid frames matter: normalization zeroes the rest
        let offset = (N_FFT - WIN) / 2;
        let mut frame = [0.0f32; N_FFT];
        let mut power = [0.0f32; BINS];
        for t in 0..valid {
            frame.fill(0.0);
            let start = t * HOP + offset;
            let available = WIN.min(padded.len().saturating_sub(start));
            for i in 0..available {
                frame[offset + i] = padded[start + i] * self.hann[i];
            }
            self.fft.power(&frame, &mut power);
            for (m, (first, weights)) in self.filters.iter().enumerate() {
                let energy: f32 = weights
                    .iter()
                    .zip(&power[*first..])
                    .map(|(w, p)| w * p)
                    .sum();
                mel[m * self.total_frames + t] = (energy + LOG_GUARD).ln();
            }
        }
        normalize_per_feature(&mut mel, self.total_frames, valid);
        (mel, valid)
    }
}

/// NeMo `normalize_batch(per_feature)`: each mel bin minus its mean over
/// the valid frames, over the unbiased standard deviation plus 1e-5.
fn normalize_per_feature(mel: &mut [f32], frames: usize, valid: usize) {
    let denominator = valid.saturating_sub(1).max(1) as f32;
    for row in mel.chunks_mut(frames) {
        let values = &mut row[..valid];
        let mean = values.iter().sum::<f32>() / valid as f32;
        let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / denominator;
        let std = variance.sqrt() + 1e-5;
        for value in values {
            *value = (*value - mean) / std;
        }
    }
}

/// Slaney-scale triangular filters with Slaney area normalization
/// (librosa's default), 0 Hz to Nyquist.
fn slaney_filters() -> Vec<(usize, Vec<f32>)> {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1_000.0;
    let min_log_mel = min_log_hz / f_sp;
    let log_step = 6.4f64.ln() / 27.0;
    let hz_to_mel = |hz: f64| {
        if hz >= min_log_hz {
            min_log_mel + (hz / min_log_hz).ln() / log_step
        } else {
            hz / f_sp
        }
    };
    let mel_to_hz = |mel: f64| {
        if mel >= min_log_mel {
            min_log_hz * (log_step * (mel - min_log_mel)).exp()
        } else {
            f_sp * mel
        }
    };
    let top = hz_to_mel(SAMPLE_RATE / 2.0);
    let points: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(i as f64 * top / (N_MELS + 1) as f64))
        .collect();
    (0..N_MELS)
        .map(|m| {
            let (left, center, right) = (points[m], points[m + 1], points[m + 2]);
            let norm = 2.0 / (right - left);
            let weights: Vec<f32> = (0..BINS)
                .map(|bin| {
                    let freq = bin as f64 * SAMPLE_RATE / N_FFT as f64;
                    let weight = if freq >= left && freq < center {
                        norm * (freq - left) / (center - left)
                    } else if freq >= center && freq <= right {
                        norm * (right - freq) / (right - center)
                    } else {
                        0.0
                    };
                    weight as f32
                })
                .collect();
            let first = weights.iter().position(|&w| w != 0.0).unwrap_or(0);
            let last = weights.iter().rposition(|&w| w != 0.0).unwrap_or(first);
            (first, weights[first..=last].to_vec())
        })
        .collect()
}

/// Iterative radix-2 FFT of `N_FFT` real samples.
struct Fft {
    /// `e^{-2πik/N}` for k in 0..N/2.
    twiddles: Vec<(f32, f32)>,
    bit_reversed: Vec<usize>,
}

impl Fft {
    fn new() -> Fft {
        let bits = N_FFT.trailing_zeros();
        Fft {
            twiddles: (0..N_FFT / 2)
                .map(|k| {
                    let angle = -2.0 * PI * k as f64 / N_FFT as f64;
                    (angle.cos() as f32, angle.sin() as f32)
                })
                .collect(),
            bit_reversed: (0..N_FFT)
                .map(|i| i.reverse_bits() >> (usize::BITS - bits))
                .collect(),
        }
    }

    /// `|X[k]|^2` for k in `0..=N/2`.
    fn power(&self, input: &[f32; N_FFT], power: &mut [f32; BINS]) {
        let mut re = [0.0f32; N_FFT];
        let mut im = [0.0f32; N_FFT];
        for (i, &j) in self.bit_reversed.iter().enumerate() {
            re[j] = input[i];
        }
        let mut size = 2;
        while size <= N_FFT {
            let half = size / 2;
            let step = N_FFT / size;
            for start in (0..N_FFT).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = self.twiddles[k * step];
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
        for (k, slot) in power.iter_mut().enumerate() {
            *slot = re[k] * re[k] + im[k] * im[k];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference signal: two tones plus LCG noise, 12,345 real samples
    /// in a 1 s window. Phases are reduced in integers so Swift and Rust
    /// feed `sin` the same small arguments.
    fn reference_signal() -> Vec<f32> {
        let tau = 2.0 * std::f32::consts::PI;
        let mut state: u32 = 12_345;
        (0..12_345usize)
            .map(|n| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / 16_777_216.0 - 0.5;
                let first = ((440 * n) % 16_000) as f32 / 16_000.0;
                let second = ((12_345 * n) % 160_000) as f32 / 160_000.0;
                0.3 * (tau * first).sin() + 0.2 * (tau * second + 0.5).sin() + 0.05 * noise
            })
            .collect()
    }

    /// Values computed by FluidAudio's `AudioMelSpectrogram` and
    /// `UnifiedMelExtractor` normalization on the same signal.
    #[test]
    fn matches_fluidaudio_reference() {
        let want: [(usize, usize, f32); 30] = [
            (0, 0, 4.2858276),
            (0, 1, 0.4483178),
            (0, 38, 0.48540154),
            (0, 76, 0.024059853),
            (0, 77, 0.0),
            (7, 0, 3.9461634),
            (7, 1, 0.9452959),
            (7, 38, -1.1527057),
            (7, 76, 1.0978596),
            (7, 77, 0.0),
            (31, 0, 1.4047583),
            (31, 1, -0.07855411),
            (31, 38, 0.9593242),
            (31, 76, -0.055000775),
            (31, 77, 0.0),
            (64, 0, 2.700851),
            (64, 1, 0.12619025),
            (64, 38, 0.3155013),
            (64, 76, -0.7440492),
            (64, 77, 0.0),
            (100, 0, -2.1157646),
            (100, 1, 1.3925421),
            (100, 38, 1.4114907),
            (100, 76, -0.60061467),
            (100, 77, 0.0),
            (127, 0, -3.101492),
            (127, 1, 0.32073006),
            (127, 38, -0.71184695),
            (127, 76, -1.5298518),
            (127, 77, 0.0),
        ];
        let extractor = MelExtractor::new(16_000);
        assert_eq!(extractor.total_frames(), 101);
        let (mel, valid) = extractor.features(&reference_signal());
        assert_eq!(valid, 77);
        for (m, t, value) in want {
            let got = mel[m * 101 + t];
            assert!(
                (got - value).abs() < 1e-3,
                "mel[{m}, {t}] = {got}, want {value}"
            );
        }
        let abs_sum: f64 = mel.iter().map(|v| f64::from(v.abs())).sum();
        assert!((abs_sum - 7_146.521_7).abs() < 0.5, "{abs_sum}");
    }

    #[test]
    fn short_and_empty_windows() {
        let extractor = MelExtractor::new(107_520);
        assert_eq!(extractor.total_frames(), 673);
        let cases = [(0, 0), (159, 0), (160, 1), (107_520, 672), (200_000, 672)];
        for (len, want) in cases {
            let (mel, valid) = extractor.features(&vec![0.1; len]);
            assert_eq!(valid, want, "{len}");
            assert_eq!(mel.len(), N_MELS * 673, "{len}");
            assert!(mel.iter().all(|v| v.is_finite()), "{len}");
        }
    }

    #[test]
    fn fft_matches_a_direct_dft() {
        let mut input = [0.0f32; N_FFT];
        for (i, slot) in input.iter_mut().enumerate() {
            *slot = ((i * 37) % 11) as f32 - 5.0;
        }
        let mut power = [0.0f32; BINS];
        Fft::new().power(&input, &mut power);
        for k in [0, 1, 17, 128, 256] {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (n, &x) in input.iter().enumerate() {
                let angle = -2.0 * PI * (k * n) as f64 / N_FFT as f64;
                re += f64::from(x) * angle.cos();
                im += f64::from(x) * angle.sin();
            }
            let want = re * re + im * im;
            let got = f64::from(power[k]);
            assert!(
                (got - want).abs() <= 1e-3 * want.max(1.0),
                "bin {k}: {got} vs {want}"
            );
        }
    }
}
