//! The General page (WIN-8..WIN-15, LOGIN-1..LOGIN-3, UPD-1, UPD-8).

use std::cmp::Ordering;
use std::time::SystemTime;

use onda_domain::messages::{
    CaptionTone, LOGIN_ITEM_TOGGLE_LABEL, LoginItemError, LoginItemStatus, login_item_caption,
};
use onda_domain::platform::Platform;
use onda_domain::setup::{accessibility_row, microphone_row};
use onda_domain::types::{MicrophonePermission, OutputMode};
use onda_domain::updates::{
    UpdateCaption, UpdateControl, UpdateStatus, update_caption, update_control,
};

use super::{PermissionRowView, ToggleView};
use crate::seams::InputDeviceInfo;

/// The picker entry for following the OS default input (WIN-9).
const SYSTEM_DEFAULT: &str = "System Default";
/// A selected device that is not connected (AUD-3).
const UNAVAILABLE_MICROPHONE: &str = "Unavailable Microphone";
/// The orange note under a missing device (AUD-3).
const UNAVAILABLE_WARNING: &str =
    "The selected microphone is not connected. Onda will not fall back to another input.";

/// The updater's state, read from its port (UPD-1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdaterState {
    /// What the updater is doing.
    pub status: UpdateStatus,
    /// "Check for updates automatically".
    pub automatic: bool,
    /// When the updater last checked.
    pub last_check: Option<SystemTime>,
}

/// What the General page is built from.
#[derive(Debug, Clone, Copy)]
pub struct GeneralInputs<'a> {
    /// Microphone permission.
    pub microphone: MicrophonePermission,
    /// Accessibility is granted.
    pub accessibility_granted: bool,
    /// Accessibility is needed by the current settings (UI-8).
    pub accessibility_needed: bool,
    /// The system the shell runs on: it names the Accessibility
    /// permission and the place "System Default" follows (WIN-9).
    pub platform: Platform,
    /// Connected input devices.
    pub devices: &'a [InputDeviceInfo],
    /// `inputDeviceUID`; empty for the system default.
    pub selected_uid: &'a str,
    /// Output mode.
    pub output_mode: OutputMode,
    /// Pause media while recording.
    pub pause_media: bool,
    /// Play sounds.
    pub sounds: bool,
    /// Launch-at-login status.
    pub login: LoginItemStatus,
    /// The last failed toggle, kept until the next success (LOGIN-2).
    pub login_error: Option<&'a LoginItemError>,
    /// The updater's state.
    pub updater: &'a UpdaterState,
    /// A session records or transcribes, so the restart waits (UPD-8).
    pub dictating: bool,
    /// A restart was accepted and is in progress (UPD-8).
    pub restarting: bool,
    /// The app version, e.g. "1.2.0".
    pub version: &'a str,
}

/// One input picker entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputOption {
    /// Value for `select_input_device`; empty for "System Default".
    pub uid: String,
    /// Display name.
    pub name: String,
}

/// The "Input" picker (WIN-9, AUD-3).
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputModeOption {
    /// The mode.
    pub mode: OutputMode,
    /// Title.
    pub title: String,
}

/// The "After transcription" radio group (WIN-10).
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCaptionView {
    /// Text.
    pub text: String,
    /// Orange or secondary.
    pub tone: CaptionTone,
    /// "Open Login Items", which calls `open_login_items`.
    pub button: Option<String>,
}

/// The General page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Builds the General page.
pub fn general_page(inputs: GeneralInputs<'_>) -> GeneralPage {
    GeneralPage {
        microphone: microphone_row(inputs.microphone).into(),
        accessibility: accessibility_row(
            inputs.accessibility_granted,
            inputs.accessibility_needed,
            inputs.platform,
        )
        .into(),
        input: input_picker(inputs.devices, inputs.selected_uid, inputs.platform.os_name),
        output: OutputModePicker {
            label: "After transcription".to_owned(),
            options: vec![
                OutputModeOption {
                    mode: OutputMode::Clipboard,
                    title: "Copy to clipboard".to_owned(),
                },
                OutputModeOption {
                    mode: OutputMode::Type,
                    title: "Type into active app".to_owned(),
                },
            ],
            selected: inputs.output_mode,
            caption: (inputs.output_mode == OutputMode::Clipboard)
                .then(|| "Output will be copied to the clipboard only.".to_owned()),
        },
        pause_media: ToggleView::new("Pause media while recording", inputs.pause_media, true),
        pause_media_caption: "When enabled, Onda pauses current media playback when dictation \
                              starts and resumes only if Onda paused it. Audio it cannot pause \
                              is muted until dictation ends."
            .to_owned(),
        sounds: ToggleView::new(
            "Play sounds when dictation starts and stops",
            inputs.sounds,
            true,
        ),
        launch_at_login: ToggleView::new(
            LOGIN_ITEM_TOGGLE_LABEL,
            inputs.login.is_on(),
            inputs.login.toggle_enabled(),
        ),
        launch_at_login_caption: login_item_caption(inputs.login, inputs.login_error).map(
            |caption| LoginCaptionView {
                text: caption.text,
                tone: caption.tone,
                button: caption.button.map(|button| button.title().to_owned()),
            },
        ),
        automatic_updates: ToggleView::new(
            "Check for updates automatically",
            inputs.updater.automatic,
            inputs.updater.status != UpdateStatus::Disabled,
        ),
        update: update_control(
            &inputs.updater.status,
            inputs.dictating || inputs.restarting,
        ),
        update_caption: update_caption(
            &inputs.updater.status,
            inputs.dictating,
            inputs.platform.os_name,
        ),
        version_footer: format!("Current version: {}", inputs.version),
        last_update_check: inputs.updater.last_check,
    }
}

fn input_picker(devices: &[InputDeviceInfo], selected_uid: &str, os_name: &str) -> InputPickerView {
    let mut sorted: Vec<&InputDeviceInfo> = devices.iter().collect();
    sorted.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    let mut options = vec![InputOption {
        uid: String::new(),
        name: SYSTEM_DEFAULT.to_owned(),
    }];
    options.extend(sorted.iter().map(|device| InputOption {
        uid: device.uid.clone(),
        name: device.name.clone(),
    }));
    let selected = devices.iter().find(|device| device.uid == selected_uid);
    let (warning, caption) = match (selected_uid, selected) {
        ("", _) => {
            let caption = match devices.iter().find(|device| device.is_default) {
                Some(device) => format!(
                    "Follows the input selected in {os_name}. Currently using {}.",
                    device.name
                ),
                None => format!("Follows the input selected in {os_name}."),
            };
            (None, Some(caption))
        }
        (_, Some(device)) => (
            None,
            Some(format!(
                "Onda will keep using {} when the system default changes.",
                device.name
            )),
        ),
        (uid, None) => {
            options.push(InputOption {
                uid: uid.to_owned(),
                name: UNAVAILABLE_MICROPHONE.to_owned(),
            });
            (Some(UNAVAILABLE_WARNING.to_owned()), None)
        }
    };
    InputPickerView {
        label: "Input".to_owned(),
        options,
        selected_uid: selected_uid.to_owned(),
        warning,
        caption,
    }
}

/// Natural order for names (WIN-9): case-insensitive, with digit runs
/// compared by value, so "Mic 2" sorts before "Mic 10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) if l.is_ascii_digit() && r.is_ascii_digit() => {
                let ln = digits(&mut left);
                let rn = digits(&mut right);
                let order = ln
                    .trim_start_matches('0')
                    .len()
                    .cmp(&rn.trim_start_matches('0').len())
                    .then_with(|| ln.trim_start_matches('0').cmp(rn.trim_start_matches('0')));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(l), Some(r)) => {
                let order = l.to_lowercase().cmp(r.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left.next();
                right.next();
            }
        }
    }
}

fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
        run.push(c);
        chars.next();
    }
    run
}

#[cfg(test)]
mod tests {
    use onda_domain::privacy_naming::PrivacyNaming;

    use super::*;

    fn device(uid: &str, name: &str, is_default: bool) -> InputDeviceInfo {
        InputDeviceInfo {
            uid: uid.to_owned(),
            name: name.to_owned(),
            is_default,
        }
    }

    #[test]
    fn natural_cmp_orders_digit_runs_by_value() {
        let cases = [
            ("Mic 2", "Mic 10", Ordering::Less),
            ("mic a", "Mic B", Ordering::Less),
            ("Mic 10", "Mic 9", Ordering::Greater),
            ("Mic", "Mic 1", Ordering::Less),
            ("Mic 02", "Mic 2", Ordering::Less),
        ];
        for (a, b, want) in cases {
            assert_eq!(natural_cmp(a, b), want, "{a} vs {b}");
        }
    }

    #[test]
    fn win_9_input_picker() {
        struct Case {
            name: &'static str,
            platform: Platform,
            devices: Vec<InputDeviceInfo>,
            selected: &'static str,
            want_names: Vec<&'static str>,
            want_warning: bool,
            want_caption: Option<&'static str>,
        }
        let macos = Platform::macos(PrivacyNaming::Accessibility);
        let cases = [
            Case {
                name: "system default names the current default",
                platform: macos,
                devices: vec![
                    device("usb-10", "Greendale USB 10", false),
                    device("usb-2", "Greendale USB 2", false),
                    device("built-in", "MacBook Pro Microphone", true),
                ],
                selected: "",
                want_names: vec![
                    "System Default",
                    "Greendale USB 2",
                    "Greendale USB 10",
                    "MacBook Pro Microphone",
                ],
                want_warning: false,
                want_caption: Some(
                    "Follows the input selected in macOS. Currently using MacBook Pro Microphone.",
                ),
            },
            Case {
                name: "system default on Linux names Linux",
                platform: Platform::linux(),
                devices: vec![device("alsa-1", "Greendale USB 2", true)],
                selected: "",
                want_names: vec!["System Default", "Greendale USB 2"],
                want_warning: false,
                want_caption: Some(
                    "Follows the input selected in Linux. Currently using Greendale USB 2.",
                ),
            },
            Case {
                name: "system default on Linux with no default device",
                platform: Platform::linux(),
                devices: vec![device("alsa-1", "Greendale USB 2", false)],
                selected: "",
                want_names: vec!["System Default", "Greendale USB 2"],
                want_warning: false,
                want_caption: Some("Follows the input selected in Linux."),
            },
            Case {
                name: "a specific connected device",
                platform: macos,
                devices: vec![device("usb-2", "Greendale USB 2", true)],
                selected: "usb-2",
                want_names: vec!["System Default", "Greendale USB 2"],
                want_warning: false,
                want_caption: Some(
                    "Onda will keep using Greendale USB 2 when the system default changes.",
                ),
            },
            Case {
                name: "a missing device shows as unavailable with the AUD-3 note",
                platform: macos,
                devices: vec![device("built-in", "MacBook Pro Microphone", true)],
                selected: "troy-podcast-mic",
                want_names: vec![
                    "System Default",
                    "MacBook Pro Microphone",
                    "Unavailable Microphone",
                ],
                want_warning: true,
                want_caption: None,
            },
        ];
        for tc in cases {
            let got = input_picker(&tc.devices, tc.selected, tc.platform.os_name);
            let names: Vec<&str> = got.options.iter().map(|o| o.name.as_str()).collect();
            assert_eq!(names, tc.want_names, "{}", tc.name);
            assert_eq!(got.warning.is_some(), tc.want_warning, "{}", tc.name);
            assert_eq!(got.caption.as_deref(), tc.want_caption, "{}", tc.name);
            assert_eq!(got.selected_uid, tc.selected, "{}", tc.name);
        }
    }
}
