//! The Replacements page (REP-8..REP-11).

/// Footer under the rules (REP-11).
const FOOTER: &str = "Runs after transcription and post-processing, right before Onda types. \
                      Matching ignores case and only hits whole words. Separate several \
                      phrases with commas. Leave With empty to delete the phrase, such as \"um\" \
                      or \"you know\".";

/// One rule row (REP-9), with the trigger text exactly as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementRowView {
    /// Pass to the row commands.
    pub id: String,
    /// Comma-separated triggers, raw.
    pub triggers: String,
    /// Replacement text.
    pub replacement: String,
}

/// The Replacements page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementsPage {
    /// "Instant Replacement".
    pub title: String,
    /// The empty-state caption when there are no rows (REP-8).
    pub empty_caption: Option<String>,
    /// Rows in order.
    pub rows: Vec<ReplacementRowView>,
    /// Bumped when rows are reloaded, added, or deleted, never on typing:
    /// rebind text fields only when it changes.
    pub revision: u64,
    /// "Replace".
    pub replace_label: String,
    /// "With".
    pub with_label: String,
    /// "greendale, greendale cc".
    pub triggers_placeholder: String,
    /// "Greendale Community College".
    pub replacement_placeholder: String,
    /// "Delete Rule".
    pub delete_title: String,
    /// "Add Rule".
    pub add_title: String,
    /// Footer.
    pub footer: String,
}

/// Builds the page.
pub fn replacements_page(rows: Vec<ReplacementRowView>, revision: u64) -> ReplacementsPage {
    ReplacementsPage {
        title: "Instant Replacement".to_owned(),
        empty_caption: rows.is_empty().then(|| {
            "No rules yet. Add a phrase Onda keeps getting wrong and what it should type instead."
                .to_owned()
        }),
        rows,
        revision,
        replace_label: "Replace".to_owned(),
        with_label: "With".to_owned(),
        triggers_placeholder: "greendale, greendale cc".to_owned(),
        replacement_placeholder: "Greendale Community College".to_owned(),
        delete_title: "Delete Rule".to_owned(),
        add_title: "Add Rule".to_owned(),
        footer: FOOTER.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rep_11_footer_copy() {
        assert_eq!(
            replacements_page(Vec::new(), 0).footer,
            "Runs after transcription and post-processing, right before Onda types. Matching \
             ignores case and only hits whole words. Separate several phrases with commas. \
             Leave With empty to delete the phrase, such as \"um\" or \"you know\"."
        );
    }
}
