//! Save rules for the OpenRouter API key field on the settings page (CLN-25).
//!
//! The field loads the stored key asynchronously and saves on Return, end of
//! editing, page disappear, window close, and quit. These rules keep those
//! saves from clobbering the stored key.

/// What the field knows about the stored key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyBaseline {
    /// The initial load is in flight.
    Loading,
    /// The load failed or was cancelled by typing; the stored value is
    /// unknown, so a blank field must not delete it.
    Unknown,
    /// The value loaded or last saved.
    Known(String),
}

impl KeyBaseline {
    /// The baseline after the user types: typing during the load cancels it
    /// (the caller cancels the read), leaving the stored value unknown.
    pub fn after_user_edit(self) -> Self {
        match self {
            Self::Loading => Self::Unknown,
            other => other,
        }
    }
}

/// A credential-store write the field should perform (CLN-24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyWrite {
    /// Store this trimmed key; afterwards the baseline is `Known(key)`.
    Save(String),
    /// Delete the stored key; afterwards the baseline is `Known("")`.
    Delete,
}

/// Decides whether saving `field` writes anything (CLN-24, CLN-25).
///
/// - Never while the initial load is in flight.
/// - After a failed load, only a non-empty value may be saved.
/// - Skip when the trimmed value equals what was loaded or last saved.
pub fn key_write(baseline: &KeyBaseline, field: &str) -> Option<KeyWrite> {
    let value = field.trim();
    let write = match value {
        "" => KeyWrite::Delete,
        key => KeyWrite::Save(key.to_owned()),
    };
    match baseline {
        KeyBaseline::Loading => None,
        KeyBaseline::Unknown if value.is_empty() => None,
        KeyBaseline::Unknown => Some(write),
        KeyBaseline::Known(known) if known.trim() == value => None,
        KeyBaseline::Known(_) => Some(write),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_write_cln_25() {
        struct Case {
            name: &'static str,
            baseline: KeyBaseline,
            field: &'static str,
            want: Option<KeyWrite>,
        }
        let known = |key: &str| KeyBaseline::Known(key.to_owned());
        let save = |key: &str| Some(KeyWrite::Save(key.to_owned()));
        let cases = [
            Case {
                name: "load in progress: skip a blank save",
                baseline: KeyBaseline::Loading,
                field: "",
                want: None,
            },
            Case {
                name: "load in progress: skip a typed value",
                baseline: KeyBaseline::Loading,
                field: "sk-key-greendale",
                want: None,
            },
            Case {
                name: "load failed: a blank field never deletes",
                baseline: KeyBaseline::Unknown,
                field: "",
                want: None,
            },
            Case {
                name: "load failed: whitespace never deletes",
                baseline: KeyBaseline::Unknown,
                field: "   ",
                want: None,
            },
            Case {
                name: "load failed, user typed a new key: save",
                baseline: KeyBaseline::Unknown,
                field: "sk-key-greendale",
                want: save("sk-key-greendale"),
            },
            Case {
                name: "load failed, user typed a padded key: save it trimmed",
                baseline: KeyBaseline::Unknown,
                field: "  sk-key-greendale  ",
                want: save("sk-key-greendale"),
            },
            Case {
                name: "loaded, unchanged: skip",
                baseline: known("sk-troy-barnes"),
                field: "sk-troy-barnes",
                want: None,
            },
            Case {
                name: "loaded with whitespace, unchanged: skip",
                baseline: known("  sk-troy-barnes  "),
                field: "sk-troy-barnes",
                want: None,
            },
            Case {
                name: "loaded, edited: save",
                baseline: known("sk-old"),
                field: "sk-new-abed",
                want: save("sk-new-abed"),
            },
            Case {
                name: "loaded, cleared: delete",
                baseline: known("sk-señor-chang"),
                field: "",
                want: Some(KeyWrite::Delete),
            },
            Case {
                name: "loaded, blanked to whitespace: delete",
                baseline: known("sk-señor-chang"),
                field: "   ",
                want: Some(KeyWrite::Delete),
            },
            Case {
                name: "nothing stored, still blank: skip",
                baseline: known(""),
                field: "",
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(key_write(&tc.baseline, tc.field), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn after_user_edit_cancels_the_load_cln_25() {
        struct Case {
            name: &'static str,
            baseline: KeyBaseline,
            want: KeyBaseline,
        }
        let cases = [
            Case {
                name: "typing during the load makes the stored value unknown",
                baseline: KeyBaseline::Loading,
                want: KeyBaseline::Unknown,
            },
            Case {
                name: "typing after a failed load changes nothing",
                baseline: KeyBaseline::Unknown,
                want: KeyBaseline::Unknown,
            },
            Case {
                name: "typing after a load keeps the baseline",
                baseline: KeyBaseline::Known("sk-troy".to_owned()),
                want: KeyBaseline::Known("sk-troy".to_owned()),
            },
        ];
        for tc in cases {
            assert_eq!(tc.baseline.after_user_edit(), tc.want, "{}", tc.name);
        }
    }
}
