use super::{Window, page};
use adw::prelude::*;
use onda_app::views::{GeneralPage, InputOption, OutputModeOption};
use onda_domain::{types::OutputMode, updates::UpdateAction};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct Page {
    pub widget: gtk::Box,
    inputs: adw::ComboRow,
    input_options: RefCell<Vec<InputOption>>,
    output: adw::ComboRow,
    output_options: RefCell<Vec<OutputModeOption>>,
    terminal: adw::SwitchRow,
    media: adw::SwitchRow,
    sounds: adw::SwitchRow,
    launch: adw::SwitchRow,
    automatic: adw::SwitchRow,
    update_row: adw::ActionRow,
    update_button: gtk::Button,
    update_action: RefCell<UpdateAction>,
    last_checked: adw::ActionRow,
    updating: Cell<bool>,
}

impl Page {
    pub fn new(terminal_paste_shortcut: bool) -> Self {
        let widget = page();

        let group = adw::PreferencesGroup::builder().title("Microphone").build();
        let inputs = adw::ComboRow::builder().use_markup(false).build();
        group.add(&inputs);
        widget.append(&group);

        let group = adw::PreferencesGroup::builder().title("Output").build();
        let output = adw::ComboRow::builder().use_markup(false).build();
        group.add(&output);
        let terminal = adw::SwitchRow::builder()
            .title("Terminal paste shortcut")
            .subtitle("Paste with Ctrl+Shift+V instead of Ctrl+V.")
            .use_markup(false)
            .active(terminal_paste_shortcut)
            .build();
        group.add(&terminal);
        widget.append(&group);

        let group = adw::PreferencesGroup::builder().title("Media").build();
        let media = adw::SwitchRow::builder().use_markup(false).build();
        group.add(&media);
        widget.append(&group);

        let group = adw::PreferencesGroup::builder().title("Feedback").build();
        let sounds = adw::SwitchRow::builder().use_markup(false).build();
        group.add(&sounds);
        widget.append(&group);

        let group = adw::PreferencesGroup::builder().title("Startup").build();
        let launch = adw::SwitchRow::builder().use_markup(false).build();
        group.add(&launch);
        widget.append(&group);

        let group = adw::PreferencesGroup::builder()
            .title("Updates")
            .description("Download the matching package, then install it with your package manager or the archive installer.")
            .build();
        let automatic = adw::SwitchRow::builder().use_markup(false).build();
        group.add(&automatic);
        let update_row = adw::ActionRow::builder().use_markup(false).build();
        let update_button = gtk::Button::builder().valign(gtk::Align::Center).build();
        update_row.add_suffix(&update_button);
        group.add(&update_row);
        let last_checked = adw::ActionRow::builder()
            .title("Last checked")
            .use_markup(false)
            .visible(false)
            .build();
        group.add(&last_checked);
        widget.append(&group);

        Self {
            widget,
            inputs,
            input_options: RefCell::default(),
            output,
            output_options: RefCell::default(),
            terminal,
            media,
            sounds,
            launch,
            automatic,
            update_row,
            update_button,
            update_action: RefCell::new(UpdateAction::Check),
            last_checked,
            updating: Cell::new(false),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.inputs.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
                && let Some(device) = owner
                    .general
                    .input_options
                    .borrow()
                    .get(row.selected() as usize)
            {
                owner.core.app.select_input_device(&device.uid);
            }
        });

        let weak = Rc::downgrade(owner);
        self.output.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
                && let Some(option) = owner
                    .general
                    .output_options
                    .borrow()
                    .get(row.selected() as usize)
            {
                owner.core.app.set_output_mode(option.mode);
            }
        });

        let weak = Rc::downgrade(owner);
        self.terminal.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
                && let Err(error) = owner
                    .core
                    .settings
                    .set_terminal_paste_shortcut(row.is_active())
            {
                tracing::error!(%error, "could not save the terminal paste shortcut");
                owner.general.updating.set(true);
                row.set_active(owner.core.settings.terminal_paste_shortcut());
                owner.general.updating.set(false);
            }
        });

        let weak = Rc::downgrade(owner);
        self.media.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
            {
                owner.core.app.set_pause_media(row.is_active());
            }
        });

        let weak = Rc::downgrade(owner);
        self.sounds.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
            {
                owner.core.app.set_sounds(row.is_active());
            }
        });

        let weak = Rc::downgrade(owner);
        self.launch.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
            {
                owner.core.app.set_launch_at_login(row.is_active());
            }
        });

        let weak = Rc::downgrade(owner);
        self.automatic.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.general.updating.get()
            {
                owner.core.app.set_automatic_update_checks(row.is_active());
            }
        });

        let weak = Rc::downgrade(owner);
        self.update_button.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner
                    .core
                    .app
                    .update_action(*owner.general.update_action.borrow());
            }
        });
    }

    pub fn update(&self, owner: &Rc<Window>, page: GeneralPage) {
        self.updating.set(true);

        self.inputs.set_title(&page.input.label);
        if *self.input_options.borrow() != page.input.options {
            let names: Vec<_> = page
                .input
                .options
                .iter()
                .map(|device| device.name.as_str())
                .collect();
            self.inputs.set_model(Some(&gtk::StringList::new(&names)));
            *self.input_options.borrow_mut() = page.input.options.clone();
        }
        self.inputs.set_selected(
            page.input
                .options
                .iter()
                .position(|device| device.uid == page.input.selected_uid)
                .unwrap_or(0) as u32,
        );
        if let Some(warning) = page.input.warning {
            self.inputs.set_subtitle(&warning);
            self.inputs.add_css_class("warning");
        } else {
            self.inputs
                .set_subtitle(page.input.caption.as_deref().unwrap_or(""));
            self.inputs.remove_css_class("warning");
        }

        self.output.set_title(&page.output.label);
        if *self.output_options.borrow() != page.output.options {
            let names: Vec<_> = page
                .output
                .options
                .iter()
                .map(|option| option.title.as_str())
                .collect();
            self.output.set_model(Some(&gtk::StringList::new(&names)));
            *self.output_options.borrow_mut() = page.output.options.clone();
        }
        self.output.set_selected(
            page.output
                .options
                .iter()
                .position(|option| option.mode == page.output.selected)
                .unwrap_or(0) as u32,
        );
        self.output.set_subtitle(
            page.output
                .caption
                .as_deref()
                .unwrap_or("Your desktop may request keyboard access on the first paste."),
        );

        self.terminal
            .set_active(owner.core.settings.terminal_paste_shortcut());
        self.terminal
            .set_sensitive(page.output.selected == OutputMode::Type);

        self.media.set_title(&page.pause_media.label);
        self.media.set_subtitle(&page.pause_media_caption);
        self.media.set_active(page.pause_media.on);
        self.media.set_sensitive(page.pause_media.enabled);

        self.sounds.set_title(&page.sounds.label);
        self.sounds.set_active(page.sounds.on);

        self.launch.set_title(&page.launch_at_login.label);
        self.launch.set_active(page.launch_at_login.on);
        self.launch.set_sensitive(page.launch_at_login.enabled);
        self.launch.set_subtitle(
            page.launch_at_login_caption
                .as_ref()
                .map_or("", |caption| caption.text.as_str()),
        );

        self.automatic.set_title(&page.automatic_updates.label);
        self.automatic.set_active(page.automatic_updates.on);
        self.automatic.set_sensitive(page.automatic_updates.enabled);

        self.update_row.set_title(&page.version_footer);
        if page.update.action == UpdateAction::Download {
            self.update_row.set_subtitle(
                "An update is available. Install the new package, then restart Onda.",
            );
        } else {
            self.update_row.set_subtitle(
                page.update_caption
                    .as_ref()
                    .map_or("", |caption| caption.text.as_str()),
            );
        }
        self.update_button.set_label(&page.update.title);
        self.update_button.set_sensitive(page.update.enabled);
        *self.update_action.borrow_mut() = page.update.action;

        if let Some(last) = page.last_update_check
            && let Ok(since) = last.duration_since(std::time::UNIX_EPOCH)
            && let Ok(date) = gtk::glib::DateTime::from_unix_local(since.as_secs() as i64)
            && let Ok(date) = date.format("%x %X")
        {
            self.last_checked.set_subtitle(date.as_str());
            self.last_checked.set_visible(true);
        } else {
            self.last_checked.set_visible(false);
        }

        self.updating.set(false);
    }
}
