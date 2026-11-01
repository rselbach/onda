//! Script-aware token filtering for Parakeet Ultra (ASR-12), ported from
//! FluidAudio's `TokenLanguageFilter` and `TdtDecoderV3`'s English
//! blocklist.
//!
//! When the joint network's best token is in the wrong script for the
//! selected language (Cyrillic while transcribing Polish, Latin while
//! transcribing Russian), the best right-script candidate of the joint's
//! top-K replaces it. For French, English-only function words are also
//! swapped for the best other Latin candidate.

use onda_domain::languages::Language;

use crate::tdt::TopK;
use crate::vocab::{Vocabulary, WORD_BOUNDARY};

/// A writing script the filter can enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Script {
    Latin,
    Cyrillic,
    Greek,
}

/// FluidAudio's `Language.script`: the languages FluidAudio knows, by code.
/// Others (Japanese, Turkish, ...) get no filter, as FluidAudio's
/// `Language(rawValue:)` is nil for them.
pub(crate) fn script_for(language: Language) -> Option<Script> {
    match language.code() {
        "en" | "es" | "fr" | "de" | "it" | "pt" | "ro" | "nl" | "da" | "sv" | "fi" | "hu"
        | "et" | "lv" | "lt" | "mt" | "pl" | "cs" | "sk" | "sl" | "hr" | "bs" => {
            Some(Script::Latin)
        }
        "ru" | "uk" | "be" | "bg" | "sr" => Some(Script::Cyrillic),
        "el" => Some(Script::Greek),
        _ => None,
    }
}

/// English-only whole-word pieces (space-prefixed in the v3 vocabulary)
/// that are essentially impossible in French prose (FluidAudio #630).
const ENGLISH_BLOCKLIST: &[usize] = &[
    506, 1502, // the, The
    575, 1976, // and, And
    2530, // they
    1180, 3247, // you, You
    1868, // with
    1050, 7603, // that, That
    1974, 5831, // this, This
    1647, // have
    3109, // from
    924, 4020, // was, Was
    4250, // were
    1714, // are
    4908, // been
    4223, // would
    6167, // could
    2783, // will
    4396, // their
    3357, // there
    4611, // when
    3470, // what
    6843, // where
    4333, // which
    4980, // who
    1491, // not
    3592, 2681, // But, but
    1960, 547,  // So, so
    2336, // It
    750, 1842, // we, We
    3285, // our
    3629, // your
    1103, // my
    4384, // him
    1535, // her
    4421, // them
    5726, // these
];

/// The per-step filter for one decode: the selected language's script and,
/// for French, the English blocklist.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TokenFilter<'a> {
    pub script: Script,
    pub french: bool,
    pub vocab: &'a Vocabulary,
    pub blank: usize,
}

impl<'a> TokenFilter<'a> {
    /// The filter for `language` on a v3-family model; `None` when the
    /// language has no script FluidAudio filters.
    pub(crate) fn new(
        language: Language,
        vocab: &'a Vocabulary,
        blank: usize,
    ) -> Option<TokenFilter<'a>> {
        Some(TokenFilter {
            script: script_for(language)?,
            french: language.code() == "fr",
            vocab,
            blank,
        })
    }

    /// Replaces a wrong-script or blocklisted `label` with the best allowed
    /// top-K candidate, updating `score` to its top-K softmax. Blanks are
    /// never replaced: swapping silence for a candidate would invent speech.
    pub(crate) fn apply(&self, label: &mut usize, score: &mut f32, top_k: Option<&TopK>) {
        let Some(top_k) = top_k else {
            return;
        };
        if *label == self.blank || top_k.ids.is_empty() {
            return;
        }
        if let Some(text) = self.vocab.get(*label)
            && !matches(text, self.script)
            && let Some((id, probability)) = self.best(top_k, self.script, false)
        {
            *label = id;
            *score = probability.clamp(0.0, 1.0);
        }
        if self.french
            && *label != self.blank
            && ENGLISH_BLOCKLIST.contains(label)
            && let Some((id, probability)) = self.best(top_k, Script::Latin, true)
        {
            *label = id;
            *score = probability;
        }
    }

    /// The highest-logit candidate in `script` (skipping the blank and,
    /// when `skip_blocklist`, blocklisted ids) and its softmax over the
    /// top-K logits. CoreML's top-K is not guaranteed sorted.
    fn best(&self, top_k: &TopK, script: Script, skip_blocklist: bool) -> Option<(usize, f32)> {
        let count = top_k.ids.len().min(top_k.logits.len());
        let mut best: Option<usize> = None;
        for index in 0..count {
            let id = top_k.ids[index];
            if skip_blocklist && (id == self.blank || ENGLISH_BLOCKLIST.contains(&id)) {
                continue;
            }
            let Some(text) = self.vocab.get(id) else {
                continue;
            };
            if !matches(text, script) {
                continue;
            }
            if best.is_none_or(|b| top_k.logits[index] > top_k.logits[b]) {
                best = Some(index);
            }
        }
        let best = best?;
        let logits = &top_k.logits[..count];
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if !max.is_finite() {
            return Some((top_k.ids[best], 0.0));
        }
        let sum: f32 = logits.iter().map(|l| (l - max).exp()).sum();
        if sum <= 0.0 {
            return Some((top_k.ids[best], 0.0));
        }
        Some((top_k.ids[best], (logits[best] - max).exp() / sum))
    }
}

/// Whether every character of `piece` (ignoring `▁`) fits `script`. Bare
/// boundary markers are script-neutral. ASCII letters are Latin only; ASCII
/// digits, punctuation, and spaces fit every script.
pub(crate) fn matches(piece: &str, script: Script) -> bool {
    let not_ascii_letter = |c: char| !c.is_ascii_alphabetic();
    piece
        .chars()
        .filter(|&c| c != WORD_BOUNDARY)
        .all(|c| match script {
            Script::Latin => matches!(c,
                '\u{20}'..='\u{7F}'
                | '\u{A0}'..='\u{24F}'
                | '\u{300}'..='\u{36F}'
                | '\u{1E00}'..='\u{1EFF}'),
            Script::Cyrillic => match c {
                '\u{400}'..='\u{4FF}' => true,
                '\u{20}'..='\u{7F}' => not_ascii_letter(c),
                _ => false,
            },
            Script::Greek => match c {
                '\u{370}'..='\u{3FF}' | '\u{1F00}'..='\u{1FFF}' | '\u{300}'..='\u{36F}' => true,
                '\u{20}'..='\u{7F}' => not_ascii_letter(c),
                _ => false,
            },
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_matching() {
        let cases = [
            ("▁Greendale", Script::Latin, true),
            ("▁Gdańsk", Script::Latin, true),
            ("▁Москва", Script::Latin, false),
            ("▁Москва", Script::Cyrillic, true),
            ("▁Troy", Script::Cyrillic, false),
            ("▁Αθήνα", Script::Greek, true),
            ("▁Abed", Script::Greek, false),
            ("▁", Script::Cyrillic, true),
            ("", Script::Latin, true),
            ("7,", Script::Cyrillic, true),
            ("日本", Script::Latin, false),
        ];
        for (piece, script, want) in cases {
            assert_eq!(matches(piece, script), want, "{piece} {script:?}");
        }
    }

    #[test]
    fn languages_map_to_fluidaudio_scripts() {
        let cases = [
            ("pl", Some(Script::Latin)),
            ("ru", Some(Script::Cyrillic)),
            ("sr", Some(Script::Cyrillic)),
            ("el", Some(Script::Greek)),
            ("ja", None),
            ("tr", None),
        ];
        for (code, want) in cases {
            let language = Language::from_code(code).expect("offered language");
            assert_eq!(script_for(language), want, "{code}");
        }
    }

    /// 0 `▁Troy`, 1 `▁Трой`, 2 `▁Abed`, 3 `▁Абед`; blank 4.
    fn vocab() -> Vocabulary {
        Vocabulary::new(
            ["▁Troy", "▁Трой", "▁Abed", "▁Абед"]
                .map(str::to_owned)
                .to_vec(),
        )
    }

    #[test]
    fn filter_swaps_wrong_script_tokens() {
        struct Case {
            name: &'static str,
            language: &'static str,
            label: usize,
            top_k: Option<TopK>,
            want: usize,
        }
        let top_k = |ids: &[usize], logits: &[f32]| {
            Some(TopK {
                ids: ids.to_vec(),
                logits: logits.to_vec(),
            })
        };
        let cases = [
            Case {
                name: "Cyrillic token while transcribing Polish",
                language: "pl",
                label: 1,
                top_k: top_k(&[1, 2, 0], &[5.0, 3.0, 4.0]),
                want: 0,
            },
            Case {
                name: "Latin token while transcribing Russian",
                language: "ru",
                label: 2,
                top_k: top_k(&[2, 3, 1], &[5.0, 4.0, 1.0]),
                want: 3,
            },
            Case {
                name: "right script is kept",
                language: "pl",
                label: 2,
                top_k: top_k(&[2, 1], &[5.0, 4.0]),
                want: 2,
            },
            Case {
                name: "blank is never replaced",
                language: "pl",
                label: 4,
                top_k: top_k(&[4, 0], &[5.0, 4.0]),
                want: 4,
            },
            Case {
                name: "blank candidate is never chosen",
                language: "pl",
                label: 1,
                top_k: top_k(&[1, 4], &[5.0, 4.0]),
                want: 1,
            },
            Case {
                name: "no top-K, no change",
                language: "pl",
                label: 1,
                top_k: None,
                want: 1,
            },
        ];
        let vocab = vocab();
        for tc in cases {
            let language = Language::from_code(tc.language).expect("offered language");
            let filter = TokenFilter::new(language, &vocab, 4).expect("filter");
            let (mut label, mut score) = (tc.label, 0.9);
            filter.apply(&mut label, &mut score, tc.top_k.as_ref());
            assert_eq!(label, tc.want, "{}", tc.name);
            assert!((0.0..=1.0).contains(&score), "{}", tc.name);
        }
    }

    #[test]
    fn french_blocklist_replaces_english_words() {
        // ids 506 (" the") and 1502 (" The") are blocklisted
        let mut pieces = vec![String::new(); 1503];
        pieces[0] = "▁le".to_owned();
        pieces[1] = "▁Ле".to_owned();
        pieces[506] = "▁the".to_owned();
        pieces[1502] = "▁The".to_owned();
        let vocab = Vocabulary::new(pieces);
        let blank = 1503;
        struct Case {
            name: &'static str,
            language: &'static str,
            ids: Vec<usize>,
            logits: Vec<f32>,
            want: usize,
        }
        let cases = [
            Case {
                name: "English word in French becomes the best Latin candidate",
                language: "fr",
                ids: vec![506, 1502, 1, 0],
                logits: vec![9.0, 8.0, 7.0, 2.0],
                want: 0,
            },
            Case {
                name: "only blocklisted candidates keep the word",
                language: "fr",
                ids: vec![506, 1502],
                logits: vec![9.0, 8.0],
                want: 506,
            },
            Case {
                name: "German keeps the word (FluidAudio #840)",
                language: "de",
                ids: vec![506, 0],
                logits: vec![9.0, 2.0],
                want: 506,
            },
        ];
        for tc in cases {
            let language = Language::from_code(tc.language).expect("offered language");
            let filter = TokenFilter::new(language, &vocab, blank).expect("filter");
            let top_k = TopK {
                ids: tc.ids,
                logits: tc.logits,
            };
            let (mut label, mut score) = (506, 0.9);
            filter.apply(&mut label, &mut score, Some(&top_k));
            assert_eq!(label, tc.want, "{}", tc.name);
        }
    }
}
