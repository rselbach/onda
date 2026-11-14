//! Copy on a system that is not a Mac: nothing the Linux platform value
//! produces names macOS or its parts. Paused clock, fake ports.

mod common;

use std::collections::BTreeSet;

use common::{Harness, asr_failure, download, lock, loud, sleep};
use jiff::Timestamp;
use onda_app::seams::{DownloadUpdateKind, Settings};
use onda_app::{Event, MainWindowSection};
use onda_domain::cleanup::{CleanupNotice, CleanupProvider};
use onda_domain::history::NewEntry;
use onda_domain::hotkey::{ModifierKey, RegistrationFailureNotice, Shortcut};
use onda_domain::messages::UserMessage;
use onda_domain::models::ModelId;
use onda_domain::onboarding::{OnboardingFlags, OnboardingStep};
use onda_domain::platform::Platform;
use onda_domain::types::OutputMode;
use onda_domain::updates::{UpdateFailure, UpdateStatus};
use uuid::Uuid;

const BANNED: [&str; 6] = [
    "Mac",
    "macOS",
    "Finder",
    "Keychain",
    "System Settings",
    "OSStatus",
];

/// The banned word `text` contains, in any letter case. "Mac" counts only
/// as a whole word, so "machine" passes.
fn banned_word(text: &str) -> Option<&'static str> {
    let text = text.to_lowercase();
    BANNED.into_iter().find(|word| match *word {
        "Mac" => text
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token == "mac"),
        _ => text.contains(&word.to_lowercase()),
    })
}

/// The view an event carries, or `None` for an event without copy.
fn view(event: &Event) -> Option<&'static str> {
    match event {
        Event::TrayChanged(_) => Some("tray"),
        Event::MenuChanged(_) => Some("menu"),
        Event::OverlayChanged(_) => Some("overlay"),
        Event::DownloadOverlayChanged(_) => Some("download overlay"),
        Event::HotkeyChanged(_) => Some("hotkey"),
        Event::OnboardingChanged(_) => Some("onboarding"),
        Event::ShowUpdateNotice(_) => Some("update notice"),
        Event::GettingStartedChanged(_) => Some("getting started"),
        Event::GeneralChanged(_) => Some("general"),
        Event::HotkeyPageChanged(_) => Some("hotkey page"),
        Event::SpeechChanged(_) => Some("speech"),
        Event::PostProcessingChanged(_) => Some("post-processing"),
        Event::ReplacementsChanged(_) => Some("replacements"),
        Event::HistoryPageChanged(_) => Some("history"),
        Event::AboutChanged(_) => Some("about"),
        // an alert shows a critical message, and no message is critical
        // yet; `no_linux_message_names_a_mac_term` covers what it would say
        Event::ShowAlert { .. } => None,
        Event::HistoryChanged
        | Event::OpenOnboarding
        | Event::CloseOnboarding
        | Event::OpenMainWindow(_)
        | Event::CloseUpdateNotice
        | Event::AppPresenceChanged { .. }
        | Event::ActivateApp => None,
    }
}

const VIEWS: [&str; 15] = [
    "tray",
    "menu",
    "overlay",
    "download overlay",
    "hotkey",
    "onboarding",
    "update notice",
    "getting started",
    "general",
    "hotkey page",
    "speech",
    "post-processing",
    "replacements",
    "history",
    "about",
];

/// The API key field's copy, which its `Debug` leaves out along with the
/// key.
fn key_field_copy(event: &Event) -> Option<String> {
    let Event::PostProcessingChanged(page) = event else {
        return None;
    };
    let key = page.details.as_ref()?.dialog.as_ref()?.key.as_ref()?;
    Some(
        [
            key.label.as_str(),
            &key.placeholder,
            &key.note,
            &key.get_key_title,
        ]
        .join(" | "),
    )
}

/// Everything `h` showed: each event as its debug text, the key field's
/// copy, and each notification body.
fn shown(h: &Harness) -> (BTreeSet<&'static str>, Vec<String>) {
    let events = h.events();
    let views = events.iter().filter_map(view).collect();
    let mut texts: Vec<String> = events.iter().map(|event| format!("{event:?}")).collect();
    texts.extend(events.iter().filter_map(key_field_copy));
    texts.extend(lock(&h.notifier.bodies).iter().cloned());
    (views, texts)
}

fn recording() -> onda_domain::history::HistoryEntry {
    NewEntry {
        id: Uuid::new_v4(),
        timestamp: Timestamp::now(),
        sample_count: 112_000,
        model_id: ModelId::ParakeetTdtV2.as_str().to_owned(),
        model_name: Some("Parakeet TDT v2".to_owned()),
        language: "en".to_owned(),
        transcript: "Troy Barnes studies at Greendale.".to_owned(),
        raw_transcript: Some("um troy barnes studies at greendale".to_owned()),
        was_post_processed: true,
        post_processing_error_description: None,
        app_name: Some("Greendale Messenger".to_owned()),
        app_bundle_id: None,
        audio_byte_count: Some(84),
    }
    .into_entry()
}

// The fake shell reports the microphone and typing grants as given, which
// is all a Linux shell can report: the system has neither.

async fn first_run() -> Harness {
    let h = Harness::on_platform(Platform::linux(), |s| {
        s.onboarding = OnboardingFlags::default();
    })
    .await;
    sleep(800).await;
    let step = |h: &Harness| {
        h.last(|event| match event {
            Event::OnboardingChanged(view) => Some(view.step),
            _ => None,
        })
    };
    for _ in 0..OnboardingStep::ALL.len() {
        if step(&h) == Some(OnboardingStep::Ready) {
            break;
        }
        h.app.onboarding_continue();
        sleep(10).await;
    }
    assert_eq!(
        step(&h),
        Some(OnboardingStep::Ready),
        "the walk reached the last step"
    );
    h
}

async fn main_window() -> Harness {
    let h = Harness::on_platform(Platform::linux(), |s| {
        s.cleanup_enabled = true;
        s.keep_audio = true;
    })
    .await;
    *lock(&h.history.stored) = vec![recording()];
    lock(&h.system.updater).status = UpdateStatus::Failed(UpdateFailure::RequiresNewerOs {
        version: "2.0.0".to_owned(),
        minimum_os: "27.0".to_owned(),
    });
    h.app.updater_changed();
    for section in MainWindowSection::ALL {
        h.app.page_appeared(section);
    }
    sleep(10).await;
    h.app.open_providers_dialog();
    sleep(10).await;
    for provider in [CleanupProvider::Fireworks, CleanupProvider::OpenRouter] {
        h.app.select_dialog_provider(provider);
        sleep(10).await;
    }
    h.app.download_and_use(ModelId::ParakeetUnified);
    sleep(10).await;
    for kind in [
        DownloadUpdateKind::Started,
        DownloadUpdateKind::Progress(0.5),
    ] {
        h.app
            .download_event(download(ModelId::ParakeetUnified, kind));
    }
    sleep(10).await;
    h.app.hotkey_registration_failed(1);
    sleep(100).await;
    h.audio.starts_with(loud(24_000));
    h.script.finals(vec![Err(asr_failure())]);
    h.dictate().await;
    h
}

fn carried_over_from_a_mac(s: &mut Settings) {
    s.stored_model = "whisper-greendale".to_owned();
    s.cleanup_enabled = true;
    s.cleanup_provider = CleanupProvider::AppleIntelligence;
    s.cleanup_model = CleanupProvider::AppleIntelligence.default_model().clone();
}

async fn settings_from_a_mac() -> Harness {
    let h = Harness::build_on(None, Platform::linux(), carried_over_from_a_mac);
    // the model that replaces the stored one is not on disk, so Onda asks
    *lock(&h.models.downloaded) = vec![ModelId::ParakeetUltra];
    h.app.ui_ready();
    sleep(800).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.app.test_cleanup();
    sleep(10).await;
    h
}

#[tokio::test(start_paused = true)]
async fn no_linux_view_names_a_mac_term() {
    let mut seen = BTreeSet::new();
    for (name, h) in [
        ("first run", first_run().await),
        ("main window", main_window().await),
        ("settings from a Mac", settings_from_a_mac().await),
    ] {
        let (views, texts) = shown(&h);
        seen.extend(views);
        for text in texts {
            assert_eq!(banned_word(&text), None, "{name}: {text}");
        }
    }
    let missing: Vec<&str> = VIEWS
        .into_iter()
        .filter(|view| !seen.contains(view))
        .collect();
    assert!(missing.is_empty(), "views never built: {missing:?}");
}

#[tokio::test(start_paused = true)]
async fn the_sweep_reaches_the_copy_that_names_the_platform() {
    let wants = [
        (
            "first run",
            first_run().await,
            vec!["Transcripts are saved on this computer for 1 month."],
        ),
        (
            "main window",
            main_window().await,
            vec![
                "Saved only on this computer. Audio keeps the newest 50 recordings, up to 500 MB.",
                "Show Recording in Folder",
                "Stored in your desktop keyring.",
                "The API key is stored in your desktop keyring, not in preferences.",
                "Onda 2.0.0 requires Linux 27.0 or later.",
                "It may be in use by another application.",
                "Choose Retry Failed Dictation from the application to try again.",
            ],
        ),
        (
            "settings from a Mac",
            settings_from_a_mac().await,
            vec![
                "Post-processing runs on this computer with Apple Intelligence",
                "Runs on this computer with Apple Intelligence.",
                "This computer doesn't support Apple Intelligence.",
            ],
        ),
    ];
    for (name, h, want) in wants {
        let (_, texts) = shown(&h);
        for copy in want {
            assert!(
                texts.iter().any(|text| text.contains(copy)),
                "{name}: nothing showed {copy:?}"
            );
        }
    }
}

/// The message after `message`, starting from `None`. The match names
/// every variant, so a new message cannot be left out of the sweep.
fn next_message(message: Option<&UserMessage>) -> Option<UserMessage> {
    use UserMessage::*;
    let error = || "Pierce unplugged the router".to_owned();
    let failed = |shortcut: Shortcut| {
        HotkeyRegistrationFailed(RegistrationFailureNotice::new(
            &shortcut,
            1,
            Platform::linux(),
        ))
    };
    let combo = failed(Shortcut::default());
    Some(match message {
        None => ModelNotReady,
        Some(ModelNotReady) => ModelNotDownloaded,
        Some(ModelNotDownloaded) => StillTranscribing,
        Some(StillTranscribing) => MicrophonePermissionRequired,
        Some(MicrophonePermissionRequired) => AccessibilityPermissionRequired,
        Some(AccessibilityPermissionRequired) => NoSpeechDetected,
        Some(NoSpeechDetected) => MicrophoneUnavailable { error: error() },
        Some(MicrophoneUnavailable { .. }) => AudioProcessingError { error: error() },
        Some(AudioProcessingError { .. }) => TranscriptCopiedToClipboard,
        Some(TranscriptCopiedToClipboard) => TargetLeft,
        Some(TargetLeft) => PasteFailed,
        Some(PasteFailed) => ClipboardRestoreFailed,
        Some(ClipboardRestoreFailed) => CopyFailed,
        Some(CopyFailed) => Cleanup {
            notice: CleanupNotice::CouldNotRefine {
                provider: CleanupProvider::OpenRouter,
                error: error(),
            },
            output_mode: OutputMode::Type,
        },
        Some(Cleanup {
            notice: CleanupNotice::CouldNotRefine { .. },
            ..
        }) => Cleanup {
            notice: CleanupNotice::UsedRawTranscript,
            output_mode: OutputMode::Clipboard,
        },
        Some(Cleanup {
            notice: CleanupNotice::UsedRawTranscript,
            ..
        }) => TranscriptionFailed { error: error() },
        Some(TranscriptionFailed { .. }) => RecordingLimitReached,
        Some(RecordingLimitReached) => CouldntLoadSpeechModel {
            error: error(),
            files_on_disk: false,
        },
        Some(CouldntLoadSpeechModel {
            files_on_disk: false,
            ..
        }) => CouldntLoadSpeechModel {
            error: error(),
            files_on_disk: true,
        },
        Some(CouldntLoadSpeechModel {
            files_on_disk: true,
            ..
        }) => ModelDownloadFailed {
            model: "Parakeet TDT v2".to_owned(),
            error: error(),
        },
        Some(ModelDownloadFailed { .. }) => combo,
        Some(message @ HotkeyRegistrationFailed(_)) if *message == combo => {
            failed(Shortcut::lone(ModifierKey::AltRight))
        }
        Some(HotkeyRegistrationFailed(_)) => InvalidLanguageSetting {
            code: "tlh".to_owned(),
        },
        Some(InvalidLanguageSetting { .. }) => return None,
    })
}

#[test]
fn no_linux_message_names_a_mac_term() {
    let platform = Platform::linux();
    let messages: Vec<UserMessage> =
        std::iter::successors(next_message(None), |message| next_message(Some(message))).collect();
    assert_eq!(messages.len(), 23, "every message and its variations");
    for message in messages {
        let title = message.title(platform.privacy_naming());
        let body = message.body(platform.tray_term, platform.privacy_naming());
        assert_eq!(banned_word(title), None, "{message:?}: {title}");
        assert_eq!(banned_word(&body), None, "{message:?}: {body}");
    }
}

#[test]
fn banned_words_match_whole_mac_and_any_case() {
    let cases = [
        ("Saved only on this Mac.", Some("Mac")),
        ("Stored in your system keychain.", Some("Keychain")),
        ("Show Recording in Finder", Some("Finder")),
        ("Turn it on in System Settings.", Some("System Settings")),
        ("OSStatus: -50", Some("OSStatus")),
        ("Follows the input selected in macOS.", Some("macOS")),
        ("This machine runs Linux.", None),
        ("Saved only on this computer.", None),
    ];
    for (text, want) in cases {
        assert_eq!(banned_word(text), want, "{text}");
    }
}
