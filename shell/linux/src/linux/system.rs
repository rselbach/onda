//! Linux desktop services supplied to the application core.

use super::{Message, process, send};
use gtk::{gio, glib, prelude::*};
use onda_app::{ports::*, seams::SecretSource};
use onda_domain::{
    cleanup::CleanupProvider,
    messages::{LoginItemRegistryStatus, NotificationAuthorization},
    types::MicrophonePermission,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    fn open_settings(&self, _: PrivacyPane) {}
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
        let Err(error) = show_item(path, &self.activation_token()) else {
            return Ok(());
        };
        tracing::debug!(%error, "no file manager selected the file; opening its folder");
        self.open(path.parent().unwrap_or(path))
    }
    fn open(&self, path: &Path) -> Result<(), String> {
        process::open_uri(&gio::File::for_path(path).uri()).map_err(|e| e.to_string())
    }
}

impl Desktop {
    /// The token that lets the file manager raise a window it already has
    /// open. Only the main loop can ask the display for one. Empty when it
    /// has none within a second, and the file manager then decides alone.
    fn activation_token(&self) -> String {
        let (reply, token) = std::sync::mpsc::channel();
        send(&self.events, Message::ActivationToken(reply));
        token
            .recv_timeout(Duration::from_secs(1))
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

// asks the file manager to open the folder of `path` with the file selected;
// the bus starts the file manager when it is not running
fn show_item(path: &Path, activation_token: &str) -> Result<(), glib::Error> {
    let uri = gio::File::for_path(path).uri();
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?.call_sync(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        "org.freedesktop.FileManager1",
        "ShowItems",
        Some(&(vec![uri.as_str()], activation_token).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    )?;
    Ok(())
}

fn autostart() -> Option<PathBuf> {
    let root = std::env::var_os("ONDA_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(|data| PathBuf::from(data).join("config"))
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(root.join("autostart/com.rselbach.onda.desktop"))
}

fn escape_desktop_exec_arg(executable: &Path) -> Result<String, String> {
    let raw = executable.to_string_lossy();
    if raw.contains(['\n', '\r', '\0']) {
        return Err("Executable path contains control characters".into());
    }
    let exec_quoted = raw
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    Ok(exec_quoted.replace('\\', "\\\\"))
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
        let executable = escape_desktop_exec_arg(&executable)?;
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
        if !process::available("secret-tool") {
            return Err("Install libsecret's secret-tool to store API keys.".into());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_exec_arg_escapes_spec_metacharacters_and_rejects_control_chars() {
        let escaped =
            escape_desktop_exec_arg(Path::new(r#"/opt/on"da/a\b`c$d%e/onda"#)).expect("valid");
        assert_eq!(escaped, r#"/opt/on\\"da/a\\\\b\\`c\\$d%%e/onda"#);

        assert!(escape_desktop_exec_arg(Path::new("/opt/onda\nExec=evil")).is_err());
        assert!(escape_desktop_exec_arg(Path::new("/opt/onda\r")).is_err());
        assert!(escape_desktop_exec_arg(Path::new("/opt/onda\0")).is_err());
    }
}
