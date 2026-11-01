//! `settings`: reads or writes one setting in a data folder, validated the
//! way the app reads it, so verification runs can change the shortcut or
//! activation mode without the settings window. The app reads settings at
//! launch, so restart it afterwards.

use anyhow::{Context, Result, bail};
use onda_adapters::DataPaths;
use onda_domain::hotkey::Shortcut;
use onda_domain::language_menu::recent_languages;
use onda_domain::languages::LanguageSetting;
use onda_domain::models::{Catalog, ModelId};
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use onda_settings::{Key, Settings};
use serde_json::Value;

/// `settings` subcommands.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Print a setting's effective value as JSON.
    Get {
        /// Setting key, e.g. `hotkey` (09 "Settings keys").
        key: String,
    },
    /// Store a setting from JSON, e.g. `hotkey '{"key":"AltRight","modifiers":[]}'`.
    Set {
        /// Setting key.
        key: String,
        /// The value as JSON.
        value: String,
    },
}

// the keys a verification run needs; anything else is refused
const KEYS: [Key; 13] = [
    Key::SelectedModel,
    Key::SelectedLanguage,
    Key::RecentLanguages,
    Key::HistoryRetention,
    Key::Hotkey,
    Key::HotkeyActivationMode,
    Key::OutputMode,
    Key::InputDeviceUid,
    Key::SoundFeedbackEnabled,
    Key::PauseMediaDuringRecording,
    Key::HistoryEnabled,
    Key::HistoryKeepsAudio,
    Key::PostProcessingEnabled,
];

const ACTIVATION_MODES: [(&str, ActivationMode); 3] = [
    ("hold", ActivationMode::Hold),
    ("toggle", ActivationMode::Toggle),
    ("automatic", ActivationMode::Automatic),
];

const RETENTIONS: [(&str, HistoryRetention); 3] = [
    ("week", HistoryRetention::Week),
    ("month", HistoryRetention::Month),
    ("forever", HistoryRetention::Forever),
];

const OUTPUT_MODES: [(&str, OutputMode); 2] = [
    ("type", OutputMode::Type),
    ("clipboard", OutputMode::Clipboard),
];

pub fn run(command: Command, paths: &DataPaths) -> Result<()> {
    let settings = Settings::open(paths.settings_file());
    match command {
        Command::Get { key } => {
            let key = parse_key(&key)?;
            println!("{}", get(&settings, key)?);
        }
        Command::Set { key, value } => {
            let key = parse_key(&key)?;
            let value: Value = serde_json::from_str(&value).context("the value is not JSON")?;
            set(&settings, key, &value)?;
            eprintln!("{key} = {}", get(&settings, key)?);
        }
    }
    Ok(())
}

fn parse_key(name: &str) -> Result<Key> {
    match KEYS.into_iter().find(|key| key.as_str() == name) {
        Some(key) => Ok(key),
        None => {
            let known: Vec<&str> = KEYS.iter().map(|key| key.as_str()).collect();
            bail!("unsupported key {name}; use one of {}", known.join(", "))
        }
    }
}

fn get(settings: &Settings, key: Key) -> Result<Value> {
    let value = match key {
        Key::SelectedModel => Value::from(settings.selected_model(&Catalog::new(None), None)),
        Key::SelectedLanguage => Value::from(settings.selected_language(None).code()),
        Key::RecentLanguages => Value::from(settings.recent_languages()),
        Key::HistoryRetention => Value::from(name_of(&RETENTIONS, settings.history_retention())),
        Key::Hotkey => serde_json::to_value(settings.hotkey())?,
        Key::HotkeyActivationMode => {
            Value::from(name_of(&ACTIVATION_MODES, settings.activation_mode()))
        }
        Key::OutputMode => Value::from(name_of(&OUTPUT_MODES, settings.output_mode())),
        Key::InputDeviceUid => Value::from(settings.input_device_uid()),
        Key::SoundFeedbackEnabled => Value::from(settings.sound_feedback_enabled()),
        Key::PauseMediaDuringRecording => Value::from(settings.pause_media_during_recording()),
        Key::HistoryEnabled => Value::from(settings.history_enabled()),
        Key::HistoryKeepsAudio => Value::from(settings.history_keeps_audio()),
        Key::PostProcessingEnabled => Value::from(settings.post_processing_enabled()),
        _ => bail!("unsupported key {key}"),
    };
    Ok(value)
}

fn set(settings: &Settings, key: Key, value: &Value) -> Result<()> {
    match key {
        Key::SelectedModel => {
            let id = value.as_str().context("expected a JSON string")?;
            let model = ModelId::parse(id).context("unknown model id")?;
            settings.set_selected_model(model)?;
        }
        Key::SelectedLanguage => settings.set_selected_language(language(value)?)?,
        Key::RecentLanguages => {
            let codes = value.as_str().context("expected a JSON string")?;
            let mut languages = recent_languages(LanguageSetting::Auto, codes);
            // keep only what was given, in order
            if !codes
                .split(',')
                .any(|code| code == LanguageSetting::Auto.code())
            {
                languages.retain(|l| *l != LanguageSetting::Auto);
            }
            settings.set_recent_languages(&languages)?;
        }
        Key::HistoryRetention => settings.set_history_retention(value_of(&RETENTIONS, value)?)?,
        Key::Hotkey => {
            let shortcut = Shortcut::from_setting(value).context("invalid hotkey")?;
            settings.set_hotkey(&shortcut)?;
        }
        Key::HotkeyActivationMode => {
            settings.set_activation_mode(value_of(&ACTIVATION_MODES, value)?)?;
        }
        Key::OutputMode => settings.set_output_mode(value_of(&OUTPUT_MODES, value)?)?,
        Key::InputDeviceUid => {
            let uid = value.as_str().context("expected a JSON string")?;
            settings.set_input_device_uid(uid)?;
        }
        Key::SoundFeedbackEnabled => settings.set_sound_feedback_enabled(boolean(value)?)?,
        Key::PauseMediaDuringRecording => {
            settings.set_pause_media_during_recording(boolean(value)?)?;
        }
        Key::HistoryEnabled => settings.set_history_enabled(boolean(value)?)?,
        Key::HistoryKeepsAudio => settings.set_history_keeps_audio(boolean(value)?)?,
        Key::PostProcessingEnabled => settings.set_post_processing_enabled(boolean(value)?)?,
        _ => bail!("unsupported key {key}"),
    }
    Ok(())
}

fn name_of<T: PartialEq + Copy>(table: &[(&'static str, T)], value: T) -> &'static str {
    table
        .iter()
        .find(|(_, v)| *v == value)
        .map_or("", |(name, _)| name)
}

fn value_of<T: Copy>(table: &[(&'static str, T)], value: &Value) -> Result<T> {
    let name = value.as_str().context("expected a JSON string")?;
    match table.iter().find(|(n, _)| *n == name) {
        Some((_, value)) => Ok(*value),
        None => {
            let known: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
            bail!("expected one of {}", known.join(", "))
        }
    }
}

fn language(value: &Value) -> Result<LanguageSetting> {
    let code = value.as_str().context("expected a JSON string")?;
    LanguageSetting::parse(code).context("not an offered language code")
}

fn boolean(value: &Value) -> Result<bool> {
    value.as_bool().context("expected true or false")
}
