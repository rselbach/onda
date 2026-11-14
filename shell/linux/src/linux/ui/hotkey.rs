use super::{Window, page, parse_shortcut};
use adw::prelude::*;
use onda_app::views::HotkeyPage;
use onda_domain::types::ActivationMode;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct Page {
    pub widget: gtk::Box,
    entry: adw::EntryRow,
    status: adw::ActionRow,
    reset: gtk::Button,
    activation_group: adw::PreferencesGroup,
    activation: adw::ComboRow,
    // the picker's modes in row order, as the core last sent them
    activation_modes: RefCell<Vec<ActivationMode>>,
    updating: Cell<bool>,
    initialized: Cell<bool>,
}

impl Page {
    pub fn new() -> Self {
        let widget = page();
        let group = adw::PreferencesGroup::builder()
            .title("Shortcut")
            .description(
                "Use a key with Ctrl, Alt, or Super. Your desktop may assign a different shortcut.",
            )
            .build();
        let entry = adw::EntryRow::builder().show_apply_button(true).build();
        group.add(&entry);
        let status = adw::ActionRow::builder()
            .title("Desktop shortcut")
            .use_markup(false)
            .build();
        group.add(&status);
        let reset = gtk::Button::new();
        reset.set_halign(gtk::Align::Start);
        reset.set_margin_top(8);
        group.add(&reset);
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
            entry,
            status,
            reset,
            activation_group,
            activation,
            activation_modes: RefCell::default(),
            updating: Cell::new(false),
            initialized: Cell::new(false),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.entry.connect_apply(move |entry| {
            if let Some(owner) = weak.upgrade() {
                match parse_shortcut(&entry.text()) {
                    Ok(shortcut) => owner.core.app.set_hotkey(shortcut),
                    Err(error) => owner.toast.add_toast(adw::Toast::new(&error.to_string())),
                }
            }
        });
        let weak = Rc::downgrade(owner);
        self.reset.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                let platform = owner.core.platform;
                owner
                    .hotkey
                    .entry
                    .set_text(&platform.default_shortcut.display(platform.key_names));
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

    pub fn update(&self, owner: &Window, view: HotkeyPage) {
        self.updating.set(true);
        if !self.initialized.replace(true) {
            let hotkey = owner
                .core
                .settings
                .hotkey(owner.core.platform.default_shortcut);
            self.entry
                .set_text(&super::super::hotkeys::trigger(&hotkey).unwrap_or_default());
        }
        self.reset.set_label(&view.reset.title);
        self.reset.set_sensitive(view.reset.enabled);
        self.entry.set_title(&view.press_to_talk_label);
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
    use onda_domain::hotkey::Shortcut;
    use x11rb::{
        connection::Connection,
        protocol::xproto::{ConnectionExt, GrabMode, ModMask},
        rust_connection::RustConnection,
    };

    use super::super::integration_tests::{Start, launch, text_visible, wait_for};

    const SPACE_KEYSYM: u32 = 0x20;

    /// Another application that wants Ctrl+Alt+Space, the default shortcut.
    struct Rival {
        connection: RustConnection,
        root: u32,
        space: u8,
    }

    impl Rival {
        fn connect() -> Self {
            let (connection, screen) = x11rb::connect(None).unwrap();
            let setup = connection.setup();
            let root = setup.roots[screen].root;
            let first = setup.min_keycode;
            let mapping = connection
                .get_keyboard_mapping(first, setup.max_keycode - first + 1)
                .unwrap()
                .reply()
                .unwrap();
            let space = mapping
                .keysyms
                .chunks(mapping.keysyms_per_keycode as usize)
                .position(|keysyms| keysyms.contains(&SPACE_KEYSYM))
                .unwrap();
            Self {
                connection,
                root,
                space: first + space as u8,
            }
        }

        // the X server grants a key to one application at a time
        fn grab(&self) -> bool {
            self.connection
                .grab_key(
                    false,
                    self.root,
                    ModMask::CONTROL | ModMask::M1,
                    self.space,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .unwrap()
                .check()
                .is_ok()
        }

        fn release(&self) {
            self.connection
                .ungrab_key(self.space, self.root, ModMask::CONTROL | ModMask::M1)
                .unwrap()
                .check()
                .unwrap();
        }
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_a_taken_shortcut_is_reported_until_it_registers() {
        let rival = Rival::connect();
        assert!(rival.grab(), "the shortcut must be free before the test");
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
        assert_eq!(owner.shortcut.text(), status);

        rival.release();
        owner.core.app.set_hotkey(Shortcut::CONTROL_ALT_SPACE);
        wait_for(|| owner.hotkey.status.subtitle().as_deref() == Some("Control+Alt+Space"));
        assert_eq!(owner.shortcut.text(), "Control+Alt+Space");
        assert!(!rival.grab(), "Onda must hold the shortcut now");
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }
}
