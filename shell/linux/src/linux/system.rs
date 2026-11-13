//! Linux desktop services supplied to the application core.

use super::{Message, process, send};
use gtk::gio::prelude::FileExt;
use onda_app::{ports::*, seams::SecretSource};
use onda_domain::{
    cleanup::CleanupProvider,
    messages::{LoginItemRegistryStatus, NotificationAuthorization},
    types::MicrophonePermission,
};
use std::path::{Path, PathBuf};

pub struct Desktop {
    pub events: async_channel::Sender<Message>,
}

impl Permissions for Desktop {
    fn microphone(&self) -> MicrophonePermission {
        MicrophonePermission::Granted
    }
    fn request_microphone(&self) -> bool {
        true
    }
    // Unsandboxed Linux audio has no macOS privacy grant. Keyboard portal
    // consent is requested at delivery, and refusal preserves the transcript.
    fn accessibility_trusted(&self) -> bool {
        true
    }
    fn prompt_accessibility(&self) {}
    fn open_settings(&self, _: PrivacyPane) {
        if let Err(error) = process::launch("gnome-control-center", &["privacy"]) {
            tracing::debug!(%error);
        }
    }
}

impl Notifier for Desktop {
    fn authorization(&self) -> NotificationAuthorization {
        NotificationAuthorization::Authorized
    }
    fn post(&self, title: &str, body: &str) -> Result<(), String> {
        send(&self.events, Message::Notice(title.into(), body.into()));
        Ok(())
    }
    fn request_authorization(&self) {}
}

impl SoundPlayer for Desktop {
    fn play(&self, cue: Cue) {
        let event = match cue {
            Cue::Start => "audio-rec-volume",
            Cue::Stop => "complete",
        };
        if process::available("canberra-gtk-play")
            && let Err(error) = process::run("canberra-gtk-play", &["-i", event], &[])
        {
            tracing::debug!(%error, "sound cue unavailable");
        }
    }
}

impl Power for Desktop {
    fn begin_prevent_idle_sleep(&self, reason: &str) {
        send(&self.events, Message::Inhibit(Some(reason.into())));
    }
    fn end_prevent_idle_sleep(&self) {
        send(&self.events, Message::Inhibit(None));
    }
}

impl Files for Desktop {
    fn reveal(&self, path: &Path) -> Result<(), String> {
        self.open(path.parent().unwrap_or(path))
    }
    fn open(&self, path: &Path) -> Result<(), String> {
        process::open_uri(&gtk::gio::File::for_path(path).uri()).map_err(|e| e.to_string())
    }
}

fn autostart() -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(root.join("autostart/com.rselbach.onda.desktop"))
}

impl LoginItem for Desktop {
    fn status(&self) -> Option<LoginItemRegistryStatus> {
        Some(if autostart()?.exists() {
            LoginItemRegistryStatus::Enabled
        } else {
            LoginItemRegistryStatus::NotRegistered
        })
    }
    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let path = autostart().ok_or("No configuration directory")?;
        if !enabled {
            return match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            };
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let executable = executable
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
            .replace('%', "%%");
        let parent = path.parent().ok_or("No autostart directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        std::fs::write(path, format!("[Desktop Entry]\nType=Application\nName=Onda\nExec=\"{executable}\" --background\nIcon={}\n", super::APP_ID)).map_err(|e| e.to_string())
    }
    fn open_settings(&self) {}
}

pub struct Secrets {
    pub application: String,
}

impl SecretSource for Secrets {
    fn api_key(&self, provider: CleanupProvider) -> Result<Option<String>, String> {
        let Some(account) = onda_settings::key_account(provider) else {
            return Ok(None);
        };
        if !process::available("secret-tool") {
            return Err("Install libsecret's secret-tool to store API keys.".into());
        }
        match process::optional(
            "secret-tool",
            &[
                "lookup",
                "application",
                &self.application,
                "account",
                account,
            ],
            &[],
        ) {
            Ok(Some(bytes)) => String::from_utf8(bytes)
                .map(|value| Some(value.trim().to_owned()).filter(|value| !value.is_empty()))
                .map_err(|_| "The stored API key is not valid UTF-8.".into()),
            Ok(None) => Ok(None),
            Err(error) => Err(format!("Could not read the Secret Service: {error}")),
        }
    }
    fn save_api_key(&self, provider: CleanupProvider, value: &str) -> Result<(), String> {
        let Some(account) = onda_settings::key_account(provider) else {
            return Ok(());
        };
        let args = if value.trim().is_empty() {
            vec![
                "clear",
                "application",
                &self.application,
                "account",
                account,
            ]
        } else {
            vec![
                "store",
                "--label=Onda",
                "application",
                &self.application,
                "account",
                account,
            ]
        };
        process::optional("secret-tool", &args, value.trim().as_bytes())
            .map_err(|e| e.to_string())
            .and_then(|result| {
                if result.is_some() || value.trim().is_empty() {
                    Ok(())
                } else {
                    Err("The desktop keyring did not save the API key.".into())
                }
            })
    }
}
