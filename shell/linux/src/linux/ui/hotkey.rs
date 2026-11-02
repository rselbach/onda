use super::{Window, page, parse_shortcut};
use adw::prelude::*;
use onda_app::views::HotkeyPage;
use onda_domain::types::ActivationMode;
use std::{cell::Cell, rc::Rc};

pub(super) struct Page {
    pub widget: gtk::Box,
    entry: adw::EntryRow,
    status: adw::ActionRow,
    reset: gtk::Button,
    activation: adw::ComboRow,
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
        let entry = adw::EntryRow::builder()
            .title("Press to talk")
            .show_apply_button(true)
            .build();
        group.add(&entry);
        let status = adw::ActionRow::builder()
            .title("Desktop shortcut")
            .use_markup(false)
            .build();
        group.add(&status);
        let reset = gtk::Button::with_label("Reset to Ctrl+Alt+Space");
        reset.set_halign(gtk::Align::Start);
        reset.set_margin_top(8);
        group.add(&reset);
        widget.append(&group);
        let group = adw::PreferencesGroup::builder().title("Activation").build();
        let names: Vec<_> = ActivationMode::ALL
            .iter()
            .map(|mode| mode.display_name())
            .collect();
        let activation = adw::ComboRow::builder()
            .title("Activation")
            .model(&gtk::StringList::new(&names))
            .build();
        group.add(&activation);
        widget.append(&group);
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
            activation,
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
                owner.hotkey.entry.set_text("Ctrl+Alt+Space");
                owner
                    .core
                    .app
                    .set_hotkey(parse_shortcut("Ctrl+Alt+Space").expect("valid default shortcut"));
            }
        });
        let weak = Rc::downgrade(owner);
        self.activation.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.hotkey.updating.get()
                && let Some(mode) = ActivationMode::ALL.get(row.selected() as usize)
            {
                owner.core.app.set_activation_mode(*mode);
            }
        });
    }

    pub fn set_status(&self, status: &str) {
        self.status.set_subtitle(status);
    }

    pub fn update(&self, owner: &Window, view: HotkeyPage) {
        self.updating.set(true);
        if !self.initialized.replace(true) {
            self.entry.set_text(
                &super::super::hotkeys::trigger(&owner.core.settings.hotkey()).unwrap_or_default(),
            );
        }
        self.activation.set_selected(
            ActivationMode::ALL
                .iter()
                .position(|mode| *mode == view.activation)
                .unwrap_or(0) as u32,
        );
        self.activation.set_subtitle(&view.activation_description);
        self.updating.set(false);
    }
}
