//! `onda-cli`: exercises the core from a terminal (M2–M6).
//!
//! Logs go to stderr; set `RUST_LOG=debug` for detail.

mod dictate;
mod models;
mod onboarding;
mod record;
mod settings;
mod transcribe;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use onda_adapters::DataPaths;

#[derive(Parser)]
#[command(about = "Onda developer CLI")]
struct Cli {
    /// Onda data folder (settings, history, models); defaults to the app's.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Speech model folder; defaults to <data>/Models.
    #[arg(long, global = true)]
    models_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List input devices; `*` marks the system default.
    Devices,
    /// Record sessions from a microphone to 16 kHz WAV files.
    Record(record::Args),
    /// List, download, validate, or delete speech models.
    Models {
        #[command(subcommand)]
        command: models::Command,
    },
    /// Transcribe a 16 kHz WAV file.
    Transcribe(transcribe::Args),
    /// Dictate with the session engine: real microphone, real model, the
    /// terminal as the shell.
    Dictate(dictate::Args),
    /// Complete or reset first-run onboarding in the data folder.
    Onboarding {
        #[command(subcommand)]
        command: onboarding::Command,
    },
    /// Read or write one setting in the data folder.
    Settings {
        #[command(subcommand)]
        command: settings::Command,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let paths = DataPaths::new(match cli.data_dir {
        Some(dir) => dir,
        None => default_data_dir()?,
    });
    let models_dir = cli.models_dir.unwrap_or_else(|| paths.models_dir());
    match cli.command {
        Command::Devices => record::devices(),
        Command::Record(args) => record::run(&args),
        Command::Models { command } => models::run(command, &models_dir),
        Command::Transcribe(args) => transcribe::run(&args, &models_dir),
        Command::Dictate(args) => dictate::run(&args, &paths, &models_dir),
        Command::Onboarding { command } => onboarding::run(command, &paths),
        Command::Settings { command } => settings::run(command, &paths),
    }
}

// ~/Library/Application Support/Onda on macOS
fn default_data_dir() -> Result<PathBuf> {
    let data = dirs::data_dir().context("no application data directory on this system")?;
    Ok(data.join("Onda"))
}
