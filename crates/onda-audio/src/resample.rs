//! Resampling from the device rate to 16 kHz mono (AUD-1, AUD-7).

use onda_domain::pre_roll::SAMPLE_RATE;
use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _, WindowFunction};
use tracing::warn;

use crate::AudioError;

/// Input frames per resampler chunk; rubato rounds it to suit the rate pair.
const CHUNK_FRAMES: usize = 1_024;

/// Upper bound on silent chunks pushed through at a flush. The filter delay
/// is a few milliseconds, so a handful always suffices.
const MAX_FLUSH_CHUNKS: usize = 64;

/// Streaming resampler from one device rate to [`SAMPLE_RATE`] mono.
///
/// Accepts buffers of any size, trims the filter's startup delay so output
/// lines up with input, and [`Resampler::flush`] pushes the filter tail out
/// so the output holds exactly `ceil(input × 16000 / rate)` samples.
pub(crate) struct Resampler {
    // none when the device already runs at 16 kHz
    fft: Option<Fft<f32>>,
    ratio: f64,
    pending: Vec<f32>,
    scratch: Vec<f32>,
    delay_left: usize,
    fed: usize,
    produced: usize,
}

impl Resampler {
    /// A resampler from `input_rate` Hz.
    pub(crate) fn new(input_rate: u32) -> Result<Self, AudioError> {
        let input_rate = input_rate as usize;
        let ratio = SAMPLE_RATE as f64 / input_rate as f64;
        if input_rate == SAMPLE_RATE {
            return Ok(Self {
                fft: None,
                ratio,
                pending: Vec::new(),
                scratch: Vec::new(),
                delay_left: 0,
                fed: 0,
                produced: 0,
            });
        }
        // one FFT block per chunk: the longest anti-aliasing filter rubato offers
        let fft = Fft::<f32>::new_custom(
            input_rate,
            SAMPLE_RATE,
            CHUNK_FRAMES,
            1,
            1,
            WindowFunction::BlackmanHarris2,
            FixedSync::Input,
        )
        .map_err(|err| {
            warn!(input_rate, %err, "could not create the resampler");
            AudioError::ConverterUnavailable
        })?;
        let scratch = vec![0.0; fft.output_frames_max()];
        let pending = Vec::with_capacity(fft.input_frames_max() * 2);
        let delay_left = fft.output_delay();
        Ok(Self {
            fft: Some(fft),
            ratio,
            pending,
            scratch,
            delay_left,
            fed: 0,
            produced: 0,
        })
    }

    /// Resamples `input` (mono, device rate) and appends finished samples to
    /// `out`. A partial chunk waits for the next call or [`Resampler::flush`].
    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), AudioError> {
        let Some(fft) = self.fft.as_mut() else {
            out.extend_from_slice(input);
            return Ok(());
        };
        self.fed += input.len();
        self.pending.extend_from_slice(input);
        let mut offset = 0;
        while self.pending.len() - offset >= fft.input_frames_next() {
            let chunk = run_chunk(fft, &self.pending[offset..], None, &mut self.scratch);
            let (used, made) = match chunk {
                Ok(counts) => counts,
                Err(err) => {
                    self.pending.clear();
                    return Err(err);
                }
            };
            offset += used;
            keep(
                &self.scratch[..made],
                &mut self.delay_left,
                &mut self.produced,
                out,
            );
        }
        self.pending.drain(..offset);
        Ok(())
    }

    /// Pushes the held partial chunk and the filter tail into `out`, then
    /// resets for a new stream (AUD-7).
    pub(crate) fn flush(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        let Some(fft) = self.fft.as_mut() else {
            return Ok(());
        };
        let expected = (self.fed as f64 * self.ratio).ceil() as usize;
        let before = out.len();
        let mut result = Ok(());
        for _ in 0..MAX_FLUSH_CHUNKS {
            if self.produced >= expected {
                break;
            }
            let held = self.pending.len();
            match run_chunk(fft, &self.pending, Some(held), &mut self.scratch) {
                Ok((_, made)) => {
                    self.pending.clear();
                    keep(
                        &self.scratch[..made],
                        &mut self.delay_left,
                        &mut self.produced,
                        out,
                    );
                }
                Err(err) => {
                    result = Err(err);
                    break;
                }
            }
        }
        // the last chunk overshoots with filter silence; drop it
        let excess = self.produced.saturating_sub(expected);
        out.truncate(out.len().saturating_sub(excess).max(before));
        self.reset();
        result
    }

    /// Drops held input and filter state for a new stream.
    pub(crate) fn reset(&mut self) {
        self.pending.clear();
        self.fed = 0;
        self.produced = 0;
        self.delay_left = 0;
        if let Some(fft) = self.fft.as_mut() {
            fft.reset();
            self.delay_left = fft.output_delay();
        }
    }
}

// runs one chunk; `partial` marks a final chunk shorter than the resampler
// wants, padded with silence.
fn run_chunk(
    fft: &mut Fft<f32>,
    input: &[f32],
    partial: Option<usize>,
    scratch: &mut [f32],
) -> Result<(usize, usize), AudioError> {
    let conversion = |err: &dyn std::fmt::Display| AudioError::ConversionFailed(err.to_string());
    let frames = partial.unwrap_or_else(|| fft.input_frames_next());
    let input = SequentialSlice::new(&input[..frames], 1, frames).map_err(|e| conversion(&e))?;
    let capacity = scratch.len();
    let mut output = SequentialSlice::new_mut(scratch, 1, capacity).map_err(|e| conversion(&e))?;
    let indexing = partial.map(|len| Indexing::new().partial_len(len));
    fft.process_into_buffer(&input, &mut output, indexing.as_ref())
        .map_err(|e| conversion(&e))
}

// appends `made` past whatever is left of the startup delay.
fn keep(made: &[f32], delay_left: &mut usize, produced: &mut usize, out: &mut Vec<f32>) {
    let skip = (*delay_left).min(made.len());
    *delay_left -= skip;
    *produced += made.len() - skip;
    out.extend_from_slice(&made[skip..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, frequency: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                (0.5 * (2.0 * std::f64::consts::PI * frequency * t).sin()) as f32
            })
            .collect()
    }

    // feeds `input` in uneven buffers, like a device would, then flushes.
    fn resample(rate: u32, input: &[f32]) -> Vec<f32> {
        let mut resampler = Resampler::new(rate).expect("resampler");
        let mut out = Vec::new();
        let sizes = [480, 512, 1, 333, 4_096, 7];
        let mut rest = input;
        for size in sizes.iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (head, tail) = rest.split_at((*size).min(rest.len()));
            resampler.process(head, &mut out).expect("process");
            rest = tail;
        }
        resampler.flush(&mut out).expect("flush");
        out
    }

    #[test]
    fn output_length_is_the_exact_rate_ratio_aud_1_aud_7() {
        struct Case {
            name: &'static str,
            rate: u32,
            frames: usize,
            want: usize,
        }
        let cases = [
            Case {
                name: "48 kHz second",
                rate: 48_000,
                frames: 48_000,
                want: 16_000,
            },
            Case {
                name: "44.1 kHz second",
                rate: 44_100,
                frames: 44_100,
                want: 16_000,
            },
            Case {
                name: "48 kHz rounds up rather than dropping a sample",
                rate: 48_000,
                frames: 1_024,
                want: 342,
            },
            Case {
                name: "shorter than one chunk still comes out",
                rate: 48_000,
                frames: 100,
                want: 34,
            },
            Case {
                name: "16 kHz passes through",
                rate: 16_000,
                frames: 12_345,
                want: 12_345,
            },
            Case {
                name: "8 kHz upsamples",
                rate: 8_000,
                frames: 4_000,
                want: 8_000,
            },
            Case {
                name: "96 kHz downsamples",
                rate: 96_000,
                frames: 96_000,
                want: 16_000,
            },
            Case {
                name: "nothing in, nothing out",
                rate: 48_000,
                frames: 0,
                want: 0,
            },
        ];
        for tc in cases {
            let got = resample(tc.rate, &vec![0.1; tc.frames]);
            assert_eq!(got.len(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn output_is_aligned_with_input_including_the_tail_aud_7() {
        let cases = [("48 kHz", 48_000), ("44.1 kHz", 44_100)];
        for (name, rate) in cases {
            let frames = rate as usize / 2;
            let got = resample(rate, &sine(rate, 440.0, frames));
            let want = sine(16_000, 440.0, got.len());
            // skip the edges, where the abrupt start and end ring
            let edge = 64;
            let worst = got[edge..got.len() - edge]
                .iter()
                .zip(&want[edge..want.len() - edge])
                .map(|(g, w)| (g - w).abs())
                .fold(0.0_f32, f32::max);
            // a one-sample slip at 440 Hz would err by about 0.086; the 44.1 kHz
            // pair sits a fifth of a sample off (11 µs), about 0.016
            assert!(worst < 0.03, "{name}: worst error {worst}");
            // the tail flush keeps the last few milliseconds of signal
            let tail = &got[got.len() - edge..got.len() - 8];
            let tail_peak = tail.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
            assert!(tail_peak > 0.3, "{name}: tail peak {tail_peak}");
        }
    }

    #[test]
    fn flush_resets_for_the_next_stream() {
        let mut resampler = Resampler::new(48_000).expect("resampler");
        for round in 0..2 {
            let mut out = Vec::new();
            resampler.process(&[0.2; 4_800], &mut out).expect("process");
            resampler.flush(&mut out).expect("flush");
            assert_eq!(out.len(), 1_600, "round {round}");
        }
    }

    #[test]
    fn a_zero_rate_cannot_build_a_converter() {
        assert!(matches!(
            Resampler::new(0),
            Err(AudioError::ConverterUnavailable)
        ));
    }
}
