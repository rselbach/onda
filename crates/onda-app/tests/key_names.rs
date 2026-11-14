//! Key names per platform: nothing the core shows a Linux user writes a
//! shortcut the macOS way (HOT-6).

mod common;

use common::{Harness, lock, sleep};
use onda_app::{Event, MainWindowSection};
use onda_domain::hotkey::{KeyCode, Modifier, ModifierKey, ModifierSet, RecorderEvent, Shortcut};
use onda_domain::onboarding::OnboardingFlags;
use onda_domain::platform::Platform;
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::types::OutputMode;

const MACOS_KEY_NAMES: [&str; 6] = ["⌃", "⌥", "⇧", "⌘", "Option", "Command"];

fn all_modifiers() -> ModifierSet {
    Modifier::ALL.into_iter().collect()
}

fn key(code: &str) -> KeyCode {
    KeyCode::parse(code).expect("a key code")
}

/// Everything the core shows through a first run, every page, a rejected
/// recording, a lone-modifier shortcut, a failed registration, and a reset.
/// View models are dumped with `Debug`.
async fn everything_shown(platform: Platform) -> String {
    let h = Harness::on_platform(platform, |s| {
        s.onboarding = OnboardingFlags::default();
        s.output_mode = OutputMode::Clipboard;
        s.hotkey = Shortcut::new(key("Enter"), all_modifiers()).expect("a valid shortcut");
    })
    .await;
    sleep(800).await;
    for _ in 0..8 {
        h.app.onboarding_continue();
        sleep(1).await;
    }
    assert_eq!(
        h.count(|e| matches!(e, Event::CloseOnboarding)),
        1,
        "the wizard ran to its end"
    );

    h.app.main_window_appeared();
    for page in MainWindowSection::ALL {
        h.app.page_appeared(page);
    }
    sleep(10).await;

    h.app.toggle_hotkey_capture();
    sleep(10).await;
    h.app.recorder_event(RecorderEvent::KeyDown {
        key: key("KeyA"),
        modifiers: ModifierSet::EMPTY.with(Modifier::Shift),
    });
    sleep(10).await;
    h.app.end_hotkey_capture();
    sleep(10).await;

    h.app.set_hotkey(Shortcut::lone(ModifierKey::AltRight));
    sleep(10).await;
    h.app.set_hotkey(Shortcut::lone(ModifierKey::MetaLeft));
    sleep(10).await;
    h.app.hotkey_registration_failed(-1);
    sleep(10).await;
    h.app.reset_hotkey();
    sleep(10).await;

    let notifications = lock(&h.notifier.bodies).join("\n");
    format!("{:#?}\n{notifications}", h.events())
}

/// The macOS key names in `text`. A word counts only on its own, so a Rust
/// name in a `Debug` dump, such as `RightOption`, does not.
fn macos_key_names(text: &str) -> Vec<&'static str> {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    MACOS_KEY_NAMES
        .into_iter()
        .filter(|name| {
            if name.is_ascii() {
                return words.contains(name);
            }
            text.contains(name)
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn hot_6_linux_never_reads_macos_key_names() {
    let cases = [
        (
            "macOS shows each of them, so the sweep reaches them",
            Platform::macos(PrivacyNaming::Accessibility),
            MACOS_KEY_NAMES.to_vec(),
        ),
        ("Linux shows none", Platform::linux(), vec![]),
    ];
    for (name, platform, want) in cases {
        let shown = everything_shown(platform).await;
        let lines: Vec<&str> = shown
            .lines()
            .filter(|line| !macos_key_names(line).is_empty())
            .collect();
        assert_eq!(macos_key_names(&shown), want, "{name}: {lines:#?}");
    }
}

#[tokio::test(start_paused = true)]
async fn hot_6_linux_spells_the_shortcut_everywhere_it_is_named() {
    let shown = everything_shown(Platform::linux()).await;
    let wants = [
        "Ready to Dictate — Ctrl+Alt+Shift+Super+Enter",
        "Current shortcut: Right Alt",
        "Reset to Ctrl+Alt+Space",
        "Shortcut must include Ctrl, Alt, or Super",
        "Press a key with Ctrl, Alt, or Super.",
        "paste with Ctrl+V.",
        "Could not register the global hotkey (Super).",
        "Ready to Dictate — Ctrl+Alt+Space",
    ];
    for want in wants {
        assert!(shown.contains(want), "nothing shows {want:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn hot_2_reset_restores_the_platforms_default() {
    let cases = [
        (
            Platform::macos(PrivacyNaming::Accessibility),
            Shortcut::OPTION_SPACE,
            "Ready to Dictate — ⌥ Space",
        ),
        (
            Platform::linux(),
            Shortcut::CONTROL_ALT_SPACE,
            "Ready to Dictate — Ctrl+Alt+Space",
        ),
    ];
    for (platform, want, want_header) in cases {
        let h = Harness::on_platform(platform, |s| {
            s.hotkey = Shortcut::lone(ModifierKey::AltRight);
        })
        .await;
        h.app.reset_hotkey();
        sleep(10).await;
        assert_eq!(
            lock(&h.settings.settings).hotkey,
            want,
            "{}",
            platform.os_name
        );
        assert_eq!(h.menu().header, want_header, "{}", platform.os_name);
    }
}
