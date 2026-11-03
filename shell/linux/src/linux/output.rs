//! Clipboard delivery and consented Wayland keyboard injection.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use ashpd::desktop::{
    PersistMode, Session,
    remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions},
};
use onda_app::ports::{CopyOutcome, Delivery, FieldText, InsertOutcome, Target, TextOutput};
use tokio::sync::OwnedMutexGuard;

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

    // Everything that waits comes before the commit: the held shortcut
    // keys and, without a virtual keyboard, the RemoteDesktop consent
    // prompt. None when the delivery was cancelled during the prompt.
    fn keyboard(&self, delivery: &Delivery) -> Option<anyhow::Result<Keyboard>> {
        if let Err(error) = self.held.settle() {
            return Some(Err(error));
        }
        if !process::wayland() {
            return Some(Ok(Keyboard::Xdotool));
        }
        if virtual_keyboard() && process::available("wtype") {
            return Some(Ok(Keyboard::Wtype));
        }
        unless_cancelled(
            &self.runtime,
            grant(self.portal.clone(), self.token.clone()),
            delivery,
        )
        .map(|opened| opened.map(Keyboard::Portal))
    }

    fn press(&self, keyboard: Keyboard, shift: bool) -> anyhow::Result<()> {
        let mut grant = match keyboard {
            Keyboard::Xdotool => {
                process::run(
                    "xdotool",
                    &["key", if shift { "ctrl+shift+v" } else { "ctrl+v" }],
                    &[],
                )?;
                return Ok(());
            }
            Keyboard::Wtype => {
                let keys = wtype_arguments(shift);
                let args: Vec<&str> = keys.iter().map(String::as_str).collect();
                process::run("wtype", &args, &[])?;
                return Ok(());
            }
            Keyboard::Portal(grant) => grant,
        };
        self.runtime.block_on(async {
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
}

// Runs `opening` in its own task and waits for it unless the delivery is
// cancelled first. A cancel stops only the wait: a consent prompt the user
// is still answering yields a grant for later.
fn unless_cancelled<T: Send + 'static>(
    runtime: &tokio::runtime::Handle,
    opening: impl Future<Output = anyhow::Result<T>> + Send + 'static,
    delivery: &Delivery,
) -> Option<anyhow::Result<T>> {
    let opening = runtime.spawn(opening);
    runtime.block_on(async {
        tokio::select! {
            opened = opening => Some(
                opened
                    .map_err(anyhow::Error::from)
                    .and_then(|opened| opened),
            ),
            () = delivery.cancelled() => None,
        }
    })
}

enum Keyboard {
    Xdotool,
    Wtype,
    Portal(OwnedMutexGuard<Option<Grant>>),
}

// The keyboard grant, opening one when none is held. The guard keeps the
// idle closer away until the keys are sent.
async fn grant(
    portal: Arc<tokio::sync::Mutex<Option<Grant>>>,
    token: PathBuf,
) -> anyhow::Result<OwnedMutexGuard<Option<Grant>>> {
    let mut grant = portal.clone().lock_owned().await;
    if grant.is_none() {
        *grant = Some(tokio::time::timeout(Duration::from_secs(120), open_grant(&token)).await??);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let mut grant = portal.lock().await;
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
    Ok(grant)
}

async fn open_grant(path: &Path) -> anyhow::Result<Grant> {
    let portal = RemoteDesktop::with_connection(super::portal::connection().await?).await?;
    let session = portal.create_session(Default::default()).await?;
    let start = async {
        let token = std::fs::read_to_string(path).ok();
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
            let staged = path.with_extension("next");
            let save = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .mode(0o600)
                    .open(&staged)?;
                file.write_all(token.as_bytes())?;
                file.sync_all()?;
                std::fs::rename(staged, path)
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

impl TextOutput for Output {
    fn focused_target(&self) -> Option<Target> {
        None
    }
    fn field_text(&self, _: Option<&Target>) -> Option<FieldText> {
        None
    }
    fn insert(&self, text: &str, _: Option<&Target>, delivery: &Delivery) -> InsertOutcome {
        let _serial = self.serial.lock().unwrap_or_else(|e| e.into_inner());
        let Some(keyboard) = self.keyboard(delivery) else {
            return InsertOutcome::Cancelled;
        };
        let previous = match super::clipboard::Snapshot::read() {
            Ok(previous) => Some(previous),
            Err(error) => {
                tracing::debug!(%error, "clipboard cannot be preserved");
                None
            }
        };
        if !delivery.commit() {
            return InsertOutcome::Cancelled;
        }
        if let Err(error) = self.copy_text(text) {
            tracing::warn!(%error, "clipboard write failed");
            return InsertOutcome::CopyFailed;
        }
        match keyboard
            .and_then(|keyboard| self.press(keyboard, self.terminal.load(Ordering::Relaxed)))
        {
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
    fn copy(&self, text: &str, delivery: &Delivery) -> CopyOutcome {
        // an insert holding the lock can wait on a consent prompt for long
        let _serial = self.serial.lock().unwrap_or_else(|e| e.into_inner());
        if !delivery.commit() {
            return CopyOutcome::Cancelled;
        }
        match self.copy_text(text) {
            Ok(()) => CopyOutcome::Copied,
            Err(error) => {
                tracing::warn!(%error, "clipboard write failed");
                CopyOutcome::Failed
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
    use super::*;

    #[test]
    // DICT-19: the consent prompt comes before the commit; cancelling ends
    // the wait at once, and the answer still lands for the next delivery
    fn a_cancel_ends_the_consent_wait_and_keeps_the_grant() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let portal = Arc::new(tokio::sync::Mutex::new(None));
        let (answer, answered) = tokio::sync::oneshot::channel();
        let (started, starting) = tokio::sync::oneshot::channel();
        let opening = {
            let portal = portal.clone();
            async move {
                let mut grant = portal.lock_owned().await;
                started.send(()).expect("consent wait started");
                *grant = Some(answered.await?);
                Ok::<_, anyhow::Error>(grant)
            }
        };
        let delivery = Delivery::new();
        let waiting = std::thread::spawn({
            let runtime = runtime.handle().clone();
            let delivery = delivery.clone();
            move || unless_cancelled(&runtime, opening, &delivery).is_none()
        });
        runtime.block_on(starting).expect("consent wait started");
        delivery.cancel();
        assert!(waiting.join().expect("waiting thread"), "the wait ended");

        assert!(answer.send("Troy Barnes consents").is_ok());
        let kept = runtime.block_on(async { *portal.lock().await });
        assert_eq!(kept, Some("Troy Barnes consents"));
    }

    #[test]
    // DICT-19: a copy queued behind an insert that waits on consent commits
    // only once it holds the lock, so a cancel meanwhile copies nothing
    fn a_copy_cancelled_while_queued_copies_nothing() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let output = Arc::new(Output::new(
            runtime.handle().clone(),
            Path::new("/nonexistent/greendale"),
            Arc::default(),
        ));
        let inserting = output.serial.lock().expect("serial lock");
        let delivery = Delivery::new();
        let copying = std::thread::spawn({
            let output = output.clone();
            let delivery = delivery.clone();
            move || output.copy("Troy Barnes", &delivery)
        });
        delivery.cancel();
        drop(inserting);
        assert_eq!(
            copying.join().expect("copying thread"),
            CopyOutcome::Cancelled
        );
    }

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
