//! Composes portable Onda services with Linux desktop ports.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use onda_adapters::{
    CloudCleanup, DataPaths, DownloadListener, History, Microphone, Models, SettingsAdapter,
    forward_audio_events, forward_download_events,
};
use onda_app::{App, Event, EventSink, Ports};
use onda_audio::{AudioCapture, AudioDeviceEvents, DeviceSelection, PollingDeviceEvents};
use onda_domain::{
    hotkey::{KeyCode, Modifier, ModifierSet, Shortcut},
    models::Catalog,
    privacy_naming::PrivacyNaming,
};
use onda_history::{HistoryHooks, HistoryStore};
use onda_settings::Settings;

use super::{
    Message,
    hotkeys::{HeldKeys, Hotkeys},
    output::Output,
    send, speech,
    system::{Desktop, Secrets},
};

pub struct Core {
    pub app: App,
    pub settings: Arc<Settings>,
    pub output: Arc<Output>,
    pub runtime: tokio::runtime::Runtime,
    _devices: PollingDeviceEvents,
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("ONDA_DATA_DIR") {
        return Ok(path.into());
    }
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or_else(|| anyhow::anyhow!("No application data directory"))?;
    Ok(root.join("onda"))
}

impl Core {
    pub fn new(events: async_channel::Sender<Message>) -> anyhow::Result<Self> {
        let paths = DataPaths::new(data_dir()?);
        std::fs::create_dir_all(paths.root())?;
        let settings = Arc::new(Settings::open(paths.settings_file()));
        settings.canonicalize()?;
        if !paths.settings_file().exists() {
            settings.set_hotkey(&Shortcut::new(
                KeyCode::SPACE,
                ModifierSet::EMPTY
                    .with(Modifier::Control)
                    .with(Modifier::Alt),
            )?)?;
        }
        let catalog = Catalog::gguf();
        let language = std::env::var("LANG")
            .ok()
            .and_then(|locale| locale.split(['_', '.', '-']).next().map(str::to_owned));
        let source = Arc::new(SettingsAdapter::new(
            settings.clone(),
            catalog.clone(),
            language,
        ));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let (manager, downloads) =
            onda_models::ModelManager::new(paths.models_dir(), speech::manifest)?;
        let download_events = events.clone();
        let listener: DownloadListener =
            Arc::new(move |update| send(&download_events, Message::Download(update)));
        forward_download_events(runtime.handle(), downloads, listener.clone());
        let models = Models::new(
            manager,
            catalog.clone(),
            Box::new(speech::engine),
            runtime.handle().clone(),
            listener,
        );
        let (capture, audio_events) =
            AudioCapture::new(DeviceSelection::from_setting(&settings.input_device_uid()))?;
        let mut devices = PollingDeviceEvents::new(Duration::from_secs(2));
        let device_events = events.clone();
        devices.subscribe(
            capture
                .device_change_sink()
                .with_listener(Arc::new(move |_| {
                    send(&device_events, Message::DevicesChanged)
                })),
        );
        let held = Arc::new(HeldKeys::default());
        let output = Arc::new(Output::new(
            runtime.handle().clone(),
            paths.root(),
            held.clone(),
        ));
        let desktop = Arc::new(Desktop {
            events: events.clone(),
        });
        let app = App::start_on(
            runtime.handle(),
            Ports {
                text_output: output.clone(),
                permissions: desktop.clone(),
                media: None,
                sounds: desktop.clone(),
                power: desktop.clone(),
                hotkeys: Arc::new(Hotkeys::new(runtime.handle(), events.clone(), held)),
                notifier: desktop.clone(),
                login_item: desktop.clone(),
                updater: desktop.clone(),
                files: desktop,
                audio: Arc::new(Microphone::new(capture)),
                models: Arc::new(models),
                catalog,
                history: Arc::new(History::new(
                    Arc::new(HistoryStore::new(
                        paths.history_dir(),
                        Hooks(events.clone()),
                    )),
                    settings.clone(),
                )),
                cleanup: Arc::new(CloudCleanup::new(onda_cleanup::CleanupClient::new()?, None)),
                secrets: Arc::new(Secrets {
                    application: super::application_id(),
                }),
                settings: source,
                tray_term: "application".into(),
                privacy_naming: PrivacyNaming::Accessibility,
                app_version: env!("CARGO_PKG_VERSION").into(),
            },
            Arc::new(Sink(events)),
        );
        forward_audio_events(audio_events, app.audio_signals());
        Ok(Self {
            app,
            settings,
            output,
            runtime,
            _devices: devices,
        })
    }

    pub fn finish_setup(&self) -> anyhow::Result<()> {
        if !self.settings.onboarding_completed() {
            self.settings.complete_onboarding()?;
            self.app.settings_changed();
        }
        Ok(())
    }
}

struct Sink(async_channel::Sender<Message>);
impl EventSink for Sink {
    fn emit(&self, event: Event) {
        send(&self.0, Message::Core(event));
    }
}

struct Hooks(async_channel::Sender<Message>);
impl HistoryHooks for Hooks {
    fn history_changed(&self) {
        send(&self.0, Message::Core(Event::HistoryChanged));
    }
    fn exclude_from_backup(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}
