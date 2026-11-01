//! `onboarding`: sets or clears the first-run flags in a data folder
//! (SET-4). Until the app ships its onboarding window, the core loads no
//! model while first-run onboarding is pending (ASR-1), so verification
//! runs complete it here.

use anyhow::{Context, Result};
use onda_adapters::DataPaths;
use onda_settings::{Key, Settings};

/// `onboarding` subcommands.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Mark onboarding finished, as "Start Dictating" does (ONB-10).
    Complete,
    /// Clear both onboarding flags, so first-run onboarding is pending again.
    Reset,
}

pub fn run(command: Command, paths: &DataPaths) -> Result<()> {
    let settings = Settings::open(paths.settings_file());
    match command {
        Command::Complete => settings
            .complete_onboarding()
            .context("saving the onboarding flags")?,
        Command::Reset => {
            for key in [Key::OnboardingCompleted, Key::DidShowFirstRunSetup] {
                settings
                    .remove(key)
                    .with_context(|| format!("clearing {}", key.as_str()))?;
            }
        }
    }
    eprintln!(
        "first-run onboarding pending in {}: {}",
        paths.root().display(),
        settings.first_run_pending()
    );
    Ok(())
}
