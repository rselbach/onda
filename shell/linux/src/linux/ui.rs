//! GTK widgets render core views and send commands back to the core.

mod about;
mod general;
mod getting_started;
mod history;
mod hotkey;
mod navigation;
mod notice;
mod onboarding;
mod post_processing;
mod recorder;
mod replacements;
mod speech;
mod try_it;

#[cfg(test)]
mod integration_tests;

use adw::prelude::*;
use gtk::{gio, glib};
use onda_app::{Event, views::MainWindowSection};
use onda_domain::non_dictation_ui::DownloadOverlayView;
use std::{cell::Cell, rc::Rc};

use super::{
    Message,
    core::Core,
    overlay::{Overlay, Press},
    tray::Command,
};

use getting_started::IDLE_HINT;

pub struct Window {
    pub core: Core,
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    getting_started: getting_started::Page,
    speech: speech::Page,
    history: history::Page,
    hotkey: hotkey::Page,
    recorder: recorder::KeyMonitor,
    notice: notice::Notice,
    onboarding: onboarding::Onboarding,
    replacements: replacements::Page,
    about: gtk::Box,
    section: Cell<MainWindowSection>,
    general: general::Page,
    post_processing: post_processing::Page,
    toast: adw::ToastOverlay,
    overlay: Overlay,
    overlay_visible: Cell<bool>,
    inhibitor: Cell<Option<u32>>,
    tray: Option<super::tray::TrayHandle>,
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

        let overlay =
            Overlay::new(move |press| super::send(&overlay_events, Message::Overlay(press)));
        let getting_started = getting_started::Page::new(overlay.available());
        stack.add_titled(
            &scrolled(&getting_started.widget),
            Some("dictation"),
            MainWindowSection::GettingStarted.title(),
        );
        let speech = speech::Page::new();
        stack.add_titled(
            &scrolled(&speech.widget),
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
        let general = general::Page::new(core.settings.terminal_paste_shortcut());
        stack.add_titled(
            &scrolled(&general.widget),
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
        let this = Rc::new(Self {
            core,
            window,
            stack,
            getting_started,
            speech,
            history,
            hotkey,
            recorder: recorder::KeyMonitor::default(),
            notice: notice::Notice::default(),
            onboarding: onboarding::Onboarding::default(),
            replacements,
            about,
            section: Cell::new(MainWindowSection::GettingStarted),
            general,
            post_processing,
            toast,
            overlay,
            overlay_visible: Cell::new(false),
            inhibitor: Cell::new(None),
            tray,
        });
        this.getting_started.connect(&this);
        this.speech.connect(&this);
        this.general.connect(&this);
        this.post_processing.connect(&this);
        this.hotkey.connect(&this);
        this.history.connect(&this);
        this.replacements.connect(&this);
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
                if this.window.is_visible() {
                    if previous != section {
                        this.core.app.page_disappeared(previous);
                    }
                    this.core.app.page_appeared(section);
                }
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
        self.getting_started.status()
    }

    fn set_shortcut_status(&self, status: &str) {
        self.getting_started.set_shortcut(status);
        self.hotkey.set_status(status);
    }

    /// The window's surface on Wayland, while the window has one.
    fn toplevel(&self) -> Option<gdk4_wayland::WaylandToplevel> {
        self.window.surface()?.downcast().ok()
    }

    /// Answers with the open window's handle once the compositor has
    /// exported it. A closed window drops `reply`, which reads as no parent.
    fn export_window(&self, reply: tokio::sync::oneshot::Sender<String>) {
        let Some(toplevel) = self.toplevel().filter(|_| self.window.is_visible()) else {
            return;
        };
        let reply = Cell::new(Some(reply));
        toplevel.export_handle(move |toplevel, handle| {
            if let (Some(reply), Ok(handle)) = (reply.take(), handle)
                // the worker stopped waiting, so nothing else releases it
                && let Err(handle) = reply.send(handle.into())
            {
                toplevel.drop_exported_handle(&handle);
            }
        });
    }

    /// A token from the display that lets the file manager raise its
    /// window. GNOME's compositor and KWin honored one asked for while
    /// Onda's window had the focus, and GNOME's no other. `None` when no
    /// application opens folders: X11 names that application in the token.
    fn activation_token(&self) -> Option<String> {
        let files = gio::AppInfo::default_for_type("inode/directory", false)?;
        let token = gtk::prelude::WidgetExt::display(&self.window)
            .app_launch_context()
            .startup_notify_id(Some(&files), &[])?;
        Some(token.into())
    }

    /// The one place a tray choice becomes a core call.
    fn tray(&self, command: Command) {
        let app = &self.core.app;
        match command {
            Command::Show => self.present(),
            Command::MenuWillOpen => app.menu_will_open(),
            Command::FixPermissions => app.fix_permissions(),
            Command::ToggleDictation => {
                self.core.output.held.update(false);
                app.toggle_dictation_from_menu();
            }
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
            Message::Overlay(Press::Cancel) => self.core.app.cancel_dictation(),
            Message::Overlay(Press::Hide) => self.core.app.dismiss_download_overlay(),
            Message::Hotkey(event) => self.core.app.on_hotkey_event(event),
            Message::Download(update) => self.core.app.download_event(update),
            Message::DevicesChanged => self.core.app.input_devices_changed(),
            Message::UpdaterChanged => self.core.app.updater_changed(),
            Message::ShortcutStatus(status) => self.set_shortcut_status(&status),
            Message::ShortcutUnavailable(reason, status) => {
                self.set_shortcut_status(&reason);
                self.core.app.hotkey_registration_failed(status);
            }
            Message::DialogParent(reply) => self.export_window(reply),
            Message::DialogParentReleased(handle) => {
                if let Some(toplevel) = self.toplevel() {
                    toplevel.drop_exported_handle(&handle);
                }
            }
            Message::ActivationToken(reply) => {
                // the receiver is gone once it stopped waiting
                let _ = reply.send(self.activation_token());
            }
            Message::Notice(title, body) => {
                let toast = adw::Toast::new(&format!("{title}: {body}"));
                toast.set_use_markup(false);
                self.toast.add_toast(toast);
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
                    self.getting_started.set_status(&view.accessibility_label);
                    if let Some(tray) = &self.tray {
                        tray.set_view(view);
                    }
                }
                Event::DownloadOverlayChanged(view) => {
                    self.overlay.download(&view);
                    match view {
                        DownloadOverlayView::Shown { status, .. } => {
                            self.getting_started.set_detail(&status);
                        }
                        // the core also hides the download when a recording
                        // or a notice takes the line, and reports that
                        // overlay first
                        DownloadOverlayView::Hidden => {
                            if !self.overlay_visible.get() {
                                self.getting_started.set_detail(IDLE_HINT);
                            }
                        }
                    }
                }
                Event::OverlayChanged(view) => {
                    self.overlay.update(&view);
                    self.overlay_visible.set(view.visible);
                    let (text, level) = super::overlay::content(&view);
                    self.getting_started
                        .set_detail(if view.visible { &text } else { IDLE_HINT });
                    self.getting_started.set_level(level);
                }
                Event::SpeechChanged(page) => self.speech.update(self, *page),
                Event::MenuChanged(menu) => {
                    if let Some(tray) = &self.tray {
                        tray.set_menu(*menu);
                    }
                }
                Event::GeneralChanged(page) => self.general.update(self, *page),
                Event::HistoryPageChanged(page) => self.history.update(self, *page),
                Event::HotkeyPageChanged(page) => self.hotkey.update(*page),
                Event::ReplacementsChanged(page) => self.replacements.update(self, *page),
                Event::AboutChanged(page) => self.about(*page),
                Event::PostProcessingChanged(page) => self.post_processing.update(self, *page),
                Event::HistoryChanged => {}
                Event::OpenOnboarding => self.onboarding.open(self),
                Event::OnboardingChanged(view) => self.onboarding.update(*view),
                Event::CloseOnboarding => self.onboarding.close(),
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
                Event::HotkeyChanged(view) => {
                    self.recorder.render(self, &view);
                    self.hotkey.render(&view);
                    self.onboarding.render(&view);
                }
                Event::GettingStartedChanged(page) => self.getting_started.update(self, *page),
                // a GTK application is a regular one whenever a window is
                // open; only macOS switches its Dock presence
                Event::AppPresenceChanged { .. } => {}
            },
        }
    }
}

// the icon theme has Onda's icon only where Onda is installed
fn app_icon() -> Option<gtk::gdk::Texture> {
    let bytes = glib::Bytes::from_static(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/AppIcon.appiconset/icon_256x256.png"
    )));
    gtk::gdk::Texture::from_bytes(&bytes).ok()
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
