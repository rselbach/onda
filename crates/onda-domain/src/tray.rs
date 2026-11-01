//! Tray icon view model (UI-5, SET-4).

use crate::non_dictation_ui::NonDictationState;
use crate::privacy_naming::PrivacyNaming;
use crate::setup::MissingPermission;
use crate::types::SessionPhase;

/// The app-level state the tray icon and the menu header describe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppStatus {
    /// Downloading or loading a model.
    Preparing {
        /// Fraction in `[0, 1]`, or `None` when unknown.
        progress: Option<f64>,
    },
    /// A model is loaded and dictation would work.
    Ready,
    /// The recording UI is up.
    Recording,
    /// Transcribing a finished recording.
    Transcribing,
    /// No usable model.
    Unavailable,
}

impl AppStatus {
    /// Combines the session phase with the non-dictation state.
    ///
    /// A provisional recording has not shown its UI yet (HOT-30), so it
    /// keeps the non-dictation status until it is confirmed.
    pub fn resolve(phase: SessionPhase, non_dictation: &NonDictationState) -> Self {
        match phase {
            SessionPhase::Recording { provisional: false } => Self::Recording,
            SessionPhase::Transcribing => Self::Transcribing,
            SessionPhase::Idle | SessionPhase::Recording { provisional: true } => {
                Self::from_non_dictation(non_dictation)
            }
        }
    }

    fn from_non_dictation(state: &NonDictationState) -> Self {
        match state {
            NonDictationState::Ready => Self::Ready,
            NonDictationState::Downloading(download) => Self::Preparing {
                progress: Some(download.progress),
            },
            NonDictationState::Loading { progress, .. } => Self::Preparing {
                progress: *progress,
            },
            NonDictationState::Error => Self::Unavailable,
        }
    }
}

/// Whole percent for a progress fraction, clamped to `0..=100`.
///
/// Truncates like Jabber, with a small epsilon so 0.29 reads 29, not 28.
pub fn percent(progress: f64) -> u32 {
    let scaled = (progress.clamp(0.0, 1.0) * 100.0 + 1e-9).floor();
    // clamped above, so the cast cannot truncate meaningfully; NaN casts to 0
    scaled as u32
}

/// Platform-neutral tray icon. The shell maps each to its own image; on
/// macOS: `waveform`, `waveform.circle.fill`, `ellipsis.circle`,
/// `arrow.down.circle`, the custom ring (UI-5), `exclamationmark.triangle`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayIcon {
    /// Ready to dictate.
    Ready,
    /// Recording.
    Recording,
    /// Transcribing.
    Transcribing,
    /// Preparing a model, progress unknown.
    Preparing,
    /// Preparing a model with known progress: a ring filling clockwise from
    /// 12 o'clock around a down arrow (UI-5).
    ProgressRing {
        /// Fraction in `[0, 1]`.
        progress: f64,
    },
    /// Model unavailable, setup unfinished, or a permission missing.
    Warning,
}

/// Everything the shell needs to draw the tray icon (`TrayChanged`).
#[derive(Debug, Clone, PartialEq)]
pub struct TrayView {
    /// Icon to show.
    pub icon: TrayIcon,
    /// Tint the icon red (recording only).
    pub red_tint: bool,
    /// What screen readers announce.
    pub accessibility_label: String,
}

/// Builds the tray view model.
///
/// A missing permission only matters while ready; first-run pending
/// (SET-4) only changes the unavailable label. `naming` is the OS's name
/// for the Accessibility permission.
pub fn tray_view(
    status: AppStatus,
    missing_permission: Option<MissingPermission>,
    first_run_pending: bool,
    naming: PrivacyNaming,
) -> TrayView {
    let (icon, label) = match status {
        AppStatus::Ready => match missing_permission {
            None => (TrayIcon::Ready, "Onda, ready to dictate".to_owned()),
            Some(MissingPermission::Microphone) => (
                TrayIcon::Warning,
                "Onda, microphone access needed".to_owned(),
            ),
            Some(MissingPermission::Accessibility) => {
                (TrayIcon::Warning, naming.needed_label().to_owned())
            }
        },
        AppStatus::Recording => (TrayIcon::Recording, "Onda, recording".to_owned()),
        AppStatus::Transcribing => (TrayIcon::Transcribing, "Onda, transcribing".to_owned()),
        AppStatus::Preparing { progress: None } => (
            TrayIcon::Preparing,
            "Onda, preparing speech model".to_owned(),
        ),
        AppStatus::Preparing {
            progress: Some(progress),
        } => (
            TrayIcon::ProgressRing {
                progress: progress.clamp(0.0, 1.0),
            },
            format!(
                "Onda, preparing speech model, {} percent",
                percent(progress)
            ),
        ),
        AppStatus::Unavailable if first_run_pending => (
            TrayIcon::Warning,
            "Onda, finish setup to dictate".to_owned(),
        ),
        AppStatus::Unavailable => (TrayIcon::Warning, "Onda, model unavailable".to_owned()),
    };
    TrayView {
        icon,
        red_tint: status == AppStatus::Recording,
        accessibility_label: label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::non_dictation_ui::ModelDownload;

    #[test]
    fn tray_icon_states() {
        struct Case {
            name: &'static str,
            status: AppStatus,
            missing: Option<MissingPermission>,
            pending: bool,
            want_icon: TrayIcon,
            want_red: bool,
            want_label: &'static str,
        }
        let cases = [
            Case {
                name: "ready",
                status: AppStatus::Ready,
                missing: None,
                pending: false,
                want_icon: TrayIcon::Ready,
                want_red: false,
                want_label: "Onda, ready to dictate",
            },
            Case {
                name: "recording is red",
                status: AppStatus::Recording,
                missing: None,
                pending: false,
                want_icon: TrayIcon::Recording,
                want_red: true,
                want_label: "Onda, recording",
            },
            Case {
                name: "transcribing",
                status: AppStatus::Transcribing,
                missing: None,
                pending: false,
                want_icon: TrayIcon::Transcribing,
                want_red: false,
                want_label: "Onda, transcribing",
            },
            Case {
                name: "preparing without progress",
                status: AppStatus::Preparing { progress: None },
                missing: None,
                pending: false,
                want_icon: TrayIcon::Preparing,
                want_red: false,
                want_label: "Onda, preparing speech model",
            },
            Case {
                name: "UI-5 preparing with known progress",
                status: AppStatus::Preparing {
                    progress: Some(0.426),
                },
                missing: None,
                pending: false,
                want_icon: TrayIcon::ProgressRing { progress: 0.426 },
                want_red: false,
                want_label: "Onda, preparing speech model, 42 percent",
            },
            Case {
                name: "unavailable",
                status: AppStatus::Unavailable,
                missing: None,
                pending: false,
                want_icon: TrayIcon::Warning,
                want_red: false,
                want_label: "Onda, model unavailable",
            },
            Case {
                name: "SET-4 unavailable before first-run setup",
                status: AppStatus::Unavailable,
                missing: None,
                pending: true,
                want_icon: TrayIcon::Warning,
                want_red: false,
                want_label: "Onda, finish setup to dictate",
            },
            Case {
                name: "ready but microphone missing",
                status: AppStatus::Ready,
                missing: Some(MissingPermission::Microphone),
                pending: false,
                want_icon: TrayIcon::Warning,
                want_red: false,
                want_label: "Onda, microphone access needed",
            },
            Case {
                name: "ready but accessibility missing",
                status: AppStatus::Ready,
                missing: Some(MissingPermission::Accessibility),
                pending: false,
                want_icon: TrayIcon::Warning,
                want_red: false,
                want_label: "Onda, accessibility access needed",
            },
            Case {
                name: "missing permission ignored while recording",
                status: AppStatus::Recording,
                missing: Some(MissingPermission::Accessibility),
                pending: false,
                want_icon: TrayIcon::Recording,
                want_red: true,
                want_label: "Onda, recording",
            },
            Case {
                name: "missing permission wins over pending setup while ready",
                status: AppStatus::Ready,
                missing: Some(MissingPermission::Microphone),
                pending: true,
                want_icon: TrayIcon::Warning,
                want_red: false,
                want_label: "Onda, microphone access needed",
            },
        ];
        for tc in cases {
            let got = tray_view(
                tc.status,
                tc.missing,
                tc.pending,
                PrivacyNaming::Accessibility,
            );
            assert_eq!(got.icon, tc.want_icon, "{}", tc.name);
            assert_eq!(got.red_tint, tc.want_red, "{}", tc.name);
            assert_eq!(got.accessibility_label, tc.want_label, "{}", tc.name);
        }
    }

    #[test]
    fn accessibility_label_follows_privacy_naming() {
        let cases = [
            (
                PrivacyNaming::Accessibility,
                "Onda, accessibility access needed",
            ),
            (
                PrivacyNaming::DeviceControlAndDataAccess,
                "Onda, device control and data access needed",
            ),
        ];
        for (naming, want) in cases {
            let got = tray_view(
                AppStatus::Ready,
                Some(MissingPermission::Accessibility),
                false,
                naming,
            );
            assert_eq!(got.icon, TrayIcon::Warning, "{naming:?}");
            assert_eq!(got.accessibility_label, want, "{naming:?}");
        }
    }

    #[test]
    fn app_status_resolve() {
        struct Case {
            name: &'static str,
            phase: SessionPhase,
            state: NonDictationState,
            want: AppStatus,
        }
        let download = ModelDownload {
            model_id: "base".to_owned(),
            progress: 0.5,
            status: "Downloading Base... 50%".to_owned(),
        };
        let cases = [
            Case {
                name: "idle ready",
                phase: SessionPhase::Idle,
                state: NonDictationState::Ready,
                want: AppStatus::Ready,
            },
            Case {
                name: "idle downloading",
                phase: SessionPhase::Idle,
                state: NonDictationState::Downloading(download),
                want: AppStatus::Preparing {
                    progress: Some(0.5),
                },
            },
            Case {
                name: "idle loading",
                phase: SessionPhase::Idle,
                state: NonDictationState::Loading {
                    status: "Loading model...".to_owned(),
                    progress: None,
                },
                want: AppStatus::Preparing { progress: None },
            },
            Case {
                name: "idle error",
                phase: SessionPhase::Idle,
                state: NonDictationState::Error,
                want: AppStatus::Unavailable,
            },
            Case {
                name: "confirmed recording",
                phase: SessionPhase::Recording { provisional: false },
                state: NonDictationState::Ready,
                want: AppStatus::Recording,
            },
            Case {
                name: "provisional recording keeps ready",
                phase: SessionPhase::Recording { provisional: true },
                state: NonDictationState::Ready,
                want: AppStatus::Ready,
            },
            Case {
                name: "transcribing",
                phase: SessionPhase::Transcribing,
                state: NonDictationState::Ready,
                want: AppStatus::Transcribing,
            },
        ];
        for tc in cases {
            assert_eq!(
                AppStatus::resolve(tc.phase, &tc.state),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn percent_truncates_and_clamps() {
        struct Case {
            name: &'static str,
            progress: f64,
            want: u32,
        }
        let cases = [
            Case {
                name: "truncates",
                progress: 0.426,
                want: 42,
            },
            Case {
                name: "float noise",
                progress: 0.29,
                want: 29,
            },
            Case {
                name: "zero",
                progress: 0.0,
                want: 0,
            },
            Case {
                name: "full",
                progress: 1.0,
                want: 100,
            },
            Case {
                name: "over one clamps",
                progress: 1.7,
                want: 100,
            },
            Case {
                name: "negative clamps",
                progress: -0.2,
                want: 0,
            },
            Case {
                name: "NaN is zero",
                progress: f64::NAN,
                want: 0,
            },
        ];
        for tc in cases {
            assert_eq!(percent(tc.progress), tc.want, "{}", tc.name);
        }
    }
}
