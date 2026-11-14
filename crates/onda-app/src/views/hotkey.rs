//! The Hotkey page beyond the recorder (WIN-16, WIN-17). The shortcut,
//! recorder, and Fn warning arrive in `HotkeyChanged`.

use onda_domain::hotkey::Shortcut;
use onda_domain::platform::Platform;
use onda_domain::types::ActivationMode;

use super::ButtonView;

/// One activation mode in the picker (WIN-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationOption {
    /// The mode.
    pub mode: ActivationMode,
    /// "Hold", "Toggle", or "Automatic".
    pub title: String,
}

/// The Hotkey page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyPage {
    /// "Press to talk", the row with the keycaps.
    pub press_to_talk_label: String,
    /// "Reset to ⌥ Space", naming the platform's default shortcut.
    /// Disabled when already the default.
    pub reset: ButtonView,
    /// Footer under the shortcut section.
    pub footer: String,
    /// "Activation".
    pub activation_label: String,
    /// Hold, Toggle, Automatic.
    pub activation_options: Vec<ActivationOption>,
    /// The selected mode.
    pub activation: ActivationMode,
    /// The selected mode's description.
    pub activation_description: String,
}

/// Builds the Hotkey page for `platform`, which names the keys and holds
/// the default shortcut.
pub fn hotkey_page(
    shortcut: &Shortcut,
    activation: ActivationMode,
    platform: Platform,
) -> HotkeyPage {
    let names = platform.key_names;
    let default = platform.default_shortcut;
    HotkeyPage {
        press_to_talk_label: "Press to talk".to_owned(),
        reset: ButtonView::new(
            &format!("Reset to {}", default.display(names)),
            shortcut != default,
        ),
        footer: names.hotkey_footer.to_owned(),
        activation_label: "Activation".to_owned(),
        activation_options: ActivationMode::ALL
            .into_iter()
            .map(|mode| ActivationOption {
                mode,
                title: mode.display_name().to_owned(),
            })
            .collect(),
        activation,
        activation_description: activation.description().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onda_domain::hotkey::ModifierKey;
    use onda_domain::privacy_naming::PrivacyNaming;

    #[test]
    fn win_16_reset_is_disabled_for_the_platforms_default() {
        let macos = Platform::macos(PrivacyNaming::Accessibility);
        let linux = Platform::linux();
        let right_option = Shortcut::lone(ModifierKey::AltRight);
        let cases = [
            (
                "the macOS default",
                macos,
                Shortcut::OPTION_SPACE,
                "Reset to ⌥ Space",
                false,
            ),
            (
                "right option",
                macos,
                right_option.clone(),
                "Reset to ⌥ Space",
                true,
            ),
            (
                "the Linux default",
                linux,
                Shortcut::CONTROL_ALT_SPACE,
                "Reset to Ctrl+Alt+Space",
                false,
            ),
            (
                "the macOS default on Linux",
                linux,
                Shortcut::OPTION_SPACE,
                "Reset to Ctrl+Alt+Space",
                true,
            ),
            (
                "right alt on Linux",
                linux,
                right_option,
                "Reset to Ctrl+Alt+Space",
                true,
            ),
        ];
        for (name, platform, shortcut, want_title, want_enabled) in cases {
            let page = hotkey_page(&shortcut, ActivationMode::Toggle, platform);
            assert_eq!(page.reset.title, want_title, "{name}");
            assert_eq!(page.reset.enabled, want_enabled, "{name}");
            assert_eq!(
                page.activation_description, "Tap once to start recording, tap again to stop.",
                "{name}"
            );
        }
    }

    #[test]
    fn win_16_the_footer_names_the_platforms_keys() {
        let cases = [
            (
                Platform::macos(PrivacyNaming::Accessibility),
                "Shortcuts must include Command, Control, or Option — or use a single modifier key like Right Option or Fn on its own — so Onda does not steal every innocent keystroke like a gremlin.",
            ),
            (
                Platform::linux(),
                "Shortcuts must include Ctrl, Alt, or Super, so Onda does not steal every innocent keystroke like a gremlin.",
            ),
        ];
        for (platform, want) in cases {
            let page = hotkey_page(&Shortcut::OPTION_SPACE, ActivationMode::Hold, platform);
            assert_eq!(page.footer, want, "{}", platform.os_name);
        }
    }
}
