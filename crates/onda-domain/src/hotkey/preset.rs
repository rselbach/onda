//! One-click shortcut choices offered in onboarding (Presets table, ONB-7).
//!
//! A system offers only the presets it can register.
//! [`Platform::hotkey_presets`](crate::platform::Platform::hotkey_presets)
//! is its list.

use super::key_names::KeyNames;
use super::shortcut::{ModifierKey, Shortcut};

/// A preset shortcut. Anything else is "Another shortcut", recorded with the
/// recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyPreset {
    /// `⌥ Space`.
    OptionSpace,
    /// Right Option on its own.
    RightOption,
    /// Fn (🌐) on its own.
    Fn,
    /// `Ctrl+Alt+Space`.
    ControlAltSpace,
    /// `Alt+Super+Space`.
    AltSuperSpace,
    /// `Ctrl+Super+Space`.
    ControlSuperSpace,
}

impl HotkeyPreset {
    /// macOS's presets in display order. The Swift shell has a case for
    /// each, restated in `onda-ffi`, so a new one needs a case there.
    pub const MACOS: [Self; 3] = [Self::OptionSpace, Self::RightOption, Self::Fn];

    /// A Linux desktop's presets in display order. Each is a combo, because
    /// a Linux desktop registers no lone modifier, and each is one that
    /// GNOME and KDE leave unassigned.
    pub const LINUX: [Self; 3] = [
        Self::ControlAltSpace,
        Self::AltSuperSpace,
        Self::ControlSuperSpace,
    ];

    /// The shortcut this preset selects.
    pub fn shortcut(self) -> Shortcut {
        match self {
            Self::OptionSpace => Shortcut::OPTION_SPACE,
            Self::RightOption => Shortcut::lone(ModifierKey::AltRight),
            Self::Fn => Shortcut::lone(ModifierKey::Fn),
            Self::ControlAltSpace => Shortcut::CONTROL_ALT_SPACE,
            Self::AltSuperSpace => Shortcut::ALT_SUPER_SPACE,
            Self::ControlSuperSpace => Shortcut::CONTROL_SUPER_SPACE,
        }
    }

    /// The detail text under the preset's keycaps, in the system's key
    /// names.
    pub fn detail(self, names: &KeyNames) -> String {
        match self {
            Self::OptionSpace => {
                "Works without Typing Access. Alfred, Raycast, and ChatGPT default to it too."
                    .to_owned()
            }
            Self::RightOption => format!(
                "The {alt} key right of the space bar. Left {alt} works as usual.",
                alt = names.alt.key
            ),
            Self::Fn => "The fn or 🌐 key. Some third-party keyboards don't send it.".to_owned(),
            Self::ControlAltSpace => "The default. GNOME and KDE leave it unassigned.".to_owned(),
            Self::AltSuperSpace => format!(
                "{alt} and {meta} sit side by side, left of the space bar.",
                alt = names.alt.key,
                meta = names.meta.key
            ),
            Self::ControlSuperSpace => {
                "A spare for when another app already uses the other two.".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::shortcut::{KeyCode, Modifier};

    fn every_preset() -> impl Iterator<Item = HotkeyPreset> {
        HotkeyPreset::MACOS.into_iter().chain(HotkeyPreset::LINUX)
    }

    #[test]
    fn presets_are_valid_shortcuts() {
        for preset in every_preset() {
            let shortcut = preset.shortcut();
            let got = Shortcut::new(shortcut.key().clone(), shortcut.modifiers());
            assert_eq!(got, Ok(shortcut), "{preset:?}");
        }
    }

    #[test]
    fn no_two_presets_select_the_same_shortcut() {
        let presets: Vec<HotkeyPreset> = every_preset().collect();
        for (i, first) in presets.iter().enumerate() {
            for second in &presets[i + 1..] {
                assert_ne!(first.shortcut(), second.shortcut(), "{first:?} {second:?}");
            }
        }
    }

    #[test]
    fn macos_preset_details() {
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
        assert_eq!(HotkeyPreset::MACOS, cases.map(|(preset, _, _)| preset));
        for (preset, want_detail, want_display) in cases {
            assert_eq!(preset.detail(&KeyNames::MACOS), want_detail, "{preset:?}");
            assert_eq!(
                preset.shortcut().display(&KeyNames::MACOS),
                want_display,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn linux_presets_are_space_combos() {
        let cases = [
            (
                HotkeyPreset::ControlAltSpace,
                "The default. GNOME and KDE leave it unassigned.",
                "Ctrl+Alt+Space",
                [Modifier::Control, Modifier::Alt],
            ),
            (
                HotkeyPreset::AltSuperSpace,
                "Alt and Super sit side by side, left of the space bar.",
                "Alt+Super+Space",
                [Modifier::Alt, Modifier::Meta],
            ),
            (
                HotkeyPreset::ControlSuperSpace,
                "A spare for when another app already uses the other two.",
                "Ctrl+Super+Space",
                [Modifier::Control, Modifier::Meta],
            ),
        ];
        assert_eq!(HotkeyPreset::LINUX, cases.map(|(preset, ..)| preset));
        for (preset, want_detail, want_display, want_modifiers) in cases {
            let shortcut = preset.shortcut();
            assert_eq!(preset.detail(&KeyNames::LINUX), want_detail, "{preset:?}");
            assert_eq!(
                shortcut.display(&KeyNames::LINUX),
                want_display,
                "{preset:?}"
            );
            assert_eq!(shortcut.key(), &KeyCode::SPACE, "{preset:?}");
            assert_eq!(
                shortcut.modifiers(),
                want_modifiers.into_iter().collect(),
                "{preset:?}"
            );
        }
    }

    #[test]
    fn preset_details_use_the_systems_key_names() {
        assert_eq!(
            HotkeyPreset::RightOption.detail(&KeyNames::LINUX),
            "The Alt key right of the space bar. Left Alt works as usual."
        );
    }
}
