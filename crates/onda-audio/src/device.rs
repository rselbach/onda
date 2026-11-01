//! Input selection, device listing types, and the optional device-change
//! port (AUD-2, AUD-3, AUD-10).

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::capture::Command;

/// Which microphone to record from (AUD-2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum DeviceSelection {
    /// Follow the OS default input, including changes while running.
    #[default]
    SystemDefault,
    /// One device by its stable unique id ([`InputDevice::id`]). Never falls
    /// back to another input when it is missing (AUD-3).
    Device(String),
}

impl DeviceSelection {
    /// Reads the `inputDeviceUID` setting: empty means the system default.
    pub fn from_setting(uid: &str) -> Self {
        match uid {
            "" => Self::SystemDefault,
            uid => Self::Device(uid.to_owned()),
        }
    }

    /// The value to store in the `inputDeviceUID` setting.
    pub fn as_setting(&self) -> &str {
        match self {
            Self::SystemDefault => "",
            Self::Device(uid) => uid,
        }
    }
}

/// One input device for the settings picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    /// Stable unique id that survives reconnects and restarts. On macOS this
    /// is the CoreAudio device UID, the same value Jabber stored.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this is the current OS default input.
    pub is_default: bool,
}

/// A change in the OS audio device setup (AUD-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceChange {
    /// The OS default input changed.
    DefaultInputChanged,
    /// A device appeared or disappeared.
    DeviceListChanged,
}

/// Something else told about each device change, such as a settings page
/// that lists devices (WIN-9).
pub type DeviceChangeListener = Arc<dyn Fn(DeviceChange) + Send + Sync>;

/// Where device changes go: a handle into a running
/// [`AudioCapture`](crate::AudioCapture), plus an optional listener. Cheap to
/// clone and send.
#[derive(Clone)]
pub struct DeviceChangeSink {
    commands: Sender<Command>,
    listener: Option<DeviceChangeListener>,
}

impl fmt::Debug for DeviceChangeSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceChangeSink")
            .field("listener", &self.listener.is_some())
            .finish_non_exhaustive()
    }
}

impl DeviceChangeSink {
    pub(crate) fn new(commands: Sender<Command>) -> Self {
        Self {
            commands,
            listener: None,
        }
    }

    /// Also calls `listener` after each change reaches the capture.
    pub fn with_listener(self, listener: DeviceChangeListener) -> Self {
        Self {
            listener: Some(listener),
            ..self
        }
    }

    /// Reports `change`. Returns false once the capture is gone, so a feeder
    /// can stop.
    pub fn notify(&self, change: DeviceChange) -> bool {
        if self.commands.send(Command::DeviceChanged(change)).is_err() {
            return false;
        }
        if let Some(listener) = &self.listener {
            listener(change);
        }
        true
    }
}

/// Port for OS device-change notifications.
///
/// cpal reports a lost device or a format change on an open stream, but not
/// default-input or device-list changes, so something else must feed them.
/// A shell MAY implement this with OS notifications;
/// [`PollingDeviceEvents`](crate::PollingDeviceEvents) is a portable fallback.
pub trait AudioDeviceEvents {
    /// Starts delivering changes to `sink` until the capture is gone or this
    /// value is dropped.
    fn subscribe(&mut self, sink: DeviceChangeSink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc;

    #[test]
    fn listener_hears_changes_that_reach_the_capture_win_9() {
        struct Case {
            name: &'static str,
            capture_alive: bool,
            want_delivered: bool,
            want_heard: Vec<DeviceChange>,
        }
        let cases = [
            Case {
                name: "capture running",
                capture_alive: true,
                want_delivered: true,
                want_heard: vec![DeviceChange::DeviceListChanged],
            },
            Case {
                name: "capture gone",
                capture_alive: false,
                want_delivered: false,
                want_heard: Vec::new(),
            },
        ];
        for tc in cases {
            let (commands, inbox) = mpsc::channel();
            if !tc.capture_alive {
                drop(inbox);
            }
            let heard = Arc::new(Mutex::new(Vec::new()));
            let seen = heard.clone();
            let sink = DeviceChangeSink::new(commands).with_listener(Arc::new(move |change| {
                if let Ok(mut seen) = seen.lock() {
                    seen.push(change);
                }
            }));
            let delivered = sink.notify(DeviceChange::DeviceListChanged);
            assert_eq!(delivered, tc.want_delivered, "{}", tc.name);
            let got = heard.lock().map(|h| h.clone()).unwrap_or_default();
            assert_eq!(got, tc.want_heard, "{}", tc.name);
        }
    }

    #[test]
    fn selection_round_trips_through_the_setting_aud_2() {
        let cases = [
            (
                "empty is the system default",
                "",
                DeviceSelection::SystemDefault,
            ),
            (
                "a uid is that device",
                "greendale-podcast",
                DeviceSelection::Device("greendale-podcast".to_owned()),
            ),
        ];
        for (name, setting, want) in cases {
            let got = DeviceSelection::from_setting(setting);
            assert_eq!(got, want, "{name}");
            assert_eq!(got.as_setting(), setting, "{name}");
        }
    }
}
