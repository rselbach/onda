//! `dictate`: the whole session engine against a real microphone and the
//! real Parakeet engine (M4).
//!
//! The terminal stands in for the shell. Delivered text prints to stdout;
//! overlay, tray, and cue changes print to stderr. The hotkey is simulated:
//!
//! - `--hold-ms N` presses the hotkey, holds it N ms, releases it, waits for
//!   the transcript, and exits. Works in every activation mode.
//! - Otherwise Enter drives the hotkey: in toggle and automatic modes each
//!   Enter is a quick tap (start, then stop); in hold mode the first Enter
//!   presses and the next one releases. `q` then Enter quits.
//!
//! Settings, history, and the OpenRouter client are the real ones, under
//! `--data-dir`. Flags like `--mode` are saved to that settings file. The
//! OpenRouter key comes from `ONDA_OPENROUTER_KEY` when set (kept in memory
//! only), else from the Keychain.

use std::io::BufRead;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use onda_adapters::{
    CloudCleanup, DataPaths, History, Keychain, Microphone, Models, SettingsAdapter,
    forward_audio_events,
};
use onda_app::ports::{
    Cue, FieldText, Files, HotkeySource, InsertOutcome, LoginItem, Notifier, Permissions, Power,
    PrivacyPane, SoundPlayer, Target, TextOutput, Updater,
};
use onda_app::seams::{ModelStore, SecretSource, SettingsSource};
use onda_app::views::UpdaterState;
use onda_app::{App, Event, EventSink, HotkeyEvent, Ports};
use onda_asr::SpeechEngine;
use onda_asr_coreml::ParakeetEngine;
use onda_asr_coreml::manifest::manifest;
use onda_audio::{AudioCapture, DeviceSelection};
use onda_cleanup::CleanupClient;
use onda_domain::cleanup::CleanupProvider;
use onda_domain::hotkey::{GlobeKeyAction, ModifierSet, Shortcut};
use onda_domain::messages::{LoginItemRegistryStatus, NotificationAuthorization};
use onda_domain::models::{Catalog, ModelId};
use onda_domain::overlay::{OverlayContent, OverlayView};
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::types::{ActivationMode, MicrophonePermission};
use onda_history::{HistoryHooks, HistoryStore};
use onda_models::ModelManager;
use onda_settings::{KeychainSecretStore, MemorySecretStore, SecretStore, Settings};

use crate::models::parse_model;

const KEY_VARIABLE: &str = "ONDA_OPENROUTER_KEY";
const READY_LABEL: &str = "Onda, ready to dictate";
const LOAD_TIMEOUT: Duration = Duration::from_secs(180);
const SESSION_TIMEOUT: Duration = Duration::from_secs(120);

/// `dictate` options.
#[derive(clap::Args)]
pub struct Args {
    /// Input device id from `devices`, or "" for the system default (saved
    /// to settings).
    #[arg(long)]
    device: Option<String>,
    /// Select this model (saved to settings).
    #[arg(long, value_parser = parse_model)]
    model: Option<ModelId>,
    /// Activation mode: hold, toggle, or automatic (saved to settings).
    #[arg(long, value_parser = parse_mode)]
    mode: Option<ActivationMode>,
    /// Turn OpenRouter cleanup on or off (saved to settings).
    #[arg(long)]
    cleanup: Option<bool>,
    /// Mark first-run onboarding complete (saved to settings); until then
    /// the core loads no model (ASR-1).
    #[arg(long)]
    complete_onboarding: bool,
    /// Hold the simulated hotkey this long, then exit after one dictation.
    #[arg(long)]
    hold_ms: Option<u64>,
}

fn parse_mode(mode: &str) -> Result<ActivationMode, String> {
    ActivationMode::ALL
        .into_iter()
        .find(|m| m.display_name().eq_ignore_ascii_case(mode))
        .ok_or_else(|| format!("unknown mode {mode}; use hold, toggle, or automatic"))
}

pub fn run(args: &Args, paths: &DataPaths, models_dir: &Path) -> Result<()> {
    let started = Instant::now();
    let settings = Arc::new(Settings::open(paths.settings_file()));
    apply_overrides(args, &settings)?;
    let os_language = os_language();
    let catalog = Catalog::new(None);
    let settings_source = SettingsAdapter::new(settings.clone(), catalog.clone(), os_language);
    let snapshot = settings_source.snapshot();
    if snapshot.first_run_pending {
        bail!(
            "first-run onboarding is pending in {}, so the core loads no model (ASR-1); \
             pass --complete-onboarding to mark it done",
            paths.root().display()
        );
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("onda-core")
        .enable_all()
        .build()?;
    let (manager, _download_events) = ModelManager::new(models_dir, manifest)?;
    let models = Models::new(
        manager,
        catalog.clone(),
        Box::new(parakeet_engine),
        runtime.handle().clone(),
        Arc::new(|update| {
            eprintln!("download of {}: {:?}", update.model, update.kind);
        }),
    );
    if !models.downloaded().contains(&snapshot.selected_model) {
        bail!(
            "{} is not installed in {}; run `onda-cli models download {}`",
            snapshot.selected_model,
            models_dir.display(),
            snapshot.selected_model
        );
    }

    let selection = DeviceSelection::from_setting(&settings.input_device_uid());
    let (capture, audio_events) = AudioCapture::new(selection)?;
    capture.prepare();
    let (tray_tx, trays) = mpsc::channel();
    let console = Arc::new(Console {
        started,
        last: Mutex::new(Shown::default()),
        trays: tray_tx,
    });
    let history = HistoryStore::new(paths.history_dir(), Logged(console.clone()));
    let terminal = Arc::new(Terminal {
        console: console.clone(),
    });
    let ports = Ports {
        text_output: terminal.clone(),
        permissions: terminal.clone(),
        media: None,
        sounds: terminal.clone(),
        power: terminal.clone(),
        hotkeys: terminal.clone(),
        notifier: terminal.clone(),
        login_item: terminal.clone(),
        updater: terminal.clone(),
        files: terminal.clone(),
        audio: Arc::new(Microphone::new(capture)),
        models: Arc::new(models),
        catalog,
        history: Arc::new(History::new(Arc::new(history), settings.clone())),
        cleanup: Arc::new(CloudCleanup::new(CleanupClient::new()?)),
        secrets: secrets(),
        settings: Arc::new(settings_source),
        tray_term: "menu bar icon".into(),
        // the terminal always reports Accessibility as trusted, so no copy
        // names the permission
        privacy_naming: PrivacyNaming::Accessibility,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let app = App::start_on(runtime.handle(), ports, console.clone());
    forward_audio_events(audio_events, app.audio_signals());

    app.ui_ready();
    console.say(&format!(
        "data in {}; loading {}...",
        paths.root().display(),
        snapshot.selected_model
    ));
    wait_for_ready(&trays, LOAD_TIMEOUT).context("loading the model")?;
    let mode = snapshot.activation_mode;
    console.say(&format!("ready ({} mode)", mode.display_name()));

    let result = match args.hold_ms {
        Some(hold_ms) => hold_once(&app, &trays, &console, mode, hold_ms),
        None => interactive(&app, &trays, &console, mode),
    };
    app.shutdown();
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

fn apply_overrides(args: &Args, settings: &Settings) -> Result<()> {
    if let Some(model) = args.model {
        settings.set_selected_model(model)?;
    }
    if let Some(mode) = args.mode {
        settings.set_activation_mode(mode)?;
    }
    if let Some(cleanup) = args.cleanup {
        settings.set_post_processing_enabled(cleanup)?;
    }
    if args.complete_onboarding {
        settings.complete_onboarding()?;
    }
    if let Some(device) = &args.device {
        settings.set_input_device_uid(device)?;
    }
    Ok(())
}

// the OS locale's language code from the POSIX locale variables (LANG-2)
fn os_language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .and_then(|value| value.split(['_', '.', '-']).next().map(str::to_lowercase))
}

fn secrets() -> Arc<dyn SecretSource> {
    let Some(key) = std::env::var(KEY_VARIABLE)
        .ok()
        .filter(|key| !key.trim().is_empty())
    else {
        return Arc::new(Keychain::new(KeychainSecretStore::new()));
    };
    let store = MemorySecretStore::new();
    if let Err(error) = store.save(CleanupProvider::OpenRouter, &key) {
        eprintln!("could not hold {KEY_VARIABLE}: {error}");
    }
    Arc::new(Keychain::new(store))
}

fn parakeet_engine(model: ModelId, dir: &Path) -> Option<Box<dyn SpeechEngine>> {
    let engine = ParakeetEngine::new(model, dir)?;
    Some(Box::new(engine))
}

/// History hooks for the terminal: no backup exclusion outside the app.
struct Logged(Arc<Console>);

impl HistoryHooks for Logged {
    fn history_changed(&self) {
        self.0.say("history saved");
    }

    fn exclude_from_backup(&self, directory: &Path) -> std::io::Result<()> {
        self.0
            .say(&format!("history folder ready: {}", directory.display()));
        Ok(())
    }
}

fn hold_once(
    app: &App,
    trays: &Receiver<String>,
    console: &Console,
    mode: ActivationMode,
    hold_ms: u64,
) -> Result<()> {
    console.say("hotkey down");
    app.on_hotkey_event(HotkeyEvent::ComboDown);
    thread::sleep(Duration::from_millis(hold_ms));
    drain(trays);
    console.say("hotkey up");
    app.on_hotkey_event(HotkeyEvent::ComboUp);
    if mode == ActivationMode::Toggle {
        // a toggle key-up does nothing (HOT-26); tap again to stop
        console.say("hotkey tap");
        app.on_hotkey_event(HotkeyEvent::ComboDown);
        app.on_hotkey_event(HotkeyEvent::ComboUp);
    }
    wait_for_ready(trays, SESSION_TIMEOUT).context("waiting for the transcript")
}

fn interactive(
    app: &App,
    trays: &Receiver<String>,
    console: &Console,
    mode: ActivationMode,
) -> Result<()> {
    match mode {
        ActivationMode::Hold => {
            console.say("Enter presses the hotkey, Enter again releases it; q quits")
        }
        ActivationMode::Toggle | ActivationMode::Automatic => {
            console.say("Enter taps the hotkey (start, then stop); q quits");
        }
    }
    let mut held = false;
    for line in std::io::stdin().lock().lines() {
        if line?.trim() == "q" {
            return Ok(());
        }
        match mode {
            ActivationMode::Hold if held => {
                app.on_hotkey_event(HotkeyEvent::ComboUp);
                held = false;
            }
            ActivationMode::Hold => {
                app.on_hotkey_event(HotkeyEvent::ComboDown);
                held = true;
            }
            ActivationMode::Toggle | ActivationMode::Automatic => {
                app.on_hotkey_event(HotkeyEvent::ComboDown);
                app.on_hotkey_event(HotkeyEvent::ComboUp);
            }
        }
    }
    drain(trays);
    Ok(())
}

fn wait_for_ready(trays: &Receiver<String>, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match trays.recv_timeout(left) {
            Ok(label) if label == READY_LABEL => return Ok(()),
            Ok(_) => {}
            Err(_) => bail!("timed out"),
        }
    }
}

fn drain(trays: &Receiver<String>) {
    while trays.try_recv().is_ok() {}
}

/// Prints events and feeds tray labels to the main thread.
struct Console {
    started: Instant,
    last: Mutex<Shown>,
    trays: Sender<String>,
}

#[derive(Default)]
struct Shown {
    overlay: String,
}

impl Console {
    fn say(&self, line: &str) {
        eprintln!("[{:7.3}s] {line}", self.started.elapsed().as_secs_f64());
    }
}

impl EventSink for Console {
    fn emit(&self, event: Event) {
        match event {
            Event::TrayChanged(tray) => {
                self.say(&format!("tray: {}", tray.accessibility_label));
                let _ = self.trays.send(tray.accessibility_label);
            }
            Event::OverlayChanged(view) => {
                let line = describe(&view);
                let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
                if line != last.overlay {
                    self.say(&line);
                    last.overlay = line;
                }
            }
            Event::HistoryChanged => self.say("history changed"),
            Event::OpenOnboarding => self.say("would open onboarding"),
            Event::OpenMainWindow(section) => self.say(&format!("would open {section:?}")),
            Event::ShowUpdateNotice(notice) => {
                self.say(&format!("would show the update notice: {}", notice.body));
            }
            // windows the terminal cannot show
            _ => {}
        }
    }
}

// one line per overlay state, ignoring level-only changes
fn describe(view: &OverlayView) -> String {
    if !view.visible {
        return "overlay: hidden".to_owned();
    }
    let clock = view
        .trailing
        .as_ref()
        .and_then(|t| t.clock.as_ref())
        .map_or(String::new(), |c| format!(" [{}]", c.text));
    match &view.content {
        OverlayContent::Notice(notice) => format!("notice: {} — {}", notice.title, notice.message),
        OverlayContent::Processing { label, preview } => match preview {
            Some(preview) => format!("overlay: {label} (last preview: {preview})"),
            None => format!("overlay: {label}"),
        },
        OverlayContent::SilentMicrophone { title, .. } => format!("overlay: {title}{clock}"),
        OverlayContent::Preview { text, .. } => format!("preview: {text}{clock}"),
        OverlayContent::Waveform { .. } => format!("overlay: recording{clock}"),
    }
}

/// The terminal as the shell's ports.
struct Terminal {
    console: Arc<Console>,
}

impl TextOutput for Terminal {
    fn focused_target(&self) -> Option<Target> {
        Some(Target {
            process_id: std::process::id(),
            name: Some("Terminal".to_owned()),
            app_id: None,
        })
    }

    fn field_text(&self, _: Option<&Target>) -> Option<FieldText> {
        None
    }

    fn insert(&self, text: &str, _: Option<&Target>) -> InsertOutcome {
        self.console.say("delivered (stdout)");
        println!("{text}");
        InsertOutcome::Inserted
    }

    fn copy(&self, text: &str) -> bool {
        println!("{text}");
        true
    }
}

impl Permissions for Terminal {
    fn microphone(&self) -> MicrophonePermission {
        // the OS prompts on first capture; the terminal app owns the grant
        MicrophonePermission::Granted
    }

    fn request_microphone(&self) -> bool {
        true
    }

    fn accessibility_trusted(&self) -> bool {
        true
    }

    fn prompt_accessibility(&self) {}

    fn open_settings(&self, pane: PrivacyPane) {
        self.console.say(&format!("open {pane:?} privacy settings"));
    }
}

impl SoundPlayer for Terminal {
    fn play(&self, cue: Cue) {
        self.console.say(&format!("cue: {cue:?}"));
    }
}

impl Power for Terminal {
    fn begin_prevent_idle_sleep(&self, reason: &str) {
        self.console.say(&format!("idle sleep held: {reason}"));
    }

    fn end_prevent_idle_sleep(&self) {
        self.console.say("idle sleep released");
    }
}

// the terminal feeds combo presses itself, so registering always works
impl HotkeySource for Terminal {
    fn register(&self, _: &Shortcut) -> Result<(), i32> {
        Ok(())
    }

    fn unregister(&self) {}

    fn reenable_stream(&self) {}

    fn register_cancel(&self, _: &[ModifierSet]) -> Result<(), String> {
        Err("the terminal has no global Escape".to_owned())
    }

    fn unregister_cancel(&self) {}

    fn globe_key_action(&self) -> GlobeKeyAction {
        GlobeKeyAction::Unknown
    }

    fn open_keyboard_settings(&self) {}
}

impl Notifier for Terminal {
    fn authorization(&self) -> NotificationAuthorization {
        NotificationAuthorization::Unsupported
    }

    fn post(&self, _: &str, _: &str) -> Result<(), String> {
        Err("no notifications in the terminal".to_owned())
    }

    fn request_authorization(&self) {}
}

// the terminal has no login item, updater, or file manager
impl LoginItem for Terminal {
    fn status(&self) -> Option<LoginItemRegistryStatus> {
        None
    }

    fn set_enabled(&self, _: bool) -> Result<(), String> {
        Err("no login items in the terminal".to_owned())
    }

    fn open_settings(&self) {}
}

impl Updater for Terminal {
    fn state(&self) -> UpdaterState {
        UpdaterState::default()
    }

    fn check_now(&self) {}

    fn set_automatic_checks(&self, _: bool) {}

    fn restart_to_update(&self) -> bool {
        false
    }

    fn open_download(&self) {}
}

impl Files for Terminal {
    fn reveal(&self, path: &Path) -> Result<(), String> {
        self.console.say(&format!("reveal {}", path.display()));
        Ok(())
    }

    fn open(&self, path: &Path) -> Result<(), String> {
        self.console.say(&format!("open {}", path.display()));
        Ok(())
    }
}
