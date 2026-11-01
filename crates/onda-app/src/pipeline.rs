//! Background steps of a session. Each runs off the session actor and
//! returns a message tagged with the session it started under; the actor
//! drops results whose session is no longer current (ARCH-7, DICT-15).

use std::sync::Arc;
use std::time::Duration;

use onda_asr::AsrError;
use onda_domain::cleanup::{
    self, CleanupModel, CleanupOutcome, CleanupProvider, CleanupStart, CleanupStep,
};
use onda_domain::joiner::join;
use onda_domain::languages::Language;
use onda_domain::messages::UserMessage;
use onda_domain::preview::{PREVIEW_INTERVAL, PREVIEW_STOP_WAIT, PreviewLoop, PreviewSnapshot};
use onda_domain::replacements::{self, ReplacementRule};
use onda_domain::types::{MicrophonePermission, MonoTime, OutputMode};
use onda_domain::typing::{DeliveryRoute, delivery_route, text_before_caret};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, spawn_blocking};
use tokio::time::Instant;

use crate::ports::{Permissions, Target, TextOutput};
use crate::seams::{AudioSource, CleanupTransport, SecretSource};
use crate::session::{Msg, SessionId};
use crate::transcription::TranscriptionService;

/// What the start checks found (DICT-1 steps 3 and 5, DICT-3).
#[derive(Debug, Clone)]
pub(crate) struct CheckReport {
    /// Microphone permission after any prompt.
    pub(crate) microphone: MicrophonePermission,
    pub(crate) accessibility: bool,
    pub(crate) target: Option<Target>,
}

/// Reads permissions, prompting for the microphone when it was never asked
/// and `wants_microphone`, then captures the focused app.
pub(crate) async fn check(
    permissions: Arc<dyn Permissions>,
    text_output: Arc<dyn TextOutput>,
    wants_microphone: bool,
) -> CheckReport {
    let report = spawn_blocking(move || {
        let microphone = match permissions.microphone() {
            MicrophonePermission::NotDetermined if wants_microphone => {
                if permissions.request_microphone() {
                    MicrophonePermission::Granted
                } else {
                    MicrophonePermission::Denied
                }
            }
            other => other,
        };
        CheckReport {
            microphone,
            accessibility: permissions.accessibility_trusted(),
            target: text_output.focused_target(),
        }
    })
    .await;
    report.unwrap_or_else(|error| {
        tracing::error!(%error, "permission check failed");
        CheckReport {
            microphone: MicrophonePermission::NotDetermined,
            accessibility: false,
            target: None,
        }
    })
}

/// A running live-preview loop (ASR-15..ASR-19). Dropping it aborts the
/// loop; [`PreviewHandle::finish`] lets an in-flight decode complete.
pub(crate) struct PreviewHandle {
    stop: watch::Sender<bool>,
    join: JoinHandle<Option<PreviewSnapshot>>,
}

impl PreviewHandle {
    /// Starts the loop for session `id`.
    pub(crate) fn spawn(context: PreviewContext) -> Self {
        let (stop, stopped) = watch::channel(false);
        let join = tokio::spawn(run_preview(context, stopped));
        Self { stop, join }
    }

    /// Stops scheduling ticks; an in-flight decode still finishes.
    pub(crate) fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// Stops the loop and waits up to [`PREVIEW_STOP_WAIT`] for an in-flight
    /// decode (DICT-13), returning the last decode for reuse (ASR-21).
    async fn finish(mut self) -> Option<PreviewSnapshot> {
        self.stop();
        match tokio::time::timeout(PREVIEW_STOP_WAIT, &mut self.join).await {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(_)) => None,
            Err(_) => {
                // the decode itself cannot be interrupted; the final decode
                // queues behind it on the model gate
                tracing::warn!("timed out waiting for the preview to stop");
                None
            }
        }
    }
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        self.join.abort();
    }
}

pub(crate) struct PreviewContext {
    pub(crate) id: SessionId,
    pub(crate) service: TranscriptionService,
    pub(crate) audio: Arc<dyn AudioSource>,
    pub(crate) rules: Vec<ReplacementRule>,
    pub(crate) language: Option<Language>,
    pub(crate) origin: Instant,
    pub(crate) messages: mpsc::UnboundedSender<Msg>,
}

async fn run_preview(
    context: PreviewContext,
    mut stopped: watch::Receiver<bool>,
) -> Option<PreviewSnapshot> {
    context.service.reset_preview().await;
    let mut preview = PreviewLoop::new();
    let mut last = None;
    loop {
        if *stopped.borrow() {
            break;
        }
        let count = context.audio.sample_count();
        if preview.tick(count) == onda_domain::preview::TickDecision::Decode {
            let samples = context.audio.samples();
            let decoded = samples.len();
            let started = Instant::now();
            match context.service.preview(samples, context.language).await {
                Ok(text) => {
                    tracing::info!(
                        audio_ms = decoded * 1_000 / 16_000,
                        decode_ms = started.elapsed().as_millis() as u64,
                        "preview tick"
                    );
                    let text = text.trim().to_owned();
                    let replaced = replacements::apply(&text, &context.rules);
                    if let Some(shown) = preview.decode_finished(&replaced) {
                        let _ = context.messages.send(Msg::Preview {
                            id: context.id,
                            text: shown,
                        });
                    }
                    last = Some(PreviewSnapshot {
                        text,
                        sample_count: decoded,
                        finished_at: mono(context.origin),
                    });
                }
                Err(error) => {
                    // preview errors are logged, never shown (ASR-22)
                    tracing::warn!(?error, "preview decode failed");
                    preview.decode_failed();
                }
            }
        }
        tokio::select! {
            () = tokio::time::sleep(PREVIEW_INTERVAL) => {}
            _ = stopped.changed() => break,
        }
    }
    last
}

/// The final decode's result.
#[derive(Debug)]
pub(crate) enum Decoded {
    Text(String),
    /// Shorter than the loaded engine accepts
    /// ([`SpeechEngine::min_samples`](onda_asr::SpeechEngine::min_samples)):
    /// dropped silently like a GATE-2 reject, not a failure.
    TooShort,
    Failed(AsrError),
}

pub(crate) struct DecodeJob {
    pub(crate) id: SessionId,
    pub(crate) service: TranscriptionService,
    pub(crate) audio: Arc<[f32]>,
    pub(crate) preview: Option<PreviewHandle>,
    /// Parakeet TDT models may reuse a fresh preview (ASR-21).
    pub(crate) reuse_allowed: bool,
    pub(crate) language: Option<Language>,
    pub(crate) origin: Instant,
}

/// Waits for the preview, then decodes or reuses it (DICT-13, ASR-21).
pub(crate) async fn decode(job: DecodeJob) -> Msg {
    let snapshot = match job.preview {
        Some(preview) => preview.finish().await,
        None => None,
    };
    let asr_started = Instant::now();
    let reusable = snapshot
        .filter(|snapshot| job.reuse_allowed && snapshot.can_reuse(&job.audio, mono(job.origin)));
    let result = if job.audio.len() < job.service.min_samples() {
        Decoded::TooShort
    } else if let Some(snapshot) = reusable {
        tracing::info!("final transcript reused the last preview");
        Decoded::Text(snapshot.text)
    } else {
        match job.service.transcribe(job.audio, job.language).await {
            Ok(text) => Decoded::Text(text),
            Err(error) => Decoded::Failed(error),
        }
    };
    Msg::Decoded {
        id: job.id,
        result,
        asr: (asr_started, Instant::now()),
    }
}

pub(crate) struct CleanupJob {
    pub(crate) id: SessionId,
    pub(crate) secrets: Arc<dyn SecretSource>,
    pub(crate) transport: Arc<dyn CleanupTransport>,
    pub(crate) provider: CleanupProvider,
    pub(crate) model: CleanupModel,
    pub(crate) raw: String,
    pub(crate) app_name: Option<String>,
    pub(crate) messages: mpsc::UnboundedSender<Msg>,
}

/// Runs cleanup with its one guardrail retry (CLN-1..CLN-15). The key is
/// read once and used for both requests (CLN-2).
pub(crate) async fn clean_up(job: CleanupJob) -> Msg {
    let secrets = job.secrets.clone();
    let provider = job.provider;
    let key = match spawn_blocking(move || secrets.api_key(provider)).await {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => {
            tracing::warn!(%error, provider = provider.id(), "could not read the cleanup API key");
            None
        }
        Err(error) => {
            tracing::error!(%error, "key read failed");
            None
        }
    };
    let outcome = match cleanup::start(provider, true, &job.raw, key.as_deref()) {
        CleanupStart::Skip(outcome) => outcome,
        CleanupStart::Request(key) => {
            let _ = job.messages.send(Msg::Refining(job.id));
            let request = || {
                cleanup::build_request(
                    provider,
                    &key,
                    &job.model,
                    &job.raw,
                    job.app_name.as_deref(),
                )
            };
            let first = job.transport.send(request()).await;
            match cleanup::after_first_attempt(provider, &job.raw, first) {
                CleanupStep::Done(outcome) => outcome,
                CleanupStep::Retry(violation) => {
                    tracing::info!(%violation, "cleanup rejected by guardrails; retrying once");
                    let second = job.transport.send(request()).await;
                    cleanup::after_retry(provider, &job.raw, second)
                }
            }
        }
    };
    log_cleanup(&outcome);
    Msg::Cleaned {
        id: job.id,
        outcome,
    }
}

fn log_cleanup(outcome: &CleanupOutcome) {
    match outcome {
        CleanupOutcome::NotRun => {}
        CleanupOutcome::Cleaned { .. } => tracing::info!("cleanup applied"),
        CleanupOutcome::FellBack { error, .. } => {
            tracing::warn!(%error, "cleanup fell back to the raw transcript");
        }
    }
}

/// Delivers `text` through the output mode (OUT-1..OUT-11) and returns the
/// notices to show.
pub(crate) async fn deliver(
    text_output: Arc<dyn TextOutput>,
    permissions: Arc<dyn Permissions>,
    text: String,
    target: Option<Target>,
    mode: OutputMode,
) -> Vec<UserMessage> {
    let delivered = spawn_blocking(move || {
        let route = delivery_route(mode, permissions.accessibility_trusted());
        match route {
            DeliveryRoute::Copy if text_output.copy(&text) => vec![],
            DeliveryRoute::CopyWithNotice if text_output.copy(&text) => {
                vec![UserMessage::TranscriptCopiedToClipboard]
            }
            DeliveryRoute::Copy | DeliveryRoute::CopyWithNotice => vec![UserMessage::CopyFailed],
            DeliveryRoute::Insert => {
                let joined = match text_output.field_text(target.as_ref()) {
                    Some(field) => join(&text, text_before_caret(&field.text, field.caret_utf16)),
                    None => text,
                };
                text_output
                    .insert(&joined, target.as_ref())
                    .notice()
                    .into_iter()
                    .collect()
            }
        }
    })
    .await;
    delivered.unwrap_or_else(|error| {
        tracing::error!(%error, "delivery failed");
        vec![]
    })
}

/// Time since `origin` in the domain's terms.
pub(crate) fn mono(origin: Instant) -> MonoTime {
    Instant::now().saturating_duration_since(origin)
}

/// Milliseconds between two instants, for the DICT-18 log line.
pub(crate) fn millis(from: Instant, to: Instant) -> u64 {
    to.saturating_duration_since(from).as_millis() as u64
}

/// Sleeps for `after`, then sends `msg`.
pub(crate) async fn after(after: Duration, msg: Msg, messages: mpsc::UnboundedSender<Msg>) {
    tokio::time::sleep(after).await;
    let _ = messages.send(msg);
}
