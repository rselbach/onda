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
    /// One keycap per modifier plus one for the key.
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
        let names = platform.key_names;
        Self {
            display: shortcut.display(names),
            keycaps: shortcut.keycaps(names),
            needs_accessibility: shortcut.needs_accessibility(platform),
            recording: recorder.is_recording(),
            listening: recorder.is_recording(),
            record_button_title: recorder.button_title(),
            cancel_button_title: RECORDER_CANCEL_TITLE,
            help_text: recorder.help_text(shortcut, names),
            message: recorder.message().map(|m| m.text(names)),
            fn_warning: fn_key_warning(shortcut, globe, platform),
            open_keyboard_settings_title: OPEN_KEYBOARD_SETTINGS_TITLE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::key_names::KeyNames;
    use crate::hotkey::recorder::RecorderEvent;
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
        fn idle() -> ShortcutRecorder {
            ShortcutRecorder::new(Platform::macos(PrivacyNaming::Accessibility))
        }
        let recording = || {
            let mut recorder = idle();
            recorder.start();
            recorder
        };
        let rejected = || {
            let mut recorder = idle();
            recorder.start();
            recorder.handle(RecorderEvent::KeyDown {
                key: KeyCode::SPACE,
                modifiers: ModifierSet::EMPTY,
            });
            recorder
        };
        let failed = || {
            let mut recorder = idle();
            recorder.start();
            recorder.start_failed();
            recorder
        };
        let cases = [
            Case {
                name: "start failed stops listening with a message",
                shortcut: Shortcut::OPTION_SPACE,
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
                shortcut: Shortcut::OPTION_SPACE,
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
                want_help: KeyNames::MACOS.recording_help.to_owned(),
                want_message: None,
                want_warning: false,
                want_accessibility: true,
            },
            Case {
                name: "rejected combo keeps recording with a message",
                shortcut: Shortcut::OPTION_SPACE,
                recorder: rejected,
                globe: GlobeKeyAction::DoNothing,
                want_title: "Press shortcut...",
                want_listening: true,
                want_help: KeyNames::MACOS.recording_help.to_owned(),
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
            assert_eq!(
                view.display,
                tc.shortcut.display(&KeyNames::MACOS),
                "{}",
                tc.name
            );
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
    fn the_view_uses_the_platforms_key_names() {
        let mut recorder = ShortcutRecorder::new(Platform::linux());
        let idle = HotkeyView::resolve(
            &Shortcut::CONTROL_ALT_SPACE,
            &recorder,
            GlobeKeyAction::DoNothing,
            Platform::linux(),
        );
        assert_eq!(idle.display, "Ctrl+Alt+Space");
        assert_eq!(idle.keycaps, vec!["Ctrl", "Alt", "Space"]);
        assert_eq!(idle.help_text, "Current shortcut: Ctrl+Alt+Space");

        recorder.start();
        recorder.handle(RecorderEvent::KeyDown {
            key: KeyCode::SPACE,
            modifiers: ModifierSet::EMPTY,
        });
        let rejected = HotkeyView::resolve(
            &Shortcut::CONTROL_ALT_SPACE,
            &recorder,
            GlobeKeyAction::DoNothing,
            Platform::linux(),
        );
        assert_eq!(rejected.help_text, KeyNames::LINUX.recording_help);
        assert_eq!(
            rejected.message,
            Some(KeyNames::LINUX.missing_modifier_message)
        );
    }

    #[test]
    fn a_lone_modifier_needs_no_accessibility_without_the_permission() {
        let view = HotkeyView::resolve(
            &Shortcut::lone(ModifierKey::AltRight),
            &ShortcutRecorder::new(Platform::linux()),
            GlobeKeyAction::DoNothing,
            Platform::linux(),
        );
        assert!(!view.needs_accessibility);
    }

    #[test]
    fn hot_35_no_fn_warning_where_fn_cannot_be_the_shortcut() {
        let linux = Platform::linux();
        for globe in [
            GlobeKeyAction::Unknown,
            GlobeKeyAction::ChangeInputSource,
            GlobeKeyAction::ShowEmojiAndSymbols,
            GlobeKeyAction::StartDictation,
        ] {
            let view = HotkeyView::resolve(
                &Shortcut::lone(ModifierKey::Fn),
                &ShortcutRecorder::new(linux),
                globe,
                linux,
            );
            assert_eq!(view.fn_warning, None, "{globe:?}");
        }
    }
}
