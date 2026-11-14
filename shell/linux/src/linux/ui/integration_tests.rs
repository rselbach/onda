use super::Window;
use adw::prelude::*;
use gtk::gio;
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

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
fn wait_for(ready: impl Fn() -> bool) {
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

fn widgets<T: IsA<gtk::Widget> + Clone + 'static>(root: &impl IsA<gtk::Widget>) -> Vec<T> {
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

fn text_visible(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
    widgets::<gtk::Label>(root)
        .iter()
        .any(|label| label.text().contains(text))
}

fn button(root: &impl IsA<gtk::Widget>, title: &str) -> gtk::Button {
    widgets::<gtk::Button>(root)
        .into_iter()
        .find(|button| button.label().as_deref() == Some(title))
        .unwrap()
}

fn settle() {
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
    let Some(directory) = std::env::var_os("ONDA_TEST_SCREENSHOT_DIR") else {
        return;
    };
    settle();
    // a window that awaits layout paints nothing
    let node = RefCell::new(None);
    wait_for(|| {
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&owner.window)).snapshot(
            &snapshot,
            owner.window.width() as f64,
            owner.window.height() as f64,
        );
        node.replace(snapshot.to_node());
        node.borrow().is_some()
    });
    let node = node.into_inner().unwrap();
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    owner
        .window
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
    let entry = widgets::<adw::EntryRow>(&owner.hotkey.widget).remove(0);
    entry.set_text("Ctrl+Alt+G");
    entry.emit_by_name::<()>("apply", &[]);
    wait_for(|| {
        super::super::hotkeys::trigger(&owner.core.settings.hotkey()).unwrap() == "CTRL+ALT+G"
    });
    button(&owner.hotkey.widget, "Reset to Ctrl+Alt+Space").emit_clicked();
    wait_for(|| owner.core.settings.hotkey() == super::parse_shortcut("Ctrl+Alt+Space").unwrap());

    owner.stack.set_visible_child_name("history");
    wait_for(|| text_visible(&owner.history.widget, "Troy Barnes"));
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
