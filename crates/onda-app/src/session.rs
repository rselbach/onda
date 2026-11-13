//! The session actor (ARCH-7): one task owns every piece of dictation
//! state and handles commands, hotkey events, timers, and background
//! results from one ordered channel (ARCH-11, HOT-20a).
//!
//! Handling a message never awaits: shell port calls, engine calls, and
//! HTTP run in spawned tasks that send a message back tagged with the
//! session id. A result for a session that is no longer current is dropped
//! (DICT-15), and cancelling frees the session at once while background
//! inference finishes on its own (ARCH-10).

mod cleanup;
mod history;
mod hotkeys;
mod models;
mod onboarding;
mod rules_editor;
mod windows;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use onda_asr::AsrError;
use onda_domain::cleanup::CleanupOutcome;
use onda_domain::clock::{RecordingClock, SESSION_LIMIT};
use onda_domain::history::{NewEntry, RecentTranscript};
use onda_domain::hotkey::{
    DebounceToken, GestureInput, GlobeKeyAction, HotkeyView, LONE_MODIFIER_DEBOUNCE, PressIntent,
    PressRouter, SessionSnapshot, Shortcut, ShortcutRecorder, cancel_hotkey_modifiers,
    provisional_confirmation_delay,
};
use onda_domain::language_menu;
use onda_domain::level::SilentMicDetector;
use onda_domain::messages::{MessageClass, NoticeRateLimiter, RateLimitedNotice, UserMessage};
use onda_domain::model_loads::ModelLoads;
use onda_domain::model_selection::{
    PendingSelection, SelectOutcome, load_failure_notification, reload_after_download, select_model,
};
use onda_domain::models::{Catalog, ModelId};
use onda_domain::non_dictation_ui::{self, DownloadOverlay, DownloadOverlayView, DownloadTracker};
use onda_domain::overlay::{ClockDisplay, Notice, Overlay, OverlayView, TranscribingEntry};
use onda_domain::preview_text::menu_preview;
use onda_domain::replacements;
use onda_domain::setup::{
    MissingPermission, SetupChecklist, SetupChecklistInput, SetupGuidance, accessibility_needed,
    missing_permission, setup_guidance,
};
use onda_domain::speech_gate::{self, GateVerdict};
use onda_domain::status_menu::{
    LanguageMenuItem, RecentTranscriptItem, StatusMenu, StatusMenuInput,
};
use onda_domain::tray::{AppStatus, TrayView, tray_view};
use onda_domain::types::{MicrophonePermission, ModelState, SessionPhase};
use onda_domain::typing::requires_accessibility;
use onda_domain::updates::update_control;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, spawn_blocking};
use tokio::time::Instant;
use uuid::Uuid;

use crate::effects::{Effect, EffectPorts, Effects, RegisterOutcome};
use crate::events::{Event, EventBus};
use crate::media::Media;
use crate::pipeline::{
    self, CheckReport, CleanupJob, DecodeJob, Decoded, PreviewContext, PreviewHandle, millis, mono,
};
use crate::ports::{Cue, Delivery, Target};
use crate::seams::{AudioSignal, CaptureInfo, DownloadUpdate, Settings};
use crate::transcription::{LoadError, TranscriptionService};
use crate::views::MainWindowSection;
use crate::{HotkeyEvent, Ports};
use hotkeys::Hotkeys;
use windows::Ui;
pub(crate) use windows::UiMsg;

/// Recording continues this long after a hotkey stop (DICT-11).
pub(crate) const RELEASE_TAIL: Duration = Duration::from_millis(200);

/// The only button of a critical alert.
const ALERT_BUTTON: &str = "OK";

/// How long shutdown waits for media and shell calls to drain.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(2);

/// Identifies one session; background results carry it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SessionId(pub(crate) u64);

/// Everything the actor handles, in arrival order.
pub(crate) enum Msg {
    // shell commands
    Hotkey(HotkeyEvent),
    HotkeyRegistrationFailed(i32),
    SetHotkey(Shortcut),
    CaptureBegan,
    CaptureEnded,
    CaptureCommitted(Shortcut),
    RecorderChanged,
    DismissDownloadOverlay,
    MenuToggle,
    StartDictation,
    StopDictation,
    Cancel,
    Retry,
    DiscardFailed,
    DeliverLast,
    DeliverText(String),
    SelectModel(ModelId),
    DownloadAndUse(ModelId),
    Download(DownloadUpdate),
    RetryModelLoad,
    UiReady,
    AppBecameActive,
    SettingsChanged,
    Audio(AudioSignal),
    Ui(UiMsg),
    Shutdown(oneshot::Sender<()>),
    // timers
    Debounce(DebounceToken),
    Confirm(SessionId),
    ClockTick,
    Limit(SessionId),
    TailElapsed(SessionId),
    NoticeTick,
    PollAccessibility,
    // background results
    HotkeyRegistered {
        generation: u64,
        outcome: RegisterOutcome,
    },
    HotkeyFailure {
        // `None` for failures the shell reported on its own
        generation: Option<u64>,
        status: i32,
        trusted: bool,
    },
    TrustChecked(bool),
    GlobeKey(GlobeKeyAction),
    StartChecked {
        id: SessionId,
        report: CheckReport,
    },
    CaptureOpened {
        id: SessionId,
        result: Result<CaptureInfo, UserMessage>,
    },
    Preview {
        id: SessionId,
        text: String,
    },
    Decoded {
        id: SessionId,
        result: Decoded,
        asr: (Instant, Instant),
    },
    Refining(SessionId),
    Cleaned {
        id: SessionId,
        outcome: CleanupOutcome,
    },
    Saved {
        id: SessionId,
        saved: bool,
    },
    Delivered {
        id: Option<SessionId>,
        notices: Vec<UserMessage>,
    },
    RedeliveryChecked {
        text: String,
        report: CheckReport,
        delivery: Delivery,
    },
    Fallback(UserMessage),
    ModelState(ModelState),
    LoadFinished(Result<(), LoadError>),
    Permissions(PermissionSnapshot),
    Downloaded(Vec<ModelId>),
    LaunchChecked(Vec<ModelId>),
    Recent {
        generation: u64,
        recent: Vec<RecentTranscript>,
    },
}

/// Last known permissions, for the tray, the menu, and setup guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PermissionSnapshot {
    microphone: MicrophonePermission,
    accessibility: bool,
}

// a spawned task aborted when its owner drops it, so leaving a phase
// cancels everything that phase started
struct Task(AbortHandle);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// a delivery cancelled when its owner drops it: aborting the task alone
// leaves the blocking port calls running (DICT-19)
struct Outgoing(Delivery);

impl Drop for Outgoing {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartKind {
    Record { provisional: bool },
    Retry,
}

struct Opening {
    target: Option<Target>,
    media_begun: bool,
}

// a start awaiting its checks or the microphone
struct Starting {
    id: SessionId,
    kind: StartKind,
    settings: Arc<Settings>,
    opening: Option<Opening>,
    _task: Task,
}

struct Recording {
    id: SessionId,
    provisional: bool,
    capture_id: u64,
    device_name: String,
    started_at: Instant,
    target: Option<Target>,
    settings: Arc<Settings>,
    media_begun: bool,
    preview: Option<PreviewHandle>,
    // the overlay resets when it appears (OVL-11), so a preview decoded
    // during the provisional window is shown again then
    last_preview: Option<String>,
    silent: SilentMicDetector,
    _timers: Vec<Task>,
    confirm: Option<Task>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Recorded { media_begun: bool },
    Retry,
}

#[derive(Debug, Clone, Copy)]
struct Timings {
    stopped: Instant,
    captured: Instant,
    asr: Option<(Instant, Instant)>,
    cleanup: Option<(Instant, Instant)>,
    history: Option<(Instant, Instant)>,
}

impl Timings {
    fn new(stopped: Instant, captured: Instant) -> Self {
        Self {
            stopped,
            captured,
            asr: None,
            cleanup: None,
            history: None,
        }
    }

    // DICT-18: one line per dictation, timings only (LOG-2)
    fn log(&self) {
        let span = |pair: Option<(Instant, Instant)>| pair.map_or(0, |(a, b)| millis(a, b));
        let queue = self
            .asr
            .map_or(0, |(start, _)| millis(self.captured, start));
        tracing::info!(
            total_ms = millis(self.stopped, Instant::now()),
            capture_ms = millis(self.stopped, self.captured),
            queue_ms = queue,
            asr_ms = span(self.asr),
            cleanup_ms = span(self.cleanup),
            history_ms = span(self.history),
            "dictation pipeline"
        );
    }
}

struct Head {
    id: SessionId,
    origin: Origin,
    target: Option<Target>,
    settings: Arc<Settings>,
    timings: Timings,
}

impl Head {
    fn media_session(&self) -> Option<SessionId> {
        match self.origin {
            Origin::Recorded { media_begun: true } => Some(self.id),
            Origin::Recorded { media_begun: false } | Origin::Retry => None,
        }
    }
}

enum Step {
    Decoding {
        _task: Task,
    },
    Cleaning {
        raw: String,
        _task: Task,
    },
    Saving {
        outcome: CleanupOutcome,
        text: String,
    },
    Delivering {
        _task: Task,
        _delivery: Outgoing,
    },
}

enum Stage {
    // the 200 ms after a hotkey stop: still capturing, already reported as
    // transcribing (DICT-11)
    Tail {
        release_count: usize,
        capture_id: u64,
        preview: Option<PreviewHandle>,
        _timer: Task,
    },
    Pipeline {
        audio: Arc<[f32]>,
        step: Step,
    },
}

struct Transcribing {
    head: Head,
    stage: Stage,
}

enum Phase {
    Idle,
    Starting(Starting),
    // cancelled while its check or microphone step ran. It holds the start
    // slot until that step returns, so two starts never overlap (HOT-28).
    Aborting(Starting),
    Recording(Box<Recording>),
    Transcribing(Box<Transcribing>),
}

#[derive(Default)]
struct Published {
    tray: Option<TrayView>,
    menu: Option<StatusMenu>,
    overlay: Option<OverlayView>,
    download: Option<DownloadOverlayView>,
    hotkey: Option<HotkeyView>,
}

pub(crate) struct Actor {
    messages: mpsc::UnboundedSender<Msg>,
    origin: Instant,
    ports: Ports,
    catalog: Catalog,
    events: Arc<EventBus>,
    effects: Effects,
    media: Option<Media>,
    service: TranscriptionService,
    settings: Arc<Settings>,
    phase: Phase,
    next_id: u64,
    press: PressRouter,
    hotkeys: Hotkeys,
    limiter: NoticeRateLimiter,
    overlay: Overlay,
    download_overlay: DownloadOverlay,
    notice_timer: Option<Task>,
    model_state: ModelState,
    loads: ModelLoads,
    downloaded: Vec<ModelId>,
    downloads: DownloadTracker,
    // the declined-migration fallback loaded instead of the selection (DL-14)
    override_model: Option<ModelId>,
    // the selected model whose download finished while a session was using
    // the engine; go_idle reloads for it
    reload_when_idle: Option<ModelId>,
    pending: PendingSelection,
    permissions: PermissionSnapshot,
    accessibility_prompted: bool,
    failed_audio: Option<Arc<[f32]>>,
    // a restart to update was accepted and has not been declined: no
    // dictation may start (UPD-8)
    restarting: bool,
    last_transcript: Option<String>,
    // output from Paste Last Transcript or a Recent Transcripts entry
    redelivery: Option<Outgoing>,
    recent: Vec<RecentTranscript>,
    cancel_registered: bool,
    preventing_sleep: bool,
    published: Published,
    ui: Ui,
}

impl Actor {
    pub(crate) fn new(
        ports: Ports,
        events: Arc<EventBus>,
        messages: mpsc::UnboundedSender<Msg>,
        recorder: Arc<Mutex<ShortcutRecorder>>,
    ) -> Self {
        let fallback = messages.clone();
        let effects = Effects::spawn(
            EffectPorts {
                sounds: ports.sounds.clone(),
                power: ports.power.clone(),
                hotkeys: ports.hotkeys.clone(),
                permissions: ports.permissions.clone(),
                notifier: ports.notifier.clone(),
                login_item: ports.login_item.clone(),
                updater: ports.updater.clone(),
                tray_term: ports.tray_term.clone(),
                privacy_naming: ports.privacy_naming,
            },
            move |message| {
                let _ = fallback.send(Msg::Fallback(message));
            },
        );
        let states = messages.clone();
        let service = TranscriptionService::new(
            ports.models.clone(),
            Arc::new(move |state| {
                let _ = states.send(Msg::ModelState(state));
            }),
        );
        let media = ports.media.clone().map(Media::spawn);
        let settings = Arc::new(ports.settings.snapshot());
        Self {
            messages,
            origin: Instant::now(),
            catalog: ports.catalog.clone(),
            events,
            effects,
            media,
            service,
            settings,
            phase: Phase::Idle,
            next_id: 0,
            press: PressRouter::new(),
            hotkeys: Hotkeys::new(recorder),
            limiter: NoticeRateLimiter::new(),
            overlay: Overlay::new(),
            download_overlay: DownloadOverlay::new(),
            notice_timer: None,
            model_state: ModelState::NotReady,
            loads: ModelLoads::new(),
            downloaded: Vec::new(),
            downloads: DownloadTracker::new(),
            override_model: None,
            reload_when_idle: None,
            pending: PendingSelection::new(),
            permissions: PermissionSnapshot {
                microphone: MicrophonePermission::NotDetermined,
                accessibility: false,
            },
            accessibility_prompted: false,
            failed_audio: None,
            restarting: false,
            last_transcript: None,
            redelivery: None,
            recent: Vec::new(),
            cancel_registered: false,
            preventing_sleep: false,
            published: Published::default(),
            ui: Ui::default(),
            ports,
        }
    }

    pub(crate) async fn run(mut self, mut inbox: mpsc::UnboundedReceiver<Msg>) {
        self.register_hotkey(true);
        self.publish();
        self.refresh_permissions();
        self.refresh_downloaded();
        self.refresh_recent();
        while let Some(msg) = inbox.recv().await {
            if let Msg::Shutdown(done) = msg {
                self.shut_down(done);
                return;
            }
            self.handle(msg);
            self.publish();
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            // settings are re-read only when a press reaches the router, so
            // the keystrokes a lone-modifier stream forwards stay cheap
            Msg::Hotkey(event) => self.on_hotkey(event),
            Msg::HotkeyRegistrationFailed(status) => self.check_hotkey_failure(None, status),
            Msg::SetHotkey(shortcut) => self.set_hotkey(&shortcut),
            Msg::CaptureBegan => self.capture_began(),
            Msg::CaptureEnded => self.capture_ended(),
            Msg::CaptureCommitted(shortcut) => self.capture_committed(&shortcut),
            Msg::RecorderChanged => {}
            Msg::DismissDownloadOverlay => self.download_overlay.dismiss(),
            Msg::MenuToggle => {
                self.refresh_settings();
                self.menu_toggle();
            }
            Msg::StartDictation => {
                if matches!(self.phase, Phase::Idle) {
                    self.refresh_settings();
                    self.menu_toggle();
                }
            }
            Msg::StopDictation => match self.phase {
                Phase::Starting(_) => self.abort_start(),
                Phase::Recording(_) => self.menu_toggle(),
                _ => {}
            },
            Msg::Cancel => self.cancel(),
            Msg::Retry => {
                self.refresh_settings();
                self.request_start(StartKind::Retry);
            }
            Msg::DiscardFailed => self.discard_failed(),
            Msg::DeliverLast => {
                if let Some(text) = self.last_transcript.clone() {
                    self.redeliver(text);
                }
            }
            Msg::DeliverText(text) => self.redeliver(text),
            Msg::SelectModel(model) => {
                self.refresh_settings();
                // any explicit selection attempt clears "Download & Use"
                // (DL-8) and ends a declined migration (DL-14)
                self.pending.clear();
                self.end_declined();
                self.switch_model(model, true);
            }
            Msg::DownloadAndUse(model) => {
                self.pending.download_and_use(model);
                self.begin_download(model);
            }
            Msg::Download(update) => {
                self.refresh_settings();
                self.on_download(update);
            }
            Msg::RetryModelLoad => {
                // ignored while a load is in flight (ASR-10)
                if !self.load_in_flight() {
                    self.refresh_settings();
                    self.start_load(false);
                }
            }
            Msg::UiReady => {
                self.refresh_settings();
                self.ui_ready();
            }
            Msg::AppBecameActive => {
                self.refresh_settings();
                self.refresh_permissions();
                self.hotkey_app_became_active();
                self.windows_app_became_active();
            }
            Msg::SettingsChanged => self.hotkey_settings_changed(),
            Msg::Audio(signal) => self.on_audio(signal),
            Msg::Ui(msg) => self.on_ui(msg),
            Msg::Shutdown(done) => {
                let _ = done.send(());
            }
            Msg::Debounce(token) => self.gesture_input(GestureInput::DebounceElapsed(token)),
            Msg::Confirm(id) => self.confirm(id),
            Msg::ClockTick => {}
            Msg::Limit(id) => self.limit_reached(id),
            Msg::TailElapsed(id) => self.end_tail(id),
            Msg::NoticeTick => self.notice_tick(),
            Msg::PollAccessibility => self.poll_accessibility(),
            Msg::HotkeyRegistered {
                generation,
                outcome,
            } => self.hotkey_registered(generation, outcome),
            Msg::HotkeyFailure {
                generation,
                status,
                trusted,
            } => self.hotkey_failure(generation, status, trusted),
            Msg::TrustChecked(trusted) => self.trust_checked(trusted),
            Msg::GlobeKey(action) => self.hotkeys.globe = action,
            Msg::StartChecked { id, report } => self.start_checked(id, report),
            Msg::CaptureOpened { id, result } => self.capture_opened(id, result),
            Msg::Preview { id, text } => self.preview(id, text),
            Msg::Decoded { id, result, asr } => self.decoded(id, result, asr),
            Msg::Refining(id) => {
                if self.current_id() == Some(id) {
                    self.overlay.enter_refining();
                }
            }
            Msg::Cleaned { id, outcome } => self.cleaned(id, outcome),
            Msg::Saved { id, saved } => self.saved(id, saved),
            Msg::Delivered { id, notices } => self.delivered(id, notices),
            Msg::RedeliveryChecked {
                text,
                report,
                delivery,
            } => self.redelivery_checked(text, report, delivery),
            Msg::Fallback(message) => self.show_notice(&message),
            Msg::ModelState(state) => {
                self.model_state = state;
                self.loads.reported();
            }
            Msg::LoadFinished(result) => self.load_finished(result),
            Msg::Permissions(snapshot) => self.permissions = snapshot,
            Msg::Downloaded(models) => self.downloaded = models,
            Msg::LaunchChecked(models) => self.launch_checked(models),
            Msg::Recent { generation, recent } => {
                if generation == self.ui.history.generation {
                    self.recent = recent;
                }
            }
        }
    }

    // --- hotkey presses (HOT-24..HOT-33) ---

    fn key_down(&mut self) {
        self.refresh_settings();
        let mut down_at = mono(self.origin);
        // a lone-modifier press gets here only once its debounce has run,
        // and Automatic measures the hold from the physical press
        if self.settings.hotkey.is_lone_modifier() {
            down_at = down_at.saturating_sub(LONE_MODIFIER_DEBOUNCE);
        }
        let intent = self
            .press
            .key_down(self.settings.activation_mode, self.snapshot(), down_at);
        self.apply_intent(intent);
    }

    fn key_up(&mut self) {
        self.refresh_settings();
        let intent = self.press.key_up(
            self.settings.activation_mode,
            self.snapshot(),
            mono(self.origin),
        );
        self.apply_intent(intent);
    }

    fn apply_intent(&mut self, intent: PressIntent) {
        match intent {
            PressIntent::None => {}
            PressIntent::Start { provisional } => {
                self.request_start(StartKind::Record { provisional });
            }
            PressIntent::StopWithTail => self.stop_with_tail(),
            PressIntent::Cancel => self.cancel(),
            PressIntent::AbortPendingStart => self.abort_start(),
        }
    }

    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            phase: self.session_phase(),
            start_pending: matches!(self.phase, Phase::Starting(_) | Phase::Aborting(_)),
        }
    }

    /// A session is starting, recording, or transcribing (UPD-8).
    fn dictating(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    fn menu_toggle(&mut self) {
        // UI-9, DICT-21: menu starts are not provisional and stops have no tail
        match self.phase {
            Phase::Idle => self.request_start(StartKind::Record { provisional: false }),
            Phase::Starting(_) | Phase::Aborting(_) => {}
            Phase::Recording(_) => self.stop_without_tail(),
            Phase::Transcribing(_) => self.cancel(),
        }
    }

    // --- starting (DICT-1..DICT-6, HOT-28..HOT-30, DICT-23) ---

    fn request_start(&mut self, kind: StartKind) {
        if self.restarting {
            tracing::info!("no dictation while Onda restarts to update");
            return;
        }
        if self.model_state != ModelState::Ready {
            // guidance shares the notice's 1.5 s rate limit (DICT-2)
            if self.notify(match self.declined() {
                true => UserMessage::ModelNotDownloaded,
                false => UserMessage::ModelNotReady,
            }) {
                self.setup_guidance();
            }
            return;
        }
        match self.phase {
            Phase::Transcribing(_) => {
                self.notify(UserMessage::StillTranscribing);
                return;
            }
            Phase::Starting(_) | Phase::Recording(_) => return,
            Phase::Aborting(_) => {
                self.resume_start(kind);
                return;
            }
            Phase::Idle => {}
        }
        if kind == StartKind::Retry && self.failed_audio.is_none() {
            return;
        }
        // the new session owns the output (DICT-26, DICT-27)
        self.redelivery = None;
        let id = self.next_session();
        let check = pipeline::check(
            self.ports.permissions.clone(),
            self.ports.text_output.clone(),
            matches!(kind, StartKind::Record { .. }),
        );
        let task = self.spawn(async move {
            let report = check.await;
            Msg::StartChecked { id, report }
        });
        self.phase = Phase::Starting(Starting {
            id,
            kind,
            settings: self.settings.clone(),
            opening: None,
            _task: task,
        });
    }

    // a start asked for while an abandoned one of the same kind is still
    // finishing takes that one over. dropped, it would leave the user
    // holding the key and speaking into nothing once the old start ends.
    fn resume_start(&mut self, kind: StartKind) {
        self.phase = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Aborting(starting) if starting.kind == kind => Phase::Starting(starting),
            other => other,
        };
    }

    fn abort_start(&mut self) {
        self.phase = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Starting(starting) => Phase::Aborting(starting),
            other => other,
        };
    }

    fn start_checked(&mut self, id: SessionId, report: CheckReport) {
        if !matches!(&self.phase, Phase::Starting(s) | Phase::Aborting(s) if s.id == id && s.opening.is_none())
        {
            return;
        }
        self.permissions = PermissionSnapshot {
            microphone: report.microphone,
            accessibility: report.accessibility,
        };
        let (starting, aborted) = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Starting(starting) => (starting, false),
            Phase::Aborting(starting) => (starting, true),
            _ => return,
        };
        if let StartKind::Record { .. } = starting.kind
            && report.microphone != MicrophonePermission::Granted
        {
            if self.notify(UserMessage::MicrophonePermissionRequired) {
                self.setup_guidance();
            }
            return;
        }
        // released while the prompt was up: stop silently (DICT-1 step 4)
        if aborted {
            return;
        }
        if !self.output_ready(&starting.settings, report.accessibility) {
            return;
        }
        match starting.kind {
            StartKind::Retry => {
                let Some(audio) = self.failed_audio.clone() else {
                    return;
                };
                self.enter_retry(id, starting.settings, report.target, audio);
            }
            StartKind::Record { provisional } => {
                let media_begun = !provisional && self.begin_media(id, &starting.settings);
                let audio = self.ports.audio.clone();
                let task = self.spawn(async move {
                    let result =
                        spawn_blocking(move || audio.start())
                            .await
                            .unwrap_or_else(|error| {
                                Err(UserMessage::AudioProcessingError {
                                    error: error.to_string(),
                                })
                            });
                    Msg::CaptureOpened { id, result }
                });
                self.phase = Phase::Starting(Starting {
                    opening: Some(Opening {
                        target: report.target,
                        media_begun,
                    }),
                    _task: task,
                    ..starting
                });
            }
        }
    }

    fn capture_opened(&mut self, id: SessionId, result: Result<CaptureInfo, UserMessage>) {
        if !matches!(&self.phase, Phase::Starting(s) | Phase::Aborting(s) if s.id == id && s.opening.is_some())
        {
            return;
        }
        let (starting, aborted) = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Starting(starting) => (starting, false),
            Phase::Aborting(starting) => (starting, true),
            _ => return,
        };
        let StartKind::Record { provisional } = starting.kind else {
            return;
        };
        let Some(opening) = starting.opening else {
            return;
        };
        let media_begun = opening.media_begun;
        let end_media = move |actor: &mut Self| {
            if media_begun {
                actor.end_media(id);
            }
        };
        match result {
            Err(notice) => {
                // DICT-6: resume media, report, stay idle
                tracing::error!(error = %notice.body(&self.ports.tray_term, self.ports.privacy_naming), "capture failed to start");
                end_media(self);
                self.notify(notice);
            }
            Ok(_) if aborted => {
                self.ports.audio.stop();
                end_media(self);
            }
            Ok(info) => {
                let now = Instant::now();
                self.hold_session_resources();
                let preview = self
                    .service
                    .supports_preview()
                    .then(|| self.spawn_preview(id, &starting.settings));
                let timers = vec![
                    self.timer(SESSION_LIMIT, Msg::Limit(id)),
                    self.clock_ticker(now),
                ];
                let confirm = provisional.then(|| {
                    self.timer(
                        provisional_confirmation_delay(&starting.settings.hotkey),
                        Msg::Confirm(id),
                    )
                });
                self.phase = Phase::Recording(Box::new(Recording {
                    id,
                    provisional,
                    capture_id: info.capture_id,
                    device_name: info.device_name,
                    started_at: now,
                    target: opening.target,
                    settings: starting.settings,
                    media_begun: opening.media_begun,
                    preview,
                    last_preview: None,
                    silent: SilentMicDetector::default(),
                    _timers: timers,
                    confirm,
                }));
                if !provisional {
                    self.show_recording_ui();
                }
            }
        }
    }

    // DICT-1 step 5 and DICT-28: Accessibility for typed output
    fn output_ready(&mut self, settings: &Settings, accessibility: bool) -> bool {
        if !requires_accessibility(settings.output_mode) || accessibility {
            return true;
        }
        if self.notify(UserMessage::AccessibilityPermissionRequired) {
            self.setup_guidance();
        }
        if !self.accessibility_prompted {
            self.accessibility_prompted = true;
            self.effects.send(Effect::PromptAccessibility);
        }
        false
    }

    fn confirm(&mut self, id: SessionId) {
        let Phase::Recording(recording) = &mut self.phase else {
            return;
        };
        if recording.id != id || !recording.provisional {
            return;
        }
        recording.provisional = false;
        recording.confirm = None;
        if recording.settings.pause_media
            && let Some(media) = &self.media
        {
            media.begin(id);
            recording.media_begun = true;
        }
        self.show_recording_ui();
    }

    fn show_recording_ui(&mut self) {
        let Phase::Recording(recording) = &self.phase else {
            return;
        };
        self.overlay.show_recording(
            app_id(recording.target.as_ref()),
            Some(recording.device_name.clone()),
        );
        if let Some(text) = &recording.last_preview {
            self.overlay.set_preview(text.clone());
        }
        if recording.settings.sounds {
            self.effects.send(Effect::Sound(Cue::Start));
        }
    }

    fn enter_retry(
        &mut self,
        id: SessionId,
        settings: Arc<Settings>,
        target: Option<Target>,
        audio: Arc<[f32]>,
    ) {
        // no recording phase: no media, preview, limit, or cue (DICT-23)
        self.hold_session_resources();
        self.overlay.enter_transcribing(app_id(target.as_ref()));
        let now = Instant::now();
        let head = Head {
            id,
            origin: Origin::Retry,
            target,
            settings,
            timings: Timings::new(now, now),
        };
        self.start_pipeline(head, audio, None);
    }

    // --- recording ---

    fn on_audio(&mut self, signal: AudioSignal) {
        match signal {
            AudioSignal::Level(level) => self.level(level),
            AudioSignal::Rebuilt { device_name } => {
                // AUD-13: the silent-mic title names the device now in use
                if let Phase::Recording(recording) = &mut self.phase {
                    if !recording.provisional {
                        self.overlay.set_input_device(Some(device_name.clone()));
                    }
                    recording.device_name = device_name;
                }
            }
            AudioSignal::Interrupted { capture_id, notice } => {
                if self.capturing(capture_id) {
                    // DICT-11, DICT-12: stop with what was captured and report
                    self.notify(notice);
                    match &self.phase {
                        Phase::Recording(_) => self.stop_without_tail(),
                        Phase::Transcribing(t) => {
                            let id = t.head.id;
                            self.end_tail(id);
                        }
                        Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => {}
                    }
                }
            }
            AudioSignal::ConversionFailed { capture_id, notice } => {
                if self.capturing(capture_id)
                    && self
                        .limiter
                        .allow(RateLimitedNotice::AudioConversionFailed, mono(self.origin))
                {
                    self.show_notice(&notice);
                }
            }
        }
    }

    fn preview(&mut self, id: SessionId, text: String) {
        if self.current_id() != Some(id) {
            return;
        }
        if let Phase::Recording(recording) = &mut self.phase {
            recording.last_preview = Some(text.clone());
        }
        self.overlay.set_preview(text);
    }

    fn capturing(&self, capture_id: u64) -> bool {
        match &self.phase {
            Phase::Recording(recording) => recording.capture_id == capture_id,
            Phase::Transcribing(t) => {
                matches!(t.stage, Stage::Tail { capture_id: id, .. } if id == capture_id)
            }
            Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => false,
        }
    }

    fn level(&mut self, level: f32) {
        let now = mono(self.origin);
        match &mut self.phase {
            Phase::Recording(recording) => {
                self.overlay.push_level(level);
                let silent = recording.silent.observe(level, now);
                self.overlay.set_silent(silent);
            }
            Phase::Transcribing(t) if matches!(t.stage, Stage::Tail { .. }) => {
                self.overlay.push_level(level);
            }
            Phase::Transcribing(_) | Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => {}
        }
    }

    fn limit_reached(&mut self, id: SessionId) {
        if !matches!(&self.phase, Phase::Recording(r) if r.id == id) {
            return;
        }
        tracing::info!("recording reached the session limit; transcribing");
        self.notify(UserMessage::RecordingLimitReached);
        self.stop_without_tail();
    }

    // --- stopping (DICT-11..DICT-13) ---

    fn stop_with_tail(&mut self) {
        let Phase::Recording(recording) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return;
        };
        let recording = *recording;
        let release_count = self.ports.audio.mark_release();
        if let Some(preview) = &recording.preview {
            preview.stop();
        }
        self.enter_transcribing_ui(&recording);
        let now = Instant::now();
        let timer = self.timer(RELEASE_TAIL, Msg::TailElapsed(recording.id));
        self.phase = Phase::Transcribing(Box::new(Transcribing {
            head: Head {
                id: recording.id,
                origin: Origin::Recorded {
                    media_begun: recording.media_begun,
                },
                target: recording.target,
                settings: recording.settings,
                timings: Timings::new(now, now),
            },
            stage: Stage::Tail {
                release_count,
                capture_id: recording.capture_id,
                preview: recording.preview,
                _timer: timer,
            },
        }));
    }

    fn end_tail(&mut self, id: SessionId) {
        if !matches!(&self.phase, Phase::Transcribing(t) if t.head.id == id && matches!(t.stage, Stage::Tail { .. }))
        {
            return;
        }
        let Phase::Transcribing(transcribing) = std::mem::replace(&mut self.phase, Phase::Idle)
        else {
            return;
        };
        let Transcribing { mut head, stage } = *transcribing;
        let Stage::Tail {
            release_count,
            preview,
            ..
        } = stage
        else {
            return;
        };
        let samples = self.ports.audio.stop();
        head.timings.captured = Instant::now();
        // the gate judges only the audio before the release (GATE-6)
        match speech_gate::assess_released(&samples, release_count) {
            GateVerdict::Accept if self.too_short_for_engine(samples.len()) => {
                self.go_idle(head.media_session());
            }
            GateVerdict::Accept => self.start_pipeline(head, samples.into(), preview),
            GateVerdict::RejectWithWarning => {
                self.go_idle(head.media_session());
                self.notify(UserMessage::NoSpeechDetected);
            }
            GateVerdict::RejectSilently(_) => self.go_idle(head.media_session()),
        }
    }

    fn stop_without_tail(&mut self) {
        let Phase::Recording(recording) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return;
        };
        let mut recording = *recording;
        let stopped = Instant::now();
        let preview = recording.preview.take();
        if let Some(preview) = &preview {
            preview.stop();
        }
        let samples = self.ports.audio.stop();
        let media_session = recording.media_begun.then_some(recording.id);
        // the gate runs before Transcribing, so a reject has no stop cue
        // (DICT-12)
        match speech_gate::assess(&samples) {
            GateVerdict::Accept if self.too_short_for_engine(samples.len()) => {
                self.go_idle(media_session);
            }
            GateVerdict::Accept => {
                self.enter_transcribing_ui(&recording);
                let head = Head {
                    id: recording.id,
                    origin: Origin::Recorded {
                        media_begun: recording.media_begun,
                    },
                    target: recording.target,
                    settings: recording.settings,
                    timings: Timings::new(stopped, Instant::now()),
                };
                self.start_pipeline(head, samples.into(), preview);
            }
            GateVerdict::RejectWithWarning => {
                self.go_idle(media_session);
                self.notify(UserMessage::NoSpeechDetected);
            }
            GateVerdict::RejectSilently(_) => self.go_idle(media_session),
        }
    }

    // GATE-2 extended to the loaded engine's own minimum (Parakeet: 0.3 s),
    // so a clip between 0.25 s and that minimum ends like a too-short
    // recording: no notice, no history, no kept audio. (Deviation: Jabber
    // sends such a clip to the engine, which fails, and reports
    // "Transcription Failed" with the audio kept for Retry; a clip that
    // short is a stray tap, not a dictation worth retrying. System Speech
    // has no minimum and still transcribes it.)
    fn too_short_for_engine(&self, samples: usize) -> bool {
        samples < self.service.min_samples()
    }

    fn enter_transcribing_ui(&mut self, recording: &Recording) {
        let entry = self
            .overlay
            .enter_transcribing(app_id(recording.target.as_ref()));
        // SND-1: the stop cue only for a session that showed its recording UI
        if entry == TranscribingEntry::FromRecording && recording.settings.sounds {
            self.effects.send(Effect::Sound(Cue::Stop));
        }
    }

    // --- pipeline (DICT-14..DICT-17, DICT-22) ---

    fn start_pipeline(&mut self, head: Head, audio: Arc<[f32]>, preview: Option<PreviewHandle>) {
        let job = DecodeJob {
            id: head.id,
            service: self.service.clone(),
            audio: audio.clone(),
            preview,
            reuse_allowed: matches!(
                self.service.loaded_model(),
                Some(ModelId::ParakeetTdtV2 | ModelId::ParakeetUltra | ModelId::ParakeetJapanese)
            ),
            language: head.settings.language.engine_language(),
            origin: self.origin,
        };
        let task = self.spawn(pipeline::decode(job));
        self.phase = Phase::Transcribing(Box::new(Transcribing {
            head,
            stage: Stage::Pipeline {
                audio,
                step: Step::Decoding { _task: task },
            },
        }));
    }

    fn decoded(&mut self, id: SessionId, result: Decoded, asr: (Instant, Instant)) {
        let Some((head, audio, step)) = self.pipeline_mut(id) else {
            return;
        };
        if !matches!(step, Step::Decoding { .. }) {
            return;
        }
        head.timings.asr = Some(asr);
        let audio = audio.clone();
        match result {
            Decoded::Text(raw) => self.start_cleanup(id, raw),
            // a retry, or a model switched after the gate ran
            Decoded::TooShort => self.end_session(),
            Decoded::Failed(AsrError::Cancelled) => {
                // SYS-7: the platform cancelled the decode; not a failure
                tracing::info!("transcription cancelled");
                self.end_session();
            }
            Decoded::Failed(error) => {
                tracing::error!(?error, "transcription failed");
                // kept before the message so Retry is already available
                // (DICT-22); a newer failure replaces an older one
                self.failed_audio = Some(audio);
                self.end_session();
                self.notify(UserMessage::TranscriptionFailed {
                    error: error.to_string(),
                });
            }
        }
    }

    fn start_cleanup(&mut self, id: SessionId, raw: String) {
        let Some((head, _, _)) = self.pipeline_mut(id) else {
            return;
        };
        let settings = head.settings.clone();
        // read before the cleanup await and used for delivery (DICT-16)
        let app_name = head.target.as_ref().and_then(|t| t.name.clone());
        if !settings.cleanup_enabled || raw.trim().is_empty() {
            self.cleaned_with(id, raw, CleanupOutcome::NotRun);
            return;
        }
        let job = CleanupJob {
            id,
            secrets: self.ports.secrets.clone(),
            transport: self.ports.cleanup.clone(),
            provider: settings.cleanup_provider,
            model: settings.cleanup_model.clone(),
            prompt: settings.cleanup_prompt.clone(),
            raw: raw.clone(),
            app_name,
            messages: self.messages.clone(),
        };
        let started = Instant::now();
        let task = self.spawn(pipeline::clean_up(job));
        if let Some((head, _, step)) = self.pipeline_mut(id) {
            head.timings.cleanup = Some((started, started));
            *step = Step::Cleaning { raw, _task: task };
        }
    }

    fn cleaned(&mut self, id: SessionId, outcome: CleanupOutcome) {
        let Some((head, _, step)) = self.pipeline_mut(id) else {
            return;
        };
        let Step::Cleaning { raw, .. } = step else {
            return;
        };
        let raw = std::mem::take(raw);
        if let Some((started, _)) = head.timings.cleanup {
            head.timings.cleanup = Some((started, Instant::now()));
        }
        self.cleaned_with(id, raw, outcome);
    }

    fn cleaned_with(&mut self, id: SessionId, raw: String, outcome: CleanupOutcome) {
        let model = self.service.loaded_model();
        let Some((head, audio, step)) = self.pipeline_mut(id) else {
            return;
        };
        let settings = head.settings.clone();
        let notice = outcome.notice().cloned();
        let text = replacements::apply(outcome.text(&raw), &settings.replacement_rules);
        let model = model.unwrap_or(settings.selected_model);
        let entry = NewEntry {
            id: Uuid::new_v4(),
            timestamp: Timestamp::now(),
            sample_count: audio.len(),
            model_id: model.as_str().to_owned(),
            model_name: None,
            language: settings.language.code().to_owned(),
            transcript: text.clone(),
            raw_transcript: outcome.was_cleaned().then(|| raw.clone()),
            was_post_processed: outcome.was_cleaned(),
            post_processing_error_description: outcome.error().map(str::to_owned),
            app_name: head.target.as_ref().and_then(|t| t.name.clone()),
            app_bundle_id: head.target.as_ref().and_then(|t| t.app_id.clone()),
            audio_byte_count: None,
        };
        let audio = audio.clone();
        *step = Step::Saving { outcome, text };
        if let Some(notice) = notice {
            self.notify(UserMessage::Cleanup {
                notice,
                output_mode: settings.output_mode,
            });
        }
        // history preferences are read at save time (HIS-1)
        let fresh = self.ports.settings.snapshot();
        if !fresh.history_enabled {
            self.saved(id, false);
            return;
        }
        let entry = NewEntry {
            model_name: self.catalog.model(model).map(|info| info.name.to_owned()),
            ..entry
        };
        let history = self.ports.history.clone();
        let audio = fresh.keep_audio.then_some(audio);
        let started = Instant::now();
        if let Some((head, _, _)) = self.pipeline_mut(id) {
            head.timings.history = Some((started, started));
        }
        // a save cannot be interrupted, so it is not tied to the session
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            let saved = match history.save(entry, audio) {
                Ok(()) => true,
                Err(error) => {
                    // never blocks delivery (HIS-2)
                    tracing::error!(%error, "could not save history");
                    false
                }
            };
            Msg::Saved { id, saved }
        }));
    }

    fn saved(&mut self, id: SessionId, saved: bool) {
        if saved {
            self.history_changed();
        }
        let Some((head, _, step)) = self.pipeline_mut(id) else {
            return;
        };
        let Step::Saving { outcome, text, .. } = step else {
            return;
        };
        if let Some((started, _)) = head.timings.history {
            head.timings.history = Some((started, Instant::now()));
        }
        let text = text.trim().to_owned();
        let was_cleaned = outcome.was_cleaned();
        let target = head.target.clone();
        let mode = head.settings.output_mode;
        let retry = head.origin == Origin::Retry;
        if retry {
            // a successful retry drops the kept audio (DICT-23)
            self.failed_audio = None;
        }
        if text.is_empty() {
            self.finish_logged();
            // empty after a successful cleanup is a deliberate "scratch
            // that" (DICT-17)
            if !was_cleaned {
                self.notify(UserMessage::NoSpeechDetected);
            }
            return;
        }
        self.last_transcript = Some(text.clone());
        let delivery = Delivery::new();
        let deliver = pipeline::deliver(
            self.ports.text_output.clone(),
            self.ports.permissions.clone(),
            text,
            target,
            mode,
            delivery.clone(),
        );
        let task = self.spawn(async move {
            let notices = deliver.await;
            Msg::Delivered {
                id: Some(id),
                notices,
            }
        });
        if let Some((_, _, step)) = self.pipeline_mut(id) {
            *step = Step::Delivering {
                _task: task,
                _delivery: Outgoing(delivery),
            };
        }
    }

    fn delivered(&mut self, id: Option<SessionId>, notices: Vec<UserMessage>) {
        if let Some(id) = id {
            let delivering = matches!(self.pipeline_mut(id), Some((_, _, Step::Delivering { .. })));
            if !delivering {
                return;
            }
            self.finish_logged();
        }
        for notice in notices {
            self.notify(notice);
        }
    }

    fn finish_logged(&mut self) {
        if let Phase::Transcribing(t) = &self.phase {
            t.head.timings.log();
        }
        self.end_session();
    }

    fn pipeline_mut(&mut self, id: SessionId) -> Option<(&mut Head, &Arc<[f32]>, &mut Step)> {
        let Phase::Transcribing(t) = &mut self.phase else {
            return None;
        };
        if t.head.id != id {
            return None;
        }
        match &mut t.stage {
            Stage::Pipeline { audio, step } => Some((&mut t.head, audio, step)),
            Stage::Tail { .. } => None,
        }
    }

    // --- cancelling and finishing (DICT-19..DICT-21, DICT-28a) ---

    fn cancel(&mut self) {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {}
            Phase::Starting(starting) | Phase::Aborting(starting) => {
                self.phase = Phase::Aborting(starting);
            }
            Phase::Recording(recording) => {
                self.ports.audio.stop();
                self.go_idle(recording.media_begun.then_some(recording.id));
            }
            Phase::Transcribing(transcribing) => {
                if let Stage::Tail { .. } = transcribing.stage {
                    self.ports.audio.stop();
                }
                // dropping the phase aborts its tasks; inference already on
                // the model keeps running and its result is dropped (ARCH-10)
                self.go_idle(transcribing.head.media_session());
            }
        }
    }

    fn end_session(&mut self) {
        let media_session = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Recording(recording) => recording.media_begun.then_some(recording.id),
            Phase::Transcribing(transcribing) => transcribing.head.media_session(),
            Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => None,
        };
        self.go_idle(media_session);
    }

    fn go_idle(&mut self, media_session: Option<SessionId>) {
        self.phase = Phase::Idle;
        if let Some(id) = media_session {
            self.end_media(id);
        }
        self.overlay.end_session();
        if self.cancel_registered {
            self.cancel_registered = false;
            self.effects.send(Effect::UnregisterCancel);
        }
        if self.preventing_sleep {
            self.preventing_sleep = false;
            self.effects.send(Effect::AllowIdleSleep);
        }
        if let Some(model) = self.reload_when_idle.take() {
            self.reload_after_download(model);
        }
        // a "Download & Use" that finished mid-session applies now (DICT-29)
        if let Some(model) = self.pending.session_idle(&self.downloaded) {
            self.switch_model(model, false);
        }
    }

    fn hold_session_resources(&mut self) {
        if !self.cancel_registered {
            self.cancel_registered = true;
            let modifiers = cancel_hotkey_modifiers(&self.settings.hotkey);
            self.effects.send(Effect::RegisterCancel(modifiers));
        }
        if !self.preventing_sleep {
            self.preventing_sleep = true;
            self.effects.send(Effect::PreventIdleSleep);
        }
    }

    fn begin_media(&self, id: SessionId, settings: &Settings) -> bool {
        match &self.media {
            Some(media) if settings.pause_media => {
                media.begin(id);
                true
            }
            Some(_) | None => false,
        }
    }

    fn end_media(&self, id: SessionId) {
        if let Some(media) = &self.media {
            media.end(id);
        }
    }

    fn shut_down(&mut self, done: oneshot::Sender<()>) {
        // quitting saves the editors' pending changes (REP-12, CLN-25)
        self.flush_replacements();
        let key_save = self.pending_key_save();
        // nothing is delivered and no audio is kept (DICT-28a)
        self.cancel();
        self.redelivery = None;
        if let Phase::Aborting(starting) = std::mem::replace(&mut self.phase, Phase::Idle) {
            self.ports.audio.stop();
            if starting.opening.is_some_and(|opening| opening.media_begun) {
                self.end_media(starting.id);
            }
        }
        self.failed_audio = None;
        self.publish();
        let media = self.media.clone();
        let effects = self.effects.clone();
        tokio::spawn(async move {
            let drained = async {
                if let Some(save) = key_save
                    && let Err(error) = spawn_blocking(save).await
                {
                    tracing::error!(%error, "could not save the API key at quit");
                }
                if let Some(media) = media {
                    media.flush().await;
                }
                effects.flush().await;
            };
            if tokio::time::timeout(SHUTDOWN_DRAIN, drained).await.is_err() {
                tracing::warn!("shell calls did not drain before shutdown");
            }
            let _ = done.send(());
        });
    }

    // --- failed dictations and re-delivery (DICT-24..DICT-28) ---

    fn discard_failed(&mut self) {
        let retrying = match &self.phase {
            Phase::Starting(starting) | Phase::Aborting(starting) => {
                starting.kind == StartKind::Retry
            }
            Phase::Transcribing(t) => t.head.origin == Origin::Retry,
            Phase::Idle | Phase::Recording(_) => false,
        };
        if !retrying {
            self.failed_audio = None;
        }
    }

    fn redeliver(&mut self, text: String) {
        if !matches!(self.phase, Phase::Idle) {
            return;
        }
        self.refresh_settings();
        // the request owns the output from now on, so a newer request, a
        // session, or quitting cancels it even while its check still runs
        // (DICT-26, DICT-27)
        let delivery = Delivery::new();
        self.redelivery = Some(Outgoing(delivery.clone()));
        let check = pipeline::check(
            self.ports.permissions.clone(),
            self.ports.text_output.clone(),
            false,
        );
        tokio::spawn(send_when(self.messages.clone(), async move {
            let report = check.await;
            Msg::RedeliveryChecked {
                text,
                report,
                delivery,
            }
        }));
    }

    fn redelivery_checked(&mut self, text: String, report: CheckReport, delivery: Delivery) {
        // replaced during the check, even by a session that already ended
        if delivery.is_cancelled() || !matches!(self.phase, Phase::Idle) {
            return;
        }
        let settings = self.settings.clone();
        if !self.output_ready(&settings, report.accessibility) {
            return;
        }
        let deliver = pipeline::deliver(
            self.ports.text_output.clone(),
            self.ports.permissions.clone(),
            text,
            report.target,
            settings.output_mode,
            delivery,
        );
        tokio::spawn(send_when(self.messages.clone(), async move {
            let notices = deliver.await;
            Msg::Delivered { id: None, notices }
        }));
    }

    // --- models (ASR-1, ASR-6..ASR-10, DL-8, DICT-29) ---

    fn ui_ready(&mut self) {
        self.refresh_permissions();
        // the menu's update item follows the updater from the first render
        self.refresh_updater();
        self.deliver_held_hotkey_failure();
        if let Some(warning) = self.ports.settings.reset_invalid_language() {
            // LANG-3: the code now reads `auto` and is stored that way
            self.refresh_settings();
            self.notify(warning);
        }
        // the launch decision needs the installed models (ASR-1, DL-12);
        // removed models' folders go first (DL-15)
        let models = self.ports.models.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            models.remove_obsolete();
            Msg::LaunchChecked(models.downloaded())
        }));
    }

    fn start_load(&mut self, unload_first: bool) {
        let model = self.desired_model();
        self.service.set_desired(model);
        // loading downloads the model first when it is missing (ASR-2)
        if !self.downloaded.contains(&model) && !self.catalog.is_built_in(model) {
            self.download_then_load(model);
            return;
        }
        self.loads.started();
        let service = self.service.clone();
        tokio::spawn(send_when(self.messages.clone(), async move {
            if unload_first {
                service.unload().await;
            }
            Msg::LoadFinished(service.ensure_loaded().await)
        }));
    }

    fn load_finished(&mut self, result: Result<(), LoadError>) {
        self.loads.finished();
        if let Err(LoadError::Failed(error)) = result {
            // built-in engines have no files on disk (ASR-6)
            let on_disk: Vec<ModelId> = self
                .downloaded
                .iter()
                .copied()
                .filter(|model| !self.catalog.is_built_in(*model))
                .collect();
            let message = load_failure_notification(self.desired_model(), &on_disk, &error);
            self.notify(message);
        }
    }

    fn switch_model(&mut self, model: ModelId, explicit: bool) {
        match select_model(
            model,
            self.settings.selected_model.as_str(),
            &self.downloaded,
        ) {
            SelectOutcome::NotDownloaded => {
                tracing::warn!(
                    model = model.as_str(),
                    "cannot select a model that is not downloaded"
                );
            }
            SelectOutcome::AlreadySelected if self.override_model.is_some() => {
                // choosing the selection again ends the temporary override
                // (ASR-8, DL-14)
                self.override_model = None;
                if explicit {
                    self.cancel();
                }
                self.start_load(true);
            }
            SelectOutcome::AlreadySelected => {}
            SelectOutcome::Selected => {
                self.ports.settings.set_selected_model(model);
                self.refresh_settings();
                self.override_model = None;
                self.end_declined();
                // an explicit change cancels a running session (ASR-8)
                if explicit {
                    self.cancel();
                }
                self.start_load(true);
            }
        }
    }

    // ASR-9: clear the temporary override, then load the selection
    fn reload_after_download(&mut self, model: ModelId) {
        if reload_after_download(
            model,
            self.settings.selected_model.as_str(),
            self.load_in_flight(),
        ) {
            self.override_model = None;
            self.loads.announce();
            self.start_load(false);
        }
    }

    fn download_finished(&mut self, model: ModelId) {
        if !self.downloaded.contains(&model) {
            self.downloaded.push(model);
        }
        self.loads.download_ended(model);
        match self.phase {
            // the engine is in use; the reload waits for the session to end,
            // as a "Download & Use" does (DICT-29)
            Phase::Recording(_) | Phase::Transcribing(_) => {
                if model == self.settings.selected_model {
                    self.reload_when_idle = Some(model);
                }
            }
            Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => {
                self.reload_after_download(model);
            }
        }
        let idle = matches!(self.phase, Phase::Idle);
        if let Some(selected) = self.pending.download_finished(model, idle) {
            self.switch_model(selected, false);
        }
    }

    // --- messages (DICT-2, MSG-1) ---

    // routes `message` to its channel; false when the
    // rate limit swallowed it (DICT-2)
    fn notify(&mut self, message: UserMessage) -> bool {
        if let Some(kind) = message.rate_limit()
            && !self.limiter.allow(kind, mono(self.origin))
        {
            return false;
        }
        match message.class() {
            MessageClass::Feedback => self.show_notice(&message),
            MessageClass::Background => self.effects.send(Effect::Notify(message)),
            MessageClass::Critical => self.events.emit(Event::ShowAlert {
                title: message.title(self.ports.privacy_naming).to_owned(),
                message: message.body(&self.ports.tray_term, self.ports.privacy_naming),
                button: ALERT_BUTTON,
            }),
        }
        true
    }

    fn show_notice(&mut self, message: &UserMessage) {
        let now = mono(self.origin);
        self.overlay.show_notice(
            Notice {
                title: message.title(self.ports.privacy_naming).to_owned(),
                message: message.body(&self.ports.tray_term, self.ports.privacy_naming),
            },
            now,
        );
        if let Some(deadline) = self.overlay.notice_deadline() {
            self.notice_timer = Some(self.timer(deadline.saturating_sub(now), Msg::NoticeTick));
        }
    }

    fn notice_tick(&mut self) {
        self.overlay.tick(mono(self.origin));
        let now = mono(self.origin);
        if let Some(deadline) = self.overlay.notice_deadline() {
            self.notice_timer = Some(self.timer(deadline.saturating_sub(now), Msg::NoticeTick));
        }
    }

    // DICT-5
    fn setup_guidance(&mut self) {
        let guidance = setup_guidance(
            self.settings.first_run_pending,
            self.checklist().is_complete(),
            self.declined(),
        );
        match guidance {
            Some(SetupGuidance::Onboarding) => self.open_onboarding(false),
            Some(SetupGuidance::GettingStarted) => {
                self.open_section(MainWindowSection::GettingStarted);
            }
            Some(SetupGuidance::Speech) => self.open_section(MainWindowSection::Speech),
            None => {}
        }
    }

    fn checklist(&self) -> SetupChecklist {
        SetupChecklist::resolve(SetupChecklistInput {
            microphone: self.permissions.microphone,
            accessibility_granted: self.permissions.accessibility,
            accessibility_required: self.accessibility_needed(),
            privacy_naming: self.ports.privacy_naming,
            selected_model_downloaded: self.downloaded.contains(&self.settings.selected_model),
            any_download_running: self.downloads.any_running(),
        })
    }

    // --- state reads ---

    fn refresh_settings(&mut self) {
        self.settings = Arc::new(self.ports.settings.snapshot());
    }

    fn refresh_permissions(&self) {
        let permissions = self.ports.permissions.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::Permissions(PermissionSnapshot {
                microphone: permissions.microphone(),
                accessibility: permissions.accessibility_trusted(),
            })
        }));
    }

    fn refresh_downloaded(&self) {
        let models = self.ports.models.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::Downloaded(models.downloaded())
        }));
    }

    fn refresh_recent(&self) {
        let history = self.ports.history.clone();
        let generation = self.ui.history.generation;
        tokio::spawn(blocking_send(self.messages.clone(), move || Msg::Recent {
            generation,
            recent: history.recent_transcripts(),
        }));
    }

    fn current_id(&self) -> Option<SessionId> {
        match &self.phase {
            Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => None,
            Phase::Recording(recording) => Some(recording.id),
            Phase::Transcribing(t) => Some(t.head.id),
        }
    }

    fn session_phase(&self) -> SessionPhase {
        match &self.phase {
            Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) => SessionPhase::Idle,
            Phase::Recording(recording) => SessionPhase::Recording {
                provisional: recording.provisional,
            },
            Phase::Transcribing(_) => SessionPhase::Transcribing,
        }
    }

    fn accessibility_needed(&self) -> bool {
        accessibility_needed(
            self.settings.output_mode,
            self.settings.hotkey.is_lone_modifier(),
        )
    }

    fn missing_permission(&self) -> Option<MissingPermission> {
        missing_permission(
            self.permissions.microphone,
            self.permissions.accessibility,
            self.accessibility_needed(),
        )
    }

    // --- views (ARCH-5: complete view models, emitted when they change) ---

    fn publish(&mut self) {
        let phase = self.session_phase();
        let selected = self.settings.selected_model.as_str();
        let non_dictation = non_dictation_ui::resolve(
            &self.model_state,
            self.downloads.current(Some(selected)),
            self.load_in_flight(),
            self.loads.announced(),
        );
        let status = AppStatus::resolve(phase, &non_dictation);
        let missing = self.missing_permission();
        let tray = tray_view(
            status,
            missing,
            self.settings.first_run_pending,
            self.ports.privacy_naming,
        );
        let menu = self.menu(status, missing, phase);
        let clock = match &self.phase {
            Phase::Recording(recording) => {
                let clock = RecordingClock::at(
                    recording.started_at.saturating_duration_since(self.origin),
                    mono(self.origin),
                );
                Some(ClockDisplay {
                    text: clock.text,
                    warning: clock.is_warning,
                })
            }
            Phase::Idle | Phase::Starting(_) | Phase::Aborting(_) | Phase::Transcribing(_) => None,
        };
        let overlay = self.overlay.view(clock.as_ref());
        let download = self.download_overlay.apply(
            &non_dictation,
            phase != SessionPhase::Idle,
            self.overlay.has_notice(),
        );
        if self.published.tray.as_ref() != Some(&tray) {
            self.events.emit(Event::TrayChanged(tray.clone()));
            self.published.tray = Some(tray);
        }
        if self.published.menu.as_ref() != Some(&menu) {
            self.events.emit(Event::MenuChanged(Box::new(menu.clone())));
            self.published.menu = Some(menu);
        }
        if self.published.overlay.as_ref() != Some(&overlay) {
            self.events.emit(Event::OverlayChanged(overlay.clone()));
            self.published.overlay = Some(overlay);
        }
        let hotkey = self.hotkey_view();
        if self.published.hotkey.as_ref() != Some(&hotkey) {
            self.events.emit(Event::HotkeyChanged(hotkey.clone()));
            self.published.hotkey = Some(hotkey);
        }
        if self.published.download.as_ref() != Some(&download) {
            self.events
                .emit(Event::DownloadOverlayChanged(download.clone()));
            self.published.download = Some(download);
        }
        self.publish_windows();
    }

    fn menu(
        &self,
        status: AppStatus,
        missing: Option<MissingPermission>,
        phase: SessionPhase,
    ) -> StatusMenu {
        let settings = &self.settings;
        let languages = language_menu::menu_items(&settings.recent_languages, settings.language)
            .into_iter()
            .map(|item| LanguageMenuItem {
                code: item.language.code().to_owned(),
                name: item.title.to_owned(),
                checked: item.is_selected,
            })
            .collect();
        let recent_transcripts = self
            .recent
            .iter()
            .map(|recent| RecentTranscriptItem {
                title: menu_preview(&recent.text),
                // the shell writes the localized date and time (HIS-28)
                time: recent.timestamp,
                app_name: recent.subtitle_app().map(str::to_owned),
                text: recent.text.clone(),
            })
            .collect();
        let hotkey = settings.hotkey.display();
        StatusMenu::resolve(StatusMenuInput {
            status,
            hotkey_display: &hotkey,
            first_run_pending: settings.first_run_pending,
            missing_permission: missing,
            privacy_naming: self.ports.privacy_naming,
            phase,
            output_mode: settings.output_mode,
            last_transcript_preview: self.last_transcript.as_deref().map(menu_preview),
            history_enabled: settings.history_enabled,
            recent_transcripts,
            has_failed_dictation: self.failed_audio.is_some(),
            can_start_session: matches!(self.phase, Phase::Idle),
            model_state: &self.model_state,
            load_in_flight: self.load_in_flight(),
            selected_model_on_disk: self.downloaded.contains(&settings.selected_model)
                && !self.catalog.is_built_in(settings.selected_model),
            current_language_name: language_menu::subtitle(settings.language).to_owned(),
            languages,
            update: update_control(
                &self.ui.general.updater.status,
                self.dictating() || self.restarting,
            ),
        })
    }

    // --- task helpers ---

    fn next_session(&mut self) -> SessionId {
        self.next_id += 1;
        SessionId(self.next_id)
    }

    fn spawn(&self, work: impl Future<Output = Msg> + Send + 'static) -> Task {
        Task(tokio::spawn(send_when(self.messages.clone(), work)).abort_handle())
    }

    fn timer(&self, after: Duration, msg: Msg) -> Task {
        Task(tokio::spawn(pipeline::after(after, msg, self.messages.clone())).abort_handle())
    }

    // ticks on whole seconds of the recording so the clock text changes on
    // time (DICT-9)
    fn clock_ticker(&self, started: Instant) -> Task {
        let messages = self.messages.clone();
        Task(
            tokio::spawn(async move {
                let second = Duration::from_secs(1);
                let mut ticks = tokio::time::interval_at(started + second, second);
                loop {
                    ticks.tick().await;
                    if messages.send(Msg::ClockTick).is_err() {
                        return;
                    }
                }
            })
            .abort_handle(),
        )
    }

    fn spawn_preview(&self, id: SessionId, settings: &Settings) -> PreviewHandle {
        PreviewHandle::spawn(PreviewContext {
            id,
            service: self.service.clone(),
            audio: self.ports.audio.clone(),
            // read once per session (ASR-18)
            rules: settings.replacement_rules.clone(),
            language: settings.language.engine_language(),
            origin: self.origin,
            messages: self.messages.clone(),
        })
    }
}

async fn send_when(messages: mpsc::UnboundedSender<Msg>, work: impl Future<Output = Msg>) {
    let msg = work.await;
    let _ = messages.send(msg);
}

async fn blocking_send(
    messages: mpsc::UnboundedSender<Msg>,
    work: impl FnOnce() -> Msg + Send + 'static,
) {
    match spawn_blocking(work).await {
        Ok(msg) => {
            let _ = messages.send(msg);
        }
        Err(error) => tracing::error!(%error, "background call failed"),
    }
}

fn app_id(target: Option<&Target>) -> Option<String> {
    target.and_then(|target| target.app_id.clone())
}
