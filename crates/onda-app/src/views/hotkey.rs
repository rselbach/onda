//! The Hotkey page beyond the recorder (WIN-16, WIN-17). The shortcut,
//! recorder, and Fn warning arrive in `HotkeyChanged`.

use onda_domain::hotkey::Shortcut;
use onda_domain::types::ActivationMode;

use super::ButtonView;

/// The footer under the shortcut section (WIN-16).
const FOOTER: &str = "Shortcuts must include Command, Control, or Option — or use a single \
                      modifier key like Right Option or Fn on its own — so Onda does not steal \
                      every innocent keystroke like a gremlin.";

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
    /// "Reset to ⌥ Space", disabled when already the default.
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

/// Builds the Hotkey page.
pub fn hotkey_page(shortcut: &Shortcut, activation: ActivationMode) -> HotkeyPage {
    let default = Shortcut::default();
    HotkeyPage {
        press_to_talk_label: "Press to talk".to_owned(),
        reset: ButtonView::new(
            &format!("Reset to {}", default.display()),
            *shortcut != default,
        ),
        footer: FOOTER.to_owned(),
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

    #[test]
    fn win_16_reset_is_disabled_for_the_default() {
        let cases = [
            ("default", Shortcut::default(), false),
            ("right option", Shortcut::lone(ModifierKey::AltRight), true),
        ];
        for (name, shortcut, want) in cases {
            let page = hotkey_page(&shortcut, ActivationMode::Toggle);
            assert_eq!(page.reset.enabled, want, "{name}");
            assert_eq!(page.reset.title, "Reset to ⌥ Space", "{name}");
            assert_eq!(
                page.activation_description, "Tap once to start recording, tap again to stop.",
                "{name}"
            );
        }
    }
}
