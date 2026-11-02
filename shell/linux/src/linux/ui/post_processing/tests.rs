use std::time::{Duration, Instant};

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

#[test]
#[ignore = "requires an isolated GTK display, D-Bus session, keyring, and ONDA_DATA_DIR"]
fn real_gui_preserves_provider_keys_and_model_selection() {
    assert!(
        std::env::var_os("ONDA_DATA_DIR").is_some(),
        "use a scratch data directory"
    );
    adw::init().unwrap();
    super::super::integration_tests::seed_history();
    let application = adw::Application::builder()
        .application_id(crate::linux::application_id())
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let owner = Window::new(&application).unwrap();
    owner.present();
    wait_for("initial page", || {
        !owner.post_processing.details.is_visible()
    });
    owner.stack.set_visible_child_name("post-processing");
    let page = &owner.post_processing;
    page.enabled.set_active(true);
    wait_for("post-processing enabled", || page.details.is_visible());
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
