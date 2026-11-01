//! Warm-start pre-roll ring and session buffer sizing (AUD-1, AUD-6, AUD-12).

use std::collections::VecDeque;

/// Session audio sample rate in Hz: 16 kHz mono `f32` (AUD-1).
///
/// Every sample count in the domain (speech gate, preview, pre-roll) is at
/// this rate.
pub const SAMPLE_RATE: usize = 16_000;

/// Samples of pre-roll kept during warm standby: the newest 450 ms (AUD-6).
pub const PRE_ROLL_SAMPLES: usize = SAMPLE_RATE * 450 / 1_000;

/// Samples the session buffer reserves up front: one minute (AUD-12).
pub const SESSION_RESERVE_SAMPLES: usize = SAMPLE_RATE * 60;

/// The newest [`PRE_ROLL_SAMPLES`] samples heard while the input idles in
/// warm standby (AUD-6).
///
/// A warm start prepends [`PreRoll::take`] to the session so speech that began
/// as the hotkey went down keeps its first syllable. A cold start, or any
/// rebuild of the input, calls [`PreRoll::clear`] instead.
#[derive(Debug, Clone)]
pub struct PreRoll {
    samples: VecDeque<f32>,
}

impl Default for PreRoll {
    fn default() -> Self {
        Self::new()
    }
}

impl PreRoll {
    /// An empty pre-roll with room for [`PRE_ROLL_SAMPLES`].
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
        }
    }

    /// Appends `samples`, dropping the oldest so at most [`PRE_ROLL_SAMPLES`]
    /// remain.
    pub fn push(&mut self, samples: &[f32]) {
        let newest = &samples[samples.len().saturating_sub(PRE_ROLL_SAMPLES)..];
        let excess = (self.samples.len() + newest.len()).saturating_sub(PRE_ROLL_SAMPLES);
        self.samples.drain(..excess);
        self.samples.extend(newest);
    }

    /// Removes and returns the held samples, oldest first.
    pub fn take(&mut self) -> Vec<f32> {
        self.samples.drain(..).collect()
    }

    /// Drops the held samples.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Number of samples held.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples are held.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_aud_6_aud_12() {
        let cases = [
            ("AUD-6 pre-roll is 450 ms", PRE_ROLL_SAMPLES, 7_200),
            (
                "AUD-12 session reserve is one minute",
                SESSION_RESERVE_SAMPLES,
                960_000,
            ),
        ];
        for (name, got, want) in cases {
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn pre_roll_keeps_only_the_newest_samples_aud_6() {
        struct Case {
            name: &'static str,
            pushes: Vec<Vec<f32>>,
            want: Vec<f32>,
        }
        let limit = PRE_ROLL_SAMPLES;
        let cases = [
            Case {
                name: "under the limit keeps everything",
                pushes: vec![vec![1.0; limit - 10]],
                want: vec![1.0; limit - 10],
            },
            Case {
                name: "crossing the limit drops the oldest",
                pushes: vec![vec![1.0; limit - 10], vec![2.0; 30]],
                want: [vec![1.0; limit - 30], vec![2.0; 30]].concat(),
            },
            Case {
                name: "one oversized push keeps its newest samples",
                pushes: vec![
                    vec![1.0; limit - 10],
                    vec![2.0; 30],
                    [vec![4.0; 5], vec![3.0; limit]].concat(),
                ],
                want: vec![3.0; limit],
            },
            Case {
                name: "empty push changes nothing",
                pushes: vec![vec![1.0; 3], vec![]],
                want: vec![1.0; 3],
            },
        ];
        for tc in cases {
            let mut pre_roll = PreRoll::new();
            for push in &tc.pushes {
                pre_roll.push(push);
            }
            assert_eq!(pre_roll.len(), tc.want.len(), "{}", tc.name);
            assert_eq!(pre_roll.take(), tc.want, "{}", tc.name);
            assert!(pre_roll.is_empty(), "{}: take drains", tc.name);
        }
    }

    #[test]
    fn clear_drops_samples_aud_11() {
        let mut pre_roll = PreRoll::new();
        pre_roll.push(&[0.5; 100]);
        pre_roll.clear();
        assert!(pre_roll.take().is_empty());
    }
}
