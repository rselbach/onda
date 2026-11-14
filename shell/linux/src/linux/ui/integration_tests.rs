use super::{super::Message, Window};
use adw::prelude::*;
use gtk::gio;
use onda_adapters::DataPaths;
use onda_app::Event;
use onda_domain::{
    models::ModelId,
    non_dictation_ui::{DownloadOverlayView, OverlayProgress},
    overlay::{OverlayContent, OverlayIcon, OverlayView},
};
use std::{
    cell::RefCell,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{ConnectionExt, GrabMode, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, ModMask},
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// The key of the update notice that [`Start`] makes due.
pub(super) const NOTICE_KEY: &str = "parakeet-ja->parakeet-ultra";

/// The model the wizard recommends for English.
pub(super) const MODEL: ModelId = ModelId::ParakeetTdtV2;

/// The settings a real-GUI test starts from. None of them makes the app
/// download a model by itself.
pub(super) enum Start {
    /// No setting is stored, so the wizard opens. The recommended model
    /// downloads when the user leaves the wizard's Language step.
    FirstRun,
    /// Setup is finished and the selected model has left the catalog, so
    /// the update notice is due and the user's answer decides the download.
    NoticeDue,
    /// As `NoticeDue`, but the user declined the replacement model: no
    /// notice and no download.
    Declined,
    /// Setup is finished and [`MODEL`] is selected and in the data
    /// directory, as [`seed_model`] leaves it.
    Installed,
}

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ONDA_DATA_DIR").expect("use a scratch data directory"))
}

/// Puts [`MODEL`] in the data directory, so the app finds it downloaded and
/// asks the network for nothing. `ONDA_TEST_SPEECH_MODEL` names a real copy
/// of the file, which also loads. Without it the file has the right size and
/// no content: it counts as downloaded and then fails its check at load.
pub(super) fn seed_model() -> bool {
    let file = &crate::linux::speech::manifest(MODEL).unwrap().files[0];
    let folder = DataPaths::new(data_dir()).models_dir().join(MODEL.as_str());
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(file.path);
    match std::env::var_os("ONDA_TEST_SPEECH_MODEL") {
        Some(real) => {
            std::os::unix::fs::symlink(real, path).unwrap();
            true
        }
        None => {
            std::fs::File::create_new(path)
                .unwrap()
                .set_len(file.size)
                .unwrap();
            false
        }
    }
}

/// Starts the app on the data directory of the test session.
pub(super) fn launch(start: Start) -> Rc<Window> {
    let directory = data_dir();
    let finished = serde_json::json!({
        "onboardingCompleted": true,
        "didShowFirstRunSetup": true,
        "selectedModel": "parakeet-ja",
    });
    let settings = match start {
        Start::FirstRun => serde_json::json!({}),
        Start::NoticeDue => finished,
        Start::Declined => {
            let mut settings = finished;
            settings["declinedModelMigrationNoticeKey"] = NOTICE_KEY.into();
            settings["lastModelMigrationNoticeKey"] = NOTICE_KEY.into();
            settings
        }
        Start::Installed => {
            seed_model();
            let mut settings = finished;
            settings["selectedModel"] = MODEL.as_str().into();
            settings["selectedLanguage"] = "en".into();
            settings
        }
    };
    // fails in a directory that holds settings, so a run cannot replace the
    // settings of a real installation
    std::fs::File::create_new(directory.join("settings.json"))
        .expect("a data directory without settings")
        .write_all(settings.to_string().as_bytes())
        .unwrap();
    adw::init().unwrap();
    let application = adw::Application::builder()
        .application_id(crate::linux::application_id())
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    Window::new(&application).unwrap()
}

pub(super) fn seed_history() {
    struct Hooks;
    impl onda_history::HistoryHooks for Hooks {
        fn history_changed(&self) {}
        fn exclude_from_backup(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }
    }
    let directory = std::path::PathBuf::from(std::env::var_os("ONDA_DATA_DIR").unwrap());
    let store = onda_history::HistoryStore::new(directory.join("History"), Hooks);
    struct Seed {
        transcript: &'static str,
        raw: Option<&'static str>,
        samples: &'static [f32],
    }
    for seed in [
        Seed {
            transcript: "Troy Barnes is organizing the Greendale study group.",
            raw: None,
            samples: &[],
        },
        Seed {
            transcript: "Remember to return the library books tomorrow.",
            raw: None,
            samples: &[],
        },
        Seed {
            transcript: "Abed Nadir is filming a documentary.",
            raw: Some("um abed nadir is like filming a documentary"),
            samples: &[],
        },
        Seed {
            transcript: "Annie Edison booked the study room.",
            raw: None,
            samples: &[0.0; 1600],
        },
    ] {
        let now = SystemTime::now().try_into().unwrap();
        store
            .save(
                onda_domain::history::NewEntry {
                    id: uuid::Uuid::new_v4(),
                    timestamp: now,
                    sample_count: 96000,
                    model_id: "parakeet-tdt-v2".into(),
                    model_name: Some("Parakeet TDT v2".into()),
                    language: "en".into(),
                    transcript: seed.transcript.into(),
                    raw_transcript: seed.raw.map(Into::into),
                    was_post_processed: seed.raw.is_some(),
                    post_processing_error_description: None,
                    app_name: None,
                    app_bundle_id: None,
                    audio_byte_count: None,
                },
                seed.samples,
                onda_history::HistoryPreferences {
                    enabled: true,
                    keep_audio: !seed.samples.is_empty(),
                    retention: onda_domain::types::HistoryRetention::Month,
                },
                now,
            )
            .unwrap();
    }
}

#[track_caller]
pub(super) fn wait_for(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let context = gtk::glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "GUI state did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn widgets<T: IsA<gtk::Widget> + Clone + 'static>(
    root: &impl IsA<gtk::Widget>,
) -> Vec<T> {
    let root = root.as_ref();
    let mut result = Vec::new();
    if let Ok(widget) = root.clone().downcast::<T>() {
        result.push(widget);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        result.extend(widgets::<T>(&widget));
        child = widget.next_sibling();
    }
    result
}

pub(super) fn text_visible(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
    widgets::<gtk::Label>(root)
        .iter()
        .any(|label| label.text().contains(text))
}

pub(super) fn button(root: &impl IsA<gtk::Widget>, title: &str) -> gtk::Button {
    widgets::<gtk::Button>(root)
        .into_iter()
        .find(|button| button.label().as_deref() == Some(title))
        .unwrap()
}

fn options(row: &adw::ComboRow) -> Vec<String> {
    let model = row.model().and_downcast::<gtk::StringList>().unwrap();
    (0..model.n_items())
        .filter_map(|index| model.string(index))
        .map(String::from)
        .collect()
}

/// The X11 window on screen whose title is `name`.
pub(super) fn x11_window(name: &str) -> Option<u32> {
    use x11rb::{connection::Connection, protocol::xproto::*};
    let (connection, screen) = x11rb::connect(None).unwrap();
    let root = connection.setup().roots[screen].root;
    let windows = connection.query_tree(root).unwrap().reply().unwrap();
    windows.children.into_iter().find(|&window| {
        connection
            .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 64)
            .unwrap()
            .reply()
            .is_ok_and(|title| title.value == name.as_bytes())
            && connection
                .get_window_attributes(window)
                .unwrap()
                .reply()
                .is_ok_and(|attributes| attributes.map_state == MapState::VIEWABLE)
    })
}

/// Gives the main window the keyboard.
pub(super) fn focus_onda() {
    focus("Onda");
}

/// Gives the window with this title the keyboard, which no window manager
/// does here.
pub(super) fn focus(title: &str) {
    use x11rb::{protocol::xproto::*, wrapper::ConnectionExt as _};
    wait_for(|| x11_window(title).is_some());
    let (connection, _) = x11rb::connect(None).unwrap();
    connection
        .set_input_focus(
            InputFocus::POINTER_ROOT,
            x11_window(title).unwrap(),
            x11rb::CURRENT_TIME,
        )
        .unwrap();
    connection.sync().unwrap();
}

// hardware keycodes, which name a key's place whatever the layout
pub(super) const ESCAPE: u8 = 9;
pub(super) const CONTROL_LEFT: u8 = 37;
pub(super) const KEY_Q: u8 = 24;
pub(super) const KEY_A: u8 = 38;
pub(super) const KEY_G: u8 = 42;
pub(super) const ALT_LEFT: u8 = 64;
pub(super) const SPACE: u8 = 65;

/// Another application that wants a key with Ctrl and Alt.
pub(super) struct Rival {
    connection: RustConnection,
    root: u32,
}

impl Rival {
    pub fn connect() -> Self {
        let (connection, screen) = x11rb::connect(None).unwrap();
        let root = connection.setup().roots[screen].root;
        Self { connection, root }
    }

    // the X server grants a key to one application at a time
    pub fn grab(&self, key: u8) -> bool {
        self.connection
            .grab_key(
                false,
                self.root,
                ModMask::CONTROL | ModMask::M1,
                key,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .unwrap()
            .check()
            .is_ok()
    }

    pub fn release(&self, key: u8) {
        self.connection
            .ungrab_key(key, self.root, ModMask::CONTROL | ModMask::M1)
            .unwrap()
            .check()
            .unwrap();
    }
}

/// The user's hands: key presses that reach Onda's window as a real
/// keyboard's do.
pub(super) struct Keyboard {
    connection: RustConnection,
    root: u32,
}

impl Keyboard {
    pub fn connect() -> Self {
        let (connection, screen) = x11rb::connect(None).unwrap();
        let root = connection.setup().roots[screen].root;
        Self { connection, root }
    }

    fn send(&self, kind: u8, keys: &[u8]) {
        for &key in keys {
            self.connection
                .xtest_fake_input(kind, key, x11rb::CURRENT_TIME, self.root, 0, 0, 0)
                .unwrap();
        }
        self.connection.sync().unwrap();
    }

    /// Swaps what two keys type, as a layout other than the US one does.
    pub fn swap(&self, a: u8, b: u8) {
        let typed = |key| {
            self.connection
                .get_keyboard_mapping(key, 1)
                .unwrap()
                .reply()
                .unwrap()
        };
        let (first, second) = (typed(a), typed(b));
        for (key, mapping) in [(a, second), (b, first)] {
            self.connection
                .change_keyboard_mapping(1, key, mapping.keysyms_per_keycode, &mapping.keysyms)
                .unwrap();
        }
        self.connection.sync().unwrap();
    }

    pub fn press(&self, keys: &[u8]) {
        self.send(KEY_PRESS_EVENT, keys);
    }

    pub fn release(&self, keys: &[u8]) {
        self.send(KEY_RELEASE_EVENT, keys);
    }

    pub fn tap(&self, held: &[u8], key: u8) {
        self.press(held);
        self.press(&[key]);
        self.release(&[key]);
        self.release(held);
    }
}

// whether the floating overlay's X11 window is on screen
fn overlay_on_screen() -> bool {
    x11_window("onda-recording").is_some()
}

pub(super) fn settle() {
    let until = Instant::now() + Duration::from_millis(350);
    wait_for(|| Instant::now() >= until);
}

fn row_menu(owner: &Window, transcript: &str) -> gtk::MenuButton {
    widgets::<gtk::MenuButton>(&owner.history.widget)
        .into_iter()
        .find(|button| {
            let row = button.parent().and_then(|top| top.parent()).unwrap();
            text_visible(&row, transcript)
        })
        .unwrap()
}

// title and action of each item, top to bottom, with "-" between sections
fn menu_items(button: &gtk::MenuButton) -> Vec<(String, String)> {
    let text = |model: &gio::MenuModel, item, attribute| {
        model
            .item_attribute_value(item, attribute, None)
            .and_then(|value| value.str().map(str::to_owned))
            .unwrap()
    };
    let menu = button.menu_model().unwrap();
    let mut items = Vec::new();
    for index in 0..menu.n_items() {
        if index > 0 {
            items.push(("-".into(), String::new()));
        }
        let section = menu.item_link(index, "section").unwrap();
        for item in 0..section.n_items() {
            items.push((
                text(&section, item, "label"),
                text(&section, item, "action"),
            ));
        }
    }
    items
}

fn menu_titles(owner: &Window, transcript: &str) -> Vec<String> {
    menu_items(&row_menu(owner, transcript))
        .into_iter()
        .map(|(title, _)| title)
        .collect()
}

fn choose(owner: &Window, transcript: &str, title: &str) {
    let button = row_menu(owner, transcript);
    let (_, action) = menu_items(&button)
        .into_iter()
        .find(|(item, _)| item == title)
        .unwrap();
    button.activate_action(&action, None).unwrap();
}

// stands in for a file manager on the test bus and records the URIs it is
// asked to select
fn file_manager(shown: &Rc<RefCell<Vec<String>>>) -> impl FnOnce() {
    const NAME: &str = "org.freedesktop.FileManager1";
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    let node = gio::DBusNodeInfo::for_xml(
        r#"<node><interface name="org.freedesktop.FileManager1">
            <method name="ShowItems">
                <arg type="as" direction="in"/><arg type="s" direction="in"/>
            </method>
        </interface></node>"#,
    )
    .unwrap();
    let shown = shown.clone();
    let object = connection
        .register_object(
            "/org/freedesktop/FileManager1",
            &node.lookup_interface(NAME).unwrap(),
        )
        .method_call(move |_, _, _, _, _, parameters, invocation| {
            let (uris, _) = parameters.get::<(Vec<String>, String)>().unwrap();
            shown.borrow_mut().extend(uris);
            invocation.return_value(None);
        })
        .build()
        .unwrap();
    let acquired = Rc::new(std::cell::Cell::new(false));
    let flag = acquired.clone();
    let name = gio::bus_own_name_on_connection(
        &connection,
        NAME,
        gio::BusNameOwnerFlags::NONE,
        move |_, _| flag.set(true),
        |_, _| {},
    );
    wait_for(|| acquired.get());
    move || {
        gio::bus_unown_name(name);
        connection.unregister_object(object).unwrap();
    }
}

pub(super) fn capture(owner: &Window, name: &str) {
    capture_window(&owner.window, name);
}

pub(super) fn capture_window(window: &impl IsA<gtk::Window>, name: &str) {
    let Some(directory) = std::env::var_os("ONDA_TEST_SCREENSHOT_DIR") else {
        return;
    };
    let window = window.as_ref();
    settle();
    // a window that awaits layout paints nothing
    let node = RefCell::new(None);
    wait_for(|| {
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(window)).snapshot(
            &snapshot,
            window.width() as f64,
            window.height() as f64,
        );
        node.replace(snapshot.to_node());
        node.borrow().is_some()
    });
    let node = node.into_inner().unwrap();
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    window
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png(directory.join(format!("{name}.png")))
        .unwrap();
}

pub(super) fn exercise_pages(owner: &Rc<Window>) {
    owner.stack.set_visible_child_name("replacements");
    wait_for(|| text_visible(&owner.replacements.widget, "No rules yet"));
    button(&owner.replacements.widget, "Add Rule").emit_clicked();
    wait_for(|| !widgets::<adw::EntryRow>(&owner.replacements.widget).is_empty());
    let entries = widgets::<adw::EntryRow>(&owner.replacements.widget);
    entries[0].set_text("greendale, greendale cc");
    entries[1].set_text("Greendale Community College");
    wait_for(|| owner.core.settings.replacement_rules().len() == 1);
    assert_eq!(
        entries[0],
        widgets::<adw::EntryRow>(&owner.replacements.widget)[0],
        "typing must preserve the field and focus"
    );
    capture(owner, "replacements");
    owner.stack.set_visible_child_name("general");
    wait_for(|| {
        widgets::<adw::SwitchRow>(&owner.general)
            .iter()
            .any(|row| row.title() == "Check for updates automatically")
    });
    let automatic = widgets::<adw::SwitchRow>(&owner.general)
        .into_iter()
        .find(|row| row.title() == "Check for updates automatically")
        .unwrap();
    assert!(automatic.is_active());
    assert!(button(&owner.general, "Check for Updates…").is_sensitive());
    automatic.set_active(false);
    wait_for(|| {
        std::fs::read(
            super::super::core::data_dir()
                .unwrap()
                .join("updates/updater.json"),
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|prefs| prefs["automatic_checks"] == false)
    });
    let media = widgets::<adw::SwitchRow>(&owner.general)
        .into_iter()
        .find(|row| row.title() == "Pause media while recording")
        .unwrap();
    assert!(!media.is_active());
    assert!(
        media
            .subtitle()
            .is_some_and(|caption| caption.contains("resumes only if Onda paused it"))
    );
    media.set_active(true);
    wait_for(|| owner.core.settings.pause_media_during_recording());
    wait_for(|| {
        owner
            .core
            .settings
            .replacement_rules()
            .first()
            .is_some_and(|rule| rule.replacement == "Greendale Community College")
    });
    owner.stack.set_visible_child_name("replacements");
    wait_for(|| {
        widgets::<adw::EntryRow>(&owner.replacements.widget)
            .get(1)
            .is_some_and(|entry| entry.text() == "Greendale Community College")
    });

    owner.stack.set_visible_child_name("hotkey");
    wait_for(|| text_visible(&owner.hotkey.widget, "Activation"));
    assert!(text_visible(&owner.hotkey.widget, "Press to talk"));
    let activation = widgets::<adw::ComboRow>(&owner.hotkey.widget).remove(0);
    assert_eq!(activation.title(), "Activation");
    assert_eq!(options(&activation), ["Hold", "Toggle", "Automatic"]);

    owner.stack.set_visible_child_name("history");
    wait_for(|| text_visible(&owner.history.widget, "Troy Barnes"));
    assert!(text_visible(
        &owner.history.widget,
        "Saved only on this computer."
    ));
    assert!(!text_visible(&owner.history.widget, "backups"));
    let retention = widgets::<adw::ComboRow>(&owner.history.widget).remove(0);
    assert_eq!(retention.title(), "Keep for");
    assert_eq!(options(&retention), ["1 Week", "1 Month", "Forever"]);
    capture(owner, "history");
    assert_eq!(menu_titles(owner, "Troy Barnes"), ["Copy", "-", "Delete"]);
    assert_eq!(
        menu_titles(owner, "Abed Nadir"),
        ["Copy", "Copy Original", "-", "Delete"]
    );
    assert_eq!(
        menu_titles(owner, "Annie Edison"),
        [
            "Copy",
            "Play Recording",
            "Show Recording in Folder",
            "-",
            "Delete"
        ]
    );
    let menu = row_menu(owner, "Troy Barnes");
    menu.popup();
    wait_for(|| menu.is_active());
    owner.core.app.search_history("library");
    settle();
    assert!(
        menu.is_active() && text_visible(&owner.history.widget, "Troy Barnes"),
        "a page update must not close an open row menu"
    );
    menu.popdown();
    wait_for(|| !text_visible(&owner.history.widget, "Troy Barnes"));
    let menu = row_menu(owner, "library books");
    menu.popup();
    wait_for(|| menu.is_active());
    owner
        .core
        .app
        .search_history("no-matching-greendale-transcript");
    settle();
    menu.popdown();
    owner.core.app.search_history("");
    // the newer view is waiting when the main loop next runs
    std::thread::sleep(Duration::from_millis(300));
    settle();
    assert!(
        text_visible(&owner.history.widget, "Troy Barnes"),
        "a held view must not replace a newer one"
    );
    choose(owner, "Abed Nadir", "Copy Original");
    wait_for(|| {
        super::super::process::run("xclip", &["-selection", "clipboard", "-out"], &[])
            .is_ok_and(|text| text == b"um abed nadir is like filming a documentary")
    });
    let shown = Rc::new(RefCell::new(Vec::new()));
    let close_file_manager = file_manager(&shown);
    choose(owner, "Annie Edison", "Show Recording in Folder");
    wait_for(|| !shown.borrow().is_empty());
    close_file_manager();
    let recording = gio::File::for_uri(&shown.borrow()[0]).path().unwrap();
    assert!(
        recording.starts_with(std::env::var_os("ONDA_DATA_DIR").unwrap()) && recording.is_file(),
        "{recording:?}"
    );
    choose(owner, "Abed Nadir", "Delete");
    wait_for(|| !text_visible(&owner.history.widget, "Abed Nadir"));
    assert!(text_visible(&owner.history.widget, "Annie Edison"));
    let search = widgets::<gtk::SearchEntry>(&owner.history.widget)
        .into_iter()
        .find(|entry| entry.placeholder_text().as_deref() == Some("Search transcripts"))
        .unwrap();
    search.set_text("library");
    wait_for(|| !text_visible(&owner.history.widget, "Troy Barnes"));
    assert!(text_visible(&owner.history.widget, "library books"));
    assert_eq!(search.text().as_str(), "library");
    search.set_text("no-matching-greendale-transcript");
    wait_for(|| text_visible(&owner.history.widget, "No matching transcripts"));
    search.set_text("");
    wait_for(|| text_visible(&owner.history.widget, "Troy Barnes"));
    owner.core.app.request_clear_history();
    wait_for(|| owner.window.visible_dialog().is_some());
    let dialog = owner
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    dialog.close();
    wait_for(|| owner.window.visible_dialog().is_none());
    assert!(text_visible(&owner.history.widget, "Troy Barnes"));
    owner.window.close();
    owner.present();
    search.set_text("library");
    wait_for(|| !text_visible(&owner.history.widget, "Troy Barnes"));
    search.set_text("");
    wait_for(|| text_visible(&owner.history.widget, "Troy Barnes"));

    let download = |view| owner.event(Message::Core(Event::DownloadOverlayChanged(view)));
    let recording = |visible| {
        owner.event(Message::Core(Event::OverlayChanged(OverlayView {
            visible,
            icon: OverlayIcon::None,
            content: OverlayContent::Processing {
                label: "Transcribing...",
                preview: None,
            },
            trailing: None,
        })))
    };
    let downloading = || {
        download(DownloadOverlayView::Shown {
            status: "Downloading Greendale model… 42%".into(),
            progress: OverlayProgress::Determinate(0.42),
        })
    };
    assert!(!overlay_on_screen());
    downloading();
    assert_eq!(
        owner.getting_started.detail(),
        "Downloading Greendale model… 42%"
    );
    wait_for(overlay_on_screen);
    download(DownloadOverlayView::Hidden);
    assert_eq!(owner.getting_started.detail(), super::IDLE_HINT);
    wait_for(|| !overlay_on_screen());
    downloading();
    recording(true);
    download(DownloadOverlayView::Hidden);
    assert!(
        owner
            .getting_started
            .detail()
            .starts_with("Transcribing..."),
        "a hidden download must not replace the recording status"
    );
    assert!(
        overlay_on_screen(),
        "a hidden download must not take the recording overlay off screen"
    );
    recording(false);
    assert_eq!(owner.getting_started.detail(), super::IDLE_HINT);
    wait_for(|| !overlay_on_screen());
    assert!(
        widgets::<adw::PreferencesGroup>(&owner.about)
            .iter()
            .any(|group| group.title() == "Libraries")
    );
    assert!(
        widgets::<adw::PreferencesGroup>(&owner.about)
            .iter()
            .all(|group| group.title() != "Updates"),
        "General owns updates"
    );

    for name in [
        "dictation",
        "general",
        "hotkey",
        "speech",
        "post-processing",
        "about",
    ] {
        owner.stack.set_visible_child_name(name);
        if name == "post-processing" {
            owner.core.app.set_cleanup_enabled(true);
            wait_for(|| text_visible(&owner.post_processing.widget, "greendale/human-being"));
        }
        capture(owner, name);
    }
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    owner.stack.set_visible_child_name("general");
    capture(owner, "general-dark");
    owner.window.set_default_size(820, 650);
    owner.stack.set_visible_child_name("speech");
    capture(owner, "speech-small");
    wait_for(|| owner.window.width() <= 820);
}
