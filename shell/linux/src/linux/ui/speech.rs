use super::{Window, label, page};
use crate::linux::speech::{cpus, preview_note};
use adw::prelude::*;
use onda_app::views::{LanguageOption, ModelRow, ModelRowControl, NoteActionView, SpeechPage};
use onda_domain::{models::ModelId, routing::NoteActionKind};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Copy)]
enum ControlAction {
    Select,
    DownloadAndUse,
}

struct ModelRowWidgets {
    id: ModelId,
    row: adw::ActionRow,
    badge: gtk::Label,
    selected: gtk::Image,
    action: gtk::Button,
    action_kind: Rc<Cell<Option<ControlAction>>>,
    progress: gtk::ProgressBar,
    cancel: gtk::Button,
    delete: gtk::Button,
}

impl ModelRowWidgets {
    fn new(id: ModelId, owner: &Rc<Window>) -> Self {
        let row = adw::ActionRow::builder().use_markup(false).build();
        let badge = gtk::Label::builder()
            .css_classes(["caption", "accent"])
            .visible(false)
            .build();
        row.add_suffix(&badge);

        let selected = gtk::Image::from_icon_name("object-select-symbolic");
        selected.add_css_class("accent");
        selected.set_tooltip_text(Some("Selected model"));
        selected.set_visible(false);
        row.add_suffix(&selected);

        let action_kind = Rc::new(Cell::new(None));
        let action = gtk::Button::builder()
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let weak = Rc::downgrade(owner);
        let kind_cell = action_kind.clone();
        action.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade()
                && let Some(kind) = kind_cell.get()
            {
                match kind {
                    ControlAction::Select => owner.core.app.select_model(id),
                    ControlAction::DownloadAndUse => owner.core.app.download_and_use(id),
                }
            }
        });
        row.add_suffix(&action);

        let progress = gtk::ProgressBar::builder()
            .show_text(true)
            .valign(gtk::Align::Center)
            .width_request(100)
            .visible(false)
            .build();
        row.add_suffix(&progress);

        let cancel = gtk::Button::builder()
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let weak = Rc::downgrade(owner);
        cancel.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.cancel_download(id);
            }
        });
        row.add_suffix(&cancel);

        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.add_css_class("flat");
        delete.set_valign(gtk::Align::Center);
        delete.set_visible(false);
        let weak = Rc::downgrade(owner);
        delete.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.request_delete_model(id);
            }
        });
        row.add_suffix(&delete);

        Self {
            id,
            row,
            badge,
            selected,
            action,
            action_kind,
            progress,
            cancel,
            delete,
        }
    }

    fn update(&self, model: &ModelRow) {
        let mut subtitle = format!("{} · {}", model.size_hint, model.description);
        if let Some(note) = &model.unsupported {
            subtitle.push_str(&format!("\n{note}"));
        }
        self.row.set_title(&model.name);
        self.row.set_subtitle(&subtitle);

        if let Some(recommended) = &model.recommended {
            self.badge.set_text(recommended);
            self.badge.set_visible(true);
        } else {
            self.badge.set_text("");
            self.badge.set_visible(false);
        }

        match &model.control {
            ModelRowControl::Selected => {
                self.selected.set_visible(true);
                self.action_kind.set(None);
                self.action.set_label("");
                self.action.set_visible(false);
                self.progress.set_visible(false);
                self.cancel.set_label("");
                self.cancel.set_visible(false);
            }
            ModelRowControl::Select { title } | ModelRowControl::DownloadAndUse { title } => {
                let is_download = matches!(model.control, ModelRowControl::DownloadAndUse { .. });
                self.selected.set_visible(false);
                self.action_kind.set(Some(if is_download {
                    ControlAction::DownloadAndUse
                } else {
                    ControlAction::Select
                }));
                self.action.set_label(title);
                self.action.set_visible(true);
                self.progress.set_visible(false);
                self.cancel.set_label("");
                self.cancel.set_visible(false);
            }
            ModelRowControl::Downloading {
                progress,
                percent,
                cancel_title,
            } => {
                self.selected.set_visible(false);
                self.action_kind.set(None);
                self.action.set_label("");
                self.action.set_visible(false);
                self.progress.set_fraction(*progress);
                self.progress.set_text(Some(percent));
                self.progress.set_visible(true);
                self.cancel.set_label(cancel_title);
                self.cancel.set_visible(true);
            }
        }

        if let Some(tooltip) = &model.delete_tooltip {
            self.delete.set_tooltip_text(Some(tooltip));
            self.delete.set_visible(true);
        } else {
            self.delete.set_visible(false);
        }
    }
}

pub(super) struct Page {
    pub widget: gtk::Box,
    language: adw::ComboRow,
    languages: RefCell<Vec<LanguageOption>>,
    language_codes: RefCell<Vec<String>>,
    note_button: gtk::Button,
    note_action: RefCell<Option<NoteActionView>>,
    models_group: adw::PreferencesGroup,
    rows: RefCell<Vec<ModelRowWidgets>>,
    retry: gtk::Button,
    updating: Cell<bool>,
    alert: Cell<Option<u64>>,
}

impl Page {
    pub fn new() -> Self {
        let widget = page();
        let language_group = adw::PreferencesGroup::builder()
            .title("Transcription language")
            .build();
        let language = adw::ComboRow::builder()
            .title("Language")
            .use_markup(false)
            .build();
        let note_button = gtk::Button::builder()
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        language.add_suffix(&note_button);
        language_group.add(&language);
        widget.append(&language_group);

        let models_group = adw::PreferencesGroup::builder()
            .title("Speech models")
            .description(
                "Transcription runs on this computer. Download a model once to use it offline.",
            )
            .build();
        widget.append(&models_group);

        let note = label(&preview_note(cpus()));
        note.add_css_class("dim-label");
        note.set_xalign(0.0);
        widget.append(&note);

        let retry = gtk::Button::with_label("Retry loading model");
        retry.set_halign(gtk::Align::Start);
        widget.append(&retry);

        Self {
            widget,
            language,
            languages: RefCell::default(),
            language_codes: RefCell::default(),
            note_button,
            note_action: RefCell::default(),
            models_group,
            rows: RefCell::default(),
            retry,
            updating: Cell::new(false),
            alert: Cell::new(None),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.language.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.speech.updating.get()
                && let Some(language) = owner.speech.languages.borrow().get(row.selected() as usize)
                && let Some(language) =
                    onda_domain::languages::LanguageSetting::parse(&language.code)
            {
                owner.core.app.select_language(language);
            }
        });
        let weak = Rc::downgrade(owner);
        self.note_button.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade()
                && let Some(action) = owner.speech.note_action.borrow().as_ref()
                && let Some(id) = ModelId::parse(&action.model_id)
            {
                match action.kind {
                    NoteActionKind::Use => owner.core.app.select_model(id),
                    NoteActionKind::DownloadAndUse => owner.core.app.download_and_use(id),
                    NoteActionKind::Downloading => {}
                }
            }
        });
        let weak = Rc::downgrade(owner);
        self.retry.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.retry_model_load();
            }
        });
    }

    pub fn update(&self, owner: &Rc<Window>, page: SpeechPage) {
        self.updating.set(true);
        self.language.set_title(&page.language_label);
        let codes: Vec<String> = page
            .languages
            .iter()
            .map(|language| language.code.clone())
            .collect();
        if *self.language_codes.borrow() != codes {
            let names: Vec<_> = page
                .languages
                .iter()
                .map(|language| language.name.as_str())
                .collect();
            self.language.set_model(Some(&gtk::StringList::new(&names)));
            *self.language_codes.borrow_mut() = codes;
        }
        self.language.set_selected(
            page.languages
                .iter()
                .position(|language| language.code == page.selected_language)
                .unwrap_or(0) as u32,
        );
        *self.languages.borrow_mut() = page.languages;

        if let Some(note) = page.compatibility {
            self.language.set_subtitle(&note.text);
            if note.warning {
                self.language.add_css_class("warning");
            } else {
                self.language.remove_css_class("warning");
            }
            if let Some(action) = note.action {
                self.note_button.set_label(&action.title);
                self.note_button.set_sensitive(action.enabled);
                self.note_button.set_visible(true);
                *self.note_action.borrow_mut() = Some(action);
            } else {
                self.note_button.set_label("");
                self.note_button.set_visible(false);
                *self.note_action.borrow_mut() = None;
            }
        } else {
            self.language.set_subtitle("");
            self.language.remove_css_class("warning");
            self.note_button.set_label("");
            self.note_button.set_visible(false);
            *self.note_action.borrow_mut() = None;
        }

        let models: Vec<(ModelId, &ModelRow)> = page
            .models
            .iter()
            .filter_map(|model| ModelId::parse(&model.id).map(|id| (id, model)))
            .collect();
        let mut rows = self.rows.borrow_mut();
        let same_shape = rows.len() == models.len()
            && rows
                .iter()
                .zip(models.iter())
                .all(|(existing, (id, _))| existing.id == *id);
        if !same_shape {
            for existing in rows.drain(..) {
                self.models_group.remove(&existing.row);
            }
            for (id, _) in &models {
                let row = ModelRowWidgets::new(*id, owner);
                self.models_group.add(&row.row);
                rows.push(row);
            }
        }
        for (existing, (_, model)) in rows.iter().zip(models.iter()) {
            existing.update(model);
        }
        drop(rows);
        self.updating.set(false);

        if let Some(alert) = page.alert
            && self.alert.get() != Some(alert.id)
        {
            self.alert.set(Some(alert.id));
            let dialog = adw::AlertDialog::builder()
                .heading(&alert.title)
                .body(&alert.message)
                .build();
            dialog.add_response("cancel", &alert.dismiss);
            dialog.set_close_response("cancel");
            if let Some(confirm) = alert.confirm {
                dialog.add_response("confirm", &confirm);
                dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
            }
            let weak = Rc::downgrade(owner);
            dialog.connect_response(None, move |_, response| {
                if let Some(owner) = weak.upgrade() {
                    owner.speech.alert.set(None);
                    if response == "confirm" {
                        owner.core.app.confirm_speech_alert(alert.id);
                    } else {
                        owner.core.app.dismiss_speech_alert(alert.id);
                    }
                }
            });
            dialog.present(Some(&owner.window));
        }
    }
}

#[cfg(test)]
mod tests {
    use adw::prelude::*;

    use super::super::integration_tests::{Start, button, capture, launch, wait_for, widgets};

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_choosing_japanese_offers_parakeet_japanese() {
        let owner = launch(Start::Declined);
        let row = |title: &str| {
            widgets::<adw::ActionRow>(&owner.speech.widget)
                .into_iter()
                .find(|row| row.title() == title)
        };
        wait_for(|| row("Language").is_some());
        let language: adw::ComboRow = row("Language").unwrap().downcast().unwrap();
        let names: gtk::StringList = language.model().unwrap().downcast().unwrap();
        let japanese = (0..names.n_items())
            .find(|index| names.string(*index).as_deref() == Some("Japanese"))
            .unwrap();
        language.set_selected(japanese);
        let recommended = || {
            row("Parakeet Japanese").is_some_and(|row| {
                widgets::<gtk::Label>(&row)
                    .iter()
                    .any(|label| label.text() == "Recommended")
            })
        };
        wait_for(recommended);
        let japanese = row("Parakeet Japanese").unwrap();
        assert_eq!(
            japanese.subtitle().as_deref(),
            Some("~733MB · NVIDIA Parakeet 0.6B tuned for Japanese")
        );
        button(&japanese, "Download & Use");
        capture(&owner, "speech-japanese");
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }
}
