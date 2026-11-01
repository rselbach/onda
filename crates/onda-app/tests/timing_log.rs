//! The DICT-18 timing log line. Its own test binary: tracing caches each
//! log site's interest process-wide, so a capturing subscriber races with
//! tests on other threads that have none.

mod common;

use std::sync::Arc;

use common::{Harness, lock};

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
async fn dict_18_one_timing_line_per_dictation() {
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer({
            let logs = logs.clone();
            move || logs.clone()
        })
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.cleanup.respond(vec![Ok("Hello world.".to_owned())]);
    h.dictate().await;
    assert_eq!(h.typed(), vec!["Hello world."]);
    let text = String::from_utf8_lossy(&lock(&logs.0)).into_owned();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("dictation pipeline"))
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    for field in [
        "total_ms=",
        "capture_ms=",
        "queue_ms=",
        "asr_ms=",
        "cleanup_ms=",
        "history_ms=",
    ] {
        assert!(lines[0].contains(field), "{field} missing: {}", lines[0]);
    }
    assert!(!lines[0].contains("hello"), "timings only (LOG-2)");
}
