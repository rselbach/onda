//! Fits a dictation to the text just before the caret (OUT-8..OUT-11), so
//! bursts of dictation read "Done. Next thing" rather than "Done.Next thing".

/// Words a speech model capitalizes at the start of every utterance that
/// usually continue a sentence instead (OUT-10).
const CONTINUATION_WORDS: [&str; 18] = [
    "a", "an", "and", "as", "because", "but", "for", "if", "nor", "or", "so", "the", "then",
    "though", "to", "when", "while", "yet",
];

/// What the focused field holds, which decides how a dictation is fitted to
/// the text before the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Text the user writes. The dictation takes its case and its spacing
    /// from it.
    Prose,
    /// A terminal's screen. A prompt or a program's output comes before the
    /// caret and says nothing about sentences, so the dictation keeps its
    /// case and takes only its spacing.
    Terminal,
}

/// Returns `text` adjusted to follow `preceding`, the text before the caret
/// of a `field`.
///
/// - OUT-8: an empty `preceding` (start of field) or empty `text` returns
///   `text` unchanged.
/// - OUT-9: at a sentence start the first letter is uppercased.
/// - OUT-10: mid-sentence, a capitalized continuation word ("And", "The") is
///   lowercased.
/// - OUT-11: one leading space is inserted when the two sides would touch,
///   unless either side is in a script written without spaces.
///
/// OUT-9 and OUT-10 apply to [`FieldKind::Prose`] only.
pub fn join(text: &str, preceding: &str, field: FieldKind) -> String {
    let Some(last) = preceding.chars().next_back() else {
        return text.to_owned();
    };
    if text.is_empty() {
        return text.to_owned();
    }

    let mut joined = match field {
        FieldKind::Prose => set_initial_case(text, is_sentence_start(preceding)),
        FieldKind::Terminal => text.to_owned(),
    };
    let needs_space = !last.is_whitespace()
        && !"([{“‘".contains(last)
        && !is_written_without_spaces(last)
        && joined.chars().next().is_some_and(|first| {
            !first.is_whitespace()
                && !",.?!;:)]}”’".contains(first)
                && !is_written_without_spaces(first)
        });
    if needs_space {
        joined.insert(0, ' ');
    }
    joined
}

// Chinese, Japanese, Thai, Lao, Khmer, and Burmese text, with its own
// punctuation, puts no space between words or sentences. Korean does, so
// Hangul is not here.
/// Whether `c` belongs to a script written without spaces between words.
pub fn is_written_without_spaces(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0E00..=0x0EFF      // Thai, Lao
        | 0x1000..=0x109F    // Myanmar
        | 0x1780..=0x17FF    // Khmer
        | 0x2E80..=0x312F    // CJK radicals and punctuation, kana, bopomofo
        | 0x3190..=0x9FFF    // kana extensions, CJK ideographs
        | 0xF900..=0xFAFF    // CJK compatibility ideographs
        | 0xFF00..=0xFF9F    // fullwidth forms, halfwidth katakana
        | 0x20000..=0x3FFFF // CJK extensions
    )
}

fn is_sentence_start(preceding: &str) -> bool {
    let mut rest = preceding
        .chars()
        .rev()
        // any space within the line: a browser ends an editable line with
        // a no-break space
        .skip_while(|c| c.is_whitespace() && !is_newline(*c))
        .peekable();
    let Some(&last) = rest.peek() else {
        return true;
    };
    if is_newline(last) {
        return true;
    }
    rest.find(|c| !"'\")]}”’".contains(*c))
        .is_some_and(|c| ".?!".contains(c))
}

// mirrors Swift's Character.isNewline
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn set_initial_case(text: &str, uppercase: bool) -> String {
    let Some((index, first)) = text.char_indices().find(|(_, c)| {
        !c.is_whitespace() && !matches!(c, '"' | '\'' | '“' | '‘' | '(' | '[' | '{')
    }) else {
        return text.to_owned();
    };
    if !first.is_alphabetic() {
        return text.to_owned();
    }
    let rest = &text[index + first.len_utf8()..];
    let replacement: String = match (uppercase, first) {
        (true, c) if c.is_lowercase() => c.to_uppercase().collect(),
        (false, c) if c.is_uppercase() && starts_with_continuation_word(&text[index..]) => {
            c.to_lowercase().collect()
        }
        _ => return text.to_owned(),
    };
    format!("{}{replacement}{rest}", &text[..index])
}

fn starts_with_continuation_word(text: &str) -> bool {
    let end = text
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic() && *c != '\'')
        .map_or(text.len(), |(i, _)| i);
    let slice = &text[..end];
    let mut chars = slice.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_uppercase() || chars.any(|c| c.is_alphabetic() && !c.is_lowercase()) {
        return false;
    }
    let word = slice.to_lowercase();
    CONTINUATION_WORDS.contains(&word.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        name: &'static str,
        preceding: &'static str,
        text: &'static str,
        want: &'static str,
    }

    #[test]
    fn join_out_8_to_out_11() {
        let cases = [
            Case {
                name: "OUT-8 start of field",
                preceding: "",
                text: "hello there.",
                want: "hello there.",
            },
            Case {
                name: "OUT-8 empty text",
                preceding: "Done.",
                text: "",
                want: "",
            },
            Case {
                name: "OUT-9 after a sentence",
                preceding: "Done.",
                text: "Next thing.",
                want: " Next thing.",
            },
            Case {
                name: "OUT-9 after a sentence lowercased",
                preceding: "Done.",
                text: "next thing.",
                want: " Next thing.",
            },
            Case {
                name: "OUT-9 after a question",
                preceding: "Ready?",
                text: "yes.",
                want: " Yes.",
            },
            Case {
                name: "OUT-9 after an exclamation",
                preceding: "Cool cool cool!",
                text: "streets ahead.",
                want: " Streets ahead.",
            },
            Case {
                name: "OUT-9 after a space",
                preceding: "Done. ",
                text: "next thing.",
                want: "Next thing.",
            },
            Case {
                name: "OUT-9 after trailing tabs and spaces",
                preceding: "Done.\t ",
                text: "next thing.",
                want: "Next thing.",
            },
            Case {
                name: "OUT-9 after a no-break space",
                preceding: "Enroll at Greendale.\u{a0}",
                text: "and bring Troy.",
                want: "And bring Troy.",
            },
            Case {
                name: "OUT-10 mid-sentence before a no-break space",
                preceding: "Troy\u{a0}",
                text: "And Abed.",
                want: "and Abed.",
            },
            Case {
                name: "OUT-9 after a newline",
                preceding: "Dear Troy,\n",
                text: "thanks for",
                want: "Thanks for",
            },
            Case {
                name: "OUT-9 whitespace-only preceding is a sentence start",
                preceding: "   ",
                text: "troy",
                want: "Troy",
            },
            Case {
                name: "OUT-9 after a closing quote",
                preceding: "He said \"go.\"",
                text: "then left.",
                want: " Then left.",
            },
            Case {
                name: "OUT-9 after a closing curly quote and bracket",
                preceding: "(He said “go.”)",
                text: "then left.",
                want: " Then left.",
            },
            Case {
                name: "OUT-9 leading punctuation before the first letter",
                preceding: "Done.",
                text: "\"troy\" said",
                want: " \"Troy\" said",
            },
            Case {
                name: "OUT-10 mid-sentence continuation word",
                preceding: "I tried it",
                text: "And it worked.",
                want: " and it worked.",
            },
            Case {
                name: "OUT-10 mid-sentence other word",
                preceding: "I asked",
                text: "Troy Barnes.",
                want: " Troy Barnes.",
            },
            Case {
                name: "OUT-10 after a comma",
                preceding: "Well,",
                text: "The plan works.",
                want: " the plan works.",
            },
            Case {
                name: "OUT-10 contraction is not a continuation word",
                preceding: "Well",
                text: "It's fine.",
                want: " It's fine.",
            },
            Case {
                name: "OUT-10 word that only starts like one",
                preceding: "We met",
                text: "Andre.",
                want: " Andre.",
            },
            Case {
                name: "OUT-11 after an opening paren",
                preceding: "See (",
                text: "Greendale",
                want: "Greendale",
            },
            Case {
                name: "OUT-11 after an opening curly quote",
                preceding: "He said “",
                text: "Greendale",
                want: "Greendale",
            },
            Case {
                name: "OUT-11 before punctuation",
                preceding: "Troy",
                text: ", Abed",
                want: ", Abed",
            },
            Case {
                name: "OUT-11 Japanese takes no space",
                preceding: "こんにちは",
                text: "世界",
                want: "世界",
            },
            Case {
                name: "OUT-11 after Japanese punctuation",
                preceding: "行きます。",
                text: "ありがとう",
                want: "ありがとう",
            },
            Case {
                name: "OUT-11 Chinese takes no space",
                preceding: "你好",
                text: "世界",
                want: "世界",
            },
            Case {
                name: "OUT-11 Thai takes no space",
                preceding: "สวัสดี",
                text: "ครับ",
                want: "ครับ",
            },
            Case {
                name: "OUT-11 Latin after Japanese takes no space",
                preceding: "場所は",
                text: "Greendale",
                want: "Greendale",
            },
            Case {
                name: "OUT-11 Korean takes a space",
                preceding: "안녕하세요",
                text: "세계",
                want: " 세계",
            },
            Case {
                name: "OUT-11 text already starts with a space",
                preceding: "Troy",
                text: " and Abed",
                want: " and Abed",
            },
            Case {
                name: "leading digit does not capitalize later word",
                preceding: "Done.",
                text: "42 things happened.",
                want: " 42 things happened.",
            },
            Case {
                name: "leading currency does not capitalize later word",
                preceding: "Done.",
                text: "$50 for Troy.",
                want: " $50 for Troy.",
            },
            Case {
                name: "leading number before period does not lowercase next sentence",
                preceding: "Count to",
                text: "5. And then 6.",
                want: " 5. And then 6.",
            },
            Case {
                name: "all-caps acronym matching continuation word keeps case",
                preceding: "I live in",
                text: "OR and WA",
                want: " OR and WA",
            },
        ];
        for tc in cases {
            assert_eq!(
                join(tc.text, tc.preceding, FieldKind::Prose),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn a_terminal_gives_spacing_and_no_case() {
        let cases = [
            Case {
                name: "after a prompt",
                preceding: "troy@greendale:~$ ",
                text: "And so it begins.",
                want: "And so it begins.",
            },
            Case {
                name: "after a word",
                preceding: "troy@greendale:~$ echo enroll",
                text: "And so it begins.",
                want: " And so it begins.",
            },
            Case {
                name: "after a full stop",
                preceding: "troy@greendale:~$ echo done.",
                text: "next thing",
                want: " next thing",
            },
            Case {
                name: "after a space",
                preceding: "troy@greendale:~$ echo ",
                text: "The plan works",
                want: "The plan works",
            },
            Case {
                name: "an empty screen",
                preceding: "",
                text: "The plan works",
                want: "The plan works",
            },
        ];
        for tc in cases {
            assert_eq!(
                join(tc.text, tc.preceding, FieldKind::Terminal),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
