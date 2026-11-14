//! Shortcut recorder reducer and copy (HOT-9 to HOT-11).

use super::key_names::KeyNames;
use super::shortcut::{
    HeldModifiers, KeyCode, ModifierKey, ModifierSet, Shortcut, ValidationError,
};
use crate::platform::Platform;

/// Button title while not recording (HOT-11).
pub const RECORD_BUTTON_TITLE: &str = "Record Shortcut";
/// Button title while recording; clicking it again cancels (HOT-11).
pub const RECORDING_BUTTON_TITLE: &str = "Press shortcut...";
/// Title of the cancel button shown while recording (HOT-11).
pub const RECORDER_CANCEL_TITLE: &str = "Cancel";

/// A local key event from Onda's own window, expressed platform-neutrally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderEvent {
    /// The modifier flags changed. `key` is the key whose flag changed
    /// (possibly a non-modifier such as `CapsLock`); `held` is the full set
    /// held after the change, Fn included.
    FlagsChanged {
        /// W3C code of the key that changed.
        key: KeyCode,
        /// Modifiers held after the change.
        held: HeldModifiers,
    },
    /// A non-modifier key went down with `modifiers` held. Fn is never part
    /// of a combo, so it has no place here (HOT-9).
    KeyDown {
        /// W3C code of the key.
        key: KeyCode,
        /// Combo modifiers held at the press.
        modifiers: ModifierSet,
    },
}

/// What the recorder decided about one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderOutcome {
    /// Not recording: pass the event through to the window.
    PassThrough,
    /// Keep recording; swallow the event.
    Wait,
    /// Recording ended with a new shortcut; save it, then re-register
    /// (HOT-12). Swallow the event.
    Commit(Shortcut),
    /// Recording ended without a change (Escape); re-register the current
    /// shortcut. Swallow the event.
    Cancel,
    /// The combo failed validation; keep recording and show
    /// [`ShortcutRecorder::message`]. Swallow the event.
    Rejected(ValidationError),
}

/// A message shown under the recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderMessage {
    /// The last combo failed validation (HOT-10).
    Invalid(ValidationError),
    /// The shell could not start listening for keys (HOT-11).
    CouldNotStart,
}

impl RecorderMessage {
    /// The text to show, in the system's key names.
    pub fn text(self, names: &KeyNames) -> &'static str {
        match self {
            Self::Invalid(err) => err.message(names),
            Self::CouldNotStart => "Could not start shortcut recording.",
        }
    }
}

/// The shortcut recorder: a pure reducer over the shell's local key events
/// (HOT-9, HOT-10) plus the state its UI renders (HOT-11).
///
/// The orchestration layer unregisters the global hotkey when
/// [`start`](Self::start) is called and re-registers on commit, cancel, or
/// [`stop`](Self::stop) (HOT-8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutRecorder {
    // the platform registers a modifier on its own; without it a modifier
    // press only ever leads into a combo.
    lone_modifiers: bool,
    recording: bool,
    // modifier pressed while it was the only one held; committed on release.
    armed: Option<ModifierKey>,
    message: Option<RecorderMessage>,
}

impl ShortcutRecorder {
    /// A recorder that is not recording. It commits only shortcuts
    /// `platform` can register.
    pub fn new(platform: Platform) -> Self {
        Self {
            lone_modifiers: platform.lone_modifier_hotkeys,
            recording: false,
            armed: None,
            message: None,
        }
    }

    /// Begins recording, clearing any earlier message and candidate.
    pub fn start(&mut self) {
        self.recording = true;
        self.armed = None;
        self.message = None;
    }

    /// The shell could not start listening for keys: end the capture at
    /// once and show "Could not start shortcut recording." (HOT-11).
    pub fn start_failed(&mut self) {
        self.recording = false;
        self.armed = None;
        self.message = Some(RecorderMessage::CouldNotStart);
    }

    /// Ends recording without a change: the button was clicked again, Cancel
    /// was clicked, or the recorder disappeared (HOT-8). Returns whether a
    /// recording was actually in progress, i.e. whether to re-register.
    pub fn stop(&mut self) -> bool {
        let was_recording = self.recording;
        self.recording = false;
        self.armed = None;
        was_recording
    }

    /// Whether recording is in progress.
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// The message to show under the recorder, if any.
    pub fn message(&self) -> Option<RecorderMessage> {
        self.message
    }

    /// The record button's title (HOT-11).
    pub fn button_title(&self) -> &'static str {
        if self.recording {
            return RECORDING_BUTTON_TITLE;
        }
        RECORD_BUTTON_TITLE
    }

    /// The help text (HOT-11) in the system's key names: instructions
    /// while recording, otherwise `Current shortcut: ⌥ Space`.
    pub fn help_text(&self, current: &Shortcut, names: &KeyNames) -> String {
        if self.recording {
            return names.recording_help.to_owned();
        }
        format!("Current shortcut: {}", current.display(names))
    }

    /// Feeds one local key event.
    pub fn handle(&mut self, event: RecorderEvent) -> RecorderOutcome {
        if !self.recording {
            return RecorderOutcome::PassThrough;
        }
        match event {
            RecorderEvent::FlagsChanged { key, held } => self.flags_changed(&key, held),
            RecorderEvent::KeyDown { key, modifiers } => self.key_down(key, modifiers),
        }
    }

    fn flags_changed(&mut self, key: &KeyCode, held: HeldModifiers) -> RecorderOutcome {
        if !self.lone_modifiers {
            return RecorderOutcome::Wait;
        }
        // caps lock and other non-modifier flag changes are ignored.
        let Some(modifier) = ModifierKey::from_code(key.as_str()) else {
            return RecorderOutcome::Wait;
        };
        if held.contains(modifier) {
            if held == HeldModifiers::only(modifier) {
                self.armed = Some(modifier);
            }
            return RecorderOutcome::Wait;
        }
        // any release clears the candidate; only the armed key released with
        // nothing else held commits.
        let armed = self.armed.take();
        if armed != Some(modifier) || !held.is_empty() {
            return RecorderOutcome::Wait;
        }
        self.finish(Shortcut::lone(modifier))
    }

    fn key_down(&mut self, key: KeyCode, modifiers: ModifierSet) -> RecorderOutcome {
        self.armed = None;
        if key == KeyCode::ESCAPE {
            self.recording = false;
            self.message = None;
            return RecorderOutcome::Cancel;
        }
        match Shortcut::new(key, modifiers) {
            Ok(shortcut) => self.finish(shortcut),
            Err(err) => {
                self.message = Some(RecorderMessage::Invalid(err));
                RecorderOutcome::Rejected(err)
            }
        }
    }

    fn finish(&mut self, shortcut: Shortcut) -> RecorderOutcome {
        self.recording = false;
        self.message = None;
        RecorderOutcome::Commit(shortcut)
    }

    #[cfg(test)]
    fn armed(&self) -> Option<ModifierKey> {
        self.armed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::shortcut::Modifier;
    use crate::privacy_naming::PrivacyNaming;

    use Modifier::{Alt, Control, Meta, Shift};
    use ModifierKey::{AltLeft, AltRight, Fn, MetaRight};

    fn key(code: &str) -> KeyCode {
        KeyCode::parse(code).expect("test key code")
    }

    fn mods(list: &[Modifier]) -> ModifierSet {
        list.iter().copied().collect()
    }

    fn flags(code: &str, held: &[Modifier], fn_key: bool) -> RecorderEvent {
        RecorderEvent::FlagsChanged {
            key: key(code),
            held: HeldModifiers::new(mods(held), fn_key),
        }
    }

    fn down(code: &str, held: &[Modifier]) -> RecorderEvent {
        RecorderEvent::KeyDown {
            key: key(code),
            modifiers: mods(held),
        }
    }

    fn combo(code: &str, held: &[Modifier]) -> Shortcut {
        Shortcut::new(key(code), mods(held)).expect("valid")
    }

    fn macos_recorder() -> ShortcutRecorder {
        ShortcutRecorder::new(Platform::macos(PrivacyNaming::Accessibility))
    }

    struct Step {
        event: RecorderEvent,
        want: RecorderOutcome,
        want_armed: Option<ModifierKey>,
    }

    fn step(event: RecorderEvent, want: RecorderOutcome, want_armed: Option<ModifierKey>) -> Step {
        Step {
            event,
            want,
            want_armed,
        }
    }

    use RecorderOutcome::{Cancel, Commit, Rejected, Wait};

    #[test]
    fn recorder_sequences_hot_9_hot_10() {
        struct Case {
            name: &'static str,
            steps: Vec<Step>,
        }
        let cases = [
            Case {
                name: "solo right option press then release commits it",
                steps: vec![
                    step(flags("AltRight", &[Alt], false), Wait, Some(AltRight)),
                    step(
                        flags("AltRight", &[], false),
                        Commit(Shortcut::lone(AltRight)),
                        None,
                    ),
                ],
            },
            Case {
                name: "solo left option press then release commits left option",
                steps: vec![
                    step(flags("AltLeft", &[Alt], false), Wait, Some(AltLeft)),
                    step(
                        flags("AltLeft", &[], false),
                        Commit(Shortcut::lone(AltLeft)),
                        None,
                    ),
                ],
            },
            Case {
                name: "solo fn press then release commits fn",
                steps: vec![
                    step(flags("Fn", &[], true), Wait, Some(Fn)),
                    step(flags("Fn", &[], false), Commit(Shortcut::lone(Fn)), None),
                ],
            },
            Case {
                name: "fn held with option does not commit fn",
                steps: vec![
                    step(flags("Fn", &[], true), Wait, Some(Fn)),
                    step(flags("AltLeft", &[Alt], true), Wait, Some(Fn)),
                    step(flags("Fn", &[Alt], false), Wait, None),
                ],
            },
            Case {
                name: "fn held with option then key records option combo without fn",
                steps: vec![
                    step(flags("Fn", &[], true), Wait, Some(Fn)),
                    step(flags("AltLeft", &[Alt], true), Wait, Some(Fn)),
                    step(
                        down("ArrowLeft", &[Alt]),
                        Commit(combo("ArrowLeft", &[Alt])),
                        None,
                    ),
                ],
            },
            Case {
                name: "regression: left option then space records option space",
                steps: vec![
                    step(flags("AltLeft", &[Alt], false), Wait, Some(AltLeft)),
                    step(down("Space", &[Alt]), Commit(Shortcut::OPTION_SPACE), None),
                ],
            },
            Case {
                name: "right command then e records command e",
                steps: vec![
                    step(flags("MetaRight", &[Meta], false), Wait, Some(MetaRight)),
                    step(down("KeyE", &[Meta]), Commit(combo("KeyE", &[Meta])), None),
                ],
            },
            Case {
                name: "escape while modifier armed cancels",
                steps: vec![
                    step(flags("AltRight", &[Alt], false), Wait, Some(AltRight)),
                    step(down("Escape", &[Alt]), Cancel, None),
                ],
            },
            Case {
                name: "escape with no modifier cancels",
                steps: vec![step(down("Escape", &[]), Cancel, None)],
            },
            Case {
                name: "bare key is rejected and recording continues",
                steps: vec![
                    step(
                        down("KeyA", &[]),
                        Rejected(ValidationError::MissingRequiredModifier),
                        None,
                    ),
                    step(
                        down("KeyA", &[Control]),
                        Commit(combo("KeyA", &[Control])),
                        None,
                    ),
                ],
            },
            Case {
                name: "shift-only combo is rejected",
                steps: vec![step(
                    down("KeyA", &[Shift]),
                    Rejected(ValidationError::MissingRequiredModifier),
                    None,
                )],
            },
            Case {
                name: "second modifier while armed does not commit on release",
                steps: vec![
                    step(flags("AltRight", &[Alt], false), Wait, Some(AltRight)),
                    step(
                        flags("MetaRight", &[Alt, Meta], false),
                        Wait,
                        Some(AltRight),
                    ),
                    step(flags("MetaRight", &[Alt], false), Wait, None),
                    step(flags("AltRight", &[], false), Wait, None),
                ],
            },
            Case {
                name: "arming right option then tapping shift commits nothing later",
                steps: vec![
                    step(flags("AltRight", &[Alt], false), Wait, Some(AltRight)),
                    step(
                        flags("ShiftLeft", &[Alt, Shift], false),
                        Wait,
                        Some(AltRight),
                    ),
                    step(flags("ShiftLeft", &[Alt], false), Wait, None),
                    step(flags("AltRight", &[], false), Wait, None),
                ],
            },
            Case {
                name: "non-modifier flag change is ignored",
                steps: vec![step(flags("CapsLock", &[], false), Wait, None)],
            },
            Case {
                name: "caps lock while armed keeps the candidate",
                steps: vec![
                    step(flags("AltRight", &[Alt], false), Wait, Some(AltRight)),
                    step(flags("CapsLock", &[Alt], false), Wait, Some(AltRight)),
                    step(
                        flags("AltRight", &[], false),
                        Commit(Shortcut::lone(AltRight)),
                        None,
                    ),
                ],
            },
            Case {
                name: "release without armed candidate waits",
                steps: vec![step(flags("AltRight", &[], false), Wait, None)],
            },
        ];
        for tc in cases {
            let mut recorder = macos_recorder();
            recorder.start();
            for (i, s) in tc.steps.into_iter().enumerate() {
                let got = recorder.handle(s.event);
                assert_eq!(got, s.want, "{} step {i}", tc.name);
                assert_eq!(recorder.armed(), s.want_armed, "{} step {i} armed", tc.name);
            }
        }
    }

    #[test]
    fn a_lone_modifier_never_commits_where_none_registers_hot_9() {
        struct Case {
            name: &'static str,
            steps: Vec<(RecorderEvent, RecorderOutcome)>,
            want_recording: bool,
        }
        let cases = [
            Case {
                name: "solo right alt press then release keeps recording",
                steps: vec![
                    (flags("AltRight", &[Alt], false), Wait),
                    (flags("AltRight", &[], false), Wait),
                ],
                want_recording: true,
            },
            Case {
                name: "every modifier key alone keeps recording",
                steps: ModifierKey::ALL
                    .into_iter()
                    .flat_map(|modifier| {
                        let change = |held| RecorderEvent::FlagsChanged {
                            key: modifier.into(),
                            held,
                        };
                        [
                            (change(HeldModifiers::only(modifier)), Wait),
                            (change(HeldModifiers::NONE), Wait),
                        ]
                    })
                    .collect(),
                want_recording: true,
            },
            Case {
                name: "modifiers then a key still record the combo",
                steps: vec![
                    (flags("ControlLeft", &[Control], false), Wait),
                    (flags("AltLeft", &[Control, Alt], false), Wait),
                    (
                        down("Space", &[Control, Alt]),
                        Commit(Shortcut::CONTROL_ALT_SPACE),
                    ),
                ],
                want_recording: false,
            },
            Case {
                name: "a lone press, a rejected key, then a combo",
                steps: vec![
                    (flags("AltRight", &[Alt], false), Wait),
                    (flags("AltRight", &[], false), Wait),
                    (
                        down("KeyA", &[]),
                        Rejected(ValidationError::MissingRequiredModifier),
                    ),
                    (down("KeyA", &[Meta]), Commit(combo("KeyA", &[Meta]))),
                ],
                want_recording: false,
            },
            Case {
                name: "escape still cancels",
                steps: vec![
                    (flags("AltRight", &[Alt], false), Wait),
                    (down("Escape", &[Alt]), Cancel),
                ],
                want_recording: false,
            },
        ];
        for tc in cases {
            let mut recorder = ShortcutRecorder::new(Platform::linux());
            recorder.start();
            for (i, (event, want)) in tc.steps.into_iter().enumerate() {
                assert_eq!(recorder.handle(event), want, "{} step {i}", tc.name);
                assert_eq!(recorder.armed(), None, "{} step {i} armed", tc.name);
            }
            assert_eq!(recorder.is_recording(), tc.want_recording, "{}", tc.name);
        }
    }

    #[test]
    fn the_rejection_offers_only_what_the_platform_registers_hot_10() {
        let cases = [
            (
                Platform::macos(PrivacyNaming::Accessibility),
                "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn.",
            ),
            (
                Platform::linux(),
                "Shortcut must include Ctrl, Alt, or Super.",
            ),
        ];
        for (platform, want) in cases {
            let mut recorder = ShortcutRecorder::new(platform);
            recorder.start();
            recorder.handle(down("KeyA", &[Shift]));
            assert_eq!(
                recorder.message().map(|m| m.text(platform.key_names)),
                Some(want),
                "{}",
                platform.os_name
            );
        }
    }

    #[test]
    fn recorder_state_after_outcomes_hot_10_hot_11() {
        struct Case {
            name: &'static str,
            events: Vec<RecorderEvent>,
            want_recording: bool,
            want_message: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "rejected combo shows message and keeps recording",
                events: vec![down("KeyA", &[Shift])],
                want_recording: true,
                want_message: Some(
                    "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn.",
                ),
            },
            Case {
                name: "escape cancels and clears the message",
                events: vec![down("KeyA", &[]), down("Escape", &[])],
                want_recording: false,
                want_message: None,
            },
            Case {
                name: "commit clears the message",
                events: vec![down("KeyA", &[]), down("KeyA", &[Meta])],
                want_recording: false,
                want_message: None,
            },
            Case {
                name: "plain armed press keeps recording",
                events: vec![flags("AltRight", &[Alt], false)],
                want_recording: true,
                want_message: None,
            },
        ];
        for tc in cases {
            let mut recorder = macos_recorder();
            recorder.start();
            for event in tc.events {
                recorder.handle(event);
            }
            assert_eq!(recorder.is_recording(), tc.want_recording, "{}", tc.name);
            assert_eq!(
                recorder.message().map(|m| m.text(&KeyNames::MACOS)),
                tc.want_message,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn recorder_lifecycle_hot_8_hot_11() {
        let current = Shortcut::OPTION_SPACE;

        let mut recorder = macos_recorder();
        assert_eq!(
            recorder.handle(down("KeyA", &[Meta])),
            RecorderOutcome::PassThrough,
            "idle passes through"
        );
        assert_eq!(recorder.button_title(), "Record Shortcut");
        assert_eq!(
            recorder.help_text(&current, &KeyNames::MACOS),
            "Current shortcut: ⌥ Space"
        );
        assert_eq!(
            recorder.help_text(&Shortcut::CONTROL_ALT_SPACE, &KeyNames::LINUX),
            "Current shortcut: Ctrl+Alt+Space"
        );
        assert!(
            !recorder.stop(),
            "stop while idle does not ask to re-register"
        );

        recorder.start();
        assert_eq!(recorder.button_title(), "Press shortcut...");
        assert_eq!(
            recorder.help_text(&current, &KeyNames::MACOS),
            "Press a key with Command, Control, or Option. A lone modifier like Right Option or Fn also works. Escape cancels."
        );
        assert_eq!(
            recorder.help_text(&current, &KeyNames::LINUX),
            "Press a key with Ctrl, Alt, or Super. Escape cancels."
        );
        recorder.handle(flags("AltRight", &[Alt], false));
        assert!(recorder.stop(), "stop while recording asks to re-register");
        assert_eq!(
            recorder.handle(flags("AltRight", &[], false)),
            RecorderOutcome::PassThrough,
            "events after stop pass through",
        );

        recorder.start();
        assert_eq!(
            recorder.handle(flags("AltRight", &[], false)),
            Wait,
            "restart drops a stale candidate",
        );

        recorder.handle(down("KeyA", &[]));
        recorder.start();
        assert_eq!(recorder.message(), None, "start clears the message");

        recorder.start_failed();
        assert!(!recorder.is_recording());
        assert_eq!(
            recorder.message().map(|m| m.text(&KeyNames::MACOS)),
            Some("Could not start shortcut recording.")
        );
    }
}
