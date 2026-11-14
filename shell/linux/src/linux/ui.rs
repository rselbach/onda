//! GTK widgets render core views and send commands back to the core.

mod about;
mod general;
mod history;
mod hotkey;
mod navigation;
mod notice;
mod post_processing;
mod replacements;
mod speech;

#[cfg(test)]
mod integration_tests;

use adw::prelude::*;
use gtk::{gio, glib};
use onda_app::{Event, views::MainWindowSection};
use onda_domain::non_dictation_ui::DownloadOverlayView;
use std::{cell::Cell, rc::Rc};

use super::{Message, core::Core, overlay::Overlay, tray::Command};

const IDLE_HINT: &str = "Use your global shortcut to dictate into another application.";

pub struct Window {
    pub core: Core,
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    models: gtk::Box,
    history: history::Page,
    hotkey: hotkey::Page,
    notice: notice::Notice,
    replacements: replacements::Page,
    about: gtk::Box,
    section: Cell<MainWindowSection>,
    speech_alert: Cell<Option<u64>>,
    general: gtk::Box,
    post_processing: post_processing::Page,
    status: gtk::Label,
    detail: gtk::Label,
    meter: gtk::LevelBar,
    shortcut: gtk::Label,
    toast: adw::ToastOverlay,
    overlay: Overlay,
    overlay_visible: Cell<bool>,
    inhibitor: Cell<Option<u32>>,
    tray: Option<ksni::blocking::Handle<super::tray::Tray>>,
}

impl Window {
    pub fn new(application: &adw::Application) -> anyhow::Result<Rc<Self>> {
        let (events, incoming) = async_channel::unbounded();
        let tray = super::tray::start(events.clone());
        let overlay_events = events.clone();
        let core = Core::new(events)?;
        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("Onda")
            .default_width(1060)
            .default_height(780)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new(MainWindowSection::GettingStarted.title(), "Onda");
        header.set_title_widget(Some(&title));
        let quit = gtk::Button::with_label("Quit");
        let app = application.clone();
        quit.connect_clicked(move |_| app.quit());
        header.pack_end(&quit);
        toolbar.add_top_bar(&header);
        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let sidebar = navigation::sidebar(&stack, &title);
        let layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        layout.append(&sidebar);
        layout.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        layout.append(&stack);
        toolbar.set_content(Some(&layout));
        let toast = adw::ToastOverlay::new();
        toast.set_child(Some(&toolbar));
        window.set_content(Some(&toast));

        let home = page();
        let image = gtk::Image::from_icon_name("audio-input-microphone-symbolic");
        image.set_pixel_size(64);
        image.set_margin_top(32);
        home.append(&image);
        let status = gtk::Label::new(Some("Welcome to Onda"));
        status.add_css_class("title-1");
        home.append(&status);
        let detail = label(
            "Download a speech model to get started. Your recordings are transcribed on this computer.",
        );
        home.append(&detail);
        let meter = gtk::LevelBar::new();
        home.append(&meter);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        actions.set_halign(gtk::Align::Center);
        let record = gtk::Button::with_label("Start / Stop");
        record.add_css_class("suggested-action");
        let cancel = gtk::Button::with_label("Cancel");
        actions.append(&record);
        actions.append(&cancel);
        home.append(&actions);
        let shortcut = label("Preparing the global shortcut…");
        home.append(&shortcut);
        home.append(&label("Close this window to keep Onda running. Reopen it from your application launcher. Use Quit to stop Onda."));
        stack.add_titled(
            &scrolled(&home),
            Some("dictation"),
            MainWindowSection::GettingStarted.title(),
        );
        let models = page();
        stack.add_titled(
            &scrolled(&models),
            Some("speech"),
            MainWindowSection::Speech.title(),
        );
        let post_processing = post_processing::Page::new();
        stack.add_titled(
            &scrolled(&post_processing.widget),
            Some("post-processing"),
            MainWindowSection::PostProcessing.title(),
        );
        let history = history::Page::new();
        stack.add_titled(
            &scrolled(&history.widget),
            Some("history"),
            MainWindowSection::History.title(),
        );
        let general = page();
        stack.add_titled(
            &scrolled(&general),
            Some("general"),
            MainWindowSection::General.title(),
        );
        let hotkey = hotkey::Page::new();
        stack.add_titled(
            &scrolled(&hotkey.widget),
            Some("hotkey"),
            MainWindowSection::Hotkey.title(),
        );
        let replacements = replacements::Page::new();
        stack.add_titled(
            &scrolled(&replacements.widget),
            Some("replacements"),
            MainWindowSection::Replacements.title(),
        );
        let about = page();
        stack.add_titled(
            &scrolled(&about),
            Some("about"),
            MainWindowSection::About.title(),
        );
        let setup = adw::PreferencesGroup::builder()
            .title("Set up dictation")
            .build();
        for (name, title, description) in [
            (
                "speech",
                "Choose a speech model",
                "Download a model to transcribe on this computer.",
            ),
            (
                "hotkey",
                "Set your shortcut",
                "Choose how you start and stop dictation.",
            ),
            (
                "general",
                "Choose your microphone and output",
                "Set where Onda listens and sends your words.",
            ),
        ] {
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(description)
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let stack = stack.clone();
            row.connect_activated(move |_| stack.set_visible_child_name(name));
            setup.add(&row);
        }
        home.append(&setup);
        let overlay = Overlay::new(move || super::send(&overlay_events, Message::CancelDictation));
        if !overlay.available() {
            home.append(&label("A floating recording indicator is unavailable on this desktop. Recording status remains visible here."));
        }
        let this = Rc::new(Self {
            core,
            window,
            stack,
            models,
            history,
            hotkey,
            notice: notice::Notice::default(),
            replacements,
            about,
            section: Cell::new(MainWindowSection::GettingStarted),
            speech_alert: Cell::new(None),
            general,
            post_processing,
            status,
            detail,
            meter,
            shortcut,
            toast,
            overlay,
            overlay_visible: Cell::new(false),
            inhibitor: Cell::new(None),
            tray,
        });
        this.post_processing.connect(&this);
        this.hotkey.connect(&this);
        this.history.connect(&this);
        this.replacements.connect(&this);
        let weak = Rc::downgrade(&this);
        record.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.core.app.toggle_dictation_from_menu();
            }
        });
        let weak = Rc::downgrade(&this);
        cancel.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.core.app.cancel_dictation();
            }
        });
        let weak = Rc::downgrade(&this);
        this.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.window.set_visible(false);
                this.core.app.main_window_closed();
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&this);
        this.window.connect_is_active_notify(move |window| {
            if window.is_active()
                && let Some(this) = weak.upgrade()
            {
                this.core.app.app_became_active();
            }
        });
        let weak = Rc::downgrade(&this);
        this.stack.connect_visible_child_name_notify(move |stack| {
            if let Some(this) = weak.upgrade() {
                let section = navigation::section(
                    stack.visible_child_name().as_deref().unwrap_or("dictation"),
                );
                let previous = this.section.replace(section);
                if previous != section {
                    this.core.app.page_disappeared(previous);
                }
                this.core.app.page_appeared(section);
            }
        });
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok(message) = incoming.recv().await {
                let Some(this) = weak.upgrade() else {
                    break;
                };
                this.event(message);
            }
        });
        for section in MainWindowSection::ALL {
            this.core.app.page_appeared(section);
        }
        this.core.app.ui_ready();
        Ok(this)
    }

    pub fn present(&self) {
        self.core.app.main_window_appeared();
        if !self.window.is_visible() {
            self.core.app.page_appeared(self.section.get());
        }
        self.window.present();
    }

    pub fn status(&self) -> String {
        self.status.text().to_string()
    }

    /// The one place a tray choice becomes a core call.
    fn tray(&self, command: Command) {
        let app = &self.core.app;
        match command {
            Command::Show => self.present(),
            Command::MenuWillOpen => app.menu_will_open(),
            Command::FixPermissions => app.fix_permissions(),
            Command::ToggleDictation => app.toggle_dictation_from_menu(),
            Command::CancelDictation => app.cancel_dictation(),
            Command::DeliverLastTranscript => app.deliver_last_transcript(),
            Command::DeliverTranscript(text) => app.deliver_transcript(text),
            Command::ShowAllHistory => app.show_all_history(),
            Command::RetryFailedDictation => app.retry_failed_dictation(),
            Command::DiscardFailedDictation => app.discard_failed_dictation(),
            Command::RetryModelLoad => app.retry_model_load(),
            Command::RedownloadModel => app.redownload_model(),
            Command::SelectLanguage(language) => app.select_language_from_menu(language),
            Command::ChooseOtherLanguage => app.choose_other_language(),
            Command::OpenApp => app.open_app(),
            Command::OpenSettings => app.open_settings(),
            Command::Update(action) => app.update_action(action),
            Command::Quit => {
                if let Some(application) = self.window.application() {
                    application.quit();
                }
            }
        }
    }

    fn event(self: &Rc<Self>, message: Message) {
        match message {
            Message::Tray(command) => self.tray(command),
            Message::CancelDictation => self.core.app.cancel_dictation(),
            Message::Hotkey(event) => self.core.app.on_hotkey_event(event),
            Message::Download(update) => self.core.app.download_event(update),
            Message::DevicesChanged => self.core.app.input_devices_changed(),
            Message::UpdaterChanged => self.core.app.updater_changed(),
            Message::ShortcutStatus(status) => {
                self.shortcut.set_text(&status);
                self.hotkey.set_status(&status);
            }
            Message::Notice(title, body) => {
                self.toast
                    .add_toast(adw::Toast::new(&format!("{title}: {body}")));
                if !self.window.is_active()
                    && let Some(app) = self.window.application()
                {
                    let notification = gio::Notification::new(&title);
                    notification.set_body(Some(&body));
                    app.send_notification(Some("dictation"), &notification);
                }
            }
            Message::Inhibit(reason) => {
                if let Some(app) = self.window.application() {
                    if let Some(cookie) = self.inhibitor.take() {
                        app.uninhibit(cookie);
                    }
                    if let Some(reason) = reason {
                        let cookie = app.inhibit(
                            Some(&self.window),
                            gtk::ApplicationInhibitFlags::IDLE
                                | gtk::ApplicationInhibitFlags::SUSPEND,
                            Some(&reason),
                        );
                        self.inhibitor.set((cookie != 0).then_some(cookie));
                    }
                }
            }
            Message::Core(event) => match event {
                Event::TrayChanged(view) => {
                    self.status.set_text(&view.accessibility_label);
                    if let Some(tray) = &self.tray {
                        tray.update(|tray| tray.view = view);
                    }
                }
                Event::DownloadOverlayChanged(DownloadOverlayView::Shown { status, .. }) => {
                    self.detail.set_text(&status)
                }
                // the core also hides the download when a recording or a
                // notice takes the line, and reports that overlay first
                Event::DownloadOverlayChanged(DownloadOverlayView::Hidden) => {
                    if !self.overlay_visible.get() {
                        self.detail.set_text(IDLE_HINT);
                    }
                }
                Event::OverlayChanged(view) => {
                    self.overlay.update(&view);
                    self.overlay_visible.set(view.visible);
                    let (text, level) = super::overlay::content(&view);
                    self.detail
                        .set_text(if view.visible { &text } else { IDLE_HINT });
                    self.meter.set_value(level);
                }
                Event::SpeechChanged(page) => self.speech(*page),
                Event::MenuChanged(menu) => {
                    if let Some(tray) = &self.tray {
                        tray.update(|tray| tray.menu = Some(*menu));
                    }
                }
                Event::GeneralChanged(page) => self.settings(*page),
                Event::HistoryPageChanged(page) => self.history.update(self, *page),
                Event::HotkeyPageChanged(page) => self.hotkey.update(self, *page),
                Event::ReplacementsChanged(page) => self.replacements.update(self, *page),
                Event::AboutChanged(page) => self.about(*page),
                Event::PostProcessingChanged(page) => self.post_processing.update(self, *page),
                Event::HistoryChanged => self.core.app.page_appeared(MainWindowSection::History),
                Event::OpenOnboarding => {
                    self.stack.set_visible_child_name("speech");
                    self.present();
                }
                Event::OpenMainWindow(section) => {
                    self.stack.set_visible_child_name(navigation::name(section));
                    self.present();
                }
                Event::ShowAlert {
                    title,
                    message,
                    button,
                } => {
                    let dialog = adw::AlertDialog::builder()
                        .heading(&title)
                        .body(&message)
                        .build();
                    dialog.add_response("close", button);
                    dialog.set_default_response(Some("close"));
                    self.present();
                    dialog.present(Some(&self.window));
                }
                Event::ShowUpdateNotice(notice) => {
                    // it arrives 750 ms after launch, when Onda is usually
                    // behind other windows or has no window open
                    self.present();
                    let weak = Rc::downgrade(self);
                    self.notice.show(&self.window, &notice, move |choice| {
                        if let Some(this) = weak.upgrade() {
                            this.core.app.update_notice_choice(choice);
                        }
                    });
                }
                Event::CloseUpdateNotice => self.notice.close(),
                Event::ActivateApp => self.present(),
                // #13 renders the shortcut recorder
                Event::HotkeyChanged(_) => {}
                // #15 renders the onboarding wizard
                Event::OnboardingChanged(_) => {}
                // #15 closes the onboarding wizard
                Event::CloseOnboarding => {}
                // #17 renders the Getting Started page
                Event::GettingStartedChanged(_) => {}
                // a GTK application is a regular one whenever a window is
                // open; only macOS switches its Dock presence
                Event::AppPresenceChanged { .. } => {}
            },
        }
    }
}

fn parse_shortcut(text: &str) -> anyhow::Result<onda_domain::hotkey::Shortcut> {
    use onda_domain::hotkey::{KeyCode, Modifier, ModifierSet, Shortcut};
    let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let key = parts
        .pop()
        .ok_or_else(|| anyhow::anyhow!("Enter a shortcut"))?;
    let mut modifiers = ModifierSet::EMPTY;
    for part in parts {
        modifiers = modifiers.with(match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Modifier::Control,
            "alt" => Modifier::Alt,
            "shift" => Modifier::Shift,
            "super" | "logo" => Modifier::Meta,
            _ => anyhow::bail!("Unknown modifier: {part}"),
        });
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "space" => "Space".into(),
        "enter" | "return" => "Enter".into(),
        "tab" => "Tab".into(),
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "backspace" => "Backspace".into(),
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
            format!("Key{}", key.to_ascii_uppercase())
        }
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() => format!("Digit{key}"),
        _ => key.to_owned(),
    };
    let key = KeyCode::parse(&code).ok_or_else(|| anyhow::anyhow!("Invalid key"))?;
    format!("Control+{code}")
        .parse::<global_hotkey::hotkey::HotKey>()
        .map_err(|_| anyhow::anyhow!("Unknown key: {key}"))?;
    let shortcut = Shortcut::new(key, modifiers)
        .map_err(|_| anyhow::anyhow!("Choose a key other than Escape with Ctrl, Alt, or Super."))?;
    super::hotkeys::trigger(&shortcut)?;
    Ok(shortcut)
}

fn page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 28);
    page.set_margin_top(24);
    page.set_margin_bottom(24);
    page.set_margin_start(28);
    page.set_margin_end(28);
    page
}
fn scrolled(child: &gtk::Box) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder()
        .maximum_size(800)
        .tightening_threshold(600)
        .child(child)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build()
}
fn clear(parent: &gtk::Box) {
    while let Some(child) = parent.first_child() {
        parent.remove(&child);
    }
}
fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).wrap(true).build()
}

#[cfg(test)]
mod tests {
    use super::parse_shortcut;

    #[test]
    fn shortcut_editor_round_trips_portal_labels() {
        for input in [
            "Ctrl+Alt+Space",
            "Super+Left",
            "Ctrl+Return",
            "Alt+D",
            "Ctrl+5",
            "Alt+F8",
        ] {
            let shortcut = parse_shortcut(input).unwrap();
            let label = crate::linux::hotkeys::trigger(&shortcut).unwrap();
            assert_eq!(parse_shortcut(&label).unwrap(), shortcut, "{input}");
        }
    }

    #[test]
    fn shortcut_editor_rejects_unknown_or_reserved_combinations() {
        for input in [
            "",
            "Space",
            "Shift+Space",
            "Ctrl+Escape",
            "Ctrl+MadeUpKey",
            "Ctrl+Alt",
        ] {
            assert!(parse_shortcut(input).is_err(), "{input}");
        }
    }
}
