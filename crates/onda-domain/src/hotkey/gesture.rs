//! Lone-modifier gesture reducer (HOT-21 to HOT-23) and the mapping from
//! modifier-flag events to gesture inputs (HOT-14a, HOT-16).

use std::time::Duration;

use super::shortcut::{KeyCode, ModifierKey};

/// Delay between a lone modifier going down and the gesture firing (HOT-21).
pub const LONE_MODIFIER_DEBOUNCE: Duration = Duration::from_millis(150);

/// Where the gesture is in its press cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GesturePhase {
    /// The configured modifier is not tracked as held.
    #[default]
    Idle,
    /// Held, waiting out the debounce.
    Pending,
    /// Held and fired.
    Active,
}

/// Identifies one scheduled debounce timer, so a tick from a timer the shell
/// failed to cancel in time cannot fire a later hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DebounceToken(u64);

/// One input to the gesture reducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureInput {
    /// The configured modifier went down.
    ModifierDown,
    /// The configured modifier went up.
    ModifierUp,
    /// Any other key, or another modifier, went down.
    OtherKeyDown,
    /// The debounce timer with this token fired.
    DebounceElapsed(DebounceToken),
    /// The event stream was disabled or unregistered (HOT-17, HOT-18).
    StreamDisabled,
}

/// What the caller must do after an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureAction {
    /// Nothing.
    None,
    /// Start a timer that feeds [`GestureInput::DebounceElapsed`] with
    /// `token` after `after`.
    ScheduleDebounce {
        /// Pass back when the timer fires.
        token: DebounceToken,
        /// Always [`LONE_MODIFIER_DEBOUNCE`].
        after: Duration,
    },
    /// Cancel the pending debounce timer.
    CancelDebounce,
    /// Deliver dictation key-down.
    Down,
    /// Deliver dictation key-up.
    Up,
    /// Deliver discard: another key joined the held modifier after it fired
    /// (HOT-33).
    Discard,
}

/// Turns raw lone-modifier events into dictation down, up, and discard
/// (state diagram under "Lone-modifier gesture").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoneModifierGesture {
    state: GestureState,
    // token of the most recent schedule; older ticks are stale.
    next_token: u64,
}

// `chorded` is set once a non-modifier key went down during this hold: the
// timer may not fire for the rest of it (HOT-22), and only the first such key
// after the start discards (HOT-33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GestureState {
    #[default]
    Idle,
    Pending {
        chorded: bool,
    },
    Active {
        chorded: bool,
    },
}

impl LoneModifierGesture {
    /// An idle gesture.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current phase, needed by [`gesture_input_for_flags_change`].
    pub fn phase(&self) -> GesturePhase {
        match self.state {
            GestureState::Idle => GesturePhase::Idle,
            GestureState::Pending { .. } => GesturePhase::Pending,
            GestureState::Active { .. } => GesturePhase::Active,
        }
    }

    /// Returns to idle without emitting anything (fresh registration).
    pub fn reset(&mut self) {
        self.state = GestureState::Idle;
    }

    /// Feeds one input.
    pub fn handle(&mut self, input: GestureInput) -> GestureAction {
        match input {
            GestureInput::ModifierDown => self.modifier_down(),
            GestureInput::ModifierUp => self.modifier_up(),
            GestureInput::OtherKeyDown => self.other_key_down(),
            GestureInput::DebounceElapsed(token) => self.debounce_elapsed(token),
            GestureInput::StreamDisabled => self.stream_disabled(),
        }
    }

    fn modifier_down(&mut self) -> GestureAction {
        // repeated downs while pending or active are ignored (HOT-23).
        if self.state != GestureState::Idle {
            return GestureAction::None;
        }
        self.state = GestureState::Pending { chorded: false };
        self.next_token += 1;
        GestureAction::ScheduleDebounce {
            token: DebounceToken(self.next_token),
            after: LONE_MODIFIER_DEBOUNCE,
        }
    }

    fn modifier_up(&mut self) -> GestureAction {
        let state = self.state;
        self.reset();
        match state {
            GestureState::Idle => GestureAction::None,
            GestureState::Pending { chorded: true } => GestureAction::None,
            GestureState::Pending { chorded: false } => GestureAction::CancelDebounce,
            GestureState::Active { .. } => GestureAction::Up,
        }
    }

    fn other_key_down(&mut self) -> GestureAction {
        match self.state {
            GestureState::Idle => GestureAction::None,
            GestureState::Pending { .. } => {
                self.state = GestureState::Pending { chorded: true };
                GestureAction::CancelDebounce
            }
            GestureState::Active { chorded } => {
                self.state = GestureState::Active { chorded: true };
                if chorded {
                    return GestureAction::None;
                }
                GestureAction::Discard
            }
        }
    }

    fn debounce_elapsed(&mut self, token: DebounceToken) -> GestureAction {
        let current = token.0 == self.next_token;
        let armed = self.state == GestureState::Pending { chorded: false };
        if !armed || !current {
            return GestureAction::None;
        }
        self.state = GestureState::Active { chorded: false };
        GestureAction::Down
    }

    fn stream_disabled(&mut self) -> GestureAction {
        let state = self.state;
        self.reset();
        match state {
            GestureState::Idle => GestureAction::None,
            GestureState::Pending { .. } => GestureAction::CancelDebounce,
            GestureState::Active { .. } => GestureAction::Up,
        }
    }
}

/// A modifier-flags event from the lone-modifier event stream, reduced to
/// what direction detection needs (HOT-16). The shell reads every field from
/// the event's own flags, never from a "current key state" query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierFlagsChange {
    /// W3C code of the key whose flag changed (possibly not a modifier).
    pub key: KeyCode,
    /// Whether the changed key's family flag is set after the event. Left
    /// and right share one family flag; Fn has its own.
    pub family_flag_set: bool,
    /// Whether the changed key's per-side device bit is set after the event.
    /// Ignored for Fn, which has no sides.
    pub side_flag_set: bool,
}

/// Maps a modifier-flags event to the gesture input it represents, or `None`
/// when it should be ignored (HOT-16).
///
/// For the configured key: family flag set while idle is a press; set while
/// already tracking is a release with a sibling still holding the flag;
/// clear is a release. Another modifier's press (its side bit, or the Fn
/// flag) while the configured key is tracked counts as another key.
pub fn gesture_input_for_flags_change(
    change: &ModifierFlagsChange,
    configured: ModifierKey,
    phase: GesturePhase,
) -> Option<GestureInput> {
    let changed = ModifierKey::from_code(change.key.as_str())?;
    if changed == configured {
        if change.family_flag_set && phase == GesturePhase::Idle {
            return Some(GestureInput::ModifierDown);
        }
        return Some(GestureInput::ModifierUp);
    }
    let is_press = match changed {
        ModifierKey::Fn => change.family_flag_set,
        _ => change.side_flag_set,
    };
    if phase == GesturePhase::Idle || !is_press {
        return None;
    }
    Some(GestureInput::OtherKeyDown)
}

/// Maps a key-down from the event stream to a gesture input. Modifier keys
/// are not "other keys"; they arrive as flag changes (HOT-14a).
pub fn gesture_input_for_key_down(key: &KeyCode) -> Option<GestureInput> {
    if ModifierKey::from_code(key.as_str()).is_some() {
        return None;
    }
    Some(GestureInput::OtherKeyDown)
}

#[cfg(test)]
mod tests {
    use super::*;

    use GestureAction as A;
    use GesturePhase::{Active, Idle, Pending};

    // feeds the debounce tick for the most recent schedule.
    fn elapse(g: &mut LoneModifierGesture) -> GestureAction {
        g.handle(GestureInput::DebounceElapsed(DebounceToken(g.next_token)))
    }

    fn schedule(token: u64) -> GestureAction {
        A::ScheduleDebounce {
            token: DebounceToken(token),
            after: LONE_MODIFIER_DEBOUNCE,
        }
    }

    #[derive(Clone, Copy)]
    enum In {
        Down,
        Up,
        Other,
        Tick,
        StaleTick,
        Disabled,
    }

    fn feed(g: &mut LoneModifierGesture, input: In) -> GestureAction {
        match input {
            In::Down => g.handle(GestureInput::ModifierDown),
            In::Up => g.handle(GestureInput::ModifierUp),
            In::Other => g.handle(GestureInput::OtherKeyDown),
            In::Tick => elapse(g),
            In::StaleTick => g.handle(GestureInput::DebounceElapsed(DebounceToken(1))),
            In::Disabled => g.handle(GestureInput::StreamDisabled),
        }
    }

    #[test]
    fn gesture_sequences_hot_21_to_23() {
        struct Case {
            name: &'static str,
            steps: Vec<(In, GestureAction, GesturePhase)>,
        }
        let cases = [
            Case {
                name: "solo hold fires only after debounce, then key-up stops",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Tick, A::Down, Active),
                    (In::Up, A::Up, Idle),
                ],
            },
            Case {
                name: "release before debounce cancels start; late tick is a no-op",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Up, A::CancelDebounce, Idle),
                    (In::Tick, A::None, Idle),
                ],
            },
            Case {
                name: "other key before debounce cancels and suppresses key-up (option+e)",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Other, A::CancelDebounce, Pending),
                    (In::Tick, A::None, Pending),
                    (In::Up, A::None, Idle),
                ],
            },
            Case {
                name: "other key after start discards once, key-up still stops",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Tick, A::Down, Active),
                    (In::Other, A::Discard, Active),
                    (In::Other, A::None, Active),
                    (In::Up, A::Up, Idle),
                ],
            },
            Case {
                name: "repeat down while pending does not re-arm",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Down, A::None, Pending),
                ],
            },
            Case {
                name: "repeat down while active is ignored",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Tick, A::Down, Active),
                    (In::Down, A::None, Active),
                    (In::Tick, A::None, Active),
                ],
            },
            Case {
                name: "inputs are no-ops when idle",
                steps: vec![
                    (In::Up, A::None, Idle),
                    (In::Other, A::None, Idle),
                    (In::Tick, A::None, Idle),
                    (In::Disabled, A::None, Idle),
                ],
            },
            Case {
                name: "stream disabled while pending cancels start and suppresses debounce",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Disabled, A::CancelDebounce, Idle),
                    (In::Tick, A::None, Idle),
                ],
            },
            Case {
                name: "stream disabled while active fires key-up",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Tick, A::Down, Active),
                    (In::Disabled, A::Up, Idle),
                ],
            },
            Case {
                name: "can re-arm after a cancel",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Other, A::CancelDebounce, Pending),
                    (In::Up, A::None, Idle),
                    (In::Down, schedule(2), Pending),
                    (In::Tick, A::Down, Active),
                ],
            },
            Case {
                name: "second other key while pending keeps the timer suppressed",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Other, A::CancelDebounce, Pending),
                    (In::Other, A::CancelDebounce, Pending),
                    (In::Tick, A::None, Pending),
                ],
            },
            Case {
                name: "stale tick from an earlier hold does not fire the next one",
                steps: vec![
                    (In::Down, schedule(1), Pending),
                    (In::Up, A::CancelDebounce, Idle),
                    (In::Down, schedule(2), Pending),
                    (In::StaleTick, A::None, Pending),
                    (In::Tick, A::Down, Active),
                ],
            },
        ];
        for tc in cases {
            let mut g = LoneModifierGesture::new();
            for (i, (input, want, want_phase)) in tc.steps.into_iter().enumerate() {
                assert_eq!(feed(&mut g, input), want, "{} step {i}", tc.name);
                assert_eq!(g.phase(), want_phase, "{} step {i} phase", tc.name);
            }
        }
    }

    #[test]
    fn reset_clears_state() {
        let mut g = LoneModifierGesture::new();
        g.handle(GestureInput::ModifierDown);
        g.handle(GestureInput::OtherKeyDown);
        g.reset();
        assert_eq!(g.phase(), Idle);
        // a chord before the reset does not carry into the next hold
        assert_eq!(g.handle(GestureInput::ModifierDown), schedule(2));
        assert_eq!(elapse(&mut g), A::Down);
    }

    #[test]
    fn debounce_is_150_ms_hot_21() {
        assert_eq!(LONE_MODIFIER_DEBOUNCE, Duration::from_millis(150));
    }

    fn change(code: &str, family: bool, side: bool) -> ModifierFlagsChange {
        ModifierFlagsChange {
            key: KeyCode::parse(code).expect("key"),
            family_flag_set: family,
            side_flag_set: side,
        }
    }

    #[test]
    fn flags_change_mapping_hot_16() {
        use GestureInput::{ModifierDown, ModifierUp, OtherKeyDown};
        use ModifierKey::{AltRight, Fn};

        struct Case {
            name: &'static str,
            change: ModifierFlagsChange,
            phase: GesturePhase,
            configured: ModifierKey,
            want: Option<GestureInput>,
        }
        let case = |name, change, phase, configured, want| Case {
            name,
            change,
            phase,
            configured,
            want,
        };
        let cases = [
            case(
                "configured press from idle",
                change("AltRight", true, false),
                Idle,
                AltRight,
                Some(ModifierDown),
            ),
            case(
                "configured press with sibling already held",
                change("AltRight", true, true),
                Idle,
                AltRight,
                Some(ModifierDown),
            ),
            case(
                "configured release while sibling holds family (active)",
                change("AltRight", true, false),
                Active,
                AltRight,
                Some(ModifierUp),
            ),
            case(
                "configured release while sibling holds family (pending)",
                change("AltRight", true, false),
                Pending,
                AltRight,
                Some(ModifierUp),
            ),
            case(
                "configured release family cleared (active)",
                change("AltRight", false, false),
                Active,
                AltRight,
                Some(ModifierUp),
            ),
            case(
                "spurious configured release family cleared (idle)",
                change("AltRight", false, false),
                Idle,
                AltRight,
                Some(ModifierUp),
            ),
            case(
                "sibling press ignored",
                change("AltLeft", true, true),
                Idle,
                AltRight,
                None,
            ),
            case(
                "sibling release ignored",
                change("AltLeft", false, false),
                Idle,
                AltRight,
                None,
            ),
            case(
                "sibling release while active ignored",
                change("AltLeft", true, false),
                Active,
                AltRight,
                None,
            ),
            case(
                "sibling press while active is a chord",
                change("AltLeft", true, true),
                Active,
                AltRight,
                Some(OtherKeyDown),
            ),
            case(
                "other modifier press while pending is a chord",
                change("MetaLeft", true, true),
                Pending,
                AltRight,
                Some(OtherKeyDown),
            ),
            case(
                "other modifier release while active ignored",
                change("MetaLeft", false, false),
                Active,
                AltRight,
                None,
            ),
            case(
                "other modifier press without device bit ignored",
                change("MetaLeft", true, false),
                Active,
                AltRight,
                None,
            ),
            case(
                "fn press while right option active is a chord",
                change("Fn", true, false),
                Active,
                AltRight,
                Some(OtherKeyDown),
            ),
            case(
                "fn release while right option active ignored",
                change("Fn", false, false),
                Active,
                AltRight,
                None,
            ),
            case(
                "fn press from idle",
                change("Fn", true, false),
                Idle,
                Fn,
                Some(ModifierDown),
            ),
            case(
                "fn release",
                change("Fn", false, false),
                Active,
                Fn,
                Some(ModifierUp),
            ),
            case(
                "option press ignored when fn is configured",
                change("AltLeft", true, false),
                Pending,
                Fn,
                None,
            ),
            case(
                "caps lock ignored while active",
                change("CapsLock", true, true),
                Active,
                AltRight,
                None,
            ),
        ];
        for tc in cases {
            let got = gesture_input_for_flags_change(&tc.change, tc.configured, tc.phase);
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn key_down_mapping_hot_14a() {
        let cases = [
            (
                "letter is another key",
                "KeyE",
                Some(GestureInput::OtherKeyDown),
            ),
            (
                "space is another key",
                "Space",
                Some(GestureInput::OtherKeyDown),
            ),
            ("modifier key-down is not", "ShiftLeft", None),
            ("fn key-down is not", "Fn", None),
        ];
        for (name, code, want) in cases {
            let got = gesture_input_for_key_down(&KeyCode::parse(code).expect("key"));
            assert_eq!(got, want, "{name}");
        }
    }

    // runs flag changes through mapping and reducer, elapsing the debounce
    // whenever the gesture is pending, like the live timer would.
    fn run(
        events: &[(&str, bool, bool)],
        configured: ModifierKey,
    ) -> (LoneModifierGesture, Vec<GestureAction>) {
        let mut g = LoneModifierGesture::new();
        let mut actions = Vec::new();
        for (code, family, side) in events {
            let Some(input) = gesture_input_for_flags_change(
                &change(code, *family, *side),
                configured,
                g.phase(),
            ) else {
                continue;
            };
            actions.push(g.handle(input));
            if g.phase() == Pending {
                actions.push(elapse(&mut g));
            }
        }
        (g, actions)
    }

    #[test]
    fn mapping_and_reducer_end_to_end_hot_16() {
        struct Case {
            name: &'static str,
            events: Vec<(&'static str, bool, bool)>,
            want: Vec<GestureAction>,
        }
        let cases = [
            Case {
                name: "normal press and release returns to idle",
                events: vec![("AltRight", true, true), ("AltRight", false, false)],
                want: vec![schedule(1), A::Down, A::Up],
            },
            Case {
                name: "regression: configured released first while sibling holds",
                events: vec![
                    ("AltLeft", true, true),
                    ("AltRight", true, true),
                    ("AltRight", true, false),
                    ("AltLeft", false, false),
                ],
                want: vec![schedule(1), A::Down, A::Up],
            },
            Case {
                name: "sibling released first",
                events: vec![
                    ("AltLeft", true, true),
                    ("AltRight", true, true),
                    ("AltLeft", true, false),
                    ("AltRight", false, false),
                ],
                want: vec![schedule(1), A::Down, A::Up],
            },
            Case {
                name: "not stuck: a later press re-arms",
                events: vec![
                    ("AltLeft", true, true),
                    ("AltRight", true, true),
                    ("AltRight", true, false),
                    ("AltLeft", false, false),
                    ("AltRight", true, true),
                    ("AltRight", false, false),
                ],
                want: vec![schedule(1), A::Down, A::Up, schedule(2), A::Down, A::Up],
            },
        ];
        for tc in cases {
            let (g, got) = run(&tc.events, ModifierKey::AltRight);
            assert_eq!(got, tc.want, "{}", tc.name);
            assert_eq!(g.phase(), Idle, "{} ends idle", tc.name);
        }
    }

    #[test]
    fn chord_while_pending_never_fires_hot_22() {
        // right option, then command joins before the debounce: the press is
        // a chord, so neither down nor a later stray tick fires.
        let mut g = LoneModifierGesture::new();
        let configured = ModifierKey::AltRight;
        let feed_change = |g: &mut LoneModifierGesture, c: ModifierFlagsChange| {
            let input = gesture_input_for_flags_change(&c, configured, g.phase()).expect("mapped");
            g.handle(input)
        };
        assert_eq!(
            feed_change(&mut g, change("AltRight", true, true)),
            schedule(1)
        );
        assert_eq!(
            feed_change(&mut g, change("MetaLeft", true, true)),
            A::CancelDebounce
        );
        assert_eq!(elapse(&mut g), A::None);
        assert_eq!(
            feed_change(&mut g, change("AltRight", false, false)),
            A::None
        );
        assert_eq!(g.phase(), Idle);
    }
}
