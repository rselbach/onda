//! Settings store behavior against real files in temp dirs (SET-1..SET-4).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use onda_domain::cleanup::{CLEANUP_MODELS, CleanupProvider, default_model};
use onda_domain::hotkey::{KeyCode, Modifier, ModifierKey, ModifierSet, Shortcut};
use onda_domain::languages::LanguageSetting;
use onda_domain::messages::UserMessage;
use onda_domain::models::{Catalog, ModelId, SystemSpeechInfo};
use onda_domain::replacements::{ReplacementRule, RuleId};
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use onda_settings::{Key, Settings};
use serde_json::{Value, json};
use tempfile::TempDir;

const OS_LANGUAGE: Option<&str> = Some("es");

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    selected_model: String,
    selected_language: LanguageSetting,
    recent_languages: String,
    input_device_uid: String,
    output_mode: OutputMode,
    hotkey: Shortcut,
    activation_mode: ActivationMode,
    pause_media_during_recording: bool,
    sound_feedback_enabled: bool,
    history_enabled: bool,
    history_keeps_audio: bool,
    history_retention: HistoryRetention,
    replacement_rules: Vec<ReplacementRule>,
    post_processing_enabled: bool,
    cleanup_provider: CleanupProvider,
    openrouter_model: String,
    fireworks_model: String,
    onboarding_completed: bool,
    did_show_first_run_setup: bool,
    last_model_migration_notice_key: String,
    declined_model_migration_notice_key: String,
}

fn snapshot(settings: &Settings) -> Snapshot {
    Snapshot {
        selected_model: settings.selected_model(&Catalog::new(None), OS_LANGUAGE),
        selected_language: settings.selected_language(OS_LANGUAGE),
        recent_languages: settings.recent_languages(),
        input_device_uid: settings.input_device_uid(),
        output_mode: settings.output_mode(),
        hotkey: settings.hotkey(),
        activation_mode: settings.activation_mode(),
        pause_media_during_recording: settings.pause_media_during_recording(),
        sound_feedback_enabled: settings.sound_feedback_enabled(),
        history_enabled: settings.history_enabled(),
        history_keeps_audio: settings.history_keeps_audio(),
        history_retention: settings.history_retention(),
        replacement_rules: settings.replacement_rules(),
        post_processing_enabled: settings.post_processing_enabled(),
        cleanup_provider: settings.cleanup_provider(),
        openrouter_model: settings
            .cleanup_model(CleanupProvider::OpenRouter)
            .slug
            .into_owned(),
        fireworks_model: settings
            .cleanup_model(CleanupProvider::Fireworks)
            .slug
            .into_owned(),
        onboarding_completed: settings.onboarding_completed(),
        did_show_first_run_setup: settings.did_show_first_run_setup(),
        last_model_migration_notice_key: settings.last_model_migration_notice_key(),
        declined_model_migration_notice_key: settings.declined_model_migration_notice_key(),
    }
}

fn defaults() -> Snapshot {
    Snapshot {
        selected_model: "parakeet-ultra".to_owned(),
        selected_language: lang("es"),
        recent_languages: String::new(),
        input_device_uid: String::new(),
        output_mode: OutputMode::Type,
        hotkey: Shortcut::default(),
        activation_mode: ActivationMode::Hold,
        pause_media_during_recording: false,
        sound_feedback_enabled: true,
        history_enabled: true,
        history_keeps_audio: false,
        history_retention: HistoryRetention::Month,
        replacement_rules: Vec::new(),
        post_processing_enabled: false,
        cleanup_provider: CleanupProvider::OpenRouter,
        openrouter_model: "~openai/gpt-luna-latest".to_owned(),
        fireworks_model: "accounts/fireworks/models/deepseek-v4p1-flash".to_owned(),
        onboarding_completed: false,
        did_show_first_run_setup: false,
        last_model_migration_notice_key: String::new(),
        declined_model_migration_notice_key: String::new(),
    }
}

fn lang(code: &str) -> LanguageSetting {
    LanguageSetting::parse(code).expect("offered language")
}

fn settings_path(dir: &TempDir) -> PathBuf {
    dir.path().join("Onda").join("settings.json")
}

fn with_file(contents: &[u8]) -> (TempDir, Settings) {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(&path, contents).expect("write settings");
    let settings = Settings::open(&path);
    (dir, settings)
}

fn with_json(value: &Value) -> (TempDir, Settings) {
    with_file(value.to_string().as_bytes())
}

fn stored(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read settings")).expect("settings are JSON")
}

fn recorder(settings: Settings) -> (Settings, Arc<Mutex<Vec<Key>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let settings = settings.on_change(move |key| sink.lock().expect("lock").push(key));
    (settings, seen)
}

fn system_catalog(languages: &[&str]) -> Catalog {
    Catalog::new(Some(SystemSpeechInfo {
        description: "Built-in speech recognition".to_owned(),
        languages: languages.iter().map(|code| (*code).to_owned()).collect(),
        license_name: "Greendale System".to_owned(),
        license_url: "https://greendale.example/legal".to_owned(),
        attribution: "Greendale Speech".to_owned(),
    }))
}

fn rule(id: &str, triggers: &[&str], replacement: &str) -> ReplacementRule {
    ReplacementRule {
        id: RuleId(id.to_owned()),
        triggers: triggers.iter().map(|t| (*t).to_owned()).collect(),
        replacement: replacement.to_owned(),
    }
}

#[test]
fn every_key_has_its_default_and_nothing_is_written_set_2a() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let settings = Settings::open(&path);

    assert_eq!(snapshot(&settings), defaults());
    assert!(
        settings.first_run_pending(),
        "first run is pending by default"
    );
    for key in Key::ALL {
        assert!(!settings.is_set(key), "{key} should be unset");
    }
    assert!(!path.exists(), "reading defaults must not create the file");
}

#[test]
fn key_names_match_prd_09() {
    let got: Vec<&str> = Key::ALL.iter().map(|key| key.as_str()).collect();
    let want = [
        "selectedModel",
        "selectedLanguage",
        "recentLanguages",
        "inputDeviceUID",
        "outputMode",
        "hotkey",
        "hotkeyActivationMode",
        "pauseMediaDuringRecording",
        "soundFeedbackEnabled",
        "historyEnabled",
        "historyKeepsAudio",
        "historyRetention",
        "replacementEntries",
        "postProcessingEnabled",
        "cleanupProvider",
        "openRouterModel",
        "fireworksModel",
        "onboardingCompleted",
        "didShowFirstRunSetup",
        "lastModelMigrationNoticeKey",
        "declinedModelMigrationNoticeKey",
    ];
    assert_eq!(got, want);
}

#[test]
fn every_setter_round_trips_through_the_file() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let settings = Settings::open(&path);
    let hotkey = Shortcut::new(
        KeyCode::parse("KeyA").expect("key"),
        ModifierSet::EMPTY
            .with(Modifier::Control)
            .with(Modifier::Alt),
    )
    .expect("valid shortcut");
    let rules = vec![rule(
        "a1b2",
        &["greendale", "greendale cc"],
        "Greendale Community College",
    )];

    settings
        .set_selected_model(ModelId::ParakeetJapanese)
        .expect("set");
    settings.set_selected_language(lang("ja")).expect("set");
    settings
        .set_recent_languages(&[lang("ja"), LanguageSetting::Auto, lang("fr")])
        .expect("set");
    settings
        .set_input_device_uid("GreendalePodcastMicrophone")
        .expect("set");
    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("set");
    settings.set_hotkey(&hotkey).expect("set");
    settings
        .set_activation_mode(ActivationMode::Automatic)
        .expect("set");
    settings
        .set_pause_media_during_recording(true)
        .expect("set");
    settings.set_sound_feedback_enabled(false).expect("set");
    settings.set_history_enabled(false).expect("set");
    settings.set_history_keeps_audio(true).expect("set");
    settings
        .set_history_retention(HistoryRetention::Forever)
        .expect("set");
    settings.set_replacement_rules(&rules).expect("set");
    settings.set_post_processing_enabled(true).expect("set");
    settings
        .set_cleanup_provider(CleanupProvider::Fireworks)
        .expect("set");
    settings
        .set_cleanup_model(CleanupProvider::OpenRouter, &CLEANUP_MODELS[1])
        .expect("set");
    settings
        .set_cleanup_model(
            CleanupProvider::Fireworks,
            &CleanupProvider::Fireworks.resolve_model(Some("accounts/fireworks/models/kimi-k3")),
        )
        .expect("set");
    settings.complete_onboarding().expect("set");
    settings
        .set_last_model_migration_notice_key("parakeet-old->parakeet-ultra")
        .expect("set");
    settings
        .set_declined_model_migration_notice_key("parakeet-old->parakeet-ultra")
        .expect("set");

    let want = Snapshot {
        selected_model: "parakeet-ja".to_owned(),
        selected_language: lang("ja"),
        recent_languages: "ja,auto,fr".to_owned(),
        input_device_uid: "GreendalePodcastMicrophone".to_owned(),
        output_mode: OutputMode::Clipboard,
        hotkey: hotkey.clone(),
        activation_mode: ActivationMode::Automatic,
        pause_media_during_recording: true,
        sound_feedback_enabled: false,
        history_enabled: false,
        history_keeps_audio: true,
        history_retention: HistoryRetention::Forever,
        replacement_rules: rules,
        post_processing_enabled: true,
        cleanup_provider: CleanupProvider::Fireworks,
        openrouter_model: "~anthropic/claude-haiku-latest".to_owned(),
        fireworks_model: "accounts/fireworks/models/kimi-k3".to_owned(),
        onboarding_completed: true,
        did_show_first_run_setup: true,
        last_model_migration_notice_key: "parakeet-old->parakeet-ultra".to_owned(),
        declined_model_migration_notice_key: "parakeet-old->parakeet-ultra".to_owned(),
    };
    assert_eq!(snapshot(&settings), want, "cached values");
    assert_eq!(
        snapshot(&Settings::open(&path)),
        want,
        "values reread from disk"
    );

    let raw = stored(&path);
    let got_keys: BTreeSet<&str> = raw
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    let want_keys: BTreeSet<&str> = Key::ALL.iter().map(|key| key.as_str()).collect();
    assert_eq!(got_keys, want_keys, "one JSON key per setting");
    assert_eq!(raw["outputMode"], "clipboard");
    assert_eq!(raw["hotkeyActivationMode"], "automatic");
    assert_eq!(raw["historyRetention"], "forever");
    assert_eq!(
        raw["hotkey"],
        json!({"key": "KeyA", "modifiers": ["control", "alt"]})
    );
    assert_eq!(
        raw["replacementEntries"],
        json!([{"id": "a1b2", "triggers": ["greendale", "greendale cc"], "replacement": "Greendale Community College"}])
    );
}

#[test]
fn lone_modifier_hotkey_round_trips_hot_1() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let settings = Settings::open(&path);
    let shortcut = Shortcut::lone(ModifierKey::AltRight);

    settings.set_hotkey(&shortcut).expect("set");

    assert_eq!(Settings::open(&path).hotkey(), shortcut);
    assert_eq!(
        stored(&path)["hotkey"],
        json!({"key": "AltRight", "modifiers": []})
    );
}

#[test]
fn remove_resets_to_the_default() {
    let (_dir, settings) = with_json(
        &json!({"inputDeviceUID": "GreendalePodcastMicrophone", "soundFeedbackEnabled": false}),
    );
    assert!(settings.is_set(Key::InputDeviceUid));

    settings.remove(Key::InputDeviceUid).expect("remove");
    settings.remove(Key::SoundFeedbackEnabled).expect("remove");

    assert_eq!(settings.input_device_uid(), "");
    assert!(settings.sound_feedback_enabled());
    assert!(!settings.is_set(Key::InputDeviceUid));
    assert!(!settings.is_set(Key::SoundFeedbackEnabled));
}

#[test]
fn corrupt_values_read_as_defaults_set_2() {
    let mut cases: Vec<(String, Value)> = vec![
        ("selectedModel".into(), json!(42)),
        ("selectedLanguage".into(), json!(7)),
        ("selectedLanguage".into(), json!(null)),
        ("selectedLanguage".into(), json!(["en"])),
        ("recentLanguages".into(), json!(5)),
        ("inputDeviceUID".into(), json!(true)),
        ("outputMode".into(), json!("paste")),
        ("outputMode".into(), json!("directTyping")),
        ("outputMode".into(), json!("TYPE")),
        ("outputMode".into(), json!(1)),
        ("hotkey".into(), json!({"key": "Space", "modifiers": []})),
        (
            "hotkey".into(),
            json!({"key": "KeyA", "modifiers": ["shift"]}),
        ),
        (
            "hotkey".into(),
            json!({"key": "Escape", "modifiers": ["alt"]}),
        ),
        (
            "hotkey".into(),
            json!({"key": 5_000_000_000u64, "modifiers": []}),
        ),
        (
            "hotkey".into(),
            json!({"key": "Space", "modifiers": ["hyper"]}),
        ),
        ("hotkey".into(), json!({"key": "", "modifiers": ["alt"]})),
        ("hotkey".into(), json!({"key": "Space"})),
        ("hotkey".into(), json!("⌥ Space")),
        ("hotkey".into(), json!(null)),
        ("hotkeyActivationMode".into(), json!("changnesia")),
        ("hotkeyActivationMode".into(), json!(0)),
        ("historyRetention".into(), json!("fortnight")),
        ("historyRetention".into(), json!(30)),
        ("replacementEntries".into(), json!("{broken")),
        ("replacementEntries".into(), json!({"id": "x"})),
        ("replacementEntries".into(), json!([{"id": "a"}])),
        ("cleanupProvider".into(), json!("greendale")),
        ("cleanupProvider".into(), json!(1)),
        ("openRouterModel".into(), json!("")),
        ("openRouterModel".into(), json!(3)),
        ("lastModelMigrationNoticeKey".into(), json!(9)),
        ("declinedModelMigrationNoticeKey".into(), json!({})),
    ];
    let bool_keys = [
        Key::PauseMediaDuringRecording,
        Key::SoundFeedbackEnabled,
        Key::HistoryEnabled,
        Key::HistoryKeepsAudio,
        Key::PostProcessingEnabled,
        Key::OnboardingCompleted,
        Key::DidShowFirstRunSetup,
    ];
    for key in bool_keys {
        for bad in [json!("yes"), json!(1), json!(null), json!([true])] {
            cases.push((key.as_str().into(), bad));
        }
    }

    for (key, bad) in cases {
        let file = json!({ key.clone(): bad.clone() });
        let (_dir, settings) = with_json(&file);
        assert_eq!(snapshot(&settings), defaults(), "{key} = {bad}");
        assert!(settings.first_run_pending(), "{key} = {bad}");
    }
}

#[test]
fn getters_never_write_back_corrupt_values() {
    let file = json!({"outputMode": "paste", "hotkey": {"key": "Escape", "modifiers": []}, "replacementEntries": "{broken"});
    let (dir, settings) = with_json(&file);
    let path = settings_path(&dir);
    let before = fs::read(&path).expect("read");

    let _ = snapshot(&settings);

    assert_eq!(fs::read(&path).expect("read"), before);
}

#[test]
fn whole_file_corruption_reads_as_defaults() {
    let cases: [(&str, &[u8]); 8] = [
        ("garbage", b"Troy and Abed in the morning"),
        ("truncated", b"{\"outputMode\": \"clip"),
        ("empty", b""),
        ("array", b"[1, 2, 3]"),
        ("string", b"\"settings\""),
        ("null", b"null"),
        ("number", b"42"),
        ("invalid utf-8", &[0xff, 0xfe, 0x7b, 0x80]),
    ];
    for (name, contents) in cases {
        let (dir, settings) = with_file(contents);
        assert_eq!(snapshot(&settings), defaults(), "{name}");

        settings.set_output_mode(OutputMode::Clipboard).expect(name);

        assert_eq!(
            stored(&settings_path(&dir)),
            json!({"outputMode": "clipboard"}),
            "{name}"
        );
    }
}

#[test]
fn unreadable_path_reads_as_defaults() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    fs::create_dir_all(&path).expect("a directory where the file should be");

    let settings = Settings::open(&path);

    assert_eq!(snapshot(&settings), defaults());
    assert!(settings.set_output_mode(OutputMode::Clipboard).is_err());
}

#[test]
fn unknown_keys_survive_writes() {
    let file = json!({
        "futureSetting": {"nested": [1, 2, 3]},
        "hotkeyKeyCode": 49,
        "outputMode": "clipboard",
    });
    let (dir, settings) = with_json(&file);

    settings.set_history_enabled(false).expect("set");
    settings.remove(Key::OutputMode).expect("remove");

    let want = json!({
        "futureSetting": {"nested": [1, 2, 3]},
        "hotkeyKeyCode": 49,
        "historyEnabled": false,
    });
    assert_eq!(stored(&settings_path(&dir)), want);
}

#[test]
fn writes_are_atomic_set_1() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let tmp = path.with_file_name("settings.json.tmp");
    let (settings, seen) = recorder(Settings::open(&path));

    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("first write creates the directory");
    assert!(!tmp.exists(), "temp file is renamed into place");
    assert_eq!(stored(&path), json!({"outputMode": "clipboard"}));

    // a directory squatting on the temp path makes the next write fail
    fs::create_dir(&tmp).expect("block the temp path");
    let before = fs::read(&path).expect("read");

    let result = settings.set_history_enabled(false);

    assert!(result.is_err(), "write must fail");
    assert_eq!(fs::read(&path).expect("read"), before, "file untouched");
    assert!(settings.history_enabled(), "cache untouched");
    assert!(!settings.is_set(Key::HistoryEnabled));
    assert_eq!(
        *seen.lock().expect("lock"),
        vec![Key::OutputMode],
        "failed write is not reported"
    );

    fs::remove_dir(&tmp).expect("unblock");
    settings.set_history_enabled(false).expect("write recovers");
    assert_eq!(
        stored(&path),
        json!({"outputMode": "clipboard", "historyEnabled": false})
    );
}

#[test]
fn canonicalize_rewrites_stale_values_once_set_3() {
    let file = json!({
        "outputMode": "paste",
        "hotkeyActivationMode": "changnesia",
        "historyRetention": 7,
        "openRouterModel": "~google/gemini-flash-latest",
        "inputDeviceUID": "GreendalePodcastMicrophone",
    });
    let (dir, settings) = with_json(&file);
    let path = settings_path(&dir);
    let (settings, seen) = recorder(settings);

    let changed = settings.canonicalize().expect("first pass");

    let want_changed = vec![
        Key::OutputMode,
        Key::HotkeyActivationMode,
        Key::HistoryRetention,
        Key::OpenRouterModel,
    ];
    assert_eq!(changed, want_changed);
    assert_eq!(
        *seen.lock().expect("lock"),
        want_changed,
        "rewrites are reported"
    );
    let want = json!({
        "outputMode": "type",
        "hotkeyActivationMode": "hold",
        "historyRetention": "month",
        "openRouterModel": "google/gemini-3.1-flash-lite",
        "inputDeviceUID": "GreendalePodcastMicrophone",
    });
    assert_eq!(stored(&path), want);

    let before = fs::read(&path).expect("read");
    assert_eq!(
        settings.canonicalize().expect("second pass"),
        Vec::<Key>::new(),
        "idempotent"
    );
    assert_eq!(
        fs::read(&path).expect("read"),
        before,
        "second pass writes nothing"
    );
    assert_eq!(seen.lock().expect("lock").len(), want_changed.len());
}

#[test]
fn canonicalize_leaves_unset_and_canonical_values_alone() {
    let cases = [
        ("empty file", json!({})),
        (
            "canonical",
            json!({"outputMode": "clipboard", "hotkeyActivationMode": "toggle", "historyRetention": "week", "openRouterModel": default_model().slug.as_ref()}),
        ),
    ];
    for (name, file) in cases {
        let (dir, settings) = with_json(&file);
        let path = settings_path(&dir);

        assert_eq!(
            settings.canonicalize().expect(name),
            Vec::<Key>::new(),
            "{name}"
        );
        assert_eq!(stored(&path), file, "{name}");
    }
}

#[test]
fn computed_defaults_follow_the_os_locale_until_set_lang_2() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let settings = Settings::open(&path);
    let no_system = Catalog::new(None);
    let system = system_catalog(&["en", "fr", "ko"]);

    struct Case {
        os: Option<&'static str>,
        catalog: &'static str,
        want_language: &'static str,
        want_model: &'static str,
    }
    let cases = [
        Case {
            os: Some("fr"),
            catalog: "none",
            want_language: "fr",
            want_model: "parakeet-ultra",
        },
        Case {
            os: Some("en"),
            catalog: "none",
            want_language: "en",
            want_model: "parakeet-tdt-v2",
        },
        Case {
            os: Some("ja"),
            catalog: "none",
            want_language: "ja",
            want_model: "parakeet-ja",
        },
        Case {
            os: Some("xx"),
            catalog: "none",
            want_language: "auto",
            want_model: "parakeet-ultra",
        },
        Case {
            os: None,
            catalog: "none",
            want_language: "auto",
            want_model: "parakeet-ultra",
        },
        Case {
            os: None,
            catalog: "system",
            want_language: "auto",
            want_model: "system-speech",
        },
        Case {
            os: Some("ko"),
            catalog: "system",
            want_language: "ko",
            want_model: "system-speech",
        },
        Case {
            os: Some("ko"),
            catalog: "none",
            want_language: "ko",
            want_model: "parakeet-ultra",
        },
    ];
    for tc in &cases {
        let catalog = if tc.catalog == "system" {
            &system
        } else {
            &no_system
        };
        let name = format!("os {:?}, catalog {}", tc.os, tc.catalog);
        assert_eq!(
            settings.selected_language(tc.os).code(),
            tc.want_language,
            "{name}"
        );
        assert_eq!(
            settings.selected_model(catalog, tc.os),
            tc.want_model,
            "{name}"
        );
    }
    assert!(!path.exists(), "computed defaults are never written");

    settings.set_selected_language(lang("de")).expect("set");
    settings
        .set_selected_model(ModelId::ParakeetTdtV2)
        .expect("set");
    for tc in &cases {
        let catalog = if tc.catalog == "system" {
            &system
        } else {
            &no_system
        };
        assert_eq!(
            settings.selected_language(tc.os),
            lang("de"),
            "picked language sticks"
        );
        assert_eq!(
            settings.selected_model(catalog, tc.os),
            "parakeet-tdt-v2",
            "picked model sticks"
        );
    }
}

#[test]
fn unknown_stored_model_is_returned_for_migration_dl_12() {
    let (_dir, settings) = with_json(&json!({"selectedModel": "whisper-large"}));

    assert_eq!(
        settings.selected_model(&Catalog::new(None), OS_LANGUAGE),
        "whisper-large"
    );
}

#[test]
fn invalid_language_resets_to_auto_with_a_warning_lang_3() {
    let (dir, settings) = with_json(&json!({"selectedLanguage": "xx"}));
    let path = settings_path(&dir);
    let (settings, seen) = recorder(settings);

    assert_eq!(
        settings.selected_language(OS_LANGUAGE),
        LanguageSetting::Auto,
        "reads as auto before the reset"
    );
    assert_eq!(
        stored(&path)["selectedLanguage"],
        "xx",
        "reading does not reset"
    );

    let warning = settings.reset_invalid_language().expect("reset");

    assert_eq!(
        warning,
        Some(UserMessage::InvalidLanguageSetting {
            code: "xx".to_owned()
        })
    );
    assert_eq!(stored(&path)["selectedLanguage"], "auto");
    assert_eq!(
        settings.selected_language(OS_LANGUAGE),
        LanguageSetting::Auto
    );
    assert_eq!(*seen.lock().expect("lock"), vec![Key::SelectedLanguage]);
    assert_eq!(
        settings.reset_invalid_language().expect("again"),
        None,
        "nothing left to reset"
    );
}

#[test]
fn valid_or_unset_language_needs_no_reset() {
    let cases = [
        ("unset", json!({})),
        ("auto", json!({"selectedLanguage": "auto"})),
        ("offered", json!({"selectedLanguage": "pt"})),
        ("wrong type", json!({"selectedLanguage": 12})),
    ];
    for (name, file) in cases {
        let (dir, settings) = with_json(&file);

        assert_eq!(
            settings.reset_invalid_language().expect(name),
            None,
            "{name}"
        );
        assert_eq!(stored(&settings_path(&dir)), file, "{name}");
    }
}

#[test]
fn corrupt_replacement_entries_stay_stored_until_replaced_rep_7() {
    let (dir, settings) = with_json(&json!({"replacementEntries": "{broken"}));
    let path = settings_path(&dir);

    assert_eq!(settings.replacement_rules(), Vec::new());
    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("unrelated write");
    assert_eq!(
        stored(&path)["replacementEntries"],
        "{broken",
        "corrupt value kept"
    );

    let rules = vec![rule("c3d4", &["señor chang"], "Ben Chang")];
    settings.set_replacement_rules(&rules).expect("replace");

    assert_eq!(settings.replacement_rules(), rules);
    assert_eq!(Settings::open(&path).replacement_rules(), rules);
}

#[test]
fn first_run_pending_set_4() {
    struct Case {
        name: &'static str,
        file: Value,
        want: bool,
    }
    let cases = [
        Case {
            name: "fresh install",
            file: json!({}),
            want: true,
        },
        Case {
            name: "both false",
            file: json!({"onboardingCompleted": false, "didShowFirstRunSetup": false}),
            want: true,
        },
        Case {
            name: "completed only",
            file: json!({"onboardingCompleted": true}),
            want: false,
        },
        Case {
            name: "shown only (rerunning setup)",
            file: json!({"didShowFirstRunSetup": true}),
            want: false,
        },
        Case {
            name: "both",
            file: json!({"onboardingCompleted": true, "didShowFirstRunSetup": true}),
            want: false,
        },
    ];
    for tc in cases {
        let (_dir, settings) = with_json(&tc.file);
        assert_eq!(settings.first_run_pending(), tc.want, "{}", tc.name);
    }
}

#[test]
fn onboarding_flags_set_4() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let (settings, seen) = recorder(Settings::open(&path));

    settings.complete_onboarding().expect("complete");
    assert!(settings.onboarding_completed());
    assert!(settings.did_show_first_run_setup());
    assert!(!settings.first_run_pending());
    assert_eq!(
        *seen.lock().expect("lock"),
        vec![Key::OnboardingCompleted, Key::DidShowFirstRunSetup]
    );

    settings.rerun_setup().expect("rerun");
    assert!(
        !settings.onboarding_completed(),
        "Run Setup Again clears onboardingCompleted"
    );
    assert!(settings.did_show_first_run_setup(), "and only that");
    assert!(
        !settings.first_run_pending(),
        "rerunning setup is not first run"
    );
    assert_eq!(
        stored(&path),
        json!({"onboardingCompleted": false, "didShowFirstRunSetup": true})
    );
}

#[test]
fn set_onboarding_flags_writes_both_flags_onb_11() {
    struct Case {
        name: &'static str,
        flags: (bool, bool),
        want_pending: bool,
    }
    let cases = [
        Case {
            name: "abandoned first run stays pending",
            flags: (false, false),
            want_pending: true,
        },
        Case {
            name: "closed rerun stays complete",
            flags: (true, true),
            want_pending: false,
        },
        Case {
            name: "rerun in progress is not first run",
            flags: (false, true),
            want_pending: false,
        },
    ];
    for tc in cases {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        let settings = Settings::open(&path);
        settings
            .set_onboarding_flags(tc.flags.0, tc.flags.1)
            .expect("set flags");
        assert_eq!(settings.onboarding_completed(), tc.flags.0, "{}", tc.name);
        assert_eq!(
            settings.did_show_first_run_setup(),
            tc.flags.1,
            "{}",
            tc.name
        );
        assert_eq!(settings.first_run_pending(), tc.want_pending, "{}", tc.name);
        assert_eq!(
            stored(&path),
            json!({"onboardingCompleted": tc.flags.0, "didShowFirstRunSetup": tc.flags.1}),
            "{}",
            tc.name
        );
    }
}

#[test]
fn change_notifications_report_only_real_changes() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let (settings, seen) = recorder(Settings::open(&path));

    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("set");
    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("same value");
    settings
        .set_hotkey(&Shortcut::lone(ModifierKey::Fn))
        .expect("set");
    settings
        .remove(Key::PostProcessingEnabled)
        .expect("remove unset key");
    settings.remove(Key::OutputMode).expect("remove");

    assert_eq!(
        *seen.lock().expect("lock"),
        vec![Key::OutputMode, Key::Hotkey, Key::OutputMode]
    );
}

#[test]
fn observer_may_read_settings() {
    let dir = TempDir::new().expect("temp dir");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let settings = Arc::new_cyclic(|weak: &std::sync::Weak<Settings>| {
        let weak = weak.clone();
        Settings::open(settings_path(&dir)).on_change(move |_key| {
            if let Some(settings) = weak.upgrade() {
                sink.lock().expect("lock").push(settings.output_mode());
            }
        })
    });

    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("set");

    assert_eq!(*seen.lock().expect("lock"), vec![OutputMode::Clipboard]);
}
