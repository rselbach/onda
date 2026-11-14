//! The onboarding window (ONB-1..ONB-9). The recorder on the Hotkey step
//! renders from `HotkeyChanged`.

use onda_domain::hotkey::{
    GlobeKeyAction, HotkeyPreset, OPEN_KEYBOARD_SETTINGS_TITLE, Shortcut, fn_key_warning,
};
use onda_domain::languages::Language;
use onda_domain::onboarding::{
    CustomHotkeyLabel, HOTKEY_TITLE, HotkeyPresetChoice, LANGUAGE_SEARCH_PLACEHOLDER,
    LANGUAGE_TITLE, LanguageOption, MODEL_TITLE, ModelAction, ModelCard, ModelChoice, Onboarding,
    OnboardingEnv, OnboardingStep, PERMISSION_READY_LABEL, PERMISSIONS_TITLE, PermissionCard,
    WELCOME_SUBTITLE, WELCOME_TITLE, hotkey_step, language_step, model_step, ready_step,
};
use onda_domain::platform::Platform;
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};

use super::PermissionFixButton;

/// The segmented progress bar (ONB-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingProgress {
    /// Capsules.
    pub total: u32,
    /// Capsules filled, up to and including the current step.
    pub filled: u32,
}

/// One language card (ONB-4).
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl From<PermissionCard> for PermissionCardView {
    fn from(card: PermissionCard) -> Self {
        Self {
            title: card.title.to_owned(),
            detail: card.detail.map(str::to_owned),
            granted: card.granted,
            ready_label: PERMISSION_READY_LABEL.to_owned(),
            button: card.button.map(PermissionFixButton::from),
        }
    }
}

/// One hotkey preset card (ONB-7).
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct OtherModelsView {
    /// "Show other models" / "Hide other models"
    /// (`onboarding_toggle_other_models`).
    pub toggle_title: String,
    /// Compact rows; empty while collapsed.
    pub rows: Vec<ModelCardView>,
}

/// The current step's content.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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

/// What the onboarding view is built from.
#[derive(Debug, Clone, Copy)]
pub struct OnboardingInputs<'a> {
    /// The wizard.
    pub wizard: &'a Onboarding,
    /// What it reads from the app.
    pub env: &'a OnboardingEnv<'a>,
    /// The system the shell runs on: it says which hotkey presets need
    /// Accessibility.
    pub platform: Platform,
    /// Language search query.
    pub language_query: &'a str,
    /// "Show All" languages.
    pub show_all_languages: bool,
    /// "Show other models".
    pub show_other_models: bool,
    /// Activation mode.
    pub activation: ActivationMode,
    /// The current shortcut.
    pub shortcut: &'a Shortcut,
    /// macOS's "Press 🌐 key to".
    pub globe: GlobeKeyAction,
    /// Models compatible with the selected language, in route order.
    pub models: &'a [ModelChoice],
    /// The selected model id.
    pub selected_model: &'a str,
    /// Save transcripts.
    pub history_enabled: bool,
    /// Keep for.
    pub retention: HistoryRetention,
    /// Output mode.
    pub output_mode: OutputMode,
}

/// Builds the onboarding view.
pub fn onboarding_view(inputs: OnboardingInputs<'_>) -> OnboardingView {
    let frame = inputs.wizard.frame(inputs.env);
    OnboardingView {
        step: frame.step,
        forward: frame.navigating_forward,
        progress: frame.progress.map(|p| OnboardingProgress {
            total: u32::try_from(p.total).unwrap_or(u32::MAX),
            filled: u32::try_from(p.filled).unwrap_or(u32::MAX),
        }),
        back_visible: frame.back_visible,
        back_title: "Back".to_owned(),
        hint: frame.hint,
        primary_title: frame.primary_title.to_owned(),
        primary_enabled: frame.primary_enabled,
        content: content(&inputs),
    }
}

fn content(inputs: &OnboardingInputs<'_>) -> OnboardingContent {
    match inputs.wizard.step() {
        OnboardingStep::Welcome => OnboardingContent::Welcome {
            title: WELCOME_TITLE.to_owned(),
            subtitle: WELCOME_SUBTITLE.to_owned(),
        },
        OnboardingStep::Language => {
            let option = |language: Language| LanguageOption {
                name: language.name().to_owned(),
                code: language.code().to_owned(),
            };
            let popular: Vec<LanguageOption> = Language::popular().map(option).collect();
            let all: Vec<LanguageOption> = Language::all().map(option).collect();
            let step = language_step(
                inputs.language_query,
                inputs.show_all_languages,
                &popular,
                &all,
                inputs.wizard.language(),
            );
            OnboardingContent::Language {
                title: LANGUAGE_TITLE.to_owned(),
                search_placeholder: LANGUAGE_SEARCH_PLACEHOLDER.to_owned(),
                query: inputs.language_query.to_owned(),
                toggle_title: step.toggle_title.map(str::to_owned),
                cards: step
                    .cards
                    .into_iter()
                    .map(|card| LanguageCardView {
                        code: card.code,
                        name: card.name,
                        code_label: card.code_label,
                        flag: card.flag.to_owned(),
                        selected: card.selected,
                    })
                    .collect(),
            }
        }
        OnboardingStep::Permissions => {
            let step = inputs.wizard.permissions(inputs.env);
            OnboardingContent::Permissions {
                title: PERMISSIONS_TITLE.to_owned(),
                microphone: step.microphone.into(),
                typing_access: step.typing_access.into(),
                clipboard_link: step.clipboard_link.map(str::to_owned),
            }
        }
        OnboardingStep::Hotkey => {
            let choices: Vec<HotkeyPresetChoice> = HotkeyPreset::ALL
                .iter()
                .map(|preset| {
                    let shortcut = preset.shortcut();
                    HotkeyPresetChoice {
                        keycaps: shortcut.keycaps(),
                        detail: preset.detail().to_owned(),
                        needs_accessibility: shortcut.needs_accessibility(inputs.platform),
                        selected: shortcut == *inputs.shortcut,
                        warning: fn_key_warning(&shortcut, inputs.globe).map(str::to_owned),
                    }
                })
                .collect();
            let step = hotkey_step(
                inputs.activation.description(),
                &choices,
                &inputs.shortcut.keycaps(),
                inputs.env.accessibility_granted,
            );
            let (custom_keycaps, custom_label) = match step.custom_label {
                CustomHotkeyLabel::Keycaps(keycaps) => (Some(keycaps), None),
                CustomHotkeyLabel::Text(text) => (None, Some(text.to_owned())),
            };
            OnboardingContent::Hotkey {
                title: HOTKEY_TITLE.to_owned(),
                subtitle: step.subtitle,
                presets: HotkeyPreset::ALL
                    .iter()
                    .zip(step.presets)
                    .map(|(preset, card)| PresetCardView {
                        preset: *preset,
                        keycaps: card.keycaps,
                        detail: card.detail,
                        enabled: card.enabled,
                        selected: card.selected,
                        warning: card.warning,
                    })
                    .collect(),
                custom_selected: step.custom_selected,
                custom_keycaps,
                custom_label,
                open_keyboard_settings_title: OPEN_KEYBOARD_SETTINGS_TITLE.to_owned(),
            }
        }
        OnboardingStep::SpeechModel => {
            let step = model_step(
                inputs.models,
                inputs.selected_model,
                inputs.show_other_models,
                inputs.wizard.download_error(),
            );
            OnboardingContent::SpeechModel {
                title: MODEL_TITLE.to_owned(),
                cards: step.cards.into_iter().map(model_card).collect(),
                other_models: step.other_models.map(|other| OtherModelsView {
                    toggle_title: other.toggle_title.to_owned(),
                    rows: other.rows.into_iter().map(model_card).collect(),
                }),
                error: step.error,
            }
        }
        OnboardingStep::Ready => {
            let step = ready_step(
                inputs.history_enabled,
                inputs.retention,
                inputs.output_mode,
                inputs.platform.computer_term,
            );
            OnboardingContent::Ready {
                title: step.title.to_owned(),
                prompt_prefix: step.prompt_prefix.to_owned(),
                keycaps: inputs.shortcut.keycaps(),
                prompt_suffix: step.prompt_suffix.to_owned(),
                try_it_placeholder: step.try_it_placeholder.to_owned(),
                retention_note: step.retention_note,
            }
        }
    }
}

fn model_card(card: ModelCard) -> ModelCardView {
    let title = card.action.title().to_owned();
    ModelCardView {
        id: card.id,
        name: card.name,
        description: card.description,
        size_hint: card.size_hint,
        recommended: card.recommended.then(|| "Recommended".to_owned()),
        selected: card.selected,
        action: match card.action {
            ModelAction::Downloading { progress, label } => ModelCardAction::Downloading {
                progress,
                label,
                cancel_title: title,
            },
            ModelAction::Selected => ModelCardAction::Selected { title },
            ModelAction::UseThisModel => ModelCardAction::UseThisModel { title },
            ModelAction::Download => ModelCardAction::Download { title },
        },
    }
}
