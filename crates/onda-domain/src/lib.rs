//! Pure Onda rules.
//!
//! Everything here is a function of its inputs (ARCH-2): no I/O, no clocks,
//! no threads. Time arrives as a [`types::MonoTime`] parameter.

pub mod cleanup;
pub mod clock;
pub mod download_progress;
pub mod history;
pub mod hotkey;
pub mod joiner;
pub mod language_menu;
pub mod languages;
pub mod level;
pub mod messages;
pub mod migration;
pub mod model_selection;
pub mod models;
pub mod non_dictation_ui;
pub mod onboarding;
pub mod overlay;
pub mod pre_roll;
pub mod preview;
pub mod preview_text;
pub mod privacy_naming;
pub mod replacements;
pub mod routing;
pub mod setup;
pub mod speech_gate;
pub mod status_menu;
pub mod tray;
pub mod types;
pub mod typing;
pub mod updates;
