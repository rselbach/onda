//! Instant replacements: user rules that swap phrases the speech model keeps
//! getting wrong.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::joiner::is_written_without_spaces;

/// Stable identifier of a rule, stored as a UUID string (REP-7).
///
/// Domain code cannot generate one (no randomness, ARCH-2); the caller mints
/// it when the user presses "Add Rule".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(pub String);

/// One replacement rule: trigger phrases and the text that replaces them.
///
/// Serializes to the stored shape
/// `{"id": "<uuid>", "triggers": [...], "replacement": "..."}` (REP-7).
/// Rows are kept exactly as the user typed them; normalization happens when
/// rules are applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementRule {
    /// Stable identifier.
    pub id: RuleId,
    /// Phrases that trigger the rule, as entered.
    pub triggers: Vec<String>,
    /// Literal replacement; empty (after trimming) deletes the phrase (REP-5).
    pub replacement: String,
}

/// Rules read from settings, plus the first decode error when part of the
/// stored value was corrupt (REP-7).
///
/// Rows decode one by one, so a single bad row costs only itself. A value
/// that is not a list reads as empty. Dictation never breaks either way; the
/// caller logs `error`.
#[derive(Debug)]
pub struct DecodedRules {
    /// The rows that decoded, in stored order.
    pub rules: Vec<ReplacementRule>,
    /// Why the first unreadable row, or the whole value, failed to decode.
    pub error: Option<serde_json::Error>,
}

/// Decodes the `replacementEntries` setting leniently (REP-7).
///
/// A missing key or JSON `null` is an empty list without error.
pub fn decode_rules(value: Option<&serde_json::Value>) -> DecodedRules {
    let mut decoded = DecodedRules {
        rules: Vec::new(),
        error: None,
    };
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return decoded;
    };
    let rows = match Vec::<serde_json::Value>::deserialize(value) {
        Ok(rows) => rows,
        Err(error) => {
            decoded.error = Some(error);
            return decoded;
        }
    };
    for row in rows {
        match ReplacementRule::deserialize(row) {
            Ok(rule) => decoded.rules.push(rule),
            Err(error) => {
                decoded.error.get_or_insert(error);
            }
        }
    }
    decoded
}

/// Parses the editor's comma-separated trigger field into trimmed, non-empty
/// triggers, keeping case (REP-9). A half-typed "troy, " yields `["troy"]`.
pub fn parse_triggers(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Formats stored triggers for the editor's trigger field (REP-9).
pub fn trigger_text(triggers: &[String]) -> String {
    triggers.join(", ")
}

/// The rows worth storing, in order and unmodified (REP-12).
///
/// Fully blank rows (an abandoned "Add Rule") are dropped; partial rows are
/// kept so a half-written rule survives closing the window.
pub fn rules_to_persist(rules: &[ReplacementRule]) -> Vec<ReplacementRule> {
    rules
        .iter()
        .filter(|r| !is_blank(&r.replacement) || r.triggers.iter().any(|t| !is_blank(t)))
        .cloned()
        .collect()
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// Applies `rules` to `text` (REP-1..REP-6).
///
/// Matching is case-insensitive, literal, and on word boundaries. The text
/// is scanned once, left to right; at each position the longest match in the
/// transcript wins, ties keep the earlier rule, and replaced text is never
/// rescanned. An empty replacement deletes the phrase and tidies up the
/// spacing, punctuation, and capitalization it leaves behind.
pub fn apply(text: &str, rules: &[ReplacementRule]) -> String {
    let compiled = compile(rules);
    if compiled.is_empty() {
        return text.to_owned();
    }

    // positions are grapheme indices, so a match never splits a character
    // the user sees as one
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut out = Resolved {
        text: String::with_capacity(text.len()),
        capitalize_next: false,
    };
    let mut pos = 0;
    while pos < graphemes.len() {
        let Some((end, replacement)) = longest_match(&graphemes, pos, &compiled) else {
            out.append(graphemes[pos]);
            pos += 1;
            continue;
        };
        if !replacement.is_empty() {
            out.append(replacement);
            pos = end;
            continue;
        }
        pos = out.delete(&graphemes, pos, end);
    }
    out.text
}

struct CompiledRule<'a> {
    folded_trigger: Vec<char>,
    replacement: &'a str,
}

// REP-1: triggers trimmed and lowercased, empty ones dropped, duplicates keep
// the first occurrence; REP-4: replacements trimmed.
fn compile(rules: &[ReplacementRule]) -> Vec<CompiledRule<'_>> {
    let mut seen = HashSet::new();
    let mut compiled = Vec::new();
    for rule in rules {
        let replacement = rule.replacement.trim();
        for trigger in &rule.triggers {
            let normalized = trigger.trim().to_lowercase();
            if normalized.is_empty() {
                continue;
            }
            let folded_trigger = fold(&normalized);
            if !seen.insert(normalized) {
                continue;
            }
            compiled.push(CompiledRule {
                folded_trigger,
                replacement,
            });
        }
    }
    compiled
}

// approximates Unicode full case folding with std only: "ß" folds to "ss",
// so "strasse" matches "straße" (REP-3)
fn fold(text: &str) -> Vec<char> {
    text.chars().flat_map(fold_char).collect()
}

fn fold_char(c: char) -> impl Iterator<Item = char> {
    c.to_uppercase().flat_map(char::to_lowercase)
}

// returns the end (exclusive grapheme index) and replacement of the longest
// rule matching at `pos`, measured in the transcript (REP-3)
fn longest_match<'a>(
    graphemes: &[&str],
    pos: usize,
    rules: &[CompiledRule<'a>],
) -> Option<(usize, &'a str)> {
    if pos > 0 && is_word_grapheme(graphemes[pos - 1]) {
        return None;
    }
    let mut best: Option<(usize, &'a str)> = None;
    for rule in rules {
        let Some(end) = match_end(graphemes, pos, &rule.folded_trigger) else {
            continue;
        };
        if end < graphemes.len() && is_word_grapheme(graphemes[end]) {
            continue;
        }
        if best.is_none_or(|(best_end, _)| end > best_end) {
            best = Some((end, rule.replacement));
        }
    }
    best
}

// a match must consume whole graphemes and end exactly where the folded
// trigger ends
fn match_end(graphemes: &[&str], pos: usize, folded_trigger: &[char]) -> Option<usize> {
    let mut remaining = folded_trigger;
    for (offset, grapheme) in graphemes[pos..].iter().enumerate() {
        for c in grapheme.chars().flat_map(fold_char) {
            let (first, rest) = remaining.split_first()?;
            if *first != c {
                return None;
            }
            remaining = rest;
        }
        if remaining.is_empty() {
            return Some(pos + offset + 1);
        }
    }
    None
}

// REP-2: letters and digits block a match, except letters of scripts
// written without spaces, where a word can start or end at any letter
fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(|c| {
        let unspaced_letter = c.is_alphabetic()
            && is_written_without_spaces(c)
            && !('\u{FF01}'..='\u{FF5E}').contains(&c);
        c.is_alphanumeric() && !unspaced_letter
    })
}

fn is_horizontal_space(grapheme: &str) -> bool {
    grapheme == " " || grapheme == "\t"
}

fn is_clause_punctuation(grapheme: &str) -> bool {
    matches!(grapheme, "." | "!" | "?" | "," | ";" | ":")
}

struct Resolved {
    text: String,
    // a deleted sentence-initial phrase hands its capital to the next word
    capitalize_next: bool,
}

impl Resolved {
    fn append(&mut self, segment: &str) {
        if !self.capitalize_next {
            self.text.push_str(segment);
            return;
        }
        let Some((index, c)) = segment.char_indices().find(|(_, c)| !c.is_whitespace()) else {
            self.text.push_str(segment);
            return;
        };
        self.capitalize_next = false;
        self.text.push_str(&segment[..index]);
        if c.is_lowercase() && self.is_at_sentence_start() {
            self.text.extend(c.to_uppercase());
        } else {
            self.text.push(c);
        }
        self.text.push_str(&segment[index + c.len_utf8()..]);
    }

    // REP-5: drops graphemes[start..end] and returns where scanning resumes
    fn delete(&mut self, graphemes: &[&str], start: usize, end: usize) -> usize {
        let skip = |from: usize, pred: fn(&str) -> bool| {
            graphemes[from..]
                .iter()
                .position(|g| !pred(g))
                .map_or(graphemes.len(), |i| from + i)
        };
        let ends_at = |i: usize| i == graphemes.len() || graphemes[i] == "\n";
        let after_spaces = skip(end, is_horizontal_space);

        if self.is_at_sentence_start() {
            let phrase_is_capitalized = graphemes[start..end]
                .iter()
                .flat_map(|g| g.chars())
                .find(|c| c.is_alphabetic())
                .is_some_and(char::is_uppercase);
            if phrase_is_capitalized {
                self.capitalize_next = true;
            }
            let resume = skip(skip(end, is_clause_punctuation), is_horizontal_space);
            if ends_at(resume) {
                self.trim_trailing_space();
            }
            return resume;
        }

        let followed_by_punctuation =
            end < graphemes.len() && is_clause_punctuation(graphemes[end]);
        let ends_line = ends_at(after_spaces);
        if followed_by_punctuation || ends_line {
            self.trim_trailing_space();
            if self.text.ends_with(',') {
                self.text.pop();
            }
            return if ends_line { after_spaces } else { end };
        }
        if self.text.ends_with([' ', '\t']) {
            return after_spaces;
        }
        end
    }

    fn is_at_sentence_start(&self) -> bool {
        self.text
            .chars()
            .rev()
            .find(|c| *c != ' ' && *c != '\t')
            .is_none_or(|c| matches!(c, '.' | '!' | '?' | '\n'))
    }

    fn trim_trailing_space(&mut self) {
        let trimmed = self.text.trim_end_matches([' ', '\t']).len();
        self.text.truncate(trimmed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(triggers: &[&str], replacement: &str) -> ReplacementRule {
        ReplacementRule {
            id: RuleId(String::new()),
            triggers: triggers.iter().map(|t| (*t).to_owned()).collect(),
            replacement: replacement.to_owned(),
        }
    }

    fn rule_with_id(id: &str, triggers: &[&str], replacement: &str) -> ReplacementRule {
        ReplacementRule {
            id: RuleId(id.to_owned()),
            ..rule(triggers, replacement)
        }
    }

    struct ApplyCase {
        name: &'static str,
        rules: Vec<ReplacementRule>,
        text: &'static str,
        want: &'static str,
    }

    #[test]
    fn apply_rep_1_to_rep_6() {
        let cases = [
            ApplyCase {
                name: "REP-6 no rules leaves text untouched",
                rules: vec![],
                text: "troy and abed",
                want: "troy and abed",
            },
            ApplyCase {
                name: "REP-6 rules without triggers leave text untouched",
                rules: vec![rule(&[], "Abed Nadir")],
                text: "troy and  abed ",
                want: "troy and  abed ",
            },
            ApplyCase {
                name: "REP-1 case-insensitive literal replacement",
                rules: vec![rule(&["troy"], "Troy Barnes")],
                text: "I saw TROY and troy and Troy",
                want: "I saw Troy Barnes and Troy Barnes and Troy Barnes",
            },
            ApplyCase {
                name: "REP-1 uppercase trigger is lowercased",
                rules: vec![rule(&["  GREENDALE  "], "Greendale")],
                text: "welcome to greendale",
                want: "welcome to Greendale",
            },
            ApplyCase {
                name: "REP-1 regex metacharacters are literal",
                rules: vec![rule(&["a.b"], "X")],
                text: "a.b acb",
                want: "X acb",
            },
            ApplyCase {
                name: "REP-1 multi-word trigger",
                rules: vec![rule(&["señor chang"], "Señor Chang")],
                text: "hello señor chang welcome",
                want: "hello Señor Chang welcome",
            },
            ApplyCase {
                name: "REP-1 empty triggers are ignored",
                rules: vec![rule(&["", "  "], "ShouldNotApply"), rule(&["abed"], "Abed")],
                text: "troy and abed",
                want: "troy and Abed",
            },
            ApplyCase {
                name: "REP-1 duplicate triggers keep the first replacement",
                rules: vec![rule(&["troy", "TROY"], "First"), rule(&["troy"], "Second")],
                text: "troy",
                want: "First",
            },
            ApplyCase {
                name: "REP-1 several triggers in one rule share the replacement",
                rules: vec![rule(&["troy", "abed", "annie"], "Human Being")],
                text: "troy met abed and annie",
                want: "Human Being met Human Being and Human Being",
            },
            ApplyCase {
                name: "REP-2 punctuation next to the trigger matches",
                rules: vec![rule(&["troy"], "Troy")],
                text: "(troy), troy. troy!",
                want: "(Troy), Troy. Troy!",
            },
            ApplyCase {
                name: "REP-2 substring of a larger word does not match",
                rules: vec![rule(&["troy"], "Troy")],
                text: "TroyBarnes and GreendaleTroy and troy",
                want: "TroyBarnes and GreendaleTroy and Troy",
            },
            ApplyCase {
                name: "REP-2 digits block a match",
                rules: vec![rule(&["troy"], "Troy")],
                text: "troy2 2troy troy",
                want: "troy2 2troy Troy",
            },
            ApplyCase {
                name: "REP-2 multi-word trigger rejects partial overlap",
                rules: vec![rule(&["señor chang"], "Señor Chang")],
                text: "welcome to señor changnesia",
                want: "welcome to señor changnesia",
            },
            ApplyCase {
                name: "REP-2 string edges are boundaries",
                rules: vec![rule(&["troy"], "Troy")],
                text: "troy troy",
                want: "Troy Troy",
            },
            ApplyCase {
                name: "REP-2 whole transcript is the trigger",
                rules: vec![rule(&["troy"], "Troy")],
                text: "troy",
                want: "Troy",
            },
            ApplyCase {
                name: "REP-2 combining accent after the trigger blocks a match",
                rules: vec![rule(&["cafe"], "Coffee")],
                text: "cafe\u{301} cafe",
                want: "cafe\u{301} Coffee",
            },
            ApplyCase {
                name: "REP-3 longest trigger wins at the same position",
                rules: vec![
                    rule(&["troy"], "Troy"),
                    rule(&["troy barnes"], "Troy Barnes"),
                ],
                text: "troy barnes is here",
                want: "Troy Barnes is here",
            },
            ApplyCase {
                name: "REP-3 replacements do not chain",
                rules: vec![rule(&["troy"], "abed"), rule(&["abed"], "annie")],
                text: "troy at greendale",
                want: "abed at greendale",
            },
            ApplyCase {
                name: "REP-3 replaced text is not rescanned",
                rules: vec![rule(&["ab"], "abab"), rule(&["abab"], "X")],
                text: "ab cd",
                want: "abab cd",
            },
            ApplyCase {
                name: "REP-3 case folding matches across lengths",
                rules: vec![rule(&["strasse"], "Street")],
                text: "on the STRAßE today",
                want: "on the Street today",
            },
            ApplyCase {
                name: "REP-3 longest match is measured in the transcript",
                rules: vec![rule(&["ssssss"], "Chang"), rule(&["ßßß xy"], "Greendale")],
                text: "ßßß xy",
                want: "Greendale",
            },
            ApplyCase {
                name: "REP-3 tie keeps the earlier rule",
                rules: vec![rule(&["straße"], "First"), rule(&["strasse"], "Second")],
                text: "strasse",
                want: "First",
            },
            ApplyCase {
                name: "REP-4 replacement is literal",
                rules: vec![rule(&["greendale"], "GREENDALE!!")],
                text: "greendale rules",
                want: "GREENDALE!! rules",
            },
            ApplyCase {
                name: "REP-4 replacement is trimmed",
                rules: vec![rule(&["greendale"], "  Greendale Community College\n")],
                text: "go greendale",
                want: "go Greendale Community College",
            },
            ApplyCase {
                name: "REP-2 matches inside text written without spaces",
                rules: vec![rule(&["東京"], "Tokyo"), rule(&["abed"], "Abed")],
                text: "私は東京に行くabedと",
                want: "私はTokyoに行くAbedと",
            },
            ApplyCase {
                name: "REP-2 digits and fullwidth Latin still block",
                rules: vec![rule(&["troy"], "Troy")],
                text: "troy１ ๑troy Ａtroy",
                want: "troy１ ๑troy Ａtroy",
            },
        ];
        for tc in cases {
            assert_eq!(apply(tc.text, &tc.rules), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn apply_rep_5_deletion_tidies_up() {
        let rules = [
            rule(&["um", "uh"], ""),
            rule(&["you know"], " "),
            rule(&["abed"], "Abed"),
        ];
        struct Case {
            name: &'static str,
            text: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "between words",
                text: "I um think so.",
                want: "I think so.",
            },
            Case {
                name: "between commas",
                text: "I, um, think so.",
                want: "I, think so.",
            },
            Case {
                name: "consecutive fillers",
                text: "I, um, uh, think so.",
                want: "I, think so.",
            },
            Case {
                name: "sentence start passes capital on",
                text: "Um, the build passed.",
                want: "The build passed.",
            },
            Case {
                name: "consecutive at sentence start",
                text: "Um, uh, the build passed.",
                want: "The build passed.",
            },
            Case {
                name: "lowercase sentence start stays lowercase",
                text: "um, the build passed.",
                want: "the build passed.",
            },
            Case {
                name: "before final period",
                text: "I think so um.",
                want: "I think so.",
            },
            Case {
                name: "after comma before period",
                text: "I think so, um.",
                want: "I think so.",
            },
            Case {
                name: "at the very end",
                text: "I think so, um",
                want: "I think so",
            },
            Case {
                name: "whole transcript",
                text: "Um.",
                want: "",
            },
            Case {
                name: "after a sentence",
                text: "Hello. Um.",
                want: "Hello.",
            },
            Case {
                name: "between sentences",
                text: "Hello. Um. The build passed.",
                want: "Hello. The build passed.",
            },
            Case {
                name: "before a newline",
                text: "Hello um\nWorld",
                want: "Hello\nWorld",
            },
            Case {
                name: "sentence start after a newline",
                text: "Hello.\nUm, the build passed.",
                want: "Hello.\nThe build passed.",
            },
            Case {
                name: "sentence start after a question",
                text: "Ready? Uh, yes.",
                want: "Ready? Yes.",
            },
            Case {
                name: "capital passes to a replacement",
                text: "Um, abed is here.",
                want: "Abed is here.",
            },
            Case {
                name: "whitespace-only replacement deletes",
                text: "Troy, you know, is here.",
                want: "Troy, is here.",
            },
            Case {
                name: "inside a word is untouched",
                text: "Gumball umbrella.",
                want: "Gumball umbrella.",
            },
            Case {
                name: "mid-sentence before a semicolon",
                text: "I think um; maybe.",
                want: "I think; maybe.",
            },
            Case {
                name: "trailing spaces before the end",
                text: "I think so um   ",
                want: "I think so",
            },
        ];
        for tc in cases {
            assert_eq!(apply(tc.text, &rules), tc.want, "REP-5 {}", tc.name);
        }
    }

    #[test]
    fn decode_rules_rep_7() {
        struct Case {
            name: &'static str,
            json: Option<&'static str>,
            want: Vec<ReplacementRule>,
            want_error: bool,
        }
        let cases = [
            Case {
                name: "missing key is empty",
                json: None,
                want: vec![],
                want_error: false,
            },
            Case {
                name: "null is empty",
                json: Some("null"),
                want: vec![],
                want_error: false,
            },
            Case {
                name: "empty array",
                json: Some("[]"),
                want: vec![],
                want_error: false,
            },
            Case {
                name: "stored shape",
                json: Some(
                    r#"[{"id": "00000000-0000-0000-0000-000000000001",
                        "triggers": ["greendale", "greendale cc"],
                        "replacement": "Greendale Community College"}]"#,
                ),
                want: vec![rule_with_id(
                    "00000000-0000-0000-0000-000000000001",
                    &["greendale", "greendale cc"],
                    "Greendale Community College",
                )],
                want_error: false,
            },
            Case {
                name: "a bare string is corrupt",
                json: Some(r#""just a string""#),
                want: vec![],
                want_error: true,
            },
            Case {
                name: "an object is corrupt",
                json: Some(r#"{"troy": "abed"}"#),
                want: vec![],
                want_error: true,
            },
            Case {
                name: "a row missing a field is corrupt",
                json: Some(r#"[{"id": "1", "triggers": ["troy"]}]"#),
                want: vec![],
                want_error: true,
            },
            Case {
                name: "a corrupt row costs only itself",
                json: Some(
                    r#"[{"id": "1", "triggers": ["troy"], "replacement": "Troy Barnes"},
                        {"triggers": ["abed"], "replacement": "Abed Nadir"},
                        {"id": "3", "triggers": "annie", "replacement": "Annie Edison"},
                        {"id": "4", "triggers": ["britta"], "replacement": "Britta Perry"}]"#,
                ),
                want: vec![
                    rule_with_id("1", &["troy"], "Troy Barnes"),
                    rule_with_id("4", &["britta"], "Britta Perry"),
                ],
                want_error: true,
            },
        ];
        for tc in cases {
            let value: Option<serde_json::Value> =
                tc.json.map(|j| serde_json::from_str(j).expect(tc.name));
            let got = decode_rules(value.as_ref());
            assert_eq!(got.rules, tc.want, "{}", tc.name);
            assert_eq!(got.error.is_some(), tc.want_error, "{}", tc.name);
        }
    }

    #[test]
    fn rules_round_trip_through_the_stored_shape_rep_7() {
        let rules = vec![
            rule_with_id(
                "00000000-0000-0000-0000-000000000001",
                &["troy", "abed"],
                "Troy Barnes",
            ),
            rule_with_id(
                "00000000-0000-0000-0000-000000000002",
                &["señor chang"],
                "Señor Chang",
            ),
        ];
        let value = serde_json::to_value(&rules).expect("encode");
        let want = serde_json::json!([
            {"id": "00000000-0000-0000-0000-000000000001", "triggers": ["troy", "abed"], "replacement": "Troy Barnes"},
            {"id": "00000000-0000-0000-0000-000000000002", "triggers": ["señor chang"], "replacement": "Señor Chang"},
        ]);
        assert_eq!(value, want, "stored shape");
        let decoded = decode_rules(Some(&value));
        assert!(decoded.error.is_none(), "round trip has no error");
        assert_eq!(decoded.rules, rules, "round trip");
    }

    #[test]
    fn parse_triggers_rep_9() {
        struct Case {
            name: &'static str,
            input: &'static str,
            want: &'static [&'static str],
        }
        let cases = [
            Case {
                name: "empty field",
                input: "",
                want: &[],
            },
            Case {
                name: "whitespace only",
                input: "   ",
                want: &[],
            },
            Case {
                name: "commas only",
                input: " , ,, ",
                want: &[],
            },
            Case {
                name: "single trigger",
                input: "troy",
                want: &["troy"],
            },
            Case {
                name: "comma separated with uneven spacing",
                input: "troy, abed ,annie",
                want: &["troy", "abed", "annie"],
            },
            Case {
                name: "trailing comma while typing the next trigger",
                input: "troy, ",
                want: &["troy"],
            },
            Case {
                name: "empty pieces between commas are dropped",
                input: "troy,, ,abed",
                want: &["troy", "abed"],
            },
            Case {
                name: "multi-word triggers keep inner spaces",
                input: "señor chang, greendale community college",
                want: &["señor chang", "greendale community college"],
            },
            Case {
                name: "pasted tabs and newlines are trimmed",
                input: "\ttroy\n,\nabed ",
                want: &["troy", "abed"],
            },
            Case {
                name: "case is preserved",
                input: "Troy Barnes",
                want: &["Troy Barnes"],
            },
        ];
        for tc in cases {
            assert_eq!(parse_triggers(tc.input), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn trigger_text_round_trips_rep_9() {
        struct Case {
            name: &'static str,
            input: &'static [&'static str],
            want: &'static str,
        }
        let cases = [
            Case {
                name: "no triggers",
                input: &[],
                want: "",
            },
            Case {
                name: "single trigger",
                input: &["troy"],
                want: "troy",
            },
            Case {
                name: "several triggers",
                input: &["troy", "señor chang"],
                want: "troy, señor chang",
            },
        ];
        for tc in cases {
            let input: Vec<String> = tc.input.iter().map(|t| (*t).to_owned()).collect();
            let text = trigger_text(&input);
            assert_eq!(text, tc.want, "{}", tc.name);
            assert_eq!(parse_triggers(&text), input, "{} round trip", tc.name);
        }
    }

    #[test]
    fn rules_to_persist_drops_only_blank_rows_rep_12() {
        let complete = rule_with_id(
            "1",
            &["greendale", "greendale cc"],
            "Greendale Community College",
        );
        let triggers_only = rule_with_id("2", &["troy"], "");
        let replacement_only = rule_with_id("3", &[], "Abed Nadir");
        let untrimmed = rule_with_id("4", &[" chang "], " Señor Chang ");
        let blank = rule_with_id("5", &[], "");
        let whitespace_only = rule_with_id("6", &["", "  "], " \n");

        struct Case {
            name: &'static str,
            input: Vec<ReplacementRule>,
            want: Vec<ReplacementRule>,
        }
        let cases = [
            Case {
                name: "no rules",
                input: vec![],
                want: vec![],
            },
            Case {
                name: "complete rule is kept",
                input: vec![complete.clone()],
                want: vec![complete.clone()],
            },
            Case {
                name: "blank row is dropped",
                input: vec![blank.clone()],
                want: vec![],
            },
            Case {
                name: "whitespace-only row is dropped",
                input: vec![whitespace_only.clone()],
                want: vec![],
            },
            Case {
                name: "triggers without a replacement are kept",
                input: vec![triggers_only.clone()],
                want: vec![triggers_only.clone()],
            },
            Case {
                name: "replacement without triggers is kept",
                input: vec![replacement_only.clone()],
                want: vec![replacement_only.clone()],
            },
            Case {
                name: "kept rows are stored exactly as entered",
                input: vec![untrimmed.clone()],
                want: vec![untrimmed],
            },
            Case {
                name: "ids and order survive around dropped rows",
                input: vec![
                    blank.clone(),
                    complete.clone(),
                    whitespace_only,
                    triggers_only.clone(),
                    blank,
                    replacement_only.clone(),
                ],
                want: vec![complete, triggers_only, replacement_only],
            },
        ];
        for tc in cases {
            assert_eq!(rules_to_persist(&tc.input), tc.want, "{}", tc.name);
        }
    }
}
