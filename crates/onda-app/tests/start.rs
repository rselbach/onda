//! Lifecycle tests for the core.

mod common;

use common::{Harness, sleep};
use onda_app::Event;
use onda_domain::tray::{TrayIcon, TrayView};

#[tokio::test(start_paused = true)]
async fn start_publishes_the_tray_before_anything_else() {
    let h =
        Harness::unstarted(|s| s.onboarding = onda_domain::onboarding::OnboardingFlags::default());
    sleep(1).await;
    let events = h.events();
    let want = Event::TrayChanged(TrayView {
        icon: TrayIcon::Warning,
        red_tint: false,
        accessibility_label: "Onda, finish setup to dictate".to_owned(),
    });
    assert_eq!(events.first(), Some(&want));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::MenuChanged(menu) if menu.header == "Finish Setup to Dictate"))
    );
}

#[tokio::test(start_paused = true)]
async fn ready_once_the_selected_model_loads() {
    let h = Harness::new().await;
    let tray = h.tray();
    assert_eq!(tray.icon, TrayIcon::Ready);
    assert_eq!(tray.accessibility_label, "Onda, ready to dictate");
    assert_eq!(h.menu().header, "Ready to Dictate — ⌥ Space");
    h.app.shutdown_async().await;
}
