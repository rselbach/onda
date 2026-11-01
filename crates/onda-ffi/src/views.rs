//! The window view models (`onda_app::views`) and the domain enums they
//! use, exposed to the shell as UniFFI remote types: the definitions below
//! restate the core's types field for field, so no conversion code exists
//! and nothing can drift silently (a mismatch fails to compile).

use std::time::SystemTime;

use onda_app::views::{
    AboutLibrary, AboutModel, AboutPage, ActivationOption, AlertView, ApiKeyFieldView, ButtonView,
    ChecklistButton, ChecklistRow, CleanupModelGroup, CleanupModelOption, CleanupProviderOption,
    CleanupTestResult, CompatibilityView, DayHeading, GeneralPage, GettingStartedPage,
    HistoryContent, HistoryDialog, HistoryMenuItem, HistoryPage, HistoryRow, HistorySection,
    HotkeyPage, InputOption, InputPickerView, LanguageCardView, LanguageOption, LoginCaptionView,
    MainWindowSection, ModelCardAction, ModelCardView, ModelRow, ModelRowControl, NoteActionView,
    NoticeButtonView, OnboardingContent, OnboardingProgress, OnboardingView, OtherModelsView,
    OutputModeOption, OutputModePicker, PermissionCardView, PermissionFixButton, PermissionRowView,
    PostProcessingDetails, PostProcessingPage, PresetCardView, ProvidersDialog, ReplacementRowView,
    ReplacementsPage, RetentionOption, SidebarGroup, SidebarPage, SpeechAlert, SpeechPage,
    ToggleView, UpdateNotice, sidebar,
};
use onda_domain::history::ContextMenuItem;
use onda_domain::hotkey::HotkeyPreset;
use onda_domain::messages::CaptionTone;
use onda_domain::migration::{NoticeChoice, NoticeShortcut};
use onda_domain::onboarding::OnboardingStep;
use onda_domain::routing::NoteActionKind;
use onda_domain::setup::{PermissionFix, SetupAction, SetupStepId, SetupStepStatus};
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use onda_domain::updates::{UpdateAction, UpdateCaption, UpdateControl};

/// What the update control does when chosen; pass to `update_action`.
#[uniffi::remote(Enum)]
pub enum UpdateAction {
    /// Check for updates now.
    Check,
    /// Quit, install the staged update, and relaunch.
    Restart,
    /// Open the release download in the browser.
    Download,
}

/// The line under the update control on General.
#[uniffi::remote(Record)]
pub struct UpdateCaption {
    /// Text.
    pub text: String,
    /// Orange for a failure, else secondary.
    pub tone: CaptionTone,
}

/// The update item in the status menu and the button on General.
#[uniffi::remote(Record)]
pub struct UpdateControl {
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
    /// What choosing it does.
    pub action: UpdateAction,
}

/// Where a transcript goes.
#[uniffi::remote(Enum)]
pub enum OutputMode {
    /// Type into the target app.
    Type,
    /// Copy to the clipboard only.
    Clipboard,
}

/// How the hotkey starts and stops a recording (WIN-17).
#[uniffi::remote(Enum)]
pub enum ActivationMode {
    /// Press and hold.
    Hold,
    /// Tap to start, tap to stop.
    Toggle,
    /// Quick tap toggles; holding is push-to-talk.
    Automatic,
}

/// How long history is kept (HIS-13).
#[uniffi::remote(Enum)]
pub enum HistoryRetention {
    /// 7 days.
    Week,
    /// 30 days.
    Month,
    /// Never by age.
    Forever,
}

/// A permission fix button's action; pass to `fix_permission`.
#[uniffi::remote(Enum)]
pub enum PermissionFix {
    /// Show the microphone prompt; open settings when denied.
    RequestMicrophone,
    /// Open the microphone privacy pane.
    OpenMicrophoneSettings,
    /// Prompt for Accessibility, then open its pane.
    OpenAccessibilitySettings,
}

/// A checklist row (WIN-5).
#[uniffi::remote(Enum)]
pub enum SetupStepId {
    /// Microphone.
    Microphone,
    /// Accessibility.
    Accessibility,
    /// Speech model.
    SpeechModel,
}

/// A checklist row's status and icon (WIN-5).
#[uniffi::remote(Enum)]
pub enum SetupStepStatus {
    /// Green check.
    Complete,
    /// Orange exclamation.
    NeedsAction,
    /// Blue down-arrow.
    InProgress,
}

/// A checklist button's action; pass to `setup_action`.
#[uniffi::remote(Enum)]
pub enum SetupAction {
    /// Request the microphone.
    RequestMicrophone,
    /// Prompt for Accessibility, then open its pane.
    OpenAccessibilitySettings,
    /// Download the base model.
    DownloadBaseModel,
}

/// A caption's color (LOGIN-2).
#[uniffi::remote(Enum)]
pub enum CaptionTone {
    /// Orange.
    Warning,
    /// Secondary.
    Secondary,
}

/// The compatibility note button's action.
#[uniffi::remote(Enum)]
pub enum NoteActionKind {
    /// `select_model`.
    Use,
    /// `download_and_use`.
    DownloadAndUse,
    /// Disabled while downloading.
    Downloading,
}

/// A History row context menu item (HIS-24).
#[uniffi::remote(Enum)]
pub enum ContextMenuItem {
    /// `copy_history_entry`.
    Copy,
    /// `copy_history_original`.
    CopyOriginal,
    /// `play_history_recording`.
    PlayRecording,
    /// `show_history_recording`.
    ShowRecording,
    /// A separator.
    Divider,
    /// `delete_history_entry`.
    Delete,
}

/// An onboarding step (ONB-1).
#[uniffi::remote(Enum)]
pub enum OnboardingStep {
    /// ONB-3.
    Welcome,
    /// ONB-4.
    Language,
    /// ONB-6.
    Permissions,
    /// ONB-7.
    Hotkey,
    /// ONB-8.
    SpeechModel,
    /// ONB-9.
    Ready,
}

/// An onboarding hotkey preset (ONB-7).
#[uniffi::remote(Enum)]
pub enum HotkeyPreset {
    /// ⌥ Space.
    OptionSpace,
    /// Right Option alone.
    RightOption,
    /// Fn alone.
    Fn,
}

/// An update notice button (DL-14); closing the window is `NotNow`.
#[uniffi::remote(Enum)]
pub enum NoticeChoice {
    /// Download the replacement.
    Download,
    /// Open Speech settings.
    ChooseAnother,
    /// Decline for now.
    NotNow,
}

/// A notice button's key.
#[uniffi::remote(Enum)]
pub enum NoticeShortcut {
    /// Escape.
    Escape,
    /// Return; the default button.
    Return,
}

/// A main window page (WIN-2), in sidebar order.
#[uniffi::remote(Enum)]
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

/// One sidebar page (WIN-2).
#[uniffi::remote(Record)]
pub struct SidebarPage {
    /// The page.
    pub section: MainWindowSection,
    /// Its title.
    pub title: String,
}

/// One sidebar group with its header (WIN-2).
#[uniffi::remote(Record)]
pub struct SidebarGroup {
    /// "Setup", "Configure", "Activity", or "Help".
    pub title: String,
    /// Pages in order.
    pub pages: Vec<SidebarPage>,
}

/// The main window sidebar, top to bottom (WIN-2).
#[uniffi::export]
pub fn main_window_sidebar() -> Vec<SidebarGroup> {
    sidebar()
}

/// A toggle with its label.
#[uniffi::remote(Record)]
pub struct ToggleView {
    /// Label.
    pub label: String,
    /// Switched on.
    pub on: bool,
    /// Can be used.
    pub enabled: bool,
}

/// A button with its title.
#[uniffi::remote(Record)]
pub struct ButtonView {
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
}

/// An alert with one dismiss button.
#[uniffi::remote(Record)]
pub struct AlertView {
    /// Title.
    pub title: String,
    /// Message.
    pub message: String,
}

/// A permission fix button (WIN-8, ONB-6).
#[uniffi::remote(Record)]
pub struct PermissionFixButton {
    /// Title: "Allow" or "Open Settings".
    pub title: String,
    /// What it does; pass to `fix_permission`.
    pub fix: PermissionFix,
}

/// One General page permission row (WIN-8).
#[uniffi::remote(Record)]
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

/// One checklist row (WIN-5).
#[uniffi::remote(Record)]
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
#[uniffi::remote(Record)]
pub struct ChecklistButton {
    /// Title.
    pub title: String,
    /// What it does.
    pub action: SetupAction,
}

/// The Getting Started page view model.
#[uniffi::remote(Record)]
pub struct GettingStartedPage {
    /// Green "Onda is ready" line on top, when every row is complete.
    pub ready_line: Option<String>,
    /// Microphone, Accessibility, Speech Model.
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

/// One input picker entry.
#[uniffi::remote(Record)]
pub struct InputOption {
    /// Value for `select_input_device`; empty for "System Default".
    pub uid: String,
    /// Display name.
    pub name: String,
}

/// The "Input" picker (WIN-9, AUD-3).
#[uniffi::remote(Record)]
pub struct InputPickerView {
    /// "Input".
    pub label: String,
    /// "System Default", then devices in natural order, then a missing
    /// selected device as "Unavailable Microphone".
    pub options: Vec<InputOption>,
    /// The selected option's uid.
    pub selected_uid: String,
    /// The orange AUD-3 note when the selected device is not connected.
    pub warning: Option<String>,
    /// Which device is in use.
    pub caption: Option<String>,
}

/// One output mode radio button (WIN-10).
#[uniffi::remote(Record)]
pub struct OutputModeOption {
    /// The mode.
    pub mode: OutputMode,
    /// Title.
    pub title: String,
}

/// The "After transcription" radio group (WIN-10).
#[uniffi::remote(Record)]
pub struct OutputModePicker {
    /// "After transcription".
    pub label: String,
    /// Options in order.
    pub options: Vec<OutputModeOption>,
    /// The selected mode.
    pub selected: OutputMode,
    /// "Output will be copied to the clipboard only." in clipboard mode.
    pub caption: Option<String>,
}

/// The caption under the launch-at-login toggle (LOGIN-2).
#[uniffi::remote(Record)]
pub struct LoginCaptionView {
    /// Text.
    pub text: String,
    /// Orange or secondary.
    pub tone: CaptionTone,
    /// "Open Login Items", which calls `open_login_items`.
    pub button: Option<String>,
}

/// The General page view model.
#[uniffi::remote(Record)]
pub struct GeneralPage {
    /// Microphone permission row.
    pub microphone: PermissionRowView,
    /// Accessibility permission row.
    pub accessibility: PermissionRowView,
    /// Input device picker.
    pub input: InputPickerView,
    /// Output mode.
    pub output: OutputModePicker,
    /// "Pause media while recording".
    pub pause_media: ToggleView,
    /// Caption under the media toggle.
    pub pause_media_caption: String,
    /// "Play sounds when dictation starts and stops".
    pub sounds: ToggleView,
    /// "Open Onda at login".
    pub launch_at_login: ToggleView,
    /// Caption under the login toggle.
    pub launch_at_login_caption: Option<LoginCaptionView>,
    /// "Check for updates automatically".
    pub automatic_updates: ToggleView,
    /// "Check for Updates…", "Restart to Update", or "Download Onda …".
    pub update: UpdateControl,
    /// What the updater is doing, under the update control.
    pub update_caption: Option<UpdateCaption>,
    /// "Current version: <version>".
    pub version_footer: String,
    /// When the updater last checked; the shell localizes it.
    pub last_update_check: Option<SystemTime>,
}

/// One activation mode in the picker (WIN-17).
#[uniffi::remote(Record)]
pub struct ActivationOption {
    /// The mode.
    pub mode: ActivationMode,
    /// "Hold", "Toggle", or "Automatic".
    pub title: String,
}

/// The Hotkey page view model.
#[uniffi::remote(Record)]
pub struct HotkeyPage {
    /// "Press to talk", the row with the keycaps.
    pub press_to_talk_label: String,
    /// "Reset to ⌥ Space", disabled when already the default.
    pub reset: ButtonView,
    /// Footer under the shortcut section.
    pub footer: String,
    /// "Activation".
    pub activation_label: String,
    /// Hold, Toggle, Automatic.
    pub activation_options: Vec<ActivationOption>,
    /// The selected mode.
    pub activation: ActivationMode,
    /// The selected mode's description.
    pub activation_description: String,
}

/// One "Language" picker entry (WIN-18).
#[uniffi::remote(Record)]
pub struct LanguageOption {
    /// Value for `select_language`.
    pub code: String,
    /// Display name.
    pub name: String,
    /// The "Automatic" entry.
    pub automatic: bool,
    /// Draw a divider after this entry.
    pub divider_after: bool,
}

/// The button under an "unsupported" compatibility note.
#[uniffi::remote(Record)]
pub struct NoteActionView {
    /// The recommended model it acts on.
    pub model_id: String,
    /// `Use` calls `select_model`, `DownloadAndUse` calls
    /// `download_and_use`, `Downloading` is disabled.
    pub kind: NoteActionKind,
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
}

/// The note under the language picker.
#[uniffi::remote(Record)]
pub struct CompatibilityView {
    /// Caption text.
    pub text: String,
    /// Orange warning.
    pub warning: bool,
    /// Fix button, for an unsupported pair with a recommendation.
    pub action: Option<NoteActionView>,
}

/// A model row's trailing control (WIN-19).
#[uniffi::remote(Enum)]
pub enum ModelRowControl {
    /// Progress bar, percentage, and "Cancel" (`cancel_download`).
    Downloading {
        /// Fraction in `[0, 1]`.
        progress: f64,
        /// "42%".
        percent: String,
        /// "Cancel".
        cancel_title: String,
    },
    /// Accent check mark: downloaded and selected.
    Selected,
    /// "Select" (`select_model`): downloaded, not selected.
    Select {
        /// "Select".
        title: String,
    },
    /// "Download & Use" (`download_and_use`): not downloaded.
    DownloadAndUse {
        /// "Download & Use".
        title: String,
    },
}

/// One catalog model (WIN-19).
#[uniffi::remote(Record)]
pub struct ModelRow {
    /// Catalog id.
    pub id: String,
    /// Name, semibold when selected.
    pub name: String,
    /// The selected model.
    pub selected: bool,
    /// Size hint.
    pub size_hint: String,
    /// "Recommended" pill for the language's recommended model.
    pub recommended: Option<String>,
    /// Description.
    pub description: String,
    /// Orange "Doesn't support <Language>".
    pub unsupported: Option<String>,
    /// Trailing control.
    pub control: ModelRowControl,
    /// Trash button tooltip ("Delete model") when the model can be deleted
    /// (`request_delete_model`).
    pub delete_tooltip: Option<String>,
}

/// An alert on the Speech page; they queue and show one at a time (WIN-20).
#[uniffi::remote(Record)]
pub struct SpeechAlert {
    /// Pass to `confirm_speech_alert` or `dismiss_speech_alert`.
    pub id: u64,
    /// Title.
    pub title: String,
    /// Message.
    pub message: String,
    /// Destructive confirm button ("Delete"), for a confirmation.
    pub confirm: Option<String>,
    /// "OK", or "Cancel" next to a confirm button.
    pub dismiss: String,
}

/// The Speech page view model.
#[uniffi::remote(Record)]
pub struct SpeechPage {
    /// "Language".
    pub language_label: String,
    /// "Automatic", then every language; entries mark the divider.
    pub languages: Vec<LanguageOption>,
    /// The selected language code.
    pub selected_language: String,
    /// The compatibility note.
    pub compatibility: Option<CompatibilityView>,
    /// One row per catalog model.
    pub models: Vec<ModelRow>,
    /// The alert to show now, if any.
    pub alert: Option<SpeechAlert>,
}

/// The API key field (CLN-25). `Debug` never prints the key (CLN-8).
#[uniffi::remote(Record)]
pub struct ApiKeyFieldView {
    /// "OpenRouter API key".
    pub placeholder: String,
    /// The field text as the core knows it. Replace the field's contents
    /// only when `revision` changes (the stored key finished loading), so
    /// typing is never overwritten.
    pub text: String,
    /// Bumped when the core replaces `text`.
    pub revision: u64,
    /// The stored key is loading.
    pub loading: bool,
    /// Inline Keychain error in red, e.g. "Keychain read failed (OSStatus
    /// -25293)." (CLN-24).
    pub error: Option<String>,
}

/// One provider entry in the sheet's sidebar.
#[uniffi::remote(Record)]
pub struct CleanupProviderOption {
    /// Value for `select_dialog_provider`; also names its bundled logo,
    /// `<id>.svg` or `<id>-light.svg` and `<id>-dark.svg`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// An SF Symbol to show when the provider has no bundled logo.
    pub system_symbol: Option<String>,
}

/// One model row in the sheet.
#[uniffi::remote(Record)]
pub struct CleanupModelOption {
    /// Value for `select_cleanup_model`.
    pub slug: String,
    /// Display name.
    pub name: String,
}

/// A titled run of model rows.
#[uniffi::remote(Record)]
pub struct CleanupModelGroup {
    /// "Recommended", "Custom", or "All OpenRouter Models".
    pub title: String,
    /// The rows, in display order.
    pub models: Vec<CleanupModelOption>,
}

/// A finished test (CLN-27), selectable.
#[uniffi::remote(Record)]
pub struct CleanupTestResult {
    /// "Said: <sample>".
    pub said: String,
    /// "Result: <output>".
    pub result: String,
}

/// The model sheet: pick a provider, set its key, and click the model to
/// use.
#[uniffi::remote(Record)]
pub struct ProvidersDialog {
    /// "Choose Model".
    pub title: String,
    /// The sidebar, in order: every provider this Mac offers.
    pub providers: Vec<CleanupProviderOption>,
    /// The provider whose pane shows.
    pub selected_provider: String,
    /// The provider's API key field; absent for a provider that uses no
    /// key.
    pub key: Option<ApiKeyFieldView>,
    /// Why the provider cannot run now, e.g. "Turn on Apple Intelligence in
    /// System Settings…"; shown above its models.
    pub status_message: Option<String>,
    /// "Models".
    pub models_label: String,
    /// The provider's models: curated, then the model in use when it is a
    /// typed ID, then the rest of the provider's list by name. Empty groups
    /// are left out.
    pub model_groups: Vec<CleanupModelGroup>,
    /// The model in use, when it belongs to this provider; clicking a row
    /// sends `select_cleanup_model`.
    pub selected_model: Option<String>,
    /// Placeholder for the one field above the list. Its text filters rows
    /// by name or slug in the shell; text that matches no slug offers a
    /// row that sends it to `select_cleanup_model`.
    pub search_placeholder: String,
    /// Caption on that row, under the typed ID.
    pub use_id_caption: String,
    /// "Refresh Models"; absent for a provider with one fixed model, which
    /// also hides the search field; disabled while a listing runs.
    pub refresh_models: Option<ButtonView>,
    /// A listing is running: show a spinner next to the refresh button and,
    /// with `loading_title`, a row above the list.
    pub models_loading: bool,
    /// "Loading models…".
    pub loading_title: String,
    /// Why the list is missing, under the search field: a failed listing
    /// or, for a provider that lists only with a key, a request for one.
    pub models_error: Option<String>,
    /// "Done"; sends `close_providers_dialog`.
    pub done_title: String,
}

/// What shows while post-processing is on (CLN-26).
#[uniffi::remote(Record)]
pub struct PostProcessingDetails {
    /// "Model"; clicking the row sends `open_providers_dialog`.
    pub model_label: String,
    /// The provider in use, e.g. "OpenRouter".
    pub provider_name: String,
    /// The model in use (CLN-21), by name.
    pub model_name: String,
    /// The model sheet, while open.
    pub dialog: Option<ProvidersDialog>,
    /// The privacy caption.
    pub caption: String,
    /// "Test with Sample Text"; disabled while a test runs.
    pub test_button: ButtonView,
    /// Show a small spinner next to the button.
    pub test_running: bool,
    /// The last finished test.
    pub test_result: Option<CleanupTestResult>,
    /// "Test Failed" alert; dismiss with `dismiss_cleanup_test_error`.
    pub test_error: Option<AlertView>,
}

/// The Post-Processing page view model.
#[uniffi::remote(Record)]
pub struct PostProcessingPage {
    /// "Post-process transcripts".
    pub enabled: ToggleView,
    /// Shown only while the toggle is on.
    pub details: Option<PostProcessingDetails>,
}

/// One rule row (REP-9), with the trigger text exactly as typed.
#[uniffi::remote(Record)]
pub struct ReplacementRowView {
    /// Pass to the row commands.
    pub id: String,
    /// Comma-separated triggers, raw.
    pub triggers: String,
    /// Replacement text.
    pub replacement: String,
}

/// The Replacements page view model.
#[uniffi::remote(Record)]
pub struct ReplacementsPage {
    /// "Instant Replacement".
    pub title: String,
    /// The empty-state caption when there are no rows (REP-8).
    pub empty_caption: Option<String>,
    /// Rows in order.
    pub rows: Vec<ReplacementRowView>,
    /// Bumped when rows are reloaded, added, or deleted, never on typing:
    /// rebind text fields only when it changes.
    pub revision: u64,
    /// "Replace".
    pub replace_label: String,
    /// "With".
    pub with_label: String,
    /// "greendale, greendale cc".
    pub triggers_placeholder: String,
    /// "Greendale Community College".
    pub replacement_placeholder: String,
    /// "Delete Rule".
    pub delete_title: String,
    /// "Add Rule".
    pub add_title: String,
    /// Footer.
    pub footer: String,
}

/// One "Keep for" option (HIS-13).
#[uniffi::remote(Record)]
pub struct RetentionOption {
    /// The option.
    pub retention: HistoryRetention,
    /// "1 Week", "1 Month", or "Forever".
    pub title: String,
}

/// A section header (HIS-21). The shell localizes dates.
#[uniffi::remote(Enum)]
pub enum DayHeading {
    /// "Today" or "Yesterday".
    Relative {
        /// The text.
        text: String,
    },
    /// Any other local day: weekday, month, and day within the current year
    /// ("Monday, October 27"), month, day, and year otherwise ("December 3,
    /// 2025").
    Date {
        /// Year.
        year: i32,
        /// Month, 1..=12.
        month: u8,
        /// Day of month, 1..=31.
        day: u8,
        /// The date falls in the current year: show the weekday, not the
        /// year.
        in_current_year: bool,
    },
}

/// A context menu item (HIS-24); dividers have an empty title.
#[uniffi::remote(Record)]
pub struct HistoryMenuItem {
    /// What it does.
    pub item: ContextMenuItem,
    /// Title; empty for a divider.
    pub title: String,
}

/// One entry row (HIS-22, HIS-23).
#[uniffi::remote(Record)]
pub struct HistoryRow {
    /// Pass to the entry commands.
    pub id: String,
    /// When it was saved; the shell shows the localized short time.
    pub time: SystemTime,
    /// The rest of the details line after the time: "<app> · <m:ss> ·
    /// Recording", the app and "Recording" only when present.
    pub details: String,
    /// The app identifier for the row icon.
    pub app_id: Option<String>,
    /// The trimmed transcript, or `None` to show `no_text_label` in italics.
    pub transcript: Option<String>,
    /// The raw transcript for the "Original" section (HIS-23).
    pub original: Option<String>,
    /// The entry has a recording.
    pub has_audio: bool,
    /// Context menu, top to bottom.
    pub menu: Vec<HistoryMenuItem>,
}

/// One day of entries, newest first.
#[uniffi::remote(Record)]
pub struct HistorySection {
    /// Header.
    pub heading: DayHeading,
    /// Rows.
    pub rows: Vec<HistoryRow>,
}

/// The page body (HIS-21, HIS-26).
#[uniffi::remote(Enum)]
pub enum HistoryContent {
    /// An empty state with its copy.
    Empty {
        /// Headline.
        title: String,
        /// Description.
        message: String,
        /// "Turn On History" (`set_history_enabled(true)`), when offered.
        action: Option<String>,
    },
    /// Nothing matches: the platform's standard "no results for
    /// '<query>'" view.
    NoSearchResults {
        /// The query.
        query: String,
    },
    /// Matching entries by day.
    Sections {
        /// Sections, newest day first.
        sections: Vec<HistorySection>,
    },
}

/// A destructive confirmation (HIS-17, HIS-20). Confirm with
/// `confirm_history_dialog`, cancel with `cancel_history_dialog`.
#[uniffi::remote(Record)]
pub struct HistoryDialog {
    /// Title.
    pub title: String,
    /// Note.
    pub message: String,
    /// Destructive button.
    pub confirm_title: String,
    /// "Cancel".
    pub cancel_title: String,
}

/// The History page view model.
#[uniffi::remote(Record)]
pub struct HistoryPage {
    /// "Save transcripts".
    pub save_transcripts: ToggleView,
    /// "Keep for".
    pub retention_label: String,
    /// The options.
    pub retention_options: Vec<RetentionOption>,
    /// The selected option.
    pub retention: HistoryRetention,
    /// The picker is enabled (history on).
    pub retention_enabled: bool,
    /// "Keep audio recordings".
    pub keep_audio: ToggleView,
    /// Caption under the bar (HIS-18).
    pub caption: String,
    /// "Search transcripts".
    pub search_prompt: String,
    /// The current query.
    pub query: String,
    /// "Clear History…", disabled when empty.
    pub clear_button: ButtonView,
    /// Its tooltip.
    pub clear_tooltip: String,
    /// The body.
    pub content: HistoryContent,
    /// A confirmation to show.
    pub dialog: Option<HistoryDialog>,
    /// "History Error" alert; dismiss with `dismiss_history_error`.
    pub error: Option<AlertView>,
    /// The entry whose copy button shows the checkmark (for 1.5 s).
    pub copied_entry: Option<String>,
    /// "Copy".
    pub copy_label: String,
    /// "Copied".
    pub copied_label: String,
    /// "No text".
    pub no_text_label: String,
    /// "Original".
    pub original_label: String,
}

/// A catalog model's credit (WIN-21).
#[uniffi::remote(Record)]
pub struct AboutModel {
    /// Display name.
    pub name: String,
    /// Attribution line.
    pub attribution: String,
    /// License name, the link's title.
    pub license_name: String,
    /// License URL.
    pub license_url: String,
}

/// A third-party component (WIN-21).
#[uniffi::remote(Record)]
pub struct AboutLibrary {
    /// Name.
    pub name: String,
    /// License.
    pub license: String,
    /// Project URL.
    pub url: String,
}

/// The About page view model.
#[uniffi::remote(Record)]
pub struct AboutPage {
    /// "Onda".
    pub app_name: String,
    /// "Version <x.y.z>".
    pub version: String,
    /// The tagline.
    pub tagline: String,
    /// "Models".
    pub models_title: String,
    /// Every catalog model.
    pub models: Vec<AboutModel>,
    /// "Libraries".
    pub libraries_title: String,
    /// Third-party components.
    pub libraries: Vec<AboutLibrary>,
}

/// One notice button, left to right.
#[uniffi::remote(Record)]
pub struct NoticeButtonView {
    /// Title.
    pub title: String,
    /// Pass to `update_notice_choice`.
    pub choice: NoticeChoice,
    /// Escape or Return; Return is also the default, prominent button.
    pub shortcut: Option<NoticeShortcut>,
}

/// The update notice view model. Closing the window counts as "Not Now".
#[uniffi::remote(Record)]
pub struct UpdateNotice {
    /// "Onda was updated".
    pub title: String,
    /// Selectable body; the new model name is Markdown bold (`**...**`).
    pub body: String,
    /// "Not Now", "Choose Another", "Download".
    pub buttons: Vec<NoticeButtonView>,
}

/// The segmented progress bar (ONB-1).
#[uniffi::remote(Record)]
pub struct OnboardingProgress {
    /// Capsules.
    pub total: u32,
    /// Capsules filled, up to and including the current step.
    pub filled: u32,
}

/// One language card (ONB-4).
#[uniffi::remote(Record)]
pub struct LanguageCardView {
    /// Pass to `onboarding_select_language`.
    pub code: String,
    /// Display name.
    pub name: String,
    /// Code in capitals.
    pub code_label: String,
    /// Flag emoji, or 🌐.
    pub flag: String,
    /// Check mark and accent border.
    pub selected: bool,
}

/// One permission card (ONB-6).
#[uniffi::remote(Record)]
pub struct PermissionCardView {
    /// Title.
    pub title: String,
    /// Detail line; none once granted.
    pub detail: Option<String>,
    /// Show `ready_label` in green.
    pub granted: bool,
    /// "Ready".
    pub ready_label: String,
    /// Fix button (`fix_permission`).
    pub button: Option<PermissionFixButton>,
}

/// One hotkey preset card (ONB-7).
#[uniffi::remote(Record)]
pub struct PresetCardView {
    /// Pass to `onboarding_select_hotkey`.
    pub preset: HotkeyPreset,
    /// Keycaps.
    pub keycaps: Vec<String>,
    /// Detail, or why the preset is unavailable.
    pub detail: String,
    /// Clickable; dimmed when not.
    pub enabled: bool,
    /// Check mark and accent border.
    pub selected: bool,
    /// The Fn warning while selected (HOT-35).
    pub warning: Option<String>,
}

/// A model card's action (ONB-8).
#[uniffi::remote(Enum)]
pub enum ModelCardAction {
    /// Progress bar, label, and "Cancel" (`cancel_download`).
    Downloading {
        /// Fraction in `[0, 1]`.
        progress: f64,
        /// "Downloading — N%".
        label: String,
        /// "Cancel".
        cancel_title: String,
    },
    /// Green "Selected".
    Selected {
        /// "Selected".
        title: String,
    },
    /// "Use This Model" (`onboarding_choose_model`).
    UseThisModel {
        /// "Use This Model".
        title: String,
    },
    /// "Download" (`onboarding_download_model`): selects and downloads.
    Download {
        /// "Download".
        title: String,
    },
}

/// One model card or compact row (ONB-8).
#[uniffi::remote(Record)]
pub struct ModelCardView {
    /// Catalog id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Size hint.
    pub size_hint: String,
    /// "Recommended" pill.
    pub recommended: Option<String>,
    /// Accent border.
    pub selected: bool,
    /// Action.
    pub action: ModelCardAction,
}

/// The models behind "Show other models" (ONB-8).
#[uniffi::remote(Record)]
pub struct OtherModelsView {
    /// "Show other models" / "Hide other models"
    /// (`onboarding_toggle_other_models`).
    pub toggle_title: String,
    /// Compact rows; empty while collapsed.
    pub rows: Vec<ModelCardView>,
}

/// The current step's content.
#[uniffi::remote(Enum)]
pub enum OnboardingContent {
    /// ONB-3.
    Welcome {
        /// "Welcome to Onda".
        title: String,
        /// "On-device dictation for any app."
        subtitle: String,
    },
    /// ONB-4.
    Language {
        /// "What language do you speak?"
        title: String,
        /// "Search languages".
        search_placeholder: String,
        /// The search query (`onboarding_search_languages`).
        query: String,
        /// "Show All" / "Popular Only"
        /// (`onboarding_toggle_all_languages`); hidden while searching.
        toggle_title: Option<String>,
        /// Cards, three columns.
        cards: Vec<LanguageCardView>,
    },
    /// ONB-6.
    Permissions {
        /// "Allow access".
        title: String,
        /// Microphone card.
        microphone: PermissionCardView,
        /// Typing Access card.
        typing_access: PermissionCardView,
        /// "Use the clipboard instead" (`onboarding_use_clipboard`).
        clipboard_link: Option<String>,
    },
    /// ONB-7.
    Hotkey {
        /// "Choose a hotkey".
        title: String,
        /// The activation mode's description.
        subtitle: String,
        /// Preset cards.
        presets: Vec<PresetCardView>,
        /// The "Another shortcut" card is selected.
        custom_selected: bool,
        /// The current shortcut's keycaps when it is not a preset.
        custom_keycaps: Option<Vec<String>>,
        /// "Another shortcut" when it is a preset.
        custom_label: Option<String>,
        /// "Open Keyboard Settings" next to the Fn warning.
        open_keyboard_settings_title: String,
    },
    /// ONB-8.
    SpeechModel {
        /// "Choose a speech model".
        title: String,
        /// Full cards: the recommended model and built-in engines.
        cards: Vec<ModelCardView>,
        /// Everything else.
        other_models: Option<OtherModelsView>,
        /// "Download failed: <error>" in red.
        error: Option<String>,
    },
    /// ONB-9.
    Ready {
        /// "You're all set".
        title: String,
        /// "Press".
        prompt_prefix: String,
        /// The hotkey's keycaps.
        keycaps: Vec<String>,
        /// "and start talking."
        prompt_suffix: String,
        /// Try-it placeholder.
        try_it_placeholder: String,
        /// "Transcripts are saved on this Mac for 1 month." when history is
        /// on.
        retention_note: Option<String>,
    },
}

/// The onboarding window view model.
#[uniffi::remote(Record)]
pub struct OnboardingView {
    /// Current step.
    pub step: OnboardingStep,
    /// Slide in from the trailing edge (forward) or the leading edge.
    pub forward: bool,
    /// Segmented progress; hidden on Welcome.
    pub progress: Option<OnboardingProgress>,
    /// "Back" is shown (`onboarding_back`).
    pub back_visible: bool,
    /// "Back".
    pub back_title: String,
    /// Why the primary button is disabled.
    pub hint: Option<String>,
    /// "Get Started", "Continue", or "Start Dictating"
    /// (`onboarding_continue`); the default action on Return.
    pub primary_title: String,
    /// The primary button is enabled.
    pub primary_enabled: bool,
    /// The step's content.
    pub content: OnboardingContent,
}
