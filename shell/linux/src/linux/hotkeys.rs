//! Portal shortcuts on Wayland, X11 grabs elsewhere, and command fallback.
//! On X11 a session also claims Escape, which cancels it.

mod escape;
pub mod keys;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use onda_app::{HotkeyEvent, ports::HotkeySource};
use onda_domain::hotkey::{GlobeKeyAction, Modifier, ModifierSet, Shortcut};
use tokio::sync::watch;

use super::{Message, send};

#[derive(Default)]
pub struct HeldKeys(Mutex<Option<(bool, Instant)>>);

impl HeldKeys {
    pub fn update(&self, pressed: bool) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some((pressed, Instant::now()));
    }

    pub fn settle(&self) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let state = *self.0.lock().unwrap_or_else(|e| e.into_inner());
            let wait = state.and_then(|(held, time)| {
                (time
                    + if held {
                        Duration::from_secs(2)
                    } else {
                        Duration::from_millis(250)
                    })
                .checked_duration_since(Instant::now())
            });
            match wait {
                Some(wait) => std::thread::sleep(wait.min(Duration::from_millis(20))),
                None => {
                    anyhow::ensure!(
                        !state.is_some_and(|(held, _)| held),
                        "shortcut keys are still held"
                    );
                    return Ok(());
                }
            }
            anyhow::ensure!(Instant::now() < deadline, "shortcut keys did not settle");
        }
    }
}

/// The status the core gets with a failed registration. The desktop gives
/// no code for one, and the core's Linux copy shows none.
pub const NO_STATUS: i32 = -1;

pub struct Hotkeys {
    sender: watch::Sender<Option<Shortcut>>,
    events: async_channel::Sender<Message>,
    // held from a session's start to its end, on X11 only
    escape: Mutex<Option<escape::Claim>>,
}

impl Hotkeys {
    pub fn new(
        runtime: &tokio::runtime::Handle,
        events: async_channel::Sender<Message>,
        held: Arc<HeldKeys>,
    ) -> Self {
        let (sender, mut changes) = watch::channel(None);
        let runtime = runtime.clone();
        let hotkey_events = events.clone();
        std::thread::spawn(move || {
            let events = hotkey_events;
            loop {
                let current = changes.borrow_and_update().clone();
                if let Some(shortcut) = current {
                    let result = if super::process::wayland() {
                        runtime.block_on(portal(&shortcut, &mut changes, &events, &held))
                    } else {
                        x11(&shortcut, &mut changes, &events, &held)
                    };
                    held.update(false);
                    match result {
                        Ok(()) => changes.mark_changed(),
                        // the core already asked for another shortcut, and
                        // would read this failure as that one's
                        Err(error) if changes.has_changed().unwrap_or(true) => {
                            tracing::debug!(%error, "a replaced global shortcut failed");
                        }
                        Err(error) => {
                            tracing::warn!(%error, "the global shortcut is unavailable");
                            send(
                                &events,
                                Message::ShortcutUnavailable(format!(
                                    "Global shortcut unavailable: {error}. Bind onda --toggle in your desktop keyboard settings."
                                )),
                            );
                        }
                    }
                }
                if runtime.block_on(changes.changed()).is_err() {
                    break;
                }
            }
        });
        Self {
            sender,
            events,
            escape: Mutex::default(),
        }
    }
}

impl HotkeySource for Hotkeys {
    // Hands the shortcut to the worker and returns before the desktop has
    // answered, because the portal may wait on a dialog. A refusal reaches
    // the core later as `Message::ShortcutUnavailable`.
    fn register(&self, shortcut: &Shortcut) -> Result<(), i32> {
        self.sender
            .send(Some(shortcut.clone()))
            .map_err(|_| NO_STATUS)
    }
    fn unregister(&self) {
        self.sender.send_replace(None);
    }
    fn reenable_stream(&self) {}
    fn register_cancel(&self, modifiers: &[ModifierSet]) -> Result<(), String> {
        // a portal shortcut is bound for the whole desktop session, so
        // claiming Escape there would take it from every application for good
        if super::process::wayland() {
            return Ok(());
        }
        let mut escape = self.escape.lock().unwrap_or_else(|e| e.into_inner());
        if escape.is_some() {
            return Ok(());
        }
        let events = self.events.clone();
        let claim = escape::Claim::new(modifiers, move || {
            send(&events, Message::Hotkey(HotkeyEvent::Cancel));
        })
        .map_err(|error| error.to_string())?;
        *escape = Some(claim);
        Ok(())
    }
    fn unregister_cancel(&self) {
        self.escape.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    fn globe_key_action(&self) -> GlobeKeyAction {
        GlobeKeyAction::Unknown
    }
    fn open_keyboard_settings(&self) {
        if let Err(error) = super::process::launch("gnome-control-center", &["keyboard"]) {
            tracing::debug!(%error);
        }
    }
}

pub fn trigger(shortcut: &Shortcut) -> anyhow::Result<String> {
    anyhow::ensure!(
        !shortcut.is_lone_modifier(),
        "choose a key with Ctrl, Alt or Super"
    );
    let key = shortcut.key().as_str();
    let key = match key {
        "Space" => "space",
        "Enter" => "Return",
        "Backspace" => "BackSpace",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        other => other
            .strip_prefix("Key")
            .or_else(|| other.strip_prefix("Digit"))
            .unwrap_or(other),
    };
    let mut parts = Vec::new();
    for (modifier, name) in [
        (Modifier::Control, "CTRL"),
        (Modifier::Alt, "ALT"),
        (Modifier::Shift, "SHIFT"),
        (Modifier::Meta, "LOGO"),
    ] {
        if shortcut.modifiers().contains(modifier) {
            parts.push(name);
        }
    }
    parts.push(key);
    Ok(parts.join("+"))
}

async fn portal(
    shortcut: &Shortcut,
    changes: &mut watch::Receiver<Option<Shortcut>>,
    events: &async_channel::Sender<Message>,
    held: &HeldKeys,
) -> anyhow::Result<()> {
    let trigger = trigger(shortcut)?;
    let portal = GlobalShortcuts::with_connection(super::portal::connection().await?).await?;
    let session = portal.create_session(Default::default()).await?;
    let result = async {
        let mut pressed = portal.receive_activated().await?;
        let mut released = portal.receive_deactivated().await?;
        let mut changed = portal.receive_shortcuts_changed().await?;
        let mut closed = session.receive_closed().await?;
        let shortcuts = [NewShortcut::new(format!("dictation-{trigger}"), "Dictate with Onda").preferred_trigger(trigger.as_str())];
        let bound = tokio::select! {
            result = portal.bind_shortcuts(&session, &shortcuts, None, Default::default()) => result?.response()?,
            _ = changes.changed() => return Ok(()),
        };
        let describe = |shortcuts: &[ashpd::desktop::global_shortcuts::Shortcut]| {
            let description = shortcuts.first().map(|shortcut| shortcut.trigger_description()).filter(|value| !value.is_empty())
                .unwrap_or("Assign Onda’s shortcut in desktop keyboard settings");
            send(events, Message::ShortcutStatus(description.into()));
        };
        describe(bound.shortcuts());
        loop {
            tokio::select! {
                _ = changes.changed() => break,
                event = pressed.next() => {
                    anyhow::ensure!(event.is_some(), "shortcut connection closed");
                    held.update(true);
                    send(events, Message::Hotkey(HotkeyEvent::ComboDown));
                }
                event = released.next() => {
                    anyhow::ensure!(event.is_some(), "shortcut connection closed");
                    held.update(false);
                    send(events, Message::Hotkey(HotkeyEvent::ComboUp));
                }
                event = changed.next() => {
                    if let Some(event) = event { describe(event.shortcuts()); }
                    else { anyhow::bail!("shortcut connection closed"); }
                }
                _ = closed.next() => {
                    send(events, Message::Hotkey(HotkeyEvent::Cancel));
                    anyhow::bail!("desktop closed the shortcut session");
                }
            }
        }
        Ok(())
    }.await;
    if let Err(error) = session.close().await {
        tracing::debug!(%error, "closing shortcut session");
    }
    result
}

fn x11(
    shortcut: &Shortcut,
    changes: &mut watch::Receiver<Option<Shortcut>>,
    events: &async_channel::Sender<Message>,
    held: &HeldKeys,
) -> anyhow::Result<()> {
    use x11rb::connection::Connection as _;
    use x11rb::errors::ReplyError;
    use x11rb::protocol::xkb::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};
    use x11rb::protocol::{ErrorKind, Event};
    use x11rb::wrapper::ConnectionExt as _;

    anyhow::ensure!(
        !shortcut.is_lone_modifier(),
        "modifier-only shortcuts are unavailable"
    );
    let key = shortcut.key().as_str();
    // the key is claimed by its place on the keyboard, which is how the
    // shortcut names it, so the layout does not move the shortcut
    let keycode =
        keys::keycode(key).ok_or_else(|| anyhow::anyhow!("{key} is not a key Onda can claim"))?;
    let mut label = Vec::new();
    for (modifier, name) in [
        (Modifier::Control, "Control"),
        (Modifier::Alt, "Alt"),
        (Modifier::Shift, "Shift"),
        (Modifier::Meta, "Super"),
    ] {
        if shortcut.modifiers().contains(modifier) {
            label.push(name);
        }
    }
    label.push(key);
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    wait_for_release(&connection, keycode, changes)?;
    if changes.has_changed().unwrap_or(true) {
        return Ok(());
    }
    // a held key then repeats as presses alone, with no release between them
    connection.xkb_use_extension(1, 0)?.reply()?;
    connection
        .xkb_per_client_flags(
            xkb::ID::USE_CORE_KBD.into(),
            xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
            xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
            Default::default(),
            Default::default(),
            Default::default(),
        )?
        .reply()?;
    let keyboard = escape::Keyboard::read(&connection)?;
    let locks = keyboard.locks(&connection)?;
    for mask in escape::masks(&[shortcut.modifiers()], locks) {
        connection
            .grab_key(
                false,
                root,
                ModMask::from(mask),
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .check()
            .map_err(|error| match error {
                ReplyError::X11Error(refusal) if refusal.error_kind == ErrorKind::Access => {
                    anyhow::anyhow!("another application already uses it")
                }
                other => other.into(),
            })?;
    }
    send(events, Message::ShortcutStatus(label.join("+")));
    let mut down = false;
    while !changes.has_changed().unwrap_or(true) {
        while let Some(event) = connection.poll_for_event()? {
            // while the key is down the grab also delivers every other key
            let pressed = match event {
                Event::KeyPress(press) if press.detail == keycode => true,
                Event::KeyRelease(release) if release.detail == keycode => false,
                _ => continue,
            };
            if pressed == down {
                continue;
            }
            down = pressed;
            held.update(pressed);
            send(
                events,
                Message::Hotkey(if pressed {
                    HotkeyEvent::ComboDown
                } else {
                    HotkeyEvent::ComboUp
                }),
            );
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    // closing the connection ends the grab too, but only once the server
    // notices; the next shortcut may want the same key before that
    connection.ungrab_key(keycode, root, ModMask::ANY)?;
    connection.sync()?;
    Ok(())
}

/// Returns once the key is up or the shortcut changed. The shortcut's key
/// is still down when the user has just recorded it, and X11 would deliver
/// its repeats to a new grab as presses, which start a dictation.
fn wait_for_release(
    connection: &x11rb::rust_connection::RustConnection,
    keycode: u8,
    changes: &watch::Receiver<Option<Shortcut>>,
) -> anyhow::Result<()> {
    use x11rb::protocol::xproto::ConnectionExt as _;
    while !changes.has_changed().unwrap_or(true) {
        let down = connection.query_keymap()?.reply()?.keys;
        if down[usize::from(keycode / 8)] & (1 << (keycode % 8)) == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use onda_domain::platform::Platform;

    use super::trigger;

    #[test]
    fn every_preset_linux_offers_has_a_trigger() {
        let got: Vec<String> = Platform::linux()
            .hotkey_presets
            .iter()
            .map(|preset| trigger(&preset.shortcut()).expect("a combo the desktop can bind"))
            .collect();
        assert_eq!(got, ["CTRL+ALT+space", "ALT+LOGO+space", "CTRL+LOGO+space"]);
    }
}
