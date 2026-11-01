//! Pure helpers behind "Type into active app" (03, "Text output").

use unicode_segmentation::UnicodeSegmentation;

use crate::types::OutputMode;

/// Most UTF-16 code units one synthetic Unicode key event may carry (OUT-3).
pub const UNICODE_CHUNK_UTF16: usize = 20;

/// How much text before the caret the joiner looks at, in UTF-16 code units
/// before grapheme widening (OUT-2).
pub const TEXT_BEFORE_CARET_UTF16: usize = 64;

/// Splits `text` into UTF-16 chunks of at most [`UNICODE_CHUNK_UTF16`] code
/// units for synthetic key events, never splitting a surrogate pair (OUT-3).
///
/// Empty text yields no chunks.
pub fn unicode_chunks(text: &str) -> Vec<Vec<u16>> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut end = (start + UNICODE_CHUNK_UTF16).min(units.len());
        if end < units.len() && is_high_surrogate(units[end - 1]) && is_low_surrogate(units[end]) {
            end -= 1;
        }
        chunks.push(units[start..end].to_vec());
        start = end;
    }
    chunks
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// The joiner's `preceding` text: up to [`TEXT_BEFORE_CARET_UTF16`] UTF-16
/// code units before the caret, widened backwards to whole grapheme clusters
/// (OUT-2).
///
/// `field_text` is the focused field's full value and `caret_utf16` the
/// selection start in UTF-16 code units, as accessibility APIs report it. A
/// caret past the end is clamped; a caret inside a surrogate pair moves to
/// the start of that character.
pub fn text_before_caret(field_text: &str, caret_utf16: usize) -> &str {
    let end = byte_offset_at_or_before(field_text, caret_utf16);
    let start_utf16 = caret_utf16
        .min(field_text.encode_utf16().count())
        .saturating_sub(TEXT_BEFORE_CARET_UTF16);
    let start = byte_offset_at_or_before(field_text, start_utf16);
    // widen to the start of the grapheme cluster containing `start`
    let widened = field_text[..end]
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .take_while(|i| *i <= start)
        .last()
        .unwrap_or(0);
    &field_text[widened..end]
}

// byte offset of the last char boundary whose UTF-16 offset is <= `utf16`
fn byte_offset_at_or_before(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        let next = units + c.len_utf16();
        if next > utf16 {
            return index;
        }
        units = next;
    }
    text.len()
}

/// Whether `mode` needs the Accessibility permission before dictating (DICT-1).
pub fn requires_accessibility(mode: OutputMode) -> bool {
    match mode {
        OutputMode::Type => true,
        OutputMode::Clipboard => false,
    }
}

/// How a finished transcript reaches the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryRoute {
    /// Type into the target app through the OUT-3 strategy chain.
    Insert,
    /// Copy without scheduling a clipboard restore, and show "Transcript
    /// Copied to Clipboard" (OUT-1). An untrusted process's synthetic paste
    /// is silently dropped, so a restore would destroy the transcript.
    CopyWithNotice,
    /// Replace the clipboard with the text; no paste, no restore (OUT-7).
    Copy,
}

/// Picks the delivery route at delivery time (OUT-1, OUT-7).
pub fn delivery_route(mode: OutputMode, accessibility_granted: bool) -> DeliveryRoute {
    match (mode, accessibility_granted) {
        (OutputMode::Clipboard, _) => DeliveryRoute::Copy,
        (OutputMode::Type, true) => DeliveryRoute::Insert,
        (OutputMode::Type, false) => DeliveryRoute::CopyWithNotice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_chunks_out_3() {
        struct Case {
            name: &'static str,
            text: String,
            want_lengths: Vec<usize>,
        }
        let cases = [
            Case {
                name: "empty text has no chunks",
                text: String::new(),
                want_lengths: vec![],
            },
            Case {
                name: "short text is one chunk",
                text: "Troy Barnes".to_owned(),
                want_lengths: vec![11],
            },
            Case {
                name: "exactly the cap is one chunk",
                text: "a".repeat(20),
                want_lengths: vec![20],
            },
            Case {
                name: "one over the cap spills",
                text: "a".repeat(21),
                want_lengths: vec![20, 1],
            },
            Case {
                name: "surrogate pair on the boundary moves to the next chunk",
                text: format!("{}🎤", "a".repeat(19)),
                want_lengths: vec![19, 2],
            },
            Case {
                name: "surrogate pair ending exactly on the boundary stays",
                text: format!("{}🎤", "a".repeat(18)),
                want_lengths: vec![20],
            },
        ];
        for tc in cases {
            let chunks = unicode_chunks(&tc.text);
            let lengths: Vec<usize> = chunks.iter().map(Vec::len).collect();
            assert_eq!(lengths, tc.want_lengths, "{}", tc.name);
            let reassembled: Vec<u16> = chunks.concat();
            let want: Vec<u16> = tc.text.encode_utf16().collect();
            assert_eq!(reassembled, want, "{} reassembles", tc.name);
        }
    }

    #[test]
    fn unicode_chunks_never_exceed_cap_or_split_pairs_out_3() {
        let text = "Streets ahead 🎤🎬👨‍👩‍👧 ".repeat(30);
        let chunks = unicode_chunks(&text);
        assert!(!chunks.is_empty(), "mixed emoji text has chunks");
        for chunk in &chunks {
            assert!(
                chunk.len() <= UNICODE_CHUNK_UTF16,
                "chunk of {} units",
                chunk.len()
            );
            assert!(
                String::from_utf16(chunk).is_ok(),
                "chunk decodes on its own, so no pair was split"
            );
        }
        let want: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(chunks.concat(), want, "reassembles exactly");
    }

    #[test]
    fn text_before_caret_out_2() {
        let long = format!("Greendale {}", "a".repeat(70));
        let flag_boundary = format!("🇺🇸{}", "b".repeat(63));
        let family = format!("👨‍👩‍👧{}", "c".repeat(60));
        struct Case<'a> {
            name: &'static str,
            field: &'a str,
            caret: usize,
            want: &'a str,
        }
        let cases = [
            Case {
                name: "empty field",
                field: "",
                caret: 0,
                want: "",
            },
            Case {
                name: "caret at start",
                field: "Troy",
                caret: 0,
                want: "",
            },
            Case {
                name: "short field caret at end",
                field: "Troy and Abed",
                caret: 13,
                want: "Troy and Abed",
            },
            Case {
                name: "caret mid-field",
                field: "Troy and Abed",
                caret: 4,
                want: "Troy",
            },
            Case {
                name: "caret past the end is clamped",
                field: "Troy",
                caret: 99,
                want: "Troy",
            },
            Case {
                name: "long field keeps the last 64 units",
                field: &long,
                caret: 80,
                want: &long[16..],
            },
            Case {
                name: "window starting inside a flag widens to the whole flag",
                field: &flag_boundary,
                caret: 67,
                want: &flag_boundary,
            },
            Case {
                name: "window starting inside a ZWJ family widens to the whole cluster",
                field: &family,
                caret: 68,
                want: &family,
            },
            Case {
                name: "caret inside a surrogate pair moves before it",
                field: "a🎤",
                caret: 2,
                want: "a",
            },
        ];
        for tc in cases {
            assert_eq!(
                text_before_caret(tc.field, tc.caret),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn delivery_route_out_1_out_7() {
        struct Case {
            name: &'static str,
            mode: OutputMode,
            accessibility: bool,
            want: DeliveryRoute,
            want_requires_accessibility: bool,
        }
        let cases = [
            Case {
                name: "type with accessibility inserts",
                mode: OutputMode::Type,
                accessibility: true,
                want: DeliveryRoute::Insert,
                want_requires_accessibility: true,
            },
            Case {
                name: "type without accessibility copies with a notice, never pastes",
                mode: OutputMode::Type,
                accessibility: false,
                want: DeliveryRoute::CopyWithNotice,
                want_requires_accessibility: true,
            },
            Case {
                name: "clipboard ignores accessibility",
                mode: OutputMode::Clipboard,
                accessibility: false,
                want: DeliveryRoute::Copy,
                want_requires_accessibility: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                delivery_route(tc.mode, tc.accessibility),
                tc.want,
                "{}",
                tc.name
            );
            assert_eq!(
                requires_accessibility(tc.mode),
                tc.want_requires_accessibility,
                "{}",
                tc.name
            );
        }
    }
}
