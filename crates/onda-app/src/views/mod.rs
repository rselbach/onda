//! Complete view models, not deltas, for the main window pages,
//! onboarding, and the update notice.
//!
//! Each builder is a pure function of what the session actor knows, so the
//! shell only renders (ARCH-3). Copy rides along in the view models; dates
//! cross as timestamps or calendar days for the shell to localize.

mod about;
mod general;
mod getting_started;
mod history;
mod hotkey;
mod notice;
mod onboarding;
mod post_processing;
mod replacements;
mod speech;

pub use about::{ABOUT_TAGLINE, AboutLibrary, AboutModel, AboutPage, LIBRARIES, about_page};
pub use general::{
    GeneralInputs, GeneralPage, InputOption, InputPickerView, LoginCaptionView, OutputModeOption,
    OutputModePicker, UpdaterState, general_page, natural_cmp,
};
pub use getting_started::{
    ChecklistButton, ChecklistRow, GettingStartedPage, getting_started_page,
};
pub use history::{
    DayHeading, HistoryContent, HistoryDialog, HistoryInputs, HistoryMenuItem, HistoryPage,
    HistoryRow, HistorySection, RetentionOption, history_page,
};
pub use hotkey::{ActivationOption, HotkeyPage, hotkey_page};
pub use notice::{NoticeButtonView, UpdateNotice, update_notice};
pub use onboarding::{
    LanguageCardView, ModelCardAction, ModelCardView, OnboardingContent, OnboardingInputs,
    OnboardingProgress, OnboardingView, OtherModelsView, PermissionCardView, PresetCardView,
    onboarding_view,
};
pub use post_processing::{
    ApiKeyFieldView, CleanupModelGroup, CleanupModelOption, CleanupProviderOption,
    CleanupTestResult, PostProcessingDetails, PostProcessingInputs, PostProcessingPage,
    PromptDraftInputs, PromptEditorView, PromptSummaryView, ProvidersDialog, ProvidersDialogInputs,
    key_placeholder, models_error_text, post_processing_page, prompt_saved_text,
};
pub use replacements::{ReplacementRowView, ReplacementsPage, replacements_page};
pub use speech::{
    CompatibilityView, LanguageOption, ModelRow, ModelRowControl, NoteActionView, SpeechAlert,
    SpeechInputs, SpeechPage, speech_page,
};

use onda_domain::setup::{GRANTED_LABEL, PermissionFix, PermissionRow};

/// A main window page (WIN-2), in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainWindowSection {
    /// Getting Started, with the setup checklist.
    GettingStarted,
    /// General.
    General,
    /// Hotkey.
    Hotkey,
    /// Speech.
    Speech,
    /// Post-Processing.
    PostProcessing,
    /// Replacements.
    Replacements,
    /// History.
    History,
    /// About.
    About,
}

impl MainWindowSection {
    /// Every page, in sidebar order.
    pub const ALL: [Self; 8] = [
        Self::GettingStarted,
        Self::General,
        Self::Hotkey,
        Self::Speech,
        Self::PostProcessing,
        Self::Replacements,
        Self::History,
        Self::About,
    ];

    /// The page's title in the sidebar and the window toolbar (WIN-2).
    pub fn title(self) -> &'static str {
        match self {
            Self::GettingStarted => "Getting Started",
            Self::General => "General",
            Self::Hotkey => "Hotkey",
            Self::Speech => "Speech",
            Self::PostProcessing => "Post-Processing",
            Self::Replacements => "Replacements",
            Self::History => "History",
            Self::About => "About",
        }
    }
}

/// One sidebar page (WIN-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPage {
    /// The page.
    pub section: MainWindowSection,
    /// Its title.
    pub title: String,
}

/// One sidebar group with its header (WIN-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarGroup {
    /// "Setup", "Configure", "Activity", or "Help".
    pub title: String,
    /// Pages in order.
    pub pages: Vec<SidebarPage>,
}

/// The main window sidebar, top to bottom (WIN-2).
pub fn sidebar() -> Vec<SidebarGroup> {
    use MainWindowSection::*;
    let groups: [(&str, &[MainWindowSection]); 4] = [
        ("Setup", &[GettingStarted]),
        (
            "Configure",
            &[General, Hotkey, Speech, PostProcessing, Replacements],
        ),
        ("Activity", &[History]),
        ("Help", &[About]),
    ];
    groups
        .into_iter()
        .map(|(title, pages)| SidebarGroup {
            title: title.to_owned(),
            pages: pages
                .iter()
                .map(|&section| SidebarPage {
                    section,
                    title: section.title().to_owned(),
                })
                .collect(),
        })
        .collect()
}

/// A toggle with its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleView {
    /// Label.
    pub label: String,
    /// Switched on.
    pub on: bool,
    /// Can be used.
    pub enabled: bool,
}

impl ToggleView {
    fn new(label: &str, on: bool, enabled: bool) -> Self {
        Self {
            label: label.to_owned(),
            on,
            enabled,
        }
    }
}

/// A button with its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonView {
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
}

impl ButtonView {
    fn new(title: &str, enabled: bool) -> Self {
        Self {
            title: title.to_owned(),
            enabled,
        }
    }
}

/// An alert with one dismiss button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertView {
    /// Title.
    pub title: String,
    /// Message.
    pub message: String,
}

/// A permission fix button (WIN-8, ONB-6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionFixButton {
    /// Title: "Allow" or "Open Settings".
    pub title: String,
    /// What it does; pass to `fix_permission`.
    pub fix: PermissionFix,
}

impl From<PermissionFix> for PermissionFixButton {
    fn from(fix: PermissionFix) -> Self {
        Self {
            title: fix.title().to_owned(),
            fix,
        }
    }
}

/// One General page permission row (WIN-8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRowView {
    /// Row title.
    pub title: String,
    /// Show `granted_label` in green instead of a button.
    pub granted: bool,
    /// "Granted".
    pub granted_label: String,
    /// Detail line.
    pub detail: String,
    /// Render the detail in the warning color.
    pub detail_warning: bool,
    /// Fix button, when one helps.
    pub fix: Option<PermissionFixButton>,
}

impl From<PermissionRow> for PermissionRowView {
    fn from(row: PermissionRow) -> Self {
        Self {
            title: row.title.to_owned(),
            granted: row.granted,
            granted_label: GRANTED_LABEL.to_owned(),
            detail: row.detail.to_owned(),
            detail_warning: row.needs_attention,
            fix: row.fix.map(PermissionFixButton::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_2_sidebar_lists_every_page_once_in_order() {
        let pages: Vec<MainWindowSection> = sidebar()
            .into_iter()
            .flat_map(|group| group.pages.into_iter().map(|page| page.section))
            .collect();
        assert_eq!(pages, MainWindowSection::ALL);
        let titles: Vec<String> = sidebar().into_iter().map(|group| group.title).collect();
        assert_eq!(titles, ["Setup", "Configure", "Activity", "Help"]);
    }
}
