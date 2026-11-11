//! Settings store behavior against real files in temp dirs (SET-1..SET-4).

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use onda_domain::cleanup::{
    CLEANUP_MODELS, CleanupPrompt, CleanupProvider, SYSTEM_PROMPT, default_model,
};
use onda_domain::hotkey::{KeyCode, Modifier, ModifierKey, ModifierSet, Shortcut};
use onda_domain::languages::LanguageSetting;
use onda_domain::messages::UserMessage;
use onda_domain::models::{Catalog, ModelId, SystemSpeechInfo};
use onda_domain::replacements::{ReplacementRule, RuleId};
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use onda_settings::{Key, Settings, SettingsError};
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
    cleanup_prompt: CleanupPrompt,
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
        cleanup_prompt: settings.cleanup_prompt(),
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
        cleanup_prompt: CleanupPrompt::default(),
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

// files beside settings.json other than the lock file, e.g. leftover temps
fn leftovers(path: &Path) -> Vec<String> {
    let folder = path.parent().expect("parent");
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("list folder")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name != "settings.json" && name != "settings.json.lock")
        .collect();
    names.sort();
    names
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

const CUSTOM_PROMPT: &str = "Fix punctuation only.\n\nKeep every word Troy said. ";

fn custom_prompt() -> CleanupPrompt {
    CleanupPrompt::from_text(CUSTOM_PROMPT).expect("a prompt with text")
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
        "cleanupPrompt",
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
    settings.set_cleanup_prompt(&custom_prompt()).expect("set");
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
        cleanup_prompt: custom_prompt(),
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
fn a_custom_cleanup_prompt_is_stored_until_the_default_is_saved() {
    let old = json!({"postProcessingEnabled": true, "openRouterModel": "openai/gpt-4o"});
    let (dir, settings) = with_json(&old);
    let path = settings_path(&dir);
    assert_eq!(settings.cleanup_prompt(), CleanupPrompt::default());
    assert_eq!(settings.cleanup_prompt().text(), SYSTEM_PROMPT);

    settings.set_cleanup_prompt(&custom_prompt()).expect("set");
    assert_eq!(stored(&path)["cleanupPrompt"], CUSTOM_PROMPT, "as typed");
    assert_eq!(Settings::open(&path).cleanup_prompt(), custom_prompt());

    settings.set_post_processing_enabled(false).expect("set");
    settings.set_post_processing_enabled(true).expect("set");
    assert_eq!(Settings::open(&path).cleanup_prompt(), custom_prompt());

    let default = CleanupPrompt::from_text(SYSTEM_PROMPT).expect("the default");
    settings.set_cleanup_prompt(&default).expect("set");
    assert_eq!(stored(&path).get("cleanupPrompt"), None);
    assert_eq!(settings.cleanup_prompt(), CleanupPrompt::default());
    assert_eq!(stored(&path)["openRouterModel"], "openai/gpt-4o");

    let (_dir, settings) = with_json(&json!({"cleanupPrompt": SYSTEM_PROMPT}));
    assert_eq!(settings.cleanup_prompt().custom(), None);
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
        ("cleanupPrompt".into(), json!(7)),
        ("cleanupPrompt".into(), json!(["Fix punctuation only."])),
        ("cleanupPrompt".into(), json!("")),
        ("cleanupPrompt".into(), json!(" \n\t")),
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
        let path = settings_path(&dir);
        assert_eq!(snapshot(&settings), defaults(), "{name}");

        settings.set_output_mode(OutputMode::Clipboard).expect(name);

        assert_eq!(stored(&path), json!({"outputMode": "clipboard"}), "{name}");
        let backups = leftovers(&path);
        assert_eq!(backups.len(), 1, "{name}: {backups:?}");
        assert!(backups[0].starts_with("settings.json.corrupt-"), "{name}");
        let backup = path.with_file_name(&backups[0]);
        assert_eq!(
            fs::read(backup).expect("read backup"),
            contents,
            "{name}: the corrupt file is kept"
        );
    }
}

#[test]
fn corruption_after_open_is_preserved_not_merged() {
    let (dir, settings) = with_json(&json!({"outputMode": "clipboard"}));
    let path = settings_path(&dir);
    fs::write(&path, b"Troy and Abed in the morning").expect("corrupt the file");

    settings.set_history_enabled(false).expect("set");

    assert_eq!(stored(&path), json!({"historyEnabled": false}));
    assert_eq!(
        settings.output_mode(),
        OutputMode::Type,
        "cache follows the file"
    );
    let backups = leftovers(&path);
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(
        fs::read(path.with_file_name(&backups[0])).expect("read backup"),
        b"Troy and Abed in the morning"
    );
}

#[test]
fn repeated_corruption_keeps_distinct_backups() {
    let (dir, settings) = with_json(&json!({}));
    let path = settings_path(&dir);
    let contents = ["Troy Barnes at Greendale", "Abed Nadir at Greendale"];
    for text in contents {
        fs::write(&path, text).expect("corrupt the file");
        settings
            .set_history_enabled(false)
            .expect("recover settings");
    }
    let mut backups: Vec<_> = leftovers(&path)
        .into_iter()
        .map(|name| fs::read_to_string(path.with_file_name(name)).expect("read backup"))
        .collect();
    backups.sort();
    let mut want = contents.to_vec();
    want.sort();
    assert_eq!(backups, want);
    assert_eq!(stored(&path), json!({"historyEnabled": false}));
}

#[test]
fn unreadable_path_reads_as_defaults() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    fs::create_dir_all(&path).expect("a directory where the file should be");

    let settings = Settings::open(&path);

    assert_eq!(snapshot(&settings), defaults());
    assert!(settings.set_output_mode(OutputMode::Clipboard).is_err());
    assert!(path.is_dir(), "an unreadable file is never replaced");
    assert_eq!(leftovers(&path), Vec::<String>::new());
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
    let lock = path.with_file_name("settings.json.lock");
    let (settings, seen) = recorder(Settings::open(&path));

    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("first write creates the directory");
    assert_eq!(
        leftovers(&path),
        Vec::<String>::new(),
        "temp file is renamed into place"
    );
    assert_eq!(stored(&path), json!({"outputMode": "clipboard"}));

    // a directory squatting on the lock path makes the next write fail
    fs::remove_file(&lock).expect("remove the lock file");
    fs::create_dir(&lock).expect("block the lock path");
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

    fs::remove_dir(&lock).expect("unblock");
    settings.set_history_enabled(false).expect("write recovers");
    assert_eq!(
        stored(&path),
        json!({"outputMode": "clipboard", "historyEnabled": false})
    );
}

#[test]
fn a_stale_fixed_temp_path_does_not_block_writes() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let stale = path.with_file_name("settings.json.tmp");
    fs::create_dir_all(&stale).expect("an old version's temp path, squatted");
    let settings = Settings::open(&path);

    settings
        .set_output_mode(OutputMode::Clipboard)
        .expect("set");

    assert_eq!(stored(&path), json!({"outputMode": "clipboard"}));
    assert!(stale.is_dir(), "someone else's temp path is left alone");
    assert_eq!(leftovers(&path), vec!["settings.json.tmp".to_owned()]);
}

#[test]
fn independent_instances_keep_each_others_writes() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let gui = Settings::open(&path);
    let cli = Settings::open(&path);

    cli.set_history_enabled(false)
        .expect("cli turns history off");
    cli.set_history_keeps_audio(true).expect("cli keeps audio");
    gui.set_output_mode(OutputMode::Clipboard)
        .expect("gui write");

    let reopened = Settings::open(&path);
    assert!(!reopened.history_enabled(), "privacy setting survives");
    assert!(reopened.history_keeps_audio());
    assert_eq!(reopened.output_mode(), OutputMode::Clipboard);
    assert!(!gui.history_enabled(), "a write refreshes the cache");

    cli.remove(Key::HistoryKeepsAudio)
        .expect("cli removes a key");
    gui.set_sound_feedback_enabled(false).expect("gui write");
    assert!(
        !Settings::open(&path).history_keeps_audio(),
        "removal survives"
    );
    let want = json!({
        "historyEnabled": false,
        "outputMode": "clipboard",
        "soundFeedbackEnabled": false,
    });
    assert_eq!(stored(&path), want);
}

#[test]
fn a_write_matching_the_file_saves_and_reports_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let path = settings_path(&dir);
    let (gui, seen) = recorder(Settings::open(&path));
    let cli = Settings::open(&path);
    cli.set_output_mode(OutputMode::Clipboard)
        .expect("cli write");
    let before = fs::read(&path).expect("read");

    gui.set_output_mode(OutputMode::Clipboard)
        .expect("same as the file");

    assert_eq!(fs::read(&path).expect("read"), before);
    assert!(seen.lock().expect("lock").is_empty(), "nothing changed");
    assert_eq!(gui.output_mode(), OutputMode::Clipboard);
}

#[test]
fn canonicalize_reads_the_file_it_rewrites() {
    let (dir, settings) = with_json(&json!({"outputMode": "clipboard"}));
    let path = settings_path(&dir);
    Settings::open(&path)
        .set_activation_mode(ActivationMode::Toggle)
        .expect("other writer");
    let mut file = stored(&path);
    file["outputMode"] = json!("paste");
    fs::write(&path, file.to_string()).expect("stale value from another writer");

    let changed = settings.canonicalize().expect("canonicalize");

    assert_eq!(changed, vec![Key::OutputMode]);
    assert_eq!(
        stored(&path),
        json!({"outputMode": "type", "hotkeyActivationMode": "toggle"})
    );
}

// each concurrent writer owns one key and flips it before settling on the
// value in `want_written`
type Writer = fn(&Settings, bool) -> Result<(), SettingsError>;

const WRITERS: [Writer; 4] = [
    |settings, settle| settings.set_history_enabled(!settle),
    |settings, settle| settings.set_history_keeps_audio(settle),
    |settings, settle| settings.set_sound_feedback_enabled(!settle),
    |settings, settle| settings.set_pause_media_during_recording(settle),
];

const ROUNDS: usize = 25;

fn want_written() -> Value {
    json!({
        "historyEnabled": false,
        "historyKeepsAudio": true,
        "soundFeedbackEnabled": false,
        "pauseMediaDuringRecording": true,
        "futureSetting": "Greendale",
    })
}

fn run_writer(path: &Path, writer: Writer) {
    let settings = Settings::open(path);
    for _ in 0..ROUNDS {
        writer(&settings, false).expect("flip");
        writer(&settings, true).expect("settle");
    }
}

fn shared_file() -> (TempDir, PathBuf) {
    let (dir, _settings) = with_json(&json!({"futureSetting": "Greendale"}));
    let path = settings_path(&dir);
    (dir, path)
}

#[test]
fn concurrent_instances_keep_each_others_writes() {
    let (_dir, path) = shared_file();
    let start = Arc::new(Barrier::new(WRITERS.len()));

    let handles: Vec<_> = WRITERS
        .into_iter()
        .map(|writer| {
            let path = path.clone();
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                run_writer(&path, writer);
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("writer thread");
    }

    assert_eq!(stored(&path), want_written());
    assert_eq!(leftovers(&path), Vec::<String>::new(), "no temp files left");
}

const CHILD_PATH: &str = "ONDA_SETTINGS_TEST_PATH";
const CHILD_WRITER: &str = "ONDA_SETTINGS_TEST_WRITER";

#[test]
fn concurrent_processes_keep_each_others_writes() {
    // a child process runs this test again to be one of the writers
    if let Some(path) = env::var_os(CHILD_PATH) {
        let index: usize = env::var(CHILD_WRITER)
            .expect("writer index")
            .parse()
            .expect("number");
        let path = PathBuf::from(path);
        let go = path.with_file_name("go");
        while !go.exists() {
            thread::sleep(Duration::from_millis(1));
        }
        run_writer(&path, WRITERS[index]);
        return;
    }
    let (_dir, path) = shared_file();
    let exe = env::current_exe().expect("test binary");

    let children: Vec<_> = (0..WRITERS.len())
        .map(|index| {
            Command::new(&exe)
                .args([
                    "--exact",
                    "concurrent_processes_keep_each_others_writes",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env(CHILD_PATH, &path)
                .env(CHILD_WRITER, index.to_string())
                .stdout(Stdio::null())
                .spawn()
                .expect("spawn writer")
        })
        .collect();
    let go = path.with_file_name("go");
    fs::write(&go, b"").expect("start the writers");
    for mut child in children {
        let status = child.wait().expect("wait for writer");
        assert!(status.success(), "writer failed: {status}");
    }
    fs::remove_file(&go).expect("remove start file");

    assert_eq!(stored(&path), want_written());
    assert_eq!(leftovers(&path), Vec::<String>::new(), "no temp files left");
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
