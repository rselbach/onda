//! One-click shortcut choices offered in onboarding (Presets table, ONB-7).

use super::shortcut::{ModifierKey, Shortcut};

/// A preset shortcut. Anything else is "Another shortcut", recorded with the
/// recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyPreset {
    /// `⌥ Space`, the default.
    OptionSpace,
    /// Right Option on its own.
    RightOption,
    /// Fn (🌐) on its own.
    Fn,
}

impl HotkeyPreset {
    /// Every preset in display order.
    pub const ALL: [Self; 3] = [Self::OptionSpace, Self::RightOption, Self::Fn];

    /// The shortcut this preset selects.
    pub fn shortcut(self) -> Shortcut {
        match self {
            Self::OptionSpace => Shortcut::default(),
            Self::RightOption => Shortcut::lone(ModifierKey::AltRight),
            Self::Fn => Shortcut::lone(ModifierKey::Fn),
        }
    }

    /// The detail text under the preset's keycaps.
    pub fn detail(self) -> &'static str {
        match self {
            Self::OptionSpace => {
                "Works without Typing Access. Alfred, Raycast, and ChatGPT default to it too."
            }
            Self::RightOption => {
                "The Option key right of the space bar. Left Option works as usual."
            }
            Self::Fn => "The fn or 🌐 key. Some third-party keyboards don't send it.",
        }
    }

    /// The preset matching `shortcut`, or `None` for a custom shortcut.
    pub fn from_shortcut(shortcut: &Shortcut) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.shortcut() == *shortcut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::shortcut::{KeyCode, Modifier};

    #[test]
    fn preset_for_shortcut() {
        let control_option_d = Shortcut::new(
            KeyCode::parse("KeyD").expect("key"),
            [Modifier::Control, Modifier::Alt].into_iter().collect(),
        )
        .expect("valid");
        let cases = [
            (
                "option space",
                Shortcut::default(),
                Some(HotkeyPreset::OptionSpace),
            ),
            (
                "right option",
                Shortcut::lone(ModifierKey::AltRight),
                Some(HotkeyPreset::RightOption),
            ),
            (
                "fn",
                Shortcut::lone(ModifierKey::Fn),
                Some(HotkeyPreset::Fn),
            ),
            (
                "left option is custom",
                Shortcut::lone(ModifierKey::AltLeft),
                None,
            ),
            ("control option d is custom", control_option_d, None),
        ];
        for (name, shortcut, want) in cases {
            assert_eq!(HotkeyPreset::from_shortcut(&shortcut), want, "{name}");
        }
    }

    #[test]
    fn presets_are_valid_shortcuts() {
        for preset in HotkeyPreset::ALL {
            let shortcut = preset.shortcut();
            let got = Shortcut::new(shortcut.key().clone(), shortcut.modifiers());
            assert_eq!(got, Ok(shortcut), "{preset:?}");
        }
    }

    #[test]
    fn preset_details() {
        let cases = [
            (
                HotkeyPreset::OptionSpace,
                "Works without Typing Access. Alfred, Raycast, and ChatGPT default to it too.",
                "⌥ Space",
            ),
            (
                HotkeyPreset::RightOption,
                "The Option key right of the space bar. Left Option works as usual.",
                "Right Option",
            ),
            (
                HotkeyPreset::Fn,
                "The fn or 🌐 key. Some third-party keyboards don't send it.",
                "Fn",
            ),
        ];
        for (preset, want_detail, want_display) in cases {
            assert_eq!(preset.detail(), want_detail, "{preset:?}");
            assert_eq!(preset.shortcut().display(), want_display, "{preset:?}");
        }
    }
}
