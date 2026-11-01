//! ASR-22: preview errors are logged, never shown. Its own test binary:
//! tracing caches each log site's interest process-wide, so a capturing
//! subscriber races with tests on other threads that have none.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::{EngineCall, Harness, lock, loud, sleep};
use onda_domain::types::ActivationMode;

#[derive(Clone, Default)]
struct LogBuffer(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn asr_22_preview_errors_are_logged_never_shown() {
    let logs = LogBuffer::default();
    // the production filter (LOG-1)
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer({
            let logs = logs.clone();
            move || logs.clone()
        })
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = Harness::unstarted(|s| s.activation_mode = ActivationMode::Toggle);
    h.script.supports_preview.store(true, Ordering::SeqCst);
    h.script.fail_preview.store(true, Ordering::SeqCst);
    h.app.ui_ready();
    sleep(1).await;
    h.audio.starts_with(loud(280_000));
    h.tap();
    sleep(700).await;
    assert!(
        h.script
            .calls()
            .iter()
            .any(|call| matches!(call, EngineCall::Preview(_))),
        "previews were attempted"
    );
    h.stop_now();
    sleep(1_000).await;

    assert_eq!(h.typed(), vec!["hello world"], "the dictation still lands");
    assert!(h.previews().is_empty(), "no preview text");
    assert!(h.notices().is_empty(), "no overlay notice");
    assert!(h.notifications().is_empty(), "no notification");
    let text = String::from_utf8_lossy(&lock(&logs.0)).into_owned();
    let failures: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("preview decode failed"))
        .collect();
    assert!(!failures.is_empty(), "logged: {text}");
    assert!(
        failures[0].contains("the study room projector caught fire"),
        "with its description (LOG-2): {}",
        failures[0]
    );
}
