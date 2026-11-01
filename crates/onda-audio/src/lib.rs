//! Core-owned audio capture.
//!
//! [`AudioCapture`] records the chosen microphone as 16 kHz mono `f32`
//! (AUD-1), prepares the input ahead of time (AUD-4), idles warm for 8 s
//! after a session with a 450 ms pre-roll (AUD-5, AUD-6), watches for a
//! silent first buffer (AUD-9), and rebuilds the input when devices change
//! (AUD-10, AUD-11). Errors carry the exact copy from the "Audio errors"
//! table. Pure rules (level metering, pre-roll ring, speech gate) come from
//! `onda-domain`.
//!
//! No async runtime: a worker thread owns the stream and talks over
//! channels. The cpal realtime callback only copies samples into a
//! lock-free ring.

mod capture;
mod cpal_backend;
mod device;
mod engine;
mod error;
mod resample;

pub use capture::AudioCapture;
pub use cpal_backend::{PollingDeviceEvents, input_devices};
pub use device::{
    AudioDeviceEvents, DeviceChange, DeviceChangeListener, DeviceChangeSink, DeviceSelection,
    InputDevice,
};
pub use engine::{AudioEvent, CaptureStart, RebuildReason, Recording};
pub use error::AudioError;
