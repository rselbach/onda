//! The first-run wizard (ONB-1..ONB-13). The core owns the steps and their
//! state. This window renders the `OnboardingView` the core sends and
//! reports what the user does, and its navigation stack follows the step
//! the core names.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use adw::prelude::*;
use gtk::{accessible, glib};
use onda_app::views::{
    LanguageCardView, ModelCardAction, ModelCardView, OnboardingContent, OnboardingView,
    OtherModelsView, PresetCardView,
};
use onda_domain::{
    hotkey::HotkeyPreset, languages::LanguageSetting, models::ModelId, onboarding::OnboardingStep,
};

use super::{Window, try_it::TryIt};

const CONTENT_WIDTH: i32 = 560;

/// The wizard window while it is open.
#[derive(Default)]
pub struct Onboarding {
    open: Rc<RefCell<Option<Wizard>>>,
}

impl Onboarding {
    /// Brings the wizard forward, opening its window unless it is open.
    pub fn open(&self, owner: &Rc<Window>) {
        let window = self
            .open
            .borrow_mut()
            .get_or_insert_with(|| Wizard::new(owner, &self.open))
            .window
            .clone();
        window.present();
    }

    /// Renders the core's view. A view that arrives after the user closed
    /// the window has nothing to render into.
    pub fn update(&self, view: OnboardingView) {
        if let Some(wizard) = &*self.open.borrow() {
            wizard.update(view);
        }
    }

    /// Closes the window for the core, which finished the wizard. The user
    /// closing it is a close request instead, and the core hears of that.
    pub fn close(&self) {
        if let Some(wizard) = self.open.take() {
            wizard.window.destroy();
        }
    }

    #[cfg(test)]
    pub fn window(&self) -> Option<adw::Window> {
        self.open
            .borrow()
            .as_ref()
            .map(|wizard| wizard.window.clone())
    }
}

struct Wizard {
    owner: Weak<Window>,
    window: adw::Window,
    navigation: adw::NavigationView,
    // a page outlives its place in the stack, so going forward again shows
    // what the user left on it
    steps: RefCell<HashMap<OnboardingStep, Step>>,
    // the step the core last named
    step: Rc<Cell<Option<OnboardingStep>>>,
}

impl Wizard {
    fn new(owner: &Rc<Window>, open: &Rc<RefCell<Option<Wizard>>>) -> Self {
        let navigation = adw::NavigationView::new();
        let window = adw::Window::builder()
            .title("Set Up Onda")
            .default_width(720)
            .default_height(560)
            .content(&navigation)
            .build();
        window.set_application(owner.window.application().as_ref());
        let open = Rc::downgrade(open);
        let weak = Rc::downgrade(owner);
        window.connect_close_request(move |_| {
            if let Some(open) = open.upgrade() {
                open.take();
            }
            if let Some(owner) = weak.upgrade() {
                owner.core.app.onboarding_closed();
            }
            glib::Propagation::Proceed
        });
        let step = Rc::new(Cell::new(None));
        let named = step.clone();
        let weak = Rc::downgrade(owner);
        navigation.connect_popped(move |navigation, _| {
            // the core names the earlier step before this window pops to
            // it, so a pop to any other page is the user going back
            if visible_tag(navigation).as_deref() != named.get().map(tag)
                && let Some(owner) = weak.upgrade()
            {
                owner.core.app.onboarding_back();
            }
        });
        Self {
            owner: Rc::downgrade(owner),
            window,
            navigation,
            steps: RefCell::default(),
            step,
        }
    }

    fn update(&self, view: OnboardingView) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let mut steps = self.steps.borrow_mut();
        let step = steps.entry(view.step).or_insert_with(|| {
            let step = Step::new(&owner, view.step, title(&view.content));
            // the view keeps a page it was given, where it would drop a
            // pushed one at the end of the slide that pops it
            self.navigation.add(&step.page);
            step
        });
        self.step.set(Some(view.step));
        step.update(view);
        if self.navigation.visible_page().as_ref() == Some(&step.page) {
            return;
        }
        let stacked = self
            .navigation
            .navigation_stack()
            .iter::<adw::NavigationPage>()
            .flatten()
            .any(|page| page == step.page);
        if stacked {
            self.navigation.pop_to_page(&step.page);
        } else {
            self.navigation.push(&step.page);
        }
        step.focus.grab_focus();
    }
}

fn visible_tag(navigation: &adw::NavigationView) -> Option<glib::GString> {
    navigation.visible_page().and_then(|page| page.tag())
}

fn tag(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "welcome",
        OnboardingStep::Language => "language",
        OnboardingStep::Permissions => "permissions",
        OnboardingStep::Hotkey => "hotkey",
        OnboardingStep::SpeechModel => "speech-model",
        OnboardingStep::Ready => "ready",
    }
}

/// One step's page: the header with the progress, the step's own content,
/// and the footer with the hint and the primary button.
struct Step {
    page: adw::NavigationPage,
    progress: gtk::LevelBar,
    hint: gtk::Label,
    primary: gtk::Button,
    // what holds the keyboard focus when the step appears
    focus: gtk::Widget,
    body: Body,
}

impl Step {
    fn new(owner: &Rc<Window>, step: OnboardingStep, title: &str) -> Self {
        let progress = gtk::LevelBar::builder()
            .mode(gtk::LevelBarMode::Discrete)
            .width_request(180)
            .valign(gtk::Align::Center)
            .build();
        // the offsets would color the first segments as a warning
        for offset in ["low", "high", "full"] {
            progress.remove_offset_value(Some(offset));
        }
        progress.update_property(&[accessible::Property::Label("Setup progress")]);
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&progress));
        let hint = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();
        let primary = gtk::Button::builder()
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let weak = Rc::downgrade(owner);
        primary.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.onboarding_continue();
            }
        });
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 12);
        footer.set_margin_top(12);
        footer.set_margin_bottom(24);
        footer.set_margin_start(24);
        footer.set_margin_end(24);
        footer.append(&hint);
        footer.append(&primary);
        let (body, content) = Body::new(owner, step);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));
        toolbar.add_bottom_bar(&footer);
        let focus = match &body {
            // dictated text lands in the focused field
            Body::Ready(ready) => ready.try_it.text.clone().upcast(),
            _ => primary.clone().upcast(),
        };
        Self {
            page: adw::NavigationPage::builder()
                .title(title)
                .tag(tag(step))
                .child(&toolbar)
                .build(),
            progress,
            hint,
            primary,
            focus,
            body,
        }
    }

    // the navigation view draws Back and picks the slide's direction from
    // its own stack, so the view's fields for those go unread
    fn update(&self, view: OnboardingView) {
        self.progress.set_visible(view.progress.is_some());
        if let Some(progress) = view.progress {
            self.progress.set_max_value(progress.total.into());
            self.progress.set_value(progress.filled.into());
        }
        self.hint.set_visible(view.hint.is_some());
        self.hint.set_label(&view.hint.unwrap_or_default());
        self.primary.set_label(&view.primary_title);
        self.primary.set_sensitive(view.primary_enabled);
        self.body.update(view.content);
    }
}

fn title(content: &OnboardingContent) -> &str {
    match content {
        OnboardingContent::Welcome { title, .. }
        | OnboardingContent::Language { title, .. }
        | OnboardingContent::Permissions { title, .. }
        | OnboardingContent::Hotkey { title, .. }
        | OnboardingContent::SpeechModel { title, .. }
        | OnboardingContent::Ready { title, .. } => title,
    }
}

/// A step's own widgets, built for the step and filled from its content.
enum Body {
    Welcome(adw::StatusPage),
    Language(Languages),
    // Linux has no privacy permission to grant, so the core's Linux wizard
    // has no such step (`OnboardingStep::on_platform`)
    Permissions,
    Hotkey(Presets),
    SpeechModel(Models),
    Ready(Ready),
}

impl Body {
    fn new(owner: &Rc<Window>, step: OnboardingStep) -> (Self, gtk::Widget) {
        match step {
            OnboardingStep::Welcome => {
                let page = adw::StatusPage::new();
                page.set_paintable(super::app_icon().as_ref());
                (Self::Welcome(page.clone()), page.upcast())
            }
            OnboardingStep::Language => {
                let (body, widget) = Languages::new(owner);
                (Self::Language(body), widget)
            }
            OnboardingStep::Permissions => (Self::Permissions, gtk::Box::default().upcast()),
            OnboardingStep::Hotkey => {
                let (body, widget) = Presets::new(owner);
                (Self::Hotkey(body), widget)
            }
            OnboardingStep::SpeechModel => {
                let (body, widget) = Models::new(owner);
                (Self::SpeechModel(body), widget)
            }
            OnboardingStep::Ready => {
                let (body, widget) = Ready::new();
                (Self::Ready(body), widget)
            }
        }
    }

    fn update(&self, content: OnboardingContent) {
        match (self, content) {
            (Self::Welcome(page), OnboardingContent::Welcome { title, subtitle }) => {
                page.set_title(&title);
                page.set_description(Some(&glib::markup_escape_text(&subtitle)));
            }
            (
                Self::Language(body),
                OnboardingContent::Language {
                    title,
                    search_placeholder,
                    toggle_title,
                    cards,
                    // the field owns its text, so the core's echo of it
                    // never overwrites what the user is typing
                    query: _,
                },
            ) => body.update(&title, &search_placeholder, toggle_title, &cards),
            (
                Self::Hotkey(body),
                OnboardingContent::Hotkey {
                    title,
                    subtitle,
                    presets,
                    // #16 adds the "Another shortcut" row, with the
                    // recorder of the Hotkey page, below the presets
                    custom_selected: _,
                    custom_keycaps: _,
                    custom_label: _,
                    // it sits next to the Fn warning, and Linux has no Fn
                    // shortcut to warn about
                    open_keyboard_settings_title: _,
                },
            ) => body.update(&title, &subtitle, &presets),
            (
                Self::SpeechModel(body),
                OnboardingContent::SpeechModel {
                    title,
                    cards,
                    other_models,
                    error,
                },
            ) => body.update(&title, &cards, other_models, error),
            (
                Self::Ready(body),
                OnboardingContent::Ready {
                    title,
                    prompt_prefix,
                    keycaps,
                    prompt_suffix,
                    try_it_placeholder,
                    retention_note,
                },
            ) => {
                body.heading.set_label(&title);
                body.prefix.set_label(&prompt_prefix);
                fill_keycaps(&body.keycaps, &keycaps);
                body.suffix.set_label(&prompt_suffix);
                body.try_it.set_placeholder(&try_it_placeholder);
                body.note.set_visible(retention_note.is_some());
                body.note.set_label(&retention_note.unwrap_or_default());
            }
            // Linux shows no Permissions step, and every other page gets
            // the content of the step it was built for
            _ => {}
        }
    }
}

/// A boxed list that rebuilds its rows only when their keys change, so the
/// keyboard focus and a button the user is pressing survive an update.
struct Rows<K, R> {
    list: gtk::ListBox,
    rows: Rc<RefCell<Vec<(K, R)>>>,
}

impl<K: PartialEq + 'static, R: 'static> Rows<K, R> {
    fn new() -> Self {
        Self {
            list: gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build(),
            rows: Rc::default(),
        }
    }

    fn connect_activated(&self, activate: impl Fn(&K) + 'static) {
        let rows = self.rows.clone();
        self.list.connect_row_activated(move |_, row| {
            if let Some((key, _)) = rows.borrow().get(row.index() as usize) {
                activate(key);
            }
        });
    }

    fn sync<T, W: IsA<gtk::Widget>>(
        &self,
        items: &[T],
        key: impl Fn(&T) -> K,
        build: impl Fn(&T) -> (W, R),
        update: impl Fn(&R, &T),
    ) {
        let keys: Vec<K> = items.iter().map(key).collect();
        let mut rows = self.rows.borrow_mut();
        if !rows.iter().map(|(key, _)| key).eq(&keys) {
            self.list.remove_all();
            *rows = items
                .iter()
                .zip(keys)
                .map(|(item, key)| {
                    let (row, parts) = build(item);
                    self.list.append(&row);
                    (key, parts)
                })
                .collect();
        }
        for ((_, parts), item) in rows.iter().zip(items) {
            update(parts, item);
        }
        self.list.set_visible(!rows.is_empty());
    }
}

/// A row the user picks, and the check mark that shows the current pick.
struct Choice {
    row: adw::ActionRow,
    check: gtk::Image,
}

impl Choice {
    fn new(row: adw::ActionRow) -> Self {
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.add_css_class("accent");
        row.add_suffix(&check);
        row.set_activatable(true);
        Self { row, check }
    }

    fn set_selected(&self, selected: bool) {
        self.check.set_visible(selected);
        self.row
            .upcast_ref::<gtk::Widget>()
            .update_state(&[accessible::State::Selected(Some(selected))]);
    }
}

struct Languages {
    heading: gtk::Label,
    search: gtk::SearchEntry,
    toggle: gtk::Button,
    rows: Rows<String, Choice>,
}

impl Languages {
    fn new(owner: &Rc<Window>) -> (Self, gtk::Widget) {
        let heading = heading();
        let search = gtk::SearchEntry::builder().hexpand(true).build();
        let weak = Rc::downgrade(owner);
        search.connect_search_changed(move |search| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.onboarding_search_languages(search.text());
            }
        });
        let toggle = gtk::Button::new();
        let weak = Rc::downgrade(owner);
        toggle.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.onboarding_toggle_all_languages();
            }
        });
        let rows = Rows::new();
        let weak = Rc::downgrade(owner);
        rows.connect_activated(move |code: &String| {
            if let Some(owner) = weak.upgrade()
                && let Some(language) = LanguageSetting::parse(code)
            {
                owner.core.app.onboarding_select_language(language);
            }
        });
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bar.append(&search);
        bar.append(&toggle);
        // the search stays in reach while the list scrolls
        let top = column();
        top.set_margin_bottom(0);
        top.append(&heading);
        top.append(&bar);
        let list = column();
        list.append(&rows.list);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&clamp(&top));
        widget.append(&scrolled(&list));
        let body = Self {
            heading,
            search,
            toggle,
            rows,
        };
        (body, widget.upcast())
    }

    fn update(
        &self,
        title: &str,
        placeholder: &str,
        toggle_title: Option<String>,
        cards: &[LanguageCardView],
    ) {
        self.heading.set_label(title);
        self.search.set_placeholder_text(Some(placeholder));
        self.toggle.set_visible(toggle_title.is_some());
        self.toggle.set_label(&toggle_title.unwrap_or_default());
        self.rows.sync(
            cards,
            |card| card.code.clone(),
            |card| {
                let row = adw::ActionRow::builder()
                    .title(&card.name)
                    .subtitle(&card.code_label)
                    .use_markup(false)
                    .build();
                row.add_prefix(&gtk::Label::new(Some(&card.flag)));
                (row.clone(), Choice::new(row))
            },
            |choice, card| choice.set_selected(card.selected),
        );
    }
}

struct Presets {
    heading: gtk::Label,
    subtitle: gtk::Label,
    rows: Rows<HotkeyPreset, Choice>,
}

impl Presets {
    fn new(owner: &Rc<Window>) -> (Self, gtk::Widget) {
        let heading = heading();
        let subtitle = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();
        let rows = Rows::new();
        let weak = Rc::downgrade(owner);
        rows.connect_activated(move |preset: &HotkeyPreset| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.onboarding_select_hotkey(*preset);
            }
        });
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 6);
        titles.append(&heading);
        titles.append(&subtitle);
        let content = column();
        content.append(&titles);
        content.append(&rows.list);
        let body = Self {
            heading,
            subtitle,
            rows,
        };
        (body, scrolled(&content).upcast())
    }

    fn update(&self, title: &str, subtitle: &str, presets: &[PresetCardView]) {
        self.heading.set_label(title);
        self.subtitle.set_label(subtitle);
        self.rows.sync(
            presets,
            |card| card.preset,
            |card| {
                let row = adw::ActionRow::builder().use_markup(false).build();
                let keycaps = keycaps();
                fill_keycaps(&keycaps, &card.keycaps);
                row.add_prefix(&keycaps);
                (row.clone(), Choice::new(row))
            },
            |choice, card| {
                choice.row.set_title(&card.detail);
                choice.row.set_sensitive(card.enabled);
                choice.set_selected(card.selected);
            },
        );
    }
}

struct Models {
    owner: Weak<Window>,
    heading: gtk::Label,
    cards: Rows<ModelCardView, Option<gtk::ProgressBar>>,
    toggle: gtk::Button,
    others: Rows<ModelCardView, Option<gtk::ProgressBar>>,
    error: gtk::Label,
}

impl Models {
    fn new(owner: &Rc<Window>) -> (Self, gtk::Widget) {
        let heading = heading();
        let toggle = gtk::Button::builder()
            .halign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let weak = Rc::downgrade(owner);
        toggle.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.onboarding_toggle_other_models();
            }
        });
        let error = gtk::Label::builder()
            .wrap(true)
            .css_classes(["error"])
            .build();
        let body = Self {
            owner: Rc::downgrade(owner),
            heading,
            cards: Rows::new(),
            toggle,
            others: Rows::new(),
            error,
        };
        let content = column();
        content.append(&body.heading);
        content.append(&body.cards.list);
        content.append(&body.error);
        content.append(&body.toggle);
        content.append(&body.others.list);
        (body, scrolled(&content).upcast())
    }

    fn update(
        &self,
        title: &str,
        cards: &[ModelCardView],
        other_models: Option<OtherModelsView>,
        error: Option<String>,
    ) {
        self.heading.set_label(title);
        self.sync(&self.cards, cards);
        self.error.set_visible(error.is_some());
        self.error.set_label(&error.unwrap_or_default());
        self.toggle.set_visible(other_models.is_some());
        let (toggle_title, others) = other_models
            .map(|other| (other.toggle_title, other.rows))
            .unwrap_or_default();
        self.toggle.set_label(&toggle_title);
        self.sync(&self.others, &others);
    }

    fn sync(&self, rows: &Rows<ModelCardView, Option<gtk::ProgressBar>>, cards: &[ModelCardView]) {
        rows.sync(
            cards,
            // a download reports progress many times a second, and the
            // Cancel button must outlive each report
            |card| {
                let mut shape = card.clone();
                if let ModelCardAction::Downloading {
                    progress, label, ..
                } = &mut shape.action
                {
                    *progress = 0.0;
                    label.clear();
                }
                shape
            },
            |card| self.row(card),
            |bar, card| {
                if let (
                    Some(bar),
                    ModelCardAction::Downloading {
                        progress, label, ..
                    },
                ) = (bar, &card.action)
                {
                    bar.set_fraction(*progress);
                    bar.set_text(Some(label));
                }
            },
        );
    }

    fn row(&self, card: &ModelCardView) -> (adw::ActionRow, Option<gtk::ProgressBar>) {
        let row = adw::ActionRow::builder()
            .title(&card.name)
            .subtitle(format!("{} · {}", card.size_hint, card.description))
            .use_markup(false)
            .build();
        if let Some(recommended) = &card.recommended {
            let badge = gtk::Label::new(Some(recommended));
            badge.add_css_class("caption");
            badge.add_css_class("accent");
            row.add_suffix(&badge);
        }
        let command = |title: &str, command: fn(&Window, ModelId)| {
            let button = gtk::Button::builder()
                .label(title)
                .valign(gtk::Align::Center)
                .build();
            let weak = self.owner.clone();
            let id = ModelId::parse(&card.id);
            button.connect_clicked(move |_| {
                if let (Some(owner), Some(id)) = (weak.upgrade(), id) {
                    command(&owner, id);
                }
            });
            row.add_suffix(&button);
            button
        };
        let bar = match &card.action {
            ModelCardAction::Downloading { cancel_title, .. } => {
                let bar = gtk::ProgressBar::builder()
                    .show_text(true)
                    .valign(gtk::Align::Center)
                    .width_request(150)
                    .build();
                row.add_suffix(&bar);
                command(cancel_title, |owner, id| owner.core.app.cancel_download(id));
                Some(bar)
            }
            ModelCardAction::Selected { title } => {
                let selected = gtk::Label::new(Some(title));
                selected.add_css_class("success");
                row.add_suffix(&selected);
                None
            }
            ModelCardAction::UseThisModel { title } => {
                command(title, |owner, id| {
                    owner.core.app.onboarding_choose_model(id)
                });
                None
            }
            ModelCardAction::Download { title } => {
                command(title, |owner, id| {
                    owner.core.app.onboarding_download_model(id)
                })
                .add_css_class("suggested-action");
                None
            }
        };
        (row, bar)
    }
}

struct Ready {
    heading: gtk::Label,
    prefix: gtk::Label,
    keycaps: gtk::Box,
    suffix: gtk::Label,
    try_it: TryIt,
    note: gtk::Label,
}

impl Ready {
    fn new() -> (Self, gtk::Widget) {
        let done = gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .pixel_size(48)
            .css_classes(["success"])
            .build();
        let heading = heading();
        let prefix = gtk::Label::new(None);
        let keycaps = keycaps();
        let suffix = gtk::Label::new(None);
        let prompt = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        prompt.set_halign(gtk::Align::Center);
        prompt.append(&prefix);
        prompt.append(&keycaps);
        prompt.append(&suffix);
        let try_it = TryIt::new();
        let note = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();
        let content = column();
        content.append(&done);
        content.append(&heading);
        content.append(&prompt);
        content.append(&try_it.widget);
        content.append(&note);
        let body = Self {
            heading,
            prefix,
            keycaps,
            suffix,
            try_it,
            note,
        };
        (body, scrolled(&content).upcast())
    }
}

// the theme draws a key for each `keycap` label inside a `shortcut`
fn keycaps() -> gtk::Box {
    gtk::Box::builder()
        .css_name("shortcut")
        .spacing(4)
        .valign(gtk::Align::Center)
        .build()
}

fn fill_keycaps(keycaps: &gtk::Box, labels: &[String]) {
    super::clear(keycaps);
    for label in labels {
        let keycap = gtk::Label::new(Some(label));
        keycap.add_css_class("keycap");
        keycaps.append(&keycap);
    }
}

fn heading() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["title-1"])
        .build()
}

fn column() -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    column.set_margin_top(12);
    column.set_margin_bottom(12);
    column.set_margin_start(24);
    column.set_margin_end(24);
    column
}

fn clamp(child: &impl IsA<gtk::Widget>) -> adw::Clamp {
    adw::Clamp::builder()
        .maximum_size(CONTENT_WIDTH)
        .child(child)
        .build()
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp(child))
        .build()
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    use adw::prelude::*;
    use onda_adapters::DataPaths;
    use onda_app::{
        Event,
        views::{
            ModelCardAction, ModelCardView, OnboardingContent, OnboardingProgress, OnboardingView,
            OtherModelsView,
        },
    };
    use onda_domain::{
        hotkey::Shortcut,
        languages::LanguageSetting,
        models::Catalog,
        onboarding::{AUTOMATIC_ONBOARDING_DELAY, OnboardingStep},
    };

    use super::super::{
        Window,
        integration_tests::{
            MODEL, Start, button, capture_window, launch, seed_model, text_visible, wait_for,
            widgets,
        },
    };
    use crate::linux::Message;

    const PENDING: &str = "Onda, finish setup to dictate";
    const READY: &str = "Onda, ready to dictate";

    fn data_dir() -> PathBuf {
        PathBuf::from(std::env::var_os("ONDA_DATA_DIR").expect("use a scratch data directory"))
    }

    fn wizard(owner: &Window) -> adw::Window {
        wait_for(|| owner.onboarding.window().is_some());
        owner.onboarding.window().unwrap()
    }

    fn navigation(window: &adw::Window) -> adw::NavigationView {
        widgets::<adw::NavigationView>(window).remove(0)
    }

    /// Waits for the step and returns its page. The pages below it in the
    /// stack stay in the window, so a search starts from this page.
    fn arrive(window: &adw::Window, tag: &str) -> adw::NavigationPage {
        let navigation = navigation(window);
        wait_for(|| super::visible_tag(&navigation).as_deref() == Some(tag));
        navigation.visible_page().unwrap()
    }

    fn rows(root: &impl IsA<gtk::Widget>) -> Vec<adw::ActionRow> {
        widgets::<adw::ActionRow>(root)
    }

    fn titles(root: &impl IsA<gtk::Widget>) -> Vec<String> {
        rows(root).iter().map(|row| row.title().into()).collect()
    }

    // what a click on the row or Return on it does
    fn pick(row: &adw::ActionRow) {
        assert!(gtk::prelude::WidgetExt::activate(row));
    }

    fn picked(row: &adw::ActionRow) -> bool {
        widgets::<gtk::Image>(row).iter().any(|image| {
            image.icon_name().as_deref() == Some("object-select-symbolic") && image.is_visible()
        })
    }

    fn keycaps(root: &impl IsA<gtk::Widget>) -> Vec<String> {
        widgets::<gtk::Label>(root)
            .iter()
            .filter(|label| label.has_css_class("keycap"))
            .map(|label| label.text().into())
            .collect()
    }

    fn shown(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
        widgets::<gtk::Label>(root)
            .iter()
            .any(|label| label.text() == text && label.is_visible())
    }

    fn progress(page: &adw::NavigationPage) -> gtk::LevelBar {
        widgets::<gtk::LevelBar>(page).remove(0)
    }

    fn quit(owner: &Window) {
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_first_run_walks_the_wizard_to_the_end() {
        let loads = seed_model();
        let launched = Instant::now();
        let owner = launch(Start::FirstRun);
        assert!(
            owner.onboarding.window().is_none(),
            "the wizard waits for the first-run delay"
        );
        let window = wizard(&owner);
        assert!(launched.elapsed() >= AUTOMATIC_ONBOARDING_DELAY);
        assert!(!owner.window.is_visible(), "the wizard opens by itself");
        assert_eq!(window.title().as_deref(), Some("Set Up Onda"));
        assert_eq!(owner.status(), PENDING);

        let page = arrive(&window, "welcome");
        assert!(shown(&page, "Welcome to Onda"));
        assert!(shown(&page, "On-device dictation for any app."));
        assert!(!progress(&page).is_visible());
        capture_window(&window, "onboarding-welcome");
        button(&page, "Get Started").emit_clicked();

        let page = arrive(&window, "language");
        assert!(shown(&page, "What language do you speak?"));
        assert_eq!(
            (progress(&page).value(), progress(&page).max_value()),
            (1.0, 4.0)
        );
        let popular = titles(&page);
        let search = widgets::<gtk::SearchEntry>(&page).remove(0);
        assert_eq!(
            search.placeholder_text().as_deref(),
            Some("Search languages")
        );
        let toggle = button(&page, "Show All");
        search.set_text("ger");
        wait_for(|| titles(&page) == ["German"]);
        assert!(!toggle.is_visible(), "hidden in a search");
        assert_eq!(search.text(), "ger", "the core's echo keeps the field");
        capture_window(&window, "onboarding-language-search");
        search.set_text("");
        wait_for(|| titles(&page) == popular);
        toggle.emit_clicked();
        wait_for(|| titles(&page).len() > popular.len());
        button(&page, "Popular Only").emit_clicked();
        wait_for(|| titles(&page) == popular);
        let english = || {
            rows(&page)
                .into_iter()
                .find(|row| row.title() == "English")
                .unwrap()
        };
        pick(&english());
        wait_for(|| picked(&english()));
        assert_eq!(rows(&page).into_iter().filter(picked).count(), 1);
        wait_for(|| {
            owner.core.settings.selected_language(None) == LanguageSetting::parse("en").unwrap()
        });
        assert_eq!(
            owner.core.settings.selected_model(&Catalog::gguf(), None),
            MODEL.as_str()
        );
        capture_window(&window, "onboarding-language");
        button(&page, "Continue").emit_clicked();

        let page = arrive(&window, "hotkey");
        assert!(shown(&page, "Choose a hotkey"));
        assert_eq!(progress(&page).value(), 2.0);
        let presets = rows(&page);
        let shortcuts: Vec<String> = presets.iter().map(|row| keycaps(row).join("+")).collect();
        assert_eq!(
            shortcuts,
            ["Ctrl+Alt+Space", "Alt+Super+Space", "Ctrl+Super+Space"]
        );
        assert!(picked(&presets[0]));
        let default = onda_domain::platform::Platform::linux().default_shortcut;
        pick(&presets[1]);
        wait_for(|| owner.core.settings.hotkey(default) == Shortcut::ALT_SUPER_SPACE);
        wait_for(|| picked(&presets[1]) && !picked(&presets[0]));
        // the desktop holds the new shortcut before the next pick replaces it
        wait_for(|| owner.getting_started.shortcut() == "Alt+Super+Space");
        assert_eq!(rows(&page), presets, "a pick keeps the rows and the focus");
        capture_window(&window, "onboarding-hotkey");
        pick(&presets[0]);
        wait_for(|| owner.core.settings.hotkey(default) == *default);
        wait_for(|| picked(&presets[0]));
        wait_for(|| owner.getting_started.shortcut() == "Control+Alt+Space");

        // what the header's Back button, Escape, and Alt+Left do
        assert!(navigation(&window).pop());
        let page = arrive(&window, "language");
        button(&page, "Continue").emit_clicked();
        // the core went back with the window, or it would now show the
        // model step
        let page = arrive(&window, "hotkey");
        button(&page, "Continue").emit_clicked();

        let page = arrive(&window, "speech-model");
        assert!(shown(&page, "Choose a speech model"));
        assert_eq!(progress(&page).value(), 3.0);
        assert_eq!(rows(&page).len(), 1);
        assert!(shown(&page, "Recommended") && shown(&page, "Selected"));
        let proceed = button(&page, "Continue");
        assert!(proceed.is_sensitive());
        button(&page, "Show other models").emit_clicked();
        wait_for(|| rows(&page).len() > 1);
        let downloads = widgets::<gtk::Button>(&page)
            .iter()
            .filter(|button| button.label().as_deref() == Some("Download"))
            .count();
        assert_eq!(downloads, rows(&page).len() - 1);
        capture_window(&window, "onboarding-model");
        button(&page, "Hide other models").emit_clicked();
        wait_for(|| rows(&page).len() == 1);
        proceed.emit_clicked();

        let page = arrive(&window, "ready");
        assert!(shown(&page, "You're all set"));
        assert_eq!(progress(&page).value(), 4.0);
        assert!(shown(&page, "Press") && shown(&page, "and start talking."));
        assert_eq!(keycaps(&page), ["Ctrl", "Alt", "Space"]);
        assert!(shown(
            &page,
            "Transcripts are saved on this computer for 1 month."
        ));
        let try_it = widgets::<gtk::TextView>(&page).remove(0);
        wait_for(|| gtk::prelude::RootExt::focus(&window).as_ref() == Some(try_it.upcast_ref()));
        assert!(shown(&page, "Click here and try it."));
        capture_window(&window, "onboarding-ready");
        try_it.buffer().set_text("Troy and Abed in the morning.");
        assert!(!shown(&page, "Click here and try it."));
        button(&page, "Start Dictating").emit_clicked();

        wait_for(|| owner.onboarding.window().is_none());
        assert!(!window.is_visible());
        wait_for(|| owner.core.settings.onboarding_completed());
        assert!(owner.core.settings.did_show_first_run_setup());
        if loads {
            // checking and loading the model outlasts `wait_for`
            let deadline = Instant::now() + Duration::from_secs(180);
            while owner.status() != READY {
                assert!(Instant::now() < deadline, "{}", owner.status());
                wait_for(|| true);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        quit(&owner);
    }

    fn model_step(action: ModelCardAction, error: Option<&str>) -> OnboardingView {
        let card = |id: &str, name: &str, recommended: bool, action| ModelCardView {
            id: id.into(),
            name: name.into(),
            description: "Fast and accurate English.".into(),
            size_hint: "730 MB".into(),
            recommended: recommended.then(|| "Recommended".into()),
            selected: recommended,
            action,
        };
        OnboardingView {
            step: OnboardingStep::SpeechModel,
            forward: true,
            progress: Some(OnboardingProgress {
                total: 4,
                filled: 3,
            }),
            back_visible: true,
            back_title: "Back".into(),
            hint: Some("Download or choose a speech model to continue".into()),
            primary_title: "Continue".into(),
            primary_enabled: false,
            content: OnboardingContent::SpeechModel {
                title: "Choose a speech model".into(),
                cards: vec![card(
                    "parakeet-tdt-v2",
                    "Greendale Human Being",
                    true,
                    action,
                )],
                other_models: Some(OtherModelsView {
                    toggle_title: "Hide other models".into(),
                    rows: vec![card(
                        "parakeet-ultra",
                        "City College Rival",
                        false,
                        ModelCardAction::UseThisModel {
                            title: "Use This Model".into(),
                        },
                    )],
                }),
                error: error.map(Into::into),
            },
        }
    }

    // the views a download produces, sent in the core's place: the test
    // session has no network, and a click on Download would ask for it
    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_model_step_shows_each_state_of_a_download() {
        let owner = launch(Start::FirstRun);
        let window = wizard(&owner);
        arrive(&window, "welcome");
        let show = |view| owner.event(Message::Core(Event::OnboardingChanged(Box::new(view))));
        let download = || ModelCardAction::Download {
            title: "Download".into(),
        };
        let downloading = |progress: f64| ModelCardAction::Downloading {
            progress,
            label: format!("Downloading — {}%", (progress * 100.0).round()),
            cancel_title: "Cancel".into(),
        };

        show(model_step(download(), None));
        let page = arrive(&window, "speech-model");
        assert!(button(&page, "Download").has_css_class("suggested-action"));
        assert!(shown(&page, "Recommended"));
        assert!(button(&page, "Use This Model").is_visible());
        assert!(shown(
            &page,
            "Download or choose a speech model to continue"
        ));
        assert!(!button(&page, "Continue").is_sensitive());
        capture_window(&window, "onboarding-model-download");

        show(model_step(downloading(0.42), None));
        let bar = || widgets::<gtk::ProgressBar>(&page).remove(0);
        assert_eq!(bar().fraction(), 0.42);
        assert_eq!(bar().text().as_deref(), Some("Downloading — 42%"));
        let cancel = button(&page, "Cancel");
        show(model_step(downloading(0.43), None));
        assert_eq!(bar().fraction(), 0.43);
        assert_eq!(
            button(&page, "Cancel"),
            cancel,
            "progress must not replace the button under the pointer"
        );
        capture_window(&window, "onboarding-model-downloading");

        show(model_step(
            download(),
            Some("Download failed: Greendale is offline"),
        ));
        let error = widgets::<gtk::Label>(&page)
            .into_iter()
            .find(|label| label.text() == "Download failed: Greendale is offline")
            .unwrap();
        assert!(error.is_visible() && error.has_css_class("error"));
        assert!(widgets::<gtk::ProgressBar>(&page).is_empty());
        capture_window(&window, "onboarding-model-error");

        show(model_step(download(), None));
        assert!(!error.is_visible());
        window.close();
        wait_for(|| owner.onboarding.window().is_none());
        quit(&owner);
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_closing_the_wizard_early_leaves_first_run_pending() {
        let owner = launch(Start::FirstRun);
        let window = wizard(&owner);
        let page = arrive(&window, "welcome");
        button(&page, "Get Started").emit_clicked();
        arrive(&window, "language");
        window.close();
        wait_for(|| owner.onboarding.window().is_none());

        // the core closed its wizard too: asked again, it starts a new one
        // at Welcome, where the closed one was on Language
        owner.core.app.run_setup_again();
        let window = wizard(&owner);
        let page = arrive(&window, "welcome");
        assert!(text_visible(&page, "Welcome to Onda"));
        window.close();
        wait_for(|| owner.onboarding.window().is_none());

        assert!(!owner.core.settings.onboarding_completed());
        assert!(!owner.core.settings.did_show_first_run_setup());
        assert_eq!(owner.status(), PENDING);
        assert!(
            !DataPaths::new(data_dir())
                .models_dir()
                .join(MODEL.as_str())
                .exists(),
            "closing the wizard must not download a model"
        );
        quit(&owner);
    }
}
