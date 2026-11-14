//! Escape, claimed on X11 for as long as a dictation runs (HOT-34).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use onda_domain::hotkey::{Modifier, ModifierSet};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, Keycode, ModMask, Window};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const ESCAPE: u32 = 0xff1b;
const NUM_LOCK: u32 = 0xff7f;
const SCROLL_LOCK: u32 = 0xff14;

// how often the claim reads its key presses
const EVENTS: Duration = Duration::from_millis(20);

/// Escape grabbed from every application. The grabs belong to an X11
/// connection that only this claim owns, so the server ends them when the
/// claim is dropped and when the process dies, however it dies.
pub struct Claim {
    released: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl Claim {
    /// Grabs Escape once for each set in `modifiers` and calls `pressed`
    /// for every press. A combination another application already holds is
    /// left to it, and the rest are still claimed.
    pub fn new(
        modifiers: &[ModifierSet],
        pressed: impl Fn() + Send + 'static,
    ) -> anyhow::Result<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        let keyboard = Keyboard::read(&connection)?;
        let keycodes = keyboard.keycodes(ESCAPE);
        anyhow::ensure!(!keycodes.is_empty(), "the keyboard has no Escape key");
        let masks = masks(modifiers, keyboard.locks(&connection)?);
        let mut grabbed = 0;
        for &keycode in &keycodes {
            for &mask in &masks {
                let grab = connection
                    .grab_key(
                        false,
                        root,
                        ModMask::from(mask),
                        keycode,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )?
                    .check();
                match grab {
                    Ok(()) => grabbed += 1,
                    Err(error) => tracing::debug!(%error, mask, "Escape is already grabbed"),
                }
            }
        }
        anyhow::ensure!(grabbed > 0, "another application holds Escape");
        tracing::info!(
            grabbed,
            of = keycodes.len() * masks.len(),
            "claimed Escape for the session"
        );
        let released = Arc::new(AtomicBool::new(false));
        let reader = std::thread::Builder::new()
            .name("onda-escape".into())
            .spawn({
                let released = released.clone();
                move || read(&connection, root, &keycodes, &released, &pressed)
            })?;
        Ok(Self {
            released,
            reader: Some(reader),
        })
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.released.store(true, Ordering::Release);
        let Some(reader) = self.reader.take() else {
            return;
        };
        reader.thread().unpark();
        if reader.join().is_err() {
            tracing::error!("the Escape reader panicked");
        }
    }
}

fn read(
    connection: &RustConnection,
    root: Window,
    keycodes: &[Keycode],
    released: &AtomicBool,
    pressed: &impl Fn(),
) {
    while !released.load(Ordering::Acquire) {
        loop {
            match connection.poll_for_event() {
                // while Escape is down the grab also delivers every other key
                Ok(Some(Event::KeyPress(press))) if keycodes.contains(&press.detail) => pressed(),
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "the Escape claim lost its X11 connection");
                    return;
                }
            }
        }
        std::thread::park_timeout(EVENTS);
    }
    // closing the connection ends the grabs too, but only once the server
    // notices; this ends them before the claim's owner is told it is gone
    let release = || -> anyhow::Result<()> {
        for &keycode in keycodes {
            connection.ungrab_key(keycode, root, ModMask::ANY)?;
        }
        Ok(connection.sync()?)
    };
    if let Err(error) = release() {
        tracing::debug!(%error, "releasing Escape");
    }
}

/// Every modifier state that counts as one of `modifiers`: X11 matches a
/// grab against the exact state, and a lock that is on is part of it.
fn masks(modifiers: &[ModifierSet], locks: u16) -> Vec<u16> {
    let mut masks = Vec::new();
    for set in modifiers {
        let held = set.iter().fold(0, |mask, modifier| {
            mask | u16::from(match modifier {
                Modifier::Control => ModMask::CONTROL,
                Modifier::Alt => ModMask::M1,
                Modifier::Shift => ModMask::SHIFT,
                Modifier::Meta => ModMask::M4,
            })
        });
        let mut lit = locks;
        loop {
            masks.push(held | lit);
            if lit == 0 {
                break;
            }
            lit = (lit - 1) & locks;
        }
    }
    masks
}

struct Keyboard {
    first: Keycode,
    // the first symbol of each key, from `first` on
    symbols: Vec<u32>,
}

impl Keyboard {
    fn read(connection: &RustConnection) -> anyhow::Result<Self> {
        let setup = connection.setup();
        let first = setup.min_keycode;
        let mapping = connection
            .get_keyboard_mapping(first, setup.max_keycode - first + 1)?
            .reply()?;
        let symbols = mapping
            .keysyms
            .chunks(usize::from(mapping.keysyms_per_keycode).max(1))
            .map(|key| key[0])
            .collect();
        Ok(Self { first, symbols })
    }

    fn keycodes(&self, symbol: u32) -> Vec<Keycode> {
        (self.first..=Keycode::MAX)
            .zip(&self.symbols)
            .filter(|&(_, &found)| found == symbol)
            .map(|(keycode, _)| keycode)
            .collect()
    }

    /// The modifier bits that locks set: Caps Lock's, and those of whichever
    /// modifiers this keyboard gives Num Lock and Scroll Lock.
    fn locks(&self, connection: &RustConnection) -> anyhow::Result<u16> {
        let mapping = connection.get_modifier_mapping()?.reply()?;
        let per_modifier = usize::from(mapping.keycodes_per_modifier()).max(1);
        let lock_keys = [self.keycodes(NUM_LOCK), self.keycodes(SCROLL_LOCK)].concat();
        let mut locks = u16::from(ModMask::LOCK);
        for (modifier, keys) in mapping.keycodes.chunks(per_modifier).enumerate() {
            if keys.iter().any(|key| lock_keys.contains(key)) {
                locks |= 1 << modifier;
            }
        }
        Ok(locks)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use x11rb::protocol::xproto::{
        CreateWindowAux, EventMask, InputFocus, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, WindowClass,
    };
    use x11rb::protocol::xtest::ConnectionExt as _;

    use super::*;

    const CAPS_LOCK: u32 = 0xffe5;
    const ALT: u32 = 0xffe9;
    const SHIFT: u32 = 0xffe1;

    #[test]
    fn masks_cover_every_lock_state_of_each_modifier_set() {
        let lock = u16::from(ModMask::LOCK);
        let num = u16::from(ModMask::M2);
        let alt = u16::from(ModMask::M1);
        let cases = [
            (
                "plain Escape with Caps Lock and Num Lock",
                vec![ModifierSet::EMPTY],
                lock | num,
                vec![lock | num, num, lock, 0],
            ),
            (
                "the shortcut's own modifier is claimed too",
                vec![ModifierSet::EMPTY, ModifierSet::EMPTY.with(Modifier::Alt)],
                lock,
                vec![lock, 0, alt | lock, alt],
            ),
            (
                "every modifier has its own bit",
                vec![
                    ModifierSet::EMPTY
                        .with(Modifier::Control)
                        .with(Modifier::Alt)
                        .with(Modifier::Shift)
                        .with(Modifier::Meta),
                ],
                0,
                vec![u16::from(
                    ModMask::CONTROL | ModMask::M1 | ModMask::SHIFT | ModMask::M4,
                )],
            ),
        ];
        for (name, modifiers, locks, want) in cases {
            assert_eq!(masks(&modifiers, locks), want, "{name}");
        }
    }

    #[derive(Debug, PartialEq)]
    enum Reached {
        Onda,
        Application,
        Neither,
    }

    // a desktop with one focused application, and Onda's count of cancels
    struct Desktop {
        connection: RustConnection,
        root: Window,
        keyboard: Keyboard,
        cancels: Arc<AtomicUsize>,
    }

    impl Desktop {
        fn new() -> Self {
            let (connection, screen) = x11rb::connect(None).unwrap();
            let screen = connection.setup().roots[screen].clone();
            let application = connection.generate_id().unwrap();
            connection
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    application,
                    screen.root,
                    0,
                    0,
                    200,
                    200,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    &CreateWindowAux::new().event_mask(EventMask::KEY_PRESS),
                )
                .unwrap();
            connection.map_window(application).unwrap();
            connection
                .set_input_focus(InputFocus::POINTER_ROOT, application, x11rb::CURRENT_TIME)
                .unwrap();
            connection.sync().unwrap();
            let keyboard = Keyboard::read(&connection).unwrap();
            Self {
                connection,
                root: screen.root,
                keyboard,
                cancels: Arc::default(),
            }
        }

        fn claim(&self, modifiers: &[ModifierSet]) -> anyhow::Result<Claim> {
            let cancels = self.cancels.clone();
            Claim::new(modifiers, move || {
                cancels.fetch_add(1, Ordering::SeqCst);
            })
        }

        fn key(&self, symbol: u32) -> Keycode {
            self.keyboard.keycodes(symbol)[0]
        }

        fn tap(&self, held: &[u32], symbol: u32) {
            let fake = |kind, symbol| {
                self.connection
                    .xtest_fake_input(
                        kind,
                        self.key(symbol),
                        x11rb::CURRENT_TIME,
                        self.root,
                        0,
                        0,
                        0,
                    )
                    .unwrap();
            };
            held.iter().for_each(|&held| fake(KEY_PRESS_EVENT, held));
            fake(KEY_PRESS_EVENT, symbol);
            fake(KEY_RELEASE_EVENT, symbol);
            held.iter().for_each(|&held| fake(KEY_RELEASE_EVENT, held));
            self.connection.sync().unwrap();
        }

        // who gets Escape when the user presses it with `held` down
        fn escape(&self, held: &[u32]) -> Reached {
            let before = self.cancels.load(Ordering::SeqCst);
            self.tap(held, ESCAPE);
            let escape = self.key(ESCAPE);
            let mut reached = false;
            while let Some(event) = self.connection.poll_for_event().unwrap() {
                reached |= matches!(event, Event::KeyPress(press) if press.detail == escape);
            }
            if reached {
                return Reached::Application;
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if self.cancels.load(Ordering::SeqCst) > before {
                    return Reached::Onda;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Reached::Neither
        }

        fn lock(&self, symbol: u32, bit: u16, on: bool) {
            let lit = || {
                let pointer = self.connection.query_pointer(self.root).unwrap();
                u16::from(pointer.reply().unwrap().mask) & bit != 0
            };
            if lit() != on {
                self.tap(&[], symbol);
            }
            assert_eq!(lit(), on, "lock {symbol:#x}");
        }
    }

    #[test]
    #[ignore = "needs an X11 display of its own; run scripts/test-linux-gui.sh"]
    fn x11_escape_is_claimed_only_while_the_claim_lives() {
        let desktop = Desktop::new();
        let plain = [ModifierSet::EMPTY];
        let with_alt = [ModifierSet::EMPTY, ModifierSet::EMPTY.with(Modifier::Alt)];
        assert_eq!(
            desktop.escape(&[]),
            Reached::Application,
            "before a session"
        );

        let claim = desktop.claim(&with_alt).unwrap();
        assert_eq!(desktop.escape(&[]), Reached::Onda);
        assert_eq!(desktop.escape(&[ALT]), Reached::Onda);
        assert_eq!(
            desktop.escape(&[SHIFT]),
            Reached::Application,
            "a combination outside the claim"
        );
        let caps = u16::from(ModMask::LOCK);
        let num = desktop.keyboard.locks(&desktop.connection).unwrap() & !caps;
        assert_ne!(num, 0, "Num Lock is a modifier");
        desktop.lock(NUM_LOCK, num, true);
        assert_eq!(desktop.escape(&[]), Reached::Onda, "Num Lock on");
        desktop.lock(CAPS_LOCK, caps, true);
        assert_eq!(desktop.escape(&[]), Reached::Onda, "both locks on");
        assert_eq!(desktop.escape(&[ALT]), Reached::Onda, "both locks on");
        desktop.lock(NUM_LOCK, num, false);
        assert_eq!(desktop.escape(&[]), Reached::Onda, "Caps Lock on");
        desktop.lock(CAPS_LOCK, caps, false);
        assert!(
            desktop.claim(&plain).is_err(),
            "nothing is left for a second claim"
        );
        drop(claim);
        assert_eq!(desktop.escape(&[]), Reached::Application, "after a session");
        assert_eq!(desktop.escape(&[ALT]), Reached::Application);

        // a window manager that keeps Alt+Escape for itself
        let (rival, _) = x11rb::connect(None).unwrap();
        rival
            .grab_key(
                false,
                desktop.root,
                ModMask::M1,
                desktop.key(ESCAPE),
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .unwrap()
            .check()
            .unwrap();
        let claim = desktop.claim(&with_alt).unwrap();
        assert_eq!(desktop.escape(&[]), Reached::Onda, "beside a rival grab");
        assert_eq!(desktop.escape(&[ALT]), Reached::Neither, "the rival's");
        drop(claim);
        assert_eq!(desktop.escape(&[]), Reached::Application);
    }
}
