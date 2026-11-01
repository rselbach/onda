//! The system speech engine end to end against a fake `SystemSpeech` port
//! (MOD-4, LANG-5, LANG-10, SYS-1..SYS-7, DL-10).

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use common::{Harness, lock, loud, sleep};
use onda_app::ports::{SystemSpeech, SystemSpeechError};
use onda_app::seams::Settings;
use onda_app::views::{AboutPage, ModelRowControl, SpeechPage};
use onda_app::{Event, MainWindowSection};
use onda_domain::languages::LanguageSetting;
use onda_domain::models::{ModelId, SystemSpeechInfo};
use onda_domain::non_dictation_ui::{DownloadOverlayView, OverlayProgress};
use onda_domain::tray::TrayIcon;

/// What the fake recognizer does, call by call.
#[derive(Default)]
struct FakeSpeech {
    calls: Mutex<Vec<String>>,
    os_locale: Mutex<String>,
    /// Reported by the next prepare, as an asset download.
    download: Mutex<Vec<f64>>,
    /// Results of upcoming prepares, then `Ok`.
    prepares: Mutex<VecDeque<Result<(), SystemSpeechError>>>,
    /// Results of upcoming transcriptions, then `Ok(["Cool."])`.
    transcriptions: Mutex<VecDeque<Result<Vec<String>, SystemSpeechError>>>,
}

impl FakeSpeech {
    fn new() -> Arc<Self> {
        let speech = Self::default();
        *lock(&speech.os_locale) = "en_US".to_owned();
        Arc::new(speech)
    }

    fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }
}

impl SystemSpeech for FakeSpeech {
    fn info(&self) -> SystemSpeechInfo {
        SystemSpeechInfo {
            description: "Built-in macOS speech recognition — no download required".to_owned(),
            languages: ["ar", "en", "es", "ko", "tr", "zh"]
                .map(String::from)
                .to_vec(),
            license_name: "Apple System".to_owned(),
            license_url: "https://www.apple.com/legal/sla/".to_owned(),
            attribution: "Apple Speech Framework".to_owned(),
        }
    }

    fn os_locale(&self) -> String {
        lock(&self.os_locale).clone()
    }

    fn prepare(
        &self,
        locale: &str,
        progress: &mut dyn FnMut(f64),
    ) -> Result<(), SystemSpeechError> {
        lock(&self.calls).push(format!("prepare {locale}"));
        let download = std::mem::take(&mut *lock(&self.download));
        for fraction in download {
            progress(fraction);
        }
        lock(&self.prepares).pop_front().unwrap_or(Ok(()))
    }

    fn transcribe(&self, _: &[f32], locale: &str) -> Result<Vec<String>, SystemSpeechError> {
        lock(&self.calls).push(format!("transcribe {locale}"));
        lock(&self.transcriptions)
            .pop_front()
            .unwrap_or_else(|| Ok(vec!["Cool.".to_owned()]))
    }
}

fn language(code: &str) -> LanguageSetting {
    LanguageSetting::parse(code).expect("offered language")
}

fn system_selected(code: &'static str) -> impl FnOnce(&mut Settings) {
    move |s| {
        s.selected_model = ModelId::SystemSpeech;
        s.stored_model = ModelId::SystemSpeech.as_str().to_owned();
        s.language = language(code);
    }
}

fn speech_page(h: &Harness) -> SpeechPage {
    h.last(|e| match e {
        Event::SpeechChanged(page) => Some(*page),
        _ => None,
    })
    .expect("a Speech page")
}

fn about_page(h: &Harness) -> AboutPage {
    h.last(|e| match e {
        Event::AboutChanged(page) => Some(*page),
        _ => None,
    })
    .expect("an About page")
}

fn bodies(h: &Harness) -> Vec<String> {
    lock(&h.notifier.bodies).clone()
}

fn overlay_statuses(h: &Harness) -> Vec<(String, OverlayProgress)> {
    h.events()
        .into_iter()
        .filter_map(|e| match e {
            Event::DownloadOverlayChanged(DownloadOverlayView::Shown { status, progress }) => {
                Some((status, progress))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn catalog_has_system_speech_only_with_the_port_mod_4() {
    struct Case {
        name: &'static str,
        speech: Option<Arc<FakeSpeech>>,
        want_models: Vec<&'static str>,
        want_attribution: Option<&'static str>,
        want_recommended_for_auto: &'static str,
    }
    let cases = [
        Case {
            name: "no port",
            speech: None,
            want_models: vec![
                "parakeet-tdt-v2",
                "parakeet-ultra",
                "parakeet-ja",
                "parakeet-unified",
            ],
            want_attribution: None,
            want_recommended_for_auto: "parakeet-ultra",
        },
        Case {
            name: "port",
            speech: Some(FakeSpeech::new()),
            want_models: vec![
                "parakeet-tdt-v2",
                "parakeet-ultra",
                "parakeet-ja",
                "parakeet-unified",
                "system-speech",
            ],
            want_attribution: Some("Apple Speech Framework"),
            want_recommended_for_auto: "system-speech",
        },
    ];
    for tc in cases {
        let speech = tc.speech.map(|s| s as Arc<dyn SystemSpeech>);
        let h = Harness::build(speech, |_| {});
        h.app.ui_ready();
        sleep(1).await;
        h.app.page_appeared(MainWindowSection::Speech);
        h.app.page_appeared(MainWindowSection::About);
        sleep(10).await;

        let page = speech_page(&h);
        let ids: Vec<&str> = page.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, tc.want_models, "{}", tc.name);
        // LANG-5: Automatic recommends system speech when there is one
        let recommended: Vec<&str> = page
            .models
            .iter()
            .filter(|m| m.recommended.is_some())
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            recommended,
            vec![tc.want_recommended_for_auto],
            "{}",
            tc.name
        );
        let about = about_page(&h);
        let attribution = about
            .models
            .iter()
            .find(|m| m.name == "System Speech")
            .map(|m| m.attribution.as_str());
        assert_eq!(attribution, tc.want_attribution, "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn system_speech_row_is_built_in_selectable_and_undeletable_dl_9_dl_10() {
    let h = Harness::with_system_speech(FakeSpeech::new(), |_| {}).await;
    h.app.page_appeared(MainWindowSection::Speech);
    sleep(10).await;
    let page = speech_page(&h);
    let row = page
        .models
        .iter()
        .find(|m| m.id == "system-speech")
        .expect("a System Speech row");
    assert_eq!(row.name, "System Speech");
    assert_eq!(row.size_hint, "Built-in");
    assert_eq!(
        row.description,
        "Built-in macOS speech recognition — no download required"
    );
    assert!(
        matches!(row.control, ModelRowControl::Select { .. }),
        "built-in counts as downloaded: {:?}",
        row.control
    );
    assert_eq!(row.delete_tooltip, None, "no delete control");
}

#[tokio::test(start_paused = true)]
async fn load_prepares_the_selected_language_with_download_progress_sys_2_sys_4() {
    struct Case {
        name: &'static str,
        language: &'static str,
        want_locale: &'static str,
    }
    let cases = [
        Case {
            name: "korean",
            language: "ko",
            want_locale: "ko-KR",
        },
        Case {
            name: "chinese",
            language: "zh",
            want_locale: "zh-CN",
        },
        Case {
            name: "automatic uses the OS locale",
            language: "auto",
            want_locale: "pt_BR",
        },
    ];
    for tc in cases {
        let speech = FakeSpeech::new();
        *lock(&speech.os_locale) = "pt_BR".to_owned();
        *lock(&speech.download) = vec![0.25, 1.0];
        let h = Harness::with_system_speech(speech.clone(), system_selected(tc.language)).await;
        assert_eq!(
            speech.calls(),
            vec![format!("prepare {}", tc.want_locale)],
            "{}",
            tc.name
        );
        assert_eq!(h.tray().icon, TrayIcon::Ready, "{}", tc.name);
        let downloading: Vec<(String, OverlayProgress)> = overlay_statuses(&h)
            .into_iter()
            .filter(|(status, _)| status == "Downloading speech model...")
            .collect();
        assert_eq!(
            downloading,
            vec![
                (
                    "Downloading speech model...".to_owned(),
                    OverlayProgress::Determinate(0.25)
                ),
                (
                    "Downloading speech model...".to_owned(),
                    OverlayProgress::Determinate(1.0)
                ),
            ],
            "{}",
            tc.name
        );
        assert!(h.notifications().is_empty(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn dictation_joins_final_results_without_a_preview_sys_7() {
    let speech = FakeSpeech::new();
    lock(&speech.transcriptions).push_back(Ok(vec![
        "안녕하세요.".to_owned(),
        String::new(),
        " 반갑습니다. ".to_owned(),
    ]));
    let h = Harness::with_system_speech(speech.clone(), system_selected("ko")).await;
    h.audio.starts_with(loud(24_000));
    h.dictate().await;
    assert_eq!(h.typed(), vec!["안녕하세요. 반갑습니다."]);
    assert!(h.previews().is_empty(), "no live preview");
    assert_eq!(speech.calls(), vec!["prepare ko-KR", "transcribe ko-KR"]);
    assert_eq!(
        h.history.entries()[0].model_id,
        "system-speech",
        "HIS-3 records the engine"
    );
}

#[tokio::test(start_paused = true)]
async fn a_clip_under_parakeets_minimum_is_still_transcribed_gate_2() {
    let speech = FakeSpeech::new();
    let h = Harness::with_system_speech(speech.clone(), system_selected("en")).await;
    // past GATE-2's 0.25 s, under Parakeet's 0.3 s
    h.audio.starts_with(loud(4_500));
    h.app.toggle_dictation_from_menu();
    sleep(10).await;
    h.app.toggle_dictation_from_menu();
    sleep(500).await;
    assert_eq!(h.typed(), vec!["Cool."]);
    assert_eq!(speech.calls(), vec!["prepare en-US", "transcribe en-US"]);
}

#[tokio::test(start_paused = true)]
async fn language_change_without_model_change_reprepares_on_the_next_decode_sys_5() {
    let speech = FakeSpeech::new();
    let h = Harness::with_system_speech(speech.clone(), system_selected("ko")).await;
    h.app.select_language(language("tr"));
    sleep(10).await;
    h.dictate().await;
    assert_eq!(
        speech.calls(),
        vec!["prepare ko-KR", "prepare tr-TR", "transcribe tr-TR"],
        "re-prepared inside the decode"
    );

    // SYS-2: Automatic keeps what is prepared instead of the OS locale
    h.app.select_language(LanguageSetting::Auto);
    sleep(10).await;
    h.dictate().await;
    assert_eq!(
        speech.calls(),
        vec![
            "prepare ko-KR",
            "prepare tr-TR",
            "transcribe tr-TR",
            "transcribe tr-TR"
        ]
    );
    assert_eq!(h.typed(), vec!["Cool.", "Cool."]);
}

#[tokio::test(start_paused = true)]
async fn load_errors_sys_6_asr_6_asr_7() {
    struct Case {
        name: &'static str,
        error: SystemSpeechError,
        want_bodies: Vec<&'static str>,
        want_retry_item: bool,
    }
    let cases = [
        Case {
            name: "unsupported locale names it",
            error: SystemSpeechError::UnsupportedLocale {
                display_name: "Korean (South Korea)".to_owned(),
            },
            want_bodies: vec![
                "System Speech can't transcribe Korean (South Korea) on this Mac Check your internet connection, then choose Retry Loading Model from the menu bar icon.",
            ],
            want_retry_item: true,
        },
        Case {
            name: "other failures use the generic copy",
            error: SystemSpeechError::Failed {
                detail: "asset server unreachable".to_owned(),
            },
            want_bodies: vec![
                "Failed to load the transcription model Check your internet connection, then choose Retry Loading Model from the menu bar icon.",
            ],
            want_retry_item: true,
        },
        Case {
            name: "cancelled is not an error",
            error: SystemSpeechError::Cancelled,
            want_bodies: vec![],
            want_retry_item: false,
        },
    ];
    for tc in cases {
        let speech = FakeSpeech::new();
        lock(&speech.prepares).push_back(Err(tc.error));
        let h = Harness::with_system_speech(speech.clone(), system_selected("ko")).await;
        sleep(10).await;
        assert_eq!(bodies(&h), tc.want_bodies, "{}", tc.name);
        assert_eq!(
            h.menu().retry_model_load.is_some(),
            tc.want_retry_item,
            "{}",
            tc.name
        );
        assert!(
            h.menu().redownload_model.is_none(),
            "{}: built-in engines cannot be re-downloaded (DL-11)",
            tc.name
        );
    }
}

#[tokio::test(start_paused = true)]
async fn decode_errors_sys_6_sys_7() {
    struct Case {
        name: &'static str,
        prepare: Option<SystemSpeechError>,
        transcription: Option<SystemSpeechError>,
        want_bodies: Vec<&'static str>,
        want_retry_kept: bool,
    }
    let failed_body = "Onda kept the recording. Choose Retry Failed Dictation from the menu bar icon to try again. Error: Transcription failed";
    let cases = [
        Case {
            name: "a recognizer failure is Transcription failed",
            prepare: None,
            transcription: Some(SystemSpeechError::Failed {
                detail: "analyzer stopped".to_owned(),
            }),
            want_bodies: vec![failed_body],
            want_retry_kept: true,
        },
        Case {
            name: "an unsupported locale while transcribing is Transcription failed",
            prepare: None,
            transcription: Some(SystemSpeechError::UnsupportedLocale {
                display_name: "Turkish (Türkiye)".to_owned(),
            }),
            want_bodies: vec![failed_body],
            want_retry_kept: true,
        },
        Case {
            name: "re-preparing an unsupported locale names it",
            prepare: Some(SystemSpeechError::UnsupportedLocale {
                display_name: "Turkish (Türkiye)".to_owned(),
            }),
            transcription: None,
            want_bodies: vec![
                "Onda kept the recording. Choose Retry Failed Dictation from the menu bar icon to try again. Error: System Speech can't transcribe Turkish (Türkiye) on this Mac",
            ],
            want_retry_kept: true,
        },
        Case {
            name: "cancellation is silent",
            prepare: None,
            transcription: Some(SystemSpeechError::Cancelled),
            want_bodies: vec![],
            want_retry_kept: false,
        },
    ];
    for tc in cases {
        let speech = FakeSpeech::new();
        let h = Harness::with_system_speech(speech.clone(), system_selected("ko")).await;
        if let Some(error) = tc.prepare {
            // the load's prepare already ran; this one is the re-prepare
            lock(&speech.prepares).push_back(Err(error));
            h.app.select_language(language("tr"));
            sleep(10).await;
        }
        if let Some(error) = tc.transcription {
            lock(&speech.transcriptions).push_back(Err(error));
        }
        h.dictate().await;
        assert!(h.typed().is_empty(), "{}", tc.name);
        assert_eq!(bodies(&h), tc.want_bodies, "{}", tc.name);
        assert_eq!(
            h.menu().retry_failed_dictation.is_some(),
            tc.want_retry_kept,
            "{}",
            tc.name
        );
        assert!(h.idle(), "{}", tc.name);
    }
}

#[tokio::test(start_paused = true)]
async fn menu_switch_to_korean_selects_system_speech_lang_10() {
    let speech = FakeSpeech::new();
    let h = Harness::with_system_speech(speech.clone(), |_| {}).await;
    assert!(speech.calls().is_empty(), "Parakeet v2 is loaded");

    h.app.select_language_from_menu(language("ko"));
    sleep(10).await;
    assert_eq!(
        h.log.with("settings:selected"),
        vec!["settings:selected system-speech"]
    );
    assert_eq!(speech.calls(), vec!["prepare ko-KR"]);
    assert_eq!(h.tray().icon, TrayIcon::Ready);
    h.dictate().await;
    assert_eq!(h.typed(), vec!["Cool."]);
}

#[tokio::test(start_paused = true)]
async fn menu_switch_to_korean_without_the_port_opens_speech_settings_lang_10() {
    let h = Harness::new().await;
    h.app.select_language_from_menu(language("ko"));
    sleep(10).await;
    assert!(h.log.with("settings:selected").is_empty());
    let opened: Vec<MainWindowSection> = h
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::OpenMainWindow(section) => Some(section),
            _ => None,
        })
        .collect();
    assert_eq!(opened, vec![MainWindowSection::Speech]);
}
