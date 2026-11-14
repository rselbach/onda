//! In-memory fakes for every port and seam, plus a harness that runs the
//! core on the test's paused-clock runtime.
//!
//! Timing: fakes return at once, and tokio does not auto-advance a paused
//! clock while a blocking call runs, so after `sleep(d)` everything due
//! before `d` has settled. A fake call that is *held* blocks that
//! auto-advance; tests then poll with [`eventually`] and move the clock with
//! `tokio::time::advance`.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use onda_app::ports::{
    CopyOutcome, Cue, Delivery, FieldText, Files, HotkeySource, InsertOutcome, LoginItem,
    MediaControl, Notifier, NowPlaying, Permissions, Power, PrivacyPane, SoundPlayer, SystemSpeech,
    Target, TextOutput, Updater,
};
use onda_app::seams::{
    AudioSource, BoxFuture, CaptureInfo, CleanupTransport, DownloadUpdate, DownloadUpdateKind,
    HistorySink, InputDeviceInfo, ModelStore, SecretSource, SettingChange, Settings,
    SettingsSource,
};
use onda_app::system_speech::SystemSpeechEngine;
use onda_app::views::UpdaterState;
use onda_app::{App, Event, EventSink, HotkeyEvent, Ports};
use onda_asr::{AsrError, LoadProgress, SpeechEngine};
use onda_domain::cleanup::{
    ApiKey, CleanupError, CleanupModel, CleanupPrompt, CleanupProvider, CleanupRequest,
    ModelListing, OnDeviceStatus, default_model,
};
use onda_domain::history::{HistoryEntry, NewEntry, RecentTranscript, plan_retention};
use onda_domain::hotkey::{GlobeKeyAction, HotkeyView, KeyNames, ModifierSet, Shortcut};
use onda_domain::language_menu::recent_languages;
use onda_domain::languages::{Language, LanguageSetting};
use onda_domain::messages::LoginItemRegistryStatus;
use onda_domain::messages::{NotificationAuthorization, UserMessage};
use onda_domain::models::{Catalog, ModelId};
use onda_domain::non_dictation_ui::DownloadOverlayView;
use onda_domain::onboarding::OnboardingFlags;
use onda_domain::overlay::{OverlayContent, OverlayView};
use onda_domain::platform::Platform;
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::status_menu::StatusMenu;
use onda_domain::tray::TrayView;
use onda_domain::types::{ActivationMode, HistoryRetention, MicrophonePermission, OutputMode};
use onda_domain::updates::UpdateStatus;
use uuid::Uuid;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Shell calls in the order they happened, as short strings.
#[derive(Default)]
pub struct Log(Mutex<Vec<String>>);

impl Log {
    pub fn push(&self, entry: impl Into<String>) {
        lock(&self.0).push(entry.into());
    }

    pub fn all(&self) -> Vec<String> {
        lock(&self.0).clone()
    }

    /// Entries starting with `prefix`.
    pub fn with(&self, prefix: &str) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|entry| entry.starts_with(prefix))
            .collect()
    }

    pub fn count(&self, entry: &str) -> usize {
        self.all().iter().filter(|e| *e == entry).count()
    }
}

/// A point where a fake blocks until the test releases it.
#[derive(Default)]
pub struct Hold {
    started: AtomicBool,
    released: Mutex<bool>,
    signal: Condvar,
}

impl Hold {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn wait(&self) {
        self.started.store(true, Ordering::SeqCst);
        let mut released = lock(&self.released);
        while !*released {
            released = self
                .signal
                .wait(released)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    pub fn release(&self) {
        *lock(&self.released) = true;
        self.signal.notify_all();
    }
}

// blocks on the hold in `slot`, if any, taking it
fn wait_at(slot: &Mutex<Option<Arc<Hold>>>) {
    let hold = lock(slot).take();
    if let Some(hold) = hold {
        hold.wait();
    }
}

/// Polls `cond` in real time while letting the runtime run, for steps that
/// wait on a held fake.
pub async fn eventually(what: &str, cond: impl Fn() -> bool) {
    for _ in 0..3_000 {
        if cond() {
            return;
        }
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {what}");
}

/// Advances the paused clock by `ms`, letting everything due run.
pub async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// `count` samples of a 220 Hz sine at amplitude 0.05: passes the speech
/// gate.
pub fn loud(count: usize) -> Vec<f32> {
    (0..count)
        .map(|i| 0.05 * (TAU * 220.0 * i as f32 / 16_000.0).sin())
        .collect()
}

/// `count` near-silent samples: fails the speech gate with a warning.
pub fn quiet(count: usize) -> Vec<f32> {
    vec![0.001; count]
}

// --- text output ---

pub struct FakeText {
    pub log: Arc<Log>,
    pub typed: Mutex<Vec<(String, Option<u32>)>>,
    pub copied: Mutex<Vec<String>>,
    pub focused: Mutex<Option<Target>>,
    pub field: Mutex<Option<FieldText>>,
    pub insert_outcome: Mutex<InsertOutcome>,
    pub copy_ok: AtomicBool,
    /// Holds the next field read (OUT-2).
    pub hold_field: Mutex<Option<Arc<Hold>>>,
    /// Holds the next insert or copy before it commits, like a consent
    /// prompt or a lock another output holds.
    pub hold_commit: Mutex<Option<Arc<Hold>>>,
    /// Holds the next insert after it commits: the output is underway.
    pub hold_insert: Mutex<Option<Arc<Hold>>>,
    /// Holds the next focused-target read, which every start and
    /// re-delivery check makes.
    pub hold_focus: Mutex<Option<Arc<Hold>>>,
}

impl TextOutput for FakeText {
    fn focused_target(&self) -> Option<Target> {
        let hold = lock(&self.hold_focus).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        lock(&self.focused).clone()
    }

    fn field_text(&self, _: Option<&Target>) -> Option<FieldText> {
        wait_at(&self.hold_field);
        lock(&self.field).clone()
    }

    fn insert(&self, text: &str, target: Option<&Target>, delivery: &Delivery) -> InsertOutcome {
        self.log.push("text:insert");
        wait_at(&self.hold_commit);
        if !delivery.commit() {
            self.log.push("text:cancelled");
            return InsertOutcome::Cancelled;
        }
        wait_at(&self.hold_insert);
        lock(&self.typed).push((text.to_owned(), target.map(|t| t.process_id)));
        *lock(&self.insert_outcome)
    }

    fn copy(&self, text: &str, delivery: &Delivery) -> CopyOutcome {
        self.log.push("text:copy");
        wait_at(&self.hold_commit);
        if !delivery.commit() {
            self.log.push("text:cancelled");
            return CopyOutcome::Cancelled;
        }
        lock(&self.copied).push(text.to_owned());
        match self.copy_ok.load(Ordering::SeqCst) {
            true => CopyOutcome::Copied,
            false => CopyOutcome::Failed,
        }
    }
}

// --- permissions ---

pub struct FakePermissions {
    pub log: Arc<Log>,
    pub microphone: Mutex<MicrophonePermission>,
    pub grant_on_request: AtomicBool,
    pub accessibility: AtomicBool,
    pub trust_reads: AtomicU64,
    pub hold_request: Mutex<Option<Arc<Hold>>>,
}

impl Permissions for FakePermissions {
    fn microphone(&self) -> MicrophonePermission {
        *lock(&self.microphone)
    }

    fn request_microphone(&self) -> bool {
        self.log.push("permissions:request-microphone");
        let hold = lock(&self.hold_request).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        let granted = self.grant_on_request.load(Ordering::SeqCst);
        *lock(&self.microphone) = if granted {
            MicrophonePermission::Granted
        } else {
            MicrophonePermission::Denied
        };
        granted
    }

    fn accessibility_trusted(&self) -> bool {
        self.trust_reads.fetch_add(1, Ordering::SeqCst);
        self.accessibility.load(Ordering::SeqCst)
    }

    fn prompt_accessibility(&self) {
        self.log.push("permissions:prompt-accessibility");
    }

    fn open_settings(&self, pane: PrivacyPane) {
        self.log.push(format!("permissions:open-settings {pane:?}"));
    }
}

// --- media, sounds, power, hotkeys, notifications ---

pub struct FakeMedia {
    pub log: Arc<Log>,
    pub playing: Mutex<VecDeque<bool>>,
    pub playing_default: AtomicBool,
    /// Another app plays where pause cannot reach it.
    pub audible: AtomicBool,
}

impl MediaControl for FakeMedia {
    fn now_playing(&self) -> NowPlaying {
        let playing = lock(&self.playing)
            .pop_front()
            .unwrap_or(self.playing_default.load(Ordering::SeqCst));
        NowPlaying::Track {
            is_playing: Some(playing),
            playback_rate: None,
        }
    }

    fn pause(&self) -> bool {
        self.log.push("media:pause");
        // pausing works: the next query says not playing
        self.playing_default.store(false, Ordering::SeqCst);
        true
    }

    fn play(&self) -> bool {
        self.log.push("media:play");
        true
    }

    fn play_pause_key(&self) -> bool {
        self.log.push("media:key");
        true
    }

    fn mute_audible_output(&self) -> Option<u32> {
        if !self.audible.load(Ordering::SeqCst) {
            return None;
        }
        self.log.push("media:mute");
        Some(7)
    }

    fn unmute_output(&self, _device: u32) -> bool {
        self.log.push("media:unmute");
        true
    }
}

pub struct FakeShell {
    pub log: Arc<Log>,
    /// Results of the next registrations, then success.
    pub register_results: Mutex<VecDeque<Result<(), i32>>>,
    /// The modifier sets of each Escape registration.
    pub cancel_modifiers: Mutex<Vec<Vec<ModifierSet>>>,
    pub globe: Mutex<GlobeKeyAction>,
}

impl FakeShell {
    /// The next `count` registrations fail with `status`.
    pub fn fail_registrations(&self, count: usize, status: i32) {
        lock(&self.register_results).extend(std::iter::repeat_n(Err(status), count));
    }
}

impl SoundPlayer for FakeShell {
    fn play(&self, cue: Cue) {
        self.log.push(match cue {
            Cue::Start => "sound:start",
            Cue::Stop => "sound:stop",
        });
    }
}

impl Power for FakeShell {
    fn begin_prevent_idle_sleep(&self, reason: &str) {
        self.log.push(format!("power:begin {reason}"));
    }

    fn end_prevent_idle_sleep(&self) {
        self.log.push("power:end");
    }
}

impl HotkeySource for FakeShell {
    fn register(&self, shortcut: &Shortcut) -> Result<(), i32> {
        self.log.push(format!(
            "hotkey:register {}",
            shortcut.display(&KeyNames::MACOS)
        ));
        lock(&self.register_results).pop_front().unwrap_or(Ok(()))
    }

    fn unregister(&self) {
        self.log.push("hotkey:unregister");
    }

    fn reenable_stream(&self) {
        self.log.push("hotkey:reenable");
    }

    fn register_cancel(&self, modifiers: &[ModifierSet]) -> Result<(), String> {
        self.log.push("escape:register");
        lock(&self.cancel_modifiers).push(modifiers.to_vec());
        Ok(())
    }

    fn globe_key_action(&self) -> GlobeKeyAction {
        self.log.push("hotkey:globe");
        *lock(&self.globe)
    }

    fn unregister_cancel(&self) {
        self.log.push("escape:unregister");
    }

    fn open_keyboard_settings(&self) {
        self.log.push("hotkey:keyboard-settings");
    }
}

/// The login item, updater, and file manager.
pub struct FakeSystem {
    pub log: Arc<Log>,
    pub login: Mutex<Option<LoginItemRegistryStatus>>,
    pub login_error: Mutex<Option<String>>,
    pub updater: Mutex<UpdaterState>,
    /// When set, `restart_to_update` blocks until a test sends its answer;
    /// otherwise it answers `true` at once.
    pub restart_answer: Mutex<Option<std::sync::mpsc::Receiver<bool>>>,
    pub files_error: Mutex<Option<String>>,
}

impl LoginItem for FakeSystem {
    fn status(&self) -> Option<LoginItemRegistryStatus> {
        *lock(&self.login)
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.log.push(format!("login:set {enabled}"));
        if let Some(error) = lock(&self.login_error).clone() {
            return Err(error);
        }
        *lock(&self.login) = Some(match enabled {
            true => LoginItemRegistryStatus::Enabled,
            false => LoginItemRegistryStatus::NotRegistered,
        });
        Ok(())
    }

    fn open_settings(&self) {
        self.log.push("login:open-settings");
    }
}

impl Updater for FakeSystem {
    fn state(&self) -> UpdaterState {
        lock(&self.updater).clone()
    }

    fn check_now(&self) {
        self.log.push("updater:check");
    }

    fn set_automatic_checks(&self, enabled: bool) {
        self.log.push(format!("updater:automatic {enabled}"));
        lock(&self.updater).automatic = enabled;
    }

    fn restart_to_update(&self) -> bool {
        self.log.push("updater:restart");
        let answer = lock(&self.restart_answer).take();
        answer.is_none_or(|answer| answer.recv().unwrap_or(false))
    }

    fn open_download(&self) {
        self.log.push("updater:download");
    }
}

impl Files for FakeSystem {
    fn reveal(&self, path: &Path) -> Result<(), String> {
        self.log.push(format!("files:reveal {}", path.display()));
        lock(&self.files_error).clone().map_or(Ok(()), Err)
    }

    fn open(&self, path: &Path) -> Result<(), String> {
        self.log.push(format!("files:open {}", path.display()));
        lock(&self.files_error).clone().map_or(Ok(()), Err)
    }
}

pub struct FakeNotifier {
    pub log: Arc<Log>,
    pub authorization: Mutex<NotificationAuthorization>,
    pub post_fails: AtomicBool,
    /// Every posted body, in order.
    pub bodies: Mutex<Vec<String>>,
}

impl Notifier for FakeNotifier {
    fn authorization(&self) -> NotificationAuthorization {
        *lock(&self.authorization)
    }

    fn post(&self, title: &str, body: &str) -> Result<(), String> {
        if self.post_fails.load(Ordering::SeqCst) {
            return Err("Greendale notification center is closed".to_owned());
        }
        self.log.push(format!("notification:{title}"));
        lock(&self.bodies).push(body.to_owned());
        Ok(())
    }

    fn request_authorization(&self) {
        self.log.push("notifications:request");
    }
}

// --- audio ---

#[derive(Default)]
pub struct FakeAudio {
    pub devices: Mutex<Vec<InputDeviceInfo>>,
    pub selected: Mutex<Vec<String>>,
    buffer: Mutex<Vec<f32>>,
    on_start: Mutex<Vec<f32>>,
    open: AtomicBool,
    next_capture: AtomicU64,
    pub start_error: Mutex<Option<UserMessage>>,
    pub starts: AtomicU64,
    pub released_at: Mutex<Option<usize>>,
    hold_start: Mutex<Option<Arc<Hold>>>,
}

impl FakeAudio {
    /// Blocks the next `start` until the hold is released.
    pub fn hold_next_start(&self) -> Arc<Hold> {
        let hold = Hold::new();
        *lock(&self.hold_start) = Some(hold.clone());
        hold
    }

    /// Samples every new capture starts with.
    pub fn starts_with(&self, samples: Vec<f32>) {
        *lock(&self.on_start) = samples;
    }

    /// Appends samples to the open capture.
    pub fn push(&self, samples: Vec<f32>) {
        lock(&self.buffer).extend(samples);
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    pub fn capture_id(&self) -> u64 {
        self.next_capture.load(Ordering::SeqCst)
    }
}

impl AudioSource for FakeAudio {
    fn start(&self) -> Result<CaptureInfo, UserMessage> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        wait_at(&self.hold_start);
        if let Some(error) = lock(&self.start_error).clone() {
            return Err(error);
        }
        *lock(&self.buffer) = lock(&self.on_start).clone();
        self.open.store(true, Ordering::SeqCst);
        let capture_id = self.next_capture.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(CaptureInfo {
            capture_id,
            device_name: "Greendale USB Microphone".to_owned(),
        })
    }

    fn mark_release(&self) -> usize {
        let count = lock(&self.buffer).len();
        *lock(&self.released_at) = Some(count);
        count
    }

    fn stop(&self) -> Vec<f32> {
        self.open.store(false, Ordering::SeqCst);
        std::mem::take(&mut *lock(&self.buffer))
    }

    fn samples(&self) -> Vec<f32> {
        lock(&self.buffer).clone()
    }

    fn sample_count(&self) -> usize {
        lock(&self.buffer).len()
    }

    fn input_devices(&self) -> Vec<InputDeviceInfo> {
        lock(&self.devices).clone()
    }

    fn select_input(&self, uid: &str) {
        lock(&self.selected).push(uid.to_owned());
    }
}

// --- speech engine and models ---

#[derive(Debug, Clone, PartialEq)]
pub enum EngineCall {
    Load(ModelId),
    Unload(ModelId),
    Reset,
    Preview(usize),
    Final(usize),
}

/// What every fake engine does, shared across loads.
pub struct Script {
    pub calls: Mutex<Vec<EngineCall>>,
    pub finals: Mutex<VecDeque<Result<String, AsrError>>>,
    pub final_default: Mutex<String>,
    pub preview_text: Mutex<String>,
    pub supports_preview: AtomicBool,
    /// What every engine reports as its shortest audio; Parakeet's 0.3 s
    /// unless a test changes it.
    pub min_samples: AtomicUsize,
    pub hold_final: Mutex<Option<Arc<Hold>>>,
    pub hold_preview: Mutex<Option<Arc<Hold>>>,
    pub hold_load: Mutex<Option<Arc<Hold>>>,
    pub fail_load: AtomicBool,
    /// Every preview decode fails.
    pub fail_preview: AtomicBool,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            calls: Mutex::default(),
            finals: Mutex::default(),
            final_default: Mutex::new("hello world".to_owned()),
            preview_text: Mutex::default(),
            supports_preview: AtomicBool::new(false),
            min_samples: AtomicUsize::new(4_800),
            hold_final: Mutex::default(),
            hold_preview: Mutex::default(),
            hold_load: Mutex::default(),
            fail_load: AtomicBool::new(false),
            fail_preview: AtomicBool::new(false),
        }
    }
}

impl Script {
    pub fn calls(&self) -> Vec<EngineCall> {
        lock(&self.calls).clone()
    }

    pub fn finals_run(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| matches!(c, EngineCall::Final(_)))
            .count()
    }

    /// Final decodes return these in order, then the default.
    pub fn finals(&self, results: Vec<Result<String, AsrError>>) {
        *lock(&self.finals) = results.into();
    }

    pub fn hold_next_final(&self) -> Arc<Hold> {
        let hold = Hold::new();
        *lock(&self.hold_final) = Some(hold.clone());
        hold
    }

    pub fn hold_next_preview(&self) -> Arc<Hold> {
        let hold = Hold::new();
        *lock(&self.hold_preview) = Some(hold.clone());
        hold
    }
}

pub fn asr_failure() -> AsrError {
    AsrError::TranscriptionFailed {
        detail: "the study room flooded".to_owned(),
    }
}

struct FakeEngine {
    model: ModelId,
    script: Arc<Script>,
}

impl SpeechEngine for FakeEngine {
    fn load(&mut self, _: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        lock(&self.script.calls).push(EngineCall::Load(self.model));
        let hold = lock(&self.script.hold_load).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        if self.script.fail_load.load(Ordering::SeqCst) {
            return Err(AsrError::LoadFailed {
                detail: "Greendale weights missing".to_owned(),
            });
        }
        Ok(())
    }

    fn unload(&mut self) {
        lock(&self.script.calls).push(EngineCall::Unload(self.model));
    }

    fn supports_preview(&self) -> bool {
        self.script.supports_preview.load(Ordering::SeqCst)
    }

    fn min_samples(&self) -> usize {
        self.script.min_samples.load(Ordering::SeqCst)
    }

    fn transcribe(&mut self, samples: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        lock(&self.script.calls).push(EngineCall::Final(samples.len()));
        let hold = lock(&self.script.hold_final).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        let scripted = lock(&self.script.finals).pop_front();
        scripted.unwrap_or_else(|| Ok(lock(&self.script.final_default).clone()))
    }

    fn preview(&mut self, samples: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        lock(&self.script.calls).push(EngineCall::Preview(samples.len()));
        let hold = lock(&self.script.hold_preview).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        if self.script.fail_preview.load(Ordering::SeqCst) {
            return Err(AsrError::TranscriptionFailed {
                detail: "the study room projector caught fire".to_owned(),
            });
        }
        Ok(lock(&self.script.preview_text).clone())
    }

    fn reset_preview(&mut self) {
        lock(&self.script.calls).push(EngineCall::Reset);
    }
}

/// The shell's system engine and the settings its engine reads.
pub type SystemEngineParts = (Arc<dyn SystemSpeech>, Arc<dyn SettingsSource>);

pub struct FakeModels {
    pub log: Arc<Log>,
    pub script: Arc<Script>,
    pub downloaded: Mutex<Vec<ModelId>>,
    pub delete_error: Mutex<Option<String>>,
    pub hold_delete: Mutex<Option<Arc<Hold>>>,
    /// Serves `system-speech` with the real adapter when set.
    pub system_speech: Mutex<Option<SystemEngineParts>>,
}

impl ModelStore for FakeModels {
    fn downloaded(&self) -> Vec<ModelId> {
        let mut downloaded = lock(&self.downloaded).clone();
        // built-in engines always count as downloaded (DL-10)
        if lock(&self.system_speech).is_some() && !downloaded.contains(&ModelId::SystemSpeech) {
            downloaded.push(ModelId::SystemSpeech);
        }
        downloaded
    }

    fn engine(&self, model: ModelId) -> Option<Box<dyn SpeechEngine>> {
        if model == ModelId::SystemSpeech
            && let Some((speech, settings)) = lock(&self.system_speech).clone()
        {
            return Some(Box::new(SystemSpeechEngine::new(speech, settings)));
        }
        Some(Box::new(FakeEngine {
            model,
            script: self.script.clone(),
        }))
    }

    fn start_download(&self, model: ModelId) {
        self.log.push(format!("download:{model}"));
    }

    fn cancel_download(&self, model: ModelId) {
        self.log.push(format!("download-cancel:{model}"));
    }

    fn delete(&self, model: ModelId) -> Result<(), String> {
        self.log.push(format!("delete:{model}"));
        wait_at(&self.hold_delete);
        if let Some(error) = lock(&self.delete_error).clone() {
            return Err(error);
        }
        lock(&self.downloaded).retain(|m| *m != model);
        Ok(())
    }

    fn redownload(&self, model: ModelId) {
        self.log.push(format!("redownload:{model}"));
    }

    fn remove_obsolete(&self) {
        self.log.push("models:remove-obsolete");
    }
}

/// A download update for `model`.
pub fn download(model: ModelId, kind: DownloadUpdateKind) -> DownloadUpdate {
    DownloadUpdate { model, kind }
}

// --- history, cleanup, secrets, settings ---

#[derive(Default)]
pub struct FakeHistory {
    pub entries: Mutex<Vec<(NewEntry, Option<usize>)>>,
    /// What the History page lists, newest first.
    pub stored: Mutex<Vec<HistoryEntry>>,
    pub ops: Mutex<Vec<String>>,
    pub op_error: Mutex<Option<String>>,
    pub hold_save: Mutex<Option<Arc<Hold>>>,
    /// Holds the next page read after it took its snapshot.
    pub hold_entries: Mutex<Option<Arc<Hold>>>,
    /// Holds the next menu read after it took its snapshot.
    pub hold_recent: Mutex<Option<Arc<Hold>>>,
    /// What page reads fail with.
    pub entries_error: Mutex<Option<String>>,
    pub fail: AtomicBool,
}

impl FakeHistory {
    pub fn hold_next_entries(&self) -> Arc<Hold> {
        let hold = Hold::new();
        *lock(&self.hold_entries) = Some(hold.clone());
        hold
    }

    pub fn hold_next_recent(&self) -> Arc<Hold> {
        let hold = Hold::new();
        *lock(&self.hold_recent) = Some(hold.clone());
        hold
    }

    pub fn entries(&self) -> Vec<NewEntry> {
        lock(&self.entries).iter().map(|(e, _)| e.clone()).collect()
    }

    pub fn audio_lengths(&self) -> Vec<Option<usize>> {
        lock(&self.entries).iter().map(|(_, a)| *a).collect()
    }
}

impl HistorySink for FakeHistory {
    fn save(&self, entry: NewEntry, audio: Option<Arc<[f32]>>) -> Result<(), String> {
        lock(&self.entries).push((entry, audio.map(|a| a.len())));
        let hold = lock(&self.hold_save).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err("disk full at Greendale".to_owned());
        }
        Ok(())
    }

    fn recent_transcripts(&self) -> Vec<RecentTranscript> {
        let recent = onda_domain::history::recent_transcripts(&lock(&self.stored), true);
        let hold = lock(&self.hold_recent).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        recent
    }

    fn entries(&self) -> Result<Vec<HistoryEntry>, String> {
        let entries = match lock(&self.entries_error).clone() {
            Some(error) => Err(error),
            None => Ok(lock(&self.stored).clone()),
        };
        let hold = lock(&self.hold_entries).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        entries
    }

    fn delete(&self, id: Uuid) -> Result<(), String> {
        lock(&self.ops).push(format!("delete {id}"));
        if let Some(error) = lock(&self.op_error).clone() {
            return Err(error);
        }
        lock(&self.stored).retain(|entry| entry.id != id);
        Ok(())
    }

    fn delete_all(&self) -> Result<(), String> {
        lock(&self.ops).push("delete all".to_owned());
        if let Some(error) = lock(&self.op_error).clone() {
            return Err(error);
        }
        lock(&self.stored).clear();
        Ok(())
    }

    fn apply_retention(&self, retention: HistoryRetention, now: Timestamp) -> Result<bool, String> {
        lock(&self.ops).push(format!("retention {retention:?}"));
        let plan = plan_retention(&lock(&self.stored), now, retention, None);
        let changed = !plan.is_noop();
        *lock(&self.stored) = plan.kept;
        Ok(changed)
    }

    fn audio_path(&self, id: Uuid) -> Result<PathBuf, String> {
        let stored = lock(&self.stored);
        match stored
            .iter()
            .find(|entry| entry.id == id && entry.has_audio())
        {
            Some(entry) => Ok(PathBuf::from(format!(
                "/History/{}/audio.wav",
                entry.directory_name.as_str()
            ))),
            None => Err("The recording for this dictation is missing.".to_owned()),
        }
    }
}

#[derive(Default)]
pub struct FakeCleanup {
    pub results: Mutex<VecDeque<Result<String, CleanupError>>>,
    pub requests: Mutex<Vec<serde_json::Value>>,
    pub hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    pub started: AtomicU64,
    /// What `list_models` answers; an empty list by default.
    pub listed: Mutex<Option<Result<Vec<CleanupModel>, CleanupError>>>,
    pub list_calls: AtomicU64,
    /// The provider and key of every listing, in order.
    pub list_requests: Mutex<Vec<(CleanupProvider, Option<String>)>>,
    /// What `on_device_status` reports; not eligible by default.
    pub on_device: Mutex<Option<OnDeviceStatus>>,
}

impl FakeCleanup {
    pub fn respond(&self, results: Vec<Result<String, CleanupError>>) {
        *lock(&self.results) = results.into();
    }

    /// The user message of each request.
    pub fn user_messages(&self) -> Vec<String> {
        lock(&self.requests)
            .iter()
            .map(|body| {
                body["messages"][1]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned()
            })
            .collect()
    }

    /// The system instructions of each request, as its provider carries
    /// them.
    pub fn instructions(&self) -> Vec<String> {
        lock(&self.requests)
            .iter()
            .map(|body| {
                body["messages"][0]["content"]
                    .as_str()
                    .or(body["instructions"].as_str())
                    .unwrap_or("")
                    .to_owned()
            })
            .collect()
    }

    pub fn calls(&self) -> usize {
        lock(&self.requests).len()
    }
}

impl CleanupTransport for FakeCleanup {
    fn on_device_status(&self) -> OnDeviceStatus {
        lock(&self.on_device).unwrap_or(OnDeviceStatus::DeviceNotEligible)
    }

    fn send(&self, request: CleanupRequest) -> BoxFuture<'static, Result<String, CleanupError>> {
        let body = serde_json::to_value(&request.body).unwrap_or_default();
        lock(&self.requests).push(body);
        self.started.fetch_add(1, Ordering::SeqCst);
        let result = lock(&self.results)
            .pop_front()
            .unwrap_or(Err(CleanupError::Empty));
        let hold = lock(&self.hold).clone();
        Box::pin(async move {
            if let Some(hold) = hold {
                hold.notified().await;
            }
            result
        })
    }

    fn list_models(
        &self,
        provider: CleanupProvider,
        key: Option<ApiKey>,
    ) -> BoxFuture<'static, Result<Vec<CleanupModel>, CleanupError>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let key = key.map(|key| key.expose().to_owned());
        let missing = provider.model_listing() == ModelListing::WithKey && key.is_none();
        lock(&self.list_requests).push((provider, key));
        let result = match missing {
            true => Err(CleanupError::MissingKey),
            false => lock(&self.listed).clone().unwrap_or(Ok(Vec::new())),
        };
        Box::pin(async move { result })
    }
}

/// OpenRouter's stored key, a hold the next save waits on before writing,
/// and the provider of every read and save, in order.
pub struct FakeSecrets(
    pub Mutex<Result<Option<String>, String>>,
    pub Mutex<Option<Arc<Hold>>>,
    pub Mutex<Vec<(&'static str, CleanupProvider)>>,
);

impl FakeSecrets {
    pub fn stored(&self) -> Option<String> {
        lock(&self.0).clone().ok().flatten()
    }

    /// Makes the next save wait until the returned hold is released.
    pub fn hold_save(&self) -> Arc<Hold> {
        let hold = Hold::new();
        *lock(&self.1) = Some(hold.clone());
        hold
    }
}

impl SecretSource for FakeSecrets {
    fn api_key(&self, provider: CleanupProvider) -> Result<Option<String>, String> {
        lock(&self.2).push(("read", provider));
        lock(&self.0).clone()
    }

    fn save_api_key(&self, provider: CleanupProvider, value: &str) -> Result<(), String> {
        lock(&self.2).push(("save", provider));
        let hold = lock(&self.1).take();
        if let Some(hold) = hold {
            hold.wait();
        }
        let value = value.trim();
        *lock(&self.0) = Ok((!value.is_empty()).then(|| value.to_owned()));
        Ok(())
    }
}

pub struct FakeSettings {
    pub settings: Mutex<Settings>,
    pub log: Arc<Log>,
    /// A stored language code that is not offered (LANG-3).
    pub invalid_language: Mutex<Option<String>>,
    /// The error the next `apply` fails with, saving nothing.
    pub fail_apply: Mutex<Option<String>>,
}

impl SettingsSource for FakeSettings {
    fn snapshot(&self) -> Settings {
        let mut settings = lock(&self.settings).clone();
        settings.recent_languages =
            recent_languages(settings.language, &settings.stored_recent_languages);
        settings.first_run_pending = settings.onboarding.first_run_pending();
        settings
    }

    fn set_selected_model(&self, model: ModelId) {
        self.log.push(format!("settings:selected {model}"));
        let mut settings = lock(&self.settings);
        settings.selected_model = model;
        settings.stored_model = model.as_str().to_owned();
    }

    fn apply(&self, change: SettingChange) -> Result<(), String> {
        if let Some(error) = lock(&self.fail_apply).take() {
            return Err(error);
        }
        self.log.push(format!("settings:{change:?}"));
        let mut s = lock(&self.settings);
        match change {
            SettingChange::SelectedModel(model) => {
                s.selected_model = model;
                s.stored_model = model.as_str().to_owned();
            }
            SettingChange::Language(language) => s.language = language,
            SettingChange::RecentLanguages(languages) => {
                s.stored_recent_languages = onda_domain::language_menu::encode_recent(&languages);
            }
            SettingChange::InputDevice(uid) => s.input_device_uid = uid,
            SettingChange::OutputMode(mode) => s.output_mode = mode,
            SettingChange::ActivationMode(mode) => s.activation_mode = mode,
            SettingChange::PauseMedia(on) => s.pause_media = on,
            SettingChange::Sounds(on) => s.sounds = on,
            SettingChange::HistoryEnabled(on) => s.history_enabled = on,
            SettingChange::HistoryKeepsAudio(on) => s.keep_audio = on,
            SettingChange::HistoryRetention(retention) => s.history_retention = retention,
            SettingChange::ReplacementRules(rules) => s.replacement_rules = rules,
            SettingChange::CleanupEnabled(on) => s.cleanup_enabled = on,
            SettingChange::CleanupProvider(provider) => s.cleanup_provider = provider,
            SettingChange::CleanupModel(_, model) => s.cleanup_model = model,
            SettingChange::CleanupPrompt(prompt) => s.cleanup_prompt = prompt,
            SettingChange::Onboarding(flags) => s.onboarding = flags,
            SettingChange::LastNoticeKey(key) => s.last_notice_key = key,
            SettingChange::DeclinedNoticeKey(key) => s.declined_notice_key = key,
        }
        Ok(())
    }

    fn set_hotkey(&self, shortcut: &Shortcut) -> Result<(), String> {
        self.log.push(format!(
            "settings:hotkey {}",
            shortcut.display(&KeyNames::MACOS)
        ));
        lock(&self.settings).hotkey = shortcut.clone();
        Ok(())
    }

    fn reset_invalid_language(&self) -> Option<UserMessage> {
        let code = lock(&self.invalid_language).take()?;
        self.log.push(format!("settings:reset-language {code}"));
        lock(&self.settings).language = LanguageSetting::Auto;
        Some(UserMessage::InvalidLanguageSetting { code })
    }
}

pub fn default_settings() -> Settings {
    Settings {
        activation_mode: ActivationMode::Hold,
        output_mode: OutputMode::Type,
        language: LanguageSetting::Auto,
        recent_languages: vec![LanguageSetting::Auto],
        selected_model: ModelId::ParakeetTdtV2,
        cleanup_enabled: false,
        cleanup_provider: CleanupProvider::OpenRouter,
        cleanup_model: default_model().clone(),
        cleanup_prompt: CleanupPrompt::default(),
        replacement_rules: Vec::new(),
        history_enabled: true,
        keep_audio: false,
        pause_media: false,
        sounds: true,
        hotkey: Shortcut::OPTION_SPACE,
        first_run_pending: false,
        onboarding: OnboardingFlags {
            onboarding_completed: true,
            did_show_first_run_setup: true,
        },
        input_device_uid: String::new(),
        history_retention: HistoryRetention::Month,
        stored_model: ModelId::ParakeetTdtV2.as_str().to_owned(),
        stored_recent_languages: String::new(),
        last_notice_key: String::new(),
        declined_notice_key: String::new(),
        os_language: Some("en".to_owned()),
    }
}

// --- events ---

#[derive(Default)]
pub struct Recorder(Mutex<Vec<Event>>);

impl EventSink for Recorder {
    fn emit(&self, event: Event) {
        lock(&self.0).push(event);
    }
}

// --- harness ---

pub struct Harness {
    pub app: App,
    pub log: Arc<Log>,
    pub shell: Arc<FakeShell>,
    pub text: Arc<FakeText>,
    pub permissions: Arc<FakePermissions>,
    pub media: Arc<FakeMedia>,
    pub notifier: Arc<FakeNotifier>,
    pub audio: Arc<FakeAudio>,
    pub script: Arc<Script>,
    pub models: Arc<FakeModels>,
    pub history: Arc<FakeHistory>,
    pub cleanup: Arc<FakeCleanup>,
    pub secrets: Arc<FakeSecrets>,
    pub settings: Arc<FakeSettings>,
    pub system: Arc<FakeSystem>,
    pub recorder: Arc<Recorder>,
}

impl Harness {
    /// A ready core: model loaded, permissions granted, Hold mode.
    pub async fn new() -> Self {
        Self::with(|_| {}).await
    }

    /// A ready core with adjusted settings.
    pub async fn with(adjust: impl FnOnce(&mut Settings)) -> Self {
        let harness = Self::unstarted(adjust);
        harness.app.ui_ready();
        sleep(1).await;
        harness
    }

    /// A ready core whose shell provides `speech` as its system engine.
    pub async fn with_system_speech(
        speech: Arc<dyn SystemSpeech>,
        adjust: impl FnOnce(&mut Settings),
    ) -> Self {
        let harness = Self::build(Some(speech), adjust);
        harness.app.ui_ready();
        sleep(1).await;
        harness
    }

    /// A core whose `ui_ready` has not been called.
    pub fn unstarted(adjust: impl FnOnce(&mut Settings)) -> Self {
        Self::build(None, adjust)
    }

    /// A core whose `ui_ready` has not been called, with the system engine
    /// when `system_speech` is set (MOD-4).
    pub fn build(
        system_speech: Option<Arc<dyn SystemSpeech>>,
        adjust: impl FnOnce(&mut Settings),
    ) -> Self {
        Self::build_on(
            system_speech,
            Platform::macos(PrivacyNaming::Accessibility),
            adjust,
        )
    }

    /// A ready core with adjusted settings, running on `platform`.
    pub async fn on_platform(platform: Platform, adjust: impl FnOnce(&mut Settings)) -> Self {
        let harness = Self::build_on(None, platform, adjust);
        harness.app.ui_ready();
        sleep(1).await;
        harness
    }

    /// A core running on `platform` whose `ui_ready` has not been called.
    pub fn build_on(
        system_speech: Option<Arc<dyn SystemSpeech>>,
        platform: Platform,
        adjust: impl FnOnce(&mut Settings),
    ) -> Self {
        let mut settings = default_settings();
        adjust(&mut settings);
        let log = Arc::new(Log::default());
        let shell = Arc::new(FakeShell {
            log: log.clone(),
            register_results: Mutex::default(),
            cancel_modifiers: Mutex::default(),
            globe: Mutex::new(GlobeKeyAction::DoNothing),
        });
        let text = Arc::new(FakeText {
            log: log.clone(),
            typed: Mutex::default(),
            copied: Mutex::default(),
            focused: Mutex::new(Some(target(42, "Greendale Messenger"))),
            field: Mutex::default(),
            insert_outcome: Mutex::new(InsertOutcome::Inserted),
            copy_ok: AtomicBool::new(true),
            hold_field: Mutex::default(),
            hold_commit: Mutex::default(),
            hold_insert: Mutex::default(),
            hold_focus: Mutex::default(),
        });
        let permissions = Arc::new(FakePermissions {
            log: log.clone(),
            microphone: Mutex::new(MicrophonePermission::Granted),
            grant_on_request: AtomicBool::new(true),
            accessibility: AtomicBool::new(true),
            trust_reads: AtomicU64::new(0),
            hold_request: Mutex::default(),
        });
        let media = Arc::new(FakeMedia {
            log: log.clone(),
            playing: Mutex::default(),
            playing_default: AtomicBool::new(false),
            audible: AtomicBool::new(false),
        });
        let notifier = Arc::new(FakeNotifier {
            log: log.clone(),
            authorization: Mutex::new(NotificationAuthorization::Authorized),
            post_fails: AtomicBool::new(false),
            bodies: Mutex::default(),
        });
        let audio = Arc::new(FakeAudio::default());
        audio.starts_with(loud(16_000));
        let script = Arc::new(Script::default());
        let models = Arc::new(FakeModels {
            log: log.clone(),
            script: script.clone(),
            downloaded: Mutex::new(vec![ModelId::ParakeetTdtV2]),
            delete_error: Mutex::default(),
            hold_delete: Mutex::default(),
            system_speech: Mutex::default(),
        });
        let history = Arc::new(FakeHistory::default());
        let cleanup = Arc::new(FakeCleanup::default());
        let secrets = Arc::new(FakeSecrets(
            Mutex::new(Ok(Some("sk-or-greendale".to_owned()))),
            Mutex::default(),
            Mutex::default(),
        ));
        let settings = Arc::new(FakeSettings {
            settings: Mutex::new(settings),
            log: log.clone(),
            invalid_language: Mutex::default(),
            fail_apply: Mutex::default(),
        });
        let system = Arc::new(FakeSystem {
            log: log.clone(),
            login: Mutex::new(Some(LoginItemRegistryStatus::NotRegistered)),
            login_error: Mutex::default(),
            updater: Mutex::new(UpdaterState {
                status: UpdateStatus::Disabled,
                automatic: true,
                last_check: None,
            }),
            restart_answer: Mutex::default(),
            files_error: Mutex::default(),
        });
        let catalog = Catalog::new(system_speech.as_ref().map(|speech| speech.info()));
        if let Some(speech) = system_speech {
            *lock(&models.system_speech) = Some((speech, settings.clone()));
        }
        let recorder = Arc::new(Recorder::default());
        let ports = Ports {
            text_output: text.clone(),
            permissions: permissions.clone(),
            media: Some(media.clone()),
            sounds: shell.clone(),
            power: shell.clone(),
            hotkeys: shell.clone(),
            notifier: notifier.clone(),
            login_item: system.clone(),
            updater: system.clone(),
            files: system.clone(),
            audio: audio.clone(),
            models: models.clone(),
            catalog,
            history: history.clone(),
            cleanup: cleanup.clone(),
            secrets: secrets.clone(),
            settings: settings.clone(),
            platform,
            app_version: "1.2.3".to_owned(),
        };
        let app = App::start_on(&tokio::runtime::Handle::current(), ports, recorder.clone());
        Self {
            app,
            log,
            shell,
            text,
            permissions,
            media,
            notifier,
            audio,
            script,
            models,
            history,
            cleanup,
            secrets,
            settings,
            system,
            recorder,
        }
    }

    pub fn update(&self, adjust: impl FnOnce(&mut Settings)) {
        adjust(&mut lock(&self.settings.settings));
    }

    pub fn press(&self) {
        self.app.on_hotkey_event(HotkeyEvent::ComboDown);
    }

    pub fn release(&self) {
        self.app.on_hotkey_event(HotkeyEvent::ComboUp);
    }

    /// Stops from the menu: no tail, so a held decode starts at once without
    /// the clock having to move.
    pub fn stop_now(&self) {
        self.app.toggle_dictation_from_menu();
    }

    /// Press and release at once (a tap).
    pub fn tap(&self) {
        self.press();
        self.release();
    }

    pub fn typed(&self) -> Vec<String> {
        lock(&self.text.typed)
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }

    pub fn typed_targets(&self) -> Vec<Option<u32>> {
        lock(&self.text.typed).iter().map(|(_, p)| *p).collect()
    }

    pub fn events(&self) -> Vec<Event> {
        self.app.flush_events();
        lock(&self.recorder.0).clone()
    }

    pub fn overlays(&self) -> Vec<OverlayView> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::OverlayChanged(view) => Some(view),
                _ => None,
            })
            .collect()
    }

    pub fn overlay(&self) -> OverlayView {
        self.overlays()
            .pop()
            .unwrap_or_else(|| panic!("no overlay published"))
    }

    pub fn tray(&self) -> TrayView {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::TrayChanged(view) => Some(view),
                _ => None,
            })
            .next_back()
            .unwrap_or_else(|| panic!("no tray published"))
    }

    pub fn menu(&self) -> StatusMenu {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::MenuChanged(menu) => Some(*menu),
                _ => None,
            })
            .next_back()
            .unwrap_or_else(|| panic!("no menu published"))
    }

    pub fn hotkey_view(&self) -> HotkeyView {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::HotkeyChanged(view) => Some(view),
                _ => None,
            })
            .next_back()
            .unwrap_or_else(|| panic!("no hotkey view published"))
    }

    pub fn download_overlay(&self) -> DownloadOverlayView {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::DownloadOverlayChanged(view) => Some(view),
                _ => None,
            })
            .next_back()
            .unwrap_or(DownloadOverlayView::Hidden)
    }

    /// The newest event `pick` accepts.
    pub fn last<T>(&self, pick: impl Fn(Event) -> Option<T>) -> Option<T> {
        self.events().into_iter().filter_map(pick).next_back()
    }

    /// How many events `pick` accepts.
    pub fn count(&self, pick: impl Fn(&Event) -> bool) -> usize {
        self.events().iter().filter(|event| pick(event)).count()
    }

    /// Drops every event recorded so far.
    pub fn clear_events(&self) {
        self.app.flush_events();
        lock(&self.recorder.0).clear();
    }

    /// Whether the menu offers "Start Dictation": no session is running.
    pub fn idle(&self) -> bool {
        self.menu().dictation.title == "Start Dictation"
    }

    /// Titles of overlay notices, in the order they appeared.
    pub fn notices(&self) -> Vec<String> {
        let mut titles: Vec<String> = Vec::new();
        let mut previous = None;
        for view in self.overlays() {
            let current = match view.content {
                OverlayContent::Notice(notice) => Some(notice.title),
                _ => None,
            };
            if let Some(title) = &current
                && previous.as_ref() != Some(title)
            {
                titles.push(title.clone());
            }
            previous = current;
        }
        titles
    }

    /// Titles of posted notifications.
    pub fn notifications(&self) -> Vec<String> {
        self.log
            .with("notification:")
            .into_iter()
            .map(|e| e.trim_start_matches("notification:").to_owned())
            .collect()
    }

    pub fn processing_labels(&self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for view in self.overlays() {
            if let OverlayContent::Processing { label, .. } = view.content
                && labels.last() != Some(&label)
            {
                labels.push(label);
            }
        }
        labels
    }

    /// Distinct preview texts shown while recording.
    pub fn previews(&self) -> Vec<String> {
        let mut texts: Vec<String> = Vec::new();
        for view in self.overlays() {
            if let OverlayContent::Preview { text, .. } = view.content
                && texts.last() != Some(&text)
            {
                texts.push(text);
            }
        }
        texts
    }

    /// Runs one Hold-mode dictation to the end.
    pub async fn dictate(&self) {
        self.press();
        sleep(400).await;
        self.release();
        sleep(1_000).await;
    }
}

pub fn target(process_id: u32, name: &str) -> Target {
    Target {
        process_id,
        name: Some(name.to_owned()),
        app_id: Some(format!(
            "edu.greendale.{}",
            name.to_lowercase().replace(' ', "-")
        )),
    }
}
