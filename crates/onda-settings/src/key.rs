//! Setting key names (09 "Settings keys").

use std::fmt;

/// One key in `settings.json`. Names are exactly those in prd/09.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// `selectedModel`: speech model id.
    SelectedModel,
    /// `selectedLanguage`: `auto` or an offered language code.
    SelectedLanguage,
    /// `recentLanguages`: comma-separated codes (LANG-8).
    RecentLanguages,
    /// `inputDeviceUID`: input device, empty for the system default.
    InputDeviceUid,
    /// `outputMode`: `type` or `clipboard`.
    OutputMode,
    /// `hotkey`: `{key, modifiers[]}` (HOT-1).
    Hotkey,
    /// `hotkeyActivationMode`: `hold`, `toggle`, or `automatic`.
    HotkeyActivationMode,
    /// `pauseMediaDuringRecording`.
    PauseMediaDuringRecording,
    /// `soundFeedbackEnabled`.
    SoundFeedbackEnabled,
    /// `historyEnabled`.
    HistoryEnabled,
    /// `historyKeepsAudio`.
    HistoryKeepsAudio,
    /// `historyRetention`: `week`, `month`, or `forever`.
    HistoryRetention,
    /// `replacementEntries`: array of rules (REP-7).
    ReplacementEntries,
    /// `postProcessingEnabled`.
    PostProcessingEnabled,
    /// `openRouterModel`: cleanup model slug (CLN-21).
    OpenRouterModel,
    /// `onboardingCompleted` (SET-4).
    OnboardingCompleted,
    /// `didShowFirstRunSetup` (SET-4).
    DidShowFirstRunSetup,
    /// `lastModelMigrationNoticeKey`: `old->new` (DL-14).
    LastModelMigrationNoticeKey,
    /// `declinedModelMigrationNoticeKey`: `old->new` (DL-14).
    DeclinedModelMigrationNoticeKey,
}

impl Key {
    /// Every key, in prd/09 order.
    pub const ALL: [Key; 19] = [
        Key::SelectedModel,
        Key::SelectedLanguage,
        Key::RecentLanguages,
        Key::InputDeviceUid,
        Key::OutputMode,
        Key::Hotkey,
        Key::HotkeyActivationMode,
        Key::PauseMediaDuringRecording,
        Key::SoundFeedbackEnabled,
        Key::HistoryEnabled,
        Key::HistoryKeepsAudio,
        Key::HistoryRetention,
        Key::ReplacementEntries,
        Key::PostProcessingEnabled,
        Key::OpenRouterModel,
        Key::OnboardingCompleted,
        Key::DidShowFirstRunSetup,
        Key::LastModelMigrationNoticeKey,
        Key::DeclinedModelMigrationNoticeKey,
    ];

    /// The JSON key name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Key::SelectedModel => "selectedModel",
            Key::SelectedLanguage => "selectedLanguage",
            Key::RecentLanguages => "recentLanguages",
            Key::InputDeviceUid => "inputDeviceUID",
            Key::OutputMode => "outputMode",
            Key::Hotkey => "hotkey",
            Key::HotkeyActivationMode => "hotkeyActivationMode",
            Key::PauseMediaDuringRecording => "pauseMediaDuringRecording",
            Key::SoundFeedbackEnabled => "soundFeedbackEnabled",
            Key::HistoryEnabled => "historyEnabled",
            Key::HistoryKeepsAudio => "historyKeepsAudio",
            Key::HistoryRetention => "historyRetention",
            Key::ReplacementEntries => "replacementEntries",
            Key::PostProcessingEnabled => "postProcessingEnabled",
            Key::OpenRouterModel => "openRouterModel",
            Key::OnboardingCompleted => "onboardingCompleted",
            Key::DidShowFirstRunSetup => "didShowFirstRunSetup",
            Key::LastModelMigrationNoticeKey => "lastModelMigrationNoticeKey",
            Key::DeclinedModelMigrationNoticeKey => "declinedModelMigrationNoticeKey",
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
