//! SentencePiece vocabulary: token ids to text, plus the piece classes the
//! long-audio merge needs (ported from FluidAudio's `AsrModels` vocabulary
//! loading, `convertTokensToText`, and `ChunkProcessor` helpers).

use std::collections::HashMap;

/// SentencePiece word-boundary marker (U+2581).
pub(crate) const WORD_BOUNDARY: char = '▁';

/// SentencePiece unknown piece, dropped from text.
pub(crate) const UNKNOWN: &str = "<unk>";

/// Token pieces indexed by id.
#[derive(Debug)]
pub(crate) struct Vocabulary {
    pieces: Vec<String>,
    splice_safe: Vec<bool>,
    splice_guard: bool,
    case_canonical: HashMap<usize, usize>,
}

impl Vocabulary {
    /// Parses FluidInference's `parakeet_vocab.json`: an object of
    /// `"id": "piece"` or an array indexed by id. Every id below `blank`
    /// must be present.
    pub(crate) fn parse(json: &str, blank: usize) -> Result<Vocabulary, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| format!("invalid vocabulary: {error}"))?;
        let mut entries: Vec<(usize, String)> = Vec::new();
        match value {
            serde_json::Value::Array(items) => {
                for (id, item) in items.into_iter().enumerate() {
                    let serde_json::Value::String(piece) = item else {
                        return Err("vocabulary array holds a non-string".to_owned());
                    };
                    entries.push((id, piece));
                }
            }
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    let (Ok(id), serde_json::Value::String(piece)) = (key.parse::<usize>(), item)
                    else {
                        continue;
                    };
                    entries.push((id, piece));
                }
            }
            _ => return Err("vocabulary has an unexpected format".to_owned()),
        }
        let len = entries
            .iter()
            .map(|(id, _)| id + 1)
            .max()
            .unwrap_or(0)
            .max(blank);
        let mut pieces: Vec<Option<String>> = vec![None; len];
        for (id, piece) in entries {
            pieces[id] = Some(piece);
        }
        if let Some(missing) = pieces[..blank].iter().position(Option::is_none) {
            return Err(format!("vocabulary lacks token {missing}"));
        }
        Ok(Vocabulary::new(
            pieces.into_iter().map(Option::unwrap_or_default).collect(),
        ))
    }

    pub(crate) fn new(pieces: Vec<String>) -> Vocabulary {
        let splice_safe = pieces
            .iter()
            .map(|piece| is_splice_safe_piece(piece))
            .collect();
        let case_canonical = case_canonical_ids(&pieces);
        Vocabulary {
            pieces,
            splice_safe,
            splice_guard: true,
            case_canonical,
        }
    }

    /// Turns the seam splice guard off, for scripts written without word
    /// spaces: no piece marks a word start, so the guard would read every
    /// piece as mid-word and drop tokens up to the next punctuation mark
    /// (FluidAudio `spliceSafeTokenIds` for Japanese). Every id then counts
    /// as splice-safe.
    pub(crate) fn without_splice_guard(mut self) -> Vocabulary {
        self.splice_guard = false;
        self
    }

    /// Whether the seam splice guard is on.
    pub(crate) fn has_splice_guard(&self) -> bool {
        self.splice_guard
    }

    /// The piece for `id`; empty for unknown ids.
    pub(crate) fn piece(&self, id: usize) -> &str {
        self.get(id).unwrap_or("")
    }

    /// The piece for `id`; `None` for ids outside the vocabulary, such as
    /// the blank.
    pub(crate) fn get(&self, id: usize) -> Option<&str> {
        self.pieces.get(id).map(String::as_str)
    }

    /// Whether splicing a seam at `id` cannot glue two words: it starts a
    /// word or is pure punctuation (FluidAudio issue #683). Always true
    /// without the splice guard.
    pub(crate) fn is_splice_safe(&self, id: usize) -> bool {
        !self.splice_guard || self.splice_safe.get(id).copied().unwrap_or(false)
    }

    /// Ids equal, or case-only variants of one piece (FluidAudio #706).
    pub(crate) fn same_ignoring_case(&self, left: usize, right: usize) -> bool {
        if left == right {
            return true;
        }
        match (
            self.case_canonical.get(&left),
            self.case_canonical.get(&right),
        ) {
            (Some(l), Some(r)) => l == r,
            _ => false,
        }
    }

    /// Joins pieces into text: `▁` becomes a space, `<unk>` and unknown ids
    /// are dropped, and the result is trimmed.
    pub(crate) fn detokenize(&self, ids: &[usize]) -> String {
        let joined: String = ids
            .iter()
            .map(|&id| self.piece(id))
            .filter(|piece| !piece.is_empty() && *piece != UNKNOWN)
            .collect();
        joined.replace(WORD_BOUNDARY, " ").trim().to_owned()
    }
}

/// The lower-case piece is canonical; without one, the smallest id.
fn case_canonical_ids(pieces: &[String]) -> HashMap<usize, usize> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (id, piece) in pieces.iter().enumerate() {
        groups.entry(piece.to_lowercase()).or_default().push(id);
    }
    let mut canonical = HashMap::new();
    for (folded, ids) in groups {
        if ids.len() < 2 {
            continue;
        }
        let Some(&first) = ids.iter().find(|&&id| pieces[id] == folded).or(ids.first()) else {
            continue;
        };
        for id in ids {
            canonical.insert(id, first);
        }
    }
    canonical
}

/// Starts with `▁` or a space.
pub(crate) fn starts_word(piece: &str) -> bool {
    piece.starts_with(WORD_BOUNDARY) || piece.starts_with(' ')
}

/// Drops one leading `▁` or space.
pub(crate) fn strip_word_boundary(piece: &str) -> &str {
    piece
        .strip_prefix(WORD_BOUNDARY)
        .or_else(|| piece.strip_prefix(' '))
        .unwrap_or(piece)
}

fn is_splice_safe_piece(piece: &str) -> bool {
    !piece.is_empty() && (starts_word(piece) || is_punctuation_only(piece))
}

/// Non-empty and made only of punctuation and symbols.
pub(crate) fn is_punctuation_only(piece: &str) -> bool {
    !piece.is_empty() && piece.chars().all(|c| is_punctuation(c) || is_symbol(c))
}

/// Unicode punctuation (general category P), for the scripts the Parakeet
/// vocabularies cover.
pub(crate) fn is_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation() && !is_ascii_symbol(c);
    }
    matches!(c,
        '¡' | '§' | '«' | '¶' | '·' | '»' | '¿'
        | '\u{2010}'..='\u{2027}'
        | '\u{2030}'..='\u{2043}'
        | '\u{2045}'..='\u{2051}'
        | '\u{2053}'..='\u{205E}'
        | '\u{3001}'..='\u{3003}'
        | '\u{3008}'..='\u{3011}'
        | '\u{3014}'..='\u{301F}'
        | '\u{30FB}'
        | '\u{FF01}'..='\u{FF03}'
        | '\u{FF05}'..='\u{FF0A}'
        | '\u{FF0C}'..='\u{FF0F}'
        | '\u{FF1A}' | '\u{FF1B}' | '\u{FF1F}' | '\u{FF20}'
        | '\u{FF3B}'..='\u{FF3D}'
        | '\u{FF3F}' | '\u{FF5B}' | '\u{FF5D}'
        | '\u{FF5F}'..='\u{FF65}')
}

/// Unicode symbols (general category S) the vocabularies use.
pub(crate) fn is_symbol(c: char) -> bool {
    if c.is_ascii() {
        return is_ascii_symbol(c);
    }
    matches!(c,
        '¢'..='¦' | '¨' | '©' | '¬' | '®'..='±' | '´' | '¸' | '×' | '÷'
        | '\u{2044}' | '\u{2052}'
        | '\u{20A0}'..='\u{20C0}'
        | '\u{2100}'..='\u{214F}'
        | '\u{2190}'..='\u{23FF}'
        | '\u{2500}'..='\u{27BF}'
        | '\u{FF04}' | '\u{FF0B}' | '\u{FF1C}'..='\u{FF1E}' | '\u{FF3E}' | '\u{FF40}'
        | '\u{FF5C}' | '\u{FF5E}' | '\u{FFE0}'..='\u{FFEE}')
}

fn is_ascii_symbol(c: char) -> bool {
    matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~')
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny vocabulary with Greendale words; id 9 is the blank.
    pub(crate) fn greendale() -> Vocabulary {
        Vocabulary::new(
            [
                "<unk>", "▁Troy", "▁Bar", "nes", "▁at", "▁Green", "dale", ".", "▁troy", "▁",
            ]
            .map(str::to_owned)
            .to_vec(),
        )
    }

    #[test]
    fn detokenize_joins_pieces() {
        struct Case {
            name: &'static str,
            ids: Vec<usize>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "words and continuation pieces",
                ids: vec![1, 2, 3, 4, 5, 6, 7],
                want: "Troy Barnes at Greendale.",
            },
            Case {
                name: "unknown pieces and ids dropped",
                ids: vec![0, 1, 0, 42],
                want: "Troy",
            },
            Case {
                name: "bare boundary trimmed",
                ids: vec![9, 5, 6, 9],
                want: "Greendale",
            },
            Case {
                name: "empty",
                ids: vec![],
                want: "",
            },
        ];
        let vocab = greendale();
        for tc in cases {
            assert_eq!(vocab.detokenize(&tc.ids), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn parse_accepts_object_and_array() {
        struct Case {
            name: &'static str,
            json: &'static str,
            blank: usize,
            want: Result<Vec<&'static str>, ()>,
        }
        let cases = [
            Case {
                name: "object keyed by id",
                json: r#"{"1": "▁Abed", "0": "<unk>", "2": "▁Nadir"}"#,
                blank: 3,
                want: Ok(vec!["<unk>", "▁Abed", "▁Nadir"]),
            },
            Case {
                name: "array",
                json: r#"["<unk>", "▁Annie"]"#,
                blank: 2,
                want: Ok(vec!["<unk>", "▁Annie"]),
            },
            Case {
                name: "token before blank missing",
                json: r#"{"0": "<unk>", "2": "▁Britta"}"#,
                blank: 3,
                want: Err(()),
            },
            Case {
                name: "not a vocabulary",
                json: r#""Señor Chang""#,
                blank: 1,
                want: Err(()),
            },
        ];
        for tc in cases {
            let got = Vocabulary::parse(tc.json, tc.blank)
                .map(|vocab| vocab.pieces.clone())
                .map_err(|_| ());
            let want = tc
                .want
                .map(|pieces| pieces.into_iter().map(str::to_owned).collect::<Vec<_>>());
            assert_eq!(got, want, "{}", tc.name);
        }
    }

    #[test]
    fn piece_classes() {
        let vocab = greendale();
        assert!(vocab.is_splice_safe(1), "word start");
        assert!(!vocab.is_splice_safe(3), "continuation");
        assert!(vocab.is_splice_safe(7), "punctuation");
        assert!(vocab.same_ignoring_case(1, 8), "Troy/troy");
        assert!(!vocab.same_ignoring_case(1, 2));
        assert!(is_punctuation_only("¿"));
        assert!(is_punctuation_only("€"));
        assert!(!is_punctuation_only("▁a"));
        assert_eq!(vocab.get(9), Some("▁"));
        assert_eq!(vocab.get(10), None);
        let unguarded = greendale().without_splice_guard();
        assert!(
            unguarded.is_splice_safe(3),
            "continuation without the guard"
        );
    }
}
