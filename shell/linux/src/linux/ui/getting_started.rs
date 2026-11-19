//! The Getting Started page (WIN-4..WIN-7): the core's setup checklist and
//! try-it field, and below them the live dictation status that only the
//! Linux window shows.

use super::{Window, label, page, recorder, try_it::TryIt};
use adw::prelude::*;
use onda_app::views::{ChecklistRow, GettingStartedPage};
use onda_domain::setup::SetupStepStatus;
use std::{cell::RefCell, rc::Rc};

pub(super) const IDLE_HINT: &str = "Use your global shortcut to dictate into another application.";

pub(super) struct Page {
    pub widget: gtk::Box,
    ready: gtk::Box,
    ready_line: gtk::Label,
    setup: adw::PreferencesGroup,
    // the checklist as the core last sent it, each row with its widget
    steps: RefCell<Vec<(ChecklistRow, adw::ActionRow)>>,
    run_setup_again: gtk::Button,
    prefix: gtk::Label,
    keycaps: gtk::Box,
    suffix: gtk::Label,
    try_it: TryIt,
    // the tray's status as the title, the overlay's text as the subtitle
    status: adw::ActionRow,
    meter: gtk::LevelBar,
    record: gtk::Button,
    cancel: gtk::Button,
    shortcut: adw::ActionRow,
}

impl Page {
    /// `indicator` is whether this desktop shows the floating overlay.
    pub fn new(indicator: bool) -> Self {
        let widget = page();
        let ready_line = gtk::Label::new(None);
        let ready = gtk::Box::builder()
            .spacing(6)
            .css_classes(["success"])
            .visible(false)
            .build();
        ready.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        ready.append(&ready_line);
        let setup = adw::PreferencesGroup::builder()
            .title("Setup")
            .header_suffix(&ready)
            .build();
        let run_setup_again = gtk::Button::builder()
            .halign(gtk::Align::Start)
            .margin_top(12)
            .build();
        setup.add(&run_setup_again);
        widget.append(&setup);

        let prefix = gtk::Label::builder().css_classes(["dim-label"]).build();
        let keycaps = recorder::keycaps();
        let suffix = gtk::Label::builder().css_classes(["dim-label"]).build();
        let prompt = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        prompt.append(&prefix);
        prompt.append(&keycaps);
        prompt.append(&suffix);
        let try_it = TryIt::new();
        try_it.widget.set_margin_top(12);
        let group = adw::PreferencesGroup::builder().title("Try It").build();
        group.add(&prompt);
        group.add(&try_it.widget);
        widget.append(&group);

        let status = adw::ActionRow::builder()
            .title("Welcome to Onda")
            .subtitle(IDLE_HINT)
            .use_markup(false)
            .build();
        let meter = gtk::LevelBar::builder()
            .width_request(96)
            .valign(gtk::Align::Center)
            .build();
        status.add_suffix(&meter);
        // a click keeps the caret where it is, so a dictation started here
        // can land in the try-it field
        let button = |title: &str| {
            let button = gtk::Button::builder()
                .label(title)
                .valign(gtk::Align::Center)
                .focus_on_click(false)
                .build();
            status.add_suffix(&button);
            button
        };
        let record = button("Start / Stop");
        let cancel = button("Cancel");
        let shortcut = adw::ActionRow::builder()
            .title("Desktop shortcut")
            .subtitle("Preparing the global shortcut…")
            .use_markup(false)
            .build();
        let group = adw::PreferencesGroup::builder().title("Dictation").build();
        group.add(&status);
        group.add(&shortcut);
        let mut notes = vec![
            "Close this window to keep Onda running. Reopen it from your application launcher. \
             Use Quit to stop Onda.",
        ];
        if !indicator {
            notes.push(
                "A floating indicator is unavailable on this desktop. Recording and download \
                 status remain visible here.",
            );
        }
        for note in notes {
            let note = label(note);
            note.set_xalign(0.0);
            note.set_margin_top(12);
            note.add_css_class("dim-label");
            group.add(&note);
        }
        widget.append(&group);
        Self {
            widget,
            ready,
            ready_line,
            setup,
            steps: RefCell::default(),
            run_setup_again,
            prefix,
            keycaps,
            suffix,
            try_it,
            status,
            meter,
            record,
            cancel,
            shortcut,
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.run_setup_again.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.run_setup_again();
            }
        });
        let weak = Rc::downgrade(owner);
        self.record.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.output.held.update(false);
                owner.core.app.toggle_dictation_from_menu();
            }
        });
        let weak = Rc::downgrade(owner);
        self.cancel.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.cancel_dictation();
            }
        });
    }

    pub fn update(&self, owner: &Rc<Window>, view: GettingStartedPage) {
        self.ready.set_visible(view.ready_line.is_some());
        self.ready_line
            .set_label(&view.ready_line.unwrap_or_default());
        self.run_setup_again.set_label(&view.run_setup_again_title);
        self.prefix.set_label(&view.try_it_prefix);
        recorder::set_keycaps(&self.keycaps, &view.keycaps);
        self.suffix.set_label(&view.try_it_suffix);
        self.try_it.set_placeholder(&view.try_it_placeholder);
        // the shortcut and the output mode change the view too, and a button
        // under the pointer must outlive those
        let mut steps = self.steps.borrow_mut();
        if steps.iter().map(|(step, _)| step).eq(&view.steps) {
            return;
        }
        for (_, row) in steps.drain(..) {
            self.setup.remove(&row);
        }
        for step in view.steps {
            let row = row(owner, &step);
            self.setup.add(&row);
            steps.push((step, row));
        }
    }

    pub fn status(&self) -> String {
        self.status.title().into()
    }

    pub fn set_status(&self, status: &str) {
        self.status.set_title(status);
    }

    #[cfg(test)]
    pub fn detail(&self) -> String {
        self.status.subtitle().unwrap_or_default().into()
    }

    pub fn set_detail(&self, detail: &str) {
        self.status.set_subtitle(detail);
    }

    pub fn set_level(&self, level: f64) {
        self.meter.set_value(level);
    }

    #[cfg(test)]
    pub fn shortcut(&self) -> String {
        self.shortcut.subtitle().unwrap_or_default().into()
    }

    pub fn set_shortcut(&self, status: &str) {
        self.shortcut.set_subtitle(status);
    }
}

/// One checklist row: the status icon, the title, the message while the
/// step is incomplete, and the button when the step needs the user (WIN-5).
fn row(owner: &Rc<Window>, step: &ChecklistRow) -> adw::ActionRow {
    let (icon, color) = match step.status {
        SetupStepStatus::Complete => ("object-select-symbolic", "success"),
        SetupStepStatus::NeedsAction => ("dialog-warning-symbolic", "warning"),
        SetupStepStatus::InProgress => ("folder-download-symbolic", "accent"),
    };
    let row = adw::ActionRow::builder()
        .title(&step.title)
        .subtitle(step.message.as_deref().unwrap_or_default())
        .use_markup(false)
        .build();
    let icon = gtk::Image::from_icon_name(icon);
    icon.add_css_class(color);
    row.add_prefix(&icon);
    if let Some(button) = &step.button {
        let action = button.action;
        let button = gtk::Button::builder()
            .label(&button.title)
            .valign(gtk::Align::Center)
            .css_classes(["suggested-action"])
            .build();
        let weak = Rc::downgrade(owner);
        button.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.setup_action(action);
            }
        });
        row.add_suffix(&button);
    }
    row
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use adw::prelude::*;
    use onda_app::{
        Event,
        views::{ChecklistRow, GettingStartedPage},
    };
    use onda_domain::{
        hotkey::Shortcut,
        setup::{SetupStepId, SetupStepStatus},
        types::OutputMode,
    };

    use super::super::{
        Window,
        integration_tests::{
            Start, button, capture, focus_onda, launch, text_visible, wait_for, widgets,
        },
    };
    use crate::linux::Message;

    const TRANSCRIPT: &str = "Troy and Abed in the morning.";

    fn steps(owner: &Window) -> Vec<adw::ActionRow> {
        widgets::<adw::ActionRow>(&owner.getting_started.setup)
    }

    // the icon name and the color class of the row's status icon
    fn status(row: &adw::ActionRow) -> (String, Vec<String>) {
        let icon = widgets::<gtk::Image>(row).remove(0);
        (
            icon.icon_name().unwrap_or_default().into(),
            icon.css_classes()
                .iter()
                .map(|class| class.to_string())
                .collect(),
        )
    }

    fn keycaps(owner: &Window) -> Vec<String> {
        widgets::<gtk::Label>(&owner.getting_started.keycaps)
            .iter()
            .map(|label| label.text().into())
            .collect()
    }

    fn try_it_text(owner: &Window) -> String {
        let buffer = owner.getting_started.try_it.text.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .into()
    }

    fn quit(owner: &Window) {
        owner.core.runtime.block_on(owner.core.app.shutdown_async());
        owner.window.close();
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_a_missing_model_is_left_to_set_up() {
        let owner = launch(Start::Declined);
        // a declined model opens the Speech page once the app is up
        wait_for(|| owner.stack.visible_child_name().as_deref() == Some("speech"));
        owner.stack.set_visible_child_name("dictation");
        let page = &owner.getting_started;
        wait_for(|| steps(&owner).len() == 1);
        let step = steps(&owner).remove(0);
        assert_eq!(step.title(), "Speech Model");
        assert_eq!(
            step.subtitle().as_deref(),
            Some("Download a local model before dictating.")
        );
        assert_eq!(
            status(&step),
            ("dialog-warning-symbolic".into(), vec!["warning".to_owned()])
        );
        let download = button(&step, "Download Base Model");
        assert!(!page.ready.is_visible(), "a step is left");
        assert!(text_visible(&page.widget, "Press") && text_visible(&page.widget, "and speak."));
        assert_eq!(keycaps(&owner), ["Ctrl", "Alt", "Space"]);
        wait_for(|| page.shortcut() == "Control+Alt+Space");
        let placeholder = widgets::<gtk::Label>(&page.try_it.widget).remove(0);
        assert_eq!(placeholder.text(), "Click here and try it.");
        assert!(placeholder.is_visible());
        capture(&owner, "getting-started-model-missing");

        // the page follows the shortcut and the output mode, and keeps the
        // row a pointer may be over
        owner.core.app.set_hotkey(Shortcut::ALT_SUPER_SPACE);
        wait_for(|| keycaps(&owner) == ["Alt", "Super", "Space"]);
        owner.core.app.set_output_mode(OutputMode::Clipboard);
        wait_for(|| placeholder.text() == "Click here, dictate, then paste with Ctrl+V.");
        assert_eq!(button(&steps(&owner)[0], "Download Base Model"), download);
        owner.core.app.set_output_mode(OutputMode::Type);
        wait_for(|| placeholder.text() == "Click here and try it.");

        // the paste goes to the X server's focus, which is the try-it field
        // once the window has the keyboard
        focus_onda();
        page.try_it.text.grab_focus();
        owner.core.app.deliver_transcript(TRANSCRIPT);
        wait_for(|| try_it_text(&owner) == TRANSCRIPT);
        assert!(!placeholder.is_visible());

        button(&page.widget, "Run Setup Again").emit_clicked();
        wait_for(|| owner.onboarding.window().is_some());
        let wizard = owner.onboarding.window().unwrap();
        wait_for(|| text_visible(&wizard, "Welcome to Onda"));
        wizard.close();
        wait_for(|| owner.onboarding.window().is_none());
        // a rerun is pending only while its wizard is open
        wait_for(|| owner.core.settings.onboarding_completed());
        assert_eq!(try_it_text(&owner), TRANSCRIPT, "the page keeps its text");

        // a download in the core's place: the test session has no network
        let downloading = GettingStartedPage {
            ready_line: None,
            steps: vec![ChecklistRow {
                step: SetupStepId::SpeechModel,
                title: "Speech Model".into(),
                status: SetupStepStatus::InProgress,
                message: Some("Onda is downloading a local transcription model.".into()),
                button: None,
            }],
            run_setup_again_title: "Run Setup Again".into(),
            try_it_prefix: "Press".into(),
            keycaps: vec!["Alt".into(), "Super".into(), "Space".into()],
            try_it_suffix: "and speak.".into(),
            try_it_placeholder: "Click here and try it.".into(),
        };
        owner.event(Message::Core(Event::GettingStartedChanged(Box::new(
            downloading,
        ))));
        let step = steps(&owner).remove(0);
        assert_eq!(
            step.subtitle().as_deref(),
            Some("Onda is downloading a local transcription model.")
        );
        assert_eq!(
            status(&step),
            ("folder-download-symbolic".into(), vec!["accent".to_owned()])
        );
        assert!(widgets::<gtk::Button>(&step).is_empty());
        capture(&owner, "getting-started-downloading");
        quit(&owner);
    }

    #[test]
    #[ignore = "requires an isolated GTK display, D-Bus session, and ONDA_DATA_DIR"]
    fn real_gui_an_installed_model_leaves_nothing_to_set_up() {
        let owner = launch(Start::Installed);
        owner.present();
        let page = &owner.getting_started;
        wait_for(|| page.ready.is_visible());
        assert!(text_visible(&page.ready, "Onda is ready"));
        let step = steps(&owner).remove(0);
        assert_eq!(steps(&owner).len(), 1);
        assert_eq!(step.title(), "Speech Model");
        assert_eq!(step.subtitle().as_deref(), Some(""));
        assert_eq!(
            status(&step),
            ("object-select-symbolic".into(), vec!["success".to_owned()])
        );
        assert!(widgets::<gtk::Button>(&step).is_empty());
        assert!(button(&page.widget, "Run Setup Again").is_visible());
        if std::env::var_os("ONDA_TEST_SPEECH_MODEL").is_some() {
            // checking and loading a real model outlasts `wait_for`
            let deadline = Instant::now() + Duration::from_secs(180);
            while owner.status() != "Onda, ready to dictate" {
                assert!(Instant::now() < deadline, "{}", owner.status());
                wait_for(|| true);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        capture(&owner, "getting-started-ready");
        quit(&owner);
    }
}
