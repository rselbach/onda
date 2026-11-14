//! Dictation press routing by activation mode (HOT-24 to HOT-28, HOT-33) and
//! the provisional-recording confirmation delay (HOT-30).

use std::time::Duration;

use super::gesture::LONE_MODIFIER_DEBOUNCE;
use super::shortcut::Shortcut;
use crate::types::{ActivationMode, MonoTime, SessionPhase};

/// In Automatic mode, a press that started recording and was held at least
/// this long stops on release (push-to-talk) (HOT-27).
pub const AUTOMATIC_HOLD_THRESHOLD: Duration = Duration::from_millis(400);
/// How long a Hold-mode recording stays provisional before it is confirmed
/// (HOT-30).
pub const HOLD_TAP_THRESHOLD: Duration = Duration::from_millis(300);

impl ActivationMode {
    /// Every mode in picker order.
    pub const ALL: [Self; 3] = [Self::Hold, Self::Toggle, Self::Automatic];

    /// The picker label.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Hold => "Hold",
            Self::Toggle => "Toggle",
            Self::Automatic => "Automatic",
        }
    }

    /// The description shown under the picker and in onboarding.
    pub fn description(self) -> &'static str {
        match self {
            Self::Hold => "Press and hold to record, release to stop.",
            Self::Toggle => "Tap once to start recording, tap again to stop.",
            Self::Automatic => "Quick tap toggles recording; holding behaves like push-to-talk.",
        }
    }
}

/// How long after entering Recording a provisional Hold recording waits
/// before it is confirmed (HOT-30): 300 ms, minus the 150 ms debounce a
/// lone-modifier press already spent.
pub fn provisional_confirmation_delay(shortcut: &Shortcut) -> Duration {
    if shortcut.is_lone_modifier() {
        return HOLD_TAP_THRESHOLD.saturating_sub(LONE_MODIFIER_DEBOUNCE);
    }
    HOLD_TAP_THRESHOLD
}

/// What the router needs to know about the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionSnapshot {
    /// The session phase.
    pub phase: SessionPhase,
    /// A hotkey start is still awaiting the microphone permission prompt
    /// (HOT-28).
    pub start_pending: bool,
}

impl SessionSnapshot {
    fn recording(self) -> bool {
        matches!(self.phase, SessionPhase::Recording { .. })
    }

    fn active_or_pending(self) -> bool {
        self.recording() || self.start_pending
    }
}

/// What the session should do in response to a hotkey event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressIntent {
    /// Nothing.
    None,
    /// Attempt a start (DICT-1), provisional for Hold (HOT-29).
    Start {
        /// True for a Hold press.
        provisional: bool,
    },
    /// Stop the recording with the hotkey release tail (DICT-11).
    StopWithTail,
    /// Cancel the recording silently: an unconfirmed Hold tap (HOT-31) or a
    /// discarded combo (HOT-33).
    Cancel,
    /// Abort the start still awaiting the microphone prompt so it does not
    /// begin after release (HOT-28).
    AbortPendingStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Press {
    mode: ActivationMode,
    down_at: MonoTime,
    // automatic mode only: this key-down attempted the start.
    started_recording: bool,
}

/// Routes dictation key-down, key-up, and discard to session intents by
/// activation mode, one press at a time (HOT-24).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PressRouter {
    press: Option<Press>,
}

impl PressRouter {
    /// A router with no press in progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// The dictation hotkey went down at `now` with the activation mode
    /// setting at `mode`. A key-down while a press is still down is ignored,
    /// and the mode is captured for the matching key-up (HOT-24).
    pub fn key_down(
        &mut self,
        mode: ActivationMode,
        session: SessionSnapshot,
        now: MonoTime,
    ) -> PressIntent {
        if self.press.is_some() {
            return PressIntent::None;
        }
        let mut press = Press {
            mode,
            down_at: now,
            started_recording: false,
        };
        let intent = match mode {
            // a recording already running (e.g. from the menu) makes the
            // start a silent no-op; the key-up stops it (HOT-25).
            ActivationMode::Hold if session.recording() => PressIntent::None,
            ActivationMode::Hold => PressIntent::Start { provisional: true },
            ActivationMode::Toggle | ActivationMode::Automatic if session.active_or_pending() => {
                stop_or_abort(session)
            }
            ActivationMode::Toggle | ActivationMode::Automatic => {
                press.started_recording = mode == ActivationMode::Automatic;
                PressIntent::Start { provisional: false }
            }
        };
        self.press = Some(press);
        intent
    }

    /// The dictation hotkey went up at `now`. Uses the mode captured at
    /// key-down, or `current_mode` when no key-down was tracked (HOT-24).
    pub fn key_up(
        &mut self,
        current_mode: ActivationMode,
        session: SessionSnapshot,
        now: MonoTime,
    ) -> PressIntent {
        let press = self.press.take();
        let mode = press.map_or(current_mode, |p| p.mode);
        match mode {
            ActivationMode::Hold => match session.phase {
                SessionPhase::Recording { provisional: true } => PressIntent::Cancel,
                _ => stop_or_abort(session),
            },
            ActivationMode::Toggle => PressIntent::None,
            ActivationMode::Automatic => match press {
                Some(p)
                    if p.started_recording
                        && now.saturating_sub(p.down_at) >= AUTOMATIC_HOLD_THRESHOLD =>
                {
                    stop_or_abort(session)
                }
                _ => PressIntent::None,
            },
        }
    }

    /// Another key joined the held lone-modifier shortcut after it fired
    /// (HOT-33). Cancels the recording only if this press started it:
    /// always in Hold, in Automatic only if this key-down started recording,
    /// never in Toggle. The press stays down until its key-up.
    pub fn discard(&mut self, session: SessionSnapshot) -> PressIntent {
        let Some(press) = self.press.as_mut() else {
            return PressIntent::None;
        };
        let started_by_this_press = match press.mode {
            ActivationMode::Hold => true,
            ActivationMode::Automatic => std::mem::take(&mut press.started_recording),
            ActivationMode::Toggle => false,
        };
        if !started_by_this_press {
            return PressIntent::None;
        }
        if session.recording() {
            return PressIntent::Cancel;
        }
        if session.start_pending {
            return PressIntent::AbortPendingStart;
        }
        PressIntent::None
    }
}

fn stop_or_abort(session: SessionSnapshot) -> PressIntent {
    if session.recording() {
        return PressIntent::StopWithTail;
    }
    if session.start_pending {
        return PressIntent::AbortPendingStart;
    }
    PressIntent::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::shortcut::ModifierKey;

    use ActivationMode::{Automatic, Hold, Toggle};
    use PressIntent::{AbortPendingStart, Cancel, Start, StopWithTail};

    const IDLE: SessionSnapshot = SessionSnapshot {
        phase: SessionPhase::Idle,
        start_pending: false,
    };
    const PENDING: SessionSnapshot = SessionSnapshot {
        phase: SessionPhase::Idle,
        start_pending: true,
    };
    const PROVISIONAL: SessionSnapshot = SessionSnapshot {
        phase: SessionPhase::Recording { provisional: true },
        start_pending: false,
    };
    const RECORDING: SessionSnapshot = SessionSnapshot {
        phase: SessionPhase::Recording { provisional: false },
        start_pending: false,
    };
    const TRANSCRIBING: SessionSnapshot = SessionSnapshot {
        phase: SessionPhase::Transcribing,
        start_pending: false,
    };

    fn ms(n: u64) -> MonoTime {
        Duration::from_millis(n)
    }

    #[derive(Clone, Copy)]
    enum Ev {
        // setting value at the event, session, time in ms.
        Down(ActivationMode, SessionSnapshot, u64),
        Up(ActivationMode, SessionSnapshot, u64),
        Discard(SessionSnapshot),
    }

    #[test]
    fn press_routing_hot_24_to_28_hot_33() {
        use Ev::{Discard, Down, Up};
        struct Case {
            name: &'static str,
            steps: Vec<(Ev, PressIntent)>,
        }
        let cases = [
            // hold (HOT-25)
            Case {
                name: "hold: down starts provisional, quick release cancels",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Up(Hold, PROVISIONAL, 100), Cancel),
                ],
            },
            Case {
                name: "hold: release after confirmation stops with tail",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Up(Hold, RECORDING, 2000), StopWithTail),
                ],
            },
            Case {
                name: "hold: release while start awaits mic prompt aborts it",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Up(Hold, PENDING, 500), AbortPendingStart),
                ],
            },
            Case {
                name: "hold: menu recording makes down a no-op and up stops it",
                steps: vec![
                    (Down(Hold, RECORDING, 0), PressIntent::None),
                    (Up(Hold, RECORDING, 50), StopWithTail),
                ],
            },
            Case {
                name: "hold: down while transcribing attempts start (still transcribing notice)",
                steps: vec![
                    (Down(Hold, TRANSCRIBING, 0), Start { provisional: true }),
                    (Up(Hold, TRANSCRIBING, 50), PressIntent::None),
                ],
            },
            Case {
                name: "hold: release after start failed does nothing",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Up(Hold, IDLE, 500), PressIntent::None),
                ],
            },
            // toggle (HOT-26)
            Case {
                name: "toggle: tap starts, next tap stops with tail",
                steps: vec![
                    (Down(Toggle, IDLE, 0), Start { provisional: false }),
                    (Up(Toggle, RECORDING, 80), PressIntent::None),
                    (Down(Toggle, RECORDING, 5000), StopWithTail),
                    (Up(Toggle, TRANSCRIBING, 5080), PressIntent::None),
                ],
            },
            Case {
                name: "toggle: down while start pending aborts it",
                steps: vec![
                    (Down(Toggle, PENDING, 0), AbortPendingStart),
                    (Up(Toggle, PENDING, 80), PressIntent::None),
                ],
            },
            Case {
                name: "toggle: down while transcribing attempts start",
                steps: vec![(Down(Toggle, TRANSCRIBING, 0), Start { provisional: false })],
            },
            Case {
                name: "toggle: long hold does not stop on release",
                steps: vec![
                    (Down(Toggle, IDLE, 0), Start { provisional: false }),
                    (Up(Toggle, RECORDING, 3000), PressIntent::None),
                ],
            },
            // automatic (HOT-27)
            Case {
                name: "automatic: quick tap leaves it recording",
                steps: vec![
                    (Down(Automatic, IDLE, 0), Start { provisional: false }),
                    (Up(Automatic, RECORDING, 399), PressIntent::None),
                ],
            },
            Case {
                name: "automatic: hold of exactly 400 ms is push-to-talk",
                steps: vec![
                    (Down(Automatic, IDLE, 0), Start { provisional: false }),
                    (Up(Automatic, RECORDING, 400), StopWithTail),
                ],
            },
            Case {
                name: "automatic: long hold with start still pending aborts it",
                steps: vec![
                    (Down(Automatic, IDLE, 0), Start { provisional: false }),
                    (Up(Automatic, PENDING, 900), AbortPendingStart),
                ],
            },
            Case {
                name: "automatic: tap after a latched recording stops it; release does nothing",
                steps: vec![
                    (Down(Automatic, RECORDING, 0), StopWithTail),
                    (Up(Automatic, TRANSCRIBING, 1000), PressIntent::None),
                ],
            },
            Case {
                name: "automatic: down while start pending aborts it",
                steps: vec![(Down(Automatic, PENDING, 0), AbortPendingStart)],
            },
            // one press at a time and mode capture (HOT-24)
            Case {
                name: "second down before up is ignored",
                steps: vec![
                    (Down(Toggle, IDLE, 0), Start { provisional: false }),
                    (Down(Toggle, RECORDING, 10), PressIntent::None),
                    (Up(Toggle, RECORDING, 20), PressIntent::None),
                    (Down(Toggle, RECORDING, 30), StopWithTail),
                ],
            },
            Case {
                name: "mode captured at down survives a setting change (hold then toggle)",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Up(Toggle, PROVISIONAL, 100), Cancel),
                ],
            },
            Case {
                name: "mode captured at down survives a setting change (toggle then hold)",
                steps: vec![
                    (Down(Toggle, IDLE, 0), Start { provisional: false }),
                    (Up(Hold, RECORDING, 100), PressIntent::None),
                ],
            },
            Case {
                name: "untracked up uses the current mode (hold)",
                steps: vec![(Up(Hold, RECORDING, 0), StopWithTail)],
            },
            Case {
                name: "untracked up uses the current mode (automatic)",
                steps: vec![(Up(Automatic, RECORDING, 5000), PressIntent::None)],
            },
            // discard (HOT-33)
            Case {
                name: "discard in hold cancels the recording",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Discard(RECORDING), Cancel),
                    (Up(Hold, IDLE, 900), PressIntent::None),
                ],
            },
            Case {
                name: "discard in hold aborts a pending start",
                steps: vec![
                    (Down(Hold, IDLE, 0), Start { provisional: true }),
                    (Discard(PENDING), AbortPendingStart),
                ],
            },
            Case {
                name: "discard in automatic cancels when this press started it, release then does nothing",
                steps: vec![
                    (Down(Automatic, IDLE, 0), Start { provisional: false }),
                    (Discard(RECORDING), Cancel),
                    (Up(Automatic, IDLE, 900), PressIntent::None),
                ],
            },
            Case {
                name: "discard in automatic keeps a recording latched by an earlier press",
                steps: vec![
                    (Down(Automatic, IDLE, 0), Start { provisional: false }),
                    (Up(Automatic, RECORDING, 100), PressIntent::None),
                    (Down(Automatic, RECORDING, 3000), StopWithTail),
                    (Discard(TRANSCRIBING), PressIntent::None),
                ],
            },
            Case {
                name: "discard in automatic fires once per press",
                steps: vec![
                    (Down(Automatic, IDLE, 0), Start { provisional: false }),
                    (Discard(PENDING), AbortPendingStart),
                    (Discard(PENDING), PressIntent::None),
                ],
            },
            Case {
                name: "discard in toggle never cancels",
                steps: vec![
                    (Down(Toggle, IDLE, 0), Start { provisional: false }),
                    (Discard(RECORDING), PressIntent::None),
                ],
            },
            Case {
                name: "discard without a press does nothing",
                steps: vec![(Discard(RECORDING), PressIntent::None)],
            },
        ];
        for tc in cases {
            let mut router = PressRouter::new();
            for (i, (event, want)) in tc.steps.into_iter().enumerate() {
                let got = match event {
                    Down(mode, session, t) => router.key_down(mode, session, ms(t)),
                    Up(mode, session, t) => router.key_up(mode, session, ms(t)),
                    Discard(session) => router.discard(session),
                };
                assert_eq!(got, want, "{} step {i}", tc.name);
            }
        }
    }

    #[test]
    fn provisional_confirmation_delay_hot_30() {
        let cases = [
            (
                "combo waits the full threshold",
                Shortcut::OPTION_SPACE,
                ms(300),
            ),
            (
                "lone modifier subtracts the debounce",
                Shortcut::lone(ModifierKey::AltRight),
                ms(150),
            ),
            (
                "fn subtracts the debounce",
                Shortcut::lone(ModifierKey::Fn),
                ms(150),
            ),
        ];
        for (name, shortcut, want) in cases {
            assert_eq!(provisional_confirmation_delay(&shortcut), want, "{name}");
        }
    }

    #[test]
    fn activation_mode_copy() {
        let cases = [
            (Hold, "Hold", "Press and hold to record, release to stop."),
            (
                Toggle,
                "Toggle",
                "Tap once to start recording, tap again to stop.",
            ),
            (
                Automatic,
                "Automatic",
                "Quick tap toggles recording; holding behaves like push-to-talk.",
            ),
        ];
        for (mode, want_name, want_description) in cases {
            assert_eq!(mode.display_name(), want_name, "{mode:?}");
            assert_eq!(mode.description(), want_description, "{mode:?}");
        }
        assert_eq!(ActivationMode::default(), Hold);
    }
}
