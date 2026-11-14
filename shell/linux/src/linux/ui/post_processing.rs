//! Persistent post-processing widgets preserve focus and in-flight key edits.

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use onda_app::views::{ModelsNotice, PostProcessingPage, PromptEditorView, ProvidersDialog};
use onda_domain::cleanup::{CleanupProvider, custom_model_slug};

use super::{Window, clear, label, page, scrolled};

pub(super) struct Page {
    pub widget: gtk::Box,
    enabled: adw::SwitchRow,
    details: gtk::Box,
    model: adw::ActionRow,
    caption: gtk::Label,
    prompt: adw::PreferencesGroup,
    prompt_row: adw::ActionRow,
    prompt_preview: gtk::Label,
    test: gtk::Button,
    busy: gtk::Spinner,
    prompt_status: gtk::Label,
    result: gtk::Label,
    error: gtk::Label,
    dismiss_error: gtk::Button,
    updating: Cell<bool>,
    chooser: RefCell<Option<Rc<Chooser>>>,
    editor: RefCell<Option<Rc<Editor>>>,
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
        let prompt = adw::PreferencesGroup::new();
        let prompt_row = adw::ActionRow::builder()
            .use_markup(false)
            .activatable(true)
            .build();
        prompt_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        prompt.add(&prompt_row);
        let prompt_preview = gtk::Label::builder()
            .wrap(true)
            .lines(3)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .xalign(0.0)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        prompt_preview.add_css_class("caption");
        prompt_preview.add_css_class("dim-label");
        prompt.add(
            &adw::PreferencesRow::builder()
                .activatable(false)
                .child(&prompt_preview)
                .build(),
        );
        details.append(&prompt);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let test = gtk::Button::with_label("Test with Sample Text");
        let busy = gtk::Spinner::new();
        actions.append(&test);
        actions.append(&busy);
        details.append(&actions);
        let prompt_status = dim_label();
        details.append(&prompt_status);
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
            prompt,
            prompt_row,
            prompt_preview,
            test,
            busy,
            prompt_status,
            result,
            error,
            dismiss_error,
            updating: Cell::new(false),
            chooser: RefCell::new(None),
            editor: RefCell::new(None),
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
        self.prompt_row.connect_activated(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.open_prompt_editor();
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
        let (dialog, editor) = if let Some(details) = view.details {
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
            let prompt = details.prompt;
            // a group's title and description are Pango markup
            self.prompt
                .set_title(&gtk::glib::markup_escape_text(&prompt.title));
            self.prompt
                .set_description(Some(&gtk::glib::markup_escape_text(&prompt.caption)));
            self.prompt_row.set_title(&prompt.name);
            self.prompt_row.set_subtitle(&prompt.subtitle);
            self.prompt_preview.set_text(&prompt.preview);
            self.prompt_status
                .set_text(prompt.status.as_deref().unwrap_or_default());
            self.prompt_status.set_visible(prompt.status.is_some());
            (details.dialog, prompt.editor)
        } else {
            (None, None)
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
            close(&chooser.dialog);
        }
        if let Some(view) = editor {
            let editor = self
                .editor
                .borrow_mut()
                .get_or_insert_with(|| Editor::new(owner))
                .clone();
            editor.update(view);
            if !editor.presented.replace(true) {
                editor.dialog.present(Some(&owner.window));
            }
        } else if let Some(editor) = self.editor.borrow_mut().take() {
            close(&editor.dialog);
        }
    }
}

struct Chooser {
    dialog: adw::Dialog,
    provider: adw::ComboRow,
    key: adw::PasswordEntryRow,
    key_busy: gtk::Spinner,
    get_key: gtk::LinkButton,
    key_error: gtk::Label,
    status: gtk::Label,
    search: gtk::SearchEntry,
    refresh: gtk::Button,
    busy: gtk::Spinner,
    models_notice: gtk::Label,
    models: gtk::Box,
    in_use: gtk::Label,
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
        let provider = adw::ComboRow::builder()
            .title("Provider")
            .use_markup(false)
            .build();
        group.add(&provider);
        let key = adw::PasswordEntryRow::builder()
            .title("API key")
            .show_apply_button(true)
            .build();
        let key_busy = gtk::Spinner::new();
        key.add_prefix(&key_busy);
        group.add(&key);
        content.append(&group);
        let get_key = gtk::LinkButton::new("");
        get_key.set_halign(gtk::Align::End);
        content.append(&get_key);
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
        let in_use = dim_label();
        in_use.set_wrap(false);
        in_use.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        in_use.set_margin_top(10);
        in_use.set_margin_bottom(10);
        in_use.set_margin_start(28);
        in_use.set_margin_end(28);
        toolbar.add_bottom_bar(&in_use);
        dialog.set_child(Some(&toolbar));
        let this = Rc::new(Self {
            dialog,
            provider,
            key,
            key_busy,
            get_key,
            key_error,
            status,
            search,
            refresh,
            busy,
            models_notice,
            models,
            in_use,
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
        self.provider.set_subtitle(&view.privacy_note);
        self.in_use.set_text(&format!(
            "{}: {} · {}",
            view.in_use_label, view.in_use_model, view.in_use_provider
        ));
        self.key.set_visible(view.key.is_some());
        self.get_key.set_visible(view.key.is_some());
        if let Some(key) = &view.key {
            self.key.set_title(&key.placeholder);
            self.get_key.set_label(&key.get_key_title);
            self.get_key.set_uri(&key.get_key_url);
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

// The prompt sheet. The core owns the draft; this forwards typing and takes
// the core's text only when the core replaces the draft.
struct Editor {
    dialog: adw::Dialog,
    intro: gtk::Label,
    label: gtk::Label,
    badge: gtk::Label,
    text: gtk::TextView,
    status: gtk::Label,
    character_count: gtk::Label,
    help: gtk::Expander,
    help_text: gtk::Label,
    restore: gtk::Button,
    cancel: gtk::Button,
    save: gtk::Button,
    revision: Cell<Option<u64>>,
    updating: Cell<bool>,
    presented: Cell<bool>,
}

impl Editor {
    fn new(owner: &Rc<Window>) -> Rc<Self> {
        let dialog = adw::Dialog::builder()
            .content_width(640)
            .content_height(640)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        let cancel = gtk::Button::new();
        header.pack_start(&cancel);
        let save = gtk::Button::new();
        save.add_css_class("suggested-action");
        header.pack_end(&save);
        toolbar.add_top_bar(&header);
        let content = page();
        content.set_spacing(12);
        let intro = dim_label();
        content.append(&intro);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        label.add_css_class("heading");
        heading.append(&label);
        let badge = gtk::Label::new(None);
        badge.add_css_class("caption");
        badge.add_css_class("dim-label");
        heading.append(&badge);
        content.append(&heading);
        let text = gtk::TextView::builder()
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();
        text.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
        content.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .has_frame(true)
                .vexpand(true)
                .child(&text)
                .build(),
        );
        let counts = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let status = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build();
        status.add_css_class("caption");
        counts.append(&status);
        let character_count = gtk::Label::builder().valign(gtk::Align::Start).build();
        for class in ["caption", "dim-label", "numeric"] {
            character_count.add_css_class(class);
        }
        counts.append(&character_count);
        content.append(&counts);
        let help = gtk::Expander::new(None);
        let help_text = dim_label();
        help_text.set_margin_top(6);
        help.set_child(Some(&help_text));
        content.append(&help);
        let restore = gtk::Button::new();
        restore.set_halign(gtk::Align::Start);
        content.append(&restore);
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));
        dialog.set_focus(Some(&text));
        let this = Rc::new(Self {
            dialog,
            intro,
            label,
            badge,
            text,
            status,
            character_count,
            help,
            help_text,
            restore,
            cancel,
            save,
            revision: Cell::new(None),
            updating: Cell::new(false),
            presented: Cell::new(false),
        });
        // Escape, Cancel, and the core closing the sheet all end here; the
        // core has already dropped a saved draft
        let weak = Rc::downgrade(owner);
        this.dialog.connect_closed(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.cancel_prompt_editor();
            }
        });
        let weak = Rc::downgrade(&this);
        this.cancel.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.dialog.close();
            }
        });
        let weak = Rc::downgrade(&this);
        let owner_weak = Rc::downgrade(owner);
        this.text.buffer().connect_changed(move |buffer| {
            if let Some(this) = weak.upgrade()
                && !this.updating.get()
                && let Some(owner) = owner_weak.upgrade()
            {
                let (start, end) = buffer.bounds();
                owner
                    .core
                    .app
                    .prompt_edited(buffer.text(&start, &end, true).to_string());
            }
        });
        let weak = Rc::downgrade(owner);
        this.restore.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.restore_default_prompt();
            }
        });
        let weak = Rc::downgrade(owner);
        this.save.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.save_prompt();
            }
        });
        this
    }

    fn update(&self, view: PromptEditorView) {
        self.dialog.set_title(&view.title);
        self.intro.set_text(&view.intro);
        self.label.set_text(&view.label);
        self.badge.set_text(&view.badge);
        if self.revision.replace(Some(view.revision)) != Some(view.revision) {
            let buffer = self.text.buffer();
            self.updating.set(true);
            buffer.set_text(&view.text);
            self.updating.set(false);
            buffer.place_cursor(&buffer.start_iter());
        }
        self.status.set_text(&view.status);
        let (shown, hidden) = match view.status_is_error {
            true => ("error", "dim-label"),
            false => ("dim-label", "error"),
        };
        self.status.remove_css_class(hidden);
        self.status.add_css_class(shown);
        self.character_count.set_text(&view.character_count);
        self.help.set_label(Some(&view.help_title));
        self.help_text.set_text(&view.help);
        self.restore.set_label(&view.restore.title);
        self.restore.set_sensitive(view.restore.enabled);
        self.cancel.set_label(&view.cancel_title);
        self.save.set_label(&view.save.title);
        self.save.set_sensitive(view.save.enabled);
    }
}

// The core's view drops a sheet the user has already closed. libadwaita logs
// a critical when asked to close a dialog that is no longer presented.
fn close(dialog: &adw::Dialog) {
    if dialog.parent().is_some() {
        dialog.close();
    }
}

fn matches_model(name: &str, slug: &str, query: &str) -> bool {
    let query = query.to_lowercase();
    name.to_lowercase().contains(&query) || slug.to_lowercase().contains(&query)
}

fn dim_label() -> gtk::Label {
    let label = label("");
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    label
}

fn error_label() -> gtk::Label {
    let label = label("");
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.add_css_class("error");
    label
}
