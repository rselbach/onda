//! The hotkey settings view model: the shortcut's display (HOT-6), what it
//! needs (HOT-7), the recorder's state and copy (HOT-10, HOT-11), and the Fn
//! warning (HOT-35).

use super::globe::{GlobeKeyAction, OPEN_KEYBOARD_SETTINGS_TITLE, fn_key_warning};
use super::recorder::{RECORDER_CANCEL_TITLE, ShortcutRecorder};
use super::shortcut::Shortcut;
use crate::platform::Platform;

/// Everything a hotkey page or onboarding step shows about the shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyView {
    /// Display string, e.g. `⌥ Space`.
    pub display: String,
    /// One keycap per modifier glyph plus one for the key.
    pub keycaps: Vec<String>,
    /// The shortcut needs Accessibility (lone modifiers).
    pub needs_accessibility: bool,
    /// A recording is in progress: Cancel shows next to the record button.
    pub recording: bool,
    /// The shell keeps exactly one local key monitor installed while this
    /// is true and hands every key event to the recorder (HOT-9, HOT-10).
    pub listening: bool,
    /// The record button's title.
    pub record_button_title: &'static str,
    /// The cancel button's title, shown while recording.
    pub cancel_button_title: &'static str,
    /// Instructions while recording, else `Current shortcut: ⌥ Space`.
    pub help_text: String,
    /// The message under the recorder (a rejected combo, or recording could
    /// not start).
    pub message: Option<&'static str>,
    /// The Fn warning, when the shortcut is Fn and 🌐 does something else.
    pub fn_warning: Option<&'static str>,
    /// The title of the button next to the Fn warning.
    pub open_keyboard_settings_title: &'static str,
}

impl HotkeyView {
    /// The view for `shortcut` on `platform`, the recorder's state, and the
    /// Globe key setting.
    pub fn resolve(
        shortcut: &Shortcut,
        recorder: &ShortcutRecorder,
        globe: GlobeKeyAction,
        platform: Platform,
    ) -> Self {
        Self {
            display: shortcut.display(),
            keycaps: shortcut.keycaps(),
            needs_accessibility: shortcut.needs_accessibility(platform),
            recording: recorder.is_recording(),
            listening: recorder.is_recording(),
            record_button_title: recorder.button_title(),
            cancel_button_title: RECORDER_CANCEL_TITLE,
            help_text: recorder.help_text(shortcut),
            message: recorder.message().map(|m| m.text()),
            fn_warning: fn_key_warning(shortcut, globe),
            open_keyboard_settings_title: OPEN_KEYBOARD_SETTINGS_TITLE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::recorder::{RECORDING_HELP_TEXT, RecorderEvent};
    use crate::hotkey::shortcut::{KeyCode, ModifierKey, ModifierSet};
    use crate::privacy_naming::PrivacyNaming;

    #[test]
    fn resolves_shortcut_recorder_and_fn_warning() {
        struct Case {
            name: &'static str,
            shortcut: Shortcut,
            recorder: fn() -> ShortcutRecorder,
            globe: GlobeKeyAction,
            want_title: &'static str,
            want_listening: bool,
            want_help: String,
            want_message: Option<&'static str>,
            want_warning: bool,
            want_accessibility: bool,
        }
        let idle = ShortcutRecorder::new;
        let recording = || {
            let mut recorder = ShortcutRecorder::new();
            recorder.start();
            recorder
        };
        let rejected = || {
            let mut recorder = ShortcutRecorder::new();
            recorder.start();
            recorder.handle(RecorderEvent::KeyDown {
                key: KeyCode::SPACE,
                modifiers: ModifierSet::EMPTY,
            });
            recorder
        };
        let failed = || {
            let mut recorder = ShortcutRecorder::new();
            recorder.start();
            recorder.start_failed();
            recorder
        };
        let cases = [
            Case {
                name: "start failed stops listening with a message",
                shortcut: Shortcut::default(),
                recorder: failed,
                globe: GlobeKeyAction::DoNothing,
                want_title: "Record Shortcut",
                want_listening: false,
                want_help: "Current shortcut: ⌥ Space".to_owned(),
                want_message: Some("Could not start shortcut recording."),
                want_warning: false,
                want_accessibility: false,
            },
            Case {
                name: "idle combo",
                shortcut: Shortcut::default(),
                recorder: idle,
                globe: GlobeKeyAction::ChangeInputSource,
                want_title: "Record Shortcut",
                want_listening: false,
                want_help: "Current shortcut: ⌥ Space".to_owned(),
                want_message: None,
                want_warning: false,
                want_accessibility: false,
            },
            Case {
                name: "recording",
                shortcut: Shortcut::lone(ModifierKey::AltRight),
                recorder: recording,
                globe: GlobeKeyAction::DoNothing,
                want_title: "Press shortcut...",
                want_listening: true,
                want_help: RECORDING_HELP_TEXT.to_owned(),
                want_message: None,
                want_warning: false,
                want_accessibility: true,
            },
            Case {
                name: "rejected combo keeps recording with a message",
                shortcut: Shortcut::default(),
                recorder: rejected,
                globe: GlobeKeyAction::DoNothing,
                want_title: "Press shortcut...",
                want_listening: true,
                want_help: RECORDING_HELP_TEXT.to_owned(),
                want_message: Some(
                    "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn.",
                ),
                want_warning: false,
                want_accessibility: false,
            },
            Case {
                name: "fn with emoji picker warns",
                shortcut: Shortcut::lone(ModifierKey::Fn),
                recorder: idle,
                globe: GlobeKeyAction::ShowEmojiAndSymbols,
                want_title: "Record Shortcut",
                want_listening: false,
                want_help: "Current shortcut: Fn".to_owned(),
                want_message: None,
                want_warning: true,
                want_accessibility: true,
            },
            Case {
                name: "fn doing nothing does not warn",
                shortcut: Shortcut::lone(ModifierKey::Fn),
                recorder: idle,
                globe: GlobeKeyAction::DoNothing,
                want_title: "Record Shortcut",
                want_listening: false,
                want_help: "Current shortcut: Fn".to_owned(),
                want_message: None,
                want_warning: false,
                want_accessibility: true,
            },
        ];
        for tc in cases {
            let view = HotkeyView::resolve(
                &tc.shortcut,
                &(tc.recorder)(),
                tc.globe,
                Platform::macos(PrivacyNaming::Accessibility),
            );
            assert_eq!(view.display, tc.shortcut.display(), "{}", tc.name);
            assert_eq!(view.record_button_title, tc.want_title, "{}", tc.name);
            assert_eq!(view.listening, tc.want_listening, "{}", tc.name);
            assert_eq!(view.recording, tc.want_listening, "{}", tc.name);
            assert_eq!(view.help_text, tc.want_help, "{}", tc.name);
            assert_eq!(view.message, tc.want_message, "{}", tc.name);
            assert_eq!(view.fn_warning.is_some(), tc.want_warning, "{}", tc.name);
            assert_eq!(
                view.needs_accessibility, tc.want_accessibility,
                "{}",
                tc.name
            );
            assert_eq!(view.cancel_button_title, "Cancel", "{}", tc.name);
            assert_eq!(
                view.open_keyboard_settings_title, "Open Keyboard Settings",
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn a_lone_modifier_needs_no_accessibility_without_the_permission() {
        let view = HotkeyView::resolve(
            &Shortcut::lone(ModifierKey::AltRight),
            &ShortcutRecorder::new(),
            GlobeKeyAction::DoNothing,
            Platform::linux(),
        );
        assert!(!view.needs_accessibility);
    }
}
