//! WAV encoding of kept recordings (HIS-5, HIS-11).

use std::fmt;

use super::SAMPLE_RATE;

const HEADER_LEN: usize = 44;
const BYTES_PER_SAMPLE: usize = 2;

/// The recording does not fit in a WAV file (HIS-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavTooLarge;

impl fmt::Display for WavTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Audio recording is too large to save as WAV")
    }
}

impl std::error::Error for WavTooLarge {}

/// Encodes 16 kHz mono samples as a 16-bit PCM WAV file (HIS-5). NaN
/// samples write as 0; values clamp to ±1, so -1 and below write as -32768
/// and 1 and above as 32767.
pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, WavTooLarge> {
    let data_len = data_len(samples.len()).ok_or(WavTooLarge)?;
    let channels: u16 = 1;
    let block_align = channels * BYTES_PER_SAMPLE as u16;

    let mut wav = Vec::with_capacity(HEADER_LEN + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * u32::from(block_align)).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        wav.extend_from_slice(&pcm16(sample).to_le_bytes());
    }
    Ok(wav)
}

// the RIFF size field (36 + data) must fit in a u32.
fn data_len(sample_count: usize) -> Option<u32> {
    let len = sample_count.checked_mul(BYTES_PER_SAMPLE)?;
    let len = u32::try_from(len).ok()?;
    match len <= u32::MAX - 36 {
        true => Some(len),
        false => None,
    }
}

fn pcm16(sample: f32) -> i16 {
    if sample.is_nan() {
        return 0;
    }
    if sample <= -1.0 {
        return i16::MIN;
    }
    if sample >= 1.0 {
        return i16::MAX;
    }
    // |sample| < 1, so the product is within i16 range.
    (sample * f32::from(i16::MAX)).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm_samples(wav: &[u8]) -> Vec<i16> {
        wav[HEADER_LEN..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect()
    }

    #[test]
    fn wav_header_his_5() {
        let wav = encode_wav(&[0.0, 0.5, -1.0, 1.0]).expect("encodes");
        assert_eq!(wav.len(), 44 + 4 * 2, "length");
        struct Case {
            name: &'static str,
            range: std::ops::Range<usize>,
            want: Vec<u8>,
        }
        let cases = [
            Case {
                name: "RIFF tag",
                range: 0..4,
                want: b"RIFF".to_vec(),
            },
            Case {
                name: "RIFF size",
                range: 4..8,
                want: (36u32 + 8).to_le_bytes().to_vec(),
            },
            Case {
                name: "WAVE tag",
                range: 8..12,
                want: b"WAVE".to_vec(),
            },
            Case {
                name: "fmt tag",
                range: 12..16,
                want: b"fmt ".to_vec(),
            },
            Case {
                name: "fmt chunk size",
                range: 16..20,
                want: 16u32.to_le_bytes().to_vec(),
            },
            Case {
                name: "PCM format",
                range: 20..22,
                want: 1u16.to_le_bytes().to_vec(),
            },
            Case {
                name: "mono",
                range: 22..24,
                want: 1u16.to_le_bytes().to_vec(),
            },
            Case {
                name: "16 kHz",
                range: 24..28,
                want: 16_000u32.to_le_bytes().to_vec(),
            },
            Case {
                name: "byte rate",
                range: 28..32,
                want: 32_000u32.to_le_bytes().to_vec(),
            },
            Case {
                name: "block align",
                range: 32..34,
                want: 2u16.to_le_bytes().to_vec(),
            },
            Case {
                name: "bits per sample",
                range: 34..36,
                want: 16u16.to_le_bytes().to_vec(),
            },
            Case {
                name: "data tag",
                range: 36..40,
                want: b"data".to_vec(),
            },
            Case {
                name: "data size",
                range: 40..44,
                want: 8u32.to_le_bytes().to_vec(),
            },
        ];
        for tc in cases {
            assert_eq!(wav[tc.range].to_vec(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn wav_samples_his_5() {
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            want: Vec<i16>,
        }
        let cases = [
            Case {
                name: "scales and rounds",
                samples: vec![0.0, 0.5, -0.5, 0.25],
                want: vec![0, 16_384, -16_384, 8_192],
            },
            Case {
                name: "clamps at and beyond ±1",
                samples: vec![1.0, -1.0, 1.5, -2.0],
                want: vec![i16::MAX, i16::MIN, i16::MAX, i16::MIN],
            },
            Case {
                name: "non-finite samples do not trap",
                samples: vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5],
                want: vec![0, i16::MAX, i16::MIN, 16_384],
            },
            Case {
                name: "empty recording",
                samples: vec![],
                want: vec![],
            },
        ];
        for tc in cases {
            let wav = encode_wav(&tc.samples).expect("encodes");
            assert_eq!(wav.len(), 44 + 2 * tc.samples.len(), "{}", tc.name);
            assert_eq!(pcm_samples(&wav), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn wav_size_limit_his_11() {
        let max_samples = (u32::MAX as usize - 36) / 2;
        struct Case {
            name: &'static str,
            sample_count: usize,
            want: Option<u32>,
        }
        let cases = [
            Case {
                name: "fifteen minutes fits",
                sample_count: 900 * 16_000,
                want: Some(900 * 16_000 * 2),
            },
            Case {
                name: "largest that fits",
                sample_count: max_samples,
                want: Some((max_samples * 2) as u32),
            },
            Case {
                name: "one more overflows the RIFF size",
                sample_count: max_samples + 1,
                want: None,
            },
            Case {
                name: "byte count overflows usize",
                sample_count: usize::MAX,
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(data_len(tc.sample_count), tc.want, "{}", tc.name);
        }
        assert_eq!(
            WavTooLarge.to_string(),
            "Audio recording is too large to save as WAV",
            "error text"
        );
    }
}
