//! Guardrails that reject cleanup output that looks wrong (CLN-16..CLN-19).
//!
//! Conservative on purpose: a false positive costs the user their cleanup.

use std::fmt;

/// Only check shrinkage once the raw transcript has this many words (CLN-16).
const SHRINKAGE_MIN_RAW_WORDS: usize = 8;

/// Cleaned text may have twice the raw transcript's words plus this many.
/// Cleanup adds punctuation, not words, so more than that is a model that
/// answered the dictation or kept writing.
const GROWTH_SLACK_WORDS: usize = 10;

/// Phrases that legitimately shrink the output (CLN-17).
const CORRECTION_TRIGGERS: [&str; 9] = [
    "scratch that",
    "delete that",
    "never mind",
    "cancel",
    "actually",
    "no wait",
    "wait wait",
    "oops",
    "sorry",
];

/// Words that mean the user asked for formatting (CLN-18).
const FORMATTING_WORDS: [&str; 15] = [
    "header",
    "heading",
    "headings",
    "bullet",
    "bullets",
    "list",
    "lists",
    "bold",
    "italics",
    "italic",
    "underline",
    "underlines",
    "title",
    "titles",
    "numbered",
];

/// Which guardrail rejected the output. `Display` is the user-facing message
/// recorded in history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailViolation {
    /// Cleaned text has under half the raw transcript's words (CLN-16).
    Shrinkage {
        /// Words in the trimmed raw transcript.
        raw_words: usize,
        /// Words in the trimmed cleaned text.
        cleaned_words: usize,
    },
    /// Cleaned text has far more words than the raw transcript.
    Growth {
        /// Words in the trimmed raw transcript.
        raw_words: usize,
        /// Words in the trimmed cleaned text.
        cleaned_words: usize,
    },
    /// Cleaned text opens with Markdown structure nobody dictated (CLN-18).
    RogueMarkdown,
}

impl fmt::Display for GuardrailViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shrinkage { .. } | Self::Growth { .. } => f.write_str(
                "Post-processing output looked too different from the transcript, so Onda used the raw transcript.",
            ),
            Self::RogueMarkdown => f.write_str(
                "Post-processing produced unexpected Markdown formatting, so Onda used the raw transcript.",
            ),
        }
    }
}

impl std::error::Error for GuardrailViolation {}

/// Checks `cleaned` against `raw`; both are trimmed first. Shrinkage and
/// growth are checked before Markdown (CLN-19).
pub fn check_guardrails(raw: &str, cleaned: &str) -> Result<(), GuardrailViolation> {
    let (raw, cleaned) = (raw.trim(), cleaned.trim());
    check_shrinkage(raw, cleaned)?;
    check_growth(raw, cleaned)?;
    if begins_with_markdown(cleaned) && !contains_formatting_word(raw) {
        return Err(GuardrailViolation::RogueMarkdown);
    }
    Ok(())
}

fn check_shrinkage(raw: &str, cleaned: &str) -> Result<(), GuardrailViolation> {
    let raw_words = word_count(raw);
    if raw_words < SHRINKAGE_MIN_RAW_WORDS || contains_correction_trigger(raw) {
        return Ok(());
    }
    let cleaned_words = word_count(cleaned);
    // cleaned / raw < 0.5, in integers
    if cleaned_words * 2 < raw_words {
        return Err(GuardrailViolation::Shrinkage {
            raw_words,
            cleaned_words,
        });
    }
    Ok(())
}

fn check_growth(raw: &str, cleaned: &str) -> Result<(), GuardrailViolation> {
    let (raw_words, cleaned_words) = (word_count(raw), word_count(cleaned));
    if cleaned_words > raw_words * 2 + GROWTH_SLACK_WORDS {
        return Err(GuardrailViolation::Growth {
            raw_words,
            cleaned_words,
        });
    }
    Ok(())
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

// CLN-17: case-insensitive, with no letter right before or after; digits do
// not block a match
fn contains_correction_trigger(text: &str) -> bool {
    let lower = text.to_lowercase();
    CORRECTION_TRIGGERS.iter().any(|phrase| {
        lower.match_indices(phrase).any(|(start, _)| {
            let end = start + phrase.len();
            let left_ok = !lower[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphabetic);
            let right_ok = !lower[end..].chars().next().is_some_and(char::is_alphabetic);
            left_ok && right_ok
        })
    })
}

// CLN-18: words split on whitespace and punctuation (anything that is not a
// letter or digit)
fn contains_formatting_word(text: &str) -> bool {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| FORMATTING_WORDS.contains(&word))
}

// CLN-18: ATX heading, bullet, or ordered list on the first non-empty line.
// each marker needs what Markdown needs after it, so prose that opens with
// "3.5 percent", "2026. That year" or "#1 priority" is not taken for one.
fn begins_with_markdown(text: &str) -> bool {
    let Some(line) = text.split('\n').find(|line| !line.is_empty()) else {
        return false;
    };
    let line = line.trim();
    if ["- ", "* ", "+ "].iter().any(|p| line.starts_with(p)) {
        return true;
    }
    let after_hashes = line.trim_start_matches('#');
    let hashes = line.len() - after_hashes.len();
    if (1..=6).contains(&hashes) && (after_hashes.is_empty() || after_hashes.starts_with(' ')) {
        return true;
    }
    let after_digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits = line.len() - after_digits.len();
    (1..=3).contains(&digits) && after_digits.starts_with(". ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> String {
        (1..=n)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    struct Case {
        name: &'static str,
        raw: String,
        cleaned: String,
        want: Result<(), GuardrailViolation>,
    }

    fn case(
        name: &'static str,
        raw: &str,
        cleaned: &str,
        want: Result<(), GuardrailViolation>,
    ) -> Case {
        Case {
            name,
            raw: raw.to_owned(),
            cleaned: cleaned.to_owned(),
            want,
        }
    }

    const fn shrinkage(raw_words: usize, cleaned_words: usize) -> Result<(), GuardrailViolation> {
        Err(GuardrailViolation::Shrinkage {
            raw_words,
            cleaned_words,
        })
    }

    const MARKDOWN: Result<(), GuardrailViolation> = Err(GuardrailViolation::RogueMarkdown);

    #[test]
    fn check_guardrails_cln_16_to_cln_19() {
        let greendale = "Troy and Abed are studying at Greendale Community College for their Spanish exam with Señor Chang and they hope to pass the class this semester.";
        let over_transformed = "This is a test of the transcribing capabilities. I am going to say some things and we will see what the result is. Newline. Now I want to see if the commands are working.";
        let mut cases = vec![
            case(
                "clean output passes",
                "hello world this is a test transcript with enough words",
                "Hello world, this is a test transcript with enough words.",
                Ok(()),
            ),
            case(
                "CLN-16 below minimum raw words",
                "one two three four",
                "one two",
                Ok(()),
            ),
            case(
                "CLN-16 exact minimum raw words triggers",
                &words(8),
                "word1 word2",
                shrinkage(8, 2),
            ),
            case("CLN-16 exact ratio passes", &words(8), &words(4), Ok(())),
            case(
                "CLN-16 summarized transcript",
                greendale,
                "Studying at Greendale.",
                shrinkage(25, 3),
            ),
            case(
                "CLN-16 empty output of a long transcript",
                greendale,
                "",
                shrinkage(25, 0),
            ),
            case(
                "CLN-17 substring of a trigger does not suppress",
                &format!("{} factually", words(7)),
                "word1",
                shrinkage(8, 1),
            ),
            case(
                "CLN-17 digits do not block a trigger",
                &format!("{} 2sorry2 word8", words(6)),
                "word8",
                Ok(()),
            ),
            case(
                "CLN-17 trigger matches regardless of case",
                &format!("{} SCRATCH THAT word8", words(6)),
                "word8",
                Ok(()),
            ),
            case(
                "CLN-18 heading is rejected",
                "hello world this is a test",
                "# Hello World",
                MARKDOWN,
            ),
            case(
                "CLN-18 rogue heading on a short transcript",
                "the trojan horse is a great plan",
                "# The Trojan Horse Is A Great Plan",
                MARKDOWN,
            ),
            case(
                "CLN-18 explicit header command allows markdown",
                "header shopping list",
                "# Shopping List",
                Ok(()),
            ),
            case(
                "CLN-18 formatting word next to punctuation counts",
                "make this bold, please",
                "**This**",
                Ok(()),
            ),
            case(
                "CLN-18 formatting word inside another word does not count",
                "a boldly listed plan",
                "- plan",
                MARKDOWN,
            ),
            case(
                "CLN-18 plain prose passes",
                "hello world this is a test",
                "Hello world, this is a test.",
                Ok(()),
            ),
            case(
                "CLN-18 a number in a sentence passes",
                "the answer is forty two",
                "The answer is 42.",
                Ok(()),
            ),
            case(
                "an answer in place of the dictation is growth",
                "what time does the study group meet",
                &words(25),
                Err(GuardrailViolation::Growth {
                    raw_words: 7,
                    cleaned_words: 25,
                }),
            ),
            case(
                "twice the words plus the slack still passes",
                "what time does the study group meet",
                &words(24),
                Ok(()),
            ),
            case(
                "a one-word dictation may gain a few words",
                "thx",
                "Thanks.",
                Ok(()),
            ),
            case(
                "CLN-19 shrinkage is checked before markdown",
                over_transformed,
                "# Testing Transcription Capabilities\nSee results. Commands: working.",
                shrinkage(34, 8),
            ),
            case(
                "CLN-19 inputs are trimmed",
                &format!("  {}  ", words(8)),
                &format!("\n {} \n", words(4)),
                Ok(()),
            ),
        ];
        for trigger in CORRECTION_TRIGGERS {
            cases.push(Case {
                name: "CLN-17 correction trigger suppresses shrinkage",
                raw: format!("{} {trigger} word8", words(7)),
                cleaned: "word8".to_owned(),
                want: Ok(()),
            });
        }
        for word in FORMATTING_WORDS {
            cases.push(Case {
                name: "CLN-18 formatting word allows markdown",
                raw: format!("make this a {word} please"),
                cleaned: "# Formatted Text".to_owned(),
                want: Ok(()),
            });
        }
        for tc in cases {
            assert_eq!(
                check_guardrails(&tc.raw, &tc.cleaned),
                tc.want,
                "{}: raw {:?}, cleaned {:?}",
                tc.name,
                tc.raw,
                tc.cleaned
            );
        }
    }

    #[test]
    fn begins_with_markdown_cln_18() {
        struct Case {
            name: &'static str,
            text: &'static str,
            want: bool,
        }
        let cases = [
            Case {
                name: "heading",
                text: "# heading",
                want: true,
            },
            Case {
                name: "subheading",
                text: "## subheading",
                want: true,
            },
            Case {
                name: "dash bullet",
                text: "- bullet",
                want: true,
            },
            Case {
                name: "star bullet",
                text: "* bullet",
                want: true,
            },
            Case {
                name: "plus bullet",
                text: "+ bullet",
                want: true,
            },
            Case {
                name: "ordered",
                text: "1. ordered",
                want: true,
            },
            Case {
                name: "multi-digit ordered",
                text: "42. ordered",
                want: true,
            },
            Case {
                name: "plain text",
                text: "plain text",
                want: false,
            },
            Case {
                name: "bare number",
                text: "42",
                want: false,
            },
            Case {
                name: "a number that ends the sentence",
                text: "42.",
                want: false,
            },
            Case {
                name: "decimal number",
                text: "3.5 percent of students",
                want: false,
            },
            Case {
                name: "year that ends a sentence",
                text: "2026. That was the year Troy left.",
                want: false,
            },
            Case {
                name: "number sign",
                text: "#1 priority is the Spanish exam",
                want: false,
            },
            Case {
                name: "hashtag",
                text: "#sixseasonsandamovie",
                want: false,
            },
            Case {
                name: "bare hash",
                text: "#",
                want: true,
            },
            Case {
                name: "bare dash",
                text: "-",
                want: false,
            },
            Case {
                name: "dash without space",
                text: "-troy",
                want: false,
            },
            Case {
                name: "dot without digits",
                text: ".hidden",
                want: false,
            },
            Case {
                name: "empty",
                text: "",
                want: false,
            },
            Case {
                name: "heading after blank lines",
                text: "\n\n# heading after blanks",
                want: true,
            },
            Case {
                name: "indented bullet",
                text: "   - bullet",
                want: true,
            },
            Case {
                name: "line of only spaces is not markdown",
                text: "   \n# heading",
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                begins_with_markdown(tc.text),
                tc.want,
                "{}: {:?}",
                tc.name,
                tc.text
            );
        }
    }

    #[test]
    fn contains_correction_trigger_cln_17() {
        struct Case {
            name: &'static str,
            text: &'static str,
            want: bool,
        }
        let cases = [
            Case {
                name: "leading trigger",
                text: "oops I did it again",
                want: true,
            },
            Case {
                name: "uppercase",
                text: "OOPS",
                want: true,
            },
            Case {
                name: "surrounded by punctuation",
                text: "...cancel...",
                want: true,
            },
            Case {
                name: "between commas",
                text: "well, sorry, I forgot",
                want: true,
            },
            Case {
                name: "multi-word trigger",
                text: "scratch that last part",
                want: true,
            },
            Case {
                name: "later occurrence matches",
                text: "cancelled, then cancel",
                want: true,
            },
            Case {
                name: "inside factually",
                text: "factually correct",
                want: false,
            },
            Case {
                name: "inside cancelled",
                text: "cancelled the flight",
                want: false,
            },
            Case {
                name: "inside sorrowful",
                text: "sorrowful",
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(contains_correction_trigger(tc.text), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn word_count_cln_19() {
        struct Case {
            name: &'static str,
            text: &'static str,
            want: usize,
        }
        let cases = [
            Case {
                name: "empty",
                text: "",
                want: 0,
            },
            Case {
                name: "one",
                text: "one",
                want: 1,
            },
            Case {
                name: "three",
                text: "one two three",
                want: 3,
            },
            Case {
                name: "padded",
                text: "  one  two  ",
                want: 2,
            },
            Case {
                name: "mixed whitespace",
                text: "one\ntwo\tthree",
                want: 3,
            },
        ];
        for tc in cases {
            assert_eq!(word_count(tc.text), tc.want, "{}", tc.name);
        }
    }
}
