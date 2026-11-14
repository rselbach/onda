//! The dictation hotkey end to end against fakes: the lone-modifier gesture
//! in every activation mode, the event-stream re-enable bound, unregistering
//! while held, registration failures and the Accessibility retry episode,
//! the recorder's capture, Escape, and the Fn warning (HOT-8..HOT-35).

mod common;

use std::sync::atomic::Ordering;

use common::{Harness, lock, sleep};
use onda_app::HotkeyEvent;
use onda_domain::hotkey::{
    GlobeKeyAction, HeldModifiers, KeyCode, Modifier, ModifierFlagsChange, ModifierKey,
    ModifierSet, RecorderEvent, Shortcut,
};
use onda_domain::platform::Platform;
use onda_domain::tray::TrayIcon;
use onda_domain::types::ActivationMode;

/// What the shell reports when it gives up on the stream (`eventNotHandledErr`).
const STREAM_STATUS: i32 = -9874;

fn code(name: &str) -> KeyCode {
    KeyCode::parse(name).unwrap_or_else(|| panic!("{name} is a key code"))
}

fn flags(h: &Harness, key: ModifierKey, family: bool, side: bool) {
    h.app
        .on_hotkey_event(HotkeyEvent::ModifierFlags(ModifierFlagsChange {
            key: KeyCode::from(key),
            family_flag_set: family,
            side_flag_set: side,
        }));
}

fn right_option(h: &Harness, down: bool) {
    flags(h, ModifierKey::AltRight, down, down);
}

fn key_down(h: &Harness, name: &str) {
    h.app.on_hotkey_event(HotkeyEvent::KeyDown(code(name)));
}

fn disable_stream(h: &Harness) {
    h.app.on_hotkey_event(HotkeyEvent::StreamDisabled {
        status: STREAM_STATUS,
    });
}

fn starts(h: &Harness) -> u64 {
    h.audio.starts.load(Ordering::SeqCst)
}

fn registrations(h: &Harness) -> usize {
    h.log.with("hotkey:register").len()
}

fn recording(h: &Harness) -> bool {
    h.tray().icon == TrayIcon::Recording
}

fn mods(modifiers: &[Modifier]) -> ModifierSet {
    modifiers.iter().copied().collect()
}

async fn lone(mode: ActivationMode) -> Harness {
    Harness::with(|s| {
        s.hotkey = Shortcut::lone(ModifierKey::AltRight);
        s.activation_mode = mode;
    })
    .await
}

/// A Right Option core whose launch registrations fail `failures` times
/// with Accessibility at `trusted`, after `ui_ready`.
async fn failing_lone(failures: usize, trusted: bool) -> Harness {
    let h = Harness::unstarted(|s| s.hotkey = Shortcut::lone(ModifierKey::AltRight));
    h.permissions.accessibility.store(trusted, Ordering::SeqCst);
    h.shell.fail_registrations(failures, STREAM_STATUS);
    h.app.ui_ready();
    sleep(10).await;
    h
}

// ---------------------------------------------------------------------------
// lone-modifier gesture and activation modes (HOT-21..HOT-33)

#[tokio::test(start_paused = true)]
async fn hot_25_lone_modifier_hold_records_while_held() {
    let h = lone(ActivationMode::Hold).await;
    assert_eq!(
        h.log.with("hotkey:register"),
        vec!["hotkey:register Right Option"]
    );
    right_option(&h, true);
    sleep(1_000).await;
    assert!(recording(&h));
    right_option(&h, false);
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn hot_26_lone_modifier_toggle_starts_and_stops_on_taps() {
    let h = lone(ActivationMode::Toggle).await;
    right_option(&h, true);
    sleep(200).await;
    right_option(&h, false);
    sleep(1_000).await;
    assert!(recording(&h), "a toggle tap latches");
    assert!(h.overlay().visible, "toggle starts are not provisional");
    right_option(&h, true);
    sleep(200).await;
    right_option(&h, false);
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn hot_27_lone_modifier_automatic_by_hold_length() {
    struct Case {
        name: &'static str,
        held_ms: u64,
        want_recording_after_release: bool,
    }
    let cases = [
        Case {
            name: "quick tap latches",
            held_ms: 250,
            want_recording_after_release: true,
        },
        Case {
            name: "just under the threshold latches",
            held_ms: 390,
            want_recording_after_release: true,
        },
        Case {
            // measured from the physical press, not from the debounce
            name: "just over the threshold is push-to-talk",
            held_ms: 410,
            want_recording_after_release: false,
        },
        Case {
            name: "long hold is push-to-talk",
            held_ms: 700,
            want_recording_after_release: false,
        },
    ];
    for tc in cases {
        let h = lone(ActivationMode::Automatic).await;
        right_option(&h, true);
        sleep(tc.held_ms).await;
        right_option(&h, false);
        sleep(1).await;
        assert_eq!(
            recording(&h),
            tc.want_recording_after_release,
            "{}",
            tc.name
        );
        sleep(1_000).await;
        let want_typed = usize::from(!tc.want_recording_after_release);
        assert_eq!(h.typed().len(), want_typed, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn hot_21_a_tap_shorter_than_the_debounce_does_nothing() {
    let h = lone(ActivationMode::Hold).await;
    right_option(&h, true);
    sleep(100).await;
    right_option(&h, false);
    sleep(1_000).await;
    assert_eq!(starts(&h), 0);
    assert!(h.log.with("sound:").is_empty());
    assert!(h.idle());
}

#[tokio::test(start_paused = true)]
async fn hot_22_another_key_while_pending_cancels_the_whole_hold() {
    struct Case {
        name: &'static str,
        other: fn(&Harness),
    }
    let cases = [
        Case {
            name: "letter key (Option+G typing)",
            other: |h| key_down(h, "KeyG"),
        },
        Case {
            name: "another modifier's press, by its side bit",
            other: |h| flags(h, ModifierKey::ShiftLeft, true, true),
        },
        Case {
            name: "fn press, by its own flag",
            other: |h| flags(h, ModifierKey::Fn, true, false),
        },
    ];
    for tc in cases {
        let h = lone(ActivationMode::Hold).await;
        right_option(&h, true);
        sleep(50).await;
        (tc.other)(&h);
        sleep(1_000).await;
        assert_eq!(starts(&h), 0, "{}", tc.name);
        right_option(&h, false);
        sleep(500).await;
        assert_eq!(starts(&h), 0, "{}", tc.name);
        assert!(h.idle(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn hot_14a_modifier_key_downs_and_releases_are_not_other_keys() {
    let h = lone(ActivationMode::Hold).await;
    right_option(&h, true);
    sleep(50).await;
    key_down(&h, "ShiftLeft");
    // a sibling's release keeps the family flag but clears its side bit
    flags(&h, ModifierKey::AltLeft, true, false);
    sleep(500).await;
    assert_eq!(starts(&h), 1);
}

#[tokio::test(start_paused = true)]
async fn hot_31_lone_modifier_release_before_confirmation_leaves_no_trace() {
    let h = lone(ActivationMode::Hold).await;
    right_option(&h, true);
    // fires at 150 ms; confirmation is 150 ms later (HOT-30)
    sleep(250).await;
    assert_eq!(starts(&h), 1);
    right_option(&h, false);
    sleep(1_000).await;
    assert!(h.idle());
    assert!(h.log.with("sound:").is_empty());
    assert!(h.history.entries().is_empty());
    assert!(!h.overlays().iter().any(|view| view.visible));
}

#[tokio::test(start_paused = true)]
async fn hot_33_discard_cancels_only_what_this_press_started() {
    let cases = [
        ("hold always cancels", ActivationMode::Hold, false),
        ("toggle keeps recording", ActivationMode::Toggle, true),
        (
            "automatic cancels a start by this press",
            ActivationMode::Automatic,
            false,
        ),
    ];
    for (name, mode, want_recording) in cases {
        let h = lone(mode).await;
        right_option(&h, true);
        sleep(500).await;
        assert!(recording(&h), "{name}");
        key_down(&h, "KeyE");
        sleep(100).await;
        assert_eq!(recording(&h), want_recording, "{name}");
        assert!(h.typed().is_empty(), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn lone_modifier_events_are_ignored_while_a_combo_is_registered() {
    let h = Harness::new().await;
    right_option(&h, true);
    sleep(1_000).await;
    assert_eq!(starts(&h), 0);
}

// ---------------------------------------------------------------------------
// event-stream re-enable bound (HOT-17) and unregister while held (HOT-18)

#[tokio::test(start_paused = true)]
async fn hot_17_reenables_five_times_then_gives_up_and_reports() {
    let h = lone(ActivationMode::Hold).await;
    for _ in 0..5 {
        disable_stream(&h);
    }
    sleep(10).await;
    assert_eq!(h.log.count("hotkey:reenable"), 5);
    assert_eq!(h.log.count("hotkey:unregister"), 0);
    disable_stream(&h);
    sleep(10).await;
    assert_eq!(h.log.count("hotkey:reenable"), 5);
    assert_eq!(h.log.count("hotkey:unregister"), 1, "torn down");
    assert_eq!(h.notifications(), vec!["Hotkey Registration Failed"]);
    disable_stream(&h);
    sleep(10).await;
    assert_eq!(h.log.count("hotkey:reenable"), 5, "dead stays dead");
    assert_eq!(h.log.count("hotkey:unregister"), 1);
}

#[tokio::test(start_paused = true)]
async fn hot_17_a_real_event_clears_the_disable_history() {
    let h = lone(ActivationMode::Hold).await;
    for _ in 0..5 {
        disable_stream(&h);
    }
    key_down(&h, "KeyA");
    for _ in 0..5 {
        disable_stream(&h);
    }
    sleep(10).await;
    assert_eq!(h.log.count("hotkey:reenable"), 10);
    assert_eq!(h.log.count("hotkey:unregister"), 0);
    assert!(h.notifications().is_empty());
}

#[tokio::test(start_paused = true)]
async fn hot_17_a_disabled_stream_releases_an_active_gesture() {
    let h = lone(ActivationMode::Hold).await;
    right_option(&h, true);
    sleep(500).await;
    disable_stream(&h);
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["hello world"], "key-up stopped the hold");
    right_option(&h, false);
    sleep(500).await;
    assert_eq!(h.typed().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn hot_18_unregistering_while_held_delivers_the_key_up() {
    struct Case {
        name: &'static str,
        hotkey: Shortcut,
        press: fn(&Harness),
        release: fn(&Harness),
    }
    let cases = [
        Case {
            name: "combo",
            hotkey: Shortcut::OPTION_SPACE,
            press: Harness::press,
            release: Harness::release,
        },
        Case {
            name: "lone modifier",
            hotkey: Shortcut::lone(ModifierKey::AltRight),
            press: |h| right_option(h, true),
            release: |h| right_option(h, false),
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.hotkey = tc.hotkey.clone()).await;
        (tc.press)(&h);
        sleep(500).await;
        assert!(recording(&h), "{}", tc.name);
        h.app.begin_hotkey_capture();
        sleep(1_000).await;
        assert_eq!(h.typed(), vec!["hello world"], "{}", tc.name);
        assert_eq!(h.log.count("hotkey:unregister"), 1, "{}", tc.name);
        // the physical release after the unregister changes nothing
        (tc.release)(&h);
        sleep(1_000).await;
        assert_eq!(h.typed().len(), 1, "{}", tc.name);
        assert_eq!(starts(&h), 1, "{}", tc.name);
    }
}

// ---------------------------------------------------------------------------
// registration failures (HOT-19, HOT-20)

#[tokio::test(start_paused = true)]
async fn hot_19_lone_modifier_failure_polls_and_retries_once_trusted() {
    let h = failing_lone(1, false).await;
    assert_eq!(h.notifications(), vec!["Hotkey Registration Failed"]);
    let reads = h.permissions.trust_reads.load(Ordering::SeqCst);
    sleep(4_100).await;
    assert_eq!(registrations(&h), 1, "no retry while untrusted");
    assert!(
        h.permissions.trust_reads.load(Ordering::SeqCst) >= reads + 2,
        "polled every 2 s"
    );
    h.permissions.accessibility.store(true, Ordering::SeqCst);
    sleep(2_100).await;
    assert_eq!(registrations(&h), 2, "retried once trusted");
    assert_eq!(h.notifications().len(), 1);
    let reads = h.permissions.trust_reads.load(Ordering::SeqCst);
    sleep(10_000).await;
    assert_eq!(
        h.permissions.trust_reads.load(Ordering::SeqCst),
        reads,
        "polling stopped after the successful retry"
    );
    right_option(&h, true);
    sleep(500).await;
    assert!(recording(&h), "the retried registration works");
}

#[tokio::test(start_paused = true)]
async fn hot_19_failure_while_trusted_waits_for_a_trust_toggle() {
    let h = failing_lone(2, true).await;
    assert_eq!(h.notifications().len(), 1);
    sleep(6_100).await;
    assert_eq!(registrations(&h), 1, "trusted to trusted never retries");
    h.permissions.accessibility.store(false, Ordering::SeqCst);
    sleep(2_100).await;
    h.permissions.accessibility.store(true, Ordering::SeqCst);
    sleep(2_100).await;
    assert_eq!(registrations(&h), 2, "the toggle retried");
    assert_eq!(h.notifications().len(), 1, "failed retries stay silent");
    h.permissions.accessibility.store(false, Ordering::SeqCst);
    sleep(2_100).await;
    h.permissions.accessibility.store(true, Ordering::SeqCst);
    sleep(2_100).await;
    assert_eq!(registrations(&h), 3);
    assert_eq!(h.notifications().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn hot_19_app_activation_checks_trust_at_once() {
    let h = failing_lone(1, false).await;
    h.permissions.accessibility.store(true, Ordering::SeqCst);
    h.app.app_became_active();
    sleep(10).await;
    assert_eq!(registrations(&h), 2);
}

#[tokio::test(start_paused = true)]
async fn hot_19_explicit_registration_starts_a_new_episode() {
    let h = failing_lone(2, false).await;
    h.app.set_hotkey(Shortcut::lone(ModifierKey::AltRight));
    sleep(10).await;
    assert_eq!(registrations(&h), 2);
    assert_eq!(h.notifications().len(), 2, "each episode notifies once");
}

#[tokio::test(start_paused = true)]
async fn hot_19_combo_failure_ends_a_lone_modifier_episode() {
    let h = failing_lone(2, false).await;
    h.app.set_hotkey(Shortcut::OPTION_SPACE);
    sleep(10).await;
    assert_eq!(
        h.notifications().len(),
        2,
        "a combo failure always notifies"
    );
    h.permissions.accessibility.store(true, Ordering::SeqCst);
    sleep(5_000).await;
    assert_eq!(registrations(&h), 2, "no retry after the episode ended");
}

#[tokio::test(start_paused = true)]
async fn hot_20_failures_before_ui_ready_are_held_and_the_latest_delivered() {
    let h = Harness::unstarted(|_| {});
    h.shell.fail_registrations(1, -9878);
    sleep(100).await;
    h.app.hotkey_registration_failed(-50);
    sleep(100).await;
    assert!(h.notifications().is_empty(), "held until the UI is ready");
    h.app.ui_ready();
    sleep(10).await;
    assert_eq!(h.notifications(), vec!["Hotkey Registration Failed"]);
    let bodies = lock(&h.notifier.bodies).clone();
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].ends_with("OSStatus: -50"),
        "the newest failure, not the first: {}",
        bodies[0]
    );
}

// ---------------------------------------------------------------------------
// recording a shortcut (HOT-8..HOT-12) and settings changes (SET-5)

fn hotkey_calls(h: &Harness) -> Vec<String> {
    h.log
        .all()
        .into_iter()
        .filter(|e| e.starts_with("hotkey:") || e.starts_with("settings:hotkey"))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn hot_8_capture_unregisters_and_drops_presses_until_it_ends() {
    let h = Harness::new().await;
    h.app.begin_hotkey_capture();
    sleep(1).await;
    h.tap();
    sleep(1_000).await;
    assert_eq!(starts(&h), 0);
    h.app.end_hotkey_capture();
    sleep(10).await;
    assert_eq!(
        hotkey_calls(&h),
        vec![
            "hotkey:register ⌥ Space",
            "hotkey:unregister",
            "hotkey:register ⌥ Space"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn hot_9_to_hot_12_recorder_outcomes() {
    struct Case {
        name: &'static str,
        act: fn(&Harness) -> bool,
        want_swallowed: bool,
        want_calls_after_unregister: Vec<&'static str>,
        want_recording: bool,
        want_message: Option<&'static str>,
        want_display: &'static str,
    }
    let cases = [
        Case {
            name: "combo commits, saved before its one registration (HOT-12)",
            act: |h| {
                h.app.recorder_event(RecorderEvent::KeyDown {
                    key: code("KeyD"),
                    modifiers: mods(&[Modifier::Control, Modifier::Alt]),
                })
            },
            want_swallowed: true,
            want_calls_after_unregister: vec!["settings:hotkey ⌃⌥ D", "hotkey:register ⌃⌥ D"],
            want_recording: false,
            want_message: None,
            want_display: "⌃⌥ D",
        },
        Case {
            name: "lone modifier commits on release",
            act: |h| {
                let key = KeyCode::from(ModifierKey::AltRight);
                h.app.recorder_event(RecorderEvent::FlagsChanged {
                    key: key.clone(),
                    held: HeldModifiers::only(ModifierKey::AltRight),
                });
                h.app.recorder_event(RecorderEvent::FlagsChanged {
                    key,
                    held: HeldModifiers::NONE,
                })
            },
            want_swallowed: true,
            want_calls_after_unregister: vec![
                "settings:hotkey Right Option",
                "hotkey:register Right Option",
            ],
            want_recording: false,
            want_message: None,
            want_display: "Right Option",
        },
        Case {
            name: "invalid combo keeps recording with its message (HOT-10)",
            act: |h| {
                h.app.recorder_event(RecorderEvent::KeyDown {
                    key: KeyCode::SPACE,
                    modifiers: mods(&[Modifier::Shift]),
                })
            },
            want_swallowed: true,
            want_calls_after_unregister: vec![],
            want_recording: true,
            want_message: Some(
                "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn.",
            ),
            want_display: "⌥ Space",
        },
        Case {
            name: "escape cancels and re-registers the current shortcut",
            act: |h| {
                h.app.recorder_event(RecorderEvent::KeyDown {
                    key: KeyCode::ESCAPE,
                    modifiers: ModifierSet::EMPTY,
                })
            },
            want_swallowed: true,
            want_calls_after_unregister: vec!["hotkey:register ⌥ Space"],
            want_recording: false,
            want_message: None,
            want_display: "⌥ Space",
        },
        Case {
            name: "the record button again cancels (HOT-11)",
            act: |h| {
                h.app.toggle_hotkey_capture();
                false
            },
            want_swallowed: false,
            want_calls_after_unregister: vec!["hotkey:register ⌥ Space"],
            want_recording: false,
            want_message: None,
            want_display: "⌥ Space",
        },
        Case {
            name: "the recorder disappearing ends the capture (HOT-8)",
            act: |h| {
                h.app.end_hotkey_capture();
                false
            },
            want_swallowed: false,
            want_calls_after_unregister: vec!["hotkey:register ⌥ Space"],
            want_recording: false,
            want_message: None,
            want_display: "⌥ Space",
        },
        Case {
            name: "the shell could not listen for keys (HOT-11)",
            act: |h| {
                h.app.hotkey_capture_failed();
                false
            },
            want_swallowed: false,
            want_calls_after_unregister: vec!["hotkey:register ⌥ Space"],
            want_recording: false,
            want_message: Some("Could not start shortcut recording."),
            want_display: "⌥ Space",
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        h.app.toggle_hotkey_capture();
        sleep(10).await;
        let view = h.hotkey_view();
        assert!(view.recording, "{}", tc.name);
        assert!(view.listening, "{}", tc.name);
        assert_eq!(view.record_button_title, "Press shortcut...", "{}", tc.name);
        assert_eq!((tc.act)(&h), tc.want_swallowed, "{}", tc.name);
        sleep(10).await;
        let calls = hotkey_calls(&h);
        assert_eq!(
            calls[2..],
            tc.want_calls_after_unregister,
            "{}: {calls:?}",
            tc.name
        );
        let view = h.hotkey_view();
        assert_eq!(view.recording, tc.want_recording, "{}", tc.name);
        assert_eq!(view.listening, tc.want_recording, "{}", tc.name);
        assert_eq!(view.message, tc.want_message, "{}", tc.name);
        assert_eq!(view.display, tc.want_display, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn hot_10_events_pass_through_when_not_recording() {
    let h = Harness::new().await;
    let swallowed = h.app.recorder_event(RecorderEvent::KeyDown {
        key: code("KeyD"),
        modifiers: mods(&[Modifier::Meta]),
    });
    assert!(!swallowed);
    let view = h.hotkey_view();
    assert_eq!(view.help_text, "Current shortcut: ⌥ Space");
    assert_eq!(view.record_button_title, "Record Shortcut");
}

#[tokio::test(start_paused = true)]
async fn set_5_a_changed_shortcut_re_registers_once() {
    let h = Harness::new().await;
    h.app.settings_changed();
    sleep(10).await;
    assert_eq!(registrations(&h), 1, "unchanged shortcut");
    h.update(|s| s.hotkey = Shortcut::lone(ModifierKey::AltRight));
    h.app.settings_changed();
    sleep(10).await;
    assert_eq!(
        h.log.with("hotkey:register"),
        vec!["hotkey:register ⌥ Space", "hotkey:register Right Option"]
    );
    right_option(&h, true);
    sleep(500).await;
    assert!(recording(&h));
}

// ---------------------------------------------------------------------------
// Escape (HOT-34) and the Fn warning (HOT-35)

#[tokio::test(start_paused = true)]
async fn hot_34_escape_with_the_shortcuts_own_modifiers_also_cancels() {
    // (Deviation: Escape with the shortcut's own modifiers also cancels
    // during a session; HOT-34 cancels on plain Escape only.)
    let control_alt = mods(&[Modifier::Control, Modifier::Alt]);
    let cases = [
        (
            "option space",
            Shortcut::OPTION_SPACE,
            vec![ModifierSet::EMPTY, mods(&[Modifier::Alt])],
        ),
        (
            "control option D",
            Shortcut::new(code("KeyD"), control_alt).expect("valid shortcut"),
            vec![ModifierSet::EMPTY, control_alt],
        ),
        (
            "right option",
            Shortcut::lone(ModifierKey::AltRight),
            vec![ModifierSet::EMPTY, mods(&[Modifier::Alt])],
        ),
        (
            "fn",
            Shortcut::lone(ModifierKey::Fn),
            vec![ModifierSet::EMPTY],
        ),
    ];
    for (name, hotkey, want) in cases {
        let h = Harness::with(|s| s.hotkey = hotkey.clone()).await;
        h.app.toggle_dictation_from_menu();
        sleep(10).await;
        assert_eq!(*lock(&h.shell.cancel_modifiers), vec![want], "{name}");
        h.app.on_hotkey_event(HotkeyEvent::Cancel);
        sleep(10).await;
        assert!(h.idle(), "{name}");
        assert_eq!(
            h.log.with("escape:"),
            vec!["escape:register", "escape:unregister"],
            "{name}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn hot_9_linux_records_combos_and_never_a_lone_modifier() {
    let h = Harness::on_platform(Platform::linux(), |s| {
        s.hotkey = Shortcut::CONTROL_ALT_SPACE;
    })
    .await;
    h.app.begin_hotkey_capture();
    sleep(1).await;
    for modifier in ModifierKey::ALL {
        let key = KeyCode::from(modifier);
        let pressed = h.app.recorder_event(RecorderEvent::FlagsChanged {
            key: key.clone(),
            held: HeldModifiers::only(modifier),
        });
        let released = h.app.recorder_event(RecorderEvent::FlagsChanged {
            key,
            held: HeldModifiers::NONE,
        });
        assert!(pressed && released, "{modifier:?} is swallowed");
    }
    sleep(10).await;
    let view = h.hotkey_view();
    assert!(view.recording, "still recording");
    assert_eq!(view.display, "Ctrl+Alt+Space");
    assert_eq!(
        view.help_text,
        "Press a key with Ctrl, Alt, or Super. Escape cancels."
    );
    assert!(h.log.with("settings:hotkey").is_empty(), "nothing saved");

    h.app.recorder_event(RecorderEvent::KeyDown {
        key: KeyCode::SPACE,
        modifiers: mods(&[Modifier::Shift]),
    });
    sleep(10).await;
    assert_eq!(
        h.hotkey_view().message,
        Some("Shortcut must include Ctrl, Alt, or Super.")
    );

    h.app.recorder_event(RecorderEvent::KeyDown {
        key: KeyCode::SPACE,
        modifiers: mods(&[Modifier::Control, Modifier::Meta]),
    });
    sleep(10).await;
    let view = h.hotkey_view();
    assert!(!view.recording);
    assert_eq!(view.display, "Ctrl+Super+Space");
    assert_eq!(
        lock(&h.settings.settings).hotkey,
        Shortcut::CONTROL_SUPER_SPACE
    );
}

#[tokio::test(start_paused = true)]
async fn hot_35_linux_never_shows_the_fn_warning() {
    let h = Harness::build_on(None, Platform::linux(), |s| {
        s.hotkey = Shortcut::lone(ModifierKey::Fn);
    });
    *lock(&h.shell.globe) = GlobeKeyAction::Unknown;
    h.app.ui_ready();
    sleep(10).await;
    h.app.app_became_active();
    sleep(10).await;
    let view = h.hotkey_view();
    assert_eq!(view.display, "Fn");
    assert_eq!(view.fn_warning, None);
}

#[tokio::test(start_paused = true)]
async fn hot_35_fn_warning_follows_the_globe_key_setting() {
    let h = Harness::unstarted(|s| s.hotkey = Shortcut::lone(ModifierKey::Fn));
    *lock(&h.shell.globe) = GlobeKeyAction::ChangeInputSource;
    h.app.ui_ready();
    sleep(10).await;
    assert_eq!(
        h.hotkey_view().fn_warning,
        Some("Pressing 🌐 also switches your input source. Set “Press 🌐 key to” to Do Nothing.")
    );
    *lock(&h.shell.globe) = GlobeKeyAction::DoNothing;
    h.app.app_became_active();
    sleep(10).await;
    assert_eq!(h.hotkey_view().fn_warning, None, "re-read on activation");

    let combo = Harness::new().await;
    combo.app.app_became_active();
    sleep(10).await;
    assert!(
        combo.log.with("hotkey:globe").is_empty(),
        "only read for Fn"
    );
    assert_eq!(combo.hotkey_view().fn_warning, None);
}
