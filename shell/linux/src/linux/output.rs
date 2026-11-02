//! Clipboard delivery and consented Wayland keyboard injection.

use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use ashpd::desktop::{
    PersistMode, Session,
    remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions},
};
use onda_app::ports::{FieldText, InsertOutcome, Target, TextOutput};

use super::{hotkeys::HeldKeys, process};

pub struct Output {
    runtime: tokio::runtime::Handle,
    token: PathBuf,
    portal: Arc<tokio::sync::Mutex<Option<Grant>>>,
    serial: Mutex<()>,
    pub terminal: AtomicBool,
    pub held: Arc<HeldKeys>,
}

struct Grant {
    portal: RemoteDesktop,
    session: Session<RemoteDesktop>,
    used: std::time::Instant,
    restorable: bool,
}

impl Output {
    pub fn new(
        runtime: tokio::runtime::Handle,
        root: &std::path::Path,
        held: Arc<HeldKeys>,
    ) -> Self {
        Self {
            runtime,
            token: root.join("remote-desktop-token"),
            portal: Arc::default(),
            serial: Mutex::new(()),
            terminal: AtomicBool::new(false),
            held,
        }
    }

    fn copy_text(&self, text: &str) -> anyhow::Result<()> {
        if process::wayland() {
            process::write(
                "wl-copy",
                &["--type", "text/plain;charset=utf-8"],
                text.as_bytes(),
            )?;
        } else {
            process::write(
                "xclip",
                &["-selection", "clipboard", "-in"],
                text.as_bytes(),
            )?;
        }
        Ok(())
    }

    fn paste(&self, shift: bool) -> anyhow::Result<()> {
        self.held.settle()?;
        if !process::wayland() {
            process::run(
                "xdotool",
                &["key", if shift { "ctrl+shift+v" } else { "ctrl+v" }],
                &[],
            )?;
            return Ok(());
        }
        if virtual_keyboard() && process::available("wtype") {
            let keys = wtype_arguments(shift);
            let args: Vec<&str> = keys.iter().map(String::as_str).collect();
            process::run("wtype", &args, &[])?;
            return Ok(());
        }
        self.runtime.block_on(async {
            let mut grant = self.portal.lock().await;
            if grant.is_none() {
                *grant =
                    Some(tokio::time::timeout(Duration::from_secs(120), self.open_grant()).await??);
                let shared = self.portal.clone();
                self.runtime.spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(15)).await;
                        let mut grant = shared.lock().await;
                        let Some(current) = grant.as_ref() else {
                            break;
                        };
                        if !current.restorable {
                            break;
                        }
                        if current.used.elapsed() >= Duration::from_secs(15) {
                            if let Some(current) = grant.take()
                                && let Err(error) = current.session.close().await
                            {
                                tracing::debug!(%error);
                            }
                            break;
                        }
                    }
                });
            }
            // The selection must reach the target; a consent dialog may also have just closed.
            tokio::time::sleep(Duration::from_millis(250)).await;
            let current = grant
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("keyboard session missing"))?;
            current.used = std::time::Instant::now();
            let mut keys = vec![0xffe3];
            if shift {
                keys.push(0xffe1);
            }
            keys.push(0x76);
            let result = async {
                for key in &keys {
                    current
                        .portal
                        .notify_keyboard_keysym(
                            &current.session,
                            *key,
                            KeyState::Pressed,
                            Default::default(),
                        )
                        .await?;
                }
                Ok::<_, ashpd::Error>(())
            }
            .await;
            // Always attempt balanced releases, even when a press failed.
            let mut release_error = None;
            for key in keys.iter().rev() {
                if let Err(error) = current
                    .portal
                    .notify_keyboard_keysym(
                        &current.session,
                        *key,
                        KeyState::Released,
                        Default::default(),
                    )
                    .await
                {
                    release_error = Some(error);
                }
            }
            if (result.is_err() || release_error.is_some())
                && let Some(current) = grant.take()
                && let Err(error) = current.session.close().await
            {
                tracing::debug!(%error);
            }
            result?;
            if let Some(error) = release_error {
                return Err(error.into());
            }
            Ok(())
        })
    }

    async fn open_grant(&self) -> anyhow::Result<Grant> {
        let portal = RemoteDesktop::with_connection(super::portal::connection().await?).await?;
        let session = portal.create_session(Default::default()).await?;
        let start = async {
            let token = std::fs::read_to_string(&self.token).ok();
            let mut options =
                SelectDevicesOptions::default().set_devices(Some(DeviceType::Keyboard.into()));
            if portal.version() >= 2 {
                options = options
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(token.as_deref());
            }
            portal.select_devices(&session, options).await?.response()?;
            let devices = portal
                .start(&session, None, Default::default())
                .await?
                .response()?;
            anyhow::ensure!(
                devices.devices().contains(DeviceType::Keyboard),
                "keyboard access was not granted"
            );
            let mut restorable = false;
            if let Some(token) = devices.restore_token() {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let staged = self.token.with_extension("next");
                let save = (|| -> std::io::Result<()> {
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .mode(0o600)
                        .open(&staged)?;
                    file.write_all(token.as_bytes())?;
                    file.sync_all()?;
                    std::fs::rename(staged, &self.token)
                })();
                match save {
                    Ok(()) => restorable = true,
                    Err(error) => tracing::warn!(%error, "could not save keyboard grant"),
                }
            }
            Ok::<_, anyhow::Error>(restorable)
        }
        .await;
        match start {
            Ok(restorable) => Ok(Grant {
                portal,
                session,
                used: std::time::Instant::now(),
                restorable,
            }),
            Err(error) => {
                if let Err(close) = session.close().await {
                    tracing::debug!(%close);
                }
                Err(error)
            }
        }
    }
}

impl TextOutput for Output {
    fn focused_target(&self) -> Option<Target> {
        None
    }
    fn field_text(&self, _: Option<&Target>) -> Option<FieldText> {
        None
    }
    fn insert(&self, text: &str, _: Option<&Target>) -> InsertOutcome {
        let _serial = self.serial.lock().unwrap_or_else(|e| e.into_inner());
        let previous = match super::clipboard::Snapshot::read() {
            Ok(previous) => Some(previous),
            Err(error) => {
                tracing::debug!(%error, "clipboard cannot be preserved");
                None
            }
        };
        if let Err(error) = self.copy_text(text) {
            tracing::warn!(%error, "clipboard write failed");
            return InsertOutcome::CopyFailed;
        }
        match self.paste(self.terminal.load(Ordering::Relaxed)) {
            Ok(()) => {
                if let Some(previous) = previous {
                    std::thread::sleep(Duration::from_millis(500));
                    if let Err(error) = previous.restore_if_unchanged(text) {
                        tracing::debug!(%error, "clipboard restoration unavailable");
                    }
                }
                InsertOutcome::Inserted
            }
            Err(error) => {
                tracing::warn!(%error, "automatic paste failed; transcript remains on clipboard");
                InsertOutcome::PasteFailed
            }
        }
    }
    fn copy(&self, text: &str) -> bool {
        let _serial = self.serial.lock().unwrap_or_else(|e| e.into_inner());
        match self.copy_text(text) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "clipboard write failed");
                false
            }
        }
    }
}

// Adapted from Hex's paste_keys; see licenses/hex.txt.
fn wtype_arguments(shift: bool) -> Vec<String> {
    // Allow a newly advertised keyboard to reach the focused GTK client.
    let mut args = vec!["-s".into(), "100".into()];
    // wtype assigns evdev codes in first-use order. Align V with KEY_V (47)
    // so compositor bindings see Ctrl+V rather than Ctrl+Escape. Releases of
    // never-pressed Unicode symbols populate the keymap without typing text.
    for index in 0..46 {
        args.push("-p".into());
        args.push(format!("U{:04X}", 0x2460 + index));
    }
    args.extend(["-M", "ctrl"].map(String::from));
    if shift {
        args.extend(["-M", "shift"].map(String::from));
    }
    args.extend(["-k", "v"].map(String::from));
    if shift {
        args.extend(["-m", "shift"].map(String::from));
    }
    // GTK 3 skips the last keycode during reverse lookup; leave V before it.
    args.extend(["-m", "ctrl", "-p", "VoidSymbol"].map(String::from));
    args
}

fn virtual_keyboard() -> bool {
    use wayland_client::{Connection, Dispatch, QueueHandle, globals::registry_queue_init};
    use wayland_client::{globals::GlobalListContents, protocol::wl_registry};
    struct Registry;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Registry {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    Connection::connect_to_env()
        .ok()
        .and_then(|connection| registry_queue_init::<Registry>(&connection).ok())
        .is_some_and(|(globals, _)| {
            globals.contents().with_list(|list| {
                list.iter()
                    .any(|global| global.interface == "zwp_virtual_keyboard_manager_v1")
            })
        })
}

#[cfg(test)]
mod tests {
    use super::wtype_arguments;

    #[test]
    fn paste_keymap_matches_the_compositor_and_gtk() {
        for shift in [false, true] {
            let args = wtype_arguments(shift);
            let keys: Vec<&str> = args
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|pair| matches!(pair[0].as_str(), "-p" | "-k"))
                .map(|pair| pair[1].as_str())
                .collect();
            assert_eq!(keys[46], "v");
            assert_eq!(keys.last(), Some(&"VoidSymbol"));
            assert_eq!(keys.iter().filter(|key| **key == "v").count(), 1);
        }
    }
}
