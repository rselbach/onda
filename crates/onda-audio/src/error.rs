//! Audio errors with their exact user-facing messages (03 "Audio errors").

use std::fmt;

use onda_domain::messages::UserMessage;

/// Everything that can go wrong with audio capture. `Display` is the exact
/// message from the "Audio errors" table; [`AudioError::notice`] picks the
/// notice it is shown in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// The selected microphone is not connected (AUD-3). Never falls back.
    SelectedDeviceMissing,
    /// The OS would not enumerate input devices.
    DeviceSelectionUnavailable,
    /// The OS refused to select the microphone. The cpal backend selects
    /// devices itself, so it never reports this; it stays for other backends
    /// and for parity with the table.
    CouldNotSelectDevice {
        /// OS status code.
        status: i32,
    },
    /// The input started but delivered nothing, even after a rebuild (AUD-9).
    NoBuffers,
    /// Every rebuild attempt failed while recording (AUD-10).
    RecoveryFailed(Box<AudioError>),
    /// The device reported an unusable format, such as zero channels or a
    /// zero sample rate while it wakes up, or no input device exists.
    InvalidFormat,
    /// The resampler to 16 kHz could not be created.
    ConverterUnavailable,
    /// Resampling one buffer failed (the cause in the payload).
    ConversionFailed(String),
    /// The OS failed to build or start the input stream (its error text).
    StartFailed(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelectedDeviceMissing => f.write_str("The selected microphone is not connected"),
            Self::DeviceSelectionUnavailable => {
                f.write_str("Onda could not access audio input device selection")
            }
            Self::CouldNotSelectDevice { status } => {
                write!(
                    f,
                    "Onda could not select the microphone (OSStatus {status})"
                )
            }
            Self::NoBuffers => {
                f.write_str("The microphone connected, but did not provide any audio")
            }
            Self::RecoveryFailed(cause) => {
                write!(f, "Could not reconnect to the microphone: {cause}")
            }
            Self::InvalidFormat => f.write_str("Invalid audio format configuration"),
            Self::ConverterUnavailable => f.write_str("Audio converter could not be created"),
            Self::ConversionFailed(cause) => write!(f, "Audio conversion failed: {cause}"),
            Self::StartFailed(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for AudioError {}

impl AudioError {
    /// The notice this error is shown in: "Microphone Unavailable" with the
    /// message as its body, or "Audio Processing Error" with "Failed to
    /// process audio: <message>".
    pub fn notice(&self) -> UserMessage {
        let error = self.to_string();
        match self {
            Self::SelectedDeviceMissing
            | Self::DeviceSelectionUnavailable
            | Self::CouldNotSelectDevice { .. }
            | Self::NoBuffers
            | Self::RecoveryFailed(_) => UserMessage::MicrophoneUnavailable { error },
            Self::InvalidFormat
            | Self::ConverterUnavailable
            | Self::ConversionFailed(_)
            | Self::StartFailed(_) => UserMessage::AudioProcessingError { error },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_notices_match_the_audio_errors_table() {
        struct Case {
            error: AudioError,
            want_message: &'static str,
            want_title: &'static str,
        }
        let cases = [
            Case {
                error: AudioError::SelectedDeviceMissing,
                want_message: "The selected microphone is not connected",
                want_title: "Microphone Unavailable",
            },
            Case {
                error: AudioError::DeviceSelectionUnavailable,
                want_message: "Onda could not access audio input device selection",
                want_title: "Microphone Unavailable",
            },
            Case {
                error: AudioError::CouldNotSelectDevice { status: -50 },
                want_message: "Onda could not select the microphone (OSStatus -50)",
                want_title: "Microphone Unavailable",
            },
            Case {
                error: AudioError::NoBuffers,
                want_message: "The microphone connected, but did not provide any audio",
                want_title: "Microphone Unavailable",
            },
            Case {
                error: AudioError::RecoveryFailed(Box::new(AudioError::SelectedDeviceMissing)),
                want_message: "Could not reconnect to the microphone: The selected microphone is not connected",
                want_title: "Microphone Unavailable",
            },
            Case {
                error: AudioError::InvalidFormat,
                want_message: "Invalid audio format configuration",
                want_title: "Audio Processing Error",
            },
            Case {
                error: AudioError::ConverterUnavailable,
                want_message: "Audio converter could not be created",
                want_title: "Audio Processing Error",
            },
            Case {
                error: AudioError::ConversionFailed("Greendale chunk too short".to_owned()),
                want_message: "Audio conversion failed: Greendale chunk too short",
                want_title: "Audio Processing Error",
            },
            Case {
                error: AudioError::StartFailed("Troy's mic is busy".to_owned()),
                want_message: "Troy's mic is busy",
                want_title: "Audio Processing Error",
            },
        ];
        for tc in cases {
            assert_eq!(tc.error.to_string(), tc.want_message);
            assert_eq!(
                tc.error.notice().title(),
                tc.want_title,
                "{}",
                tc.want_message
            );
        }
    }

    #[test]
    fn processing_notice_body_wraps_the_message() {
        let notice = AudioError::ConverterUnavailable.notice();
        assert_eq!(
            notice.body("menu bar icon"),
            "Failed to process audio: Audio converter could not be created"
        );
    }
}
