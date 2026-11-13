//! Onboarding wizard rules (ONB-1 to ONB-13) and first-run bookkeeping
//! (SET-4).
//!
//! [`Onboarding`] holds the wizard's own state; everything it reads from the
//! rest of the app (permissions, hotkey, model status) arrives as an
//! [`OnboardingEnv`] snapshot the caller refreshes every [`POLL_INTERVAL`]
//! and on every navigation (ONB-2).

use std::time::Duration;

use crate::privacy_naming::PrivacyNaming;
use crate::setup::{PermissionFix, try_it_placeholder};
use crate::tray::percent;
use crate::types::{HistoryRetention, MicrophonePermission, OutputMode};

/// Delay before automatic first-run onboarding appears (ONB-13).
pub const AUTOMATIC_ONBOARDING_DELAY: Duration = Duration::from_millis(750);

/// How often an open onboarding re-reads its environment (ONB-2).
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Onboarding steps in order (ONB-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardingStep {
    /// App icon and greeting (ONB-3).
    Welcome,
    /// Language pick (ONB-4).
    Language,
    /// Microphone and Typing Access (ONB-6).
    Permissions,
    /// Hotkey pick (ONB-7).
    Hotkey,
    /// Speech model pick (ONB-8).
    SpeechModel,
    /// Done, try it (ONB-9).
    Ready,
}

impl OnboardingStep {
    /// Every step, in order.
    pub const ALL: [Self; 6] = [
        Self::Welcome,
        Self::Language,
        Self::Permissions,
        Self::Hotkey,
        Self::SpeechModel,
        Self::Ready,
    ];

    fn index(self) -> usize {
        match self {
            Self::Welcome => 0,
            Self::Language => 1,
            Self::Permissions => 2,
            Self::Hotkey => 3,
            Self::SpeechModel => 4,
            Self::Ready => 5,
        }
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        self.index()
            .checked_sub(1)
            .and_then(|i| Self::ALL.get(i).copied())
    }
}

/// The two onboarding settings (SET-4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OnboardingFlags {
    /// `onboardingCompleted`.
    pub onboarding_completed: bool,
    /// `didShowFirstRunSetup`.
    pub did_show_first_run_setup: bool,
}

impl OnboardingFlags {
    /// First-run onboarding is pending while both flags are false (SET-4).
    /// Drives automatic onboarding, "Finish Setup to Dictate", and the
    /// launch-time load skip.
    pub fn first_run_pending(self) -> bool {
        !self.onboarding_completed && !self.did_show_first_run_setup
    }

    /// Finishing onboarding sets both flags.
    pub fn finished(self) -> Self {
        Self {
            onboarding_completed: true,
            did_show_first_run_setup: true,
        }
    }

    /// "Run Setup Again" clears only `onboardingCompleted` (WIN-4, SET-4).
    pub fn rerun(self) -> Self {
        Self {
            onboarding_completed: false,
            ..self
        }
    }
}

/// What to do at launch (ONB-12, ONB-13, ASR-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Show onboarding after this delay; re-check
    /// [`OnboardingFlags::first_run_pending`] when it fires.
    pub show_onboarding_after: Option<Duration>,
    /// Ask for notification permission now.
    pub request_notifications: bool,
    /// Load no model: onboarding owns the first download.
    pub skip_model_load: bool,
}

/// Decides the launch-time onboarding, notification, and load skip.
pub fn launch_plan(flags: OnboardingFlags) -> LaunchPlan {
    let pending = flags.first_run_pending();
    LaunchPlan {
        show_onboarding_after: pending.then_some(AUTOMATIC_ONBOARDING_DELAY),
        request_notifications: !pending,
        skip_model_load: pending,
    }
}

/// How onboarding was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardingOrigin {
    /// Automatically at launch (ONB-13).
    Automatic,
    /// "Run Setup Again" (WIN-4).
    RunSetupAgain,
}

/// A model's download status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelStatus {
    /// Files are installed (built-in engines always are).
    pub downloaded: bool,
    /// Download progress while downloading.
    pub download_progress: Option<f64>,
}

/// The selected model as onboarding sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectedModel<'a> {
    /// Display name.
    pub name: &'a str,
    /// Download status.
    pub status: ModelStatus,
}

/// What onboarding reads from the rest of the app (ONB-2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnboardingEnv<'a> {
    /// Microphone permission.
    pub microphone: MicrophonePermission,
    /// Accessibility ("Typing Access") is granted.
    pub accessibility_granted: bool,
    /// Current hotkey display string.
    pub hotkey_display: &'a str,
    /// The current hotkey is a lone modifier, which needs Accessibility.
    pub hotkey_needs_accessibility: bool,
    /// What the OS calls the Accessibility permission.
    pub privacy_naming: PrivacyNaming,
    /// The selected model, when it is in the catalog.
    pub selected_model: Option<SelectedModel<'a>>,
    /// The selected language's recommended model (ONB-5).
    pub recommended_model: ModelStatus,
}

/// What the caller must do after [`Onboarding::advance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingEffect {
    /// Start the recommended model's download in the background (ONB-5).
    StartRecommendedDownload,
    /// Arrived at Ready: load the chosen model unless a load is in flight
    /// (ONB-9). Every arrival asks again.
    ReachedReady,
    /// "Start Dictating": save [`OnboardingFlags::finished`], close the
    /// window, and load the model unless a load is in flight (ONB-10).
    Finish,
}

/// What picking a language changes (ONB-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePick {
    /// Save as the selected language.
    pub language: String,
    /// Save as the selected model: the language's recommendation.
    pub model_id: String,
}

/// A model download update, already filtered to the selected language's
/// compatible models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingDownloadUpdate {
    /// Started, progressed, or finished.
    Active,
    /// Failed or was cancelled.
    Failed {
        /// The user cancelled it.
        cancelled: bool,
        /// Error text, or the status text when there is none.
        error: String,
    },
}

/// What closing the onboarding window means (ONB-10 to ONB-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseOutcome {
    /// Flags to save.
    pub flags: OnboardingFlags,
    /// Load the selected model (or the declined-migration fallback, DL-14).
    pub load_model: bool,
    /// Ask for notification permission now (ONB-12).
    pub request_notifications: bool,
}

/// The onboarding wizard's own state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    previously_completed: bool,
    step: OnboardingStep,
    navigating_forward: bool,
    language: Option<String>,
    skipped_accessibility: bool,
    download_error: Option<String>,
}

impl Onboarding {
    /// Opens onboarding on Welcome with the stored (or default) language
    /// already selected (ONB-4). `flags` are read when the window opens.
    pub fn open(
        origin: OnboardingOrigin,
        flags: OnboardingFlags,
        stored_language: Option<String>,
    ) -> Self {
        let previously_completed = match origin {
            OnboardingOrigin::Automatic => false,
            OnboardingOrigin::RunSetupAgain => {
                flags.onboarding_completed || flags.did_show_first_run_setup
            }
        };
        Self {
            previously_completed,
            step: OnboardingStep::Welcome,
            navigating_forward: true,
            language: stored_language.filter(|code| !code.is_empty()),
            skipped_accessibility: false,
            download_error: None,
        }
    }

    /// Current step.
    pub fn step(&self) -> OnboardingStep {
        self.step
    }

    /// The selected language, if any.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// The user chose "Use the clipboard instead".
    pub fn skipped_accessibility(&self) -> bool {
        self.skipped_accessibility
    }

    /// The last download failure on the model step (ONB-8).
    pub fn download_error(&self) -> Option<&str> {
        self.download_error.as_deref()
    }

    /// Continue is enabled (ONB-4, ONB-6, ONB-7, ONB-8).
    pub fn can_continue(&self, env: &OnboardingEnv<'_>) -> bool {
        self.continue_hint(env).is_none()
    }

    /// Why Continue is disabled, or `None` when it is enabled.
    pub fn continue_hint(&self, env: &OnboardingEnv<'_>) -> Option<String> {
        match self.step {
            OnboardingStep::Welcome | OnboardingStep::Ready => None,
            OnboardingStep::Language => self
                .language
                .is_none()
                .then(|| "Select a language to continue".to_owned()),
            OnboardingStep::Permissions => {
                if env.microphone != MicrophonePermission::Granted {
                    return Some("Microphone access is required for dictation".to_owned());
                }
                (!env.accessibility_granted && !self.skipped_accessibility)
                    .then(|| env.privacy_naming.onboarding_hint().to_owned())
            }
            OnboardingStep::Hotkey => {
                (env.hotkey_needs_accessibility && !env.accessibility_granted).then(|| {
                    format!(
                        "{} needs Typing Access — pick another hotkey",
                        env.hotkey_display
                    )
                })
            }
            OnboardingStep::SpeechModel => match env.selected_model {
                Some(model) if model.status.downloaded => None,
                Some(SelectedModel {
                    name,
                    status:
                        ModelStatus {
                            download_progress: Some(progress),
                            ..
                        },
                }) => Some(format!("Downloading {name} — {}%", percent(progress))),
                Some(_) | None => Some("Download or choose a speech model to continue".to_owned()),
            },
        }
    }

    /// The primary button (ONB-1). Moves forward when allowed and returns
    /// what the caller must do.
    pub fn advance(&mut self, env: &OnboardingEnv<'_>) -> Option<OnboardingEffect> {
        if !self.can_continue(env) {
            return None;
        }
        let Some(next) = self.step.next() else {
            return Some(OnboardingEffect::Finish);
        };
        let leaving = self.step;
        self.step = next;
        self.navigating_forward = true;
        match (leaving, next) {
            (OnboardingStep::Language, _)
                if !env.recommended_model.downloaded
                    && env.recommended_model.download_progress.is_none() =>
            {
                Some(OnboardingEffect::StartRecommendedDownload)
            }
            (_, OnboardingStep::Ready) => Some(OnboardingEffect::ReachedReady),
            _ => None,
        }
    }

    /// "Back" (ONB-1). Returns whether the step changed.
    pub fn go_back(&mut self) -> bool {
        let Some(previous) = self.step.previous() else {
            return false;
        };
        self.step = previous;
        self.navigating_forward = false;
        true
    }

    /// Picks a language (ONB-4). Re-picking the current language changes
    /// nothing, so it cannot clobber a manually chosen model.
    pub fn select_language(
        &mut self,
        code: &str,
        recommended_model_id: &str,
    ) -> Option<LanguagePick> {
        if self.language.as_deref() == Some(code) {
            return None;
        }
        self.language = Some(code.to_owned());
        Some(LanguagePick {
            language: code.to_owned(),
            model_id: recommended_model_id.to_owned(),
        })
    }

    /// "Use the clipboard instead" (ONB-6): counts as resolved. Returns the
    /// output mode to save.
    pub fn use_clipboard_instead(&mut self) -> OutputMode {
        self.skipped_accessibility = true;
        OutputMode::Clipboard
    }

    /// Applies a download update (ONB-8). Cancellation clears the error.
    pub fn download_update(&mut self, update: OnboardingDownloadUpdate) {
        self.download_error = match update {
            OnboardingDownloadUpdate::Active
            | OnboardingDownloadUpdate::Failed {
                cancelled: true, ..
            } => None,
            OnboardingDownloadUpdate::Failed {
                cancelled: false,
                error,
            } => Some(error),
        };
    }

    /// The user cancelled a download from the model step.
    pub fn cancel_download(&mut self) {
        self.download_error = None;
    }

    /// The window is closing, finished or not (ONB-10 to ONB-12).
    ///
    /// Closed early: a run that had been completed before (Run Setup Again)
    /// stays complete; an abandoned first run stays pending. Then load only
    /// a model that is already downloaded and neither loaded nor loading:
    /// never start a download the user did not agree to.
    pub fn close(
        &self,
        flags: OnboardingFlags,
        selected_model_downloaded: bool,
        model_loaded: bool,
        load_in_flight: bool,
    ) -> CloseOutcome {
        if flags.onboarding_completed {
            return CloseOutcome {
                flags,
                load_model: false,
                request_notifications: true,
            };
        }
        let flags = OnboardingFlags {
            onboarding_completed: self.previously_completed,
            ..flags
        };
        CloseOutcome {
            flags,
            load_model: selected_model_downloaded && !model_loaded && !load_in_flight,
            request_notifications: true,
        }
    }

    /// Progress bar, footer, and primary button (ONB-1).
    pub fn frame(&self, env: &OnboardingEnv<'_>) -> OnboardingFrame {
        let hint = self.continue_hint(env);
        let progress = match self.step {
            OnboardingStep::Welcome => None,
            step => Some(ProgressSegments {
                total: OnboardingStep::ALL.len() - 1,
                filled: step.index(),
            }),
        };
        OnboardingFrame {
            step: self.step,
            navigating_forward: self.navigating_forward,
            progress,
            back_visible: self.step != OnboardingStep::Welcome,
            primary_title: match self.step {
                OnboardingStep::Welcome => "Get Started",
                OnboardingStep::Ready => "Start Dictating",
                OnboardingStep::Language
                | OnboardingStep::Permissions
                | OnboardingStep::Hotkey
                | OnboardingStep::SpeechModel => "Continue",
            },
            primary_enabled: hint.is_none(),
            hint,
        }
    }

    /// The Permissions step cards (ONB-6).
    pub fn permissions(&self, env: &OnboardingEnv<'_>) -> PermissionsStepView {
        PermissionsStepView {
            microphone: microphone_card(env.microphone),
            typing_access: typing_access_card(
                env.accessibility_granted,
                self.skipped_accessibility,
            ),
            clipboard_link: (!env.accessibility_granted && !self.skipped_accessibility)
                .then_some("Use the clipboard instead"),
        }
    }
}

/// Segmented progress bar: one capsule per step after Welcome (ONB-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressSegments {
    /// Capsules.
    pub total: usize,
    /// Capsules filled, up to and including the current step.
    pub filled: usize,
}

/// The parts of the onboarding window every step shares (ONB-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingFrame {
    /// Current step.
    pub step: OnboardingStep,
    /// Slide in from the trailing edge (forward) or the leading edge (back).
    pub navigating_forward: bool,
    /// Hidden on Welcome.
    pub progress: Option<ProgressSegments>,
    /// "Back" is shown (hidden on Welcome).
    pub back_visible: bool,
    /// Why the primary button is disabled.
    pub hint: Option<String>,
    /// "Get Started", "Continue", or "Start Dictating".
    pub primary_title: &'static str,
    /// The primary button is enabled; it is the default action.
    pub primary_enabled: bool,
}

/// Welcome step title (ONB-3).
pub const WELCOME_TITLE: &str = "Welcome to Onda";
/// Welcome step subtitle (ONB-3).
pub const WELCOME_SUBTITLE: &str = "On-device dictation for any app.";

/// Label on a granted permission card (ONB-6).
pub const PERMISSION_READY_LABEL: &str = "Ready";

/// One permission card (ONB-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionCard {
    /// Card title.
    pub title: &'static str,
    /// Detail line; none once granted.
    pub detail: Option<&'static str>,
    /// Show [`PERMISSION_READY_LABEL`].
    pub granted: bool,
    /// Fix button.
    pub button: Option<PermissionFix>,
}

/// The Permissions step (ONB-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionsStepView {
    /// Microphone card.
    pub microphone: PermissionCard,
    /// Typing Access (Accessibility) card.
    pub typing_access: PermissionCard,
    /// "Use the clipboard instead" link, when Accessibility is missing and
    /// not skipped.
    pub clipboard_link: Option<&'static str>,
}

/// Permissions step title (ONB-6).
pub const PERMISSIONS_TITLE: &str = "Allow access";

fn microphone_card(microphone: MicrophonePermission) -> PermissionCard {
    let (detail, button) = match microphone {
        MicrophonePermission::Granted => (None, None),
        MicrophonePermission::NotDetermined => (
            Some("Only used while you dictate."),
            Some(PermissionFix::RequestMicrophone),
        ),
        MicrophonePermission::Denied => (
            Some("Turn it on in System Settings."),
            Some(PermissionFix::OpenMicrophoneSettings),
        ),
        // a device policy restricts it; no setting the user can change
        MicrophonePermission::Restricted => (Some("Restricted on this Mac."), None),
    };
    PermissionCard {
        title: "Microphone",
        detail,
        granted: microphone == MicrophonePermission::Granted,
        button,
    }
}

fn typing_access_card(granted: bool, skipped: bool) -> PermissionCard {
    let (detail, button) = match (granted, skipped) {
        (true, _) => (None, None),
        (false, true) => (
            Some("Transcripts go to the clipboard."),
            Some(PermissionFix::OpenAccessibilitySettings),
        ),
        (false, false) => (
            Some("Types transcripts into the app you're using."),
            Some(PermissionFix::OpenAccessibilitySettings),
        ),
    };
    PermissionCard {
        title: "Typing Access",
        detail,
        granted,
        button,
    }
}

/// Language step title (ONB-4).
pub const LANGUAGE_TITLE: &str = "What language do you speak?";
/// Language search placeholder (ONB-4).
pub const LANGUAGE_SEARCH_PLACEHOLDER: &str = "Search languages";

/// A language offered on the Language step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageOption {
    /// Display name.
    pub name: String,
    /// Language code.
    pub code: String,
}

/// One language card (ONB-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCard {
    /// Language code.
    pub code: String,
    /// Display name.
    pub name: String,
    /// Code in capitals.
    pub code_label: String,
    /// Flag emoji for popular languages, else 🌐.
    pub flag: &'static str,
    /// Check mark and accent border.
    pub selected: bool,
}

/// The Language step grid (ONB-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageStepView {
    /// "Show All" / "Popular Only"; hidden while searching.
    pub toggle_title: Option<&'static str>,
    /// Cards, three columns.
    pub cards: Vec<LanguageCard>,
}

/// The onboarding flag for a language.
pub fn language_flag(code: &str) -> &'static str {
    match code {
        "en" => "🇺🇸",
        "es" => "🇪🇸",
        "fr" => "🇫🇷",
        "de" => "🇩🇪",
        "pt" => "🇧🇷",
        "it" => "🇮🇹",
        "ja" => "🇯🇵",
        "ko" => "🇰🇷",
        "zh" => "🇨🇳",
        "hi" => "🇮🇳",
        "ar" => "🇸🇦",
        _ => "🌐",
    }
}

/// Search matches the display name or the code, ignoring case (ONB-4).
pub fn language_matches(query: &str, option: &LanguageOption) -> bool {
    let query = query.to_lowercase();
    option.name.to_lowercase().contains(&query) || option.code.to_lowercase().contains(&query)
}

/// Builds the Language step grid (ONB-4). A non-empty `query` searches all
/// languages; otherwise `show_all` picks all or popular.
pub fn language_step(
    query: &str,
    show_all: bool,
    popular: &[LanguageOption],
    all: &[LanguageOption],
    selected: Option<&str>,
) -> LanguageStepView {
    let searching = !query.is_empty();
    let shown: Vec<&LanguageOption> = match (searching, show_all) {
        (true, _) => all.iter().filter(|o| language_matches(query, o)).collect(),
        (false, true) => all.iter().collect(),
        (false, false) => popular.iter().collect(),
    };
    LanguageStepView {
        toggle_title: match (searching, show_all) {
            (true, _) => None,
            (false, true) => Some("Popular Only"),
            (false, false) => Some("Show All"),
        },
        cards: shown
            .into_iter()
            .map(|o| LanguageCard {
                code: o.code.clone(),
                name: o.name.clone(),
                code_label: o.code.to_uppercase(),
                flag: language_flag(&o.code),
                selected: selected == Some(o.code.as_str()),
            })
            .collect(),
    }
}

/// Hotkey step title (ONB-7).
pub const HOTKEY_TITLE: &str = "Choose a hotkey";

/// A hotkey preset as the hotkey code describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyPresetChoice {
    /// Keycap labels.
    pub keycaps: Vec<String>,
    /// Detail text.
    pub detail: String,
    /// A lone modifier, which needs Accessibility.
    pub needs_accessibility: bool,
    /// It is the current shortcut.
    pub selected: bool,
    /// Warning to show while selected, e.g. the Fn warning (HOT-35).
    pub warning: Option<String>,
}

/// One preset card (ONB-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyPresetCard {
    /// Keycap labels.
    pub keycaps: Vec<String>,
    /// Detail, or why the preset is unavailable.
    pub detail: String,
    /// Clickable; dimmed when not.
    pub enabled: bool,
    /// Check mark and accent border.
    pub selected: bool,
    /// Warning below the card.
    pub warning: Option<String>,
}

/// What the "Another shortcut" card shows above its recorder (ONB-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomHotkeyLabel {
    /// The current shortcut's keycaps, when it is not a preset.
    Keycaps(Vec<String>),
    /// "Another shortcut".
    Text(&'static str),
}

/// The Hotkey step (ONB-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyStepView {
    /// The activation mode's description.
    pub subtitle: String,
    /// Preset cards.
    pub presets: Vec<HotkeyPresetCard>,
    /// The custom card is selected (the shortcut is not a preset).
    pub custom_selected: bool,
    /// The custom card's label.
    pub custom_label: CustomHotkeyLabel,
}

/// Builds the Hotkey step (ONB-7). Presets that need Accessibility are
/// disabled while it is missing.
pub fn hotkey_step(
    activation_description: &str,
    presets: &[HotkeyPresetChoice],
    current_keycaps: &[String],
    accessibility_granted: bool,
) -> HotkeyStepView {
    let custom_selected = !presets.iter().any(|p| p.selected);
    HotkeyStepView {
        subtitle: activation_description.to_owned(),
        presets: presets
            .iter()
            .map(|p| {
                let enabled = !p.needs_accessibility || accessibility_granted;
                HotkeyPresetCard {
                    keycaps: p.keycaps.clone(),
                    detail: match enabled {
                        true => p.detail.clone(),
                        false => "Needs Typing Access. Go Back to turn it on.".to_owned(),
                    },
                    enabled,
                    selected: p.selected,
                    warning: p.warning.clone().filter(|_| p.selected),
                }
            })
            .collect(),
        custom_selected,
        custom_label: match custom_selected {
            true => CustomHotkeyLabel::Keycaps(current_keycaps.to_vec()),
            false => CustomHotkeyLabel::Text("Another shortcut"),
        },
    }
}

/// Speech Model step title (ONB-8).
pub const MODEL_TITLE: &str = "Choose a speech model";

/// A model compatible with the selected language, in route order.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelChoice {
    /// Catalog id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Size hint, e.g. "600 MB".
    pub size_hint: String,
    /// The language's recommended model.
    pub recommended: bool,
    /// A built-in engine.
    pub built_in: bool,
    /// Download status.
    pub status: ModelStatus,
}

/// A model card's action (ONB-8).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelAction {
    /// Progress bar, "Downloading — N%", and "Cancel".
    Downloading {
        /// Fraction in `[0, 1]`.
        progress: f64,
        /// "Downloading — N%".
        label: String,
    },
    /// Green "Selected".
    Selected,
    /// "Use This Model": downloaded, not chosen.
    UseThisModel,
    /// "Download": selects and starts the download.
    Download,
}

impl ModelAction {
    /// Button or label title.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Downloading { .. } => "Cancel",
            Self::Selected => "Selected",
            Self::UseThisModel => "Use This Model",
            Self::Download => "Download",
        }
    }
}

/// One model card or compact row (ONB-8).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCard {
    /// Catalog id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Size hint.
    pub size_hint: String,
    /// Show the "Recommended" pill.
    pub recommended: bool,
    /// The selected model (accent border).
    pub selected: bool,
    /// Action.
    pub action: ModelAction,
}

/// Models behind "Show other models" (ONB-8).
#[derive(Debug, Clone, PartialEq)]
pub struct OtherModels {
    /// "Show other models" / "Hide other models".
    pub toggle_title: &'static str,
    /// Compact rows; empty while collapsed.
    pub rows: Vec<ModelCard>,
}

/// The Speech Model step (ONB-8).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelStepView {
    /// Full cards: the recommended model and built-in engines.
    pub cards: Vec<ModelCard>,
    /// Everything else, when there is anything else.
    pub other_models: Option<OtherModels>,
    /// "Download failed: <error>" in red.
    pub error: Option<String>,
}

fn model_action(status: ModelStatus, selected: bool) -> ModelAction {
    match (status.download_progress, status.downloaded, selected) {
        (Some(progress), _, _) => ModelAction::Downloading {
            progress,
            label: format!("Downloading — {}%", percent(progress)),
        },
        (None, true, true) => ModelAction::Selected,
        (None, true, false) => ModelAction::UseThisModel,
        (None, false, _) => ModelAction::Download,
    }
}

/// Builds the Speech Model step (ONB-8).
pub fn model_step(
    models: &[ModelChoice],
    selected_model_id: &str,
    show_other_models: bool,
    download_error: Option<&str>,
) -> ModelStepView {
    let card = |m: &ModelChoice| {
        let selected = m.id == selected_model_id;
        ModelCard {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            size_hint: m.size_hint.clone(),
            recommended: m.recommended,
            selected,
            action: model_action(m.status, selected),
        }
    };
    let (primary, others): (Vec<&ModelChoice>, Vec<&ModelChoice>) =
        models.iter().partition(|m| m.recommended || m.built_in);
    let other_models = (!others.is_empty()).then(|| OtherModels {
        toggle_title: match show_other_models {
            true => "Hide other models",
            false => "Show other models",
        },
        rows: match show_other_models {
            true => others.iter().map(|m| card(m)).collect(),
            false => Vec::new(),
        },
    });
    ModelStepView {
        cards: primary.iter().map(|m| card(m)).collect(),
        other_models,
        error: download_error.map(|e| format!("Download failed: {e}")),
    }
}

/// The Ready step (ONB-9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyStepView {
    /// "You're all set".
    pub title: &'static str,
    /// Text before the hotkey keycaps.
    pub prompt_prefix: &'static str,
    /// Text after the hotkey keycaps.
    pub prompt_suffix: &'static str,
    /// Try-it field placeholder (WIN-6).
    pub try_it_placeholder: &'static str,
    /// How long transcripts are kept, when history is on.
    pub retention_note: Option<String>,
}

/// Builds the Ready step (ONB-9).
pub fn ready_step(
    history_enabled: bool,
    retention: HistoryRetention,
    output_mode: OutputMode,
) -> ReadyStepView {
    ReadyStepView {
        title: "You're all set",
        prompt_prefix: "Press",
        prompt_suffix: "and start talking.",
        try_it_placeholder: try_it_placeholder(output_mode),
        retention_note: retention_note(history_enabled, retention),
    }
}

/// "Transcripts are saved on this Mac for 1 month." (ONB-9).
pub fn retention_note(history_enabled: bool, retention: HistoryRetention) -> Option<String> {
    if !history_enabled {
        return None;
    }
    let period = match retention {
        HistoryRetention::Week => " for 1 week",
        HistoryRetention::Month => " for 1 month",
        HistoryRetention::Forever => "",
    };
    Some(format!("Transcripts are saved on this Mac{period}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOWNLOADED: ModelStatus = ModelStatus {
        downloaded: true,
        download_progress: None,
    };
    const MISSING: ModelStatus = ModelStatus {
        downloaded: false,
        download_progress: None,
    };

    fn downloading(progress: f64) -> ModelStatus {
        ModelStatus {
            downloaded: false,
            download_progress: Some(progress),
        }
    }

    fn env() -> OnboardingEnv<'static> {
        OnboardingEnv {
            microphone: MicrophonePermission::Granted,
            accessibility_granted: true,
            hotkey_display: "⌥ Space",
            hotkey_needs_accessibility: false,
            privacy_naming: PrivacyNaming::Accessibility,
            selected_model: Some(SelectedModel {
                name: "Parakeet TDT v2",
                status: DOWNLOADED,
            }),
            recommended_model: DOWNLOADED,
        }
    }

    fn at(step: OnboardingStep) -> Onboarding {
        let mut onboarding = Onboarding::open(
            OnboardingOrigin::Automatic,
            OnboardingFlags::default(),
            Some("en".to_owned()),
        );
        while onboarding.step() != step {
            onboarding.advance(&env());
        }
        onboarding
    }

    #[test]
    fn onb_1_step_order() {
        let want = [
            OnboardingStep::Welcome,
            OnboardingStep::Language,
            OnboardingStep::Permissions,
            OnboardingStep::Hotkey,
            OnboardingStep::SpeechModel,
            OnboardingStep::Ready,
        ];
        assert_eq!(OnboardingStep::ALL, want);
    }

    #[test]
    fn onb_1_frame_per_step() {
        struct Case {
            name: &'static str,
            step: OnboardingStep,
            want_progress: Option<(usize, usize)>,
            want_back: bool,
            want_primary: &'static str,
        }
        let cases = [
            Case {
                name: "welcome",
                step: OnboardingStep::Welcome,
                want_progress: None,
                want_back: false,
                want_primary: "Get Started",
            },
            Case {
                name: "language",
                step: OnboardingStep::Language,
                want_progress: Some((5, 1)),
                want_back: true,
                want_primary: "Continue",
            },
            Case {
                name: "permissions",
                step: OnboardingStep::Permissions,
                want_progress: Some((5, 2)),
                want_back: true,
                want_primary: "Continue",
            },
            Case {
                name: "hotkey",
                step: OnboardingStep::Hotkey,
                want_progress: Some((5, 3)),
                want_back: true,
                want_primary: "Continue",
            },
            Case {
                name: "speech model",
                step: OnboardingStep::SpeechModel,
                want_progress: Some((5, 4)),
                want_back: true,
                want_primary: "Continue",
            },
            Case {
                name: "ready",
                step: OnboardingStep::Ready,
                want_progress: Some((5, 5)),
                want_back: true,
                want_primary: "Start Dictating",
            },
        ];
        for tc in cases {
            let got = at(tc.step).frame(&env());
            assert_eq!(got.step, tc.step, "{}", tc.name);
            assert_eq!(
                got.progress.map(|p| (p.total, p.filled)),
                tc.want_progress,
                "{}",
                tc.name
            );
            assert_eq!(got.back_visible, tc.want_back, "{}", tc.name);
            assert_eq!(got.primary_title, tc.want_primary, "{}", tc.name);
            assert!(got.primary_enabled, "{}", tc.name);
            assert_eq!(got.hint, None, "{}", tc.name);
        }
    }

    #[test]
    fn onb_1_navigation() {
        struct Case {
            name: &'static str,
            start: OnboardingStep,
            back: bool,
            want_step: OnboardingStep,
            want_forward: bool,
            want_effect: Option<OnboardingEffect>,
        }
        let cases = [
            Case {
                name: "welcome moves to language",
                start: OnboardingStep::Welcome,
                back: false,
                want_step: OnboardingStep::Language,
                want_forward: true,
                want_effect: None,
            },
            Case {
                name: "hotkey step moves to model step",
                start: OnboardingStep::Hotkey,
                back: false,
                want_step: OnboardingStep::SpeechModel,
                want_forward: true,
                want_effect: None,
            },
            Case {
                name: "ONB-9 model step moves to ready",
                start: OnboardingStep::SpeechModel,
                back: false,
                want_step: OnboardingStep::Ready,
                want_forward: true,
                want_effect: Some(OnboardingEffect::ReachedReady),
            },
            Case {
                name: "ONB-10 ready completes",
                start: OnboardingStep::Ready,
                back: false,
                want_step: OnboardingStep::Ready,
                want_forward: true,
                want_effect: Some(OnboardingEffect::Finish),
            },
            Case {
                name: "back from language returns to welcome",
                start: OnboardingStep::Language,
                back: true,
                want_step: OnboardingStep::Welcome,
                want_forward: false,
                want_effect: None,
            },
            Case {
                name: "back from model step returns to hotkey",
                start: OnboardingStep::SpeechModel,
                back: true,
                want_step: OnboardingStep::Hotkey,
                want_forward: false,
                want_effect: None,
            },
            Case {
                name: "back from welcome is a no-op",
                start: OnboardingStep::Welcome,
                back: true,
                want_step: OnboardingStep::Welcome,
                want_forward: true,
                want_effect: None,
            },
        ];
        for tc in cases {
            let mut onboarding = at(tc.start);
            let effect = match tc.back {
                true => {
                    onboarding.go_back();
                    None
                }
                false => onboarding.advance(&env()),
            };
            assert_eq!(onboarding.step(), tc.want_step, "{}", tc.name);
            assert_eq!(
                onboarding.frame(&env()).navigating_forward,
                tc.want_forward,
                "{}",
                tc.name
            );
            assert_eq!(effect, tc.want_effect, "{}", tc.name);
        }
    }

    #[test]
    fn onb_9_returning_to_ready_reports_reaching_it_again() {
        let mut onboarding = at(OnboardingStep::SpeechModel);
        let mut reached = 0;
        for _ in 0..2 {
            if onboarding.advance(&env()) == Some(OnboardingEffect::ReachedReady) {
                reached += 1;
            }
            onboarding.go_back();
        }
        assert_eq!(reached, 2);
    }

    #[test]
    fn onb_5_leaving_language_starts_recommended_download() {
        struct Case {
            name: &'static str,
            recommended: ModelStatus,
            want: Option<OnboardingEffect>,
        }
        let cases = [
            Case {
                name: "not downloaded starts it",
                recommended: MISSING,
                want: Some(OnboardingEffect::StartRecommendedDownload),
            },
            Case {
                name: "already downloaded",
                recommended: DOWNLOADED,
                want: None,
            },
            Case {
                name: "already downloading",
                recommended: downloading(0.1),
                want: None,
            },
        ];
        for tc in cases {
            let mut onboarding = at(OnboardingStep::Language);
            let got = onboarding.advance(&OnboardingEnv {
                recommended_model: tc.recommended,
                ..env()
            });
            assert_eq!(got, tc.want, "{}", tc.name);
            assert_eq!(
                onboarding.step(),
                OnboardingStep::Permissions,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn continue_enablement_and_hints() {
        struct Case {
            name: &'static str,
            step: OnboardingStep,
            env: OnboardingEnv<'static>,
            skip_accessibility: bool,
            want_hint: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "ONB-6 microphone never asked",
                step: OnboardingStep::Permissions,
                env: OnboardingEnv {
                    microphone: MicrophonePermission::NotDetermined,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Microphone access is required for dictation"),
            },
            Case {
                name: "ONB-6 microphone denied",
                step: OnboardingStep::Permissions,
                env: OnboardingEnv {
                    microphone: MicrophonePermission::Denied,
                    accessibility_granted: false,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Microphone access is required for dictation"),
            },
            Case {
                name: "ONB-6 accessibility missing",
                step: OnboardingStep::Permissions,
                env: OnboardingEnv {
                    accessibility_granted: false,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Enable Accessibility or choose clipboard output"),
            },
            Case {
                name: "ONB-6 accessibility missing on macOS 27",
                step: OnboardingStep::Permissions,
                env: OnboardingEnv {
                    accessibility_granted: false,
                    privacy_naming: PrivacyNaming::DeviceControlAndDataAccess,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Enable Device Control and Data Access or choose clipboard output"),
            },
            Case {
                name: "ONB-6 clipboard instead resolves it",
                step: OnboardingStep::Permissions,
                env: OnboardingEnv {
                    accessibility_granted: false,
                    ..env()
                },
                skip_accessibility: true,
                want_hint: None,
            },
            Case {
                name: "ONB-6 all granted",
                step: OnboardingStep::Permissions,
                env: env(),
                skip_accessibility: false,
                want_hint: None,
            },
            Case {
                name: "ONB-7 Option Space without typing access",
                step: OnboardingStep::Hotkey,
                env: OnboardingEnv {
                    accessibility_granted: false,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: None,
            },
            Case {
                name: "ONB-7 Right Option needs typing access",
                step: OnboardingStep::Hotkey,
                env: OnboardingEnv {
                    accessibility_granted: false,
                    hotkey_display: "Right Option",
                    hotkey_needs_accessibility: true,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Right Option needs Typing Access — pick another hotkey"),
            },
            Case {
                name: "ONB-7 Fn needs typing access",
                step: OnboardingStep::Hotkey,
                env: OnboardingEnv {
                    accessibility_granted: false,
                    hotkey_display: "Fn",
                    hotkey_needs_accessibility: true,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Fn needs Typing Access — pick another hotkey"),
            },
            Case {
                name: "ONB-7 lone modifier with typing access",
                step: OnboardingStep::Hotkey,
                env: OnboardingEnv {
                    hotkey_display: "Right Option",
                    hotkey_needs_accessibility: true,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: None,
            },
            Case {
                name: "ONB-8 selected model downloaded",
                step: OnboardingStep::SpeechModel,
                env: env(),
                skip_accessibility: false,
                want_hint: None,
            },
            Case {
                name: "ONB-8 selected model downloading",
                step: OnboardingStep::SpeechModel,
                env: OnboardingEnv {
                    selected_model: Some(SelectedModel {
                        name: "Parakeet TDT v2",
                        status: downloading(0.37),
                    }),
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Downloading Parakeet TDT v2 — 37%"),
            },
            Case {
                name: "ONB-8 selected model missing",
                step: OnboardingStep::SpeechModel,
                env: OnboardingEnv {
                    selected_model: Some(SelectedModel {
                        name: "Parakeet TDT v2",
                        status: MISSING,
                    }),
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Download or choose a speech model to continue"),
            },
            Case {
                name: "ONB-8 selected model unknown",
                step: OnboardingStep::SpeechModel,
                env: OnboardingEnv {
                    selected_model: None,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: Some("Download or choose a speech model to continue"),
            },
            Case {
                name: "welcome always continues",
                step: OnboardingStep::Welcome,
                env: OnboardingEnv {
                    microphone: MicrophonePermission::Denied,
                    selected_model: None,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: None,
            },
            Case {
                name: "ready always continues",
                step: OnboardingStep::Ready,
                env: OnboardingEnv {
                    microphone: MicrophonePermission::Denied,
                    selected_model: None,
                    ..env()
                },
                skip_accessibility: false,
                want_hint: None,
            },
        ];
        for tc in cases {
            let mut onboarding = at(tc.step);
            if tc.skip_accessibility {
                assert_eq!(
                    onboarding.use_clipboard_instead(),
                    OutputMode::Clipboard,
                    "{}",
                    tc.name
                );
            }
            assert_eq!(
                onboarding.continue_hint(&tc.env).as_deref(),
                tc.want_hint,
                "{}",
                tc.name
            );
            assert_eq!(
                onboarding.can_continue(&tc.env),
                tc.want_hint.is_none(),
                "{}",
                tc.name
            );
            let frame = onboarding.frame(&tc.env);
            assert_eq!(frame.primary_enabled, tc.want_hint.is_none(), "{}", tc.name);
            let before = onboarding.step();
            let effect = onboarding.advance(&tc.env);
            if tc.want_hint.is_some() {
                assert_eq!(
                    onboarding.step(),
                    before,
                    "{}: blocked continue must not move",
                    tc.name
                );
                assert_eq!(effect, None, "{}", tc.name);
            }
        }
    }

    #[test]
    fn onb_4_language_continue_requires_selection() {
        let mut onboarding = Onboarding::open(
            OnboardingOrigin::Automatic,
            OnboardingFlags::default(),
            None,
        );
        onboarding.advance(&env());
        assert_eq!(
            onboarding.continue_hint(&env()).as_deref(),
            Some("Select a language to continue")
        );
        assert_eq!(onboarding.advance(&env()), None);
        assert_eq!(onboarding.step(), OnboardingStep::Language);
        onboarding.select_language("es", "parakeet-ultra");
        assert!(onboarding.can_continue(&env()));
    }

    #[test]
    fn onb_4_language_pick() {
        struct Case {
            name: &'static str,
            stored: Option<&'static str>,
            pick: &'static str,
            want: Option<LanguagePick>,
        }
        let cases = [
            Case {
                name: "new language saves it and the recommendation",
                stored: Some("en"),
                pick: "es",
                want: Some(LanguagePick {
                    language: "es".to_owned(),
                    model_id: "parakeet-ultra".to_owned(),
                }),
            },
            Case {
                name: "re-picking the current language changes nothing",
                stored: Some("es"),
                pick: "es",
                want: None,
            },
            Case {
                name: "first pick with nothing stored",
                stored: None,
                pick: "es",
                want: Some(LanguagePick {
                    language: "es".to_owned(),
                    model_id: "parakeet-ultra".to_owned(),
                }),
            },
            Case {
                name: "empty stored language counts as none",
                stored: Some(""),
                pick: "es",
                want: Some(LanguagePick {
                    language: "es".to_owned(),
                    model_id: "parakeet-ultra".to_owned(),
                }),
            },
        ];
        for tc in cases {
            let mut onboarding = Onboarding::open(
                OnboardingOrigin::Automatic,
                OnboardingFlags::default(),
                tc.stored.map(str::to_owned),
            );
            assert_eq!(
                onboarding.select_language(tc.pick, "parakeet-ultra"),
                tc.want,
                "{}",
                tc.name
            );
            assert_eq!(onboarding.language(), Some(tc.pick), "{}", tc.name);
        }
    }

    #[test]
    fn onb_8_download_error() {
        struct Case {
            name: &'static str,
            updates: Vec<OnboardingDownloadUpdate>,
            want: Option<&'static str>,
        }
        let failed = || OnboardingDownloadUpdate::Failed {
            cancelled: false,
            error: "Network went full Señor Chang".to_owned(),
        };
        let cases = [
            Case {
                name: "failure shows",
                updates: vec![failed()],
                want: Some("Network went full Señor Chang"),
            },
            Case {
                name: "cancellation clears it",
                updates: vec![
                    failed(),
                    OnboardingDownloadUpdate::Failed {
                        cancelled: true,
                        error: "Download cancelled".to_owned(),
                    },
                ],
                want: None,
            },
            Case {
                name: "a new download clears it",
                updates: vec![failed(), OnboardingDownloadUpdate::Active],
                want: None,
            },
        ];
        for tc in cases {
            let mut onboarding = at(OnboardingStep::SpeechModel);
            for update in tc.updates {
                onboarding.download_update(update);
            }
            assert_eq!(onboarding.download_error(), tc.want, "{}", tc.name);
        }
        let mut onboarding = at(OnboardingStep::SpeechModel);
        onboarding.download_update(failed());
        onboarding.cancel_download();
        assert_eq!(onboarding.download_error(), None, "cancel button clears it");
    }

    #[test]
    fn set_4_flags() {
        struct Case {
            name: &'static str,
            flags: OnboardingFlags,
            want_pending: bool,
        }
        let cases = [
            Case {
                name: "fresh install",
                flags: OnboardingFlags {
                    onboarding_completed: false,
                    did_show_first_run_setup: false,
                },
                want_pending: true,
            },
            Case {
                name: "completed",
                flags: OnboardingFlags {
                    onboarding_completed: true,
                    did_show_first_run_setup: true,
                },
                want_pending: false,
            },
            Case {
                name: "run setup again",
                flags: OnboardingFlags {
                    onboarding_completed: false,
                    did_show_first_run_setup: true,
                },
                want_pending: false,
            },
            Case {
                name: "legacy completed only",
                flags: OnboardingFlags {
                    onboarding_completed: true,
                    did_show_first_run_setup: false,
                },
                want_pending: false,
            },
        ];
        for tc in cases {
            assert_eq!(tc.flags.first_run_pending(), tc.want_pending, "{}", tc.name);
        }
        let done = OnboardingFlags::default().finished();
        assert_eq!(
            done,
            OnboardingFlags {
                onboarding_completed: true,
                did_show_first_run_setup: true
            },
            "finish sets both"
        );
        assert_eq!(
            done.rerun(),
            OnboardingFlags {
                onboarding_completed: false,
                did_show_first_run_setup: true
            },
            "rerun clears only completed"
        );
        assert!(
            !done.rerun().first_run_pending(),
            "rerun is not first-run pending"
        );
    }

    #[test]
    fn onb_12_onb_13_launch_plan() {
        struct Case {
            name: &'static str,
            flags: OnboardingFlags,
            want: LaunchPlan,
        }
        let cases = [
            Case {
                name: "first run shows onboarding after 750 ms and holds notifications",
                flags: OnboardingFlags::default(),
                want: LaunchPlan {
                    show_onboarding_after: Some(Duration::from_millis(750)),
                    request_notifications: false,
                    skip_model_load: true,
                },
            },
            Case {
                name: "completed users get the notification request at launch",
                flags: OnboardingFlags::default().finished(),
                want: LaunchPlan {
                    show_onboarding_after: None,
                    request_notifications: true,
                    skip_model_load: false,
                },
            },
            Case {
                name: "abandoned rerun is not first run",
                flags: OnboardingFlags::default().finished().rerun(),
                want: LaunchPlan {
                    show_onboarding_after: None,
                    request_notifications: true,
                    skip_model_load: false,
                },
            },
        ];
        for tc in cases {
            assert_eq!(launch_plan(tc.flags), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn onb_10_to_12_close() {
        struct Case {
            name: &'static str,
            origin: OnboardingOrigin,
            flags_at_open: OnboardingFlags,
            flags_at_close: OnboardingFlags,
            downloaded: bool,
            loaded: bool,
            in_flight: bool,
            want: CloseOutcome,
        }
        let fresh = OnboardingFlags::default();
        let done = fresh.finished();
        let rerun = done.rerun();
        let cases = [
            Case {
                name: "ONB-10 finished: nothing more to load here",
                origin: OnboardingOrigin::Automatic,
                flags_at_open: fresh,
                flags_at_close: done,
                downloaded: true,
                loaded: false,
                in_flight: false,
                want: CloseOutcome {
                    flags: done,
                    load_model: false,
                    request_notifications: true,
                },
            },
            Case {
                name: "ONB-11 abandoned first run stays pending",
                origin: OnboardingOrigin::Automatic,
                flags_at_open: fresh,
                flags_at_close: fresh,
                downloaded: false,
                loaded: false,
                in_flight: false,
                want: CloseOutcome {
                    flags: fresh,
                    load_model: false,
                    request_notifications: true,
                },
            },
            Case {
                name: "ONB-11 abandoned rerun stays complete",
                origin: OnboardingOrigin::RunSetupAgain,
                flags_at_open: rerun,
                flags_at_close: rerun,
                downloaded: false,
                loaded: false,
                in_flight: false,
                want: CloseOutcome {
                    flags: done,
                    load_model: false,
                    request_notifications: true,
                },
            },
            Case {
                name: "ONB-11 abandoned with a downloaded model loads it",
                origin: OnboardingOrigin::Automatic,
                flags_at_open: fresh,
                flags_at_close: fresh,
                downloaded: true,
                loaded: false,
                in_flight: false,
                want: CloseOutcome {
                    flags: fresh,
                    load_model: true,
                    request_notifications: true,
                },
            },
            Case {
                name: "ONB-11 abandoned with the model already loaded",
                origin: OnboardingOrigin::RunSetupAgain,
                flags_at_open: rerun,
                flags_at_close: rerun,
                downloaded: true,
                loaded: true,
                in_flight: false,
                want: CloseOutcome {
                    flags: done,
                    load_model: false,
                    request_notifications: true,
                },
            },
            Case {
                name: "ONB-11 abandoned with a load in flight",
                origin: OnboardingOrigin::Automatic,
                flags_at_open: fresh,
                flags_at_close: fresh,
                downloaded: true,
                loaded: false,
                in_flight: true,
                want: CloseOutcome {
                    flags: fresh,
                    load_model: false,
                    request_notifications: true,
                },
            },
            Case {
                name: "rerun opened during first run stays pending",
                origin: OnboardingOrigin::RunSetupAgain,
                flags_at_open: fresh,
                flags_at_close: fresh,
                downloaded: false,
                loaded: false,
                in_flight: false,
                want: CloseOutcome {
                    flags: fresh,
                    load_model: false,
                    request_notifications: true,
                },
            },
        ];
        for tc in cases {
            let onboarding = Onboarding::open(tc.origin, tc.flags_at_open, Some("en".to_owned()));
            let got = onboarding.close(tc.flags_at_close, tc.downloaded, tc.loaded, tc.in_flight);
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn onb_6_permission_cards() {
        struct Case {
            name: &'static str,
            microphone: MicrophonePermission,
            accessibility: bool,
            skipped: bool,
            want_mic: (Option<&'static str>, bool, Option<&'static str>),
            want_typing: (Option<&'static str>, bool, Option<&'static str>),
            want_link: bool,
        }
        let cases = [
            Case {
                name: "never asked",
                microphone: MicrophonePermission::NotDetermined,
                accessibility: false,
                skipped: false,
                want_mic: (Some("Only used while you dictate."), false, Some("Allow")),
                want_typing: (
                    Some("Types transcripts into the app you're using."),
                    false,
                    Some("Open Settings"),
                ),
                want_link: true,
            },
            Case {
                name: "denied",
                microphone: MicrophonePermission::Denied,
                accessibility: false,
                skipped: false,
                want_mic: (
                    Some("Turn it on in System Settings."),
                    false,
                    Some("Open Settings"),
                ),
                want_typing: (
                    Some("Types transcripts into the app you're using."),
                    false,
                    Some("Open Settings"),
                ),
                want_link: true,
            },
            Case {
                name: "restricted",
                microphone: MicrophonePermission::Restricted,
                accessibility: false,
                skipped: false,
                want_mic: (Some("Restricted on this Mac."), false, None),
                want_typing: (
                    Some("Types transcripts into the app you're using."),
                    false,
                    Some("Open Settings"),
                ),
                want_link: true,
            },
            Case {
                name: "skipped accessibility",
                microphone: MicrophonePermission::Granted,
                accessibility: false,
                skipped: true,
                want_mic: (None, true, None),
                want_typing: (
                    Some("Transcripts go to the clipboard."),
                    false,
                    Some("Open Settings"),
                ),
                want_link: false,
            },
            Case {
                name: "all granted",
                microphone: MicrophonePermission::Granted,
                accessibility: true,
                skipped: false,
                want_mic: (None, true, None),
                want_typing: (None, true, None),
                want_link: false,
            },
        ];
        for tc in cases {
            let mut onboarding = at(OnboardingStep::Permissions);
            if tc.skipped {
                onboarding.use_clipboard_instead();
            }
            let got = onboarding.permissions(&OnboardingEnv {
                microphone: tc.microphone,
                accessibility_granted: tc.accessibility,
                ..env()
            });
            let summary =
                |c: PermissionCard| (c.detail, c.granted, c.button.map(PermissionFix::title));
            assert_eq!(got.microphone.title, "Microphone", "{}", tc.name);
            assert_eq!(got.typing_access.title, "Typing Access", "{}", tc.name);
            assert_eq!(summary(got.microphone), tc.want_mic, "{}", tc.name);
            assert_eq!(summary(got.typing_access), tc.want_typing, "{}", tc.name);
            assert_eq!(
                got.clipboard_link,
                tc.want_link.then_some("Use the clipboard instead"),
                "{}",
                tc.name
            );
        }
    }

    fn languages() -> (Vec<LanguageOption>, Vec<LanguageOption>) {
        let lang = |name: &str, code: &str| LanguageOption {
            name: name.to_owned(),
            code: code.to_owned(),
        };
        let popular = vec![lang("English", "en"), lang("Spanish", "es")];
        let all = vec![
            lang("Croatian", "hr"),
            lang("English", "en"),
            lang("Maltese", "mt"),
            lang("Spanish", "es"),
        ];
        (popular, all)
    }

    #[test]
    fn onb_4_language_grid() {
        struct Case {
            name: &'static str,
            query: &'static str,
            show_all: bool,
            want_toggle: Option<&'static str>,
            want_codes: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "popular by default",
                query: "",
                show_all: false,
                want_toggle: Some("Show All"),
                want_codes: vec!["en", "es"],
            },
            Case {
                name: "show all",
                query: "",
                show_all: true,
                want_toggle: Some("Popular Only"),
                want_codes: vec!["hr", "en", "mt", "es"],
            },
            Case {
                name: "search by name ignores case and the toggle",
                query: "malt",
                show_all: false,
                want_toggle: None,
                want_codes: vec!["mt"],
            },
            Case {
                name: "search by code",
                query: "HR",
                show_all: false,
                want_toggle: None,
                want_codes: vec!["hr"],
            },
            Case {
                name: "search with no match",
                query: "klingon",
                show_all: true,
                want_toggle: None,
                want_codes: vec![],
            },
        ];
        let (popular, all) = languages();
        for tc in cases {
            let got = language_step(tc.query, tc.show_all, &popular, &all, Some("es"));
            assert_eq!(got.toggle_title, tc.want_toggle, "{}", tc.name);
            let codes: Vec<&str> = got.cards.iter().map(|c| c.code.as_str()).collect();
            assert_eq!(codes, tc.want_codes, "{}", tc.name);
        }
    }

    #[test]
    fn onb_4_language_card() {
        let (popular, all) = languages();
        let got = language_step("", true, &popular, &all, Some("es"));
        let want = [
            LanguageCard {
                code: "hr".to_owned(),
                name: "Croatian".to_owned(),
                code_label: "HR".to_owned(),
                flag: "🌐",
                selected: false,
            },
            LanguageCard {
                code: "en".to_owned(),
                name: "English".to_owned(),
                code_label: "EN".to_owned(),
                flag: "🇺🇸",
                selected: false,
            },
            LanguageCard {
                code: "mt".to_owned(),
                name: "Maltese".to_owned(),
                code_label: "MT".to_owned(),
                flag: "🌐",
                selected: false,
            },
            LanguageCard {
                code: "es".to_owned(),
                name: "Spanish".to_owned(),
                code_label: "ES".to_owned(),
                flag: "🇪🇸",
                selected: true,
            },
        ];
        assert_eq!(got.cards, want);
    }

    #[test]
    fn onb_4_language_flags() {
        struct Case {
            name: &'static str,
            code: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "english",
                code: "en",
                want: "🇺🇸",
            },
            Case {
                name: "portuguese is Brazil",
                code: "pt",
                want: "🇧🇷",
            },
            Case {
                name: "arabic",
                code: "ar",
                want: "🇸🇦",
            },
            Case {
                name: "non-popular",
                code: "mt",
                want: "🌐",
            },
            Case {
                name: "automatic",
                code: "auto",
                want: "🌐",
            },
        ];
        for tc in cases {
            assert_eq!(language_flag(tc.code), tc.want, "{}", tc.name);
        }
    }

    fn preset(
        keycap: &str,
        needs_accessibility: bool,
        selected: bool,
        warning: Option<&str>,
    ) -> HotkeyPresetChoice {
        HotkeyPresetChoice {
            keycaps: vec![keycap.to_owned()],
            detail: format!("{keycap} detail"),
            needs_accessibility,
            selected,
            warning: warning.map(str::to_owned),
        }
    }

    #[test]
    fn onb_7_hotkey_presets() {
        struct Case {
            name: &'static str,
            presets: Vec<HotkeyPresetChoice>,
            accessibility: bool,
            want_enabled: Vec<bool>,
            want_details: Vec<&'static str>,
            want_warnings: Vec<Option<&'static str>>,
            want_custom: CustomHotkeyLabel,
        }
        let unavailable = "Needs Typing Access. Go Back to turn it on.";
        let cases = [
            Case {
                name: "lone modifiers disabled without typing access",
                presets: vec![
                    preset("⌥ Space", false, true, None),
                    preset("Right ⌥", true, false, None),
                    preset("fn", true, false, Some("Fn opens the emoji picker")),
                ],
                accessibility: false,
                want_enabled: vec![true, false, false],
                want_details: vec!["⌥ Space detail", unavailable, unavailable],
                want_warnings: vec![None, None, None],
                want_custom: CustomHotkeyLabel::Text("Another shortcut"),
            },
            Case {
                name: "all enabled with typing access, fn warning while selected",
                presets: vec![
                    preset("⌥ Space", false, false, None),
                    preset("Right ⌥", true, false, None),
                    preset("fn", true, true, Some("Fn opens the emoji picker")),
                ],
                accessibility: true,
                want_enabled: vec![true, true, true],
                want_details: vec!["⌥ Space detail", "Right ⌥ detail", "fn detail"],
                want_warnings: vec![None, None, Some("Fn opens the emoji picker")],
                want_custom: CustomHotkeyLabel::Text("Another shortcut"),
            },
            Case {
                name: "custom shortcut shows its keycaps",
                presets: vec![preset("⌥ Space", false, false, None)],
                accessibility: true,
                want_enabled: vec![true],
                want_details: vec!["⌥ Space detail"],
                want_warnings: vec![None],
                want_custom: CustomHotkeyLabel::Keycaps(vec!["⌃".to_owned(), "D".to_owned()]),
            },
        ];
        for tc in cases {
            let got = hotkey_step(
                "Hold the hotkey while you talk.",
                &tc.presets,
                &["⌃".to_owned(), "D".to_owned()],
                tc.accessibility,
            );
            assert_eq!(
                got.subtitle, "Hold the hotkey while you talk.",
                "{}",
                tc.name
            );
            assert_eq!(
                got.presets.iter().map(|p| p.enabled).collect::<Vec<_>>(),
                tc.want_enabled,
                "{}",
                tc.name
            );
            assert_eq!(
                got.presets
                    .iter()
                    .map(|p| p.detail.as_str())
                    .collect::<Vec<_>>(),
                tc.want_details,
                "{}",
                tc.name
            );
            assert_eq!(
                got.presets
                    .iter()
                    .map(|p| p.warning.as_deref())
                    .collect::<Vec<_>>(),
                tc.want_warnings,
                "{}",
                tc.name
            );
            assert_eq!(
                got.custom_selected,
                matches!(tc.want_custom, CustomHotkeyLabel::Keycaps(_)),
                "{}",
                tc.name
            );
            assert_eq!(got.custom_label, tc.want_custom, "{}", tc.name);
        }
    }

    fn model(id: &str, recommended: bool, built_in: bool, status: ModelStatus) -> ModelChoice {
        ModelChoice {
            id: id.to_owned(),
            name: id.to_owned(),
            description: format!("{id} description"),
            size_hint: "600 MB".to_owned(),
            recommended,
            built_in,
            status,
        }
    }

    #[test]
    fn onb_8_model_actions() {
        struct Case {
            name: &'static str,
            status: ModelStatus,
            selected: bool,
            want: ModelAction,
            want_title: &'static str,
        }
        let cases = [
            Case {
                name: "downloading",
                status: downloading(0.42),
                selected: true,
                want: ModelAction::Downloading {
                    progress: 0.42,
                    label: "Downloading — 42%".to_owned(),
                },
                want_title: "Cancel",
            },
            Case {
                name: "downloaded and chosen",
                status: DOWNLOADED,
                selected: true,
                want: ModelAction::Selected,
                want_title: "Selected",
            },
            Case {
                name: "downloaded, not chosen",
                status: DOWNLOADED,
                selected: false,
                want: ModelAction::UseThisModel,
                want_title: "Use This Model",
            },
            Case {
                name: "not downloaded",
                status: MISSING,
                selected: true,
                want: ModelAction::Download,
                want_title: "Download",
            },
        ];
        for tc in cases {
            let selected = if tc.selected {
                "parakeet-tdt-v2"
            } else {
                "system"
            };
            let got = model_step(
                &[model("parakeet-tdt-v2", true, false, tc.status)],
                selected,
                false,
                None,
            );
            assert_eq!(got.cards[0].action, tc.want, "{}", tc.name);
            assert_eq!(got.cards[0].action.title(), tc.want_title, "{}", tc.name);
            assert_eq!(got.cards[0].selected, tc.selected, "{}", tc.name);
        }
    }

    #[test]
    fn onb_8_model_layout() {
        struct Case {
            name: &'static str,
            show_others: bool,
            error: Option<&'static str>,
            want_cards: Vec<&'static str>,
            want_toggle: Option<&'static str>,
            want_rows: Vec<&'static str>,
            want_error: Option<&'static str>,
        }
        let models = [
            model("parakeet-tdt-v2", true, false, MISSING),
            model("parakeet-ultra", false, false, MISSING),
            model("parakeet-unified", false, false, DOWNLOADED),
            model("system", false, true, DOWNLOADED),
        ];
        let cases = [
            Case {
                name: "others collapsed",
                show_others: false,
                error: None,
                want_cards: vec!["parakeet-tdt-v2", "system"],
                want_toggle: Some("Show other models"),
                want_rows: vec![],
                want_error: None,
            },
            Case {
                name: "others expanded",
                show_others: true,
                error: None,
                want_cards: vec!["parakeet-tdt-v2", "system"],
                want_toggle: Some("Hide other models"),
                want_rows: vec!["parakeet-ultra", "parakeet-unified"],
                want_error: None,
            },
            Case {
                name: "failed download",
                show_others: false,
                error: Some("Pierce unplugged the router"),
                want_cards: vec!["parakeet-tdt-v2", "system"],
                want_toggle: Some("Show other models"),
                want_rows: vec![],
                want_error: Some("Download failed: Pierce unplugged the router"),
            },
        ];
        for tc in cases {
            let got = model_step(&models, "system", tc.show_others, tc.error);
            assert_eq!(
                got.cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                tc.want_cards,
                "{}",
                tc.name
            );
            assert_eq!(
                got.other_models.as_ref().map(|o| o.toggle_title),
                tc.want_toggle,
                "{}",
                tc.name
            );
            let rows: Vec<&str> = got
                .other_models
                .iter()
                .flat_map(|o| o.rows.iter().map(|r| r.id.as_str()))
                .collect();
            assert_eq!(rows, tc.want_rows, "{}", tc.name);
            assert_eq!(got.error.as_deref(), tc.want_error, "{}", tc.name);
        }
        let only_recommended = model_step(&models[..1], "system", false, None);
        assert_eq!(
            only_recommended.other_models, None,
            "no toggle without other models"
        );
    }

    #[test]
    fn onb_9_ready_step() {
        struct Case {
            name: &'static str,
            history: bool,
            retention: HistoryRetention,
            want_note: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "week",
                history: true,
                retention: HistoryRetention::Week,
                want_note: Some("Transcripts are saved on this Mac for 1 week."),
            },
            Case {
                name: "month",
                history: true,
                retention: HistoryRetention::Month,
                want_note: Some("Transcripts are saved on this Mac for 1 month."),
            },
            Case {
                name: "forever",
                history: true,
                retention: HistoryRetention::Forever,
                want_note: Some("Transcripts are saved on this Mac."),
            },
            Case {
                name: "history off",
                history: false,
                retention: HistoryRetention::Month,
                want_note: None,
            },
        ];
        for tc in cases {
            let got = ready_step(tc.history, tc.retention, OutputMode::Clipboard);
            assert_eq!(got.retention_note.as_deref(), tc.want_note, "{}", tc.name);
            assert_eq!(got.title, "You're all set", "{}", tc.name);
            assert_eq!(
                got.try_it_placeholder, "Click here, dictate, then paste with ⌘V.",
                "{}",
                tc.name
            );
        }
    }
}
