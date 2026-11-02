//! The real backend: cpal streams feeding a lock-free ring (AUD-1), device
//! listing, and a polling device-change source.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};
use tracing::{debug, warn};

use crate::engine::{Backend, Input, StreamFault};
use crate::{
    AudioDeviceEvents, AudioError, DeviceChange, DeviceChangeSink, DeviceSelection, InputDevice,
};

const NO_FAULT: u8 = 0;
const FAULT_DEVICE_LOST: u8 = 1;
const FAULT_CONFIGURATION_CHANGED: u8 = 2;

/// Lists the connected input devices, sorted by name, case-insensitively.
/// Natural ordering for display is the UI's job.
pub fn input_devices() -> Result<Vec<InputDevice>, AudioError> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| stable_id(&d));
    let devices = host.input_devices().map_err(|err| {
        warn!(%err, "could not list input devices");
        AudioError::DeviceSelectionUnavailable
    })?;
    let mut listed: Vec<InputDevice> = devices
        .filter_map(|device| {
            let id = stable_id(&device)?;
            Some(InputDevice {
                is_default: default_id.as_deref() == Some(id.as_str()),
                name: device_name(&device),
                id,
            })
        })
        .collect();
    listed.sort_by_key(|d| d.name.to_lowercase());
    Ok(listed)
}

// the backend-specific part of cpal's device id: the CoreAudio UID on macOS.
fn stable_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.id().to_owned())
}

fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| device.to_string())
}

pub(crate) struct CpalBackend {
    host: cpal::Host,
}

impl CpalBackend {
    pub(crate) fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    fn find(&self, selection: &DeviceSelection) -> Result<cpal::Device, AudioError> {
        let uid = match selection {
            // no input at all looks like the zero-channel format a missing
            // default reports in Jabber
            DeviceSelection::SystemDefault => {
                return self
                    .host
                    .default_input_device()
                    .ok_or(AudioError::InvalidFormat);
            }
            DeviceSelection::Device(uid) => uid,
        };
        let mut devices = self.host.input_devices().map_err(|err| {
            warn!(%err, "could not list input devices");
            AudioError::DeviceSelectionUnavailable
        })?;
        devices
            .find(|d| stable_id(d).as_deref() == Some(uid.as_str()))
            .ok_or(AudioError::SelectedDeviceMissing)
    }
}

impl Backend for CpalBackend {
    type Identity = cpal::Device;
    type Input = CpalInput;

    fn build(&mut self, selection: &DeviceSelection) -> Result<CpalInput, AudioError> {
        let device = self.find(selection)?;
        let name = device_name(&device);
        let supported = device.default_input_config().map_err(|err| {
            warn!(device = %name, %err, "input has no usable format");
            AudioError::InvalidFormat
        })?;
        let config = supported.config();
        let channels = usize::from(config.channels);
        if config.sample_rate == 0 || channels == 0 {
            return Err(AudioError::InvalidFormat);
        }
        // a second of audio: far more than the worker ever falls behind
        let (producer, consumer) = RingBuffer::new(config.sample_rate as usize * channels);
        let shared = Shared::default();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(&device, config, producer, &shared),
            SampleFormat::F64 => build_stream::<f64>(&device, config, producer, &shared),
            SampleFormat::I8 => build_stream::<i8>(&device, config, producer, &shared),
            SampleFormat::I16 => build_stream::<i16>(&device, config, producer, &shared),
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, config, producer, &shared),
            SampleFormat::I32 => build_stream::<i32>(&device, config, producer, &shared),
            SampleFormat::U8 => build_stream::<u8>(&device, config, producer, &shared),
            SampleFormat::U16 => build_stream::<u16>(&device, config, producer, &shared),
            SampleFormat::U32 => build_stream::<u32>(&device, config, producer, &shared),
            other => {
                warn!(device = %name, format = %other, "unsupported input sample format");
                return Err(AudioError::InvalidFormat);
            }
        }
        .map_err(|err| AudioError::StartFailed(err.to_string()))?;
        Ok(CpalInput {
            device,
            name,
            sample_rate: config.sample_rate,
            channels,
            stream,
            consumer,
            shared,
            dropped_seen: 0,
        })
    }

    fn resolve(&mut self, selection: &DeviceSelection) -> Option<cpal::Device> {
        self.find(selection).ok()
    }
}

// state the audio callbacks write without locks or allocation
#[derive(Clone, Default)]
struct Shared {
    fault: Arc<AtomicU8>,
    dropped: Arc<AtomicUsize>,
}

fn build_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut producer: Producer<f32>,
    shared: &Shared,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let dropped = Arc::clone(&shared.dropped);
    let fault = Arc::clone(&shared.fault);
    device.build_input_stream::<T, _, _>(
        config,
        // realtime: whole buffers in or out, no locks, no allocation
        move |data: &[T], _| match producer.write_chunk_uninit(data.len()) {
            Ok(chunk) => {
                chunk.fill_from_iter(data.iter().map(|s| s.to_sample::<f32>()));
            }
            Err(_) => {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |err: cpal::Error| match err.kind() {
            ErrorKind::Xrun => {}
            ErrorKind::DeviceNotAvailable => fault.store(FAULT_DEVICE_LOST, Ordering::Relaxed),
            _ => fault.store(FAULT_CONFIGURATION_CHANGED, Ordering::Relaxed),
        },
        None,
    )
}

pub(crate) struct CpalInput {
    device: cpal::Device,
    name: String,
    sample_rate: u32,
    channels: usize,
    stream: cpal::Stream,
    consumer: Consumer<f32>,
    shared: Shared,
    dropped_seen: usize,
}

impl Input for CpalInput {
    type Identity = cpal::Device;

    fn identity(&self) -> cpal::Device {
        self.device.clone()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn start(&mut self) -> Result<(), AudioError> {
        self.stream
            .play()
            .map_err(|err| AudioError::StartFailed(err.to_string()))
    }

    fn stop(&mut self) -> bool {
        match self.stream.pause() {
            Ok(()) => true,
            Err(err) => {
                debug!(%err, "input cannot pause; releasing the stream");
                false
            }
        }
    }

    fn read(&mut self, out: &mut Vec<f32>) {
        let dropped = self.shared.dropped.load(Ordering::Relaxed);
        if dropped != self.dropped_seen {
            warn!(
                buffers = dropped - self.dropped_seen,
                "audio ring overflowed; buffers dropped"
            );
            self.dropped_seen = dropped;
        }
        let available = self.consumer.slots();
        let whole_frames = available - available % self.channels;
        let Ok(chunk) = self.consumer.read_chunk(whole_frames) else {
            return;
        };
        let (first, second) = chunk.as_slices();
        out.extend_from_slice(first);
        out.extend_from_slice(second);
        chunk.commit_all();
    }

    fn take_fault(&mut self) -> Option<StreamFault> {
        match self.shared.fault.swap(NO_FAULT, Ordering::Relaxed) {
            FAULT_DEVICE_LOST => Some(StreamFault::DeviceLost),
            FAULT_CONFIGURATION_CHANGED => Some(StreamFault::ConfigurationChanged),
            _ => None,
        }
    }
}

/// Portable [`AudioDeviceEvents`] that polls the device list and the default
/// input. A shell with OS change notifications can replace it.
pub struct PollingDeviceEvents {
    interval: Duration,
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl PollingDeviceEvents {
    /// Polls every `interval`.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            stop: None,
            worker: None,
        }
    }

    fn halt(&mut self) {
        self.stop = None;
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            warn!("device polling thread panicked");
        }
    }
}

impl AudioDeviceEvents for PollingDeviceEvents {
    fn subscribe(&mut self, sink: DeviceChangeSink) {
        self.halt();
        let (stop, stopped) = mpsc::channel::<()>();
        let interval = self.interval;
        let spawned = thread::Builder::new()
            .name("onda-audio-devices".to_owned())
            .spawn(move || {
                let host = cpal::default_host();
                let mut last = Snapshot::take(&host);
                loop {
                    match stopped.recv_timeout(interval) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                    }
                    let Some(now) = Snapshot::take(&host) else {
                        continue;
                    };
                    let Some(before) = last.replace(now.clone()) else {
                        continue;
                    };
                    if now.devices != before.devices {
                        debug!("audio input device list changed");
                        if !sink.notify(DeviceChange::DeviceListChanged) {
                            return;
                        }
                    }
                    if now.default != before.default {
                        debug!("default audio input changed");
                        if !sink.notify(DeviceChange::DefaultInputChanged) {
                            return;
                        }
                    }
                }
            });
        match spawned {
            Ok(worker) => {
                self.stop = Some(stop);
                self.worker = Some(worker);
            }
            Err(err) => warn!(%err, "could not start device polling"),
        }
    }
}

impl Drop for PollingDeviceEvents {
    fn drop(&mut self) {
        self.halt();
    }
}

#[derive(Clone, PartialEq)]
struct Snapshot {
    default: Option<cpal::Device>,
    // device equality is the connection, so a reconnect counts as a change
    devices: Vec<cpal::Device>,
}

impl Snapshot {
    fn take(host: &cpal::Host) -> Option<Self> {
        let devices = match host.input_devices() {
            Ok(devices) => devices.collect(),
            Err(err) => {
                debug!(%err, "could not poll input devices");
                return None;
            }
        };
        Some(Self {
            default: host.default_input_device(),
            devices,
        })
    }
}
