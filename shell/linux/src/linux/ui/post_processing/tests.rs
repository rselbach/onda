use std::time::{Duration, Instant};

use onda_domain::cleanup::SYSTEM_PROMPT;

use super::super::integration_tests::{Start, launch};
use super::*;

fn wait_for(description: &str, ready: impl Fn() -> bool) {
    let context = gtk::glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn provider_loaded(chooser: &Chooser, provider: &str) -> bool {
    chooser.view.borrow().as_ref().is_some_and(|view| {
        view.selected_provider == provider && view.key.as_ref().is_some_and(|key| !key.loading)
    })
}

fn editor_text(editor: &Editor) -> String {
    let buffer = editor.text.buffer();
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}

fn open_editor(page: &Page) -> Rc<Editor> {
    page.prompt_row.emit_by_name::<()>("activated", &[]);
    wait_for("prompt sheet", || page.editor.borrow().is_some());
    page.editor.borrow().as_ref().unwrap().clone()
}

fn type_draft(editor: &Editor, draft: &str) {
    editor.text.buffer().set_text(draft);
    wait_for("unsaved draft", || {
        editor.save.is_sensitive() && editor.status.text() == "Unsaved changes"
    });
    assert_eq!(
        editor_text(editor),
        draft,
        "a view update must not overwrite typing"
    );
}

fn stored_prompt() -> Option<serde_json::Value> {
    let path =
        std::path::PathBuf::from(std::env::var_os("ONDA_DATA_DIR").unwrap()).join("settings.json");
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    settings.get("cleanupPrompt").cloned()
}

#[test]
#[ignore = "requires an isolated GTK display, D-Bus session, keyring, and ONDA_DATA_DIR"]
fn real_gui_preserves_provider_keys_and_model_selection() {
    super::super::integration_tests::seed_history();
    let owner = launch(Start::Declined);
    // the core opens Speech when it finds no model to load
    wait_for("launch", || {
        owner.stack.visible_child_name().as_deref() == Some("speech")
    });
    wait_for("initial page", || {
        !owner.post_processing.details.is_visible()
    });
    owner.stack.set_visible_child_name("post-processing");
    let page = &owner.post_processing;
    page.enabled.set_active(true);
    wait_for("post-processing enabled", || page.details.is_visible());
    assert!(
        page.caption
            .text()
            .contains("The API key is stored in your desktop keyring"),
        "{}",
        page.caption.text()
    );
    page.model.emit_by_name::<()>("activated", &[]);
    wait_for("model chooser", || page.chooser.borrow().is_some());
    let chooser = page.chooser.borrow().as_ref().unwrap().clone();
    wait_for("OpenRouter key loaded", || {
        provider_loaded(&chooser, "openrouter")
    });
    assert!(
        !chooser.key_error.is_visible(),
        "missing key must not be a keyring error"
    );
    assert!(chooser.key.text().is_empty());
    assert!(chooser.provider.subtitle().unwrap().contains("OpenRouter"));
    assert_eq!(chooser.get_key.label().unwrap(), "Get a key");
    assert_eq!(
        Some(chooser.get_key.uri().as_str()),
        CleanupProvider::OpenRouter.key_url()
    );
    assert!(chooser.in_use.text().starts_with("In use: "));
    assert!(chooser.in_use.text().ends_with(" · OpenRouter"));
    assert!(
        !chooser
            .view
            .borrow()
            .as_ref()
            .unwrap()
            .providers
            .iter()
            .any(|provider| provider.id == "apple-intelligence")
    );

    chooser.key.set_text("sk-or-troy-barnes");
    let fireworks = chooser
        .view
        .borrow()
        .as_ref()
        .unwrap()
        .providers
        .iter()
        .position(|provider| provider.id == "fireworks")
        .unwrap();
    chooser.provider.set_selected(fireworks as u32);
    wait_for("Fireworks key loaded", || {
        provider_loaded(&chooser, "fireworks")
    });
    assert!(
        chooser.key.text().is_empty(),
        "a provider must not inherit another provider's key"
    );
    chooser.key.set_text("sk-fw-greendale");
    let openrouter = chooser
        .view
        .borrow()
        .as_ref()
        .unwrap()
        .providers
        .iter()
        .position(|provider| provider.id == "openrouter")
        .unwrap();
    chooser.provider.set_selected(openrouter as u32);
    wait_for("OpenRouter key restored", || {
        provider_loaded(&chooser, "openrouter")
    });
    assert_eq!(chooser.key.text().as_str(), "sk-or-troy-barnes");

    chooser.search.set_text("greendale/human-being");
    chooser.render_models();
    let group = chooser
        .models
        .first_child()
        .unwrap()
        .downcast::<adw::PreferencesGroup>()
        .unwrap();
    // Invoke the same row action used by a mouse click or keyboard activation.
    fn find_row(widget: &gtk::Widget) -> Option<adw::ActionRow> {
        if let Ok(row) = widget.clone().downcast::<adw::ActionRow>() {
            return Some(row);
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(row) = find_row(&widget) {
                return Some(row);
            }
            child = widget.next_sibling();
        }
        None
    }
    find_row(group.upcast_ref())
        .unwrap()
        .emit_by_name::<()>("activated", &[]);
    wait_for("custom model saved", || {
        owner
            .core
            .settings
            .cleanup_model(CleanupProvider::OpenRouter)
            .slug
            == "greendale/human-being"
    });
    assert_eq!(
        chooser.key.text().as_str(),
        "sk-or-troy-barnes",
        "model refresh must not overwrite key edits"
    );
    if let Some(path) = std::env::var_os("ONDA_TEST_SCREENSHOT") {
        let context = gtk::glib::MainContext::default();
        for _ in 0..30 {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&owner.window)).snapshot(
            &snapshot,
            owner.window.width() as f64,
            owner.window.height() as f64,
        );
        let node = snapshot.to_node().unwrap();
        owner
            .window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(path)
            .unwrap();
    }
    chooser.dialog.close();
    wait_for("chooser closed", || page.chooser.borrow().is_none());
    page.model.emit_by_name::<()>("activated", &[]);
    wait_for("chooser reopened", || page.chooser.borrow().is_some());
    let chooser = page.chooser.borrow().as_ref().unwrap().clone();
    wait_for("saved key reloaded", || {
        provider_loaded(&chooser, "openrouter")
    });
    assert_eq!(chooser.key.text().as_str(), "sk-or-troy-barnes");
    chooser.key.set_text("");
    chooser.provider.set_selected(fireworks as u32);
    wait_for("Fireworks key restored", || {
        provider_loaded(&chooser, "fireworks")
    });
    assert_eq!(chooser.key.text().as_str(), "sk-fw-greendale");
    chooser.key.set_text("");
    chooser.provider.set_selected(openrouter as u32);
    wait_for("deleted key", || provider_loaded(&chooser, "openrouter"));
    assert!(chooser.key.text().is_empty());
    chooser.dialog.close();
    wait_for("chooser closed", || page.chooser.borrow().is_none());
    page.test.emit_clicked();
    wait_for("missing key feedback", || page.error.is_visible());
    assert!(page.error.text().contains("API key"));
    page.dismiss_error.emit_clicked();
    wait_for("dismissed error", || !page.error.is_visible());

    let draft = "Rewrite the transcript the way Troy Barnes would say it.";
    assert_eq!(page.prompt_row.title(), "Onda default");
    let editor = open_editor(page);
    assert_eq!(editor_text(&editor), SYSTEM_PROMPT);
    assert!(!editor.save.is_sensitive() && !editor.restore.is_sensitive());
    type_draft(&editor, draft);
    assert_eq!(editor.badge.text(), "Custom");
    super::super::integration_tests::capture(&owner, "prompt-editor");
    editor.dialog.close();
    wait_for("prompt sheet closed", || page.editor.borrow().is_none());
    assert_eq!(stored_prompt(), None, "a closed sheet must store nothing");
    assert_eq!(page.prompt_row.title(), "Onda default");
    let editor = open_editor(page);
    assert_eq!(
        editor_text(&editor),
        SYSTEM_PROMPT,
        "a discarded draft must not come back"
    );

    type_draft(&editor, draft);
    editor.save.emit_clicked();
    wait_for("custom prompt saved", || page.editor.borrow().is_none());
    assert_eq!(stored_prompt(), Some(draft.into()));
    assert_eq!(page.prompt_row.title(), "Custom prompt");
    assert_eq!(page.prompt_preview.text(), draft);
    assert!(page.prompt_status.text().contains("Custom prompt saved"));

    let editor = open_editor(page);
    assert_eq!(editor_text(&editor), draft);
    assert!(!page.prompt_status.is_visible());
    editor.restore.emit_clicked();
    wait_for("default restored in the draft", || {
        editor_text(&editor) == SYSTEM_PROMPT && editor.save.is_sensitive()
    });
    assert_eq!(
        stored_prompt(),
        Some(draft.into()),
        "Restore Default must not save"
    );
    editor.save.emit_clicked();
    wait_for("default prompt saved", || page.editor.borrow().is_none());
    assert_eq!(stored_prompt(), None);
    assert_eq!(page.prompt_row.title(), "Onda default");
    page.enabled.set_active(false);
    wait_for("disabled details", || !page.details.is_visible());
    let settings = std::fs::read_to_string(
        std::path::PathBuf::from(std::env::var_os("ONDA_DATA_DIR").unwrap()).join("settings.json"),
    )
    .unwrap();
    assert!(!settings.contains("sk-or-troy") && !settings.contains("sk-fw-greendale"));
    super::super::integration_tests::exercise_pages(&owner);
    owner.core.runtime.block_on(owner.core.app.shutdown_async());
    owner.window.close();
}
