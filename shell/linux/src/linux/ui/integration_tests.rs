use super::Window;
use adw::prelude::*;
use std::{
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
    for text in [
        "Troy Barnes is organizing the Greendale study group.",
        "Remember to return the library books tomorrow.",
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
                    transcript: text.into(),
                    raw_transcript: None,
                    was_post_processed: false,
                    post_processing_error_description: None,
                    app_name: None,
                    app_bundle_id: None,
                    audio_byte_count: None,
                },
                &[],
                onda_history::HistoryPreferences {
                    enabled: true,
                    keep_audio: false,
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

fn capture(owner: &Window, name: &str) {
    let Some(directory) = std::env::var_os("ONDA_TEST_SCREENSHOT_DIR") else {
        return;
    };
    let until = Instant::now() + Duration::from_millis(350);
    wait_for(|| Instant::now() >= until);
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(&owner.window)).snapshot(
        &snapshot,
        owner.window.width() as f64,
        owner.window.height() as f64,
    );
    let node = snapshot.to_node().unwrap();
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
