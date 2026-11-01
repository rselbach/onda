//! The session engine end to end against fakes, including the cases of
//! Jabber's DictationCoordinatorTests.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{
    EngineCall, Harness, Hold, asr_failure, eventually, lock, loud, quiet, sleep, target,
};
use onda_app::HotkeyEvent;
use onda_app::seams::AudioSignal;
use onda_domain::cleanup::CleanupError;
use onda_domain::hotkey::{KeyCode, ModifierFlagsChange, ModifierKey, Shortcut};
use onda_domain::languages::LanguageSetting;
use onda_domain::messages::{NotificationAuthorization, UserMessage};
use onda_domain::models::ModelId;
use onda_domain::overlay::OverlayContent;
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::replacements::{ReplacementRule, RuleId};
use onda_domain::tray::TrayIcon;
use onda_domain::types::{ActivationMode, MicrophonePermission, OutputMode};

fn rule(trigger: &str, replacement: &str) -> ReplacementRule {
    ReplacementRule {
        id: RuleId("rule-1".to_owned()),
        triggers: vec![trigger.to_owned()],
        replacement: replacement.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// activation modes (HOT-24..HOT-31)

#[tokio::test(start_paused = true)]
async fn hold_records_provisionally_confirms_at_300_ms_and_types_on_release() {
    let h = Harness::new().await;
    h.press();
    sleep(250).await;
    assert!(!h.overlay().visible, "provisional: no overlay yet (HOT-29)");
    assert!(h.log.with("sound:").is_empty());
    assert_eq!(h.tray().icon, TrayIcon::Ready, "provisional keeps the tray");
    sleep(100).await;
    assert!(h.overlay().visible, "confirmed after 300 ms (HOT-30)");
    assert_eq!(h.log.with("sound:"), vec!["sound:start"]);
    assert_eq!(h.tray().icon, TrayIcon::Recording);
    h.release();
    sleep(1).await;
    assert_eq!(
        h.tray().icon,
        TrayIcon::Transcribing,
        "reports at once (DICT-11)"
    );
    assert_eq!(h.log.with("sound:"), vec!["sound:start", "sound:stop"]);
    sleep(500).await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert_eq!(
        h.typed_targets(),
        vec![Some(42)],
        "the target captured at start"
    );
    assert_eq!(h.history.entries().len(), 1);
    assert!(h.idle());
    assert_eq!(h.tray().icon, TrayIcon::Ready);
    assert!(!h.overlay().visible);
}

#[tokio::test(start_paused = true)]
async fn hot_31_a_provisional_tap_leaves_no_trace() {
    let h = Harness::with(|s| s.pause_media = true).await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    h.press();
    sleep(100).await;
    h.release();
    sleep(1_000).await;
    assert!(h.typed().is_empty());
    assert!(h.history.entries().is_empty());
    assert!(h.log.with("sound:").is_empty());
    assert!(h.log.with("media:").is_empty());
    assert!(h.overlays().iter().all(|view| !view.visible));
    assert!(!h.audio.is_open());
    // Escape is claimed even for the unconfirmed recording (HOT-34)
    assert_eq!(
        h.log.with("escape:"),
        vec!["escape:register", "escape:unregister"]
    );
    assert!(h.idle());
}

#[tokio::test(start_paused = true)]
async fn hot_32_menu_stop_of_a_provisional_recording_transcribes_without_cue_or_media() {
    let h = Harness::with(|s| s.pause_media = true).await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    let hold = h.script.hold_next_final();
    h.press();
    sleep(100).await;
    h.stop_now();
    eventually("the decode", || hold.started()).await;
    let overlay = h.overlay();
    assert!(overlay.visible, "the overlay comes up directly processing");
    assert!(matches!(overlay.content, OverlayContent::Processing { .. }));
    hold.release();
    sleep(500).await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(
        h.log.with("sound:").is_empty(),
        "never confirmed: no cues (SND-1)"
    );
    assert!(h.log.with("media:").is_empty());
    h.release();
    sleep(100).await;
    assert_eq!(h.typed().len(), 1, "the late key-up does nothing");
}

#[tokio::test(start_paused = true)]
async fn hot_30_lone_modifier_confirms_150_ms_after_its_debounce() {
    let h = Harness::with(|s| s.hotkey = Shortcut::lone(ModifierKey::AltRight)).await;
    let flags = |set: bool| {
        HotkeyEvent::ModifierFlags(ModifierFlagsChange {
            key: KeyCode::from(ModifierKey::AltRight),
            family_flag_set: set,
            side_flag_set: set,
        })
    };
    h.app.on_hotkey_event(flags(true));
    sleep(140).await;
    assert_eq!(
        h.audio.starts.load(Ordering::SeqCst),
        0,
        "debouncing (HOT-21)"
    );
    sleep(20).await;
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1);
    sleep(100).await;
    assert!(!h.overlay().visible, "300 - 150 ms not yet elapsed");
    sleep(60).await;
    assert!(h.overlay().visible);
    h.app.on_hotkey_event(flags(false));
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn hot_33_another_key_after_the_gesture_fired_discards_the_recording() {
    let h = Harness::with(|s| s.hotkey = Shortcut::lone(ModifierKey::AltRight)).await;
    h.app
        .on_hotkey_event(HotkeyEvent::ModifierFlags(ModifierFlagsChange {
            key: KeyCode::from(ModifierKey::AltRight),
            family_flag_set: true,
            side_flag_set: true,
        }));
    sleep(500).await;
    assert!(h.overlay().visible);
    let key = KeyCode::parse("KeyE").unwrap_or_else(|| panic!("KeyE parses"));
    h.app.on_hotkey_event(HotkeyEvent::KeyDown(key));
    sleep(1_000).await;
    assert!(h.idle());
    assert!(h.typed().is_empty());
    assert!(h.history.entries().is_empty());
}

#[tokio::test(start_paused = true)]
async fn toggle_starts_on_one_tap_and_stops_with_tail_on_the_next() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(1).await;
    assert!(h.overlay().visible, "toggle starts are not provisional");
    assert_eq!(h.log.with("sound:"), vec!["sound:start"]);
    sleep(2_000).await;
    assert_eq!(h.tray().icon, TrayIcon::Recording);
    h.tap();
    sleep(1).await;
    assert!(h.audio.is_open(), "still capturing during the 200 ms tail");
    sleep(250).await;
    assert!(!h.audio.is_open());
    sleep(500).await;
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn hot_27_automatic_mode_by_press_length() {
    struct Case {
        name: &'static str,
        held_ms: u64,
        want_recording_after_release: bool,
    }
    let cases = [
        Case {
            name: "quick tap latches like toggle",
            held_ms: 100,
            want_recording_after_release: true,
        },
        Case {
            name: "held 400 ms is push-to-talk",
            held_ms: 400,
            want_recording_after_release: false,
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.activation_mode = ActivationMode::Automatic).await;
        h.press();
        sleep(tc.held_ms).await;
        h.release();
        sleep(1).await;
        assert_eq!(
            h.tray().icon == TrayIcon::Recording,
            tc.want_recording_after_release,
            "{}",
            tc.name
        );
        if tc.want_recording_after_release {
            h.tap();
        }
        sleep(1_000).await;
        assert_eq!(h.typed(), vec!["hello world"], "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn hot_24_mode_is_captured_at_key_down() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.press();
    sleep(10).await;
    h.update(|s| s.activation_mode = ActivationMode::Hold);
    h.release();
    sleep(1_000).await;
    assert_eq!(
        h.tray().icon,
        TrayIcon::Recording,
        "toggle key-up does nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn hot_28_releasing_during_the_microphone_prompt_aborts_the_start() {
    let h = Harness::new().await;
    *lock(&h.permissions.microphone) = MicrophonePermission::NotDetermined;
    let prompt = common::Hold::new();
    *lock(&h.permissions.hold_request) = Some(prompt.clone());
    h.press();
    eventually("the prompt", || prompt.started()).await;
    // the release is queued before the prompt returns (ARCH-11)
    h.release();
    prompt.release();
    sleep(1_000).await;
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 0, "never began");
    assert!(h.idle());
    assert!(h.notices().is_empty());
}

// ---------------------------------------------------------------------------
// start checks (DICT-1, DICT-2, DICT-5, DICT-6)

#[tokio::test(start_paused = true)]
async fn accessibility_copy_uses_the_os_naming() {
    struct Case {
        name: &'static str,
        naming: PrivacyNaming,
        want_notice: &'static str,
        want_tray: &'static str,
    }
    let cases = [
        Case {
            name: "macOS 26",
            naming: PrivacyNaming::Accessibility,
            want_notice: "Accessibility Permission Required",
            want_tray: "Onda, accessibility access needed",
        },
        Case {
            name: "macOS 27",
            naming: PrivacyNaming::DeviceControlAndDataAccess,
            want_notice: "Device Control and Data Access Required",
            want_tray: "Onda, device control and data access needed",
        },
    ];
    for tc in cases {
        let h = Harness::with_privacy_naming(tc.naming, |s| {
            s.activation_mode = ActivationMode::Toggle;
        })
        .await;
        h.permissions.accessibility.store(false, Ordering::SeqCst);
        h.tap();
        sleep(100).await;
        assert!(
            h.notices().contains(&tc.want_notice.to_owned()),
            "{}: {:?}",
            tc.name,
            h.notices()
        );
        assert_eq!(h.tray().accessibility_label, tc.want_tray, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn dict_1_start_checks_in_order() {
    struct Case {
        name: &'static str,
        setup: fn(&Harness),
        want_notice: &'static str,
        want_prompts: usize,
    }
    let cases = [
        Case {
            name: "microphone denied",
            setup: |h| *lock(&h.permissions.microphone) = MicrophonePermission::Denied,
            want_notice: "Microphone Permission Required",
            want_prompts: 0,
        },
        Case {
            name: "microphone refused at the prompt",
            setup: |h| {
                *lock(&h.permissions.microphone) = MicrophonePermission::NotDetermined;
                h.permissions
                    .grant_on_request
                    .store(false, Ordering::SeqCst);
            },
            want_notice: "Microphone Permission Required",
            want_prompts: 0,
        },
        Case {
            name: "accessibility missing for typed output",
            setup: |h| h.permissions.accessibility.store(false, Ordering::SeqCst),
            want_notice: "Accessibility Permission Required",
            want_prompts: 1,
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
        (tc.setup)(&h);
        h.tap();
        sleep(100).await;
        h.tap();
        sleep(100).await;
        assert_eq!(h.audio.starts.load(Ordering::SeqCst), 0, "{}", tc.name);
        assert!(
            h.notices().contains(&tc.want_notice.to_owned()),
            "{}",
            tc.name
        );
        assert_eq!(
            h.log.count("permissions:prompt-accessibility"),
            tc.want_prompts,
            "{}: the Accessibility prompt shows once per run",
            tc.name
        );
        assert!(h.idle(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn dict_1_clipboard_output_does_not_need_accessibility() {
    let h = Harness::with(|s| s.output_mode = OutputMode::Clipboard).await;
    h.permissions.accessibility.store(false, Ordering::SeqCst);
    h.dictate().await;
    assert_eq!(*lock(&h.text.copied), vec!["hello world"]);
    assert!(h.typed().is_empty());
    assert!(h.notices().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dict_1_model_not_ready_is_rate_limited_and_shows_setup_guidance() {
    let h = Harness::unstarted(|_| {});
    lock(&h.models.downloaded).clear();
    sleep(1).await;
    for _ in 0..3 {
        h.press();
        h.release();
        sleep(100).await;
    }
    assert_eq!(
        h.notices(),
        vec!["Model Not Ready"],
        "one per 1.5 s (DICT-2)"
    );
    let guidance = || {
        h.count(|e| {
            matches!(
                e,
                onda_app::Event::OpenMainWindow(_) | onda_app::Event::OpenOnboarding
            )
        })
    };
    assert_eq!(
        guidance(),
        1,
        "guidance shares the notice's rate limit (DICT-2)"
    );
    assert!(h.events().contains(&onda_app::Event::OpenMainWindow(
        onda_app::MainWindowSection::GettingStarted
    )));
    // once the first notice cleared (4 s), the next one shows again
    sleep(4_500).await;
    h.press();
    h.release();
    sleep(1).await;
    assert_eq!(h.notices().len(), 2);
    assert_eq!(guidance(), 2);
}

#[tokio::test(start_paused = true)]
async fn dict_2_still_transcribing_is_rate_limited() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    let hold = h.script.hold_next_final();
    h.tap();
    sleep(500).await;
    h.stop_now();
    eventually("the final decode", || hold.started()).await;
    h.tap();
    h.tap();
    eventually("the notice", || h.notices() == vec!["Still Transcribing"]).await;
    hold.release();
    sleep(500).await;
    assert_eq!(h.notices(), vec!["Still Transcribing"]);
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn dict_6_capture_failure_resumes_media_and_reports() {
    let h = Harness::with(|s| {
        s.activation_mode = ActivationMode::Toggle;
        s.pause_media = true;
    })
    .await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    *lock(&h.audio.start_error) = Some(UserMessage::MicrophoneUnavailable {
        error: "The selected microphone is not connected".to_owned(),
    });
    h.tap();
    sleep(1_000).await;
    assert!(h.idle());
    assert_eq!(h.notices(), vec!["Microphone Unavailable"]);
    // the failure can land before or after the pause; either way media is
    // left as it was (a query answered after its session ended never pauses)
    let media = h.log.with("media:");
    assert!(
        media.is_empty() || media == vec!["media:pause", "media:play"],
        "{media:?}"
    );
    assert!(h.log.with("escape:").is_empty(), "never became a session");
}

// ---------------------------------------------------------------------------
// tail and speech gate (DICT-11..DICT-13, GATE-6)

#[tokio::test(start_paused = true)]
async fn dict_11_tail_audio_is_transcribed_but_not_judged() {
    struct Case {
        name: &'static str,
        before: Vec<f32>,
        tail: Vec<f32>,
        want_typed: Vec<&'static str>,
        want_notices: Vec<&'static str>,
        want_samples: Option<usize>,
    }
    let cases = [
        Case {
            name: "tail audio reaches the transcript",
            before: loud(16_000),
            tail: loud(800),
            want_typed: vec!["hello world"],
            want_notices: vec![],
            want_samples: Some(16_800),
        },
        Case {
            name: "a stop click in the tail cannot pass the gate",
            before: quiet(16_000),
            tail: loud(3_200),
            want_typed: vec![],
            want_notices: vec!["No Speech Detected"],
            want_samples: None,
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        h.audio.starts_with(tc.before);
        h.press();
        sleep(400).await;
        h.release();
        sleep(50).await;
        h.audio.push(tc.tail);
        sleep(1_000).await;
        assert_eq!(h.typed(), tc.want_typed, "{}", tc.name);
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
        let samples = h.history.entries().first().map(|e| e.sample_count);
        assert_eq!(samples, tc.want_samples, "{}", tc.name);
        assert!(h.idle(), "{}", tc.name);
        assert_eq!(
            h.log.with("power:").last().map(String::as_str),
            Some("power:end")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn dict_12_rejected_menu_stop_goes_idle_without_a_stop_cue() {
    struct Case {
        name: &'static str,
        samples: Vec<f32>,
        want_notices: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "empty",
            samples: vec![],
            want_notices: vec![],
        },
        Case {
            name: "too quiet",
            samples: quiet(16_000),
            want_notices: vec!["No Speech Detected"],
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        h.audio.starts_with(tc.samples);
        h.app.toggle_dictation_from_menu();
        sleep(500).await;
        h.app.toggle_dictation_from_menu();
        sleep(500).await;
        assert!(h.idle(), "{}", tc.name);
        assert_eq!(h.log.with("sound:"), vec!["sound:start"], "{}", tc.name);
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
        assert!(
            h.events()
                .iter()
                .all(|e| !matches!(e, onda_app::Event::TrayChanged(t) if t.icon == TrayIcon::Transcribing)),
            "{}: straight from Recording to Idle",
            tc.name
        );
    }
}

#[tokio::test(start_paused = true)]
async fn gate_2_audio_under_the_engine_minimum_ends_silently() {
    struct Case {
        name: &'static str,
        min_samples: usize,
        hotkey: bool,
        want_typed: Vec<&'static str>,
        want_sounds: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "menu stop under Parakeet's 0.3 s",
            min_samples: 4_800,
            hotkey: false,
            want_typed: vec![],
            want_sounds: vec!["sound:start"],
        },
        Case {
            name: "hotkey stop under Parakeet's 0.3 s",
            min_samples: 4_800,
            hotkey: true,
            want_typed: vec![],
            want_sounds: vec!["sound:start", "sound:stop"],
        },
        Case {
            name: "an engine with no minimum transcribes it",
            min_samples: 0,
            hotkey: false,
            want_typed: vec!["hello world"],
            want_sounds: vec!["sound:start", "sound:stop"],
        },
    ];
    for tc in cases {
        // the engine reports its minimum when it loads
        let h = Harness::unstarted(|_| {});
        h.script.min_samples.store(tc.min_samples, Ordering::SeqCst);
        h.app.ui_ready();
        sleep(1).await;
        // between GATE-2's 0.25 s (4,000) and 0.3 s (4,800)
        h.audio.starts_with(loud(4_500));
        match tc.hotkey {
            true => h.dictate().await,
            false => {
                h.app.toggle_dictation_from_menu();
                sleep(10).await;
                h.app.toggle_dictation_from_menu();
                sleep(500).await;
            }
        }
        assert_eq!(h.typed(), tc.want_typed, "{}", tc.name);
        assert!(h.notices().is_empty(), "{}: no notice", tc.name);
        assert!(
            h.notifications().is_empty(),
            "{}: not Transcription Failed",
            tc.name
        );
        assert_eq!(h.log.with("sound:"), tc.want_sounds, "{}", tc.name);
        assert_eq!(
            h.history.entries().len(),
            tc.want_typed.len(),
            "{}: history only for a transcript",
            tc.name
        );
        assert!(
            h.menu().retry_failed_dictation.is_none(),
            "{}: no audio kept",
            tc.name
        );
        assert!(h.idle(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn gate_3_pre_roll_counts_toward_length_and_activity() {
    struct Case {
        name: &'static str,
        pre_roll: Vec<f32>,
        live: Vec<f32>,
    }
    let cases = [
        Case {
            // 0.1 s live is under GATE-2's 0.25 s on its own
            name: "pre-roll makes up the length",
            pre_roll: quiet(3_600),
            live: loud(1_600),
        },
        Case {
            // the live audio alone is too quiet for GATE-3
            name: "pre-roll supplies the activity",
            pre_roll: loud(1_600),
            live: quiet(3_600),
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        h.audio.starts_with(tc.pre_roll);
        h.app.toggle_dictation_from_menu();
        sleep(10).await;
        h.audio.push(tc.live);
        h.app.toggle_dictation_from_menu();
        sleep(500).await;
        assert_eq!(h.typed(), vec!["hello world"], "{}", tc.name);
        assert_eq!(
            h.script.calls().last(),
            Some(&EngineCall::Final(5_200)),
            "{}",
            tc.name
        );
        assert!(h.notices().is_empty(), "{}", tc.name);
    }
}

// ---------------------------------------------------------------------------
// cancel (DICT-19, DICT-20, ARCH-10)

#[tokio::test(start_paused = true)]
// CLN-9: cancelling mid-cleanup drops the request silently. ARCH-6: a port
// call blocked in decode, history, or delivery never stalls the cancel.
async fn dict_19_cln_9_arch_6_cancel_in_every_phase_drops_the_session() {
    #[derive(Debug, Clone, Copy)]
    enum Phase {
        Recording,
        Tail,
        Decoding,
        Cleanup,
        HistorySave,
        Delivery,
    }
    for phase in [
        Phase::Recording,
        Phase::Tail,
        Phase::Decoding,
        Phase::Cleanup,
        Phase::HistorySave,
        Phase::Delivery,
    ] {
        let h = Harness::with(|s| {
            s.activation_mode = ActivationMode::Toggle;
            s.pause_media = true;
            s.cleanup_enabled = matches!(phase, Phase::Cleanup);
        })
        .await;
        h.media.playing_default.store(true, Ordering::SeqCst);
        let notify = Arc::new(tokio::sync::Notify::new());
        let hold = common::Hold::new();
        match phase {
            Phase::Decoding => *lock(&h.script.hold_final) = Some(hold.clone()),
            Phase::Cleanup => *lock(&h.cleanup.hold) = Some(notify.clone()),
            Phase::HistorySave => *lock(&h.history.hold_save) = Some(hold.clone()),
            Phase::Delivery => *lock(&h.text.hold_insert) = Some(hold.clone()),
            Phase::Recording | Phase::Tail => {}
        }
        h.cleanup.respond(vec![Ok("Hello world.".to_owned())]);
        h.tap();
        sleep(500).await;
        match phase {
            Phase::Recording => {}
            Phase::Tail => h.tap(),
            Phase::Decoding | Phase::Cleanup | Phase::HistorySave | Phase::Delivery => {
                h.stop_now();
            }
        }
        match phase {
            Phase::Recording => {}
            Phase::Tail => sleep(50).await,
            Phase::Cleanup => eventually("the request", || h.cleanup.calls() == 1).await,
            Phase::Decoding | Phase::HistorySave | Phase::Delivery => {
                eventually("the held step", || hold.started()).await;
            }
        }
        h.app.cancel_dictation();
        eventually("idle", || h.idle()).await;
        hold.release();
        notify.notify_waiters();
        sleep(1_000).await;
        let typed = h.typed();
        match phase {
            // the text was already being typed when cancel arrived
            Phase::Delivery => assert_eq!(typed.len(), 1, "{phase:?}"),
            _ => assert!(typed.is_empty(), "{phase:?}"),
        }
        assert!(h.idle(), "{phase:?}");
        assert!(h.notices().is_empty(), "{phase:?}");
        assert!(h.notifications().is_empty(), "{phase:?}");
        // the save had started before the cancel only in the last two
        let want_history = usize::from(matches!(phase, Phase::HistorySave | Phase::Delivery));
        assert_eq!(h.history.entries().len(), want_history, "{phase:?}");
        assert!(
            h.menu().retry_failed_dictation.is_none(),
            "{phase:?}: no audio kept"
        );
        assert_eq!(h.log.count("media:play"), 1, "{phase:?}: resumed once");
        assert!(!h.audio.is_open(), "{phase:?}");
        assert_eq!(
            h.log.with("escape:").last().map(String::as_str),
            Some("escape:unregister")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn arch_10_cancel_frees_the_session_while_old_inference_runs() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.script
        .finals(vec![Ok("A result".to_owned()), Ok("B result".to_owned())]);
    let hold = h.script.hold_next_final();
    h.tap();
    sleep(500).await;
    h.stop_now();
    eventually("A's decode", || hold.started()).await;
    h.app.cancel_dictation();
    eventually("idle", || h.idle()).await;
    assert!(h.menu().dictation.title == "Start Dictation");

    // session B starts at once and queues its decode behind A's
    h.tap();
    eventually("B recording", || h.tray().icon == TrayIcon::Recording).await;
    h.tap();
    eventually("B transcribing", || h.tray().icon == TrayIcon::Transcribing).await;
    hold.release();
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["B result"], "A's late result is dropped");
    assert_eq!(h.history.entries().len(), 1);
    assert!(h.idle());
}

#[tokio::test(start_paused = true)]
async fn cancelled_failure_is_neither_reported_nor_kept() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.script.finals(vec![Err(asr_failure())]);
    let hold = h.script.hold_next_final();
    h.tap();
    sleep(500).await;
    h.stop_now();
    eventually("the decode", || hold.started()).await;
    h.app.cancel_dictation();
    hold.release();
    sleep(1_000).await;
    assert!(h.notifications().is_empty());
    assert!(h.menu().retry_failed_dictation.is_none());
}

#[tokio::test(start_paused = true)]
async fn escape_cancels_and_is_only_claimed_during_a_session() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    assert!(h.log.with("escape:").is_empty());
    h.tap();
    sleep(500).await;
    assert_eq!(h.log.with("escape:"), vec!["escape:register"]);
    h.app.on_hotkey_event(HotkeyEvent::Cancel);
    sleep(100).await;
    assert!(h.idle());
    assert_eq!(
        h.log.with("escape:"),
        vec!["escape:register", "escape:unregister"]
    );
    assert!(h.typed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dict_21_menu_item_stops_while_recording_and_cancels_while_transcribing() {
    let h = Harness::new().await;
    h.app.toggle_dictation_from_menu();
    sleep(1).await;
    assert_eq!(h.menu().dictation.title, "Stop Dictation");
    assert!(
        h.overlay().visible,
        "menu starts are not provisional (UI-9)"
    );
    let hold = h.script.hold_next_final();
    h.app.toggle_dictation_from_menu();
    eventually("the decode", || hold.started()).await;
    assert_eq!(h.menu().dictation.title, "Cancel Dictation");
    h.app.toggle_dictation_from_menu();
    eventually("idle", || h.idle()).await;
    hold.release();
    sleep(500).await;
    assert!(h.typed().is_empty());
}

// ---------------------------------------------------------------------------
// limit, clock, device loss (DICT-8, DICT-9, DICT-12)

#[tokio::test(start_paused = true)]
async fn dict_8_limit_warns_then_transcribes_the_whole_buffer() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(7_500).await;
    let clock = h.overlay().trailing.and_then(|t| t.clock);
    assert_eq!(
        clock.map(|c| (c.text, c.warning)),
        Some(("0:07".to_owned(), false))
    );
    sleep(841_500 - 7_500).await;
    let clock = h.overlay().trailing.and_then(|t| t.clock);
    assert_eq!(
        clock.map(|c| (c.text, c.warning)),
        Some(("Stops in 0:59".to_owned(), true))
    );
    sleep(59_000).await;
    assert_eq!(h.notifications(), vec!["Recording Limit Reached"]);
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert_eq!(h.history.entries()[0].sample_count, 16_000);
}

#[tokio::test(start_paused = true)]
async fn msg_1_background_messages_fall_back_to_the_overlay() {
    struct Case {
        name: &'static str,
        authorization: NotificationAuthorization,
        post_fails: bool,
        want_notifications: Vec<&'static str>,
        want_notices: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "authorized",
            authorization: NotificationAuthorization::Authorized,
            post_fails: false,
            want_notifications: vec!["Transcription Failed"],
            want_notices: vec![],
        },
        Case {
            name: "denied",
            authorization: NotificationAuthorization::Denied,
            post_fails: false,
            want_notifications: vec![],
            want_notices: vec!["Transcription Failed"],
        },
        Case {
            name: "post failed",
            authorization: NotificationAuthorization::Provisional,
            post_fails: true,
            want_notifications: vec![],
            want_notices: vec!["Transcription Failed"],
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        *lock(&h.notifier.authorization) = tc.authorization;
        h.notifier.post_fails.store(tc.post_fails, Ordering::SeqCst);
        h.script.finals(vec![Err(asr_failure())]);
        h.dictate().await;
        assert_eq!(h.notifications(), tc.want_notifications, "{}", tc.name);
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn device_lost_mid_recording_transcribes_what_was_captured() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(500).await;
    h.app.audio_signals().send(AudioSignal::Interrupted {
        capture_id: h.audio.capture_id(),
        notice: UserMessage::MicrophoneUnavailable {
            error: "Could not reconnect to the microphone: gone".to_owned(),
        },
    });
    sleep(1).await;
    assert!(!h.audio.is_open(), "no tail (DICT-12)");
    sleep(500).await;
    assert_eq!(h.notices(), vec!["Microphone Unavailable"]);
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn device_lost_with_nothing_captured_goes_idle_and_reports() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.audio.starts_with(vec![]);
    h.tap();
    sleep(100).await;
    h.app.audio_signals().send(AudioSignal::Interrupted {
        capture_id: h.audio.capture_id(),
        notice: UserMessage::MicrophoneUnavailable {
            error: "The microphone connected, but did not provide any audio".to_owned(),
        },
    });
    sleep(100).await;
    assert!(h.idle());
    assert_eq!(h.notices(), vec!["Microphone Unavailable"]);
    assert!(h.typed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dict_11_interruption_during_the_tail_ends_it_early() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(500).await;
    h.tap();
    sleep(50).await;
    h.app.audio_signals().send(AudioSignal::Interrupted {
        capture_id: h.audio.capture_id(),
        notice: UserMessage::MicrophoneUnavailable {
            error: "gone".to_owned(),
        },
    });
    sleep(1).await;
    assert!(!h.audio.is_open(), "the tail ended early");
    sleep(500).await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert_eq!(h.notices(), vec!["Microphone Unavailable"]);
}

#[tokio::test(start_paused = true)]
async fn conversion_failures_while_recording_are_rate_limited() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(100).await;
    let signal = || AudioSignal::ConversionFailed {
        capture_id: h.audio.capture_id(),
        notice: UserMessage::AudioProcessingError {
            error: "Audio conversion failed: bad buffer".to_owned(),
        },
    };
    for _ in 0..5 {
        h.app.audio_signals().send(signal());
        sleep(100).await;
    }
    assert_eq!(h.notices(), vec!["Audio Processing Error"]);
    assert_eq!(h.tray().icon, TrayIcon::Recording, "recording continues");
}

#[tokio::test(start_paused = true)]
async fn aud_13_silent_microphone_shows_after_1_5_s_and_clears_on_sound() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(10).await;
    let level = |value| h.app.audio_signals().send(AudioSignal::Level(value));
    for _ in 0..16 {
        level(0.0);
        sleep(100).await;
    }
    assert!(matches!(
        h.overlay().content,
        OverlayContent::SilentMicrophone { ref title, .. } if title == "No sound from Greendale USB Microphone"
    ));
    level(0.2);
    sleep(1).await;
    assert!(matches!(
        h.overlay().content,
        OverlayContent::Waveform { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn aud_13_silent_microphone_names_the_device_after_a_rebuild() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(10).await;
    let signals = h.app.audio_signals();
    signals.send(AudioSignal::Rebuilt {
        device_name: "Abed's Podcast Mic".to_owned(),
    });
    for _ in 0..16 {
        signals.send(AudioSignal::Level(0.0));
        sleep(100).await;
    }
    assert!(matches!(
        h.overlay().content,
        OverlayContent::SilentMicrophone { ref title, .. } if title == "No sound from Abed's Podcast Mic"
    ));
}

// ---------------------------------------------------------------------------
// failed ASR and retry (DICT-22..DICT-25)

#[tokio::test(start_paused = true)]
async fn failed_asr_keeps_the_audio_and_retry_types_into_the_focused_app() {
    let h = Harness::with(|s| s.pause_media = true).await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    h.audio.starts_with(loud(24_000));
    h.script.finals(vec![Err(asr_failure())]);
    h.dictate().await;
    assert_eq!(h.notifications(), vec!["Transcription Failed"]);
    assert!(h.typed().is_empty());
    assert!(
        h.history.entries().is_empty(),
        "failed ASR is not saved (HIS-2)"
    );
    let menu = h.menu();
    assert_eq!(menu.retry_failed_dictation.map(|i| i.enabled), Some(true));
    assert_eq!(menu.discard_failed_recording.map(|i| i.enabled), Some(true));
    let media_before = h.log.with("media:");
    let sounds_before = h.log.with("sound:");

    *lock(&h.text.focused) = Some(target(54_321, "Dean's Office"));
    h.script
        .final_default
        .lock()
        .map(|mut t| *t = "troy and abed in the morning".to_owned())
        .ok();
    let hold = h.script.hold_next_final();
    h.app.retry_failed_dictation();
    eventually("the retry decode", || hold.started()).await;
    assert_eq!(
        h.tray().icon,
        TrayIcon::Transcribing,
        "straight to Transcribing"
    );
    assert!(matches!(
        h.overlay().content,
        OverlayContent::Processing { .. }
    ));
    hold.release();
    sleep(500).await;
    assert_eq!(h.typed(), vec!["troy and abed in the morning"]);
    assert_eq!(
        h.typed_targets(),
        vec![Some(54_321)],
        "focused at retry time"
    );
    assert_eq!(h.history.entries()[0].sample_count, 24_000);
    assert!(
        h.menu().retry_failed_dictation.is_none(),
        "success drops the audio"
    );
    assert_eq!(
        h.log.with("media:"),
        media_before,
        "retries never touch media"
    );
    assert_eq!(h.log.with("sound:"), sounds_before, "no cues for retries");
    assert!(
        !h.script.calls()[media_before.len()..]
            .iter()
            .any(|c| matches!(c, EngineCall::Preview(_))),
        "no preview on retry"
    );
}

#[tokio::test(start_paused = true)]
async fn retry_failure_cancel_and_discard_keep_or_drop_the_audio() {
    let h = Harness::new().await;
    h.script
        .finals(vec![Err(asr_failure()), Err(asr_failure())]);
    h.dictate().await;
    assert_eq!(h.notifications().len(), 1);

    // a failed retry keeps the audio for another try
    h.app.retry_failed_dictation();
    sleep(500).await;
    assert_eq!(h.notifications().len(), 2);
    assert!(h.menu().retry_failed_dictation.is_some());

    // cancel during a retry keeps it; discard is refused meanwhile
    let hold = h.script.hold_next_final();
    h.app.retry_failed_dictation();
    eventually("the retry decode", || hold.started()).await;
    h.app.discard_failed_dictation();
    h.app.cancel_dictation();
    eventually("idle", || h.idle()).await;
    hold.release();
    sleep(500).await;
    assert!(h.typed().is_empty());
    assert!(h.menu().retry_failed_dictation.is_some());

    h.app.discard_failed_dictation();
    sleep(1).await;
    assert!(h.menu().retry_failed_dictation.is_none());
    h.app.retry_failed_dictation();
    sleep(100).await;
    assert!(h.idle(), "nothing to retry");
}

#[tokio::test(start_paused = true)]
async fn newer_failure_replaces_the_older_recording() {
    let h = Harness::new().await;
    h.script
        .finals(vec![Err(asr_failure()), Err(asr_failure())]);
    h.audio.starts_with(loud(16_000));
    h.dictate().await;
    h.audio.starts_with(loud(32_000));
    h.dictate().await;
    h.app.retry_failed_dictation();
    sleep(500).await;
    assert_eq!(h.history.entries()[0].sample_count, 32_000);
}

#[tokio::test(start_paused = true)]
async fn retry_is_refused_while_recording_and_an_unrelated_success_keeps_the_audio() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.script.finals(vec![Err(asr_failure())]);
    h.tap();
    sleep(500).await;
    h.tap();
    sleep(500).await;
    h.tap();
    sleep(100).await;
    h.app.retry_failed_dictation();
    sleep(100).await;
    assert_eq!(h.tray().icon, TrayIcon::Recording);
    let menu = h.menu();
    assert_eq!(menu.retry_failed_dictation.map(|i| i.enabled), Some(false));
    h.tap();
    sleep(500).await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(h.menu().retry_failed_dictation.is_some());
}

// ---------------------------------------------------------------------------
// cleanup (CLN-1..CLN-15)

const LONG_RAW: &str = "so troy and abed are going to the study room at greendale community college to work on the spanish project with the rest of the study group tonight";

#[tokio::test(start_paused = true)]
async fn cleanup_outcomes() {
    struct Case {
        name: &'static str,
        raw: &'static str,
        responses: Vec<Result<String, CleanupError>>,
        want_calls: usize,
        want_typed: Vec<String>,
        want_notices: Vec<&'static str>,
        want_cleaned: bool,
        want_transcript: String,
        want_error: Option<&'static str>,
    }
    let cleaned_long = "So Troy and Abed are going to the study room at Greendale Community College to work on the Spanish project with the rest of the study group tonight.";
    let cases = [
        Case {
            name: "success",
            raw: "um hello there",
            responses: vec![Ok("Hello there.".to_owned())],
            want_calls: 1,
            want_typed: vec!["Hello there.".to_owned()],
            want_notices: vec![],
            want_cleaned: true,
            want_transcript: "Hello there.".to_owned(),
            want_error: None,
        },
        Case {
            name: "empty is a deliberate scratch that",
            raw: "troy and abed scratch that",
            responses: vec![Ok("   ".to_owned())],
            want_calls: 1,
            want_typed: vec![],
            want_notices: vec![],
            want_cleaned: true,
            want_transcript: String::new(),
            want_error: None,
        },
        Case {
            name: "throw is not retried",
            raw: "cool cool cool",
            responses: vec![Err(CleanupError::Http {
                status: 429,
                message: None,
            })],
            want_calls: 1,
            want_typed: vec!["cool cool cool".to_owned()],
            want_notices: vec!["Couldn't Refine Transcript"],
            want_cleaned: false,
            want_transcript: "cool cool cool".to_owned(),
            want_error: Some("OpenRouter returned HTTP 429."),
        },
        Case {
            name: "guardrail retry passes",
            raw: LONG_RAW,
            responses: vec![
                Ok("Studying at Greendale.".to_owned()),
                Ok(cleaned_long.to_owned()),
            ],
            want_calls: 2,
            want_typed: vec![cleaned_long.to_owned()],
            want_notices: vec![],
            want_cleaned: true,
            want_transcript: cleaned_long.to_owned(),
            want_error: None,
        },
        Case {
            name: "guardrail retry fails",
            raw: LONG_RAW,
            responses: vec![
                Ok("Studying at Greendale.".to_owned()),
                Ok("Studying.".to_owned()),
            ],
            want_calls: 2,
            want_typed: vec![LONG_RAW.to_owned()],
            want_notices: vec!["Used Raw Transcript"],
            want_cleaned: false,
            want_transcript: LONG_RAW.to_owned(),
            want_error: Some(
                "Post-processing output looked too different from the transcript, so Onda used the raw transcript.",
            ),
        },
        Case {
            name: "guardrail retry throws",
            raw: LONG_RAW,
            responses: vec![
                Ok("Studying at Greendale.".to_owned()),
                Err(CleanupError::Transport("timed out".to_owned())),
            ],
            want_calls: 2,
            want_typed: vec![LONG_RAW.to_owned()],
            want_notices: vec!["Used Raw Transcript"],
            want_cleaned: false,
            want_transcript: LONG_RAW.to_owned(),
            want_error: Some("OpenRouter request failed: timed out"),
        },
        Case {
            name: "explicit header command allows markdown",
            raw: "header shopping list",
            responses: vec![Ok("# Shopping List".to_owned())],
            want_calls: 1,
            want_typed: vec!["# Shopping List".to_owned()],
            want_notices: vec![],
            want_cleaned: true,
            want_transcript: "# Shopping List".to_owned(),
            want_error: None,
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.cleanup_enabled = true).await;
        *lock(&h.script.final_default) = tc.raw.to_owned();
        h.cleanup.respond(tc.responses);
        h.dictate().await;
        assert_eq!(h.cleanup.calls(), tc.want_calls, "{}", tc.name);
        assert_eq!(h.typed(), tc.want_typed, "{}", tc.name);
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
        let entry = &h.history.entries()[0];
        assert_eq!(entry.was_post_processed, tc.want_cleaned, "{}", tc.name);
        assert_eq!(entry.transcript, tc.want_transcript, "{}", tc.name);
        assert_eq!(
            entry.raw_transcript.as_deref(),
            tc.want_cleaned.then_some(tc.raw),
            "{}",
            tc.name
        );
        assert_eq!(
            entry.post_processing_error_description.as_deref(),
            tc.want_error,
            "{}",
            tc.name
        );
        assert_eq!(
            h.processing_labels(),
            vec!["Transcribing...", "Refining..."],
            "{}: CLN-3",
            tc.name
        );
        assert!(h.notifications().is_empty(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn cln_2_missing_key_delivers_raw_silently() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    *lock(&h.secrets.0) = Ok(None);
    h.dictate().await;
    assert_eq!(h.cleanup.calls(), 0);
    assert_eq!(
        *lock(&h.secrets.2),
        [("read", onda_domain::cleanup::CleanupProvider::OpenRouter)],
        "the selected provider's key is read"
    );
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(h.notices().is_empty());
    let entry = &h.history.entries()[0];
    assert_eq!(
        entry.post_processing_error_description.as_deref(),
        Some("OpenRouter unavailable")
    );
    assert_eq!(h.processing_labels(), vec!["Transcribing..."]);
}

#[tokio::test(start_paused = true)]
async fn apple_intelligence_cleans_on_device_without_a_key() {
    use onda_domain::cleanup::{APPLE_MODELS, CleanupProvider, OnDeviceStatus};

    let h = Harness::with(|s| {
        s.cleanup_enabled = true;
        s.cleanup_provider = CleanupProvider::AppleIntelligence;
        s.cleanup_model = APPLE_MODELS[0].clone();
    })
    .await;
    *lock(&h.cleanup.on_device) = Some(OnDeviceStatus::Available);
    h.cleanup.respond(vec![Ok("Hello world.".to_owned())]);
    h.dictate().await;
    assert_eq!(h.typed(), vec!["Hello world."]);
    assert!(lock(&h.secrets.2).is_empty(), "no keychain read");
    let requests = lock(&h.cleanup.requests).clone();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("<transcript>\nhello world\n</transcript>")),
        "{}",
        requests[0]
    );
    assert_eq!(
        h.processing_labels(),
        vec!["Transcribing...", "Refining..."]
    );
}

#[tokio::test(start_paused = true)]
async fn apple_intelligence_turned_off_delivers_raw_silently() {
    use onda_domain::cleanup::{APPLE_MODELS, CleanupProvider, OnDeviceStatus};

    let h = Harness::with(|s| {
        s.cleanup_enabled = true;
        s.cleanup_provider = CleanupProvider::AppleIntelligence;
        s.cleanup_model = APPLE_MODELS[0].clone();
    })
    .await;
    *lock(&h.cleanup.on_device) = Some(OnDeviceStatus::NotEnabled);
    h.dictate().await;
    assert_eq!(h.cleanup.calls(), 0);
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(h.notices().is_empty());
    let entry = &h.history.entries()[0];
    assert_eq!(
        entry.post_processing_error_description.as_deref(),
        Some("Apple Intelligence unavailable")
    );
    assert_eq!(h.processing_labels(), vec!["Transcribing..."]);
}

#[tokio::test(start_paused = true)]
async fn cleanup_is_skipped_when_off_or_the_transcript_is_blank() {
    struct Case {
        name: &'static str,
        enabled: bool,
        raw: &'static str,
        want_typed: Vec<&'static str>,
        want_notices: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "disabled",
            enabled: false,
            raw: " um hello ",
            want_typed: vec!["um hello"],
            want_notices: vec![],
        },
        Case {
            name: "blank transcript",
            enabled: true,
            raw: "   ",
            want_typed: vec![],
            want_notices: vec!["No Speech Detected"],
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.cleanup_enabled = tc.enabled).await;
        *lock(&h.script.final_default) = tc.raw.to_owned();
        h.dictate().await;
        assert_eq!(h.cleanup.calls(), 0, "{}", tc.name);
        assert_eq!(h.typed(), tc.want_typed, "{}", tc.name);
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
        let entry = &h.history.entries()[0];
        assert!(!entry.was_post_processed, "{}", tc.name);
        assert_eq!(entry.raw_transcript, None, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn cln_6_cleanup_names_the_target_app_and_sends_the_untrimmed_raw() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    *lock(&h.script.final_default) = " um hello ".to_owned();
    h.cleanup.respond(vec![Ok("Hello.".to_owned())]);
    h.dictate().await;
    assert_eq!(
        h.cleanup.user_messages(),
        vec!["Dictating into: Greendale Messenger\n\n<transcript>\n um hello \n</transcript>"]
    );
}

#[tokio::test(start_paused = true)]
async fn rep_replacements_apply_after_cleanup_and_to_the_preview() {
    let h = Harness::unstarted(|s| {
        s.cleanup_enabled = true;
        s.replacement_rules = vec![rule("troy barnes", "Troy Barnes")];
    });
    h.script.supports_preview.store(true, Ordering::SeqCst);
    h.app.ui_ready();
    sleep(1).await;
    *lock(&h.script.preview_text) = "i saw troy barnes".to_owned();
    *lock(&h.script.final_default) = "hello troy barnes".to_owned();
    h.cleanup.respond(vec![Ok("hello troy barnes".to_owned())]);
    h.dictate().await;
    assert_eq!(h.previews(), vec!["i saw Troy Barnes"], "ASR-18");
    assert_eq!(h.typed(), vec!["hello Troy Barnes"]);
    let entry = &h.history.entries()[0];
    assert_eq!(entry.transcript, "hello Troy Barnes");
    assert_eq!(entry.raw_transcript.as_deref(), Some("hello troy barnes"));
}

// ---------------------------------------------------------------------------
// preview (ASR-15..ASR-21, DICT-13)

async fn preview_harness() -> Harness {
    let h = Harness::unstarted(|s| s.activation_mode = ActivationMode::Toggle);
    h.script.supports_preview.store(true, Ordering::SeqCst);
    h.app.ui_ready();
    sleep(1).await;
    h
}

#[tokio::test(start_paused = true)]
async fn asr_15_preview_decodes_the_whole_prefix_every_300_ms() {
    let h = preview_harness().await;
    *lock(&h.script.preview_text) = "troy and abed".to_owned();
    h.audio.starts_with(loud(280_000));
    h.tap();
    sleep(1).await;
    assert_eq!(h.previews(), vec!["troy and abed"]);
    assert!(h.script.calls().contains(&EngineCall::Preview(280_000)));
    h.audio.push(loud(1_600));
    sleep(300).await;
    let previews: Vec<_> = h
        .script
        .calls()
        .into_iter()
        .filter(|c| matches!(c, EngineCall::Preview(_)))
        .collect();
    assert_eq!(
        previews,
        vec![EngineCall::Preview(280_000), EngineCall::Preview(281_600)]
    );
    sleep(300).await;
    assert_eq!(previews.len(), 2, "no new audio: tick skipped (ASR-16)");
    let resets = h
        .script
        .calls()
        .iter()
        .filter(|c| **c == EngineCall::Reset)
        .count();
    assert_eq!(resets, 1);
    h.app.cancel_dictation();
}

#[tokio::test(start_paused = true)]
async fn asr_16_preview_waits_for_half_a_second_of_audio() {
    let h = preview_harness().await;
    *lock(&h.script.preview_text) = "troy".to_owned();
    h.audio.starts_with(loud(4_000));
    h.tap();
    sleep(1_000).await;
    assert!(h.previews().is_empty());
    h.audio.push(loud(4_000));
    sleep(300).await;
    assert_eq!(h.previews(), vec!["troy"]);
    h.app.cancel_dictation();
}

#[tokio::test(start_paused = true)]
async fn preview_is_off_when_the_engine_does_not_support_it() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(1_000).await;
    assert!(
        !h.script
            .calls()
            .iter()
            .any(|c| matches!(c, EngineCall::Preview(_) | EngineCall::Reset))
    );
    h.app.cancel_dictation();
}

#[tokio::test(start_paused = true)]
async fn dict_13_final_decode_waits_for_the_in_flight_preview() {
    let h = preview_harness().await;
    *lock(&h.script.final_default) = "final".to_owned();
    *lock(&h.script.preview_text) = "preview".to_owned();
    let hold = h.script.hold_next_preview();
    h.tap();
    eventually("the preview", || hold.started()).await;
    h.app.toggle_dictation_from_menu();
    eventually("transcribing", || h.tray().icon == TrayIcon::Transcribing).await;
    assert_eq!(h.script.finals_run(), 0, "final waits for the preview");
    hold.release();
    sleep(500).await;
    let order: Vec<_> = h
        .script
        .calls()
        .into_iter()
        .filter(|c| matches!(c, EngineCall::Preview(_) | EngineCall::Final(_)))
        .collect();
    assert_eq!(
        order,
        vec![EngineCall::Preview(16_000), EngineCall::Final(16_000)]
    );
    assert_eq!(h.typed(), vec!["final"]);
}

#[tokio::test(start_paused = true)]
async fn dict_13_a_hung_preview_is_abandoned_after_5_s() {
    let h = preview_harness().await;
    *lock(&h.script.final_default) = "final".to_owned();
    let hold = h.script.hold_next_preview();
    h.tap();
    eventually("the preview", || hold.started()).await;
    h.app.toggle_dictation_from_menu();
    eventually("transcribing", || h.tray().icon == TrayIcon::Transcribing).await;
    tokio::time::advance(Duration::from_secs(5)).await;
    eventually("the wait to time out", || true).await;
    // the engine call itself cannot be interrupted; the final decode runs on
    // the gate right after it, and the abandoned preview is not reused
    hold.release();
    sleep(500).await;
    assert_eq!(h.typed(), vec!["final"]);
    assert_eq!(h.script.finals_run(), 1);
}

#[tokio::test(start_paused = true)]
async fn asr_21_a_fresh_covering_preview_stands_in_for_the_final_decode() {
    let h = preview_harness().await;
    *lock(&h.script.preview_text) = "troy and abed in the morning".to_owned();
    h.audio.starts_with(loud(48_000));
    h.tap();
    sleep(400).await;
    h.app.toggle_dictation_from_menu();
    sleep(500).await;
    assert_eq!(h.typed(), vec!["troy and abed in the morning"]);
    assert_eq!(h.script.finals_run(), 0, "reused (ASR-21)");
}

// ---------------------------------------------------------------------------
// output (DICT-17, DICT-26..DICT-28, OUT-1..OUT-11)

#[tokio::test(start_paused = true)]
async fn out_8_joiner_uses_the_text_before_the_caret() {
    let h = Harness::new().await;
    *lock(&h.text.field) = Some(onda_app::ports::FieldText {
        text: "Done.".to_owned(),
        caret_utf16: 5,
    });
    *lock(&h.script.final_default) = "next thing".to_owned();
    h.dictate().await;
    assert_eq!(h.typed(), vec![" Next thing"]);
    assert_eq!(
        h.menu().last_transcript.subtitle.as_deref(),
        Some("next thing")
    );
}

#[tokio::test(start_paused = true)]
async fn delivery_outcomes_show_their_notices() {
    use onda_app::ports::InsertOutcome;
    struct Case {
        name: &'static str,
        output_mode: OutputMode,
        accessibility_at_delivery: bool,
        insert: InsertOutcome,
        copy_ok: bool,
        want_notices: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "inserted",
            output_mode: OutputMode::Type,
            accessibility_at_delivery: true,
            insert: InsertOutcome::Inserted,
            copy_ok: true,
            want_notices: vec![],
        },
        Case {
            name: "accessibility revoked mid-session (OUT-1)",
            output_mode: OutputMode::Type,
            accessibility_at_delivery: false,
            insert: InsertOutcome::Inserted,
            copy_ok: true,
            want_notices: vec!["Transcript Copied to Clipboard"],
        },
        Case {
            name: "paste failed (OUT-4)",
            output_mode: OutputMode::Type,
            accessibility_at_delivery: true,
            insert: InsertOutcome::PasteFailed,
            copy_ok: true,
            want_notices: vec!["Paste Failed"],
        },
        Case {
            name: "restore failed (OUT-5)",
            output_mode: OutputMode::Type,
            accessibility_at_delivery: true,
            insert: InsertOutcome::ClipboardRestoreFailed,
            copy_ok: true,
            want_notices: vec!["Clipboard Restore Failed"],
        },
        Case {
            name: "copy failed (OUT-6)",
            output_mode: OutputMode::Clipboard,
            accessibility_at_delivery: true,
            insert: InsertOutcome::Inserted,
            copy_ok: false,
            want_notices: vec!["Copy Failed"],
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| {
            s.output_mode = tc.output_mode;
            s.activation_mode = ActivationMode::Toggle;
        })
        .await;
        *lock(&h.text.insert_outcome) = tc.insert;
        h.text.copy_ok.store(tc.copy_ok, Ordering::SeqCst);
        h.tap();
        sleep(500).await;
        h.permissions
            .accessibility
            .store(tc.accessibility_at_delivery, Ordering::SeqCst);
        h.tap();
        sleep(1_000).await;
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn dict_26_last_transcript_is_redelivered_into_the_app_focused_now() {
    let h = Harness::with(|s| {
        s.replacement_rules = vec![rule("greendale", "Greendale Community College")]
    })
    .await;
    *lock(&h.script.final_default) = "  welcome to greendale  ".to_owned();
    h.dictate().await;
    let menu = h.menu();
    assert!(menu.last_transcript.enabled);
    assert_eq!(
        menu.last_transcript.subtitle.as_deref(),
        Some("welcome to Greendale Community College")
    );
    *lock(&h.text.focused) = Some(target(7, "Study Room"));
    h.app.deliver_last_transcript();
    sleep(100).await;
    assert_eq!(
        h.typed(),
        vec![
            "welcome to Greendale Community College",
            "welcome to Greendale Community College"
        ]
    );
    assert_eq!(h.typed_targets(), vec![Some(42), Some(7)]);

    // an empty result or a failure keeps the last transcript
    *lock(&h.script.final_default) = "   ".to_owned();
    h.dictate().await;
    h.script.finals(vec![Err(asr_failure())]);
    h.dictate().await;
    assert_eq!(
        h.menu().last_transcript.subtitle.as_deref(),
        Some("welcome to Greendale Community College")
    );

    // DICT-28: re-delivery runs the output-permission check
    h.permissions.accessibility.store(false, Ordering::SeqCst);
    h.app.deliver_transcript("six seasons and a movie");
    sleep(100).await;
    assert_eq!(h.typed().len(), 2);
    assert!(
        h.notices()
            .contains(&"Accessibility Permission Required".to_owned())
    );
}

#[tokio::test(start_paused = true)]
async fn dict_26_dict_27_a_session_started_during_the_redelivery_check_wins() {
    struct Case {
        name: &'static str,
        last_transcript: bool,
    }
    let cases = [
        Case {
            name: "Paste Last Transcript (DICT-26)",
            last_transcript: true,
        },
        Case {
            name: "a Recent Transcripts entry (DICT-27)",
            last_transcript: false,
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
        *lock(&h.script.final_default) = "six seasons and a movie".to_owned();
        if tc.last_transcript {
            h.tap();
            sleep(500).await;
            h.tap();
            sleep(1_000).await;
        }
        *lock(&h.script.final_default) = "hello world".to_owned();
        let before = h.typed().len();
        let hold = Hold::new();
        *lock(&h.text.hold_focus) = Some(hold.clone());
        match tc.last_transcript {
            true => h.app.deliver_last_transcript(),
            false => h.app.deliver_transcript("six seasons and a movie"),
        }
        eventually("the re-delivery check", || hold.started()).await;
        h.tap();
        eventually("recording", || h.tray().icon == TrayIcon::Recording).await;
        hold.release();
        h.stop_now();
        sleep(1_000).await;
        let typed = h.typed().split_off(before);
        assert_eq!(
            typed,
            vec!["hello world"],
            "{}: no concurrent insert",
            tc.name
        );
        assert!(h.idle(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn dict_16_text_goes_to_the_app_focused_at_start() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.cleanup.respond(vec![Ok("Hello world.".to_owned())]);
    let cleanup = Arc::new(tokio::sync::Notify::new());
    *lock(&h.cleanup.hold) = Some(cleanup.clone());
    h.press();
    sleep(400).await;
    // focus moves while recording, then again during cleanup
    *lock(&h.text.focused) = Some(target(7, "Study Room"));
    h.release();
    sleep(1_000).await;
    assert_eq!(h.cleanup.calls(), 1);
    *lock(&h.text.focused) = Some(target(9, "Dean's Office"));
    cleanup.notify_one();
    sleep(100).await;
    assert_eq!(h.typed(), vec!["Hello world."]);
    assert_eq!(h.typed_targets(), vec![Some(42)]);
    let entry = &h.history.entries()[0];
    assert_eq!(entry.app_name.as_deref(), Some("Greendale Messenger"));
    assert!(
        h.cleanup.user_messages()[0].contains("Greendale Messenger"),
        "cleanup names the start target"
    );
}

// ---------------------------------------------------------------------------
// history (HIS-1..HIS-3)

#[tokio::test(start_paused = true)]
async fn his_3_history_records_model_language_app_and_audio() {
    let h = Harness::with(|s| {
        s.keep_audio = true;
        s.language = onda_domain::languages::LanguageSetting::parse("en")
            .unwrap_or(onda_domain::languages::LanguageSetting::Auto);
    })
    .await;
    *lock(&h.script.final_default) = " troy and abed in the morning ".to_owned();
    h.dictate().await;
    let entry = &h.history.entries()[0];
    assert_eq!(entry.transcript, " troy and abed in the morning ");
    assert_eq!(entry.model_id, "parakeet-tdt-v2");
    assert_eq!(entry.model_name.as_deref(), Some("Parakeet TDT v2"));
    assert_eq!(entry.language, "en");
    assert_eq!(entry.app_name.as_deref(), Some("Greendale Messenger"));
    assert_eq!(
        entry.app_bundle_id.as_deref(),
        Some("edu.greendale.greendale-messenger")
    );
    assert_eq!(entry.sample_count, 16_000);
    assert_eq!(h.history.audio_lengths(), vec![Some(16_000)]);
    assert!(h.events().contains(&onda_app::Event::HistoryChanged));
    assert_eq!(
        h.typed(),
        vec!["troy and abed in the morning"],
        "output is trimmed"
    );
}

#[tokio::test(start_paused = true)]
async fn his_1_history_preferences_are_read_at_save_time() {
    struct Case {
        name: &'static str,
        history: bool,
        keep_audio: bool,
        want_entries: usize,
        want_audio: Vec<Option<usize>>,
    }
    let cases = [
        Case {
            name: "history off",
            history: false,
            keep_audio: true,
            want_entries: 0,
            want_audio: vec![],
        },
        Case {
            name: "transcripts only",
            history: true,
            keep_audio: false,
            want_entries: 1,
            want_audio: vec![None],
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
        h.tap();
        sleep(500).await;
        h.update(|s| {
            s.history_enabled = tc.history;
            s.keep_audio = tc.keep_audio;
        });
        h.tap();
        sleep(1_000).await;
        assert_eq!(h.history.entries().len(), tc.want_entries, "{}", tc.name);
        assert_eq!(h.history.audio_lengths(), tc.want_audio, "{}", tc.name);
        assert_eq!(h.typed(), vec!["hello world"], "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn his_2_a_failed_save_never_blocks_delivery() {
    let h = Harness::new().await;
    h.history.fail.store(true, Ordering::SeqCst);
    h.dictate().await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(!h.events().contains(&onda_app::Event::HistoryChanged));
}

// ---------------------------------------------------------------------------
// models (ASR-6, ASR-8, ASR-10, DL-8, DICT-29)

#[tokio::test(start_paused = true)]
async fn dict_29_download_and_use_applies_when_the_session_ends() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    h.tap();
    sleep(500).await;
    h.app.download_and_use(ModelId::ParakeetUnified);
    sleep(10).await;
    assert_eq!(h.log.with("download:"), vec!["download:parakeet-unified"]);
    lock(&h.models.downloaded).push(ModelId::ParakeetUnified);
    h.app.download_event(common::download(
        ModelId::ParakeetUnified,
        onda_app::seams::DownloadUpdateKind::Finished,
    ));
    sleep(100).await;
    assert_eq!(
        h.tray().icon,
        TrayIcon::Recording,
        "never cancels the session"
    );
    assert!(h.log.with("settings:").is_empty());
    h.tap();
    sleep(1_000).await;
    assert_eq!(h.typed(), vec!["hello world"]);
    assert_eq!(
        h.log.with("settings:"),
        vec!["settings:selected parakeet-unified"]
    );
    let calls = h.script.calls();
    assert!(calls.contains(&EngineCall::Unload(ModelId::ParakeetTdtV2)));
    assert_eq!(
        calls.last(),
        Some(&EngineCall::Load(ModelId::ParakeetUnified))
    );
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn dl_8_download_and_use_while_idle_switches_at_once() {
    let h = Harness::new().await;
    h.app.download_and_use(ModelId::ParakeetUnified);
    lock(&h.models.downloaded).push(ModelId::ParakeetUnified);
    h.app.download_event(common::download(
        ModelId::ParakeetUnified,
        onda_app::seams::DownloadUpdateKind::Finished,
    ));
    sleep(100).await;
    assert_eq!(
        h.log.with("settings:"),
        vec!["settings:selected parakeet-unified"]
    );
}

#[tokio::test(start_paused = true)]
async fn dl_8_a_stopped_download_or_explicit_selection_clears_the_pending_switch() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    lock(&h.models.downloaded).push(ModelId::ParakeetUnified);
    h.app.download_and_use(ModelId::ParakeetUltra);
    h.app.download_event(common::download(
        ModelId::ParakeetUltra,
        onda_app::seams::DownloadUpdateKind::Cancelled,
    ));
    h.app.download_event(common::download(
        ModelId::ParakeetUltra,
        onda_app::seams::DownloadUpdateKind::Finished,
    ));
    sleep(100).await;
    assert!(h.log.with("settings:").is_empty());
}

#[tokio::test(start_paused = true)]
async fn asr_8_explicit_model_change_cancels_the_session_and_reloads() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    lock(&h.models.downloaded).push(ModelId::ParakeetUnified);
    h.app.ui_ready();
    sleep(10).await;
    h.tap();
    sleep(500).await;
    h.app.select_model(ModelId::ParakeetUnified);
    sleep(100).await;
    assert!(h.idle());
    assert!(h.typed().is_empty());
    assert_eq!(
        h.log.with("settings:"),
        vec!["settings:selected parakeet-unified"]
    );
    assert_eq!(
        h.script.calls().last(),
        Some(&EngineCall::Load(ModelId::ParakeetUnified))
    );
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn asr_6_load_failure_notifies_and_offers_retry() {
    let h = Harness::unstarted(|_| {});
    h.script.fail_load.store(true, Ordering::SeqCst);
    h.app.ui_ready();
    sleep(100).await;
    assert_eq!(h.notifications(), vec!["Couldn't Load Speech Model"]);
    let menu = h.menu();
    assert_eq!(menu.header, "Model Unavailable");
    assert!(menu.retry_model_load.is_some());
    assert!(menu.redownload_model.is_some(), "files are on disk");
    h.script.fail_load.store(false, Ordering::SeqCst);
    h.app.retry_model_load();
    sleep(100).await;
    assert_eq!(h.tray().icon, TrayIcon::Ready);
    assert!(h.menu().retry_model_load.is_none());
}

#[tokio::test(start_paused = true)]
async fn lang_3_an_invalid_language_resets_at_launch_and_warns() {
    struct Case {
        name: &'static str,
        authorization: NotificationAuthorization,
        want_notifications: Vec<&'static str>,
        want_notices: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "a notification when authorized",
            authorization: NotificationAuthorization::Authorized,
            want_notifications: vec!["Invalid Language Setting"],
            want_notices: vec![],
        },
        Case {
            name: "the overlay otherwise (MSG-1)",
            authorization: NotificationAuthorization::Denied,
            want_notifications: vec![],
            want_notices: vec!["Invalid Language Setting"],
        },
    ];
    for tc in cases {
        let h = Harness::unstarted(|s| {
            s.language = LanguageSetting::parse("en").unwrap_or(LanguageSetting::Auto)
        });
        *lock(&h.settings.invalid_language) = Some("xx".to_owned());
        *lock(&h.notifier.authorization) = tc.authorization;
        h.app.ui_ready();
        sleep(10).await;
        assert_eq!(h.notifications(), tc.want_notifications, "{}", tc.name);
        assert_eq!(h.notices(), tc.want_notices, "{}", tc.name);
        assert_eq!(
            h.log.with("settings:reset-language"),
            vec!["settings:reset-language xx"],
            "{}",
            tc.name
        );
        assert_eq!(h.menu().language.subtitle, "Automatic", "{}", tc.name);
    }
    let h = Harness::new().await;
    assert!(h.notifications().is_empty(), "a valid code is left alone");
}

#[tokio::test(start_paused = true)]
async fn dl_15_removed_models_are_cleaned_up_at_launch() {
    let h = Harness::unstarted(|_| {});
    sleep(10).await;
    assert!(
        h.log.with("models:").is_empty(),
        "not before the UI is ready"
    );
    h.app.ui_ready();
    sleep(10).await;
    assert_eq!(h.log.with("models:"), vec!["models:remove-obsolete"]);
}

#[tokio::test(start_paused = true)]
async fn asr_1_launch_shows_loading_at_once_and_skips_loading_during_onboarding() {
    let h = Harness::unstarted(|_| {});
    let hold = common::Hold::new();
    *lock(&h.script.hold_load) = Some(hold.clone());
    h.app.ui_ready();
    eventually("the load", || hold.started()).await;
    assert_eq!(h.tray().icon, TrayIcon::Preparing);
    assert!(matches!(
        h.download_overlay(),
        onda_domain::non_dictation_ui::DownloadOverlayView::Shown { .. }
    ));
    hold.release();
    sleep(10).await;
    assert_eq!(h.tray().icon, TrayIcon::Ready);

    let onboarding =
        Harness::unstarted(|s| s.onboarding = onda_domain::onboarding::OnboardingFlags::default());
    onboarding.app.ui_ready();
    sleep(100).await;
    assert!(onboarding.script.calls().is_empty());
    assert_eq!(
        onboarding.tray().accessibility_label,
        "Onda, finish setup to dictate"
    );
}

#[tokio::test(start_paused = true)]
async fn ovl_17_a_hidden_download_overlay_stays_hidden_until_the_load_ends() {
    let h = Harness::unstarted(|_| {});
    let hold = common::Hold::new();
    *lock(&h.script.hold_load) = Some(hold.clone());
    h.app.ui_ready();
    eventually("the load", || hold.started()).await;
    assert!(matches!(
        h.download_overlay(),
        onda_domain::non_dictation_ui::DownloadOverlayView::Shown { .. }
    ));
    h.app.dismiss_download_overlay();
    // a held load keeps the paused clock from advancing, so poll
    eventually("the hide", || {
        h.download_overlay() == onda_domain::non_dictation_ui::DownloadOverlayView::Hidden
    })
    .await;
    assert_eq!(
        h.tray().icon,
        TrayIcon::Preparing,
        "the tray keeps progress"
    );
    hold.release();
    sleep(10).await;
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn hot_19_registration_failures_notify_per_episode() {
    struct Case {
        name: &'static str,
        hotkey: Shortcut,
        failures: usize,
        want: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "combo: every failure",
            hotkey: Shortcut::default(),
            failures: 2,
            want: vec!["Hotkey Registration Failed"; 2],
        },
        Case {
            name: "lone modifier: first of the episode",
            hotkey: Shortcut::lone(ModifierKey::AltRight),
            failures: 2,
            want: vec!["Hotkey Registration Failed"],
        },
    ];
    for tc in cases {
        let h = Harness::with(|s| s.hotkey = tc.hotkey.clone()).await;
        for _ in 0..tc.failures {
            h.app.hotkey_registration_failed(-9878);
        }
        sleep(100).await;
        assert_eq!(h.notifications(), tc.want, "{}", tc.name);
    }
}

// ---------------------------------------------------------------------------
// media pause ownership, sounds, idle sleep (MEDIA-1..MEDIA-4, SND-1, DICT-10)

#[tokio::test(start_paused = true)]
async fn media_pause_ownership_across_back_to_back_sessions() {
    let h = Harness::with(|s| {
        s.activation_mode = ActivationMode::Toggle;
        s.pause_media = true;
    })
    .await;
    h.media.playing_default.store(true, Ordering::SeqCst);

    // A pauses media and resumes it when it ends, not at stop
    let hold = h.script.hold_next_final();
    h.tap();
    sleep(500).await;
    assert_eq!(h.log.with("media:"), vec!["media:pause"]);
    h.stop_now();
    eventually("A's decode", || hold.started()).await;
    assert_eq!(
        h.log.with("media:"),
        vec!["media:pause"],
        "not resumed at stop"
    );
    hold.release();
    sleep(500).await;
    assert_eq!(h.log.with("media:"), vec!["media:pause", "media:play"]);

    // B starts with media paused by the user: never resumes it
    h.tap();
    sleep(500).await;
    h.tap();
    sleep(1_000).await;
    assert_eq!(h.log.with("media:"), vec!["media:pause", "media:play"]);

    // C cancels: resumes only what it paused
    h.media.playing_default.store(true, Ordering::SeqCst);
    h.tap();
    sleep(500).await;
    h.app.cancel_dictation();
    sleep(500).await;
    assert_eq!(
        h.log.with("media:"),
        vec!["media:pause", "media:play", "media:pause", "media:play"]
    );
    assert_eq!(h.typed().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn media_1_pause_media_off_never_touches_media() {
    let h = Harness::new().await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    h.dictate().await;
    assert!(h.log.with("media:").is_empty());
}

#[tokio::test(start_paused = true)]
async fn hold_confirmation_pauses_media_once() {
    let h = Harness::with(|s| s.pause_media = true).await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    h.press();
    sleep(200).await;
    assert!(h.log.with("media:").is_empty(), "provisional (MEDIA-4)");
    sleep(800).await;
    assert_eq!(h.log.with("media:"), vec!["media:pause"]);
    h.release();
    sleep(1_000).await;
    assert_eq!(h.log.with("media:"), vec!["media:pause", "media:play"]);
}

#[tokio::test(start_paused = true)]
async fn snd_2_sounds_follow_the_setting() {
    let h = Harness::with(|s| s.sounds = false).await;
    h.dictate().await;
    assert!(h.log.with("sound:").is_empty());
    assert_eq!(h.typed(), vec!["hello world"]);
}

#[tokio::test(start_paused = true)]
async fn dict_10_idle_sleep_is_prevented_until_the_session_ends() {
    let h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    let hold = h.script.hold_next_final();
    h.tap();
    sleep(100).await;
    assert_eq!(
        h.log.with("power:"),
        vec!["power:begin Dictation in progress"]
    );
    h.stop_now();
    eventually("the decode", || hold.started()).await;
    assert_eq!(
        h.log.with("power:").len(),
        1,
        "still held while transcribing"
    );
    hold.release();
    sleep(500).await;
    assert_eq!(
        h.log.with("power:"),
        vec!["power:begin Dictation in progress", "power:end"]
    );
}

// ---------------------------------------------------------------------------
// shutdown (DICT-28a)

#[tokio::test(start_paused = true)]
async fn dict_28a_quit_cancels_resumes_media_and_delivers_nothing() {
    let h = Harness::with(|s| {
        s.activation_mode = ActivationMode::Toggle;
        s.pause_media = true;
    })
    .await;
    h.media.playing_default.store(true, Ordering::SeqCst);
    h.tap();
    sleep(500).await;
    h.tap();
    sleep(10).await;
    let log = h.log.clone();
    let text = h.text.clone();
    h.app.shutdown_async().await;
    assert_eq!(log.with("media:"), vec!["media:pause", "media:play"]);
    assert!(lock(&text.typed).is_empty());
}
