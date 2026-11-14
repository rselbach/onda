use super::{Window, clear, label, page};
use adw::prelude::*;
use gtk::{gio, glib};
use onda_app::{
    App,
    views::{DayHeading, HistoryContent, HistoryMenuItem, HistoryPage},
};
use onda_domain::{history::ContextMenuItem, types::HistoryRetention};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::UNIX_EPOCH,
};
use uuid::Uuid;

pub(super) struct Page {
    pub widget: gtk::Box,
    save: adw::SwitchRow,
    retention: adw::ComboRow,
    audio: adw::SwitchRow,
    search: gtk::SearchEntry,
    clear: gtk::Button,
    entries: gtk::Box,
    updating: Cell<bool>,
    confirming: Cell<bool>,
    showing_error: Cell<bool>,
    menu_open: Cell<bool>,
    // rebuilding the rows would close the open row menu, so a view that
    // arrives meanwhile waits here until the menu closes
    deferred: RefCell<Option<HistoryPage>>,
}

// the core's title names the macOS file manager
const SHOW_RECORDING: &str = "Show Recording in Folder";

const RETENTION: [HistoryRetention; 3] = [
    HistoryRetention::Week,
    HistoryRetention::Month,
    HistoryRetention::Forever,
];

impl Page {
    pub fn new() -> Self {
        let widget = page();
        let group = adw::PreferencesGroup::builder()
            .title("History settings")
            .description("Saved only on this computer.")
            .build();
        let save = adw::SwitchRow::builder().title("Save transcripts").build();
        let retention = adw::ComboRow::builder()
            .title("Keep for")
            .model(&gtk::StringList::new(&["1 Week", "1 Month", "Forever"]))
            .build();
        let audio = adw::SwitchRow::builder()
            .title("Keep audio recordings")
            .build();
        group.add(&save);
        group.add(&retention);
        group.add(&audio);
        widget.append(&group);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search transcripts")
            .hexpand(true)
            .build();
        toolbar.append(&search);
        let clear = gtk::Button::from_icon_name("user-trash-symbolic");
        clear.set_tooltip_text(Some("Clear History…"));
        toolbar.append(&clear);
        widget.append(&toolbar);
        let entries = gtk::Box::new(gtk::Orientation::Vertical, 24);
        widget.append(&entries);
        Self {
            widget,
            save,
            retention,
            audio,
            search,
            clear,
            entries,
            updating: Cell::new(false),
            confirming: Cell::new(false),
            showing_error: Cell::new(false),
            menu_open: Cell::new(false),
            deferred: RefCell::new(None),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.save.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.history.updating.get()
            {
                owner.core.app.set_history_enabled(row.is_active());
            }
        });
        let weak = Rc::downgrade(owner);
        self.audio.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.history.updating.get()
            {
                owner.core.app.set_history_keeps_audio(row.is_active());
            }
        });
        let weak = Rc::downgrade(owner);
        self.retention.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.history.updating.get()
                && let Some(retention) = RETENTION.get(row.selected() as usize)
            {
                owner.core.app.set_history_retention(*retention);
            }
        });
        let weak = Rc::downgrade(owner);
        self.search.connect_search_changed(move |entry| {
            if let Some(owner) = weak.upgrade()
                && !owner.history.updating.get()
            {
                owner.core.app.search_history(entry.text().as_str());
            }
        });
        let weak = Rc::downgrade(owner);
        self.clear.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.request_clear_history();
            }
        });
    }

    pub fn update(&self, owner: &Rc<Window>, view: HistoryPage) {
        if self.menu_open.get() {
            self.deferred.replace(Some(view));
            return;
        }
        self.deferred.take();
        self.updating.set(true);
        self.save.set_active(view.save_transcripts.on);
        self.audio.set_active(view.keep_audio.on);
        self.audio.set_sensitive(view.keep_audio.enabled);
        self.retention.set_sensitive(view.retention_enabled);
        self.retention.set_selected(
            RETENTION
                .iter()
                .position(|value| *value == view.retention)
                .unwrap_or(1) as u32,
        );
        self.clear.set_sensitive(view.clear_button.enabled);
        self.updating.set(false);
        clear(&self.entries);
        match view.content {
            HistoryContent::Sections { sections } => {
                for section in sections {
                    let group = adw::PreferencesGroup::builder()
                        .title(day_title(section.heading))
                        .build();
                    for entry in section.rows {
                        let Ok(id) = Uuid::parse_str(&entry.id) else {
                            continue;
                        };
                        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
                        for set in [
                            gtk::Widget::set_margin_top,
                            gtk::Widget::set_margin_bottom,
                            gtk::Widget::set_margin_start,
                            gtk::Widget::set_margin_end,
                        ] {
                            set(content.upcast_ref(), 14);
                        }
                        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        let time = entry
                            .time
                            .duration_since(UNIX_EPOCH)
                            .ok()
                            .and_then(|time| {
                                gtk::glib::DateTime::from_unix_local(time.as_secs() as i64).ok()
                            })
                            .and_then(|time| time.format("%H:%M").ok())
                            .unwrap_or_default();
                        let metadata = label(&format!("{time} · {}", entry.details));
                        metadata.set_xalign(0.0);
                        metadata.set_hexpand(true);
                        metadata.add_css_class("dim-label");
                        metadata.add_css_class("caption");
                        top.append(&metadata);
                        let copy = gtk::Button::from_icon_name(
                            if view.copied_entry.as_deref() == Some(&entry.id) {
                                "object-select-symbolic"
                            } else {
                                "edit-copy-symbolic"
                            },
                        );
                        copy.set_tooltip_text(Some("Copy transcript"));
                        copy.add_css_class("flat");
                        let weak = Rc::downgrade(owner);
                        copy.connect_clicked(move |_| {
                            if let Some(owner) = weak.upgrade() {
                                owner.core.app.copy_history_entry(id);
                            }
                        });
                        top.append(&copy);
                        top.append(&row_menu(owner, id, entry.menu));
                        content.append(&top);
                        let text =
                            label(entry.transcript.as_deref().unwrap_or(&view.no_text_label));
                        text.set_xalign(0.0);
                        text.set_selectable(true);
                        content.append(&text);
                        if let Some(original) = entry.original {
                            let text = label(&original);
                            text.set_xalign(0.0);
                            text.set_selectable(true);
                            let expander = gtk::Expander::builder()
                                .label(&view.original_label)
                                .child(&text)
                                .build();
                            content.append(&expander);
                        }
                        group.add(&content);
                    }
                    self.entries.append(&group);
                }
            }
            HistoryContent::Empty {
                title,
                message,
                action,
            } => {
                let empty = adw::StatusPage::builder()
                    .title(&title)
                    .description(&message)
                    .icon_name("document-open-recent-symbolic")
                    .build();
                if let Some(action) = action {
                    let button = gtk::Button::with_label(&action);
                    button.set_halign(gtk::Align::Center);
                    let weak = Rc::downgrade(owner);
                    button.connect_clicked(move |_| {
                        if let Some(owner) = weak.upgrade() {
                            owner.core.app.set_history_enabled(true);
                        }
                    });
                    empty.set_child(Some(&button));
                }
                self.entries.append(&empty);
            }
            HistoryContent::NoSearchResults { query } => {
                self.entries.append(
                    &adw::StatusPage::builder()
                        .title("No matching transcripts")
                        .description(format!("No results for “{query}”."))
                        .icon_name("edit-find-symbolic")
                        .build(),
                );
            }
        }
        if let Some(confirmation) = view.dialog
            && !self.confirming.replace(true)
        {
            let dialog = adw::AlertDialog::builder()
                .heading(&confirmation.title)
                .body(&confirmation.message)
                .build();
            dialog.add_response("cancel", &confirmation.cancel_title);
            dialog.add_response("confirm", &confirmation.confirm_title);
            dialog.set_close_response("cancel");
            dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
            let weak = Rc::downgrade(owner);
            dialog.connect_response(None, move |_, response| {
                if let Some(owner) = weak.upgrade() {
                    owner.history.confirming.set(false);
                    if response == "confirm" {
                        owner.core.app.confirm_history_dialog();
                    } else {
                        owner.core.app.cancel_history_dialog();
                    }
                }
            });
            dialog.present(Some(&owner.window));
        }
        if let Some(error) = view.error
            && !self.showing_error.replace(true)
        {
            let dialog = adw::AlertDialog::builder()
                .heading(&error.title)
                .body(&error.message)
                .build();
            dialog.add_response("close", "OK");
            let weak = Rc::downgrade(owner);
            dialog.connect_response(None, move |_, _| {
                if let Some(owner) = weak.upgrade() {
                    owner.history.showing_error.set(false);
                    owner.core.app.dismiss_history_error();
                }
            });
            dialog.present(Some(&owner.window));
        }
    }
}

type Command = fn(&App, Uuid);

// the action name and core command of a row menu item, `None` for a divider
fn command(item: ContextMenuItem) -> Option<(&'static str, Command)> {
    Some(match item {
        ContextMenuItem::Copy => ("copy", |app, id| app.copy_history_entry(id)),
        ContextMenuItem::CopyOriginal => ("copy-original", |app, id| app.copy_history_original(id)),
        ContextMenuItem::PlayRecording => {
            ("play-recording", |app, id| app.play_history_recording(id))
        }
        ContextMenuItem::ShowRecording => {
            ("show-recording", |app, id| app.show_history_recording(id))
        }
        ContextMenuItem::Delete => ("delete", |app, id| app.delete_history_entry(id)),
        ContextMenuItem::Divider => return None,
    })
}

fn row_menu(owner: &Rc<Window>, id: Uuid, items: Vec<HistoryMenuItem>) -> gtk::MenuButton {
    let actions = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    for entry in items {
        let Some((name, run)) = command(entry.item) else {
            menu.append_section(None, &section);
            section = gio::Menu::new();
            continue;
        };
        let title = match entry.item {
            ContextMenuItem::ShowRecording => SHOW_RECORDING,
            _ => &entry.title,
        };
        section.append(Some(title), Some(&format!("row.{name}")));
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(owner);
        action.connect_activate(move |_, _| {
            if let Some(owner) = weak.upgrade() {
                run(&owner.core.app, id);
            }
        });
        actions.add_action(&action);
    }
    menu.append_section(None, &section);
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More actions")
        .menu_model(&menu)
        .build();
    button.add_css_class("flat");
    button.insert_action_group("row", Some(&actions));
    let weak = Rc::downgrade(owner);
    button.connect_active_notify(move |button| {
        let Some(owner) = weak.upgrade() else {
            return;
        };
        owner.history.menu_open.set(button.is_active());
        if button.is_active() {
            return;
        }
        // this handler runs inside the button that the update removes
        glib::idle_add_local_once(move || {
            if let Some(view) = owner.history.deferred.take() {
                owner.history.update(&owner, view);
            }
        });
    });
    button
}

fn day_title(day: DayHeading) -> String {
    match day {
        DayHeading::Relative { text } => text,
        DayHeading::Date {
            year, month, day, ..
        } => format!("{year}-{month:02}-{day:02}"),
    }
}
