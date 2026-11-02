use super::{Window, clear};
use adw::prelude::*;
use onda_app::views::GeneralPage;
use onda_domain::types::OutputMode;
use std::{rc::Rc, sync::atomic::Ordering};

impl Window {
    pub(super) fn settings(self: &Rc<Self>, page: GeneralPage) {
        clear(&self.general);
        let group = adw::PreferencesGroup::builder().title("Microphone").build();
        let names: Vec<_> = page
            .input
            .options
            .iter()
            .map(|device| device.name.as_str())
            .collect();
        let inputs = adw::ComboRow::builder()
            .title(&page.input.label)
            .use_markup(false)
            .model(&gtk::StringList::new(&names))
            .build();
        inputs.set_selected(
            page.input
                .options
                .iter()
                .position(|device| device.uid == page.input.selected_uid)
                .unwrap_or(0) as u32,
        );
        if let Some(warning) = page.input.warning {
            inputs.set_subtitle(&warning);
            inputs.add_css_class("warning");
        } else if let Some(caption) = page.input.caption {
            inputs.set_subtitle(&caption.replace("in macOS", "in your desktop settings"));
        }
        let devices = page.input.options;
        let weak = Rc::downgrade(self);
        inputs.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && let Some(device) = devices.get(row.selected() as usize)
            {
                owner.core.app.select_input_device(&device.uid);
            }
        });
        group.add(&inputs);
        self.general.append(&group);

        let group = adw::PreferencesGroup::builder().title("Output").build();
        let output = adw::ComboRow::builder()
            .title(&page.output.label)
            .model(&gtk::StringList::new(&[
                "Type into active app",
                "Copy to clipboard",
            ]))
            .selected(u32::from(page.output.selected == OutputMode::Clipboard))
            .build();
        output.set_subtitle(
            page.output
                .caption
                .as_deref()
                .unwrap_or("Your desktop may request keyboard access on the first paste."),
        );
        let weak = Rc::downgrade(self);
        output.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.set_output_mode(if row.selected() == 1 {
                    OutputMode::Clipboard
                } else {
                    OutputMode::Type
                });
            }
        });
        group.add(&output);
        let terminal = adw::SwitchRow::builder()
            .title("Terminal paste shortcut")
            .subtitle("Use Ctrl+Shift+V instead of Ctrl+V for this session.")
            .active(self.core.output.terminal.load(Ordering::Relaxed))
            .sensitive(page.output.selected == OutputMode::Type)
            .build();
        let weak = Rc::downgrade(self);
        terminal.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade() {
                owner
                    .core
                    .output
                    .terminal
                    .store(row.is_active(), Ordering::Relaxed);
            }
        });
        group.add(&terminal);
        self.general.append(&group);

        let group = adw::PreferencesGroup::builder().title("Feedback").build();
        let sounds = adw::SwitchRow::builder()
            .title(&page.sounds.label)
            .active(page.sounds.on)
            .sensitive(super::super::process::available("canberra-gtk-play"))
            .build();
        if !sounds.is_sensitive() {
            sounds.set_subtitle("Install libcanberra-gtk3 to enable sound feedback.");
        }
        let weak = Rc::downgrade(self);
        sounds.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.set_sounds(row.is_active());
            }
        });
        group.add(&sounds);
        self.general.append(&group);

        let group = adw::PreferencesGroup::builder().title("Startup").build();
        let launch = adw::SwitchRow::builder()
            .title(&page.launch_at_login.label)
            .active(page.launch_at_login.on)
            .sensitive(page.launch_at_login.enabled)
            .build();
        if let Some(caption) = page.launch_at_login_caption {
            launch.set_subtitle(&caption.text);
        }
        let weak = Rc::downgrade(self);
        launch.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.set_launch_at_login(row.is_active());
            }
        });
        group.add(&launch);
        self.general.append(&group);
    }
}
