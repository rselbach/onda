use super::{Window, clear, label};
use adw::prelude::*;
use onda_app::views::{ModelRowControl, SpeechPage};
use onda_domain::{models::ModelId, routing::NoteActionKind};
use std::rc::Rc;

impl Window {
    pub(super) fn speech(self: &Rc<Self>, page: SpeechPage) {
        clear(&self.models);
        let group = adw::PreferencesGroup::builder()
            .title("Transcription language")
            .build();
        let names: Vec<_> = page
            .languages
            .iter()
            .map(|language| language.name.as_str())
            .collect();
        let language = adw::ComboRow::builder()
            .title(&page.language_label)
            .model(&gtk::StringList::new(&names))
            .build();
        language.set_selected(
            page.languages
                .iter()
                .position(|language| language.code == page.selected_language)
                .unwrap_or(0) as u32,
        );
        let languages = page.languages;
        let weak = Rc::downgrade(self);
        language.connect_selected_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && let Some(language) = languages.get(row.selected() as usize)
                && let Some(language) =
                    onda_domain::languages::LanguageSetting::parse(&language.code)
            {
                owner.core.app.select_language(language);
            }
        });
        if let Some(note) = page.compatibility {
            language.set_subtitle(&note.text);
            if note.warning {
                language.add_css_class("warning");
            }
            if let Some(action) = note.action {
                let button = gtk::Button::with_label(&action.title);
                button.set_sensitive(action.enabled);
                button.set_valign(gtk::Align::Center);
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(owner) = weak.upgrade()
                        && let Some(id) = ModelId::parse(&action.model_id)
                    {
                        match action.kind {
                            NoteActionKind::Use => owner.core.app.select_model(id),
                            NoteActionKind::DownloadAndUse => owner.core.app.download_and_use(id),
                            NoteActionKind::Downloading => {}
                        }
                    }
                });
                language.add_suffix(&button);
            }
        }
        group.add(&language);
        self.models.append(&group);
        let group = adw::PreferencesGroup::builder()
            .title("Speech models")
            .description(
                "Transcription runs on this computer. Download a model once to use it offline.",
            )
            .build();
        for model in page.models {
            let Some(id) = ModelId::parse(&model.id) else {
                continue;
            };
            let mut subtitle = format!("{} · {}", model.size_hint, model.description);
            if let Some(note) = model.unsupported {
                subtitle.push_str(&format!("\n{note}"));
            }
            let row = adw::ActionRow::builder()
                .title(&model.name)
                .subtitle(&subtitle)
                .use_markup(false)
                .build();
            if let Some(recommended) = model.recommended {
                let badge = gtk::Label::new(Some(&recommended));
                badge.add_css_class("caption");
                badge.add_css_class("accent");
                row.add_suffix(&badge);
            }
            let download = matches!(model.control, ModelRowControl::DownloadAndUse { .. });
            match model.control {
                ModelRowControl::Selected => {
                    let icon = gtk::Image::from_icon_name("object-select-symbolic");
                    icon.add_css_class("accent");
                    icon.set_tooltip_text(Some("Selected model"));
                    row.add_suffix(&icon);
                }
                ModelRowControl::Select { title } | ModelRowControl::DownloadAndUse { title } => {
                    let button = gtk::Button::with_label(&title);
                    button.set_valign(gtk::Align::Center);
                    let weak = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        if let Some(owner) = weak.upgrade() {
                            if let Err(error) = owner.core.finish_setup() {
                                owner.toast.add_toast(adw::Toast::new(&error.to_string()));
                                return;
                            }
                            if download {
                                owner.core.app.download_and_use(id);
                            } else {
                                owner.core.app.select_model(id);
                            }
                        }
                    });
                    row.add_suffix(&button);
                }
                ModelRowControl::Downloading {
                    progress,
                    percent,
                    cancel_title,
                } => {
                    let progress = gtk::ProgressBar::builder()
                        .fraction(progress)
                        .text(&percent)
                        .show_text(true)
                        .valign(gtk::Align::Center)
                        .width_request(100)
                        .build();
                    row.add_suffix(&progress);
                    let button = gtk::Button::with_label(&cancel_title);
                    button.set_valign(gtk::Align::Center);
                    let weak = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        if let Some(owner) = weak.upgrade() {
                            owner.core.app.cancel_download(id);
                        }
                    });
                    row.add_suffix(&button);
                }
            }
            if let Some(tooltip) = model.delete_tooltip {
                let delete = gtk::Button::from_icon_name("user-trash-symbolic");
                delete.set_tooltip_text(Some(&tooltip));
                delete.add_css_class("flat");
                delete.set_valign(gtk::Align::Center);
                let weak = Rc::downgrade(self);
                delete.connect_clicked(move |_| {
                    if let Some(owner) = weak.upgrade() {
                        owner.core.app.request_delete_model(id);
                    }
                });
                row.add_suffix(&delete);
            }
            group.add(&row);
        }
        self.models.append(&group);
        let note = label("Linux currently uses CPU inference. Live preview is unavailable.");
        note.add_css_class("dim-label");
        note.set_xalign(0.0);
        self.models.append(&note);
        let retry = gtk::Button::with_label("Retry loading model");
        retry.set_halign(gtk::Align::Start);
        let weak = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.retry_model_load();
            }
        });
        self.models.append(&retry);
        if let Some(alert) = page.alert
            && self.speech_alert.get() != Some(alert.id)
        {
            self.speech_alert.set(Some(alert.id));
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
            let weak = Rc::downgrade(self);
            dialog.connect_response(None, move |_, response| {
                if let Some(owner) = weak.upgrade() {
                    owner.speech_alert.set(None);
                    if response == "confirm" {
                        owner.core.app.confirm_speech_alert(alert.id);
                    } else {
                        owner.core.app.dismiss_speech_alert(alert.id);
                    }
                }
            });
            dialog.present(Some(&self.window));
        }
    }
}
