//! The macOS composition root and the bindings the native shell calls.
//!
//! [`OndaCore::start`] opens settings, history, and models under the data
//! directory the shell passes, picks the CoreML Parakeet backend, starts the
//! microphone, and runs the `onda-app` session engine with the shell's
//! ports. Types here mirror `onda-app` in FFI-friendly shapes; no rules live
//! in this crate.

mod commands;
mod events;
mod logging;
mod on_device;
mod ports;
mod system_speech;
mod ui;
mod updates;
mod views;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use onda_adapters::{
    CloudCleanup, DataPaths, DownloadListener, History, Keychain, Microphone, Models,
    SettingsAdapter, forward_audio_events, forward_download_events,
};
use onda_app::seams::SettingsSource;
use onda_app::system_speech::SystemSpeechEngine;
use onda_app::{App, HotkeyEvent, Ports};
use onda_asr::SpeechEngine;
use onda_asr_coreml::ParakeetEngine;
use onda_asr_coreml::manifest::manifest;
use onda_audio::{AudioCapture, AudioDeviceEvents, DeviceSelection, PollingDeviceEvents};
use onda_cleanup::CleanupClient;
use onda_domain::hotkey::{
    HeldModifiers, KeyCode, KeyNames, ModifierFlagsChange, RecorderEvent, Shortcut,
};
use onda_domain::messages;
use onda_domain::models::{Catalog, ModelId};
use onda_domain::platform::Platform;
use onda_domain::privacy_naming::PrivacyNaming;
use onda_history::HistoryStore;
use onda_models::ModelManager;
use onda_settings::{KeychainSecretStore, Settings};
use tokio::runtime::Runtime;
use updates::AppUpdater;

pub use events::*;
pub use on_device::*;
pub use ports::*;
pub use system_speech::*;
pub use ui::*;
pub use views::*;

uniffi::setup_scaffolding!();

/// How often input devices are polled for default and list changes
/// (AUD-10).
const DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Why the core could not start.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum StartError {
    /// Setup failed.
    Failed {
        /// What went wrong.
        message: String,
    },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for StartError {}

/// Copy for the critical alert the shell shows when [`OndaCore::start`]
/// fails; its text is the [`StartError`] (MSG-3).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StartFailureAlert {
    /// "Onda Could Not Start".
    pub title: String,
    /// "Quit".
    pub button: String,
}

/// The start-failure alert's copy. Free so the shell can call it without a
/// running core.
#[uniffi::export]
pub fn start_failure_alert() -> StartFailureAlert {
    StartFailureAlert {
        title: messages::START_FAILURE_TITLE.to_owned(),
        button: messages::START_FAILURE_BUTTON.to_owned(),
    }
}

/// Why a shortcut was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum HotkeyError {
    /// The key is not a W3C `KeyboardEvent.code` name, or the shortcut
    /// breaks HOT-4.
    Invalid {
        /// What to show the user.
        message: String,
    },
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for HotkeyError {}

fn failed(what: &str, error: impl fmt::Display) -> StartError {
    StartError::Failed {
        message: format!("{what}: {error}"),
    }
}

/// The shell's port implementations, passed to [`OndaCore::start`].
#[derive(uniffi::Record)]
pub struct ShellPorts {
    /// Text delivery.
    pub text_output: Arc<dyn TextOutputPort>,
    /// Privacy permissions.
    pub permissions: Arc<dyn PermissionsPort>,
    /// Cue sounds.
    pub sounds: Arc<dyn SoundPort>,
    /// Idle-sleep prevention.
    pub power: Arc<dyn PowerPort>,
    /// The dictation shortcut and the session-only Escape.
    pub hotkeys: Arc<dyn HotkeyPort>,
    /// System notifications.
    pub notifier: Arc<dyn NotifierPort>,
    /// File manager services.
    pub files: Arc<dyn FilesPort>,
    /// The login-item registry.
    pub login_item: Arc<dyn LoginItemPort>,
    /// What the updater needs from the shell.
    pub updater: Arc<dyn UpdaterPort>,
    /// The running bundle; none for a dev build, which never updates
    /// (UPD-4).
    #[uniffi(default = None)]
    pub update_host: Option<UpdateHost>,
    /// Now-playing control; none disables media pause (MEDIA-1).
    #[uniffi(default = None)]
    pub media: Option<Arc<dyn MediaControlPort>>,
    /// The platform speech recognizer; none leaves System Speech out of
    /// the catalog (MOD-4).
    #[uniffi(default = None)]
    pub system_speech: Option<Arc<dyn SystemSpeechPort>>,
    /// Apple Intelligence's on-device model; none leaves it out of the
    /// cleanup providers.
    #[uniffi(default = None)]
    pub on_device: Option<Arc<dyn OnDeviceCleanupPort>>,
}

/// The running Onda core as seen by a shell. Commands return at once; the
/// core reports through [`CoreListener`].
#[derive(uniffi::Object)]
pub struct OndaCore {
    app: Arc<Mutex<Option<App>>>,
    microphone: Arc<Microphone>,
    permissions: Arc<dyn PermissionsPort>,
    // kept alive for the core's lifetime
    _devices: Mutex<PollingDeviceEvents>,
    runtime: Mutex<Option<Runtime>>,
}

#[uniffi::export]
impl OndaCore {
    /// Starts the core with its data under `data_dir` (the shell resolves
    /// it). `os_language` is the OS locale's language
    /// code (LANG-2); `app_version` is the bundle's short version (WIN-14,
    /// WIN-21); `tray_term` is what copy calls the tray icon on this
    /// platform, "menu bar icon" on macOS;
    /// `privacy_naming` is what the running OS calls the Accessibility
    /// permission. Events flow to `listener` until
    /// [`OndaCore::shutdown`].
    #[uniffi::constructor]
    pub fn start(
        ports: ShellPorts,
        data_dir: String,
        os_language: Option<String>,
        app_version: String,
        tray_term: String,
        privacy_naming: PrivacyNaming,
        listener: Arc<dyn CoreListener>,
    ) -> Result<Arc<Self>, StartError> {
        logging::init();
        let paths = DataPaths::new(PathBuf::from(data_dir));
        tracing::info!(data_dir = %paths.root().display(), "starting the core");

        let settings = Arc::new(Settings::open(paths.settings_file()));
        // SET-3: rewrite stale enum-like values once per launch
        if let Err(error) = settings.canonicalize() {
            tracing::error!(%error, "could not canonicalize settings");
        }
        let system_speech = ports
            .system_speech
            .map(|port| Arc::new(ShellSpeech(port)) as Arc<dyn onda_app::ports::SystemSpeech>);
        let catalog = Catalog::new(system_speech.as_ref().map(|speech| speech.info()));
        let settings_source: Arc<dyn SettingsSource> = Arc::new(SettingsAdapter::new(
            settings.clone(),
            catalog.clone(),
            os_language,
            Platform::macos(privacy_naming).default_shortcut,
        ));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("onda-core")
            .enable_all()
            .build()
            .map_err(|error| failed("Could not start the core runtime", error))?;

        let app: Arc<Mutex<Option<App>>> = Arc::new(Mutex::new(None));
        let (manager, downloads) = ModelManager::new(paths.models_dir(), manifest)
            .map_err(|error| failed("Could not open the models folder", error))?;
        prepare_models_dir(&manager, ports.files.as_ref());
        let download_app = app.clone();
        let download_listener: DownloadListener = Arc::new(move |update| {
            with_app(&download_app, move |app| app.download_event(update));
        });
        forward_download_events(runtime.handle(), downloads, download_listener.clone());
        let engine_settings = settings_source.clone();
        let models = Models::new(
            manager,
            catalog.clone(),
            Box::new(move |model, dir| match (model, &system_speech) {
                (ModelId::SystemSpeech, Some(speech)) => Some(Box::new(SystemSpeechEngine::new(
                    speech.clone(),
                    engine_settings.clone(),
                ))),
                _ => parakeet_engine(model, dir),
            }),
            runtime.handle().clone(),
            download_listener,
        );

        let selection = DeviceSelection::from_setting(&settings.input_device_uid());
        let (capture, audio_events) = AudioCapture::new(selection)
            .map_err(|error| failed("Could not start audio capture", error))?;
        let mut devices = PollingDeviceEvents::new(DEVICE_POLL_INTERVAL);
        let device_app = app.clone();
        // the General page lists devices live (WIN-9)
        devices.subscribe(
            capture
                .device_change_sink()
                .with_listener(Arc::new(move |_| {
                    with_app(&device_app, App::input_devices_changed);
                })),
        );
        let microphone = Arc::new(Microphone::new(capture));

        let history = HistoryStore::new(paths.history_dir(), HistoryFiles(ports.files.clone()));
        let cleanup = CleanupClient::new()
            .map_err(|error| failed("Could not create the cleanup client", error))?;
        let shell = Arc::new(Shell {
            text: ports.text_output,
            permissions: ports.permissions.clone(),
            sounds: ports.sounds,
            power: ports.power,
            hotkeys: ports.hotkeys,
            notifier: ports.notifier,
            login_item: ports.login_item,
            files: ports.files.clone(),
        });
        let updater = AppUpdater::start(
            ports.update_host,
            ports.updater,
            app.clone(),
            paths.root(),
            &app_version,
            runtime.handle(),
        );
        // the core reads the platform for as long as the app runs, so the
        // shell's term is leaked once to live that long
        let platform = Platform {
            tray_term: Box::leak(tray_term.into_boxed_str()),
            ..Platform::macos(privacy_naming)
        };
        let core_ports = Ports {
            text_output: shell.clone(),
            permissions: shell.clone(),
            media: ports
                .media
                .map(|media| Arc::new(ShellMedia(media)) as Arc<dyn onda_app::ports::MediaControl>),
            sounds: shell.clone(),
            power: shell.clone(),
            hotkeys: shell.clone(),
            notifier: shell.clone(),
            login_item: shell.clone(),
            updater: Arc::new(updater),
            files: shell,
            audio: microphone.clone(),
            models: Arc::new(models),
            catalog,
            history: Arc::new(History::new(Arc::new(history), settings.clone())),
            cleanup: Arc::new(CloudCleanup::new(
                cleanup,
                ports.on_device.map(|port| {
                    Arc::new(ShellOnDevice(port)) as Arc<dyn onda_app::ports::OnDeviceModel>
                }),
                platform.computer_term,
            )),
            secrets: Arc::new(Keychain::new(KeychainSecretStore::new())),
            settings: settings_source,
            platform,
            app_version,
        };
        let sink = Arc::new(ListenerSink {
            listener,
            tray_term: platform.tray_term,
        });
        let started = App::start_on(runtime.handle(), core_ports, sink);
        forward_audio_events(audio_events, started.audio_signals());
        *lock(&app) = Some(started);

        Ok(Arc::new(Self {
            app,
            microphone,
            permissions: ports.permissions,
            _devices: Mutex::new(devices),
            runtime: Mutex::new(Some(runtime)),
        }))
    }

    /// The UI is up (ONB-13): loads the model unless first-run onboarding
    /// is pending (ASR-1), and prepares the microphone when access is
    /// already granted (AUD-4).
    pub fn ui_ready(&self) {
        self.with_app(App::ui_ready);
        let permissions = self.permissions.clone();
        let microphone = self.microphone.clone();
        let spawned = thread::Builder::new()
            .name("onda-audio-prepare".to_owned())
            .spawn(move || {
                let granted = matches!(permissions.microphone(), MicrophoneStatus::Granted);
                if granted {
                    microphone.capture().prepare();
                }
            });
        if let Err(error) = spawned {
            tracing::error!(%error, "could not prepare the microphone");
        }
    }

    /// Onda became the active app: re-read permissions (UI-6, PERM-2).
    pub fn app_became_active(&self) {
        self.with_app(App::app_became_active);
    }

    /// The status menu is opening: re-read permissions, installed models,
    /// and the updater, and record the current language as recent (UI-6,
    /// PERM-2, LANG-8).
    pub fn menu_will_open(&self) {
        self.with_app(App::menu_will_open);
    }

    /// A raw dictation-hotkey event, in the order it happened (HOT-20a).
    pub fn on_hotkey_event(&self, event: HotkeyInput) {
        let event = hotkey_event(event);
        self.with_app(|app| app.on_hotkey_event(event));
    }

    /// The shell hit a hotkey failure on its own (such as installing its
    /// handler) with OS status `status` (HOT-19, HOT-20).
    pub fn hotkey_registration_failed(&self, status: i32) {
        self.with_app(|app| app.hotkey_registration_failed(status));
    }

    /// Saves the shortcut `key` plus `modifiers`, then registers it once
    /// (HOT-12, SET-5).
    pub fn set_hotkey(&self, key: String, modifiers: HotkeyModifiers) -> Result<(), HotkeyError> {
        let invalid = |message: String| HotkeyError::Invalid { message };
        let key = KeyCode::parse(&key).ok_or_else(|| invalid(format!("Unknown key {key}")))?;
        let shortcut = Shortcut::new(key, modifiers.into())
            .map_err(|error| invalid(error.message(&KeyNames::MACOS).to_owned()))?;
        self.with_app(|app| app.set_hotkey(shortcut));
        Ok(())
    }

    /// The recorder's record button: cancels the running capture, else
    /// starts one (HOT-8, HOT-11).
    pub fn toggle_hotkey_capture(&self) {
        self.with_app(App::toggle_hotkey_capture);
    }

    /// The recorder was cancelled or disappeared (HOT-8).
    pub fn end_hotkey_capture(&self) {
        self.with_app(App::end_hotkey_capture);
    }

    /// The shell could not start listening for keys (HOT-11).
    pub fn hotkey_capture_failed(&self) {
        self.with_app(App::hotkey_capture_failed);
    }

    /// A local modifier-flags change while Onda's window is focused: `key`
    /// changed, and `held` plus `fn_key` are held after it (HOT-9). Returns
    /// whether to swallow the event (HOT-10).
    pub fn recorder_flags_changed(&self, key: String, held: HotkeyModifiers, fn_key: bool) -> bool {
        self.recorder_event(RecorderEvent::FlagsChanged {
            key: KeyCode::parse(&key).unwrap_or_else(unidentified_key),
            held: HeldModifiers::new(held.into(), fn_key),
        })
    }

    /// A local key-down while Onda's window is focused (HOT-9). Returns
    /// whether to swallow the event (HOT-10).
    pub fn recorder_key_down(&self, key: String, modifiers: HotkeyModifiers) -> bool {
        self.recorder_event(RecorderEvent::KeyDown {
            key: KeyCode::parse(&key).unwrap_or_else(unidentified_key),
            modifiers: modifiers.into(),
        })
    }

    /// The menu's Start / Stop / Cancel Dictation item (UI-9).
    pub fn toggle_dictation_from_menu(&self) {
        self.with_app(App::toggle_dictation_from_menu);
    }

    /// Cancels the session: overlay ✕, menu cancel (DICT-19).
    pub fn cancel_dictation(&self) {
        self.with_app(App::cancel_dictation);
    }

    /// "Retry Failed Dictation" (DICT-23).
    pub fn retry_failed_dictation(&self) {
        self.with_app(App::retry_failed_dictation);
    }

    /// "Discard Failed Recording" (DICT-24).
    pub fn discard_failed_dictation(&self) {
        self.with_app(App::discard_failed_dictation);
    }

    /// "Paste Last Transcript" / "Copy Last Transcript" (DICT-26).
    pub fn deliver_last_transcript(&self) {
        self.with_app(App::deliver_last_transcript);
    }

    /// A "Recent Transcripts" entry (DICT-27).
    pub fn deliver_transcript(&self, text: String) {
        self.with_app(|app| app.deliver_transcript(text));
    }

    /// "Retry Loading Model" (ASR-10).
    pub fn retry_model_load(&self) {
        self.with_app(App::retry_model_load);
    }

    /// The download overlay's close button (OVL-17).
    pub fn dismiss_download_overlay(&self) {
        self.with_app(App::dismiss_download_overlay);
    }

    /// Cancels any session (DICT-28a), delivers pending events, and stops.
    /// Idempotent.
    pub fn shutdown(&self) {
        let app = lock(&self.app).take();
        let Some(app) = app else {
            return;
        };
        let runtime = lock(&self.runtime).take();
        match runtime {
            Some(runtime) => {
                runtime.block_on(app.shutdown_async());
                runtime.shutdown_timeout(Duration::from_secs(1));
            }
            None => app.shutdown(),
        }
        tracing::info!("core stopped");
    }
}

impl OndaCore {
    pub(crate) fn with_app(&self, command: impl FnOnce(&App)) {
        with_app(&self.app, command);
    }

    fn recorder_event(&self, event: RecorderEvent) -> bool {
        lock(&self.app)
            .as_ref()
            .is_some_and(|app| app.recorder_event(event))
    }
}

// W3C's name for a key it has no code for: never a modifier, so a key-down
// counts as "another key" (HOT-14a) and a flags change is ignored
fn unidentified_key() -> KeyCode {
    KeyCode::parse("Unidentified").unwrap_or(KeyCode::SPACE)
}

fn hotkey_event(input: HotkeyInput) -> HotkeyEvent {
    match input {
        HotkeyInput::ComboDown => HotkeyEvent::ComboDown,
        HotkeyInput::ComboUp => HotkeyEvent::ComboUp,
        HotkeyInput::Cancel => HotkeyEvent::Cancel,
        HotkeyInput::ModifierFlags {
            key,
            family_flag_set,
            side_flag_set,
        } => HotkeyEvent::ModifierFlags(ModifierFlagsChange {
            key: KeyCode::parse(&key).unwrap_or_else(unidentified_key),
            family_flag_set,
            side_flag_set,
        }),
        HotkeyInput::KeyDown { key } => {
            HotkeyEvent::KeyDown(KeyCode::parse(&key).unwrap_or_else(unidentified_key))
        }
        HotkeyInput::StreamDisabled { status } => HotkeyEvent::StreamDisabled { status },
    }
}

fn with_app(app: &Mutex<Option<App>>, command: impl FnOnce(&App)) {
    match lock(app).as_ref() {
        Some(app) => command(app),
        None => tracing::warn!("command after shutdown"),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn parakeet_engine(model: ModelId, dir: &Path) -> Option<Box<dyn SpeechEngine>> {
    let engine = ParakeetEngine::new(model, dir)?;
    Some(Box::new(engine))
}

// creates <data>/Models and keeps it out of backups
fn prepare_models_dir(manager: &ModelManager, files: &dyn FilesPort) {
    if let Err(error) = manager.create_models_dir() {
        tracing::error!(%error, "could not create the models folder");
        return;
    }
    if let Err(error) = exclude_from_backup(files, manager.models_dir()) {
        tracing::error!(%error, "could not exclude the models folder from backups");
    }
}
