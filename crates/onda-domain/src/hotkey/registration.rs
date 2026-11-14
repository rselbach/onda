//! Global hotkey registration policies: event-stream re-enable bound
//! (HOT-17), failure notices and the Accessibility retry episode (HOT-19),
//! failures reported before wiring (HOT-20), and hotkey-id routing.

use std::time::Duration;

use super::shortcut::{ModifierSet, Shortcut};
use crate::platform::Platform;
use crate::types::MonoTime;

/// Most re-enables tolerated within [`REENABLE_WINDOW`] (HOT-17).
pub const MAX_REENABLES: usize = 5;
/// Rolling window for counting event-stream disables (HOT-17).
pub const REENABLE_WINDOW: Duration = Duration::from_secs(5);
/// How often to re-check Accessibility trust while a lone-modifier failure
/// is pending (HOT-19).
pub const ACCESSIBILITY_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What to do after the OS disabled the event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDisableDecision {
    /// Re-enable the stream.
    Reenable,
    /// Too many disables: tear the stream down, mark it dead, and report a
    /// registration failure.
    GiveUp,
    /// The stream was already given up on; a stale callback, do nothing.
    AlreadyDead,
}

/// Bounds the disable/re-enable loop of the lone-modifier event stream
/// (HOT-17): at most [`MAX_REENABLES`] re-enables in any rolling
/// [`REENABLE_WINDOW`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamReenablePolicy {
    // in-window disable times, oldest first.
    recent: Vec<MonoTime>,
    dead: bool,
}

impl StreamReenablePolicy {
    /// A fresh policy, as after a new registration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a disable at `now` and decides what to do.
    pub fn on_disabled(&mut self, now: MonoTime) -> StreamDisableDecision {
        if self.dead {
            return StreamDisableDecision::AlreadyDead;
        }
        let cutoff = now.saturating_sub(REENABLE_WINDOW);
        self.recent.retain(|t| *t >= cutoff);
        self.recent.push(now);
        if self.recent.len() <= MAX_REENABLES {
            return StreamDisableDecision::Reenable;
        }
        self.dead = true;
        self.recent.clear();
        StreamDisableDecision::GiveUp
    }

    /// A real event arrived, so the stream is healthy: forget past disables.
    pub fn on_event(&mut self) {
        self.recent.clear();
    }

    /// Whether the stream was given up on.
    pub fn is_dead(&self) -> bool {
        self.dead
    }
}

/// Title of every registration-failure notification (HOT-19).
pub const REGISTRATION_FAILURE_TITLE: &str = "Hotkey Registration Failed";

/// A non-critical notification to post about a registration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationFailureNotice {
    /// Always [`REGISTRATION_FAILURE_TITLE`].
    pub title: &'static str,
    /// Names the shortcut and, where the platform has a name for it, the
    /// OS status code.
    pub message: String,
}

impl RegistrationFailureNotice {
    /// The notice for `shortcut` failing with OS status `status` (HOT-19).
    /// A lone modifier is explained by the typing permission only where
    /// the platform has one.
    pub fn new(shortcut: &Shortcut, status: i32, platform: Platform) -> Self {
        let display = shortcut.display();
        let reason = match platform.typing_permission {
            Some(naming) if shortcut.is_lone_modifier() => format!(
                "Could not register {display} as a global hotkey. {}",
                naming.lone_modifier_requirement()
            ),
            _ => format!(
                "Could not register the global hotkey ({display}). It may be in use by another application."
            ),
        };
        let message = match platform.error_code_term {
            Some(term) => format!("{reason} {term}: {status}"),
            None => reason,
        };
        Self {
            title: REGISTRATION_FAILURE_TITLE,
            message,
        }
    }
}

/// Tracks a lone-modifier registration-failure episode and decides when to
/// notify and when to retry (HOT-19).
///
/// While [`is_pending`](Self::is_pending), the caller checks Accessibility
/// trust every [`ACCESSIBILITY_POLL_INTERVAL`] and when Onda becomes active,
/// feeding each result to [`record_trust`](Self::record_trust). Call
/// [`reset`](Self::reset) on a successful registration and before every
/// explicit registration (settings change, capture start or end).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegistrationRetry {
    // trust last observed during the episode; `None` when nothing is pending.
    last_trust: Option<bool>,
}

impl RegistrationRetry {
    /// No episode pending.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a lone-modifier failure is pending, i.e. whether to poll.
    pub fn is_pending(&self) -> bool {
        self.last_trust.is_some()
    }

    /// Records that registering `shortcut` failed with `status` while
    /// Accessibility trust was `trusted`. Returns the notice to post, if
    /// any: always for a combo (which also ends any lone-modifier episode),
    /// and only for the first failure of a lone-modifier episode.
    /// `platform` words the notice.
    pub fn record_failure(
        &mut self,
        shortcut: &Shortcut,
        status: i32,
        trusted: bool,
        platform: Platform,
    ) -> Option<RegistrationFailureNotice> {
        let notify = self.record(shortcut.is_lone_modifier(), trusted);
        notify.then(|| RegistrationFailureNotice::new(shortcut, status, platform))
    }

    /// Records a trust check. Returns true when the caller should register
    /// again: an episode is pending and trust just went from untrusted to
    /// trusted. A failure while already trusted waits for the user to
    /// toggle Accessibility rather than retrying on a timer.
    pub fn record_trust(&mut self, trusted: bool) -> bool {
        let Some(was_trusted) = self.last_trust else {
            return false;
        };
        self.last_trust = Some(trusted);
        !was_trusted && trusted
    }

    /// Ends the episode.
    pub fn reset(&mut self) {
        self.last_trust = None;
    }

    fn record(&mut self, lone_modifier: bool, trusted: bool) -> bool {
        if !lone_modifier {
            self.last_trust = None;
            return true;
        }
        let new_episode = self.last_trust.is_none();
        self.last_trust = Some(trusted);
        new_episode
    }
}

/// Holds a registration failure reported before the core wired its handler,
/// keeping only the most recent one (HOT-20).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FailureMailbox {
    handler_ready: bool,
    held: Option<i32>,
}

impl FailureMailbox {
    /// A mailbox with no handler yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// A failure with `status` happened. Returns it when the handler is
    /// ready to receive it now; otherwise holds it, replacing any older one.
    pub fn report(&mut self, status: i32) -> Option<i32> {
        if self.handler_ready {
            return Some(status);
        }
        self.held = Some(status);
        None
    }

    /// The handler is wired. Returns the held failure to deliver, if any.
    pub fn handler_ready(&mut self) -> Option<i32> {
        self.handler_ready = true;
        self.held.take()
    }
}

/// Id the shell registers the dictation shortcut under.
pub const DICTATION_HOTKEY_ID: u32 = 1;
/// Id the shell registers Escape under while a session runs (HOT-34).
pub const CANCEL_HOTKEY_ID: u32 = 2;

/// The modifier sets to claim Escape with while a session is non-idle
/// (HOT-34): plain Escape first, then Escape with the shortcut's own
/// modifiers, so a user still holding them in Hold mode can cancel. A lone
/// modifier contributes its family (Right Option gives `⌥`); Fn has no combo
/// modifier and adds nothing.
///
/// (Deviation: Escape with the shortcut's own modifiers also cancels during a
/// session; HOT-34 cancels on plain Escape only.)
pub fn cancel_hotkey_modifiers(shortcut: &Shortcut) -> Vec<ModifierSet> {
    let own = match shortcut.lone_modifier() {
        Some(key) => key
            .modifier()
            .map_or(ModifierSet::EMPTY, |m| ModifierSet::EMPTY.with(m)),
        None => shortcut.modifiers(),
    };
    if own.is_empty() {
        return vec![ModifierSet::EMPTY];
    }
    vec![ModifierSet::EMPTY, own]
}

/// What a global-hotkey press or release means. Two hotkeys share one
/// handler: the dictation shortcut and the session-only Escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyRoute {
    /// Dictation shortcut pressed.
    DictationDown,
    /// Dictation shortcut released.
    DictationUp,
    /// Escape pressed: cancel the session (HOT-34).
    Cancel,
    /// Nothing to do.
    Ignore,
}

impl HotkeyRoute {
    /// Routes an event for registered hotkey `id`. Escape fires on press;
    /// its release does nothing.
    pub fn route(id: u32, pressed: bool) -> Self {
        match (id, pressed) {
            (DICTATION_HOTKEY_ID, true) => Self::DictationDown,
            (DICTATION_HOTKEY_ID, false) => Self::DictationUp,
            (CANCEL_HOTKEY_ID, true) => Self::Cancel,
            _ => Self::Ignore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::shortcut::{KeyCode, Modifier, ModifierKey};
    use crate::privacy_naming::PrivacyNaming;

    fn secs(s: f64) -> MonoTime {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn reenable_decisions_hot_17() {
        struct Case {
            name: &'static str,
            // disables already accepted, then the one under test.
            earlier: Vec<f64>,
            disable: f64,
            want: StreamDisableDecision,
            want_history: usize,
        }
        let cases = [
            Case {
                name: "first disable re-enables",
                earlier: vec![],
                disable: 1000.0,
                want: StreamDisableDecision::Reenable,
                want_history: 1,
            },
            Case {
                name: "fifth disable within window still re-enables",
                earlier: vec![1000.0, 1001.0, 1002.0, 1003.0],
                disable: 1004.0,
                want: StreamDisableDecision::Reenable,
                want_history: 5,
            },
            Case {
                name: "sixth disable within window gives up",
                earlier: vec![1000.0, 1001.0, 1002.0, 1003.0, 1004.0],
                disable: 1004.5,
                want: StreamDisableDecision::GiveUp,
                want_history: 0,
            },
            Case {
                name: "disables outside the window are dropped",
                earlier: vec![1000.0, 1005.0, 1016.0, 1018.0],
                disable: 1020.0,
                want: StreamDisableDecision::Reenable,
                want_history: 3,
            },
            Case {
                name: "disable exactly at the cutoff age is kept",
                earlier: vec![1015.0],
                disable: 1020.0,
                want: StreamDisableDecision::Reenable,
                want_history: 2,
            },
            Case {
                name: "disable just past the cutoff age is dropped",
                earlier: vec![1014.9],
                disable: 1020.0,
                want: StreamDisableDecision::Reenable,
                want_history: 1,
            },
            Case {
                name: "early clock values do not underflow",
                earlier: vec![0.0, 1.0],
                disable: 2.0,
                want: StreamDisableDecision::Reenable,
                want_history: 3,
            },
        ];
        for tc in cases {
            let mut policy = StreamReenablePolicy::new();
            for t in &tc.earlier {
                assert_eq!(
                    policy.on_disabled(secs(*t)),
                    StreamDisableDecision::Reenable,
                    "{} setup",
                    tc.name
                );
            }
            assert_eq!(policy.on_disabled(secs(tc.disable)), tc.want, "{}", tc.name);
            assert_eq!(policy.recent.len(), tc.want_history, "{} history", tc.name);
        }
    }

    #[test]
    fn runaway_loop_gives_up_and_stays_dead_hot_17() {
        let mut policy = StreamReenablePolicy::new();
        for i in 0..5 {
            let got = policy.on_disabled(secs(2000.0 + f64::from(i) * 0.5));
            assert_eq!(got, StreamDisableDecision::Reenable, "disable #{}", i + 1);
        }
        assert_eq!(
            policy.on_disabled(secs(2002.5)),
            StreamDisableDecision::GiveUp
        );
        assert!(policy.is_dead());
        assert_eq!(
            policy.on_disabled(secs(2100.0)),
            StreamDisableDecision::AlreadyDead,
            "a stale callback after give-up does nothing",
        );

        let mut fresh = StreamReenablePolicy::new();
        assert_eq!(
            fresh.on_disabled(secs(2100.0)),
            StreamDisableDecision::Reenable,
            "new registration starts fresh"
        );
    }

    #[test]
    fn spread_out_disables_recover_after_window_hot_17() {
        let mut policy = StreamReenablePolicy::new();
        for i in 0..5 {
            policy.on_disabled(secs(2000.0 + f64::from(i) * 0.5));
        }
        assert_eq!(
            policy.on_disabled(secs(2100.0)),
            StreamDisableDecision::Reenable
        );
        assert_eq!(policy.recent.len(), 1);
    }

    #[test]
    fn real_event_clears_disable_history_hot_17() {
        let mut policy = StreamReenablePolicy::new();
        for i in 0..5 {
            policy.on_disabled(secs(3000.0 + f64::from(i) * 0.1));
        }
        policy.on_event();
        assert_eq!(
            policy.on_disabled(secs(3000.6)),
            StreamDisableDecision::Reenable,
            "history cleared by a real event",
        );
    }

    #[test]
    fn reenable_constants() {
        assert_eq!(MAX_REENABLES, 5);
        assert_eq!(REENABLE_WINDOW, Duration::from_secs(5));
        assert_eq!(ACCESSIBILITY_POLL_INTERVAL, Duration::from_secs(2));
    }

    #[derive(Clone, Copy)]
    enum Step {
        Failure { lone: bool, trusted: bool },
        Trust(bool),
        Reset,
    }

    #[test]
    fn retry_decisions_hot_19() {
        use Step::{Failure, Reset, Trust};
        struct Case {
            name: &'static str,
            steps: Vec<Step>,
            // per step: notify for a failure, retry for a trust check, false for reset.
            want: Vec<bool>,
            want_pending: bool,
        }
        let cases = [
            Case {
                name: "lone modifier fails without accessibility: notify and stay pending",
                steps: vec![Failure {
                    lone: true,
                    trusted: false,
                }],
                want: vec![true],
                want_pending: true,
            },
            Case {
                name: "combo failure: notify, nothing to retry",
                steps: vec![
                    Failure {
                        lone: false,
                        trusted: false,
                    },
                    Trust(true),
                ],
                want: vec![true, false],
                want_pending: false,
            },
            Case {
                name: "combo failure ends a pending lone-modifier episode",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                    Failure {
                        lone: false,
                        trusted: false,
                    },
                    Trust(true),
                ],
                want: vec![true, true, false],
                want_pending: false,
            },
            Case {
                name: "combo failures always notify",
                steps: vec![
                    Failure {
                        lone: false,
                        trusted: true,
                    },
                    Failure {
                        lone: false,
                        trusted: true,
                    },
                ],
                want: vec![true, true],
                want_pending: false,
            },
            Case {
                name: "accessibility still missing: keep waiting",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                    Trust(false),
                    Trust(false),
                ],
                want: vec![true, false, false],
                want_pending: true,
            },
            Case {
                name: "accessibility granted: re-register once",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                    Trust(false),
                    Trust(true),
                    Trust(true),
                ],
                want: vec![true, false, true, false],
                want_pending: true,
            },
            Case {
                name: "automatic retry fails again: no second notification",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                    Trust(true),
                    Failure {
                        lone: true,
                        trusted: true,
                    },
                    Trust(true),
                ],
                want: vec![true, true, false, false],
                want_pending: true,
            },
            Case {
                name: "stream died while trusted: no retry until accessibility is toggled",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: true,
                    },
                    Trust(true),
                    Trust(true),
                    Trust(false),
                    Trust(true),
                ],
                want: vec![true, false, false, false, true],
                want_pending: true,
            },
            Case {
                name: "trust check with nothing pending: no retry",
                steps: vec![Trust(true)],
                want: vec![false],
                want_pending: false,
            },
            Case {
                name: "reset after a successful retry stops polling",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                    Trust(true),
                    Reset,
                ],
                want: vec![true, true, false],
                want_pending: false,
            },
            Case {
                name: "reset ends the episode: the next failure notifies again",
                steps: vec![
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                    Reset,
                    Trust(true),
                    Failure {
                        lone: true,
                        trusted: false,
                    },
                ],
                want: vec![true, false, false, true],
                want_pending: true,
            },
        ];
        let lone = Shortcut::lone(ModifierKey::AltRight);
        let combo = Shortcut::default();
        for tc in cases {
            let mut retry = RegistrationRetry::new();
            let mut got = Vec::new();
            for step in tc.steps {
                match step {
                    Failure {
                        lone: is_lone,
                        trusted,
                    } => {
                        let shortcut = if is_lone { &lone } else { &combo };
                        got.push(
                            retry
                                .record_failure(
                                    shortcut,
                                    -9874,
                                    trusted,
                                    Platform::macos(PrivacyNaming::Accessibility),
                                )
                                .is_some(),
                        );
                    }
                    Trust(trusted) => got.push(retry.record_trust(trusted)),
                    Reset => {
                        retry.reset();
                        got.push(false);
                    }
                }
            }
            assert_eq!(got, tc.want, "{}", tc.name);
            assert_eq!(retry.is_pending(), tc.want_pending, "{}", tc.name);
        }
    }

    #[test]
    fn failure_notice_copy_hot_19() {
        let macos_26 = Platform::macos(PrivacyNaming::Accessibility);
        let macos_27 = Platform::macos(PrivacyNaming::DeviceControlAndDataAccess);
        let cases = [
            (
                "combo",
                Shortcut::default(),
                -9878,
                macos_26,
                "Could not register the global hotkey (⌥ Space). It may be in use by another application. OSStatus: -9878",
            ),
            (
                "lone modifier",
                Shortcut::lone(ModifierKey::AltRight),
                -9874,
                macos_26,
                "Could not register Right Option as a global hotkey. Lone modifier keys require Accessibility permission in System Settings. OSStatus: -9874",
            ),
            (
                "fn",
                Shortcut::lone(ModifierKey::Fn),
                0,
                macos_26,
                "Could not register Fn as a global hotkey. Lone modifier keys require Accessibility permission in System Settings. OSStatus: 0",
            ),
            (
                "combo on macOS 27",
                Shortcut::default(),
                -9878,
                macos_27,
                "Could not register the global hotkey (⌥ Space). It may be in use by another application. OSStatus: -9878",
            ),
            (
                "lone modifier on macOS 27",
                Shortcut::lone(ModifierKey::AltRight),
                -9874,
                macos_27,
                "Could not register Right Option as a global hotkey. Lone modifier keys require Device Control and Data Access in System Settings. OSStatus: -9874",
            ),
            (
                "combo on Linux has no status name",
                Shortcut::default(),
                1,
                Platform::linux(),
                "Could not register the global hotkey (⌥ Space). It may be in use by another application.",
            ),
            (
                "lone modifier on Linux has no typing permission to name",
                Shortcut::lone(ModifierKey::AltRight),
                1,
                Platform::linux(),
                "Could not register the global hotkey (Right Option). It may be in use by another application.",
            ),
        ];
        for (name, shortcut, status, platform, want) in cases {
            let got = RegistrationFailureNotice::new(&shortcut, status, platform);
            assert_eq!(got.title, "Hotkey Registration Failed", "{name}");
            assert_eq!(got.message, want, "{name}");
        }
    }

    #[test]
    fn failure_mailbox_hot_20() {
        struct Case {
            name: &'static str,
            before: Vec<i32>,
            want_replay: Option<i32>,
        }
        let cases = [
            Case {
                name: "nothing reported",
                before: vec![],
                want_replay: None,
            },
            Case {
                name: "one failure is held",
                before: vec![-50],
                want_replay: Some(-50),
            },
            Case {
                name: "only the most recent is kept",
                before: vec![-50, -9878],
                want_replay: Some(-9878),
            },
        ];
        for tc in cases {
            let mut mailbox = FailureMailbox::new();
            for status in tc.before {
                assert_eq!(
                    mailbox.report(status),
                    None,
                    "{} holds before wiring",
                    tc.name
                );
            }
            assert_eq!(mailbox.handler_ready(), tc.want_replay, "{}", tc.name);
            assert_eq!(mailbox.handler_ready(), None, "{} replays once", tc.name);
            assert_eq!(
                mailbox.report(-1),
                Some(-1),
                "{} delivers directly after wiring",
                tc.name
            );
        }
    }

    #[test]
    fn cancel_modifiers_add_the_shortcuts_own_hot_34_deviation() {
        let alt = ModifierSet::EMPTY.with(Modifier::Alt);
        let control_alt = alt.with(Modifier::Control);
        let space = KeyCode::SPACE;
        let cases = [
            (
                "option space adds option escape",
                Shortcut::default(),
                vec![ModifierSet::EMPTY, alt],
            ),
            (
                "every combo modifier is kept",
                Shortcut::new(space, control_alt).unwrap_or_default(),
                vec![ModifierSet::EMPTY, control_alt],
            ),
            (
                "right option adds its family",
                Shortcut::lone(ModifierKey::AltRight),
                vec![ModifierSet::EMPTY, alt],
            ),
            (
                "right command adds command",
                Shortcut::lone(ModifierKey::MetaRight),
                vec![ModifierSet::EMPTY, ModifierSet::EMPTY.with(Modifier::Meta)],
            ),
            (
                "fn has no combo modifier",
                Shortcut::lone(ModifierKey::Fn),
                vec![ModifierSet::EMPTY],
            ),
        ];
        for (name, shortcut, want) in cases {
            assert_eq!(cancel_hotkey_modifiers(&shortcut), want, "{name}");
        }
    }

    #[test]
    fn routes_each_hotkey_by_id() {
        let cases = [
            (
                "dictation press",
                DICTATION_HOTKEY_ID,
                true,
                HotkeyRoute::DictationDown,
            ),
            (
                "dictation release",
                DICTATION_HOTKEY_ID,
                false,
                HotkeyRoute::DictationUp,
            ),
            (
                "escape press cancels",
                CANCEL_HOTKEY_ID,
                true,
                HotkeyRoute::Cancel,
            ),
            (
                "escape release does nothing",
                CANCEL_HOTKEY_ID,
                false,
                HotkeyRoute::Ignore,
            ),
            ("unknown hotkey", 99, true, HotkeyRoute::Ignore),
        ];
        for (name, id, pressed, want) in cases {
            assert_eq!(HotkeyRoute::route(id, pressed), want, "{name}");
        }
    }
}
