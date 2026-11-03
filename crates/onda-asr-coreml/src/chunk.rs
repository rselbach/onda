//! Long audio: FluidAudio's `ChunkProcessor`. The encoder takes a fixed
//! 15 s window, so longer recordings decode as overlapping windows whose
//! token streams are merged at each seam, then inter-token gaps that still
//! hold speech are re-decoded (FluidAudio issue #758).
//!
//! The window layout depends on the model ([`Layout`]): v2 runs FluidAudio's
//! default path (80 ms of mel context before every window but the first,
//! regular window starts); the v3 family (Ultra) drops the mel context and
//! starts windows in pauses; v3 and Japanese end-align a short final window
//! with real audio whose tokens are suppressed (FluidAudio #747). None of
//! the models has an encoder ladder, so windows never end at pauses.

use std::collections::{HashMap, HashSet};

use onda_asr::AsrError;
use onda_domain::languages::Language;

use crate::tdt::Token;
use crate::vocab::{self, Vocabulary};

/// Model sample rate.
pub(crate) const SAMPLE_RATE: usize = 16_000;
/// Longest audio one encoder pass takes: 15 s.
pub(crate) const MAX_MODEL_SAMPLES: usize = 240_000;
/// Samples per encoder frame: 80 ms.
pub(crate) const FRAME_SAMPLES: usize = 1_280;
/// Shortest audio FluidAudio accepts: 0.3 s.
pub(crate) const MIN_SAMPLES: usize = 4_800;

const MEL_HOP_SAMPLES: usize = 160;
const MEL_CONTEXT_SAMPLES: usize = FRAME_SAMPLES;
const FRAME_SECONDS: f64 = FRAME_SAMPLES as f64 / SAMPLE_RATE as f64;
const OVERLAP_SECONDS: f64 = 2.0;
/// Frames a right-window copy must be past its decode start before its
/// timing wins at a seam.
const SEAM_HEAD_GUARD_FRAMES: i64 = 6;
const OVERLAP_FRAMES: i64 = 25;

/// How a model lays out the windows of a long recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Parakeet TDT v2: mel context, regular starts, zero-padded last window.
    V2,
    /// The v3 family: no mel context (FluidAudio #594), silence-aligned
    /// starts, end-aligned last window.
    V3,
    /// Parakeet Japanese: mel context, regular starts, end-aligned last
    /// window.
    Japanese,
}

impl Layout {
    fn mel_context(self) -> usize {
        match self {
            Layout::V3 => 0,
            Layout::V2 | Layout::Japanese => MEL_CONTEXT_SAMPLES,
        }
    }

    /// The decoder can suppress a backfilled prefix, so a short final
    /// window is filled backwards with real audio instead of zeros.
    fn end_aligned(self) -> bool {
        matches!(self, Layout::V3 | Layout::Japanese)
    }

    /// Window audio, leaving room for the mel context: 14.88 s, or 14.96 s
    /// without context.
    fn chunk_samples(self) -> usize {
        (MAX_MODEL_SAMPLES - self.mel_context() - MEL_HOP_SAMPLES) / FRAME_SAMPLES * FRAME_SAMPLES
    }

    fn overlap_samples(self) -> usize {
        let requested = OVERLAP_SECONDS as usize * SAMPLE_RATE;
        requested.min(self.chunk_samples() / 2) / FRAME_SAMPLES * FRAME_SAMPLES
    }

    fn stride_samples(self) -> usize {
        (self.chunk_samples() - self.overlap_samples()).max(FRAME_SAMPLES) / FRAME_SAMPLES
            * FRAME_SAMPLES
    }
}

/// One encoder pass: which samples to feed and how to decode them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WindowRequest {
    /// First sample fed to the model.
    pub start: usize,
    /// End of the fed samples (exclusive); the rest of the window is zeros.
    pub end: usize,
    /// Audio length declared to the preprocessor.
    pub audio_length: usize,
    /// Encoder frames of real audio, excluding the context.
    pub audio_frames: usize,
    /// Frames of prepended context the decoder skips.
    pub start_frame: usize,
    /// Global frame of local frame 0.
    pub frame_offset: i64,
    /// The window reaches the end of the recording.
    pub is_last: bool,
    /// Global frame before which decoded tokens are suppressed (the
    /// backfilled prefix of an end-aligned final window).
    pub emit_after_frame: Option<i64>,
}

/// A recording that fits one window. The declared length is rounded up to
/// a whole frame (FluidAudio `frameAlignedAudio`).
pub(crate) fn single_window(len: usize) -> WindowRequest {
    let aligned = len.div_ceil(FRAME_SAMPLES) * FRAME_SAMPLES;
    let audio_length = if aligned <= MAX_MODEL_SAMPLES {
        aligned
    } else {
        len
    };
    WindowRequest {
        start: 0,
        end: len,
        audio_length,
        audio_frames: audio_length.div_ceil(FRAME_SAMPLES),
        start_frame: 0,
        frame_offset: 0,
        is_last: true,
        emit_after_frame: None,
    }
}

/// A window's extent in global frames, for seam timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowFrames {
    decode_start: i64,
    end: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannedWindow {
    request: WindowRequest,
    frames: WindowFrames,
}

/// Lays out the overlapping windows of a recording longer than one window
/// (FluidAudio `planWindows`, without warm-up prefixes or pause cuts, which
/// need an encoder ladder).
fn plan_windows(samples: &[f32], layout: Layout) -> Vec<PlannedWindow> {
    let total = samples.len();
    let chunk = layout.chunk_samples();
    let stride = layout.stride_samples();
    let starts = match layout {
        Layout::V3 => silence_aligned_starts(samples, chunk, stride),
        Layout::V2 | Layout::Japanese => (0..total).step_by(stride).collect(),
    };
    let speech_end = if layout.end_aligned() {
        speech_end(samples)
    } else {
        total
    };
    let mut plan: Vec<PlannedWindow> = Vec::new();
    let mut start = starts.first().copied().unwrap_or(0);
    while start < total {
        let index = plan.len();
        let warmup = if layout.end_aligned() {
            last_window_warmup(start, chunk, total, speech_end)
        } else {
            0
        };
        let candidate_end = start + chunk.saturating_sub(warmup).max(FRAME_SAMPLES);
        // only a final window is backfilled, and it reaches the speech end
        // even when its visible span stops short of trailing silence.
        // (Deviation: FluidAudio drops the final tokens when a long
        // recording ends in silence; Onda finalizes the last window.)
        let is_last = candidate_end >= total || warmup > 0;
        let chunk_end = if is_last { total } else { candidate_end };
        if chunk_end <= start {
            break;
        }
        // the final window's audio stops at the last speech-bearing frame:
        // a window ending inside a dead-silence run decodes degenerately
        let end = if is_last && layout.end_aligned() {
            chunk_end.min(speech_end)
        } else {
            chunk_end
        };
        if end <= start {
            break;
        }
        let context = match (warmup, index) {
            (0, 0) => 0,
            (0, _) => layout.mel_context(),
            _ => 0,
        };
        let fed_start = start - warmup.max(context);
        let decode_start = if warmup > 0 { fed_start } else { start };
        plan.push(PlannedWindow {
            request: WindowRequest {
                start: fed_start,
                end,
                audio_length: end - fed_start,
                audio_frames: (end - fed_start - context).div_ceil(FRAME_SAMPLES),
                start_frame: context / FRAME_SAMPLES,
                frame_offset: (decode_start / FRAME_SAMPLES) as i64,
                is_last,
                emit_after_frame: (warmup > 0).then_some((start / FRAME_SAMPLES) as i64),
            },
            frames: WindowFrames {
                decode_start: (decode_start / FRAME_SAMPLES) as i64,
                end: (end / FRAME_SAMPLES) as i64,
            },
        });
        if is_last {
            break;
        }
        start = starts.get(plan.len()).copied().unwrap_or(start + stride);
    }
    plan
}

/// FluidAudio `lastChunkWarmupSamples`: a short final window starting at
/// `start` is filled backwards with real audio, frame-aligned so the
/// suppression boundary lands on a frame, ending at the speech end.
fn last_window_warmup(start: usize, chunk: usize, total: usize, speech_end: usize) -> usize {
    let is_last = start + chunk.max(FRAME_SAMPLES) >= total;
    let remaining = speech_end.min(total) as i64 - start as i64;
    if !is_last || remaining <= 0 || start == 0 {
        return 0;
    }
    let fill = (chunk as i64 - remaining).max(0) as usize / FRAME_SAMPLES * FRAME_SAMPLES;
    let available = start / FRAME_SAMPLES * FRAME_SAMPLES;
    available.min(fill)
}

/// The last speech-bearing sample: the recording minus its trailing frames
/// quieter than [`SPEECH_RMS_FLOOR`]; the whole recording when every frame
/// is that quiet.
fn speech_end(samples: &[f32]) -> usize {
    let mut end = samples.len();
    while end > 0 {
        let start = end.saturating_sub(FRAME_SAMPLES);
        if frame_rms(&samples[start..end]) >= SPEECH_RMS_FLOOR {
            return end;
        }
        end = start;
    }
    samples.len()
}

/// Mean energy of the samples within `half` of `center`.
fn boundary_energy(samples: &[f32], center: usize, half: usize) -> f32 {
    let start = center.saturating_sub(half);
    let end = (center + half).min(samples.len());
    if end <= start {
        return 0.0;
    }
    samples[start..end].iter().map(|s| s * s).sum::<f32>() / (end - start) as f32
}

/// The quietest frame boundary near `target_frame` that keeps the windows
/// overlapping, its energy, and the median energy of the candidates
/// (FluidAudio `bestBoundaryCandidate`).
fn best_boundary(
    samples: &[f32],
    target_frame: usize,
    radius: usize,
    previous_start: usize,
    latest_covered: usize,
) -> (usize, f32, f32) {
    let total = samples.len();
    let lower = target_frame.saturating_sub(radius).max(1);
    let upper = ((total.saturating_sub(1)) / FRAME_SAMPLES).min(target_frame + radius);
    let target_start = (target_frame * FRAME_SAMPLES)
        .max(previous_start + FRAME_SAMPLES)
        .min(latest_covered);
    let mut best = (target_start, f32::MAX);
    let mut scores = Vec::new();
    for frame in lower..=upper {
        let candidate = frame * FRAME_SAMPLES;
        if candidate <= previous_start || candidate > latest_covered {
            continue;
        }
        let score = boundary_energy(samples, candidate, FRAME_SAMPLES);
        scores.push(score);
        if score < best.1 {
            best = (candidate, score);
        }
    }
    if scores.is_empty() {
        return (target_start, f32::MAX, 0.0);
    }
    scores.sort_by(f32::total_cmp);
    (best.0, best.1, scores[scores.len() / 2])
}

/// FluidAudio `silenceAlignedChunkStarts` without warm-up prefixes: each
/// window after the first starts at the quietest frame within 4 s of the
/// regular stride when it is near silence (5% of the median energy), else
/// at the quietest within 0.5 s when it is a valley (35%), else on the
/// stride; windows always overlap by at least six frames.
fn silence_aligned_starts(samples: &[f32], chunk: usize, stride: usize) -> Vec<usize> {
    let total = samples.len();
    let silence_radius = 4 * SAMPLE_RATE / FRAME_SAMPLES;
    let valley_radius = SAMPLE_RATE / 2 / FRAME_SAMPLES;
    let minimum_overlap = FRAME_SAMPLES * 6;
    let mut starts = vec![0];
    let mut previous = 0;
    let mut target = stride;
    while target < total {
        let target_frame = target / FRAME_SAMPLES;
        let latest_covered = previous + chunk - minimum_overlap;
        let target_start = (target_frame * FRAME_SAMPLES)
            .max(previous + FRAME_SAMPLES)
            .min(latest_covered);
        let below = |score: f32, median: f32, ratio: f32| median > 0.0 && score <= median * ratio;
        let (silence, score, median) = best_boundary(
            samples,
            target_frame,
            silence_radius,
            previous,
            latest_covered,
        );
        let mut start = if below(score, median, 0.05) {
            silence
        } else {
            let (valley, score, median) = best_boundary(
                samples,
                target_frame,
                valley_radius,
                previous,
                latest_covered,
            );
            if below(score, median, 0.35) {
                valley
            } else {
                target_start
            }
        };
        if start <= previous {
            start = (previous + stride).min(total);
        }
        starts.push(start);
        previous = start;
        target += stride;
    }
    starts
}

/// Decodes a recording with `decode`: as one window when it fits, else
/// as overlapping windows merged at the seams.
pub(crate) fn transcribe<F>(
    samples: &[f32],
    vocab: &Vocabulary,
    layout: Layout,
    mut decode: F,
) -> Result<Vec<Token>, AsrError>
where
    F: FnMut(&WindowRequest) -> Result<Vec<Token>, AsrError>,
{
    if samples.len() <= MAX_MODEL_SAMPLES {
        return decode(&single_window(samples.len()));
    }
    transcribe_long(samples, vocab, layout, decode)
}

/// The window decodes of the last preview of the current recording.
///
/// A preview gets the whole recording so far, which only grows until the
/// preview stream resets, so a window planned again with the same request
/// covers the same samples and decodes to the same tokens. Only windows
/// that changed, such as the growing last window, decode again, which keeps
/// a preview of a 15-minute recording to a few windows (ASR-17).
#[derive(Debug, Default)]
pub(crate) struct PreviewWindows {
    sample_count: usize,
    language: Option<Language>,
    windows: HashMap<WindowRequest, Vec<Token>>,
}

impl PreviewWindows {
    /// Decodes `samples` like [`transcribe`], with `decode` called only for
    /// windows the last preview did not decode. Fewer samples than last
    /// time, or another language, start a new recording.
    pub(crate) fn transcribe<F>(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
        vocab: &Vocabulary,
        layout: Layout,
        mut decode: F,
    ) -> Result<Vec<Token>, AsrError>
    where
        F: FnMut(&WindowRequest) -> Result<Vec<Token>, AsrError>,
    {
        if samples.len() < self.sample_count || language != self.language {
            self.windows.clear();
        }
        self.sample_count = samples.len();
        self.language = language;
        let mut used = HashMap::new();
        let tokens = transcribe(samples, vocab, layout, |request| {
            let tokens = match used.get(request).or_else(|| self.windows.get(request)) {
                Some(tokens) => tokens.clone(),
                None => decode(request)?,
            };
            used.insert(*request, tokens.clone());
            Ok(tokens)
        })?;
        self.windows = used;
        Ok(tokens)
    }
}

/// Decodes a recording longer than one window with `decode`, merging the
/// windows and repairing seam gaps.
fn transcribe_long<F>(
    samples: &[f32],
    vocab: &Vocabulary,
    layout: Layout,
    mut decode: F,
) -> Result<Vec<Token>, AsrError>
where
    F: FnMut(&WindowRequest) -> Result<Vec<Token>, AsrError>,
{
    let plan = plan_windows(samples, layout);
    let mut outputs = Vec::with_capacity(plan.len());
    for window in &plan {
        outputs.push(decode(&window.request)?);
    }
    let mut outputs = outputs.into_iter();
    let mut merged = outputs.next().unwrap_or_default();
    for (index, window) in outputs.enumerate() {
        let seam = Seam {
            left: plan[index].frames,
            right: plan[index + 1].frames,
        };
        merged = merge_windows(&merged, &window, vocab, seam);
    }
    merged = enforce_monotonic(merged);
    if plan.len() > 1 {
        merged = collapse_seam_duplicates(merged, vocab);
    }
    if plan.len() > 1 && merged.len() > 1 {
        merged = repair_seam_gaps(merged, samples, vocab, &mut decode)?;
    }
    Ok(merged)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seam {
    left: WindowFrames,
    right: WindowFrames,
}

#[derive(Debug, Clone, Copy)]
struct Indexed {
    index: usize,
    token: Token,
    start: f64,
}

fn start_time(token: &Token) -> f64 {
    token.frame as f64 * FRAME_SECONDS
}

/// FluidAudio `mergeChunks`: anchor the overlap on matching tokens, else
/// cut at the midpoint.
fn merge_windows(left: &[Token], right: &[Token], vocab: &Vocabulary, seam: Seam) -> Vec<Token> {
    let (Some(last), Some(first)) = (left.last(), right.first()) else {
        return [left, right].concat();
    };
    let left_end = start_time(last) + FRAME_SECONDS;
    let right_start = start_time(first);
    if left_end <= right_start {
        return [left, right].concat();
    }
    let overlap_left: Vec<Indexed> = left
        .iter()
        .enumerate()
        .map(|(index, token)| Indexed {
            index,
            token: *token,
            start: start_time(token),
        })
        .filter(|t| t.start + FRAME_SECONDS > right_start - OVERLAP_SECONDS)
        .collect();
    let overlap_right: Vec<Indexed> = right
        .iter()
        .enumerate()
        .map(|(index, token)| Indexed {
            index,
            token: *token,
            start: start_time(token),
        })
        .filter(|t| t.start < left_end + OVERLAP_SECONDS)
        .collect();
    if overlap_left.len() < 2 || overlap_right.len() < 2 {
        return merge_by_midpoint(left, right, left_end, right_start, vocab);
    }
    let minimum_pairs = (overlap_left.len() / 2).max(1);
    let matcher = |l: &Indexed, r: &Indexed| {
        vocab.same_ignoring_case(l.token.id, r.token.id)
            && (l.start - r.start).abs() < OVERLAP_SECONDS / 2.0
    };
    let contiguous = longest_contiguous_match(&overlap_left, &overlap_right, matcher);
    let matches = if contiguous.len() >= minimum_pairs {
        contiguous
    } else {
        longest_common_subsequence(&overlap_left, &overlap_right, matcher)
    };
    if matches.is_empty() {
        return merge_by_midpoint(left, right, left_end, right_start, vocab);
    }
    let pairs: Vec<(usize, usize)> = matches
        .iter()
        .map(|&(l, r)| (overlap_left[l].index, overlap_right[r].index))
        .collect();
    merge_using_matches(&pairs, left, right, vocab, seam)
}

/// The longest run of consecutive matches, as index pairs.
fn longest_contiguous_match<T>(
    left: &[T],
    right: &[T],
    matcher: impl Fn(&T, &T) -> bool,
) -> Vec<(usize, usize)> {
    let mut best = Vec::new();
    for i in 0..left.len() {
        for j in 0..right.len() {
            if !matcher(&left[i], &right[j]) {
                continue;
            }
            let run: Vec<(usize, usize)> = (i..left.len())
                .zip(j..right.len())
                .take_while(|&(k, l)| matcher(&left[k], &right[l]))
                .collect();
            if run.len() > best.len() {
                best = run;
            }
        }
    }
    best
}

/// Matching pairs of a longest common subsequence.
fn longest_common_subsequence<T>(
    left: &[T],
    right: &[T],
    matcher: impl Fn(&T, &T) -> bool,
) -> Vec<(usize, usize)> {
    let (n, m) = (left.len(), right.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if matcher(&left[i - 1], &right[j - 1]) {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if matcher(&left[i - 1], &right[j - 1]) {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] > dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

/// FluidAudio `mergeUsingMatches`: left before the first match, the matches
/// (left identity, better-placed timing), the longer gap between matches,
/// then the right tail spliced at a word boundary.
fn merge_using_matches(
    pairs: &[(usize, usize)],
    left: &[Token],
    right: &[Token],
    vocab: &Vocabulary,
    seam: Seam,
) -> Vec<Token> {
    let resolved: Vec<Token> = pairs
        .iter()
        .map(|&(l, r)| {
            let mut merged = left[l];
            let right_copy = right[r];
            let into_right_head = right_copy.frame - seam.right.decode_start;
            let from_left_tail = seam.left.end - merged.frame;
            if into_right_head >= SEAM_HEAD_GUARD_FRAMES || into_right_head > from_left_tail {
                merged.frame = right_copy.frame;
                merged.duration = right_copy.duration;
            }
            merged
        })
        .collect();

    let mut result: Vec<Token> = Vec::new();
    if let Some(&(first_left, _)) = pairs.first() {
        result.extend_from_slice(&left[..first_left]);
    }
    for (idx, &(l, r)) in pairs.iter().enumerate() {
        result.push(resolved[idx]);
        let Some(&(next_l, next_r)) = pairs.get(idx + 1) else {
            continue;
        };
        let gap_left = left.get(l + 1..next_l).unwrap_or_default();
        let gap_right = right.get(r + 1..next_r).unwrap_or_default();
        let from_right = gap_right.len() > gap_left.len();
        let shift = if from_right {
            0
        } else {
            resolved[idx].frame - left[l].frame
        };
        let (lower, upper) = (resolved[idx].frame, resolved[idx + 1].frame);
        for token in if from_right { gap_right } else { gap_left } {
            let mut token = *token;
            token.frame = (token.frame + shift).max(lower).min(upper);
            result.push(token);
        }
    }

    let Some(&(last_left, last_right)) = pairs.last() else {
        return result;
    };
    let tail_start = last_right + 1;
    let Some(first_tail) = right.get(tail_start) else {
        return result;
    };
    if vocab.is_splice_safe(first_tail.id) {
        result.extend_from_slice(&right[tail_start..]);
        return result;
    }
    // the splice lands mid-word (FluidAudio #683): let exactly one window
    // segment the seam word. (Deviation: FluidAudio also treats a bare
    // punctuation piece as a word start here, so a left "▁pre - p" against
    // a right "▁pre p are" popped only "- p" and typed "pre prepare".)
    let starts_word = |token: &Token| vocab::starts_word(vocab.piece(token.id));
    if let Some(word_start) = (0..=last_right).rev().find(|&i| starts_word(&right[i]))
        && let Some(seam_word) = result.iter().rposition(starts_word)
    {
        result.truncate(seam_word);
        result.extend_from_slice(&right[word_start..]);
        return result;
    }
    let mut cursor = last_left + 1;
    while cursor < left.len() && !vocab.is_splice_safe(left[cursor].id) {
        result.push(left[cursor]);
        cursor += 1;
    }
    match (tail_start..right.len()).find(|&i| vocab.is_splice_safe(right[i].id)) {
        Some(resume) => result.extend_from_slice(&right[resume..]),
        None => result.extend_from_slice(&right[tail_start..]),
    }
    result
}

/// FluidAudio `mergeByMidpoint`, keeping words whole.
fn merge_by_midpoint(
    left: &[Token],
    right: &[Token],
    left_end: f64,
    right_start: f64,
    vocab: &Vocabulary,
) -> Vec<Token> {
    let cutoff = (left_end + right_start) / 2.0;
    let mut left_cut = left
        .iter()
        .position(|t| start_time(t) >= cutoff)
        .unwrap_or(left.len());
    let mut right_cut = right
        .iter()
        .position(|t| start_time(t) >= cutoff)
        .unwrap_or(right.len());
    if left_cut > 0 {
        while left_cut < left.len() && !vocab.is_splice_safe(left[left_cut].id) {
            left_cut += 1;
        }
    }
    if let Some(scan) = (right_cut..right.len()).find(|&i| vocab.is_splice_safe(right[i].id)) {
        right_cut = scan;
    }
    [&left[..left_cut], &right[right_cut..]].concat()
}

/// Clamps frames to be non-decreasing without reordering (FluidAudio #825).
fn enforce_monotonic(mut tokens: Vec<Token>) -> Vec<Token> {
    let mut latest = i64::MIN;
    for token in &mut tokens {
        if token.frame < latest {
            token.frame = latest;
        } else {
            latest = token.frame;
        }
    }
    tokens
}

/// Drops an adjacent case-only duplicate of a seam word, such as "have Have"
/// from a false sentence start (FluidAudio #706).
fn collapse_seam_duplicates(tokens: Vec<Token>, vocab: &Vocabulary) -> Vec<Token> {
    struct Word {
        tokens: Vec<Token>,
        core: String,
        start: i64,
        ends_sentence: bool,
    }
    let mut words: Vec<Word> = Vec::new();
    for token in tokens {
        match words.last_mut() {
            Some(word) if !vocab::starts_word(vocab.piece(token.id)) => word.tokens.push(token),
            _ => words.push(Word {
                tokens: vec![token],
                core: String::new(),
                start: token.frame,
                ends_sentence: false,
            }),
        }
    }
    for word in &mut words {
        let text: String = word
            .tokens
            .iter()
            .map(|t| vocab::strip_word_boundary(vocab.piece(t.id)))
            .collect();
        word.core = text
            .trim_matches(|c: char| vocab::is_punctuation(c) || c.is_whitespace())
            .to_owned();
        word.ends_sentence = text.ends_with(['.', '?', '!', ':']);
    }

    let mut keep = vec![true; words.len()];
    let mut last_kept: Option<usize> = None;
    for index in 0..words.len() {
        let Some(previous_index) = last_kept else {
            last_kept = Some(index);
            continue;
        };
        let (previous, current) = (&words[previous_index], &words[index]);
        let duplicate = !previous.core.is_empty()
            && !current.core.is_empty()
            && previous.core != current.core
            && previous.core.to_lowercase() == current.core.to_lowercase()
            && current.core.chars().next().is_some_and(char::is_alphabetic)
            && !previous.ends_sentence
            && current.start - previous.start <= OVERLAP_FRAMES;
        if !duplicate {
            last_kept = Some(index);
            continue;
        }
        // keep the lower-case copy, else the earlier one
        if current.core == current.core.to_lowercase()
            && previous.core != previous.core.to_lowercase()
        {
            keep[previous_index] = false;
            last_kept = Some(index);
        } else {
            keep[index] = false;
        }
    }
    words
        .into_iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .flat_map(|(word, _)| word.tokens)
        .collect()
}

// seam-gap repair (FluidAudio #758)

const MAX_REPAIR_PROBES: usize = 32;
const MIN_GAP_SECONDS: f64 = 1.5;
const MIN_GAP_SPEECH_SECONDS: f64 = 0.5;
const SPEECH_RMS_CEILING: f32 = 0.008;
const SPEECH_RMS_FLOOR: f32 = 0.0005;
const SPEECH_RMS_REFERENCE_SCALE: f32 = 0.3;
const SPEECH_RMS_REFERENCE_PERCENTILE: f64 = 0.75;
const EDGE_TOLERANCE_FRAMES: i64 = 6;

struct Probe {
    gap_start: i64,
    gap_end: i64,
    lead: Token,
    tail: Token,
    placements: Vec<WindowRequest>,
}

/// Re-decodes gaps between tokens that hold speech with a fresh window at
/// the gap and splices in only tokens inside it.
fn repair_seam_gaps<F>(
    tokens: Vec<Token>,
    samples: &[f32],
    vocab: &Vocabulary,
    decode: &mut F,
) -> Result<Vec<Token>, AsrError>
where
    F: FnMut(&WindowRequest) -> Result<Vec<Token>, AsrError>,
{
    let total = samples.len() as i64;
    let frame = FRAME_SAMPLES as i64;
    let window_samples =
        ((MAX_MODEL_SAMPLES - MEL_HOP_SAMPLES) / FRAME_SAMPLES * FRAME_SAMPLES) as i64;
    let min_gap_frames = ((MIN_GAP_SECONDS / FRAME_SECONDS) as i64).max(2);
    let threshold = adaptive_speech_threshold(samples);

    let mut working = tokens;
    let mut probes = 0;
    let mut probed: HashSet<i64> = HashSet::new();
    for _ in 0..3 {
        let mut pass: Vec<Probe> = Vec::new();
        for index in 0..working.len().saturating_sub(1) {
            if probes >= MAX_REPAIR_PROBES {
                break;
            }
            let (current, next) = (working[index], working[index + 1]);
            let gap_start = current.frame + current.duration.max(1);
            let gap_end = next.frame;
            if gap_end - gap_start < min_gap_frames || probed.contains(&gap_start) {
                continue;
            }
            let gap_start_sample = gap_start * frame;
            let gap_end_sample = (gap_end * frame).min(total);
            if gap_end_sample <= gap_start_sample {
                continue;
            }
            let speech = speech_seconds(
                samples,
                gap_start_sample as usize,
                gap_end_sample as usize,
                threshold,
            );
            if speech < MIN_GAP_SPEECH_SECONDS {
                continue;
            }
            probed.insert(gap_start);
            probes += 1;

            // cold-start at the gap first, then centred on it
            let center = (gap_start_sample + gap_end_sample) / 2;
            let placements = [gap_start_sample, center - window_samples / 2]
                .into_iter()
                .filter_map(|placement| {
                    let start = placement.min(total - window_samples).max(0) / frame * frame;
                    let end = (start + window_samples).min(total);
                    (end > start).then(|| {
                        let (start, end) = (start as usize, end as usize);
                        WindowRequest {
                            start,
                            end,
                            audio_length: end - start,
                            audio_frames: (end - start).div_ceil(FRAME_SAMPLES),
                            start_frame: 0,
                            frame_offset: (start / FRAME_SAMPLES) as i64,
                            is_last: end == samples.len(),
                            emit_after_frame: None,
                        }
                    })
                })
                .collect();
            pass.push(Probe {
                gap_start,
                gap_end,
                lead: word_neighbor(&working, index, -1, vocab),
                tail: word_neighbor(&working, index + 1, 1, vocab),
                placements,
            });
        }
        if pass.is_empty() {
            break;
        }
        let mut inserts = Vec::new();
        for probe in &pass {
            for placement in &probe.placements {
                let window = decode(placement)?;
                let candidate = splice_candidate(&window, probe, vocab);
                if !candidate.is_empty() {
                    tracing::debug!(
                        tokens = candidate.len(),
                        gap_start_s = probe.gap_start as f64 * FRAME_SECONDS,
                        gap_end_s = probe.gap_end as f64 * FRAME_SECONDS,
                        "seam-gap repair recovered tokens"
                    );
                    inserts.extend(candidate);
                    break;
                }
            }
        }
        if inserts.is_empty() {
            break;
        }
        working.extend(inserts);
        working.sort_by_key(|t| t.frame);
    }
    Ok(working)
}

/// The token of the word next to a gap, walking past punctuation.
fn word_neighbor(stream: &[Token], index: usize, step: isize, vocab: &Vocabulary) -> Token {
    let mut at = index;
    while let Some(next) = at.checked_add_signed(step).filter(|&n| n < stream.len()) {
        if !vocab::is_punctuation_only(vocab.piece(stream[at].id)) {
            break;
        }
        at = next;
    }
    stream[at]
}

/// The probe tokens strictly inside the gap, starting at a word, minus
/// re-heard copies of the neighbouring words.
fn splice_candidate(window: &[Token], probe: &Probe, vocab: &Vocabulary) -> Vec<Token> {
    let same_piece = |a: usize, b: usize| {
        a == b || {
            let (pa, pb) = (vocab.piece(a), vocab.piece(b));
            !pa.is_empty() && !pb.is_empty() && pa.to_lowercase() == pb.to_lowercase()
        }
    };
    let mut candidate: Vec<Token> = window
        .iter()
        .filter(|t| t.frame > probe.gap_start && t.frame < probe.gap_end - 1)
        .copied()
        .collect();
    let mut head = candidate
        .iter()
        .position(|t| vocab.is_splice_safe(t.id))
        .unwrap_or(candidate.len());
    while candidate.get(head).is_some_and(|t| {
        same_piece(t.id, probe.lead.id)
            && (t.frame - probe.lead.frame).abs() <= EDGE_TOLERANCE_FRAMES
    }) {
        head += 1;
    }
    let mut end = candidate.len();
    while end > head && {
        let t = candidate[end - 1];
        same_piece(t.id, probe.tail.id)
            && (probe.tail.frame - t.frame).abs() <= EDGE_TOLERANCE_FRAMES
    } {
        end -= 1;
    }
    while head < end && vocab.has_splice_guard() && {
        let id = candidate[head].id;
        !vocab.is_splice_safe(id) || vocab::is_punctuation_only(vocab.piece(id))
    } {
        head += 1;
    }
    candidate.truncate(end);
    candidate.drain(..head);
    candidate
}

fn frame_rms(frame: &[f32]) -> f32 {
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/// A speech-energy threshold scaled to the recording's level: 30% of the
/// 75th-percentile frame RMS, ignoring digital silence, within fixed bounds.
fn adaptive_speech_threshold(samples: &[f32]) -> f32 {
    let mut levels: Vec<f32> = samples
        .as_chunks::<FRAME_SAMPLES>()
        .0
        .iter()
        .filter(|frame| frame.iter().any(|&s| s != 0.0))
        .map(|frame| frame_rms(frame))
        .collect();
    if levels.is_empty() {
        return SPEECH_RMS_CEILING;
    }
    levels.sort_by(f32::total_cmp);
    let index =
        ((levels.len() as f64 * SPEECH_RMS_REFERENCE_PERCENTILE) as usize).min(levels.len() - 1);
    (levels[index] * SPEECH_RMS_REFERENCE_SCALE).clamp(SPEECH_RMS_FLOOR, SPEECH_RMS_CEILING)
}

/// Seconds of frames louder than `threshold` between two sample offsets.
fn speech_seconds(samples: &[f32], start: usize, end: usize, threshold: f32) -> f64 {
    let end = end.min(samples.len());
    let frames = samples
        .get(start..end)
        .unwrap_or_default()
        .as_chunks::<FRAME_SAMPLES>()
        .0
        .iter()
        .filter(|frame| frame_rms(*frame) > threshold)
        .count();
    frames as f64 * FRAME_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: usize, frame: i64) -> Token {
        Token {
            id,
            frame,
            confidence: 0.9,
            duration: 1,
        }
    }

    fn ids(tokens: &[Token]) -> Vec<usize> {
        tokens.iter().map(|t| t.id).collect()
    }

    /// Word pieces 0..=8 plus a continuation, punctuation, and case twin.
    fn words() -> Vocabulary {
        let mut pieces: Vec<String> = [
            "▁troy",
            "▁abed",
            "▁annie",
            "▁britta",
            "▁shirley",
            "▁pierce",
            "▁chang",
            "▁dean",
            "▁jeff",
        ]
        .map(str::to_owned)
        .to_vec();
        pieces.extend(
            [
                "nes", ".", "▁Troy", "▁have", "▁Have", "▁pre", "-", "p", "ay", "are",
            ]
            .map(str::to_owned),
        );
        Vocabulary::new(pieces)
    }
    const NES: usize = 9;
    const DOT: usize = 10;
    const TROY_UPPER: usize = 11;
    const HAVE: usize = 12;
    const HAVE_UPPER: usize = 13;
    const PRE: usize = 14;
    const HYPHEN: usize = 15;
    const P: usize = 16;
    const AY: usize = 17;
    const ARE: usize = 18;

    #[test]
    fn layout_constants_match_fluidaudio() {
        let cases = [
            (Layout::V2, 238_080, 206_080),
            (Layout::Japanese, 238_080, 206_080),
            (Layout::V3, 239_360, 207_360),
        ];
        for (layout, want_chunk, want_stride) in cases {
            assert_eq!(layout.chunk_samples(), want_chunk, "{layout:?}");
            assert_eq!(layout.overlap_samples(), 32_000, "{layout:?}");
            assert_eq!(layout.stride_samples(), want_stride, "{layout:?}");
        }
    }

    #[test]
    fn plan_windows_cover_the_recording() {
        struct Case {
            name: &'static str,
            total: usize,
            want_starts: Vec<usize>,
            want_last_end: usize,
        }
        let cases = [
            Case {
                name: "just over one window",
                total: 240_001,
                want_starts: vec![0, 206_080 - 1_280],
                want_last_end: 240_001,
            },
            Case {
                name: "one minute",
                total: 960_000,
                want_starts: vec![0, 204_800, 410_880, 616_960, 823_040],
                want_last_end: 960_000,
            },
            Case {
                name: "fifteen minutes",
                total: 14_400_000,
                want_starts: (0..70)
                    .map(|i| (i * 206_080usize).saturating_sub(if i > 0 { 1_280 } else { 0 }))
                    .collect(),
                want_last_end: 14_400_000,
            },
        ];
        for tc in cases {
            let plan = plan_windows(&vec![0.1; tc.total], Layout::V2);
            let starts: Vec<usize> = plan.iter().map(|w| w.request.start).collect();
            assert_eq!(starts, tc.want_starts, "{}", tc.name);
            let last = plan.last().expect("windows");
            assert_eq!(last.request.end, tc.want_last_end, "{}", tc.name);
            assert!(last.request.is_last, "{}", tc.name);
            for window in &plan {
                assert!(
                    window.request.end - window.request.start <= MAX_MODEL_SAMPLES,
                    "{}",
                    tc.name
                );
                assert!(
                    !window.request.is_last || window.request.end == tc.total,
                    "{}",
                    tc.name
                );
            }
            for pair in plan.windows(2) {
                let overlap = pair[0].request.end - pair[1].request.start;
                assert_eq!(overlap, 32_000 + MEL_CONTEXT_SAMPLES, "{}", tc.name);
            }
        }
    }

    #[test]
    fn second_window_skips_its_context_frame() {
        let plan = plan_windows(&vec![0.1; 480_000], Layout::V2);
        let second = plan[1].request;
        assert_eq!(second.start_frame, 1);
        assert_eq!(second.frame_offset, 161);
        assert_eq!(second.audio_length, 239_360);
        assert_eq!(second.audio_frames, 186);
    }

    /// A 14.96 s window, then a short tail: the v3 and Japanese final
    /// windows fill backwards with real audio and suppress the backfilled
    /// prefix; v2 zero-pads. With trailing silence the final window ends
    /// at the speech end and is still flagged last, so the decoder's
    /// end-of-recording flush runs.
    #[test]
    fn final_window_end_alignment() {
        struct Case {
            name: &'static str,
            layout: Layout,
            trailing_silence: usize,
            want_start: usize,
            want_end: usize,
            want_emit_after: Option<i64>,
            want_frame_offset: i64,
            want_start_frame: usize,
            want_last: bool,
        }
        let cases = [
            Case {
                name: "v2 keeps the zero-padded layout",
                layout: Layout::V2,
                trailing_silence: 0,
                want_start: 206_080 - 1_280,
                want_end: 336_640,
                want_emit_after: None,
                want_frame_offset: 161,
                want_start_frame: 1,
                want_last: true,
            },
            Case {
                name: "v3 backfills the final window",
                layout: Layout::V3,
                trailing_silence: 0,
                // remaining 336,640 - 207,360 = 129,280; fill = 239,360 - 129,280
                want_start: 207_360 - 110_080,
                want_end: 336_640,
                want_emit_after: Some(162),
                want_frame_offset: 76,
                want_start_frame: 0,
                want_last: true,
            },
            Case {
                name: "Japanese backfills too, without mel context",
                layout: Layout::Japanese,
                trailing_silence: 0,
                // remaining 336,640 - 206,080 = 130,560; fill = 238,080 - 130,560
                want_start: 206_080 - 107_520,
                want_end: 336_640,
                want_emit_after: Some(161),
                want_frame_offset: 77,
                want_start_frame: 0,
                want_last: true,
            },
            Case {
                // regression: FluidAudio leaves this window unflagged, so the
                // end-of-recording flush never runs and the final period is lost
                name: "v3 with trailing silence is still the last window",
                layout: Layout::V3,
                trailing_silence: 16_640,
                // remaining 320,000 - 207,360 = 112,640; fill = 239,360 - 112,640
                want_start: 207_360 - 126_720,
                want_end: 320_000,
                want_emit_after: Some(162),
                want_frame_offset: 63,
                want_start_frame: 0,
                want_last: true,
            },
            Case {
                name: "Japanese with trailing silence is still the last window",
                layout: Layout::Japanese,
                trailing_silence: 16_640,
                // remaining 320,000 - 206,080 = 113,920; fill = 238,080 - 113,920
                want_start: 206_080 - 124_160,
                want_end: 320_000,
                want_emit_after: Some(161),
                want_frame_offset: 64,
                want_start_frame: 0,
                want_last: true,
            },
            Case {
                name: "v2 with trailing silence zero-pads to the end",
                layout: Layout::V2,
                trailing_silence: 16_640,
                want_start: 206_080 - 1_280,
                want_end: 336_640,
                want_emit_after: None,
                want_frame_offset: 161,
                want_start_frame: 1,
                want_last: true,
            },
        ];
        for tc in cases {
            let mut samples = vec![0.1f32; 336_640 - tc.trailing_silence];
            samples.resize(336_640, 0.0);
            let plan = plan_windows(&samples, tc.layout);
            assert_eq!(plan.len(), 2, "{}", tc.name);
            let last = plan[1].request;
            assert_eq!(last.start, tc.want_start, "{}", tc.name);
            assert_eq!(last.end, tc.want_end, "{}", tc.name);
            assert_eq!(last.emit_after_frame, tc.want_emit_after, "{}", tc.name);
            assert_eq!(last.frame_offset, tc.want_frame_offset, "{}", tc.name);
            assert_eq!(last.start_frame, tc.want_start_frame, "{}", tc.name);
            assert_eq!(last.audio_length, last.end - last.start, "{}", tc.name);
            assert!(last.audio_length <= MAX_MODEL_SAMPLES, "{}", tc.name);
            assert_eq!(last.is_last, tc.want_last, "{}", tc.name);
            assert_eq!(plan[0].request.start, 0, "{}", tc.name);
            assert_eq!(plan[0].request.emit_after_frame, None, "{}", tc.name);
        }
    }

    #[test]
    fn v3_windows_start_in_pauses() {
        struct Case {
            name: &'static str,
            pause: Option<(usize, usize)>,
            want_second_start: usize,
        }
        let cases = [
            Case {
                name: "a pause within 4 s of the stride",
                pause: Some((180_000, 184_000)),
                // the quietest boundary sits mid-pause, frame-aligned
                want_second_start: 181_760,
            },
            Case {
                name: "no pause keeps the stride",
                pause: None,
                want_second_start: 207_360,
            },
        ];
        for tc in cases {
            // speech-like noise at a steady level
            let mut samples: Vec<f32> = (0..700_000usize)
                .map(|i| ((i * 7_919) % 1_000) as f32 / 5_000.0 - 0.1)
                .collect();
            if let Some((start, end)) = tc.pause {
                samples[start..end].fill(0.0);
            }
            let plan = plan_windows(&samples, Layout::V3);
            assert_eq!(plan[1].request.start, tc.want_second_start, "{}", tc.name);
            assert_eq!(plan[1].request.start_frame, 0, "{}", tc.name);
            for pair in plan.windows(2) {
                let overlap = pair[0].request.end as i64 - pair[1].request.start as i64;
                assert!(overlap >= 6 * FRAME_SAMPLES as i64, "{}", tc.name);
            }
            assert_eq!(
                plan.last().map(|w| w.request.end),
                Some(700_000),
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn speech_end_skips_trailing_silence() {
        let cases = [
            ("speech to the end", vec![0.1f32; 10_000], 10_000),
            (
                "trailing silence",
                [vec![0.1f32; 6_400], vec![0.0; 3_000]].concat(),
                6_400 + 3_000 - 2_560,
            ),
            ("all silent", vec![0.0f32; 5_000], 5_000),
        ];
        for (name, samples, want) in cases {
            assert_eq!(speech_end(&samples), want, "{name}");
        }
    }

    #[test]
    fn single_window_rounds_length_to_frames() {
        struct Case {
            len: usize,
            want_length: usize,
            want_frames: usize,
        }
        let cases = [
            Case {
                len: 16_000,
                want_length: 16_640,
                want_frames: 13,
            },
            Case {
                len: 12_800,
                want_length: 12_800,
                want_frames: 10,
            },
            Case {
                len: 239_500,
                want_length: 239_500,
                want_frames: 188,
            },
        ];
        for tc in cases {
            let request = single_window(tc.len);
            assert_eq!(request.audio_length, tc.want_length, "{}", tc.len);
            assert_eq!(request.audio_frames, tc.want_frames, "{}", tc.len);
        }
    }

    fn seam() -> Seam {
        Seam {
            left: WindowFrames {
                decode_start: 0,
                end: 186,
            },
            right: WindowFrames {
                decode_start: 161,
                end: 347,
            },
        }
    }

    #[test]
    fn merge_windows_cases() {
        struct Case {
            name: &'static str,
            left: Vec<Token>,
            right: Vec<Token>,
            want: Vec<usize>,
        }
        let cases = [
            Case {
                name: "no overlap concatenates",
                left: vec![token(0, 10), token(1, 20)],
                right: vec![token(2, 30)],
                want: vec![0, 1, 2],
            },
            Case {
                name: "matching overlap keeps one copy",
                left: vec![token(0, 150), token(1, 165), token(2, 175), token(3, 182)],
                right: vec![token(1, 166), token(2, 175), token(3, 182), token(4, 200)],
                want: vec![0, 1, 2, 3, 4],
            },
            Case {
                name: "case twin anchors the overlap",
                left: vec![token(0, 150), token(1, 165), token(TROY_UPPER, 175)],
                right: vec![token(1, 165), token(0, 175), token(4, 190)],
                want: vec![0, 1, TROY_UPPER, 4],
            },
            Case {
                name: "no match cuts at the midpoint",
                left: vec![token(0, 160), token(1, 170), token(2, 180)],
                right: vec![token(5, 172), token(6, 182), token(7, 195)],
                want: vec![0, 1, 6, 7],
            },
            Case {
                name: "midpoint cut finishes the left word",
                left: vec![token(0, 160), token(1, 170), token(NES, 178)],
                right: vec![token(5, 170), token(6, 185)],
                want: vec![0, 1, NES, 6],
            },
            Case {
                name: "seam word segmented differently is typed once",
                left: vec![
                    token(5, 630),
                    token(6, 640),
                    token(0, 650),
                    token(1, 662),
                    token(PRE, 664),
                    token(HYPHEN, 666),
                    token(P, 667),
                    token(AY, 668),
                ],
                right: vec![
                    token(0, 650),
                    token(1, 664),
                    token(PRE, 667),
                    token(P, 669),
                    token(ARE, 671),
                    token(2, 673),
                ],
                want: vec![5, 6, 0, 1, PRE, P, ARE, 2],
            },
            Case {
                name: "right tail resumes at a word start",
                left: vec![token(0, 160), token(1, 170), token(NES, 176)],
                right: vec![token(5, 172), token(NES, 176), token(7, 195)],
                want: vec![0, 1, NES, 7],
            },
        ];
        let vocab = words();
        for tc in cases {
            let merged = merge_windows(&tc.left, &tc.right, &vocab, seam());
            assert_eq!(ids(&merged), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn seam_timing_prefers_the_copy_away_from_its_edge() {
        let vocab = words();
        let left = vec![token(0, 150), token(1, 180), token(2, 184)];
        let right = vec![token(1, 178), token(2, 183), token(3, 200)];
        let merged = merge_windows(&left, &right, &vocab, seam());
        let frames: Vec<i64> = merged.iter().map(|t| t.frame).collect();
        assert_eq!(frames, vec![150, 178, 183, 200]);
    }

    #[test]
    fn enforce_monotonic_clamps_without_reordering() {
        let tokens = vec![token(0, 5), token(1, 9), token(2, 7), token(3, 12)];
        let frames: Vec<i64> = enforce_monotonic(tokens).iter().map(|t| t.frame).collect();
        assert_eq!(frames, vec![5, 9, 9, 12]);
    }

    #[test]
    fn collapse_drops_case_only_seam_duplicates() {
        struct Case {
            name: &'static str,
            tokens: Vec<Token>,
            want: Vec<usize>,
        }
        let cases = [
            Case {
                name: "keeps the lower-case copy",
                tokens: vec![token(0, 10), token(HAVE_UPPER, 20), token(HAVE, 22)],
                want: vec![0, HAVE],
            },
            Case {
                name: "keeps the earlier copy when both are capitalized",
                tokens: vec![token(TROY_UPPER, 20), token(TROY_UPPER, 21)],
                want: vec![TROY_UPPER, TROY_UPPER],
            },
            Case {
                name: "sentence end separates repeats",
                tokens: vec![token(HAVE, 20), token(DOT, 21), token(HAVE_UPPER, 22)],
                want: vec![HAVE, DOT, HAVE_UPPER],
            },
            Case {
                name: "far apart repeats stay",
                tokens: vec![token(HAVE, 20), token(HAVE_UPPER, 60)],
                want: vec![HAVE, HAVE_UPPER],
            },
        ];
        let vocab = words();
        for tc in cases {
            assert_eq!(
                ids(&collapse_seam_duplicates(tc.tokens, &vocab)),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn splice_candidate_keeps_new_words_inside_the_gap() {
        struct Case {
            name: &'static str,
            window: Vec<Token>,
            want: Vec<usize>,
        }
        let probe = Probe {
            gap_start: 100,
            gap_end: 130,
            lead: token(0, 98),
            tail: token(8, 130),
            placements: Vec::new(),
        };
        let cases = [
            Case {
                name: "inside only",
                window: vec![token(5, 95), token(1, 110), token(2, 120), token(3, 129)],
                want: vec![1, 2],
            },
            Case {
                name: "starts at a word",
                window: vec![token(NES, 105), token(1, 110)],
                want: vec![1],
            },
            Case {
                name: "re-heard neighbours dropped",
                window: vec![token(0, 102), token(1, 110), token(8, 127)],
                want: vec![1],
            },
            Case {
                name: "silence splices nothing",
                window: vec![],
                want: vec![],
            },
        ];
        let vocab = words();
        for tc in cases {
            assert_eq!(
                ids(&splice_candidate(&tc.window, &probe, &vocab)),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn adaptive_threshold_tracks_the_recording_level() {
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            want: f32,
        }
        let cases = [
            Case {
                name: "digital silence uses the ceiling",
                samples: vec![0.0; FRAME_SAMPLES * 4],
                want: SPEECH_RMS_CEILING,
            },
            Case {
                name: "loud speech capped at the ceiling",
                samples: vec![0.5; FRAME_SAMPLES * 4],
                want: SPEECH_RMS_CEILING,
            },
            Case {
                name: "quiet recording scales down",
                samples: vec![0.01; FRAME_SAMPLES * 4],
                want: 0.003,
            },
        ];
        for tc in cases {
            let got = adaptive_speech_threshold(&tc.samples);
            assert!((got - tc.want).abs() < 1e-6, "{}: {got}", tc.name);
        }
    }

    /// A fake model that hears `truth` wherever a window can decode it,
    /// optionally deaf to some frames outside repair probes. Like the real
    /// decoder, a window with a context frame reports frames one late.
    fn fake_decode(
        truth: &[Token],
        deaf: Option<(i64, i64)>,
    ) -> impl FnMut(&WindowRequest) -> Result<Vec<Token>, AsrError> {
        let truth = truth.to_vec();
        move |request: &WindowRequest| {
            let context = request.start_frame as i64;
            let decodable = request.audio_frames.min(188) as i64 - context;
            let (first, end) = (request.frame_offset, request.frame_offset + decodable);
            let is_probe = context == 0 && request.start != 0;
            Ok(truth
                .iter()
                .filter(|t| t.frame >= first && t.frame < end)
                .filter(|t| {
                    request
                        .emit_after_frame
                        .is_none_or(|cutoff| t.frame >= cutoff)
                })
                .filter(|t| is_probe || deaf.is_none_or(|(a, b)| t.frame < a || t.frame >= b))
                .map(|t| Token {
                    frame: t.frame + context,
                    ..*t
                })
                .collect())
        }
    }

    #[test]
    fn long_recordings_neither_lose_nor_duplicate_words() {
        struct Case {
            name: &'static str,
            layout: Layout,
            seconds: usize,
            deaf: Option<(i64, i64)>,
        }
        let cases = [
            Case {
                name: "one minute",
                layout: Layout::V2,
                seconds: 60,
                deaf: None,
            },
            Case {
                name: "fifteen minutes",
                layout: Layout::V2,
                seconds: 900,
                deaf: None,
            },
            Case {
                name: "dropped words in a window are repaired",
                layout: Layout::V2,
                seconds: 60,
                deaf: Some((60, 90)),
            },
            Case {
                name: "v3 layout, one minute and a bit",
                layout: Layout::V3,
                seconds: 67,
                deaf: None,
            },
            Case {
                name: "Japanese layout, one minute and a bit",
                layout: Layout::Japanese,
                seconds: 67,
                deaf: None,
            },
        ];
        let vocab = words();
        for tc in cases {
            let samples = vec![0.1f32; tc.seconds * SAMPLE_RATE];
            let frames = (samples.len() / FRAME_SAMPLES) as i64;
            let truth: Vec<Token> = (0..frames / 4)
                .map(|i| token(i as usize % 9, i * 4))
                .collect();
            let merged = transcribe_long(&samples, &vocab, tc.layout, fake_decode(&truth, tc.deaf))
                .expect("decode");
            assert_eq!(ids(&merged), ids(&truth), "{}", tc.name);
        }
    }

    /// A recording of `seconds` whose words are `truth`, one every four
    /// frames, rotated by `shift`.
    fn recording(seconds: usize, shift: usize) -> (Vec<f32>, Vec<Token>) {
        let samples = vec![0.1f32; seconds * SAMPLE_RATE];
        let frames = (samples.len() / FRAME_SAMPLES) as i64;
        let truth = (0..frames / 4)
            .map(|i| token((i as usize + shift) % 9, i * 4))
            .collect();
        (samples, truth)
    }

    #[test]
    fn previews_decode_only_the_windows_that_changed() {
        struct Case {
            name: &'static str,
            layout: Layout,
            seconds: usize,
            deaf: Option<(i64, i64)>,
        }
        let cases = [
            Case {
                name: "three minutes",
                layout: Layout::V2,
                seconds: 180,
                deaf: None,
            },
            Case {
                name: "seam repairs are reused too",
                layout: Layout::V2,
                seconds: 120,
                deaf: Some((60, 90)),
            },
            Case {
                name: "v3 layout",
                layout: Layout::V3,
                seconds: 90,
                deaf: None,
            },
            Case {
                name: "Japanese layout",
                layout: Layout::Japanese,
                seconds: 90,
                deaf: None,
            },
        ];
        let vocab = words();
        for tc in cases {
            let (samples, truth) = recording(tc.seconds, 0);
            let mut model = fake_decode(&truth, tc.deaf);
            let mut windows = PreviewWindows::default();
            let mut most_decoded = 0;
            // a preview every 0.4 s of audio through the first seams, then
            // every 3.2 s, still well under a window stride
            let prefixes = (MIN_SAMPLES..samples.len())
                .step_by(6_400)
                .take(100)
                .chain((40 * SAMPLE_RATE..=samples.len()).step_by(51_200));
            for len in prefixes {
                let prefix = &samples[..len];
                let mut decoded = 0;
                let got = windows
                    .transcribe(prefix, None, &vocab, tc.layout, |request| {
                        decoded += 1;
                        model(request)
                    })
                    .expect("preview");
                let want = transcribe(prefix, &vocab, tc.layout, &mut model).expect("decode");
                assert_eq!(got, want, "{}: {len}", tc.name);
                most_decoded = most_decoded.max(decoded);
            }
            // however long the recording: the window that grew, the one it
            // became when the next window started, and the placements of a
            // seam-repair probe the first time its gap holds speech
            assert!(most_decoded <= 4, "{}: {most_decoded}", tc.name);
        }
    }

    #[test]
    fn a_new_recording_decodes_every_window() {
        struct Case {
            name: &'static str,
            seconds: usize,
            language: Option<Language>,
        }
        let cases = [
            Case {
                name: "fewer samples",
                seconds: 50,
                language: None,
            },
            Case {
                name: "another language",
                seconds: 70,
                language: Language::from_code("de"),
            },
        ];
        let vocab = words();
        let (troy, troy_truth) = recording(60, 0);
        let (abed, abed_truth) = recording(70, 4);
        for tc in cases {
            let mut windows = PreviewWindows::default();
            windows
                .transcribe(
                    &troy,
                    None,
                    &vocab,
                    Layout::V2,
                    fake_decode(&troy_truth, None),
                )
                .expect("preview");
            let samples = &abed[..tc.seconds * SAMPLE_RATE];
            let got = windows
                .transcribe(
                    samples,
                    tc.language,
                    &vocab,
                    Layout::V2,
                    fake_decode(&abed_truth, None),
                )
                .expect("preview");
            let want = transcribe(samples, &vocab, Layout::V2, fake_decode(&abed_truth, None))
                .expect("decode");
            assert_eq!(got, want, "{}", tc.name);
        }
    }
}
