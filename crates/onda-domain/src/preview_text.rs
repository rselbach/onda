//! One-line transcript previews for the tray menu (DICT-26, HIS-28).

use unicode_segmentation::UnicodeSegmentation;

/// Longest preview, in user-perceived characters (grapheme clusters).
pub const MENU_PREVIEW_LENGTH: usize = 40;

/// One line of `text` for a menu item: runs of whitespace (including line
/// breaks) collapse to single spaces, and anything longer than
/// [`MENU_PREVIEW_LENGTH`] characters is cut to fit with a trailing `…`.
pub fn menu_preview(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.graphemes(true).count() <= MENU_PREVIEW_LENGTH {
        return one_line;
    }
    let mut preview: String = one_line
        .graphemes(true)
        .take(MENU_PREVIEW_LENGTH - 1)
        .collect();
    preview.push('…');
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_preview_dict_26_his_28() {
        struct Case {
            name: &'static str,
            text: String,
            want: String,
        }
        let cases = [
            Case {
                name: "short",
                text: "Troy Barnes".into(),
                want: "Troy Barnes".into(),
            },
            Case {
                name: "line breaks and runs of spaces collapse",
                text: "Troy\n\nand   Abed".into(),
                want: "Troy and Abed".into(),
            },
            Case {
                name: "leading and trailing whitespace is dropped",
                text: "  \tTroy and Abed\n".into(),
                want: "Troy and Abed".into(),
            },
            Case {
                name: "empty",
                text: String::new(),
                want: String::new(),
            },
            Case {
                name: "exactly the limit",
                text: "a".repeat(40),
                want: "a".repeat(40),
            },
            Case {
                name: "over the limit",
                text: "Greendale Community College is where Troy and Abed met".into(),
                want: "Greendale Community College is where Tr…".into(),
            },
            Case {
                name: "emoji clusters count once and are never split",
                text: "👨‍👩‍👧".repeat(41),
                want: format!("{}…", "👨‍👩‍👧".repeat(39)),
            },
        ];
        for tc in cases {
            let got = menu_preview(&tc.text);
            assert_eq!(got, tc.want, "{}", tc.name);
            assert!(
                got.graphemes(true).count() <= MENU_PREVIEW_LENGTH,
                "{} fits",
                tc.name
            );
        }
    }
}
