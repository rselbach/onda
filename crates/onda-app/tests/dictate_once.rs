//! One-shot hotkey dictation against the real session actor and fake ports.

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{Harness, Hold, eventually, lock};
use onda_app::{DictateOnceError, DictationPress, DictationPressState as PressState, Event};
use onda_domain::messages::UserMessage;
use onda_domain::types::ActivationMode;
use tokio::time::{pause, resume, sleep, timeout};

const SHORT_HOLD: Duration = Duration::from_millis(30);
const LONG_HOLD: Duration = Duration::from_millis(500);

async fn recording(press: &mut DictationPress<'_>) {
    let mut state = press.changed().await.unwrap();
    if matches!(state, PressState::Starting(_)) {
        state = press.changed().await.unwrap();
    }
    assert_eq!(state, PressState::Recording);
}

async fn finished(press: &mut DictationPress<'_>) -> Result<(), DictateOnceError> {
    loop {
        if let PressState::Done(result) = press.changed().await? {
            return result;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_provisional_one_shot_finishes_without_a_tray_change() {
    let mut h = Harness::new().await;
    h.clear_events();

    let mut press = h.app.press_dictation_hotkey();
    recording(&mut press).await;
    sleep(SHORT_HOLD).await;
    press.release();
    assert_eq!(
        timeout(LONG_HOLD, finished(&mut press)).await.unwrap(),
        Ok(())
    );
    drop(press);
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1);
    assert!(!h.audio.is_open());
    assert!(h.typed().is_empty());
    assert!(h.history.entries().is_empty());
    assert_eq!(h.count(|event| matches!(event, Event::TrayChanged(_))), 0);
}

#[tokio::test(start_paused = true)]
async fn a_released_hold_or_toggle_does_not_leave_a_pending_capture_open() {
    for mode in [ActivationMode::Hold, ActivationMode::Toggle] {
        let mut h = Harness::with(|s| s.activation_mode = mode).await;
        resume();
        let capture = h.audio.hold_next_start();
        let mut press = h.app.press_dictation_hotkey();
        let starting = press.changed().await;
        eventually("pending capture", || capture.started()).await;
        press.release();
        let finishing = timeout(LONG_HOLD, press.changed()).await;
        capture.release();
        assert_eq!(starting.unwrap(), PressState::Starting(mode));
        assert!(matches!(
            finishing.unwrap().unwrap(),
            PressState::Finishing(_)
        ));
        assert_eq!(
            timeout(LONG_HOLD, finished(&mut press)).await.unwrap(),
            Ok(())
        );
        drop(press);
        assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1, "{mode:?}");
        assert!(!h.audio.is_open(), "{mode:?}");
        assert!(h.typed().is_empty(), "{mode:?}");
        pause();
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_start_resolves_without_waiting_for_automatic_recording_or_restarting_toggle() {
    for mode in ActivationMode::ALL {
        let mut h = Harness::with(|s| s.activation_mode = mode).await;
        *lock(&h.audio.start_error) = Some(UserMessage::AudioProcessingError {
            error: "Greendale microphone unavailable".to_owned(),
        });

        let mut press = h.app.press_dictation_hotkey();
        let result = timeout(SHORT_HOLD, finished(&mut press)).await;
        assert_eq!(
            result.unwrap(),
            Err(DictateOnceError::NotStarted),
            "{mode:?}"
        );
        assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1, "{mode:?}");
        assert!(!h.audio.is_open(), "{mode:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_automatic_press_cannot_be_released_until_capture_opens() {
    let mut h = Harness::with(|s| s.activation_mode = ActivationMode::Automatic).await;
    let capture = h.audio.hold_next_start();
    let mut press = h.app.press_dictation_hotkey();
    let starting = press.changed().await;
    eventually("pending capture", || capture.started()).await;
    press.release();
    capture.release();
    assert_eq!(
        starting.unwrap(),
        PressState::Starting(ActivationMode::Automatic)
    );
    assert_eq!(
        timeout(LONG_HOLD, press.changed()).await.unwrap().unwrap(),
        PressState::Recording
    );
    press.release();
    assert_eq!(
        timeout(LONG_HOLD, finished(&mut press)).await.unwrap(),
        Ok(())
    );
    drop(press);
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(!h.audio.is_open());
}

#[tokio::test(start_paused = true)]
async fn dropping_a_press_during_startup_allows_the_next_press_to_record() {
    let mut h = Harness::with(|s| s.activation_mode = ActivationMode::Toggle).await;
    let capture = h.audio.hold_next_start();
    let mut press = h.app.press_dictation_hotkey();
    let starting = press.changed().await;
    eventually("pending capture", || capture.started()).await;
    drop(press);
    capture.release();
    assert_eq!(
        starting.unwrap(),
        PressState::Starting(ActivationMode::Toggle)
    );
    eventually("abandoned capture closed", || {
        h.audio.stops.load(Ordering::SeqCst) == 1 && h.idle() && !h.audio.is_open()
    })
    .await;

    let mut next = h.app.press_dictation_hotkey();
    recording(&mut next).await;
    next.release();
    assert_eq!(
        timeout(LONG_HOLD, finished(&mut next)).await.unwrap(),
        Ok(())
    );
    drop(next);
    assert_eq!(h.typed(), vec!["hello world"]);
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn a_successful_one_shot_waits_until_output_returns_in_every_mode() {
    let cases = [
        (ActivationMode::Hold, LONG_HOLD, true),
        (ActivationMode::Toggle, LONG_HOLD, true),
        (ActivationMode::Automatic, SHORT_HOLD, false),
        (ActivationMode::Automatic, LONG_HOLD, true),
    ];
    for (mode, held_for, wants_tail) in cases {
        let mut h = Harness::with(|s| s.activation_mode = mode).await;
        resume();
        let delivery = Hold::new();
        *lock(&h.text.hold_insert) = Some(delivery.clone());
        let mut press = h.app.press_dictation_hotkey();
        recording(&mut press).await;
        sleep(held_for).await;
        press.release();
        let result = {
            let release = finished(&mut press);
            tokio::pin!(release);
            let premature = tokio::select! {
                _ = &mut release => true,
                () = eventually("blocked delivery", || delivery.started()) => false,
            };
            delivery.release();
            assert!(!premature, "{mode:?}, {held_for:?}");
            timeout(LONG_HOLD, release).await.unwrap()
        };
        assert_eq!(result, Ok(()), "{mode:?}, {held_for:?}");
        drop(press);
        assert_eq!(h.typed(), vec!["hello world"]);
        assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1);
        assert_eq!(lock(&h.audio.released_at).is_some(), wants_tail);
        pause();
    }
}
