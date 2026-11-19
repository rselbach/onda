#[path = "../../../onda-app/tests/common/mod.rs"]
mod common;

use std::sync::atomic::Ordering;

use common::{Harness, Hold, eventually, lock};
use onda_app::seams::AudioSignal;
use onda_domain::clock::SESSION_LIMIT;
use onda_domain::messages::UserMessage;
use tokio::time::timeout;

use super::*;

const BUDGET: Duration = Duration::from_millis(500);
const WATCHDOG: Duration = Duration::from_secs(3);

#[tokio::test(start_paused = true)]
async fn a_capture_that_never_opens_hits_the_start_deadline_in_every_mode() {
    for mode in ActivationMode::ALL {
        let mut h = Harness::with(|s| s.activation_mode = mode).await;
        tokio::time::resume();
        let capture = h.audio.hold_next_start();
        *lock(&h.audio.start_error) = Some(UserMessage::AudioProcessingError {
            error: "Greendale microphone unavailable".to_owned(),
        });
        let held_for = match mode {
            ActivationMode::Automatic => Duration::ZERO,
            ActivationMode::Hold | ActivationMode::Toggle => Duration::from_millis(u64::MAX),
        };
        let result = timeout(WATCHDOG, dictate_once(&mut h.app, held_for, BUDGET)).await;
        let stopped = timeout(WATCHDOG, h.app.shutdown_async()).await;
        capture.release();
        tokio::time::pause();

        assert!(capture.started(), "{mode:?}");
        assert_eq!(
            result.unwrap().unwrap_err().to_string(),
            "timed out starting dictation",
            "{mode:?}"
        );
        stopped.unwrap();
        assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1);
        assert!(h.typed().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_long_hold_does_not_use_up_the_finish_deadline() {
    let mut h = Harness::with(|s| s.activation_mode = ActivationMode::Automatic).await;
    let held_for = BUDGET * 3;
    let pressed = tokio::time::Instant::now();

    timeout(
        held_for + BUDGET,
        dictate_once(&mut h.app, held_for, BUDGET),
    )
    .await
    .unwrap()
    .unwrap();

    assert!(pressed.elapsed() >= held_for);
    assert_eq!(h.typed(), vec!["hello world"]);
    assert!(lock(&h.audio.released_at).is_some());
}

#[tokio::test(start_paused = true)]
async fn a_stalled_delivery_hits_the_finish_deadline_after_any_stop() {
    #[derive(Clone, Copy, Debug)]
    enum Stop {
        Release,
        SessionLimit,
        DeviceLoss,
    }
    for stop in [Stop::Release, Stop::SessionLimit, Stop::DeviceLoss] {
        let mut h = Harness::with(|s| s.activation_mode = ActivationMode::Automatic).await;
        let delivery = Hold::new();
        *lock(&h.text.hold_commit) = Some(delivery.clone());
        let held_for = match stop {
            Stop::Release => Duration::ZERO,
            Stop::SessionLimit | Stop::DeviceLoss => SESSION_LIMIT * 2,
        };
        let signals = h.app.audio_signals();
        let result = {
            let once = dictate_once(&mut h.app, held_for, BUDGET);
            tokio::pin!(once);
            let drive = async {
                if !matches!(stop, Stop::Release) {
                    eventually("recording started", || {
                        h.log.count("power:begin Dictation in progress") == 1
                    })
                    .await;
                }
                match stop {
                    Stop::Release => {}
                    Stop::SessionLimit => tokio::time::advance(SESSION_LIMIT).await,
                    Stop::DeviceLoss => {
                        tokio::time::advance(BUDGET * 2).await;
                        assert!(signals.send(AudioSignal::Interrupted {
                            capture_id: h.audio.capture_id(),
                            notice: UserMessage::MicrophoneUnavailable {
                                error: "Greendale microphone disconnected".to_owned(),
                            },
                        }));
                    }
                }
            };
            tokio::select! {
                () = drive => {
                    tokio::time::resume();
                    let result = timeout(WATCHDOG, &mut once).await;
                    tokio::time::pause();
                    result
                }
                result = &mut once => Ok(result),
            }
        };
        let stopped = timeout(WATCHDOG, h.app.shutdown_async()).await;
        delivery.release();

        assert!(delivery.started(), "{stop:?}");
        assert_eq!(
            result.unwrap().unwrap_err().to_string(),
            "timed out waiting for dictation to finish",
            "{stop:?}"
        );
        stopped.unwrap();
        eventually("cancelled delivery", || h.log.count("text:cancelled") == 1).await;
        assert!(!h.audio.is_open());
        assert!(h.typed().is_empty());
    }
}

#[test]
fn shutdown_returns_while_a_worker_is_still_blocked() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let (release, blocked) = mpsc::channel();
    let (entered, started) = mpsc::channel();
    let result = shutdown(
        runtime,
        async move {
            entered.send(()).unwrap();
            let _ = blocked.recv_timeout(WATCHDOG);
        },
        Duration::from_millis(50),
    );

    assert!(result.is_err());
    assert!(started.try_recv().is_ok());
    assert!(
        release.send(()).is_ok(),
        "shutdown outlasted the blocked worker"
    );
}

#[test]
fn env_openrouter_key_layers_over_inner_store() {
    use onda_settings::MemorySecretStore;

    let inner = MemorySecretStore::new();
    inner
        .save(CleanupProvider::OpenRouter, "sk-inner-openrouter")
        .unwrap();
    inner
        .save(CleanupProvider::DeepSeek, "sk-deepseek-greendale")
        .unwrap();

    let layered = Keychain::new(EnvOpenRouterKey {
        override_key: "sk-env-troy".to_owned(),
        inner,
    });
    assert_eq!(
        layered.api_key(CleanupProvider::OpenRouter).unwrap(),
        Some("sk-env-troy".to_owned())
    );
    assert_eq!(
        layered.api_key(CleanupProvider::DeepSeek).unwrap(),
        Some("sk-deepseek-greendale".to_owned())
    );
    layered
        .save_api_key(CleanupProvider::Fireworks, "fw-abed")
        .unwrap();
    assert_eq!(
        layered.api_key(CleanupProvider::Fireworks).unwrap(),
        Some("fw-abed".to_owned())
    );
}
