//! `settings`: reads or writes one setting in a data folder, validated the
//! way the app reads it, so verification runs can change the shortcut or
//! activation mode without the settings window. The app reads settings at
//! launch, so restart it afterwards.

use anyhow::{Context, Result, anyhow, bail};
use onda_adapters::DataPaths;
use onda_domain::hotkey::{KeyNames, Shortcut};
use onda_domain::language_menu::recent_languages;
use onda_domain::languages::LanguageSetting;
use onda_domain::models::{Catalog, ModelId};
use onda_domain::platform::Platform;
use onda_domain::privacy_naming::PrivacyNaming;
use onda_settings::{Key, SettingEnum, Settings};
use serde_json::Value;

/// `settings` subcommands.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Print a setting's effective value as JSON.
    Get {
        /// Setting key, e.g. `hotkey`.
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
    let os_language = crate::os_language();
    let value = match key {
        Key::SelectedModel => {
            Value::from(settings.selected_model(&Catalog::new(None), os_language.as_deref()))
        }
        Key::SelectedLanguage => {
            Value::from(settings.selected_language(os_language.as_deref()).code())
        }
        Key::RecentLanguages => Value::from(settings.recent_languages()),
        Key::HistoryRetention => Value::from(settings.history_retention().as_setting()),
        Key::Hotkey => {
            let default = Platform::macos(PrivacyNaming::Accessibility).default_shortcut;
            serde_json::to_value(settings.hotkey(default))?
        }
        Key::HotkeyActivationMode => Value::from(settings.activation_mode().as_setting()),
        Key::OutputMode => Value::from(settings.output_mode().as_setting()),
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
            let languages = parse_recents(codes);
            settings.set_recent_languages(&languages)?;
        }
        Key::HistoryRetention => settings.set_history_retention(enum_from_json(value)?)?,
        Key::Hotkey => {
            let shortcut = Shortcut::from_setting(value)
                .map_err(|error| anyhow!(error.message(&KeyNames::MACOS)))
                .context("invalid hotkey")?;
            settings.set_hotkey(&shortcut)?;
        }
        Key::HotkeyActivationMode => settings.set_activation_mode(enum_from_json(value)?)?,
        Key::OutputMode => settings.set_output_mode(enum_from_json(value)?)?,
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

fn parse_recents(codes: &str) -> Vec<LanguageSetting> {
    codes
        .split(',')
        .find_map(LanguageSetting::parse)
        .map(|first| recent_languages(first, codes))
        .unwrap_or_default()
}

fn enum_from_json<E: SettingEnum>(value: &Value) -> Result<E> {
    let name = value.as_str().context("expected a JSON string")?;
    match E::from_setting(name) {
        Some(parsed) => Ok(parsed),
        None => {
            let known: Vec<&str> = E::VARIANTS.iter().map(|(n, _)| *n).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use onda_domain::language_menu::encode_recent;
    use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};

    #[test]
    fn recent_language_input_keeps_the_first_five_distinct_valid_codes() {
        for (input, want) in [
            ("en,es,fr,de,pt", "en,es,fr,de,pt"),
            ("unknown,en,es,en,auto,fr,de,pt", "en,es,auto,fr,de"),
            ("unknown,", ""),
        ] {
            assert_eq!(encode_recent(&parse_recents(input)), want);
        }
    }

    #[test]
    fn enum_json_uses_canonical_setting_variants() {
        assert_eq!(
            enum_from_json::<ActivationMode>(&Value::from("automatic")).unwrap(),
            ActivationMode::Automatic
        );
        assert_eq!(
            enum_from_json::<HistoryRetention>(&Value::from("forever")).unwrap(),
            HistoryRetention::Forever
        );
        assert_eq!(
            enum_from_json::<OutputMode>(&Value::from("clipboard")).unwrap(),
            OutputMode::Clipboard
        );
        assert!(enum_from_json::<OutputMode>(&Value::from("paste")).is_err());
    }
}
