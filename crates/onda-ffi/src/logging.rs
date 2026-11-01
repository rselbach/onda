//! Unified logging for the Rust core (LOG-1): the app's subsystem, one
//! category per component, picked from each event's `tracing` target.

use std::sync::Once;

use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{LevelFilter, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

/// Log subsystem (ARCH-13, LOG-1).
const LOG_SUBSYSTEM: &str = "com.rselbach.onda";

/// Where anything without a component of its own goes, third-party crates
/// included.
const FALLBACK_CATEGORY: &str = "Core";

/// Target prefixes (module paths) and their categories, most specific first.
const CATEGORIES: &[(&str, &str)] = &[
    ("onda_app::session::hotkeys", "Hotkey"),
    ("onda_app::session::history", "History"),
    ("onda_app::session::cleanup", "Cleanup"),
    ("onda_app::session::models", "Models"),
    ("onda_app::session::onboarding", "Settings"),
    ("onda_app::session::rules_editor", "Settings"),
    ("onda_app::session::windows", "Settings"),
    ("onda_app::media", "Media"),
    ("onda_app::system_speech", "Models"),
    ("onda_app", "Dictation"),
    ("onda_adapters::audio", "Audio"),
    ("onda_adapters::history", "History"),
    ("onda_adapters::models", "Models"),
    ("onda_adapters::secrets", "Cleanup"),
    ("onda_adapters::settings", "Settings"),
    ("onda_audio", "Audio"),
    ("onda_asr", "Models"),
    ("onda_asr_coreml", "Models"),
    ("onda_models", "Models"),
    ("onda_cleanup", "Cleanup"),
    ("onda_history", "History"),
    ("onda_settings", "Settings"),
    ("onda_ffi::system_speech", "Models"),
];

/// The category for an event logged under `target` (LOG-1).
fn category(target: &str) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(prefix, _)| {
            target
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
        })
        .map_or(FALLBACK_CATEGORY, |(_, category)| category)
}

// one os_log handle per category, each seeing only its own targets
fn category_layers<S>() -> Vec<Box<dyn Layer<S> + Send + Sync>>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let mut names: Vec<&'static str> = CATEGORIES.iter().map(|(_, name)| *name).collect();
    names.push(FALLBACK_CATEGORY);
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            tracing_oslog::OsLogger::new(LOG_SUBSYSTEM, name)
                .with_filter(filter_fn(move |metadata| {
                    category(metadata.target()) == name
                }))
                .boxed()
        })
        .collect()
}

/// Installs the logger, once per process (LOG-1).
pub(crate) fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Err(error) = tracing_subscriber::registry()
            .with(category_layers())
            .with(LevelFilter::INFO)
            .try_init()
        {
            eprintln!("onda: could not install the logger: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_follow_the_component_log_1() {
        let cases = [
            ("onda_app::session", "Dictation"),
            ("onda_app::pipeline", "Dictation"),
            ("onda_app::transcription", "Dictation"),
            ("onda_app::session::hotkeys", "Hotkey"),
            ("onda_app::session::history", "History"),
            ("onda_app::session::cleanup", "Cleanup"),
            ("onda_app::session::models", "Models"),
            ("onda_app::session::windows", "Settings"),
            ("onda_app::media", "Media"),
            ("onda_audio::capture", "Audio"),
            ("onda_adapters::audio", "Audio"),
            ("onda_asr_coreml::engine", "Models"),
            ("onda_asr", "Models"),
            ("onda_models::download", "Models"),
            ("onda_cleanup::openrouter", "Cleanup"),
            ("onda_history::store", "History"),
            ("onda_settings", "Settings"),
            ("onda_ffi", "Core"),
            ("onda_ffi::ports", "Core"),
            ("reqwest::connect", "Core"),
            // a crate whose name merely starts like ours is not ours
            ("onda_application", "Core"),
        ];
        for (target, want) in cases {
            assert_eq!(category(target), want, "{target}");
        }
    }
}
