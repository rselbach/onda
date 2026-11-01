//! The microphone through `onda-audio`.

use std::sync::mpsc::Receiver;
use std::thread;

use onda_app::AudioSignalSender;
use onda_app::seams::{AudioSignal, AudioSource, CaptureInfo, InputDeviceInfo};
use onda_audio::{AudioCapture, AudioEvent, DeviceSelection, input_devices};
use onda_domain::messages::UserMessage;

/// [`AudioSource`] over an [`AudioCapture`].
pub struct Microphone(AudioCapture);

impl Microphone {
    /// Wraps `capture`.
    pub fn new(capture: AudioCapture) -> Self {
        Self(capture)
    }

    /// The wrapped capture, for device selection and change sinks.
    pub fn capture(&self) -> &AudioCapture {
        &self.0
    }
}

impl AudioSource for Microphone {
    fn start(&self) -> Result<CaptureInfo, UserMessage> {
        self.0
            .start()
            .map(|start| CaptureInfo {
                capture_id: start.capture_id,
                device_name: start.device_name,
            })
            .map_err(|error| error.notice())
    }

    fn mark_release(&self) -> usize {
        self.0.mark_release()
    }

    fn stop(&self) -> Vec<f32> {
        self.0.stop().samples
    }

    fn samples(&self) -> Vec<f32> {
        self.0.samples()
    }

    fn sample_count(&self) -> usize {
        self.0.sample_count()
    }

    fn input_devices(&self) -> Vec<InputDeviceInfo> {
        match input_devices() {
            Ok(devices) => devices
                .into_iter()
                .map(|device| InputDeviceInfo {
                    uid: device.id,
                    name: device.name,
                    is_default: device.is_default,
                })
                .collect(),
            Err(error) => {
                tracing::warn!(%error, "could not list input devices");
                Vec::new()
            }
        }
    }

    fn select_input(&self, uid: &str) {
        self.0.set_selection(DeviceSelection::from_setting(uid));
    }
}

/// Forwards `onda-audio` events to the core on a background thread until
/// either side closes. Rebuilds are logged (LOG-3) and name the new device
/// for the silent-microphone title (AUD-13).
pub fn forward_audio_events(events: Receiver<AudioEvent>, signals: AudioSignalSender) {
    let spawned = thread::Builder::new()
        .name("onda-audio-events".to_owned())
        .spawn(move || {
            for event in events {
                let signal = match event {
                    AudioEvent::Level(level) => AudioSignal::Level(level),
                    AudioEvent::Interrupted { capture_id, error } => AudioSignal::Interrupted {
                        capture_id,
                        notice: error.notice(),
                    },
                    AudioEvent::ConversionFailed { capture_id, error } => {
                        AudioSignal::ConversionFailed {
                            capture_id,
                            notice: error.notice(),
                        }
                    }
                    AudioEvent::Rebuilt {
                        reason,
                        device_name,
                    } => {
                        tracing::info!(%reason, %device_name, "audio input rebuilt");
                        AudioSignal::Rebuilt { device_name }
                    }
                };
                if !signals.send(signal) {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "could not start the audio event forwarder");
    }
}
