//! Fn / Globe key warning (HOT-35).

use super::shortcut::{ModifierKey, Shortcut};
use crate::platform::Platform;

/// What the OS does when Fn (🌐) is pressed on its own: macOS's "Press 🌐
/// key to" setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobeKeyAction {
    /// Do Nothing.
    DoNothing,
    /// Change Input Source.
    ChangeInputSource,
    /// Show Emoji & Symbols.
    ShowEmojiAndSymbols,
    /// Start Dictation (on a double press).
    StartDictation,
    /// Not set, or a value this version does not know.
    Unknown,
}

impl GlobeKeyAction {
    /// Maps macOS's `AppleFnUsageType` (domain `com.apple.HIToolbox`), or
    /// `None` when it is unset.
    pub fn from_apple_fn_usage_type(usage_type: Option<i64>) -> Self {
        match usage_type {
            Some(0) => Self::DoNothing,
            Some(1) => Self::ChangeInputSource,
            Some(2) => Self::ShowEmojiAndSymbols,
            Some(3) => Self::StartDictation,
            _ => Self::Unknown,
        }
    }

    /// The warning shown when Fn is the shortcut, or `None` when the key
    /// does nothing else.
    pub fn fn_warning(self) -> Option<&'static str> {
        let message = match self {
            Self::DoNothing => return None,
            Self::ChangeInputSource => {
                "Pressing 🌐 also switches your input source. Set “Press 🌐 key to” to Do Nothing."
            }
            Self::ShowEmojiAndSymbols => {
                "Pressing 🌐 also opens Emoji & Symbols. Set “Press 🌐 key to” to Do Nothing."
            }
            Self::StartDictation => {
                "Pressing 🌐 twice also starts macOS dictation. Set “Press 🌐 key to” to Do Nothing."
            }
            Self::Unknown => {
                "If 🌐 also opens Emoji & Symbols or switches input source, set “Press 🌐 key to” to Do Nothing."
            }
        };
        Some(message)
    }
}

/// Title of the button shown next to the Fn warning.
pub const OPEN_KEYBOARD_SETTINGS_TITLE: &str = "Open Keyboard Settings";

/// The warning to show for `shortcut` given the Globe key setting: only when
/// the shortcut is Fn and the key does something else too (HOT-35). A
/// platform where Fn cannot be the shortcut has no such setting to point
/// at, so it never warns.
pub fn fn_key_warning(
    shortcut: &Shortcut,
    action: GlobeKeyAction,
    platform: Platform,
) -> Option<&'static str> {
    if !platform.lone_modifier_hotkeys || shortcut.lone_modifier() != Some(ModifierKey::Fn) {
        return None;
    }
    action.fn_warning()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy_naming::PrivacyNaming;
    use GlobeKeyAction::{
        ChangeInputSource, DoNothing, ShowEmojiAndSymbols, StartDictation, Unknown,
    };

    #[test]
    fn action_for_usage_type() {
        let cases = [
            ("do nothing", Some(0), DoNothing, false),
            ("change input source", Some(1), ChangeInputSource, true),
            ("show emoji and symbols", Some(2), ShowEmojiAndSymbols, true),
            ("start dictation", Some(3), StartDictation, true),
            ("never set", None, Unknown, true),
            ("unknown value", Some(7), Unknown, true),
            ("negative value", Some(-1), Unknown, true),
        ];
        for (name, usage_type, want, want_warning) in cases {
            let got = GlobeKeyAction::from_apple_fn_usage_type(usage_type);
            assert_eq!(got, want, "{name}");
            assert_eq!(got.fn_warning().is_some(), want_warning, "{name}");
        }
    }

    #[test]
    fn warning_copy_hot_35() {
        let cases = [
            (DoNothing, None),
            (
                ChangeInputSource,
                Some(
                    "Pressing 🌐 also switches your input source. Set “Press 🌐 key to” to Do Nothing.",
                ),
            ),
            (
                ShowEmojiAndSymbols,
                Some(
                    "Pressing 🌐 also opens Emoji & Symbols. Set “Press 🌐 key to” to Do Nothing.",
                ),
            ),
            (
                StartDictation,
                Some(
                    "Pressing 🌐 twice also starts macOS dictation. Set “Press 🌐 key to” to Do Nothing.",
                ),
            ),
            (
                Unknown,
                Some(
                    "If 🌐 also opens Emoji & Symbols or switches input source, set “Press 🌐 key to” to Do Nothing.",
                ),
            ),
        ];
        for (action, want) in cases {
            assert_eq!(action.fn_warning(), want, "{action:?}");
        }
    }

    #[test]
    fn warning_only_for_fn_shortcut_hot_35() {
        let macos = Platform::macos(PrivacyNaming::Accessibility);
        let linux = Platform::linux();
        let cases = [
            (
                "fn with emoji",
                macos,
                Shortcut::lone(ModifierKey::Fn),
                ShowEmojiAndSymbols,
                true,
            ),
            (
                "fn with do nothing",
                macos,
                Shortcut::lone(ModifierKey::Fn),
                DoNothing,
                false,
            ),
            (
                "right option with emoji",
                macos,
                Shortcut::lone(ModifierKey::AltRight),
                ShowEmojiAndSymbols,
                false,
            ),
            (
                "option space with unknown",
                macos,
                Shortcut::OPTION_SPACE,
                Unknown,
                false,
            ),
            (
                "fn with emoji where fn cannot be the shortcut",
                linux,
                Shortcut::lone(ModifierKey::Fn),
                ShowEmojiAndSymbols,
                false,
            ),
            (
                "fn with unknown where fn cannot be the shortcut",
                linux,
                Shortcut::lone(ModifierKey::Fn),
                Unknown,
                false,
            ),
        ];
        for (name, platform, shortcut, action, want) in cases {
            assert_eq!(
                fn_key_warning(&shortcut, action, platform).is_some(),
                want,
                "{name}"
            );
        }
    }
}
