use super::{Window, label, page, recorder};
use adw::prelude::*;
use onda_app::views::HotkeyPage;
use onda_domain::{hotkey::HotkeyView, types::ActivationMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// What the core's footer leaves to the shell: the keys a Linux desktop
/// never lets a window see, and the shortcut it may bind instead.
const DESKTOP_NOTE: &str = "Your desktop keeps some combinations for itself, such as many Super \
                            shortcuts, so Onda cannot record them. It may also assign a \
                            different shortcut, shown under Desktop shortcut.";

pub(super) struct Page {
    pub widget: gtk::Box,
    press_to_talk: adw::ActionRow,
    keycaps: gtk::Box,
    recorder: recorder::Controls,
    status: adw::ActionRow,
    reset: gtk::Button,
    footer: gtk::Label,
    activation_group: adw::PreferencesGroup,
    activation: adw::ComboRow,
    // the picker's modes in row order, as the core last sent them
    activation_modes: RefCell<Vec<ActivationMode>>,
    updating: Cell<bool>,
}

impl Page {
    pub fn new() -> Self {
        let widget = page();
        let group = adw::PreferencesGroup::builder().title("Shortcut").build();
        let press_to_talk = adw::ActionRow::new();
        let keycaps = recorder::keycaps();
        press_to_talk.add_suffix(&keycaps);
        group.add(&press_to_talk);
        let status = adw::ActionRow::builder()
            .title("Desktop shortcut")
            .use_markup(false)
            .build();
        group.add(&status);
        let recorder = recorder::Controls::new();
        recorder.widget.set_margin_top(12);
        group.add(&recorder.widget);
        let reset = gtk::Button::new();
        reset.set_halign(gtk::Align::Start);
        reset.set_margin_top(8);
        group.add(&reset);
        let footer = label("");
        footer.set_xalign(0.0);
        footer.set_margin_top(12);
        footer.add_css_class("dim-label");
        group.add(&footer);
        widget.append(&group);
        let activation_group = adw::PreferencesGroup::new();
        let activation = adw::ComboRow::new();
        activation_group.add(&activation);
        widget.append(&activation_group);
        let group = adw::PreferencesGroup::builder()
            .title("Desktop integration")
            .build();
        group.add(&adw::ActionRow::builder().title("Custom desktop bindings")
            .subtitle("If global shortcuts are unavailable, bind onda --toggle in your desktop keyboard settings. Separate --start and --stop commands support press and release bindings. Cancel with onda --cancel.").build());
        widget.append(&group);
        Self {
            widget,
            press_to_talk,
            keycaps,
            recorder,
            status,
            reset,
            footer,
            activation_group,
            activation,
            activation_modes: RefCell::default(),
            updating: Cell::new(false),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        self.recorder.connect(owner);
        let weak = Rc::downgrade(owner);
        self.reset.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.reset_hotkey();
            }
        });
        let weak = Rc::downgrade(owner);
        self.activation.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.hotkey.updating.get()
                && let Some(mode) = owner
                    .hotkey
                    .activation_modes
                    .borrow()
                    .get(row.selected() as usize)
                    .copied()
            {
                owner.core.app.set_activation_mode(mode);
            }
        });
    }

    pub fn set_status(&self, status: &str) {
        self.status.set_subtitle(status);
    }

    /// The shortcut and its recorder, from `HotkeyChanged`.
    pub fn render(&self, view: &HotkeyView) {
        recorder::set_keycaps(&self.keycaps, &view.keycaps);
        self.recorder.render(view);
    }

    pub fn update(&self, view: HotkeyPage) {
        self.updating.set(true);
        self.press_to_talk.set_title(&view.press_to_talk_label);
        self.reset.set_label(&view.reset.title);
        self.reset.set_sensitive(view.reset.enabled);
        self.footer
            .set_text(&format!("{} {DESKTOP_NOTE}", view.footer));
        self.activation_group.set_title(&view.activation_label);
        self.activation.set_title(&view.activation_label);
        let modes: Vec<ActivationMode> = view
            .activation_options
            .iter()
            .map(|option| option.mode)
            .collect();
        if *self.activation_modes.borrow() != modes {
            let titles: Vec<&str> = view
                .activation_options
                .iter()
                .map(|option| option.title.as_str())
                .collect();
            self.activation
                .set_model(Some(&gtk::StringList::new(&titles)));
            *self.activation_modes.borrow_mut() = modes;
        }
        if let Some(selected) = self
            .activation_modes
            .borrow()
            .iter()
            .position(|mode| *mode == view.activation)
        {
            self.activation.set_selected(selected as u32);
        }
        self.activation.set_subtitle(&view.activation_description);
        self.updating.set(false);
    }
}

#[cfg(test)]
mod tests {
    use adw::prelude::*;
    use onda_domain::hotkey::{KeyCode, KeyNames, Modifier, Shortcut};
    use onda_domain::platform::Platform;
    use x11rb::{
        connection::Connection,
        protocol::{
            xproto::{ConnectionExt, GrabMode, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, ModMask},
            xtest::ConnectionExt as _,
        },
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    use std::time::{Duration, Instant};

    use super::super::IDLE_HINT;
    use super::super::integration_tests::{
        Start, button, focus_onda, launch, settle, text_visible, wait_for, widgets,
    };

    // hardware keycodes, which name a key's place whatever the layout
    const ESCAPE: u8 = 9;
    const CONTROL_LEFT: u8 = 37;
    const KEY_Q: u8 = 24;
    const KEY_A: u8 = 38;
    const KEY_G: u8 = 42;
    const ALT_LEFT: u8 = 64;
    const SPACE: u8 = 65;

    /// Another application that wants a key with Ctrl and Alt.
    struct Rival {
        connection: RustConnection,
        root: u32,
    }

    impl Rival {
        fn connect() -> Self {
            let (connection, screen) = x11rb::connect(None).unwrap();
            let root = connection.setup().roots[screen].root;
            Self { connection, root }
        }

        // the X server grants a key to one application at a time
        fn grab(&self, key: u8) -> bool {
            self.connection
                .grab_key(
                    false,
                    self.root,
                    ModMask::CONTROL | ModMask::M1,
                    key,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .unwrap()
                .check()
                .is_ok()
        }

        fn release(&self, key: u8) {
            self.connection
                .ungrab_key(key, self.root, ModMask::CONTROL | ModMask::M1)
                .unwrap()
                .check()
                .unwrap();
        }
    }

    /// The user's hands: key presses that reach Onda's window as a real
    /// keyboard's do.
    struct Keyboard {
        connection: RustConnection,
        root: u32,
    }

    impl Keyboard {
        fn connect() -> Self {
            let (connection, screen) = x11rb::connect(None).unwrap();
            let root = connection.setup().roots[screen].root;
            Self { connection, root }
        }

        fn send(&self, kind: u8, keys: &[u8]) {
            for &key in keys {
                self.connection
                    .xtest_fake_input(kind, key, x11rb::CURRENT_TIME, self.root, 0, 0, 0)
                    .unwrap();
            }
            self.connection.sync().unwrap();
        }

        /// Swaps what two keys type, as a layout other than the US one does.
        fn swap(&self, a: u8, b: u8) {
            let typed = |key| {
                self.connection
                    .get_keyboard_mapping(key, 1)
                    .unwrap()
                    .reply()
                    .unwrap()
            };
            let (first, second) = (typed(a), typed(b));
            for (key, mapping) in [(a, second), (b, first)] {
                self.connection
                    .change_keyboard_mapping(1, key, mapping.keysyms_per_keycode, &mapping.keysyms)
                    .unwrap();
            }
            self.connection.sync().unwrap();
        }

        fn press(&self, keys: &[u8]) {
            self.send(KEY_PRESS_EVENT, keys);
        }

        fn release(&self, keys: &[u8]) {
            self.send(KEY_RELEASE_EVENT, keys);
        }

        fn tap(&self, held: &[u8], key: u8) {
            self.press(held);
            self.press(&[key]);
            self.release(&[key]);
            self.release(held);
        }
    }

    fn keycaps(page: &gtk::Box) -> Vec<String> {
        widgets::<gtk::Label>(page)
            .iter()
            .filter(|label| label.has_css_class("keycap"))
            .map(|label| label.text().into())
            .collect()
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_a_taken_shortcut_is_reported_until_it_registers() {
        let rival = Rival::connect();
        assert!(
            rival.grab(SPACE),
            "the shortcut must be free before the test"
        );
        let owner = launch(Start::Declined);
        wait_for(|| {
            text_visible(
                &owner.window,
                "Hotkey Registration Failed: Could not register the global hotkey \
                 (Ctrl+Alt+Space). It may be in use by another application.",
            )
        });
        let status = owner.hotkey.status.subtitle().unwrap_or_default();
        assert!(
            status.starts_with("Global shortcut unavailable:")
                && status.ends_with("Bind onda --toggle in your desktop keyboard settings."),
            "{status}"
        );
        assert_eq!(owner.getting_started.shortcut(), status);

        rival.release(SPACE);
        owner.core.app.set_hotkey(Shortcut::CONTROL_ALT_SPACE);
        wait_for(|| owner.hotkey.status.subtitle().as_deref() == Some("Control+Alt+Space"));
        assert_eq!(owner.getting_started.shortcut(), "Control+Alt+Space");
        assert!(!rival.grab(SPACE), "Onda must hold the shortcut now");
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_a_pressed_shortcut_is_recorded_and_registered() {
        let owner = launch(Start::Declined);
        // a declined model opens the Speech page once the app is up
        wait_for(|| owner.stack.visible_child_name().as_deref() == Some("speech"));
        owner.stack.set_visible_child_name("hotkey");
        let page = &owner.hotkey.widget;
        let default = Platform::linux().default_shortcut;
        let saved = || owner.core.settings.hotkey(default);
        let status = || owner.hotkey.status.subtitle().unwrap_or_default();
        wait_for(|| status() == "Control+Alt+Space");
        assert_eq!(keycaps(page), ["Ctrl", "Alt", "Space"]);
        assert!(text_visible(page, "Current shortcut: Ctrl+Alt+Space"));
        assert!(text_visible(page, "like a gremlin. Your desktop keeps"));
        let record = button(page, "Record Shortcut");
        let cancel = button(page, "Cancel");
        let recording = || record.label().as_deref() == Some("Press shortcut...");
        let idle = || record.label().as_deref() == Some("Record Shortcut");
        assert!(!cancel.is_visible());
        let keyboard = Keyboard::connect();
        focus_onda();

        record.grab_focus();
        record.emit_clicked();
        wait_for(recording);
        assert!(cancel.is_visible());
        assert!(text_visible(page, KeyNames::LINUX.recording_help));

        keyboard.tap(&[], SPACE);
        wait_for(|| text_visible(page, KeyNames::LINUX.missing_modifier_message));
        settle();
        assert!(
            recording(),
            "Space must reach the recorder, not the focused record button"
        );
        assert_eq!(saved(), *default);

        keyboard.tap(&[], ESCAPE);
        wait_for(idle);
        assert!(!cancel.is_visible());
        assert!(!text_visible(
            page,
            KeyNames::LINUX.missing_modifier_message
        ));
        assert_eq!(saved(), *default);

        keyboard.tap(&[], SPACE);
        wait_for(recording);
        owner.stack.set_visible_child_name("general");
        wait_for(idle);
        owner.stack.set_visible_child_name("hotkey");

        record.emit_clicked();
        wait_for(recording);
        owner.window.close();
        wait_for(idle);
        owner.present();
        focus_onda();

        record.emit_clicked();
        wait_for(recording);
        cancel.emit_clicked();
        wait_for(idle);

        record.emit_clicked();
        wait_for(recording);
        keyboard.press(&[CONTROL_LEFT, ALT_LEFT, KEY_G]);
        let control_alt_g = Shortcut::new(
            KeyCode::parse("KeyG").unwrap(),
            [Modifier::Control, Modifier::Alt].into_iter().collect(),
        )
        .unwrap();
        wait_for(|| saved() == control_alt_g);
        let repeating = Instant::now() + Duration::from_millis(1500);
        wait_for(|| Instant::now() >= repeating);
        assert_eq!(
            owner.getting_started.detail(),
            IDLE_HINT,
            "keys held past the recording must not start a dictation"
        );
        keyboard.release(&[KEY_G, ALT_LEFT, CONTROL_LEFT]);
        wait_for(|| status() == "Control+Alt+KeyG");
        assert!(idle());
        assert_eq!(keycaps(page), ["Ctrl", "Alt", "G"]);
        assert!(text_visible(page, "Current shortcut: Ctrl+Alt+G"));
        let rival = Rival::connect();
        assert!(!rival.grab(KEY_G), "Onda must hold the recorded shortcut");
        assert!(rival.grab(SPACE), "Onda must have let go of the old one");
        rival.release(SPACE);

        keyboard.swap(KEY_Q, KEY_A);
        record.emit_clicked();
        wait_for(recording);
        keyboard.tap(&[CONTROL_LEFT, ALT_LEFT], KEY_Q);
        wait_for(|| status() == "Control+Alt+KeyQ");
        assert!(
            !rival.grab(KEY_Q),
            "Onda must hold the key that was pressed, whatever it types"
        );
        assert!(rival.grab(KEY_A), "the key that now types Q is not it");
        rival.release(KEY_A);
        keyboard.swap(KEY_Q, KEY_A);

        let reset = button(page, "Reset to Ctrl+Alt+Space");
        wait_for(|| reset.is_sensitive());
        reset.emit_clicked();
        wait_for(|| saved() == *default);
        wait_for(|| !reset.is_sensitive() && keycaps(page) == ["Ctrl", "Alt", "Space"]);
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }
}
