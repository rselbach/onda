//! The Getting Started page (WIN-4..WIN-7).

use onda_domain::hotkey::KeyNames;
use onda_domain::setup::{
    SetupAction, SetupChecklist, SetupStepId, SetupStepStatus, try_it_placeholder,
};
use onda_domain::types::OutputMode;

/// "Run Setup Again" (WIN-4).
const RUN_SETUP_AGAIN: &str = "Run Setup Again";

/// One checklist row (WIN-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistRow {
    /// Which requirement.
    pub step: SetupStepId,
    /// Row title.
    pub title: String,
    /// Status, which also picks the icon: green check, orange exclamation,
    /// blue down-arrow.
    pub status: SetupStepStatus,
    /// Message, shown only while not complete.
    pub message: Option<String>,
    /// Button, only when the row needs action.
    pub button: Option<ChecklistButton>,
}

/// A checklist row's button; pass `action` to `setup_action`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistButton {
    /// Title.
    pub title: String,
    /// What it does.
    pub action: SetupAction,
}

/// The Getting Started page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GettingStartedPage {
    /// Green "Onda is ready" line on top, when every row is complete.
    pub ready_line: Option<String>,
    /// Microphone, Accessibility, Speech Model. A permission the system
    /// does not have has no row.
    pub steps: Vec<ChecklistRow>,
    /// "Run Setup Again".
    pub run_setup_again_title: String,
    /// "Press", before the keycaps (WIN-6).
    pub try_it_prefix: String,
    /// The hotkey's keycaps.
    pub keycaps: Vec<String>,
    /// "and speak.", after the keycaps.
    pub try_it_suffix: String,
    /// The try-it field's placeholder for the output mode.
    pub try_it_placeholder: String,
}

/// Builds the page from the resolved checklist.
pub fn getting_started_page(
    checklist: &SetupChecklist,
    keycaps: Vec<String>,
    output_mode: OutputMode,
    names: &KeyNames,
) -> GettingStartedPage {
    GettingStartedPage {
        ready_line: checklist.ready_line().map(str::to_owned),
        steps: checklist
            .steps
            .iter()
            .map(|step| ChecklistRow {
                step: step.id,
                title: step.title.to_owned(),
                status: step.status,
                message: step.visible_message().map(str::to_owned),
                button: step.button.map(|button| ChecklistButton {
                    title: button.title.to_owned(),
                    action: button.action,
                }),
            })
            .collect(),
        run_setup_again_title: RUN_SETUP_AGAIN.to_owned(),
        try_it_prefix: "Press".to_owned(),
        keycaps,
        try_it_suffix: "and speak.".to_owned(),
        try_it_placeholder: try_it_placeholder(output_mode, names),
    }
}
