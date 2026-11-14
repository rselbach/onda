//! What the updater shows (UPD-1..UPD-8): its status, the one update control
//! the menu and the General page share, and the caption under it.

use crate::messages::CaptionTone;

/// Why an update attempt stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFailure {
    /// The feed could not be fetched.
    Check,
    /// The feed failed its signature, identity, or ordering checks.
    Untrusted,
    /// The update archive could not be downloaded.
    Download,
    /// The archive or the app inside it failed verification.
    Verify,
    /// The verified app could not replace the installed one; nothing
    /// changed.
    Install,
    /// The update replaced the installed app, but Onda could neither
    /// schedule its relaunch nor put the previous app back. The update is
    /// in place; quitting and reopening Onda finishes it.
    Relaunch {
        /// The installed version.
        version: String,
    },
    /// The release needs a newer macOS than this Mac runs.
    RequiresNewerOs {
        /// The release's version, e.g. "1.4.0".
        version: String,
        /// The lowest macOS it runs on, e.g. "27.0".
        minimum_os: String,
    },
}

/// The updater's state (UPD-1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    /// This build cannot update itself: no bundle or build number (UPD-4).
    #[default]
    Disabled,
    /// No check has finished since launch.
    Idle,
    /// A check is running.
    Checking,
    /// The feed's release is not newer than this build.
    UpToDate,
    /// The update archive is downloading and being verified (UPD-5).
    Downloading {
        /// The version being downloaded.
        version: String,
        /// Whole percent done, 0–100.
        percent: u8,
    },
    /// A verified update is staged beside the app and waits for a restart
    /// (UPD-6).
    Ready {
        /// The staged version.
        version: String,
    },
    /// The staged update replaced the app; Onda is quitting to relaunch.
    Installing {
        /// The version being installed.
        version: String,
    },
    /// A newer release exists, but this copy of Onda cannot replace itself
    /// (read-only, translocated, or not writable): offer the download
    /// (UPD-7).
    Available {
        /// The newer version.
        version: String,
    },
    /// The last attempt failed; the control retries.
    Failed(UpdateFailure),
}

/// What the update control does when chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Check for updates now.
    Check,
    /// Quit, install the staged update, and relaunch (UPD-6).
    Restart,
    /// Open the release download in the browser (UPD-7).
    Download,
}

/// The update item in the status menu and the button on General.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateControl {
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
    /// What choosing it does.
    pub action: UpdateAction,
}

/// The update control for `status`. `dictating` is true while a session
/// records or transcribes: the restart waits for it (UPD-8).
pub fn update_control(status: &UpdateStatus, dictating: bool) -> UpdateControl {
    let control = |title: String, enabled, action| UpdateControl {
        title,
        enabled,
        action,
    };
    let check = |enabled| {
        control(
            "Check for Updates…".to_owned(),
            enabled,
            UpdateAction::Check,
        )
    };
    match status {
        UpdateStatus::Disabled | UpdateStatus::Failed(UpdateFailure::Relaunch { .. }) => {
            check(false)
        }
        UpdateStatus::Idle | UpdateStatus::UpToDate | UpdateStatus::Failed(_) => check(true),
        UpdateStatus::Checking => control(
            "Checking for Updates…".to_owned(),
            false,
            UpdateAction::Check,
        ),
        UpdateStatus::Downloading { .. } => {
            control("Downloading Update…".to_owned(), false, UpdateAction::Check)
        }
        UpdateStatus::Ready { .. } => control(
            "Restart to Update".to_owned(),
            !dictating,
            UpdateAction::Restart,
        ),
        UpdateStatus::Installing { .. } => {
            control("Restarting…".to_owned(), false, UpdateAction::Restart)
        }
        UpdateStatus::Available { version } => control(
            format!("Download Onda {version}…"),
            true,
            UpdateAction::Download,
        ),
    }
}

/// The line under the update control on General.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCaption {
    /// Text.
    pub text: String,
    /// Orange for a failure, else secondary.
    pub tone: CaptionTone,
}

/// The caption for `status`; `None` shows nothing. `os_name` is the system
/// a release can require a newer version of.
pub fn update_caption(
    status: &UpdateStatus,
    dictating: bool,
    os_name: &str,
) -> Option<UpdateCaption> {
    let text = match status {
        UpdateStatus::Disabled => "Updates are off in development builds.".to_owned(),
        UpdateStatus::Idle => return None,
        UpdateStatus::Checking => "Checking for updates…".to_owned(),
        UpdateStatus::UpToDate => "Onda is up to date.".to_owned(),
        UpdateStatus::Downloading { version, percent } => {
            format!("Downloading Onda {version}… {percent}%")
        }
        UpdateStatus::Ready { version } if dictating => {
            format!("Onda {version} is ready. Restart when dictation finishes.")
        }
        UpdateStatus::Ready { version } => {
            format!("Onda {version} is ready. Restart Onda to finish updating.")
        }
        UpdateStatus::Installing { version } => format!("Installing Onda {version}…"),
        UpdateStatus::Available { version } => format!(
            "Onda {version} is available. This copy of Onda can't replace itself, so download \
             the new version and move it to Applications."
        ),
        UpdateStatus::Failed(failure) => failure_text(failure, os_name),
    };
    let tone = match status {
        UpdateStatus::Failed(_) => CaptionTone::Warning,
        _ => CaptionTone::Secondary,
    };
    Some(UpdateCaption { text, tone })
}

fn failure_text(failure: &UpdateFailure, os_name: &str) -> String {
    match failure {
        UpdateFailure::Check => {
            "Couldn't check for updates. Check your internet connection and try again.".to_owned()
        }
        UpdateFailure::Untrusted => {
            "Couldn't verify the update information, so Onda ignored it. Try again later."
                .to_owned()
        }
        UpdateFailure::Download => "Couldn't download the update. Try again later.".to_owned(),
        UpdateFailure::Verify => {
            "The downloaded update failed verification, so Onda didn't install it.".to_owned()
        }
        UpdateFailure::Install => {
            "Couldn't install the update. Try again, or download Onda from its GitHub releases."
                .to_owned()
        }
        UpdateFailure::Relaunch { version } => format!(
            "Onda {version} is installed, but Onda couldn't restart itself. Quit Onda and open \
             it again."
        ),
        UpdateFailure::RequiresNewerOs {
            version,
            minimum_os,
        } => format!("Onda {version} requires {os_name} {minimum_os} or later."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upd_control_follows_status() {
        struct Case {
            status: UpdateStatus,
            dictating: bool,
            want_title: &'static str,
            want_enabled: bool,
            want_action: UpdateAction,
        }
        let ready = || UpdateStatus::Ready {
            version: "2.0.0".to_owned(),
        };
        let cases = [
            Case {
                status: UpdateStatus::Disabled,
                dictating: false,
                want_title: "Check for Updates…",
                want_enabled: false,
                want_action: UpdateAction::Check,
            },
            Case {
                status: UpdateStatus::UpToDate,
                dictating: true,
                want_title: "Check for Updates…",
                want_enabled: true,
                want_action: UpdateAction::Check,
            },
            Case {
                status: UpdateStatus::Checking,
                dictating: false,
                want_title: "Checking for Updates…",
                want_enabled: false,
                want_action: UpdateAction::Check,
            },
            Case {
                status: UpdateStatus::Downloading {
                    version: "2.0.0".to_owned(),
                    percent: 40,
                },
                dictating: false,
                want_title: "Downloading Update…",
                want_enabled: false,
                want_action: UpdateAction::Check,
            },
            Case {
                status: ready(),
                dictating: false,
                want_title: "Restart to Update",
                want_enabled: true,
                want_action: UpdateAction::Restart,
            },
            Case {
                status: ready(),
                dictating: true,
                want_title: "Restart to Update",
                want_enabled: false,
                want_action: UpdateAction::Restart,
            },
            Case {
                status: UpdateStatus::Available {
                    version: "2.0.0".to_owned(),
                },
                dictating: false,
                want_title: "Download Onda 2.0.0…",
                want_enabled: true,
                want_action: UpdateAction::Download,
            },
            Case {
                status: UpdateStatus::Failed(UpdateFailure::Download),
                dictating: false,
                want_title: "Check for Updates…",
                want_enabled: true,
                want_action: UpdateAction::Check,
            },
            Case {
                status: UpdateStatus::Failed(UpdateFailure::Relaunch {
                    version: "2.0.0".to_owned(),
                }),
                dictating: false,
                want_title: "Check for Updates…",
                want_enabled: false,
                want_action: UpdateAction::Check,
            },
        ];
        for tc in cases {
            let got = update_control(&tc.status, tc.dictating);
            assert_eq!(got.title, tc.want_title, "{:?}", tc.status);
            assert_eq!(got.enabled, tc.want_enabled, "{:?}", tc.status);
            assert_eq!(got.action, tc.want_action, "{:?}", tc.status);
        }
    }

    #[test]
    fn upd_captions() {
        let newer_os = || {
            UpdateStatus::Failed(UpdateFailure::RequiresNewerOs {
                version: "2.0.0".to_owned(),
                minimum_os: "27.0".to_owned(),
            })
        };
        let cases = [
            (UpdateStatus::Idle, false, "macOS", None),
            (
                UpdateStatus::Downloading {
                    version: "2.0.0".to_owned(),
                    percent: 42,
                },
                false,
                "macOS",
                Some("Downloading Onda 2.0.0… 42%"),
            ),
            (
                UpdateStatus::Ready {
                    version: "2.0.0".to_owned(),
                },
                true,
                "macOS",
                Some("Onda 2.0.0 is ready. Restart when dictation finishes."),
            ),
            (
                newer_os(),
                false,
                "macOS",
                Some("Onda 2.0.0 requires macOS 27.0 or later."),
            ),
            (
                newer_os(),
                false,
                "Linux",
                Some("Onda 2.0.0 requires Linux 27.0 or later."),
            ),
            (
                UpdateStatus::Failed(UpdateFailure::Relaunch {
                    version: "2.0.0".to_owned(),
                }),
                false,
                "macOS",
                Some(
                    "Onda 2.0.0 is installed, but Onda couldn't restart itself. Quit Onda and \
                     open it again.",
                ),
            ),
        ];
        for (status, dictating, os_name, want) in cases {
            let got = update_caption(&status, dictating, os_name);
            assert_eq!(
                got.as_ref().map(|caption| caption.text.as_str()),
                want,
                "{status:?}"
            );
            let warning = got.is_some_and(|caption| caption.tone == CaptionTone::Warning);
            assert_eq!(
                warning,
                matches!(status, UpdateStatus::Failed(_)),
                "{status:?}"
            );
        }
    }
}
