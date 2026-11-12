//! Persistent post-processing widgets preserve focus and in-flight key edits.

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use onda_app::views::{ModelsNotice, PostProcessingPage, ProvidersDialog};
use onda_domain::cleanup::{CleanupProvider, custom_model_slug};

use super::{Window, clear, label, page, scrolled};

pub(super) struct Page {
    pub widget: gtk::Box,
    enabled: adw::SwitchRow,
    details: gtk::Box,
    model: adw::ActionRow,
    caption: gtk::Label,
    test: gtk::Button,
    busy: gtk::Spinner,
    result: gtk::Label,
    error: gtk::Label,
    dismiss_error: gtk::Button,
    updating: Cell<bool>,
    chooser: RefCell<Option<Rc<Chooser>>>,
}

impl Page {
    pub fn new() -> Self {
        let widget = page();
        let group = adw::PreferencesGroup::builder()
            .title("Post-processing")
            .description("Clean up transcripts before they are pasted or copied.")
            .build();
        let enabled = adw::SwitchRow::builder()
            .title("Post-process transcripts")
            .build();
        group.add(&enabled);
        widget.append(&group);
        let details = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let model = adw::ActionRow::builder()
            .title("Model")
            .use_markup(false)
            .activatable(true)
            .build();
        model.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        group.add(&model);
        let caption = label("");
        caption.set_xalign(0.0);
        details.append(&caption);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let test = gtk::Button::with_label("Test with Sample Text");
        let busy = gtk::Spinner::new();
        actions.append(&test);
        actions.append(&busy);
        details.append(&actions);
        let result = label("");
        result.set_selectable(true);
        result.set_xalign(0.0);
        details.append(&result);
        let error = error_label();
        details.append(&error);
        let dismiss_error = gtk::Button::with_label("Dismiss error");
        dismiss_error.set_halign(gtk::Align::Start);
        details.append(&dismiss_error);
        widget.append(&details);
        Self {
            widget,
            enabled,
            details,
            model,
            caption,
            test,
            busy,
            result,
            error,
            dismiss_error,
            updating: Cell::new(false),
            chooser: RefCell::new(None),
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.enabled.connect_active_notify(move |row| {
            if let Some(owner) = weak.upgrade()
                && !owner.post_processing.updating.get()
            {
                owner.core.app.set_cleanup_enabled(row.is_active());
            }
        });
        let weak = Rc::downgrade(owner);
        self.model.connect_activated(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.open_providers_dialog();
            }
        });
        let weak = Rc::downgrade(owner);
        self.test.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.test_cleanup();
            }
        });
        let weak = Rc::downgrade(owner);
        self.dismiss_error.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.dismiss_cleanup_test_error();
            }
        });
    }

    pub fn update(&self, owner: &Rc<Window>, view: PostProcessingPage) {
        self.updating.set(true);
        self.enabled.set_active(view.enabled.on);
        self.enabled.set_sensitive(view.enabled.enabled);
        self.updating.set(false);
        self.details.set_visible(view.details.is_some());
        self.model.set_visible(view.details.is_some());
        let dialog = if let Some(details) = view.details {
            self.model.set_subtitle(&format!(
                "{} · {}",
                details.model_name, details.provider_name
            ));
            self.caption.set_text(
                &details
                    .caption
                    .replace("system keychain", "desktop keyring"),
            );
            self.caption.add_css_class("dim-label");
            self.test.set_sensitive(details.test_button.enabled);
            self.busy.set_spinning(details.test_running);
            self.busy.set_visible(details.test_running);
            self.result.set_visible(details.test_result.is_some());
            self.result.set_text(
                &details
                    .test_result
                    .map(|result| format!("{}\n\n{}", result.said, result.result))
                    .unwrap_or_default(),
            );
            self.error.set_text(
                &details
                    .test_error
                    .as_ref()
                    .map(|error| format!("{}: {}", error.title, error.message))
                    .unwrap_or_default(),
            );
            self.error.set_visible(details.test_error.is_some());
            self.dismiss_error.set_visible(details.test_error.is_some());
            details.dialog
        } else {
            None
        };
        if let Some(view) = dialog {
            let chooser = self
                .chooser
                .borrow_mut()
                .get_or_insert_with(|| Chooser::new(owner))
                .clone();
            chooser.update(view);
            if !chooser.presented.replace(true) {
                chooser.dialog.present(Some(&owner.window));
            }
        } else if let Some(chooser) = self.chooser.borrow_mut().take() {
            chooser.dialog.close();
        }
    }
}

struct Chooser {
    dialog: adw::Dialog,
    provider: adw::ComboRow,
    key: adw::PasswordEntryRow,
    key_busy: gtk::Spinner,
    key_error: gtk::Label,
    status: gtk::Label,
    search: gtk::SearchEntry,
    refresh: gtk::Button,
    busy: gtk::Spinner,
    models_notice: gtk::Label,
    models: gtk::Box,
    view: RefCell<Option<ProvidersDialog>>,
    key_revision: RefCell<Option<(String, u64)>>,
    updating: Cell<bool>,
    presented: Cell<bool>,
    owner: std::rc::Weak<Window>,
}

impl Chooser {
    fn new(owner: &Rc<Window>) -> Rc<Self> {
        let dialog = adw::Dialog::builder()
            .title("Choose Model")
            .content_width(700)
            .content_height(660)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let done = gtk::Button::with_label("Done");
        header.pack_end(&done);
        toolbar.add_top_bar(&header);
        let content = page();
        content.set_spacing(12);
        let group = adw::PreferencesGroup::new();
        let provider = adw::ComboRow::builder().title("Provider").build();
        group.add(&provider);
        let key = adw::PasswordEntryRow::builder()
            .title("API key")
            .show_apply_button(true)
            .build();
        let key_busy = gtk::Spinner::new();
        key.add_prefix(&key_busy);
        group.add(&key);
        content.append(&group);
        let key_error = error_label();
        content.append(&key_error);
        let status = label("");
        content.append(&status);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search or enter a model ID"));
        content.append(&search);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let models_title = label("Models");
        models_title.add_css_class("heading");
        models_title.set_xalign(0.0);
        models_title.set_hexpand(true);
        actions.append(&models_title);
        let refresh = gtk::Button::with_label("Refresh Models");
        let busy = gtk::Spinner::new();
        actions.append(&refresh);
        actions.append(&busy);
        content.append(&actions);
        let models_notice = label("");
        models_notice.set_xalign(0.0);
        models_notice.set_selectable(true);
        content.append(&models_notice);
        let models = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let model_scroll = scrolled(&models);
        model_scroll.set_vexpand(true);
        content.append(&model_scroll);
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));
        let this = Rc::new(Self {
            dialog,
            provider,
            key,
            key_busy,
            key_error,
            status,
            search,
            refresh,
            busy,
            models_notice,
            models,
            view: RefCell::new(None),
            key_revision: RefCell::new(None),
            updating: Cell::new(false),
            presented: Cell::new(false),
            owner: Rc::downgrade(owner),
        });
        let weak = Rc::downgrade(owner);
        this.dialog.connect_closed(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.close_providers_dialog();
            }
        });
        let weak = Rc::downgrade(&this);
        done.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.dialog.close();
            }
        });
        let weak = Rc::downgrade(&this);
        this.provider.connect_selected_notify(move |row| {
            if let Some(this) = weak.upgrade()
                && !this.updating.get()
                && let Some(view) = this.view.borrow().as_ref()
                && let Some(option) = view.providers.get(row.selected() as usize)
                && let Some(provider) = CleanupProvider::from_id(&option.id)
                && let Some(owner) = this.owner.upgrade()
            {
                owner.core.app.select_dialog_provider(provider);
            }
        });
        let weak = Rc::downgrade(&this);
        this.key.connect_changed(move |row| {
            if let Some(this) = weak.upgrade()
                && !this.updating.get()
                && let Some(owner) = this.owner.upgrade()
            {
                owner.core.app.api_key_edited(row.text().to_string());
            }
        });
        let weak = Rc::downgrade(owner);
        this.key.connect_apply(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.api_key_commit();
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(owner);
        focus.connect_leave(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.api_key_commit();
            }
        });
        this.key.add_controller(focus);
        let weak = Rc::downgrade(owner);
        this.refresh.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.api_key_commit();
                owner.core.app.refresh_cleanup_models();
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.render_models();
            }
        });
        this
    }

    fn update(&self, view: ProvidersDialog) {
        self.updating.set(true);
        let provider_changed = self
            .view
            .borrow()
            .as_ref()
            .is_none_or(|previous| previous.selected_provider != view.selected_provider);
        if self
            .view
            .borrow()
            .as_ref()
            .is_none_or(|previous| previous.providers != view.providers)
        {
            let names: Vec<&str> = view
                .providers
                .iter()
                .map(|provider| provider.name.as_str())
                .collect();
            self.provider.set_model(Some(&gtk::StringList::new(&names)));
        }
        self.provider.set_selected(
            view.providers
                .iter()
                .position(|provider| provider.id == view.selected_provider)
                .unwrap_or(0) as u32,
        );
        self.key.set_visible(view.key.is_some());
        if let Some(key) = &view.key {
            self.key.set_title(&key.placeholder);
            let revision = (view.selected_provider.clone(), key.revision);
            if self.key_revision.borrow().as_ref() != Some(&revision) {
                self.key.set_text(if provider_changed && key.loading {
                    ""
                } else {
                    &key.text
                });
                *self.key_revision.borrow_mut() = Some(revision);
            }
            self.key_busy.set_spinning(key.loading);
            self.key_busy.set_visible(key.loading);
            self.key_error
                .set_text(key.error.as_deref().unwrap_or_default());
            self.key_error.set_visible(key.error.is_some());
        } else {
            self.key.set_text("");
            self.key_error.set_visible(false);
        }
        self.status
            .set_text(view.status_message.as_deref().unwrap_or_default());
        self.status.set_visible(view.status_message.is_some());
        // a missing key is a request; only a failed listing reads as an error
        let (notice, failed) = match &view.models_notice {
            Some(ModelsNotice::NeedsKey { text }) => (text.as_str(), false),
            Some(ModelsNotice::Failed { text }) => (text.as_str(), true),
            None => ("", false),
        };
        self.models_notice.set_text(notice);
        self.models_notice.set_visible(view.models_notice.is_some());
        if failed {
            self.models_notice.remove_css_class("dim-label");
            self.models_notice.add_css_class("error");
        } else {
            self.models_notice.remove_css_class("error");
            self.models_notice.add_css_class("dim-label");
        }
        self.refresh.set_visible(view.refresh_models.is_some());
        self.refresh.set_sensitive(
            view.refresh_models
                .as_ref()
                .is_some_and(|button| button.enabled),
        );
        self.search.set_visible(view.refresh_models.is_some());
        self.busy.set_spinning(view.models_loading);
        self.busy.set_visible(view.models_loading);
        if provider_changed {
            self.search.set_text("");
        }
        *self.view.borrow_mut() = Some(view);
        self.updating.set(false);
        self.render_models();
    }

    fn render_models(&self) {
        if self.updating.get() {
            return;
        }
        clear(&self.models);
        let view = self.view.borrow();
        let Some(view) = view.as_ref() else {
            return;
        };
        let Some(provider) = CleanupProvider::from_id(&view.selected_provider) else {
            return;
        };
        let query = self.search.text();
        let query = query.trim();
        for group in &view.model_groups {
            let matching: Vec<_> = group
                .models
                .iter()
                .filter(|model| matches_model(&model.name, &model.slug, query))
                .collect();
            if matching.is_empty() {
                continue;
            }
            let rows = adw::PreferencesGroup::builder().title(&group.title).build();
            for model in matching {
                let row = self.model_row(provider, &model.slug, &model.name, &model.slug);
                if view.selected_model.as_deref() == Some(&model.slug) {
                    row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
                }
                rows.add(&row);
            }
            self.models.append(&rows);
        }
        if let Some(slug) = custom_model_slug(query)
            && !view
                .model_groups
                .iter()
                .flat_map(|group| &group.models)
                .any(|model| model.slug == slug)
        {
            let rows = adw::PreferencesGroup::new();
            rows.add(&self.model_row(provider, &slug, &slug, &view.use_id_caption));
            self.models.append(&rows);
        }
    }

    fn model_row(
        &self,
        provider: CleanupProvider,
        slug: &str,
        title: &str,
        subtitle: &str,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .use_markup(false)
            .activatable(true)
            .build();
        let owner = self.owner.clone();
        let slug = slug.to_owned();
        row.connect_activated(move |_| {
            if let Some(owner) = owner.upgrade() {
                owner.core.app.select_cleanup_model(provider, &slug);
            }
        });
        row
    }
}

fn matches_model(name: &str, slug: &str, query: &str) -> bool {
    let query = query.to_lowercase();
    name.to_lowercase().contains(&query) || slug.to_lowercase().contains(&query)
}

fn error_label() -> gtk::Label {
    let label = label("");
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.add_css_class("error");
    label
}
