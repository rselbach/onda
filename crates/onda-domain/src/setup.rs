//! Setup readiness: the Getting Started checklist (WIN-5), the General page
//! permission rows (WIN-8), the missing-permission rule (UI-8), and where
//! setup guidance sends the user (DICT-5).

use crate::privacy_naming::PrivacyNaming;
use crate::types::{MicrophonePermission, OutputMode};

/// A permission dictation needs that the OS does not grant (UI-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissingPermission {
    /// Microphone access was denied or is restricted.
    Microphone,
    /// Accessibility is needed and not granted.
    Accessibility,
}

/// Whether Accessibility is needed: type-into-app output or a lone-modifier
/// hotkey (UI-8).
pub fn accessibility_needed(output_mode: OutputMode, hotkey_is_lone_modifier: bool) -> bool {
    output_mode == OutputMode::Type || hotkey_is_lone_modifier
}

/// The permission to warn about before the user wastes a dictation (UI-8).
///
/// A never-asked microphone is not missing: the first dictation's system
/// prompt is the fix. The microphone wins over Accessibility.
pub fn missing_permission(
    microphone: MicrophonePermission,
    accessibility_granted: bool,
    accessibility_needed: bool,
) -> Option<MissingPermission> {
    match microphone {
        MicrophonePermission::Denied | MicrophonePermission::Restricted => {
            return Some(MissingPermission::Microphone);
        }
        MicrophonePermission::Granted | MicrophonePermission::NotDetermined => {}
    }
    if accessibility_needed && !accessibility_granted {
        return Some(MissingPermission::Accessibility);
    }
    None
}

/// One row of the setup checklist, in display order (WIN-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetupStepId {
    /// Microphone permission.
    Microphone,
    /// Accessibility permission.
    Accessibility,
    /// The selected speech model.
    SpeechModel,
}

/// Where a checklist row stands. Also picks the row icon: green check,
/// orange exclamation, or blue down-arrow (WIN-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetupStepStatus {
    /// Nothing left to do.
    Complete,
    /// The user has to act.
    NeedsAction,
    /// Onda is working on it (a model download).
    InProgress,
}

/// What a checklist row's button does (WIN-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetupAction {
    /// Request the microphone; open privacy settings if denied.
    RequestMicrophone,
    /// Trigger the Accessibility prompt, then open its settings pane.
    OpenAccessibilitySettings,
    /// Download [`base_model_to_download`].
    DownloadBaseModel,
}

/// A checklist row's button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupButton {
    /// Button title.
    pub title: &'static str,
    /// What the button does.
    pub action: SetupAction,
}

/// One checklist row (WIN-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupStep {
    /// Which requirement this row covers.
    pub id: SetupStepId,
    /// Row title.
    pub title: &'static str,
    /// Row status and icon.
    pub status: SetupStepStatus,
    /// Message for the current status, including the complete one.
    pub message: &'static str,
    /// Button, only when the row needs action.
    pub button: Option<SetupButton>,
}

impl SetupStep {
    /// The message to render: rows show one only while not complete.
    pub fn visible_message(&self) -> Option<&'static str> {
        match self.status {
            SetupStepStatus::Complete => None,
            SetupStepStatus::NeedsAction | SetupStepStatus::InProgress => Some(self.message),
        }
    }
}

/// What the checklist is computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupChecklistInput {
    /// Current microphone permission.
    pub microphone: MicrophonePermission,
    /// Accessibility is granted.
    pub accessibility_granted: bool,
    /// Accessibility is required by the current settings.
    pub accessibility_required: bool,
    /// What the OS calls the Accessibility permission.
    pub privacy_naming: PrivacyNaming,
    /// The selected model's files are installed.
    pub selected_model_downloaded: bool,
    /// Any model download is running.
    pub any_download_running: bool,
}

/// The Getting Started setup checklist (WIN-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupChecklist {
    /// Microphone, Accessibility, Speech Model, in that order.
    pub steps: [SetupStep; 3],
}

/// Shown above the checklist once every row is complete.
pub const READY_LINE: &str = "Onda is ready";

impl SetupChecklist {
    /// Builds the checklist.
    pub fn resolve(input: SetupChecklistInput) -> Self {
        Self {
            steps: [
                microphone_step(input.microphone),
                accessibility_step(
                    input.accessibility_granted,
                    input.accessibility_required,
                    input.privacy_naming,
                ),
                model_step(input.selected_model_downloaded, input.any_download_running),
            ],
        }
    }

    /// Every row is complete.
    pub fn is_complete(&self) -> bool {
        self.steps
            .iter()
            .all(|step| step.status == SetupStepStatus::Complete)
    }

    /// The green "Onda is ready" line, when every row is complete.
    pub fn ready_line(&self) -> Option<&'static str> {
        self.is_complete().then_some(READY_LINE)
    }
}

fn microphone_step(microphone: MicrophonePermission) -> SetupStep {
    let (status, message, button) = match microphone {
        MicrophonePermission::Granted => (
            SetupStepStatus::Complete,
            "Onda can record audio for dictation.",
            None,
        ),
        MicrophonePermission::NotDetermined | MicrophonePermission::Denied => (
            SetupStepStatus::NeedsAction,
            "Allow microphone access so Onda can record speech.",
            Some(SetupButton {
                title: "Grant Access",
                action: SetupAction::RequestMicrophone,
            }),
        ),
        // a device policy restricts it; no setting the user can change
        MicrophonePermission::Restricted => (
            SetupStepStatus::NeedsAction,
            "Restricted on this Mac, so Onda can't record.",
            None,
        ),
    };
    SetupStep {
        id: SetupStepId::Microphone,
        title: "Microphone",
        status,
        message,
        button,
    }
}

fn accessibility_step(granted: bool, required: bool, naming: PrivacyNaming) -> SetupStep {
    let (status, message, button) = match (required, granted) {
        (false, _) => (
            SetupStepStatus::Complete,
            "Not required while output is set to copy to clipboard.",
            None,
        ),
        (true, true) => (
            SetupStepStatus::Complete,
            "Onda can type transcriptions into the active app.",
            None,
        ),
        (true, false) => (
            SetupStepStatus::NeedsAction,
            naming.allow_message(),
            Some(SetupButton {
                title: "Open Settings",
                action: SetupAction::OpenAccessibilitySettings,
            }),
        ),
    };
    SetupStep {
        id: SetupStepId::Accessibility,
        title: naming.name(),
        status,
        message,
        button,
    }
}

fn model_step(selected_model_downloaded: bool, any_download_running: bool) -> SetupStep {
    let (status, message, button) = match (selected_model_downloaded, any_download_running) {
        (true, _) => (
            SetupStepStatus::Complete,
            "The selected transcription model is installed.",
            None,
        ),
        (false, true) => (
            SetupStepStatus::InProgress,
            "Onda is downloading a local transcription model.",
            None,
        ),
        (false, false) => (
            SetupStepStatus::NeedsAction,
            "Download a local model before dictating.",
            Some(SetupButton {
                title: "Download Base Model",
                action: SetupAction::DownloadBaseModel,
            }),
        ),
    };
    SetupStep {
        id: SetupStepId::SpeechModel,
        title: "Speech Model",
        status,
        message,
        button,
    }
}

/// The model "Download Base Model" fetches: the selected model when it is
/// downloadable, else the language's recommended model (WIN-5).
pub fn base_model_to_download<'a>(
    selected_model_id: &'a str,
    selected_is_downloadable: bool,
    recommended_model_id: &'a str,
) -> &'a str {
    if selected_is_downloadable {
        return selected_model_id;
    }
    recommended_model_id
}

/// Placeholder for the "Try It" text fields (WIN-6, ONB-9).
pub fn try_it_placeholder(output_mode: OutputMode) -> &'static str {
    match output_mode {
        OutputMode::Type => "Click here and try it.",
        OutputMode::Clipboard => "Click here, dictate, then paste with ⌘V.",
    }
}

/// Label shown in place of a fix button on a granted permission row.
pub const GRANTED_LABEL: &str = "Granted";

/// What a permission row's fix button does (WIN-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionFix {
    /// Show the system microphone prompt.
    RequestMicrophone,
    /// Open the microphone privacy pane.
    OpenMicrophoneSettings,
    /// Trigger the Accessibility prompt, then open its pane.
    OpenAccessibilitySettings,
}

impl PermissionFix {
    /// Button title.
    pub fn title(self) -> &'static str {
        match self {
            Self::RequestMicrophone => "Allow",
            Self::OpenMicrophoneSettings | Self::OpenAccessibilitySettings => "Open Settings",
        }
    }
}

/// One General page permission row (WIN-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PermissionRow {
    /// Row title.
    pub title: &'static str,
    /// Show [`GRANTED_LABEL`] instead of a button.
    pub granted: bool,
    /// Render the detail line in the warning color.
    pub needs_attention: bool,
    /// What the permission is for, or why it is missing.
    pub detail: &'static str,
    /// Fix button, when one helps.
    pub fix: Option<PermissionFix>,
}

/// The microphone row (WIN-8).
pub fn microphone_row(microphone: MicrophonePermission) -> PermissionRow {
    let (granted, needs_attention, detail, fix) = match microphone {
        MicrophonePermission::Granted => {
            (true, false, "Onda can hear you while you dictate.", None)
        }
        MicrophonePermission::NotDetermined => (
            false,
            true,
            "Dictation needs it to record.",
            Some(PermissionFix::RequestMicrophone),
        ),
        MicrophonePermission::Denied => (
            false,
            true,
            "Turned off, so Onda can't record.",
            Some(PermissionFix::OpenMicrophoneSettings),
        ),
        MicrophonePermission::Restricted => (
            false,
            true,
            "Restricted on this Mac, so Onda can't record.",
            None,
        ),
    };
    PermissionRow {
        title: "Microphone",
        granted,
        needs_attention,
        detail,
        fix,
    }
}

/// The Accessibility row (WIN-8), titled with the OS's name for the
/// permission. `needed` comes from [`accessibility_needed`].
pub fn accessibility_row(granted: bool, needed: bool, naming: PrivacyNaming) -> PermissionRow {
    let (needs_attention, detail, fix) = match (granted, needed) {
        (true, _) => (
            false,
            "Onda can type into apps and use lone-key hotkeys like Right Option.",
            None,
        ),
        (false, true) => (
            true,
            "Needed to type into apps and for lone-key hotkeys like Right Option.",
            Some(PermissionFix::OpenAccessibilitySettings),
        ),
        (false, false) => (
            false,
            "Only needed to type into apps or for lone-key hotkeys like Right Option.",
            Some(PermissionFix::OpenAccessibilitySettings),
        ),
    };
    PermissionRow {
        title: naming.name(),
        granted,
        needs_attention,
        detail,
        fix,
    }
}

/// Where setup guidance sends the user after a failed start (DICT-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetupGuidance {
    /// Show onboarding.
    Onboarding,
    /// Open the main window on Getting Started.
    GettingStarted,
    /// Open the main window on Speech.
    Speech,
}

/// Picks setup guidance (DICT-5): a declined model download goes to Speech;
/// pending first-run onboarding shows onboarding; an incomplete checklist
/// opens Getting Started; otherwise nothing.
pub fn setup_guidance(
    first_run_pending: bool,
    checklist_complete: bool,
    declined_model_download: bool,
) -> Option<SetupGuidance> {
    if declined_model_download {
        return Some(SetupGuidance::Speech);
    }
    if first_run_pending {
        return Some(SetupGuidance::Onboarding);
    }
    if checklist_complete {
        return None;
    }
    Some(SetupGuidance::GettingStarted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MicrophonePermission::{Denied, Granted, NotDetermined, Restricted};

    fn input() -> SetupChecklistInput {
        SetupChecklistInput {
            microphone: Granted,
            accessibility_granted: true,
            accessibility_required: true,
            privacy_naming: PrivacyNaming::Accessibility,
            selected_model_downloaded: true,
            any_download_running: false,
        }
    }

    #[test]
    fn ui_8_missing_permission() {
        struct Case {
            name: &'static str,
            microphone: MicrophonePermission,
            granted: bool,
            needed: bool,
            want: Option<MissingPermission>,
        }
        let cases = [
            Case {
                name: "all granted",
                microphone: Granted,
                granted: true,
                needed: true,
                want: None,
            },
            Case {
                name: "microphone denied",
                microphone: Denied,
                granted: true,
                needed: true,
                want: Some(MissingPermission::Microphone),
            },
            Case {
                name: "microphone restricted",
                microphone: Restricted,
                granted: true,
                needed: false,
                want: Some(MissingPermission::Microphone),
            },
            Case {
                name: "microphone never asked",
                microphone: NotDetermined,
                granted: true,
                needed: true,
                want: None,
            },
            Case {
                name: "microphone wins over accessibility",
                microphone: Denied,
                granted: false,
                needed: true,
                want: Some(MissingPermission::Microphone),
            },
            Case {
                name: "accessibility revoked",
                microphone: Granted,
                granted: false,
                needed: true,
                want: Some(MissingPermission::Accessibility),
            },
            Case {
                name: "accessibility not needed",
                microphone: Granted,
                granted: false,
                needed: false,
                want: None,
            },
            Case {
                name: "never asked and accessibility revoked",
                microphone: NotDetermined,
                granted: false,
                needed: true,
                want: Some(MissingPermission::Accessibility),
            },
        ];
        for tc in cases {
            let got = missing_permission(tc.microphone, tc.granted, tc.needed);
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn ui_8_accessibility_needed() {
        struct Case {
            name: &'static str,
            output: OutputMode,
            lone_modifier: bool,
            want: bool,
        }
        let cases = [
            Case {
                name: "type with combo",
                output: OutputMode::Type,
                lone_modifier: false,
                want: true,
            },
            Case {
                name: "clipboard with combo",
                output: OutputMode::Clipboard,
                lone_modifier: false,
                want: false,
            },
            Case {
                name: "clipboard with lone modifier",
                output: OutputMode::Clipboard,
                lone_modifier: true,
                want: true,
            },
            Case {
                name: "type with lone modifier",
                output: OutputMode::Type,
                lone_modifier: true,
                want: true,
            },
        ];
        for tc in cases {
            assert_eq!(
                accessibility_needed(tc.output, tc.lone_modifier),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn win_5_checklist_statuses_and_actions() {
        struct Case {
            name: &'static str,
            input: SetupChecklistInput,
            want_status: [SetupStepStatus; 3],
            want_actions: [Option<SetupAction>; 3],
            want_complete: bool,
        }
        use SetupStepStatus::{Complete, InProgress, NeedsAction};
        let cases = [
            Case {
                name: "ready when permissions and model are available",
                input: input(),
                want_status: [Complete, Complete, Complete],
                want_actions: [None, None, None],
                want_complete: true,
            },
            Case {
                name: "accessibility complete when clipboard mode does not require it",
                input: SetupChecklistInput {
                    accessibility_granted: false,
                    accessibility_required: false,
                    ..input()
                },
                want_status: [Complete, Complete, Complete],
                want_actions: [None, None, None],
                want_complete: true,
            },
            Case {
                name: "missing requirements expose actions",
                input: SetupChecklistInput {
                    microphone: NotDetermined,
                    accessibility_granted: false,
                    selected_model_downloaded: false,
                    ..input()
                },
                want_status: [NeedsAction, NeedsAction, NeedsAction],
                want_actions: [
                    Some(SetupAction::RequestMicrophone),
                    Some(SetupAction::OpenAccessibilitySettings),
                    Some(SetupAction::DownloadBaseModel),
                ],
                want_complete: false,
            },
            Case {
                name: "restricted microphone offers no fix",
                input: SetupChecklistInput {
                    microphone: Restricted,
                    ..input()
                },
                want_status: [NeedsAction, Complete, Complete],
                want_actions: [None, None, None],
                want_complete: false,
            },
            Case {
                name: "denied microphone still offers Grant Access",
                input: SetupChecklistInput {
                    microphone: Denied,
                    ..input()
                },
                want_status: [NeedsAction, Complete, Complete],
                want_actions: [Some(SetupAction::RequestMicrophone), None, None],
                want_complete: false,
            },
            Case {
                name: "downloading model is in progress",
                input: SetupChecklistInput {
                    selected_model_downloaded: false,
                    any_download_running: true,
                    ..input()
                },
                want_status: [Complete, Complete, InProgress],
                want_actions: [None, None, None],
                want_complete: false,
            },
        ];
        for tc in cases {
            let got = SetupChecklist::resolve(tc.input);
            let ids: Vec<_> = got.steps.iter().map(|s| s.id).collect();
            assert_eq!(
                ids,
                [
                    SetupStepId::Microphone,
                    SetupStepId::Accessibility,
                    SetupStepId::SpeechModel
                ],
                "{}",
                tc.name
            );
            let statuses: Vec<_> = got.steps.iter().map(|s| s.status).collect();
            assert_eq!(statuses, tc.want_status, "{}", tc.name);
            let actions: Vec<_> = got
                .steps
                .iter()
                .map(|s| s.button.map(|b| b.action))
                .collect();
            assert_eq!(actions, tc.want_actions, "{}", tc.name);
            assert_eq!(got.is_complete(), tc.want_complete, "{}", tc.name);
            assert_eq!(got.ready_line().is_some(), tc.want_complete, "{}", tc.name);
        }
    }

    #[test]
    fn win_5_checklist_copy() {
        struct Case {
            name: &'static str,
            input: SetupChecklistInput,
            step: usize,
            want_message: &'static str,
            want_visible: bool,
            want_button: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "microphone granted",
                input: input(),
                step: 0,
                want_message: "Onda can record audio for dictation.",
                want_visible: false,
                want_button: None,
            },
            Case {
                name: "microphone missing",
                input: SetupChecklistInput {
                    microphone: NotDetermined,
                    ..input()
                },
                step: 0,
                want_message: "Allow microphone access so Onda can record speech.",
                want_visible: true,
                want_button: Some("Grant Access"),
            },
            Case {
                name: "accessibility not required",
                input: SetupChecklistInput {
                    accessibility_required: false,
                    ..input()
                },
                step: 1,
                want_message: "Not required while output is set to copy to clipboard.",
                want_visible: false,
                want_button: None,
            },
            Case {
                name: "accessibility granted",
                input: input(),
                step: 1,
                want_message: "Onda can type transcriptions into the active app.",
                want_visible: false,
                want_button: None,
            },
            Case {
                name: "accessibility missing",
                input: SetupChecklistInput {
                    accessibility_granted: false,
                    ..input()
                },
                step: 1,
                want_message: "Allow Accessibility so Onda can type into other apps.",
                want_visible: true,
                want_button: Some("Open Settings"),
            },
            Case {
                name: "model installed",
                input: input(),
                step: 2,
                want_message: "The selected transcription model is installed.",
                want_visible: false,
                want_button: None,
            },
            Case {
                name: "model downloading",
                input: SetupChecklistInput {
                    selected_model_downloaded: false,
                    any_download_running: true,
                    ..input()
                },
                step: 2,
                want_message: "Onda is downloading a local transcription model.",
                want_visible: true,
                want_button: None,
            },
            Case {
                name: "model missing",
                input: SetupChecklistInput {
                    selected_model_downloaded: false,
                    ..input()
                },
                step: 2,
                want_message: "Download a local model before dictating.",
                want_visible: true,
                want_button: Some("Download Base Model"),
            },
        ];
        for tc in cases {
            let step = SetupChecklist::resolve(tc.input).steps[tc.step];
            assert_eq!(step.message, tc.want_message, "{}", tc.name);
            assert_eq!(
                step.visible_message().is_some(),
                tc.want_visible,
                "{}",
                tc.name
            );
            assert_eq!(step.button.map(|b| b.title), tc.want_button, "{}", tc.name);
        }
        assert_eq!(
            SetupChecklist::resolve(input()).ready_line(),
            Some("Onda is ready")
        );
    }

    #[test]
    fn win_8_microphone_row() {
        struct Case {
            name: &'static str,
            microphone: MicrophonePermission,
            want_granted: bool,
            want_attention: bool,
            want_detail: &'static str,
            want_fix: Option<PermissionFix>,
            want_button: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "granted",
                microphone: Granted,
                want_granted: true,
                want_attention: false,
                want_detail: "Onda can hear you while you dictate.",
                want_fix: None,
                want_button: None,
            },
            Case {
                name: "never asked",
                microphone: NotDetermined,
                want_granted: false,
                want_attention: true,
                want_detail: "Dictation needs it to record.",
                want_fix: Some(PermissionFix::RequestMicrophone),
                want_button: Some("Allow"),
            },
            Case {
                name: "denied",
                microphone: Denied,
                want_granted: false,
                want_attention: true,
                want_detail: "Turned off, so Onda can't record.",
                want_fix: Some(PermissionFix::OpenMicrophoneSettings),
                want_button: Some("Open Settings"),
            },
            Case {
                name: "restricted by the Mac",
                microphone: Restricted,
                want_granted: false,
                want_attention: true,
                want_detail: "Restricted on this Mac, so Onda can't record.",
                want_fix: None,
                want_button: None,
            },
        ];
        for tc in cases {
            let got = microphone_row(tc.microphone);
            assert_eq!(got.title, "Microphone", "{}", tc.name);
            assert_eq!(got.granted, tc.want_granted, "{}", tc.name);
            assert_eq!(got.needs_attention, tc.want_attention, "{}", tc.name);
            assert_eq!(got.detail, tc.want_detail, "{}", tc.name);
            assert_eq!(got.fix, tc.want_fix, "{}", tc.name);
            assert_eq!(
                got.fix.map(PermissionFix::title),
                tc.want_button,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn win_8_accessibility_row() {
        struct Case {
            name: &'static str,
            granted: bool,
            needed: bool,
            want_attention: bool,
            want_detail: &'static str,
            want_fix: Option<PermissionFix>,
        }
        let cases = [
            Case {
                name: "granted",
                granted: true,
                needed: true,
                want_attention: false,
                want_detail: "Onda can type into apps and use lone-key hotkeys like Right Option.",
                want_fix: None,
            },
            Case {
                name: "granted but unused",
                granted: true,
                needed: false,
                want_attention: false,
                want_detail: "Onda can type into apps and use lone-key hotkeys like Right Option.",
                want_fix: None,
            },
            Case {
                name: "missing while typing into apps",
                granted: false,
                needed: true,
                want_attention: true,
                want_detail: "Needed to type into apps and for lone-key hotkeys like Right Option.",
                want_fix: Some(PermissionFix::OpenAccessibilitySettings),
            },
            Case {
                name: "missing in clipboard mode",
                granted: false,
                needed: false,
                want_attention: false,
                want_detail: "Only needed to type into apps or for lone-key hotkeys like Right Option.",
                want_fix: Some(PermissionFix::OpenAccessibilitySettings),
            },
        ];
        for tc in cases {
            let got = accessibility_row(tc.granted, tc.needed, PrivacyNaming::Accessibility);
            assert_eq!(got.title, "Accessibility", "{}", tc.name);
            assert_eq!(got.granted, tc.granted, "{}", tc.name);
            assert_eq!(got.needs_attention, tc.want_attention, "{}", tc.name);
            assert_eq!(got.detail, tc.want_detail, "{}", tc.name);
            assert_eq!(got.fix, tc.want_fix, "{}", tc.name);
        }
    }

    #[test]
    fn accessibility_rows_follow_privacy_naming() {
        struct Case {
            name: &'static str,
            naming: PrivacyNaming,
            want_title: &'static str,
            want_message: &'static str,
        }
        let cases = [
            Case {
                name: "accessibility",
                naming: PrivacyNaming::Accessibility,
                want_title: "Accessibility",
                want_message: "Allow Accessibility so Onda can type into other apps.",
            },
            Case {
                name: "device control and data access",
                naming: PrivacyNaming::DeviceControlAndDataAccess,
                want_title: "Device Control and Data Access",
                want_message: "Allow Device Control and Data Access so Onda can type into other apps.",
            },
        ];
        for tc in cases {
            let checklist = SetupChecklist::resolve(SetupChecklistInput {
                accessibility_granted: false,
                privacy_naming: tc.naming,
                ..input()
            });
            let step = checklist.steps[1];
            assert_eq!(step.title, tc.want_title, "{}", tc.name);
            assert_eq!(step.message, tc.want_message, "{}", tc.name);
            let row = accessibility_row(false, true, tc.naming);
            assert_eq!(row.title, tc.want_title, "{}", tc.name);
        }
    }

    #[test]
    fn dict_5_setup_guidance() {
        struct Case {
            name: &'static str,
            pending: bool,
            complete: bool,
            declined: bool,
            want: Option<SetupGuidance>,
        }
        let cases = [
            Case {
                name: "first run pending shows onboarding",
                pending: true,
                complete: false,
                declined: false,
                want: Some(SetupGuidance::Onboarding),
            },
            Case {
                name: "incomplete checklist opens Getting Started",
                pending: false,
                complete: false,
                declined: false,
                want: Some(SetupGuidance::GettingStarted),
            },
            Case {
                name: "complete checklist shows nothing",
                pending: false,
                complete: true,
                declined: false,
                want: None,
            },
            Case {
                name: "declined download opens Speech",
                pending: false,
                complete: true,
                declined: true,
                want: Some(SetupGuidance::Speech),
            },
            Case {
                name: "declined download wins over pending onboarding",
                pending: true,
                complete: false,
                declined: true,
                want: Some(SetupGuidance::Speech),
            },
        ];
        for tc in cases {
            assert_eq!(
                setup_guidance(tc.pending, tc.complete, tc.declined),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn win_5_base_model_to_download() {
        struct Case {
            name: &'static str,
            downloadable: bool,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "downloadable selection",
                downloadable: true,
                want: "parakeet-ultra",
            },
            Case {
                name: "built-in selection falls back to recommendation",
                downloadable: false,
                want: "parakeet-tdt-v2",
            },
        ];
        for tc in cases {
            assert_eq!(
                base_model_to_download("parakeet-ultra", tc.downloadable, "parakeet-tdt-v2"),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn win_6_try_it_placeholder() {
        struct Case {
            name: &'static str,
            mode: OutputMode,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "type",
                mode: OutputMode::Type,
                want: "Click here and try it.",
            },
            Case {
                name: "clipboard",
                mode: OutputMode::Clipboard,
                want: "Click here, dictate, then paste with ⌘V.",
            },
        ];
        for tc in cases {
            assert_eq!(try_it_placeholder(tc.mode), tc.want, "{}", tc.name);
        }
    }
}
