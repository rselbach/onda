//! Window flows: onboarding, the update notice, the main window pages, and
//! the status menu items that open them (M7, M8). Paused clock, fake ports.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::{EngineCall, Harness, download, eventually, lock, sleep};
use jiff::{Timestamp, ToSpan};
use onda_app::seams::DownloadUpdateKind;
use onda_app::views::{
    DayHeading, GeneralPage, HistoryContent, HistoryPage, ModelRowControl, OnboardingContent,
    OnboardingView, PostProcessingPage, ReplacementsPage, SpeechPage,
};
use onda_app::{Event, MainWindowSection};
use onda_domain::history::{HistoryEntry, NewEntry};
use onda_domain::hotkey::HotkeyPreset;
use onda_domain::languages::{Language, LanguageSetting};
use onda_domain::messages::LoginItemRegistryStatus;
use onda_domain::migration::NoticeChoice;
use onda_domain::models::ModelId;
use onda_domain::onboarding::{OnboardingFlags, OnboardingStep};
use onda_domain::setup::PermissionFix;
use onda_domain::tray::TrayIcon;
use onda_domain::types::{HistoryRetention, MicrophonePermission};
use onda_domain::updates::{UpdateAction, UpdateStatus};
use uuid::Uuid;

const V2: ModelId = ModelId::ParakeetTdtV2;
const ULTRA: ModelId = ModelId::ParakeetUltra;
const UNIFIED: ModelId = ModelId::ParakeetUnified;

fn en() -> LanguageSetting {
    LanguageSetting::parse("en").expect("en is offered")
}

fn language(code: &str) -> LanguageSetting {
    LanguageSetting::parse(code).expect("offered language")
}

fn onboarding(h: &Harness) -> OnboardingView {
    h.last(|e| match e {
        Event::OnboardingChanged(view) => Some(*view),
        _ => None,
    })
    .expect("an onboarding view")
}

fn speech(h: &Harness) -> SpeechPage {
    h.last(|e| match e {
        Event::SpeechChanged(page) => Some(*page),
        _ => None,
    })
    .expect("a Speech page")
}

fn general(h: &Harness) -> GeneralPage {
    h.last(|e| match e {
        Event::GeneralChanged(page) => Some(*page),
        _ => None,
    })
    .expect("a General page")
}

fn history(h: &Harness) -> HistoryPage {
    h.last(|e| match e {
        Event::HistoryPageChanged(page) => Some(*page),
        _ => None,
    })
    .expect("a History page")
}

fn post_processing(h: &Harness) -> PostProcessingPage {
    h.last(|e| match e {
        Event::PostProcessingChanged(page) => Some(*page),
        _ => None,
    })
    .expect("a Post-Processing page")
}

fn replacements(h: &Harness) -> ReplacementsPage {
    h.last(|e| match e {
        Event::ReplacementsChanged(page) => Some(*page),
        _ => None,
    })
    .expect("a Replacements page")
}

fn opened_sections(h: &Harness) -> Vec<MainWindowSection> {
    h.events()
        .into_iter()
        .filter_map(|e| match e {
            Event::OpenMainWindow(section) => Some(section),
            _ => None,
        })
        .collect()
}

fn first_run(s: &mut onda_app::seams::Settings) {
    s.onboarding = OnboardingFlags::default();
}

fn settings_log(h: &Harness, prefix: &str) -> Vec<String> {
    h.log.with(&format!("settings:{prefix}"))
}

fn entry(text: &str, raw: Option<&str>, app: Option<&str>, age: jiff::Span) -> HistoryEntry {
    NewEntry {
        id: Uuid::new_v4(),
        timestamp: Timestamp::now()
            .checked_sub(age)
            .expect("a representable time"),
        sample_count: 112_000,
        model_id: V2.as_str().to_owned(),
        model_name: Some("Parakeet TDT v2".to_owned()),
        language: "en".to_owned(),
        transcript: text.to_owned(),
        raw_transcript: raw.map(str::to_owned),
        was_post_processed: raw.is_some(),
        post_processing_error_description: None,
        app_name: app.map(str::to_owned),
        app_bundle_id: app.map(|a| format!("edu.greendale.{}", a.to_lowercase())),
        audio_byte_count: None,
    }
    .into_entry()
}

// ---------------------------------------------------------------------------
// onboarding (ONB-1..ONB-13)

#[tokio::test(start_paused = true)]
async fn onb_13_first_run_onboarding_appears_750_ms_after_launch() {
    let h = Harness::unstarted(first_run);
    h.app.ui_ready();
    sleep(700).await;
    assert_eq!(h.count(|e| matches!(e, Event::OpenOnboarding)), 0);
    sleep(100).await;
    assert_eq!(h.count(|e| matches!(e, Event::OpenOnboarding)), 1);
    let view = onboarding(&h);
    assert_eq!(view.step, OnboardingStep::Welcome);
    assert_eq!(view.primary_title, "Get Started");
    assert!(!view.back_visible);
    assert!(
        h.script.calls().is_empty(),
        "ASR-1: onboarding owns the load"
    );
    assert!(
        h.log.with("notifications:").is_empty(),
        "ONB-12: never on top of the welcome screen"
    );
    assert_eq!(
        h.last(|e| match e {
            Event::AppPresenceChanged { regular } => Some(regular),
            _ => None,
        }),
        Some(true),
        "UI-2: the onboarding window makes Onda a regular app"
    );
}

#[tokio::test(start_paused = true)]
async fn onb_12_completed_users_get_the_notification_request_at_launch() {
    let h = Harness::new().await;
    assert_eq!(h.log.with("notifications:"), vec!["notifications:request"]);
    sleep(1_000).await;
    assert_eq!(h.count(|e| matches!(e, Event::OpenOnboarding)), 0);
}

#[tokio::test(start_paused = true)]
async fn onb_1_to_10_first_run_from_welcome_to_start_dictating() {
    let h = Harness::unstarted(|s| {
        first_run(s);
        s.language = LanguageSetting::Auto;
        s.selected_model = ULTRA;
        s.stored_model = ULTRA.as_str().to_owned();
    });
    lock(&h.models.downloaded).clear();
    *lock(&h.permissions.microphone) = MicrophonePermission::NotDetermined;
    h.permissions.accessibility.store(false, Ordering::SeqCst);
    h.app.ui_ready();
    sleep(800).await;

    h.app.onboarding_continue();
    sleep(1).await;
    let view = onboarding(&h);
    assert_eq!(view.step, OnboardingStep::Language);
    let OnboardingContent::Language {
        cards,
        toggle_title,
        ..
    } = view.content
    else {
        panic!("the language step");
    };
    assert_eq!(cards.len(), Language::popular().count());
    assert_eq!(toggle_title.as_deref(), Some("Show All"));

    h.app.onboarding_search_languages("ger");
    sleep(1).await;
    let OnboardingContent::Language {
        cards,
        toggle_title,
        ..
    } = onboarding(&h).content
    else {
        panic!("the language step");
    };
    let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["German"]);
    assert_eq!(toggle_title, None, "hidden while searching");

    h.app.onboarding_select_language(en());
    sleep(1).await;
    assert_eq!(
        settings_log(&h, ""),
        vec![
            "settings:Language(Specific(Language(10)))".to_owned(),
            "settings:SelectedModel(ParakeetTdtV2)".to_owned(),
        ]
    );
    h.app.onboarding_select_language(en());
    sleep(1).await;
    assert_eq!(settings_log(&h, "").len(), 2, "re-picking changes nothing");

    // ONB-5: leaving Language starts the recommended download
    h.app.onboarding_continue();
    sleep(1).await;
    assert_eq!(h.log.with("download:"), vec!["download:parakeet-tdt-v2"]);
    let view = onboarding(&h);
    assert_eq!(view.step, OnboardingStep::Permissions);
    assert_eq!(
        view.hint.as_deref(),
        Some("Microphone access is required for dictation")
    );
    h.app.onboarding_continue();
    sleep(1).await;
    assert_eq!(onboarding(&h).step, OnboardingStep::Permissions, "blocked");

    h.app.fix_permission(PermissionFix::RequestMicrophone);
    sleep(10).await;
    let view = onboarding(&h);
    assert_eq!(
        view.hint.as_deref(),
        Some("Enable Accessibility or choose clipboard output")
    );
    let OnboardingContent::Permissions { clipboard_link, .. } = view.content else {
        panic!("the permissions step");
    };
    assert_eq!(clipboard_link.as_deref(), Some("Use the clipboard instead"));
    h.app.onboarding_use_clipboard();
    sleep(1).await;
    assert_eq!(onboarding(&h).hint, None);

    h.app.onboarding_continue();
    sleep(1).await;
    let view = onboarding(&h);
    assert_eq!(view.step, OnboardingStep::Hotkey);
    let OnboardingContent::Hotkey { presets, .. } = view.content else {
        panic!("the hotkey step");
    };
    let enabled: Vec<bool> = presets.iter().map(|p| p.enabled).collect();
    assert_eq!(
        enabled,
        vec![true, false, false],
        "lone modifiers need Typing Access"
    );
    h.app.onboarding_select_hotkey(HotkeyPreset::RightOption);
    sleep(1).await;
    assert!(
        h.log.with("settings:hotkey").is_empty(),
        "a disabled preset"
    );

    h.app.onboarding_continue();
    sleep(1).await;
    h.app
        .download_event(download(V2, DownloadUpdateKind::Progress(0.42)));
    sleep(1).await;
    let view = onboarding(&h);
    assert_eq!(view.step, OnboardingStep::SpeechModel);
    assert_eq!(
        view.hint.as_deref(),
        Some("Downloading Parakeet TDT v2 — 42%")
    );
    assert!(!view.primary_enabled);

    lock(&h.models.downloaded).push(V2);
    h.app
        .download_event(download(V2, DownloadUpdateKind::Finished));
    sleep(10).await;
    assert_eq!(onboarding(&h).hint, None);
    assert!(h.script.calls().contains(&EngineCall::Load(V2)), "ASR-9");

    h.app.onboarding_continue();
    sleep(1).await;
    let view = onboarding(&h);
    assert_eq!(view.step, OnboardingStep::Ready);
    assert_eq!(view.primary_title, "Start Dictating");
    let OnboardingContent::Ready {
        retention_note,
        try_it_placeholder,
        ..
    } = view.content
    else {
        panic!("the ready step");
    };
    assert_eq!(
        retention_note.as_deref(),
        Some("Transcripts are saved on this Mac for 1 month.")
    );
    assert_eq!(
        try_it_placeholder,
        "Click here, dictate, then paste with ⌘V."
    );

    h.app.onboarding_continue();
    sleep(10).await;
    assert_eq!(h.count(|e| matches!(e, Event::CloseOnboarding)), 1);
    assert!(settings_log(&h, "Onboarding").contains(
        &"settings:Onboarding(OnboardingFlags { onboarding_completed: true, did_show_first_run_setup: true })"
            .to_owned()
    ));
    assert_eq!(h.log.with("notifications:"), vec!["notifications:request"]);
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn onb_11_an_abandoned_first_run_stays_pending_and_downloads_nothing() {
    let h = Harness::unstarted(first_run);
    lock(&h.models.downloaded).clear();
    h.app.ui_ready();
    sleep(800).await;
    h.app.onboarding_closed();
    sleep(10).await;
    assert!(settings_log(&h, "Onboarding").is_empty(), "still pending");
    assert!(h.log.with("download:").is_empty());
    assert!(h.script.calls().is_empty());
    assert_eq!(h.log.with("notifications:"), vec!["notifications:request"]);
    assert_eq!(
        h.tray().accessibility_label,
        "Onda, finish setup to dictate"
    );
}

#[tokio::test(start_paused = true)]
async fn onb_11_an_abandoned_first_run_loads_an_installed_model() {
    let h = Harness::unstarted(first_run);
    h.app.ui_ready();
    sleep(800).await;
    assert!(h.script.calls().is_empty());
    h.app.onboarding_closed();
    sleep(10).await;
    assert_eq!(h.script.calls(), vec![EngineCall::Load(V2)]);
}

#[tokio::test(start_paused = true)]
async fn win_4_run_setup_again_closed_early_stays_complete() {
    let h = Harness::new().await;
    h.app.run_setup_again();
    sleep(10).await;
    assert_eq!(h.count(|e| matches!(e, Event::OpenOnboarding)), 1);
    assert_eq!(
        settings_log(&h, "Onboarding"),
        vec![
            "settings:Onboarding(OnboardingFlags { onboarding_completed: false, did_show_first_run_setup: true })"
        ]
    );
    assert_eq!(
        h.tray().icon,
        TrayIcon::Ready,
        "rerunning setup is not first run"
    );
    h.app.onboarding_closed();
    sleep(10).await;
    assert_eq!(
        settings_log(&h, "Onboarding").last().map(String::as_str),
        Some(
            "settings:Onboarding(OnboardingFlags { onboarding_completed: true, did_show_first_run_setup: true })"
        )
    );
    assert_eq!(
        h.script.calls(),
        vec![EngineCall::Load(V2)],
        "the loaded model is not loaded again"
    );
}

#[tokio::test(start_paused = true)]
async fn onb_2_onboarding_rereads_permissions_every_second() {
    let h = Harness::unstarted(first_run);
    *lock(&h.permissions.microphone) = MicrophonePermission::NotDetermined;
    h.app.ui_ready();
    sleep(800).await;
    h.app.onboarding_continue();
    h.app.onboarding_continue();
    sleep(1).await;
    assert_eq!(
        onboarding(&h).hint.as_deref(),
        Some("Microphone access is required for dictation")
    );
    // granted in System Settings, outside Onda
    *lock(&h.permissions.microphone) = MicrophonePermission::Granted;
    sleep(1_100).await;
    assert_eq!(onboarding(&h).hint, None);
}

// ---------------------------------------------------------------------------
// the update notice (DL-14)

fn migrated(s: &mut onda_app::seams::Settings) {
    s.stored_model = "whisper-greendale".to_owned();
    s.selected_model = V2;
    s.language = LanguageSetting::Auto;
}

#[tokio::test(start_paused = true)]
async fn dl_14_not_now_declines_and_loads_the_fallback() {
    let h = Harness::unstarted(migrated);
    *lock(&h.models.downloaded) = vec![ULTRA];
    h.app.ui_ready();
    sleep(700).await;
    assert!(h.script.calls().is_empty(), "no load until answered");
    assert_eq!(h.count(|e| matches!(e, Event::ShowUpdateNotice(_))), 0);
    sleep(100).await;
    let notice = h
        .last(|e| match e {
            Event::ShowUpdateNotice(notice) => Some(*notice),
            _ => None,
        })
        .expect("the notice");
    assert_eq!(notice.title, "Onda was updated");
    assert!(
        notice.body.contains("**Parakeet TDT v2**"),
        "{}",
        notice.body
    );
    let titles: Vec<&str> = notice.buttons.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(titles, vec!["Not Now", "Choose Another", "Download"]);

    h.app.update_notice_choice(NoticeChoice::NotNow);
    sleep(10).await;
    assert_eq!(
        settings_log(&h, ""),
        vec![
            "settings:LastNoticeKey(\"whisper-greendale->parakeet-tdt-v2\")",
            "settings:DeclinedNoticeKey(\"whisper-greendale->parakeet-tdt-v2\")",
        ]
    );
    assert_eq!(h.count(|e| matches!(e, Event::CloseUpdateNotice)), 1);
    assert_eq!(h.script.calls(), vec![EngineCall::Load(ULTRA)]);
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn dl_14_the_declined_state_persists_across_launches() {
    let h = Harness::unstarted(|s| {
        migrated(s);
        s.last_notice_key = "whisper-greendale->parakeet-tdt-v2".to_owned();
        s.declined_notice_key = "whisper-greendale->parakeet-tdt-v2".to_owned();
    });
    *lock(&h.models.downloaded) = vec![ULTRA];
    h.app.ui_ready();
    sleep(1_000).await;
    assert_eq!(h.count(|e| matches!(e, Event::ShowUpdateNotice(_))), 0);
    assert_eq!(h.script.calls(), vec![EngineCall::Load(ULTRA)]);

    // any download start ends the declined state
    h.app.download_and_use(UNIFIED);
    sleep(10).await;
    assert_eq!(
        settings_log(&h, "DeclinedNoticeKey"),
        vec!["settings:DeclinedNoticeKey(\"\")"]
    );
}

#[tokio::test(start_paused = true)]
async fn dl_14_declined_without_a_fallback_opens_speech_and_says_not_downloaded() {
    let h = Harness::unstarted(|s| {
        migrated(s);
        s.declined_notice_key = "whisper-greendale->parakeet-tdt-v2".to_owned();
        s.last_notice_key = "whisper-greendale->parakeet-tdt-v2".to_owned();
    });
    lock(&h.models.downloaded).clear();
    h.app.ui_ready();
    sleep(100).await;
    assert_eq!(opened_sections(&h), vec![MainWindowSection::Speech]);
    h.press();
    sleep(10).await;
    assert_eq!(h.notices(), vec!["Model Not Downloaded"]);
    assert_eq!(
        opened_sections(&h),
        vec![MainWindowSection::Speech, MainWindowSection::Speech],
        "DICT-5: a declined download sends setup guidance to Speech"
    );
}

#[tokio::test(start_paused = true)]
async fn dl_14_download_and_choose_another() {
    struct Case {
        name: &'static str,
        choice: NoticeChoice,
        want_downloads: Vec<&'static str>,
        want_sections: Vec<MainWindowSection>,
        want_selected: bool,
    }
    let cases = [
        Case {
            name: "download selects the replacement and downloads it",
            choice: NoticeChoice::Download,
            want_downloads: vec!["download:parakeet-tdt-v2"],
            want_sections: Vec::new(),
            want_selected: true,
        },
        Case {
            name: "choose another opens Speech",
            choice: NoticeChoice::ChooseAnother,
            want_downloads: Vec::new(),
            want_sections: vec![MainWindowSection::Speech],
            want_selected: false,
        },
    ];
    for tc in cases {
        let h = Harness::unstarted(migrated);
        *lock(&h.models.downloaded) = vec![ULTRA];
        h.app.ui_ready();
        sleep(800).await;
        h.app.update_notice_choice(tc.choice);
        sleep(10).await;
        assert_eq!(h.log.with("download:"), tc.want_downloads, "{}", tc.name);
        assert_eq!(opened_sections(&h), tc.want_sections, "{}", tc.name);
        assert_eq!(
            settings_log(&h, "SelectedModel") == vec!["settings:SelectedModel(ParakeetTdtV2)"],
            tc.want_selected,
            "{}",
            tc.name
        );
        assert_eq!(
            settings_log(&h, "DeclinedNoticeKey"),
            vec!["settings:DeclinedNoticeKey(\"\")"],
            "{}",
            tc.name
        );
        assert!(h.script.calls().is_empty(), "{}", tc.name);
    }
}

// ---------------------------------------------------------------------------
// downloads end to end (DL-1..DL-4, DL-8, ASR-2, ASR-9)

#[tokio::test(start_paused = true)]
async fn asr_2_a_missing_model_downloads_before_it_loads() {
    let h = Harness::unstarted(|_| {});
    lock(&h.models.downloaded).clear();
    h.app.ui_ready();
    sleep(10).await;
    assert_eq!(h.log.with("download:"), vec!["download:parakeet-tdt-v2"]);
    assert!(h.script.calls().is_empty());
    h.app
        .download_event(download(V2, DownloadUpdateKind::Progress(0.25)));
    sleep(10).await;
    assert_eq!(h.tray().icon, TrayIcon::ProgressRing { progress: 0.25 });
    assert_eq!(h.menu().header, "Preparing Speech Model… 25%");
    assert_eq!(
        h.download_overlay(),
        onda_domain::non_dictation_ui::DownloadOverlayView::Shown {
            status: "Downloading Parakeet TDT v2... 25%".to_owned(),
            progress: onda_domain::non_dictation_ui::OverlayProgress::Determinate(0.25),
        }
    );
    h.app.retry_model_load();
    sleep(10).await;
    assert_eq!(
        h.log.with("download:").len(),
        1,
        "ASR-10: ignored while loading"
    );
    lock(&h.models.downloaded).push(V2);
    h.app
        .download_event(download(V2, DownloadUpdateKind::Finished));
    sleep(10).await;
    assert_eq!(h.script.calls(), vec![EngineCall::Load(V2)]);
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn asr_2_a_failed_load_download_is_a_load_failure() {
    let h = Harness::unstarted(|_| {});
    lock(&h.models.downloaded).clear();
    h.app.ui_ready();
    sleep(10).await;
    h.app.download_event(download(
        V2,
        DownloadUpdateKind::Failed {
            error: "The Internet is closed for the summer.".to_owned(),
        },
    ));
    sleep(10).await;
    assert_eq!(h.notifications(), vec!["Couldn't Load Speech Model"]);
    assert_eq!(h.tray().icon, TrayIcon::Warning);
    assert!(h.menu().retry_model_load.is_some());
}

#[tokio::test(start_paused = true)]
async fn dl_8_download_and_use_shows_progress_on_the_speech_page() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::Speech);
    h.app.download_and_use(UNIFIED);
    sleep(10).await;
    h.app
        .download_event(download(UNIFIED, DownloadUpdateKind::Started));
    h.app
        .download_event(download(UNIFIED, DownloadUpdateKind::Progress(0.5)));
    sleep(10).await;
    let row = |page: &SpeechPage| {
        page.models
            .iter()
            .find(|row| row.id == UNIFIED.as_str())
            .map(|row| row.control.clone())
    };
    assert_eq!(
        row(&speech(&h)),
        Some(ModelRowControl::Downloading {
            progress: 0.5,
            percent: "50%".to_owned(),
            cancel_title: "Cancel".to_owned(),
        })
    );
    lock(&h.models.downloaded).push(UNIFIED);
    h.app
        .download_event(download(UNIFIED, DownloadUpdateKind::Finished));
    sleep(10).await;
    assert_eq!(row(&speech(&h)), Some(ModelRowControl::Selected));
    assert_eq!(
        h.log.with("settings:selected"),
        vec!["settings:selected parakeet-unified"]
    );
}

#[tokio::test(start_paused = true)]
async fn dl_4_failures_alert_on_the_speech_page_else_notify() {
    struct Case {
        name: &'static str,
        speech_visible: bool,
        want_alert: Option<&'static str>,
        want_notifications: Vec<&'static str>,
    }
    let cases = [
        Case {
            name: "speech page visible",
            speech_visible: true,
            want_alert: Some("Failed to download Parakeet Ultra: disk full at Greendale"),
            want_notifications: Vec::new(),
        },
        Case {
            name: "another page visible",
            speech_visible: false,
            want_alert: None,
            want_notifications: vec!["Model Download Failed"],
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        h.app.page_appeared(match tc.speech_visible {
            true => MainWindowSection::Speech,
            false => MainWindowSection::History,
        });
        h.app.download_and_use(ULTRA);
        h.app.download_event(download(
            ULTRA,
            DownloadUpdateKind::Failed {
                error: "disk full at Greendale".to_owned(),
            },
        ));
        sleep(10).await;
        let alert = h.last(|e| match e {
            Event::SpeechChanged(page) => page.alert.map(|a| a.message),
            _ => None,
        });
        assert_eq!(alert.as_deref(), tc.want_alert, "{}", tc.name);
        assert_eq!(h.notifications(), tc.want_notifications, "{}", tc.name);
        // the pending "Download & Use" is cleared (DL-8)
        lock(&h.models.downloaded).push(ULTRA);
        h.app
            .download_event(download(ULTRA, DownloadUpdateKind::Finished));
        sleep(10).await;
        assert!(h.log.with("settings:selected").is_empty(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn win_19_cancel_download_and_compatibility_note() {
    let h = Harness::with(|s| s.language = language("de")).await;
    h.app.page_appeared(MainWindowSection::Speech);
    sleep(10).await;
    let page = speech(&h);
    let note = page.compatibility.expect("a note");
    assert!(note.warning);
    assert_eq!(
        note.text,
        "Parakeet TDT v2 can't transcribe German, so dictation will come out garbled."
    );
    let action = note.action.expect("a fix button");
    assert_eq!(action.model_id, ULTRA.as_str());
    assert!(
        action.title.starts_with("Download & Use Parakeet Ultra ("),
        "{}",
        action.title
    );
    let v2 = page
        .models
        .iter()
        .find(|r| r.id == V2.as_str())
        .expect("v2");
    assert_eq!(v2.unsupported.as_deref(), Some("Doesn't support German"));
    let ultra = page
        .models
        .iter()
        .find(|r| r.id == ULTRA.as_str())
        .expect("ultra");
    assert_eq!(ultra.recommended.as_deref(), Some("Recommended"));

    h.app.download_and_use(ULTRA);
    h.app.cancel_download(ULTRA);
    sleep(10).await;
    assert_eq!(
        h.log.with("download-cancel:"),
        vec!["download-cancel:parakeet-ultra"]
    );
}

// ---------------------------------------------------------------------------
// select and delete (DL-7, DL-9, DL-10)

#[tokio::test(start_paused = true)]
async fn dl_9_dl_10_delete_rules() {
    struct Case {
        name: &'static str,
        downloaded: Vec<ModelId>,
        delete: ModelId,
        delete_error: Option<&'static str>,
        want_deleted: bool,
        want_alert: Option<&'static str>,
        want_selected: Option<&'static str>,
    }
    let cases = [
        Case {
            name: "deleting the selected model selects another",
            downloaded: vec![V2, ULTRA],
            delete: V2,
            delete_error: None,
            want_deleted: true,
            want_alert: None,
            want_selected: Some("settings:selected parakeet-ultra"),
        },
        Case {
            name: "the only model while selected cannot be deleted",
            downloaded: vec![V2],
            delete: V2,
            delete_error: None,
            want_deleted: false,
            want_alert: Some(
                "Failed to delete Parakeet TDT v2: Cannot delete the currently active model. Please select a different model first.",
            ),
            want_selected: None,
        },
        Case {
            name: "a failed delete alerts",
            downloaded: vec![V2, ULTRA],
            delete: ULTRA,
            delete_error: Some("permission denied in the study room"),
            want_deleted: true,
            want_alert: Some(
                "Failed to delete Parakeet Ultra: permission denied in the study room",
            ),
            want_selected: None,
        },
    ];
    for tc in cases {
        let h = Harness::new().await;
        *lock(&h.models.downloaded) = tc.downloaded.clone();
        *lock(&h.models.delete_error) = tc.delete_error.map(str::to_owned);
        h.app.page_appeared(MainWindowSection::Speech);
        sleep(10).await;
        h.app.request_delete_model(tc.delete);
        sleep(1).await;
        let alert = speech(&h).alert.expect("the confirmation");
        assert_eq!(alert.title, "Delete model", "{}", tc.name);
        assert_eq!(alert.confirm.as_deref(), Some("Delete"), "{}", tc.name);
        assert_eq!(alert.dismiss, "Cancel", "{}", tc.name);
        h.app.confirm_speech_alert(alert.id);
        sleep(10).await;
        assert_eq!(
            !h.log.with("delete:").is_empty(),
            tc.want_deleted,
            "{}",
            tc.name
        );
        let shown = speech(&h).alert.map(|a| a.message);
        assert_eq!(shown.as_deref(), tc.want_alert, "{}", tc.name);
        assert_eq!(
            h.log.with("settings:selected").first().map(String::as_str),
            tc.want_selected,
            "{}",
            tc.name
        );
    }
}

#[tokio::test(start_paused = true)]
async fn dl_9_cancelling_the_confirmation_deletes_nothing() {
    let h = Harness::new().await;
    *lock(&h.models.downloaded) = vec![V2, ULTRA];
    h.app.page_appeared(MainWindowSection::Speech);
    sleep(10).await;
    h.app.request_delete_model(ULTRA);
    sleep(1).await;
    let alert = speech(&h).alert.expect("the confirmation");
    h.app.dismiss_speech_alert(alert.id);
    sleep(10).await;
    assert!(h.log.with("delete:").is_empty());
    assert_eq!(speech(&h).alert, None);
}

// ---------------------------------------------------------------------------
// status menu (UI-6..UI-11, LANG-8..LANG-10, DL-11, HIS-28)

#[tokio::test(start_paused = true)]
async fn ui_8_ui_11_menu_items_open_the_right_sections() {
    let h = Harness::new().await;
    h.app.fix_permissions();
    h.app.open_app();
    h.app.open_settings();
    h.app.show_all_history();
    h.app.choose_other_language();
    sleep(1).await;
    assert_eq!(
        opened_sections(&h),
        vec![
            MainWindowSection::General,
            MainWindowSection::GettingStarted,
            MainWindowSection::General,
            MainWindowSection::History,
            MainWindowSection::Speech,
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn status_menu_has_every_item_with_its_rules() {
    let h = Harness::unstarted(|_| {});
    *lock(&h.history.stored) = vec![entry(
        "Troy Barnes studies at Greendale Community College",
        None,
        Some("Slack"),
        1.hours(),
    )];
    *lock(&h.permissions.microphone) = MicrophonePermission::Denied;
    lock(&h.system.updater).status = UpdateStatus::UpToDate;
    h.app.ui_ready();
    sleep(10).await;
    assert!(
        h.menu().update.enabled,
        "read at launch, before the menu opens"
    );
    h.app.menu_will_open();
    sleep(10).await;
    let menu = h.menu();
    assert_eq!(menu.header, "Microphone Access Needed");
    assert_eq!(
        menu.fix_permissions.map(|i| i.title),
        Some("Fix Permissions…")
    );
    assert_eq!(menu.dictation.title, "Start Dictation");
    let recent = menu.recent_transcripts.expect("recent transcripts");
    assert_eq!(recent.items.len(), 1);
    assert_eq!(
        recent.items[0].title,
        "Troy Barnes studies at Greendale Commun…"
    );
    assert_eq!(recent.show_all_title, "Show All History…");
    assert_eq!(menu.language.title, "Language");
    assert_eq!(menu.language.subtitle, "Automatic");
    assert_eq!(menu.language.other_title, "Other Language…");
    assert_eq!(menu.open_app.title, "Open Onda");
    assert_eq!(menu.settings.title, "Settings…");
    assert!(menu.update.enabled, "follows the updater");
    assert_eq!(menu.update.title, "Check for Updates…");
    assert_eq!(menu.quit.title, "Quit Onda");
    assert_eq!(menu.redownload_model, None);

    h.app.update_action(menu.update.action);
    sleep(10).await;
    assert_eq!(h.log.with("updater:"), vec!["updater:check"]);
}

#[tokio::test(start_paused = true)]
async fn upd_8_the_update_restart_waits_for_dictation() {
    let h = Harness::new().await;
    lock(&h.system.updater).status = UpdateStatus::Ready {
        version: "2.0.0".to_owned(),
    };
    h.app.updater_changed();
    sleep(10).await;
    let ready = h.menu().update;
    assert_eq!(ready.title, "Restart to Update");
    assert!(ready.enabled);

    h.app.toggle_dictation_from_menu();
    sleep(100).await;
    assert!(!h.menu().update.enabled, "disabled while recording");
    h.app.update_action(UpdateAction::Restart);
    sleep(10).await;
    assert!(
        h.log.with("updater:").is_empty(),
        "no restart while recording"
    );

    h.app.cancel_dictation();
    sleep(100).await;
    assert!(h.idle());
    assert!(h.menu().update.enabled);
    h.app.update_action(UpdateAction::Restart);
    h.app.update_action(UpdateAction::Download);
    sleep(10).await;
    assert_eq!(
        h.log.with("updater:"),
        vec!["updater:restart", "updater:download"]
    );
}

#[tokio::test(start_paused = true)]
async fn upd_8_an_accepted_restart_blocks_dictation_until_it_is_declined() {
    let h = Harness::new().await;
    lock(&h.system.updater).status = UpdateStatus::Ready {
        version: "2.0.0".to_owned(),
    };
    h.app.updater_changed();
    sleep(10).await;
    let (answer, answered) = std::sync::mpsc::channel();
    *lock(&h.system.restart_answer) = Some(answered);

    // the paused clock stands still while the fake blocks, so this part
    // polls in real time
    h.app.update_action(UpdateAction::Restart);
    eventually("the restart starts", || {
        h.log.with("updater:") == ["updater:restart"]
    })
    .await;
    eventually("the control disables", || !h.menu().update.enabled).await;

    // the install is still running: neither the menu nor the hotkey starts
    // a capture
    h.app.toggle_dictation_from_menu();
    catch_up(&h, 1).await;
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 0);
    h.press();
    catch_up(&h, 2).await;
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 0);
    h.release();
    h.app.update_action(UpdateAction::Restart);
    catch_up(&h, 3).await;
    assert!(h.idle());
    assert_eq!(
        h.log.count("updater:restart"),
        1,
        "a second restart is ignored while one runs"
    );

    // the install failed: dictation works again
    answer.send(false).unwrap();
    eventually("the restart ends", || h.menu().update.enabled).await;
    h.app.toggle_dictation_from_menu();
    sleep(100).await;
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 1);
    assert!(!h.idle());
    h.app.update_action(UpdateAction::Restart);
    sleep(10).await;
    assert_eq!(h.log.count("updater:restart"), 1, "refused while dictating");
}

#[tokio::test(start_paused = true)]
async fn upd_8_a_restart_that_relaunches_keeps_dictation_blocked() {
    let h = Harness::new().await;
    lock(&h.system.updater).status = UpdateStatus::Ready {
        version: "2.0.0".to_owned(),
    };
    h.app.updater_changed();
    sleep(10).await;
    h.app.update_action(UpdateAction::Restart);
    sleep(10).await;
    h.app.toggle_dictation_from_menu();
    sleep(100).await;
    h.press();
    sleep(1_000).await;
    h.release();
    sleep(100).await;
    assert_eq!(h.log.with("updater:"), vec!["updater:restart"]);
    assert_eq!(h.audio.starts.load(Ordering::SeqCst), 0);
}

/// Waits in real time until the actor has handled everything sent before
/// this call (it handles messages in order, and the `n`th check marks the
/// spot), then gives any start it began time to reach the microphone.
async fn catch_up(h: &Harness, n: usize) {
    h.app.update_action(UpdateAction::Check);
    eventually("the actor catches up", || h.log.count("updater:check") == n).await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[tokio::test(start_paused = true)]
async fn dl_11_redownload_replaces_the_files_then_loads() {
    let h = Harness::unstarted(|_| {});
    h.script.fail_load.store(true, Ordering::SeqCst);
    h.app.ui_ready();
    sleep(10).await;
    assert!(
        h.menu().redownload_model.is_some(),
        "files on disk, load failed"
    );
    h.script.fail_load.store(false, Ordering::SeqCst);
    h.app.redownload_model();
    sleep(10).await;
    assert_eq!(
        h.log.with("redownload:"),
        vec!["redownload:parakeet-tdt-v2"]
    );
    h.app.redownload_model();
    sleep(10).await;
    assert_eq!(h.log.with("redownload:").len(), 1, "ignored while it runs");
    h.app
        .download_event(download(V2, DownloadUpdateKind::Finished));
    sleep(10).await;
    assert_eq!(h.tray().icon, TrayIcon::Ready);
}

#[tokio::test(start_paused = true)]
async fn lang_8_to_10_menu_language_switch() {
    let h = Harness::with(|s| s.language = en()).await;
    *lock(&h.models.downloaded) = vec![V2, ULTRA];
    h.app.menu_will_open();
    sleep(10).await;
    assert_eq!(
        settings_log(&h, "RecentLanguages"),
        vec!["settings:RecentLanguages([Specific(Language(10))])"],
        "LANG-8: opening records the current language"
    );

    h.app.select_language_from_menu(language("de"));
    sleep(10).await;
    assert_eq!(
        h.log.with("settings:selected"),
        vec!["settings:selected parakeet-ultra"]
    );
    let menu = h.menu();
    let names: Vec<&str> = menu
        .language
        .items
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(names, vec!["English", "German"], "sorted by name");
    assert_eq!(menu.language.subtitle, "German");

    // Ultra supports French: kept
    h.app.select_language_from_menu(language("fr"));
    sleep(10).await;
    assert_eq!(h.log.with("settings:selected").len(), 1);

    // nothing installed transcribes Japanese
    h.app.select_language_from_menu(language("ja"));
    sleep(10).await;
    assert_eq!(opened_sections(&h), vec![MainWindowSection::Speech]);

    // disabled while a session runs
    h.press();
    sleep(400).await;
    h.app.select_language_from_menu(en());
    sleep(10).await;
    assert!(!h.menu().language.items_enabled);
    assert_eq!(h.menu().language.subtitle, "Japanese");
}

// ---------------------------------------------------------------------------
// General (WIN-8..WIN-15, LOGIN-1..LOGIN-3)

#[tokio::test(start_paused = true)]
async fn win_9_input_picker_and_live_device_list() {
    let h = Harness::with(|s| s.input_device_uid = "troy-podcast-mic".to_owned()).await;
    *lock(&h.audio.devices) = vec![onda_app::seams::InputDeviceInfo {
        uid: "built-in".to_owned(),
        name: "MacBook Pro Microphone".to_owned(),
        is_default: true,
    }];
    h.app.page_appeared(MainWindowSection::General);
    sleep(10).await;
    let input = general(&h).input;
    let names: Vec<&str> = input.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "System Default",
            "MacBook Pro Microphone",
            "Unavailable Microphone"
        ]
    );
    assert!(input.warning.is_some());

    lock(&h.audio.devices).push(onda_app::seams::InputDeviceInfo {
        uid: "troy-podcast-mic".to_owned(),
        name: "Troy's Podcast Mic".to_owned(),
        is_default: false,
    });
    h.app.input_devices_changed();
    sleep(10).await;
    let input = general(&h).input;
    assert_eq!(input.warning, None);
    assert_eq!(
        input.caption.as_deref(),
        Some("Onda will keep using Troy's Podcast Mic when the system default changes.")
    );

    h.app.select_input_device("");
    sleep(10).await;
    assert_eq!(*lock(&h.audio.selected), vec![String::new()]);
    assert_eq!(
        general(&h).input.caption.as_deref(),
        Some("Follows the input selected in macOS. Currently using MacBook Pro Microphone.")
    );
}

#[tokio::test(start_paused = true)]
async fn login_1_to_3_launch_at_login_state_and_errors() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::General);
    sleep(10).await;
    let page = general(&h);
    assert!(!page.launch_at_login.on);
    assert!(page.launch_at_login.enabled);
    assert_eq!(page.version_footer, "Current version: 1.2.3");
    assert!(!page.update.enabled, "the updater is off in this build");
    assert!(!page.automatic_updates.enabled);
    assert_eq!(
        page.update_caption.map(|caption| caption.text).as_deref(),
        Some("Updates are off in development builds.")
    );

    *lock(&h.system.login_error) = Some("Operation not permitted".to_owned());
    h.app.set_launch_at_login(true);
    sleep(10).await;
    let caption = general(&h)
        .launch_at_login_caption
        .expect("an error caption");
    assert_eq!(
        caption.text,
        "Couldn't add Onda to Login Items: Operation not permitted"
    );

    *lock(&h.system.login_error) = None;
    h.app.set_launch_at_login(true);
    sleep(10).await;
    let page = general(&h);
    assert!(page.launch_at_login.on);
    assert_eq!(
        page.launch_at_login_caption, None,
        "a success clears the error"
    );

    *lock(&h.system.login) = Some(LoginItemRegistryStatus::RequiresApproval);
    h.app.app_became_active();
    sleep(10).await;
    let caption = general(&h)
        .launch_at_login_caption
        .expect("approval caption");
    assert_eq!(caption.button.as_deref(), Some("Open Login Items"));
    h.app.open_login_items();
    sleep(10).await;
    assert_eq!(h.log.with("login:open"), vec!["login:open-settings"]);

    *lock(&h.system.login) = None;
    h.app.page_appeared(MainWindowSection::General);
    sleep(10).await;
    assert!(
        !general(&h).launch_at_login.enabled,
        "unbundled: unavailable"
    );
}

#[tokio::test(start_paused = true)]
async fn ui_2_window_presence_follows_open_windows() {
    let h = Harness::new().await;
    let presence = |h: &Harness| {
        h.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::AppPresenceChanged { regular } => Some(regular),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    h.app.main_window_appeared();
    h.app.page_appeared(MainWindowSection::About);
    sleep(1).await;
    h.app.main_window_closed();
    sleep(1).await;
    assert_eq!(presence(&h), vec![true, false]);
    let about = h
        .last(|e| match e {
            Event::AboutChanged(page) => Some(*page),
            _ => None,
        })
        .expect("the About page");
    assert_eq!(about.version, "Version 1.2.3");
    assert_eq!(about.models.len(), 4);
}

// ---------------------------------------------------------------------------
// Post-Processing (CLN-24..CLN-27)

#[tokio::test(start_paused = true)]
async fn cln_25_the_key_field_loads_and_saves_by_the_rules() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(key.text, "sk-or-greendale");
    assert_eq!(key.revision, 1);
    assert!(!key.loading);
    assert_eq!(h.count(|e| matches!(e, Event::ActivateApp)), 1);

    // unchanged: no write; changed: saved on commit
    h.app.api_key_commit();
    h.app.api_key_edited("  sk-or-dean-pelton  ");
    sleep(1).await;
    assert_eq!(
        post_processing(&h)
            .details
            .expect("details")
            .dialog
            .expect("sheet")
            .key
            .revision,
        1,
        "typing never replaces the field"
    );
    h.app.api_key_commit();
    sleep(10).await;
    assert_eq!(h.secrets.stored().as_deref(), Some("sk-or-dean-pelton"));

    // saved again when the page disappears; blank deletes
    h.app.api_key_edited("");
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    sleep(10).await;
    assert_eq!(h.secrets.stored(), None);
}

#[tokio::test(start_paused = true)]
async fn cln_24_a_failed_load_shows_inline_and_blank_never_deletes() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    *lock(&h.secrets.0) = Err("Keychain read failed (OSStatus -25293).".to_owned());
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(
        key.error.as_deref(),
        Some("Keychain read failed (OSStatus -25293).")
    );
    h.app.api_key_commit();
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    sleep(10).await;
    assert!(
        lock(&h.secrets.0).is_err(),
        "the unknown stored key survives"
    );

    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.app.api_key_edited("sk-or-britta");
    h.app.api_key_commit();
    sleep(10).await;
    assert_eq!(h.secrets.stored().as_deref(), Some("sk-or-britta"));
    assert_eq!(
        post_processing(&h)
            .details
            .expect("details")
            .dialog
            .expect("sheet")
            .key
            .error,
        None
    );
}

#[tokio::test(start_paused = true)]
async fn cln_27_the_test_runs_resets_and_cancels_silently() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.cleanup.respond(vec![Ok(
        "Tell Troy and Abed that the study group meets at 4 in the library. Thanks!".to_owned(),
    )]);
    h.app.api_key_edited("sk-or-annie");
    h.app.test_cleanup();
    sleep(10).await;
    assert_eq!(
        h.secrets.stored().as_deref(),
        Some("sk-or-annie"),
        "saved first"
    );
    let details = post_processing(&h).details.expect("details");
    let result = details.test_result.expect("a result");
    assert_eq!(
        result.said,
        "Said: um so tell troy and abed that the study group meets at three no wait four in the library thanks"
    );
    assert_eq!(
        result.result,
        "Result: Tell Troy and Abed that the study group meets at 4 in the library. Thanks!"
    );
    assert_eq!(h.cleanup.user_messages().len(), 1);
    assert!(
        !h.cleanup.user_messages()[0].contains("Dictating into"),
        "no app name"
    );

    h.app.select_cleanup_model(
        onda_domain::cleanup::CleanupProvider::OpenRouter,
        onda_domain::cleanup::CLEANUP_MODELS[1].slug.as_ref(),
    );
    sleep(1).await;
    assert_eq!(
        post_processing(&h).details.expect("details").test_result,
        None
    );

    // an error alerts
    h.cleanup
        .respond(vec![Err(onda_domain::cleanup::CleanupError::Http {
            status: 429,
            message: None,
        })]);
    h.app.test_cleanup();
    sleep(10).await;
    let error = post_processing(&h)
        .details
        .expect("details")
        .test_error
        .expect("an alert");
    assert_eq!(error.title, "Test Failed");
    assert_eq!(error.message, "OpenRouter returned HTTP 429.");
    h.app.dismiss_cleanup_test_error();

    // leaving cancels a running test silently
    let hold = Arc::new(tokio::sync::Notify::new());
    *lock(&h.cleanup.hold) = Some(hold.clone());
    h.cleanup.respond(vec![Ok("Cool cool cool.".to_owned())]);
    h.app.test_cleanup();
    sleep(10).await;
    assert!(post_processing(&h).details.expect("details").test_running);
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    hold.notify_waiters();
    sleep(10).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    let details = post_processing(&h).details.expect("details");
    assert!(!details.test_running);
    assert_eq!(details.test_result, None);
    assert_eq!(details.test_error, None);
}

// ---------------------------------------------------------------------------
// Replacements (REP-8..REP-12)

#[tokio::test(start_paused = true)]
async fn rep_12_debounced_writes_flush_on_disappear_and_skip_unchanged() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::Replacements);
    sleep(1).await;
    let page = replacements(&h);
    assert_eq!(
        page.empty_caption.as_deref(),
        Some(
            "No rules yet. Add a phrase Onda keeps getting wrong and what it should type instead."
        )
    );
    h.app.add_replacement_rule();
    sleep(1).await;
    let page = replacements(&h);
    let id = page.rows[0].id.clone();
    let revision = page.revision;
    h.app.set_replacement_triggers(id.clone(), "troy, ");
    h.app.set_replacement_text(id.clone(), "Troy Barnes");
    sleep(400).await;
    assert!(
        settings_log(&h, "ReplacementRules").is_empty(),
        "still debouncing"
    );
    let page = replacements(&h);
    assert_eq!(
        page.rows[0].triggers, "troy, ",
        "raw text kept while typing"
    );
    assert_eq!(page.revision, revision, "typing never bumps the revision");
    sleep(200).await;
    let writes = settings_log(&h, "ReplacementRules");
    assert_eq!(writes.len(), 1);
    assert!(writes[0].contains("triggers: [\"troy\"]"), "{}", writes[0]);

    // a blank row is dropped, so nothing changed and nothing is written
    h.app.add_replacement_rule();
    sleep(600).await;
    assert_eq!(settings_log(&h, "ReplacementRules").len(), 1);

    // disappearing flushes at once
    h.app.set_replacement_text(id.clone(), "Troy and Abed");
    h.app.page_disappeared(MainWindowSection::Replacements);
    sleep(1).await;
    assert_eq!(settings_log(&h, "ReplacementRules").len(), 2);

    // reloading and leaving without edits writes nothing
    h.app.page_appeared(MainWindowSection::Replacements);
    sleep(1).await;
    assert_eq!(replacements(&h).rows[0].triggers, "troy");
    h.app.main_window_closed();
    sleep(600).await;
    assert_eq!(settings_log(&h, "ReplacementRules").len(), 2);

    h.app.page_appeared(MainWindowSection::Replacements);
    sleep(1).await;
    h.app.delete_replacement_rule(id);
    sleep(600).await;
    assert_eq!(
        settings_log(&h, "ReplacementRules")
            .last()
            .map(String::as_str),
        Some("settings:ReplacementRules([])")
    );
}

#[tokio::test(start_paused = true)]
async fn rep_12_quitting_flushes_pending_edits() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::Replacements);
    sleep(1).await;
    h.app.add_replacement_rule();
    sleep(1).await;
    let id = replacements(&h).rows[0].id.clone();
    h.app.set_replacement_triggers(id, "greendale");
    let log = h.log.clone();
    h.app.shutdown_async().await;
    assert_eq!(log.with("settings:ReplacementRules").len(), 1);
}

// ---------------------------------------------------------------------------
// History (HIS-16..HIS-27)

#[tokio::test(start_paused = true)]
async fn his_19_his_21_his_26_search_sections_and_empty_states() {
    let h = Harness::new().await;
    *lock(&h.history.stored) = vec![
        entry(
            "Troy Barnes studies at Greendale",
            None,
            Some("Slack"),
            1.minutes(),
        ),
        entry(
            "Abed Nadir films the study group",
            Some("abed nadir films um the study group"),
            Some("Notes"),
            25.hours(),
        ),
        entry("   ", None, None, 26.hours()),
    ];
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;
    let page = history(&h);
    let HistoryContent::Sections { sections } = page.content else {
        panic!("sections");
    };
    let headings: Vec<DayHeading> = sections.iter().map(|s| s.heading.clone()).collect();
    assert_eq!(
        headings[0],
        DayHeading::Relative {
            text: "Today".to_owned()
        }
    );
    let first = &sections[0].rows[0];
    assert!(
        first.details.starts_with("Slack · 0:07"),
        "{}",
        first.details
    );
    assert_eq!(
        first.transcript.as_deref(),
        Some("Troy Barnes studies at Greendale")
    );
    let rows: Vec<_> = sections.iter().flat_map(|s| s.rows.iter()).collect();
    assert_eq!(
        rows[1].original.as_deref(),
        Some("abed nadir films um the study group")
    );
    assert_eq!(rows[2].transcript, None, "shows No text");
    let menu: Vec<&str> = rows[1].menu.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(menu, vec!["Copy", "Copy Original", "", "Delete"]);
    assert!(page.clear_button.enabled);

    h.app.search_history("TROY greendale");
    sleep(1).await;
    let HistoryContent::Sections { sections } = history(&h).content else {
        panic!("sections");
    };
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].rows.len(), 1);

    h.app.search_history("pierce");
    sleep(1).await;
    assert_eq!(
        history(&h).content,
        HistoryContent::NoSearchResults {
            query: "pierce".to_owned()
        }
    );

    lock(&h.history.stored).clear();
    h.app.set_history_enabled(false);
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;
    let page = history(&h);
    assert_eq!(
        page.content,
        HistoryContent::Empty {
            title: "History Is Off".to_owned(),
            message: "Turn it on to keep your transcripts on this Mac.".to_owned(),
            action: Some("Turn On History".to_owned()),
        }
    );
    assert!(!page.retention_enabled);
    assert!(!page.keep_audio.enabled);
    assert!(!page.clear_button.enabled);
}

#[tokio::test(start_paused = true)]
async fn his_17_shortening_retention_asks_first() {
    let h = Harness::new().await;
    *lock(&h.history.stored) = vec![
        entry("Señor Chang teaches Spanish", None, None, 1.hours()),
        entry("Pierce Hawthorne moist towelettes", None, None, 240.hours()),
        entry("Shirley's sandwiches", None, None, 250.hours()),
    ];
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;
    h.app.set_history_retention(HistoryRetention::Week);
    sleep(1).await;
    let dialog = history(&h).dialog.expect("a confirmation");
    assert_eq!(dialog.title, "Delete 2 transcripts older than 1 week?");
    assert_eq!(dialog.confirm_title, "Delete Older Transcripts");
    h.app.cancel_history_dialog();
    sleep(1).await;
    let page = history(&h);
    assert_eq!(page.dialog, None);
    assert_eq!(page.retention, HistoryRetention::Month, "unchanged");

    h.app.set_history_retention(HistoryRetention::Week);
    h.app.confirm_history_dialog();
    sleep(10).await;
    assert_eq!(
        settings_log(&h, "HistoryRetention"),
        vec!["settings:HistoryRetention(Week)"]
    );
    let page = history(&h);
    assert_eq!(page.retention, HistoryRetention::Week);
    let HistoryContent::Sections { sections } = page.content else {
        panic!("sections");
    };
    assert_eq!(sections.iter().map(|s| s.rows.len()).sum::<usize>(), 1);

    // lengthening never asks
    h.app.set_history_retention(HistoryRetention::Forever);
    sleep(10).await;
    assert_eq!(history(&h).dialog, None);
    assert_eq!(history(&h).retention, HistoryRetention::Forever);
}

#[tokio::test(start_paused = true)]
async fn his_20_his_22_his_24_clear_copy_and_missing_recordings() {
    let h = Harness::new().await;
    let troy = entry("  Troy Barnes  ", None, None, 1.hours());
    let id = troy.id;
    *lock(&h.history.stored) = vec![troy];
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;

    h.app.copy_history_entry(id);
    sleep(10).await;
    assert_eq!(
        *lock(&h.text.copied),
        vec!["Troy Barnes"],
        "HIS-25: trimmed"
    );
    assert_eq!(history(&h).copied_entry, Some(id.to_string()));
    sleep(1_500).await;
    assert_eq!(history(&h).copied_entry, None);

    h.app.play_history_recording(id);
    sleep(10).await;
    let error = history(&h).error.expect("an error");
    assert_eq!(error.title, "History Error");
    assert_eq!(
        error.message,
        "The recording for this dictation is missing."
    );
    h.app.dismiss_history_error();

    h.app.request_clear_history();
    sleep(1).await;
    let dialog = history(&h).dialog.expect("a confirmation");
    assert_eq!(dialog.title, "Delete all 1 saved transcripts?");
    h.app.confirm_history_dialog();
    sleep(10).await;
    assert_eq!(*lock(&h.history.ops).last().expect("an op"), "delete all");
    assert_eq!(
        history(&h).content,
        HistoryContent::Empty {
            title: "No Dictations Yet".to_owned(),
            message: "Your transcripts appear here after you dictate.".to_owned(),
            action: None,
        }
    );
}

#[tokio::test(start_paused = true)]
async fn his_10_a_new_dictation_refreshes_a_visible_history_page() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;
    let before = h.count(|e| matches!(e, Event::HistoryPageChanged(_)));
    lock(&h.history.stored).push(entry("Cool. Cool cool cool.", None, None, 1.minutes()));
    h.dictate().await;
    assert!(h.count(|e| matches!(e, Event::HistoryPageChanged(_))) > before);
}

// ---------------------------------------------------------------------------
// Getting Started and Hotkey (WIN-4..WIN-7, WIN-16, WIN-17, HOT-8)

#[tokio::test(start_paused = true)]
async fn win_5_checklist_and_download_base_model() {
    let h = Harness::new().await;
    lock(&h.models.downloaded).clear();
    *lock(&h.permissions.microphone) = MicrophonePermission::Denied;
    h.app.page_appeared(MainWindowSection::GettingStarted);
    sleep(10).await;
    let page = h
        .last(|e| match e {
            Event::GettingStartedChanged(page) => Some(*page),
            _ => None,
        })
        .expect("Getting Started");
    assert_eq!(page.ready_line, None);
    let titles: Vec<Option<String>> = page
        .steps
        .iter()
        .map(|s| s.button.as_ref().map(|b| b.title.clone()))
        .collect();
    assert_eq!(
        titles,
        vec![
            Some("Grant Access".to_owned()),
            None,
            Some("Download Base Model".to_owned()),
        ]
    );
    assert_eq!(page.keycaps, vec!["⌥", "Space"]);

    h.app
        .setup_action(onda_domain::setup::SetupAction::DownloadBaseModel);
    h.app
        .setup_action(onda_domain::setup::SetupAction::RequestMicrophone);
    sleep(10).await;
    assert_eq!(h.log.with("download:"), vec!["download:parakeet-tdt-v2"]);
    assert_eq!(
        h.log.with("permissions:open-settings"),
        vec!["permissions:open-settings Microphone"],
        "a denied microphone opens the pane"
    );
}

#[tokio::test(start_paused = true)]
async fn hot_8_leaving_the_hotkey_page_ends_a_capture() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::Hotkey);
    h.app.begin_hotkey_capture();
    sleep(10).await;
    assert!(h.hotkey_view().recording);
    let page = h
        .last(|e| match e {
            Event::HotkeyPageChanged(page) => Some(*page),
            _ => None,
        })
        .expect("Hotkey page");
    assert!(!page.reset.enabled);
    h.app.page_disappeared(MainWindowSection::Hotkey);
    sleep(10).await;
    assert!(!h.hotkey_view().recording);
    assert!(
        !h.hotkey_view().listening,
        "the shell drops its key monitor"
    );
    assert_eq!(
        h.log.with("hotkey:register").last().map(String::as_str),
        Some("hotkey:register ⌥ Space")
    );

    h.app
        .set_activation_mode(onda_domain::types::ActivationMode::Automatic);
    h.app.page_appeared(MainWindowSection::Hotkey);
    sleep(10).await;
    let page = h
        .last(|e| match e {
            Event::HotkeyPageChanged(page) => Some(*page),
            _ => None,
        })
        .expect("Hotkey page");
    assert_eq!(
        page.activation_description,
        "Quick tap toggles recording; holding behaves like push-to-talk."
    );
}

#[tokio::test(start_paused = true)]
async fn his_16_retention_applies_at_launch_and_preloads_the_menu() {
    let h = Harness::unstarted(|_| {});
    *lock(&h.history.stored) = vec![
        entry("Britta's in the study group", None, None, 1.hours()),
        entry("Expired at Greendale", None, None, (31 * 24).hours()),
    ];
    h.app.ui_ready();
    sleep(10).await;
    assert_eq!(*lock(&h.history.ops), vec!["retention Month"]);
    let recent = h.menu().recent_transcripts.expect("recent transcripts");
    let titles: Vec<&str> = recent.items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["Britta's in the study group"]);
}

#[tokio::test(start_paused = true)]
async fn cln_25_typing_during_the_load_cancels_it() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    h.app.api_key_edited("sk-or-chang");
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(key.revision, 0, "the late load never replaces the field");
    assert!(!key.loading);
    assert_eq!(h.count(|e| matches!(e, Event::ActivateApp)), 0);
    h.app.api_key_commit();
    sleep(10).await;
    assert_eq!(h.secrets.stored().as_deref(), Some("sk-or-chang"));
}

#[tokio::test(start_paused = true)]
async fn cln_25_a_failed_reload_blanks_the_field_and_saves_nothing() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    assert_eq!(
        post_processing(&h)
            .details
            .expect("details")
            .dialog
            .expect("sheet")
            .key
            .text,
        "sk-or-greendale"
    );
    h.app.page_disappeared(MainWindowSection::PostProcessing);

    *lock(&h.secrets.0) = Err("Keychain read failed (OSStatus -25293).".to_owned());
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(key.text, "", "the earlier key is never shown again");
    assert_eq!(key.revision, 2, "the field rebinds to the blank text");
    assert!(key.error.is_some());
    h.app.api_key_commit();
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    sleep(10).await;
    assert!(
        lock(&h.secrets.0).is_err(),
        "the stale key is never saved again"
    );
}

#[tokio::test(start_paused = true)]
async fn cln_25_a_reload_waits_for_the_save_in_flight() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    let hold = h.secrets.hold_save();
    h.app.api_key_edited("  sk-or-dean-pelton  ");
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    common::eventually("the save", || hold.started()).await;

    // a quick revisit: the load queues behind the save (the paused clock
    // cannot advance while the save blocks, so poll in real time)
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    common::eventually("the revisit", || {
        post_processing(&h)
            .details
            .and_then(|details| details.dialog)
            .is_some_and(|dialog| dialog.key.loading)
    })
    .await;
    hold.release();
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(key.text, "sk-or-dean-pelton", "never the older key");
    assert!(!key.loading);
    assert_eq!(h.secrets.stored().as_deref(), Some("sk-or-dean-pelton"));
}

#[tokio::test(start_paused = true)]
async fn cln_24_a_save_shows_the_trimmed_value() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.app.api_key_edited("  sk-or-annie  ");
    h.app.api_key_commit();
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(key.text, "sk-or-annie");
    assert_eq!(key.revision, 2);
}

#[tokio::test(start_paused = true)]
async fn cln_25_the_key_loads_only_while_the_sheet_shows() {
    let h = Harness::new().await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    assert!(post_processing(&h).details.is_none(), "cleanup is off");
    assert_eq!(
        h.count(|e| matches!(e, Event::ActivateApp)),
        0,
        "no sheet, no keychain read, nothing pulled forward"
    );

    // turning it on shows the page, not the key field
    h.app.set_cleanup_enabled(true);
    sleep(10).await;
    let details = post_processing(&h).details.expect("details");
    assert_eq!(details.dialog, None);
    assert_eq!(h.count(|e| matches!(e, Event::ActivateApp)), 0);

    // opening the sheet loads the key
    h.app.open_providers_dialog();
    sleep(10).await;
    let key = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet")
        .key;
    assert_eq!(key.text, "sk-or-greendale");
    assert_eq!(h.count(|e| matches!(e, Event::ActivateApp)), 1);

    // "Done" saves what was typed and closes the sheet
    h.app.api_key_edited("sk-or-shirley");
    h.app.close_providers_dialog();
    sleep(10).await;
    assert_eq!(h.secrets.stored().as_deref(), Some("sk-or-shirley"));
    assert_eq!(post_processing(&h).details.expect("details").dialog, None);

    // every keychain call names the provider whose key it is
    let openrouter = onda_domain::cleanup::CleanupProvider::OpenRouter;
    assert_eq!(
        *lock(&h.secrets.2),
        [("read", openrouter), ("save", openrouter)]
    );

    // turning cleanup off closes the sheet and abandons a load in flight
    h.app.open_providers_dialog();
    h.app.set_cleanup_enabled(false);
    sleep(10).await;
    assert_eq!(h.count(|e| matches!(e, Event::ActivateApp)), 1);

    // leaving the page closes the sheet and saves
    h.app.set_cleanup_enabled(true);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.app.api_key_edited("sk-or-pierce");
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    sleep(10).await;
    assert_eq!(h.secrets.stored().as_deref(), Some("sk-or-pierce"));
    h.app.page_appeared(MainWindowSection::PostProcessing);
    sleep(10).await;
    assert_eq!(post_processing(&h).details.expect("details").dialog, None);
}

#[tokio::test(start_paused = true)]
async fn the_model_sheet_picks_the_model_in_one_click() {
    use onda_domain::cleanup::{CleanupError, CleanupModel, CleanupProvider};

    let openrouter = CleanupProvider::OpenRouter;
    let in_use = |h: &Harness| {
        let settings = lock(&h.settings.settings);
        (
            settings.cleanup_provider,
            settings.cleanup_model.slug.clone().into_owned(),
        )
    };
    let sheet = |h: &Harness| {
        post_processing(h)
            .details
            .expect("details")
            .dialog
            .expect("sheet")
    };
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    *lock(&h.cleanup.listed) = Some(Err(CleanupError::Transport("offline".to_owned())));
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    assert_eq!(
        h.cleanup.list_calls.load(Ordering::SeqCst),
        1,
        "page and sheet share one listing"
    );
    let dialog = sheet(&h);
    assert_eq!(
        dialog.models_error.as_deref(),
        Some("Couldn't load OpenRouter models: OpenRouter request failed: offline")
    );
    assert_eq!(
        dialog.selected_model.as_deref(),
        Some("~openai/gpt-luna-latest")
    );

    // refreshing replaces the error with the list
    *lock(&h.cleanup.listed) = Some(Ok(vec![
        CleanupModel::listed("openai/gpt-4o", "OpenAI: GPT-4o"),
        CleanupModel::listed("x-ai/grok", "xAI: Grok"),
    ]));
    h.app.refresh_cleanup_models();
    sleep(10).await;
    let dialog = sheet(&h);
    assert_eq!(dialog.models_error, None);
    let titles: Vec<&str> = dialog
        .model_groups
        .iter()
        .map(|g| g.title.as_str())
        .collect();
    assert_eq!(titles, ["Recommended", "All OpenRouter Models"]);

    // clicking a listed model uses it at once
    h.app.select_cleanup_model(openrouter, "openai/gpt-4o");
    sleep(1).await;
    assert_eq!(in_use(&h), (openrouter, "openai/gpt-4o".to_owned()));
    assert_eq!(sheet(&h).selected_model.as_deref(), Some("openai/gpt-4o"));
    let details = post_processing(&h).details.expect("details");
    assert_eq!(details.provider_name, "OpenRouter");
    assert_eq!(details.model_name, "OpenAI: GPT-4o");

    // a typed ID is trimmed and used; a blank one is ignored
    h.app
        .select_cleanup_model(openrouter, "  greendale/human-being ");
    h.app.select_cleanup_model(openrouter, "   ");
    sleep(1).await;
    assert_eq!(in_use(&h), (openrouter, "greendale/human-being".to_owned()));
    let dialog = sheet(&h);
    assert_eq!(dialog.model_groups[1].title, "Custom");
    assert_eq!(
        post_processing(&h).details.expect("details").model_name,
        "greendale/human-being"
    );

    // "Done" closes the sheet; the choice stays
    h.app.close_providers_dialog();
    sleep(1).await;
    assert_eq!(post_processing(&h).details.expect("details").dialog, None);
    assert_eq!(in_use(&h), (openrouter, "greendale/human-being".to_owned()));

    // the list is fetched once; showing the page again reuses it
    h.app.page_disappeared(MainWindowSection::PostProcessing);
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    assert_eq!(h.cleanup.list_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn fireworks_lists_its_models_only_with_its_key() {
    use onda_domain::cleanup::{CleanupModel, CleanupProvider, FIREWORKS_MODELS};

    let fireworks = CleanupProvider::Fireworks;
    let sheet = |h: &Harness| {
        post_processing(h)
            .details
            .expect("details")
            .dialog
            .expect("sheet")
    };
    let h = Harness::with(|s| {
        s.cleanup_enabled = true;
        s.cleanup_provider = CleanupProvider::Fireworks;
        s.cleanup_model = FIREWORKS_MODELS[0].clone();
    })
    .await;
    *lock(&h.cleanup.listed) = Some(Ok(vec![CleanupModel::listed(
        "accounts/fireworks/models/kimi-k3",
        "Kimi K3",
    )]));

    // the page reads no key, so it lists nothing for Fireworks
    h.app.page_appeared(MainWindowSection::PostProcessing);
    sleep(10).await;
    let details = post_processing(&h).details.expect("details");
    assert_eq!(details.provider_name, "Fireworks AI");
    assert_eq!(details.model_name, "DeepSeek V4.1 Flash");
    assert!(lock(&h.secrets.2).is_empty(), "no keychain read");
    assert_eq!(h.cleanup.list_calls.load(Ordering::SeqCst), 0);

    // the sheet lists with the stored key
    h.app.open_providers_dialog();
    sleep(10).await;
    assert_eq!(
        *lock(&h.cleanup.list_requests),
        [(fireworks, Some("sk-or-greendale".to_owned()))]
    );
    let dialog = sheet(&h);
    assert_eq!(dialog.key.placeholder, "Fireworks AI API key");
    assert_eq!(
        dialog.model_groups.last().map(|g| g.title.as_str()),
        Some("All Fireworks AI Models")
    );
    assert!(
        lock(&h.secrets.2).iter().all(|(_, p)| *p == fireworks),
        "every keychain call is Fireworks'"
    );

    // saving a new key lists again with it
    h.app.api_key_edited("fw-annie-edison");
    h.app.api_key_commit();
    sleep(10).await;
    assert_eq!(
        lock(&h.cleanup.list_requests).last(),
        Some(&(fireworks, Some("fw-annie-edison".to_owned())))
    );

    // with no key, the sheet asks for one
    h.app.api_key_edited("");
    h.app.api_key_commit();
    sleep(10).await;
    assert_eq!(
        sheet(&h).models_error.as_deref(),
        Some("Enter your Fireworks AI API key to see its models.")
    );
}

#[tokio::test(start_paused = true)]
async fn picking_another_providers_model_switches_the_provider() {
    use onda_domain::cleanup::CleanupProvider;

    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.app.select_dialog_provider(CleanupProvider::Fireworks);
    sleep(10).await;
    let dialog = post_processing(&h)
        .details
        .expect("details")
        .dialog
        .expect("sheet");
    assert_eq!(dialog.selected_provider, "fireworks");
    assert_eq!(dialog.selected_model, None, "OpenRouter is still in use");

    h.app.select_cleanup_model(
        CleanupProvider::Fireworks,
        "accounts/fireworks/models/deepseek-v4p1-flash",
    );
    sleep(1).await;
    let settings = lock(&h.settings.settings).clone();
    assert_eq!(settings.cleanup_provider, CleanupProvider::Fireworks);
    assert_eq!(
        settings.cleanup_model.slug,
        "accounts/fireworks/models/deepseek-v4p1-flash"
    );
    let details = post_processing(&h).details.expect("details");
    assert_eq!(details.provider_name, "Fireworks AI");
    assert_eq!(details.model_name, "DeepSeek V4.1 Flash");
}

#[tokio::test(start_paused = true)]
async fn cln_25_quitting_saves_the_typed_key() {
    let h = Harness::with(|s| s.cleanup_enabled = true).await;
    h.app.page_appeared(MainWindowSection::PostProcessing);
    h.app.open_providers_dialog();
    sleep(10).await;
    h.app.api_key_edited("sk-or-ben-chang");
    let secrets = h.secrets.clone();
    h.app.shutdown_async().await;
    assert_eq!(secrets.stored().as_deref(), Some("sk-or-ben-chang"));
}

#[tokio::test(start_paused = true)]
async fn his_10_his_16_retention_on_page_appear_refreshes_the_menu() {
    let h = Harness::new().await;
    *lock(&h.history.stored) = vec![
        entry("Britta's in the study group", None, None, 1.hours()),
        entry("Expired at Greendale", None, None, (31 * 24).hours()),
    ];
    h.clear_events();
    let ops_before = lock(&h.history.ops).len();
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;
    assert_eq!(
        lock(&h.history.ops)[ops_before..],
        ["retention Month".to_owned()],
        "the page applies retention as it appears (HIS-16)"
    );
    assert_eq!(h.count(|e| matches!(e, Event::HistoryChanged)), 1);
    let recent = h.menu().recent_transcripts.expect("recent transcripts");
    let titles: Vec<&str> = recent.items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["Britta's in the study group"]);

    h.app.page_disappeared(MainWindowSection::History);
    h.clear_events();
    h.app.page_appeared(MainWindowSection::History);
    sleep(10).await;
    assert_eq!(
        h.count(|e| matches!(e, Event::HistoryChanged)),
        0,
        "nothing expired"
    );
}

#[tokio::test(start_paused = true)]
async fn his_10_retention_at_launch_announces_what_expired() {
    struct Case {
        name: &'static str,
        oldest: jiff::Span,
        want: usize,
    }
    let cases = [
        Case {
            name: "an entry expired",
            oldest: (31 * 24).hours(),
            want: 1,
        },
        Case {
            name: "nothing expired",
            oldest: 2.hours(),
            want: 0,
        },
    ];
    for tc in cases {
        let h = Harness::unstarted(|_| {});
        *lock(&h.history.stored) = vec![
            entry("Britta's in the study group", None, None, 1.hours()),
            entry("Pierce at Greendale", None, None, tc.oldest),
        ];
        h.app.ui_ready();
        sleep(10).await;
        assert_eq!(
            h.count(|e| matches!(e, Event::HistoryChanged)),
            tc.want,
            "{}",
            tc.name
        );
        assert!(h.menu().recent_transcripts.is_some(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn his_28_recent_items_carry_the_time_and_known_app() {
    let h = Harness::unstarted(|_| {});
    let slack = entry(
        "Troy and Abed in the morning",
        None,
        Some("Slack"),
        1.minutes(),
    );
    let unknown = entry("Cool. Cool cool cool.", None, None, 2.minutes());
    let want_times = [slack.timestamp, unknown.timestamp];
    *lock(&h.history.stored) = vec![slack, unknown];
    h.app.ui_ready();
    sleep(10).await;
    let recent = h.menu().recent_transcripts.expect("recent transcripts");
    let got: Vec<(Timestamp, Option<&str>)> = recent
        .items
        .iter()
        .map(|item| (item.time, item.app_name.as_deref()))
        .collect();
    assert_eq!(
        got,
        vec![(want_times[0], Some("Slack")), (want_times[1], None)],
        "the shell formats the date; the app is omitted when unknown"
    );
}
