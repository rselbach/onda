//! The capture state machine: prepare, warm standby, pre-roll, first-buffer
//! watchdog, and input rebuilds (AUD-4..AUD-12).
//!
//! Time is a parameter and the stream sits behind [`Backend`], so every rule
//! here runs in unit tests against a fake device.

use std::fmt;
use std::sync::mpsc::Sender;
use std::time::Duration;

use onda_domain::level::{LevelThrottle, rms_level};
use onda_domain::pre_roll::{PreRoll, SESSION_RESERVE_SAMPLES};
use onda_domain::speech_gate::{self, GateVerdict};
use onda_domain::types::MonoTime;
use tracing::{debug, error, info, warn};

use crate::resample::Resampler;
use crate::{AudioError, DeviceChange, DeviceSelection};

/// After a session the input keeps running this long, then stops (AUD-5).
pub(crate) const STANDBY: Duration = Duration::from_secs(8);
/// No audio this long after starting triggers one rebuild (AUD-9).
pub(crate) const FIRST_BUFFER_TIMEOUT: Duration = Duration::from_secs(2);
/// Build attempts per rebuild (AUD-10).
pub(crate) const REBUILD_ATTEMPTS: usize = 6;
/// Pause between rebuild attempts (AUD-10).
pub(crate) const REBUILD_RETRY_DELAY: Duration = Duration::from_millis(200);

// how often the worker drains the ring while the input runs
const POLL_INTERVAL: Duration = Duration::from_millis(10);
// longest sleep with nothing scheduled; bounds how late an idle fault is seen
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Builds inputs for a selection. The cpal implementation is the real one;
/// tests use a fake.
pub(crate) trait Backend {
    /// Identifies one physical device connection, so a reconnect under the
    /// same stable id still counts as a change (AUD-10).
    type Identity: Clone + PartialEq + fmt::Debug;
    /// A built input stream.
    type Input: Input<Identity = Self::Identity>;

    /// Builds a stopped input for `selection`; the microphone stays closed.
    fn build(&mut self, selection: &DeviceSelection) -> Result<Self::Input, AudioError>;

    /// The connection `selection` would use right now, if any.
    fn resolve(&mut self, selection: &DeviceSelection) -> Option<Self::Identity>;
}

/// One built input stream.
pub(crate) trait Input {
    /// See [`Backend::Identity`].
    type Identity;

    fn identity(&self) -> Self::Identity;
    fn name(&self) -> &str;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> usize;
    /// Opens the microphone.
    fn start(&mut self) -> Result<(), AudioError>;
    /// Closes the microphone; the input stays built.
    fn stop(&mut self);
    /// Appends captured interleaved frames, device format, to `out`.
    fn read(&mut self, out: &mut Vec<f32>);
    /// A fault the stream reported since the last call.
    fn take_fault(&mut self) -> Option<StreamFault>;
}

/// A fault reported by an open stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamFault {
    /// The device went away.
    DeviceLost,
    /// The device's format or route changed under the stream.
    ConfigurationChanged,
}

/// Why the input was rebuilt (AUD-9, AUD-10). `Display` is the log text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebuildReason {
    /// The OS default input changed while following it.
    DefaultDeviceChanged,
    /// The selected device's connection changed.
    DeviceListChanged,
    /// The user picked another input.
    SelectionChanged,
    /// The OS reported an audio configuration change on the stream.
    ConfigurationChanged,
    /// The stream's device disappeared.
    DeviceLost,
    /// The input delivered no audio within 2 s of starting (AUD-9).
    MissingFirstBuffer,
}

impl fmt::Display for RebuildReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DefaultDeviceChanged => "the default input changed",
            Self::DeviceListChanged => "the available input devices changed",
            Self::SelectionChanged => "the selected input changed",
            Self::ConfigurationChanged => "the audio configuration changed",
            Self::DeviceLost => "the input device disappeared",
            Self::MissingFirstBuffer => "the input delivered no audio",
        })
    }
}

/// What the capture tells its owner, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// Input level in `[0, 1]`: RMS of channel 0 before resampling, at most
    /// 30 per second, only while a capture is open (AUD-8).
    Level(f32),
    /// Capture `capture_id` cannot continue: [`AudioError::NoBuffers`] or
    /// [`AudioError::RecoveryFailed`]. Stop it and transcribe what was
    /// captured; during a hotkey tail, end the tail early (DICT-11).
    Interrupted {
        /// [`CaptureStart::capture_id`] of the affected capture.
        capture_id: u64,
        /// What went wrong.
        error: AudioError,
    },
    /// One buffer of capture `capture_id` could not be resampled. Recording
    /// continues; the owner rate-limits the notice.
    ConversionFailed {
        /// [`CaptureStart::capture_id`] of the affected capture.
        capture_id: u64,
        /// The [`AudioError::ConversionFailed`].
        error: AudioError,
    },
    /// The input was rebuilt and is ready (recording continues if it was).
    Rebuilt {
        /// Why.
        reason: RebuildReason,
        /// Name of the device now in use.
        device_name: String,
    },
}

/// A capture that just started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureStart {
    /// Tags this capture's events, so a late event from an earlier capture
    /// can be told apart.
    pub capture_id: u64,
    /// Started from warm standby (AUD-5).
    pub warm: bool,
    /// Samples of pre-roll that open the capture; zero on a cold start
    /// (AUD-6).
    pub pre_roll_samples: usize,
    /// Name of the device in use, for the silent-microphone title (AUD-13).
    pub device_name: String,
}

/// A finished capture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recording {
    /// 16 kHz mono samples in `[-1, 1]`, pre-roll first (AUD-1, AUD-6).
    pub samples: Vec<f32>,
    /// How many leading samples are pre-roll.
    pub pre_roll_samples: usize,
    /// Started from warm standby.
    pub warm_start: bool,
    /// Sample count when the hotkey was released, if it was (GATE-6).
    pub release_sample_count: Option<usize>,
    /// Time from start to the first audio, if any arrived.
    pub first_buffer_latency: Option<Duration>,
}

impl Recording {
    /// The samples the speech gate judges: those before the release when
    /// there was one, else all (GATE-6).
    pub fn gate_samples(&self) -> &[f32] {
        let end = self
            .release_sample_count
            .unwrap_or(self.samples.len())
            .min(self.samples.len());
        &self.samples[..end]
    }

    /// Runs the speech gate on [`Recording::gate_samples`] (GATE-1..GATE-6).
    pub fn speech_gate(&self) -> GateVerdict {
        speech_gate::assess(self.gate_samples())
    }
}

struct Prepared<I> {
    input: I,
    resampler: Resampler,
    running: bool,
}

struct Capture {
    start: CaptureStart,
    samples: Vec<f32>,
    release: Option<usize>,
    first_buffer_latency: Option<Duration>,
    levels: LevelThrottle,
}

struct Rebuild {
    reason: RebuildReason,
    attempts: usize,
    next_at: MonoTime,
    was_capturing: bool,
}

/// The capture state machine. Single-threaded; the worker in
/// [`crate::capture`] drives it with the current time.
pub(crate) struct Engine<B: Backend> {
    backend: B,
    selection: DeviceSelection,
    events: Sender<AudioEvent>,
    prepared: Option<Prepared<B::Input>>,
    // the connection the selected device was built against
    configured: Option<B::Identity>,
    needs_rebuild: bool,
    capture: Option<Capture>,
    last_capture_id: u64,
    pre_roll: PreRoll,
    standby_until: Option<MonoTime>,
    watchdog_at: Option<MonoTime>,
    awaiting_first_buffer: Option<MonoTime>,
    retried_missing_first_buffer: bool,
    rebuild: Option<Rebuild>,
    raw: Vec<f32>,
    mono: Vec<f32>,
    converted: Vec<f32>,
}

impl<B: Backend> Engine<B> {
    pub(crate) fn new(backend: B, selection: DeviceSelection, events: Sender<AudioEvent>) -> Self {
        Self {
            backend,
            selection,
            events,
            prepared: None,
            configured: None,
            needs_rebuild: false,
            capture: None,
            last_capture_id: 0,
            pre_roll: PreRoll::new(),
            standby_until: None,
            watchdog_at: None,
            awaiting_first_buffer: None,
            retried_missing_first_buffer: false,
            rebuild: None,
            raw: Vec::new(),
            mono: Vec::new(),
            converted: Vec::new(),
        }
    }

    /// Builds the input without opening the microphone (AUD-4). Failures are
    /// only logged: the next start builds again and reports them.
    pub(crate) fn prepare(&mut self) {
        if self.prepared.is_some() || self.capture.is_some() || self.rebuild.is_some() {
            return;
        }
        if let Err(err) = self.build() {
            warn!(%err, "audio input prepare failed");
        }
    }

    /// Opens a capture. A warm input starts with the pre-roll (AUD-6).
    pub(crate) fn start(&mut self, now: MonoTime) -> Result<CaptureStart, AudioError> {
        if let Some(capture) = &self.capture {
            return Ok(capture.start.clone());
        }
        self.rebuild = None;
        self.standby_until = None;
        self.retried_missing_first_buffer = false;
        if self.needs_rebuild || self.prepared.is_none() {
            self.teardown(false, now);
            self.build()?;
        }
        // warm: the newest audio joins the pre-roll; cold: stale audio is dropped
        self.pump(now);
        let Some(prepared) = self.prepared.as_mut() else {
            return Err(AudioError::InvalidFormat);
        };
        let warm = prepared.running;
        let pre_roll = match warm {
            true => self.pre_roll.take(),
            false => {
                prepared.resampler.reset();
                self.pre_roll.clear();
                Vec::new()
            }
        };
        if !prepared.running {
            if let Err(err) = prepared.input.start() {
                warn!(%err, "audio input failed to start");
                self.needs_rebuild = true;
                return Err(err);
            }
            prepared.running = true;
        }
        self.last_capture_id += 1;
        let start = CaptureStart {
            capture_id: self.last_capture_id,
            warm,
            pre_roll_samples: pre_roll.len(),
            device_name: prepared.input.name().to_owned(),
        };
        let mut samples = Vec::with_capacity(SESSION_RESERVE_SAMPLES.max(pre_roll.len()));
        samples.extend_from_slice(&pre_roll);
        self.capture = Some(Capture {
            start: start.clone(),
            samples,
            release: None,
            first_buffer_latency: None,
            levels: LevelThrottle::default(),
        });
        self.arm_watchdog(now);
        Ok(start)
    }

    /// Records the hotkey release and returns the sample count at that
    /// moment (GATE-6). Only the first call per capture counts.
    pub(crate) fn mark_release(&mut self, now: MonoTime) -> usize {
        self.pump(now);
        let Some(capture) = self.capture.as_mut() else {
            return 0;
        };
        *capture.release.get_or_insert(capture.samples.len())
    }

    /// Closes the capture, flushing the resampler tail into it (AUD-7), and
    /// leaves the input running in warm standby (AUD-5).
    pub(crate) fn stop(&mut self, now: MonoTime) -> Recording {
        self.pump(now);
        self.flush_into_capture();
        let Some(capture) = self.capture.take() else {
            return Recording::default();
        };
        self.awaiting_first_buffer = None;
        self.watchdog_at = None;
        self.rebuild = None;
        if self.needs_rebuild {
            // a route change invalidated the input; don't keep a stale one warm
            self.teardown(false, now);
            self.prepare();
        } else if self.prepared.as_ref().is_some_and(|p| p.running) {
            self.standby_until = Some(now + STANDBY);
        }
        Recording {
            samples: capture.samples,
            pre_roll_samples: capture.start.pre_roll_samples,
            warm_start: capture.start.warm,
            release_sample_count: capture.release,
            first_buffer_latency: capture.first_buffer_latency,
        }
    }

    /// A copy of the open capture's samples so far.
    pub(crate) fn samples(&mut self, now: MonoTime) -> Vec<f32> {
        self.pump(now);
        self.capture
            .as_ref()
            .map(|c| c.samples.clone())
            .unwrap_or_default()
    }

    /// The open capture's sample count so far.
    pub(crate) fn sample_count(&mut self, now: MonoTime) -> usize {
        self.pump(now);
        self.capture.as_ref().map_or(0, |c| c.samples.len())
    }

    /// Switches input (AUD-10).
    pub(crate) fn set_selection(&mut self, selection: DeviceSelection, now: MonoTime) {
        if selection == self.selection {
            return;
        }
        self.selection = selection;
        self.request_rebuild(RebuildReason::SelectionChanged, now);
    }

    /// Reacts to an OS device change (AUD-10).
    pub(crate) fn device_changed(&mut self, change: DeviceChange, now: MonoTime) {
        match change {
            DeviceChange::DefaultInputChanged => {
                if self.selection == DeviceSelection::SystemDefault {
                    self.request_rebuild(RebuildReason::DefaultDeviceChanged, now);
                }
            }
            DeviceChange::DeviceListChanged => {
                if self.selection == DeviceSelection::SystemDefault {
                    return;
                }
                let resolved = self.backend.resolve(&self.selection);
                if resolved == self.configured {
                    return;
                }
                self.configured = resolved;
                self.request_rebuild(RebuildReason::DeviceListChanged, now);
            }
        }
    }

    /// Drains audio and fires whatever is due.
    pub(crate) fn tick(&mut self, now: MonoTime) {
        self.pump(now);
        let fault = self.prepared.as_mut().and_then(|p| p.input.take_fault());
        match fault {
            Some(StreamFault::DeviceLost) => self.request_rebuild(RebuildReason::DeviceLost, now),
            Some(StreamFault::ConfigurationChanged) => {
                self.request_rebuild(RebuildReason::ConfigurationChanged, now);
            }
            None => {}
        }
        if self.standby_until.is_some_and(|at| now >= at) {
            self.retire();
        }
        if self.watchdog_at.is_some_and(|at| now >= at) {
            self.first_buffer_overdue(now);
        }
        if self.rebuild.as_ref().is_some_and(|r| now >= r.next_at) {
            self.attempt_rebuild(now);
        }
    }

    /// How long the worker may sleep before the next [`Engine::tick`].
    pub(crate) fn next_wake(&self, now: MonoTime) -> Duration {
        let running = self.prepared.as_ref().is_some_and(|p| p.running);
        let base = if running {
            POLL_INTERVAL
        } else {
            IDLE_POLL_INTERVAL
        };
        [
            self.standby_until,
            self.watchdog_at,
            self.rebuild.as_ref().map(|r| r.next_at),
        ]
        .into_iter()
        .flatten()
        .fold(base, |wake, at| wake.min(at.saturating_sub(now)))
    }

    fn build(&mut self) -> Result<(), AudioError> {
        let input = self.backend.build(&self.selection)?;
        let resampler = Resampler::new(input.sample_rate())?;
        info!(
            device = input.name(),
            sample_rate = input.sample_rate(),
            channels = input.channels(),
            "audio input built"
        );
        self.configured = match self.selection {
            DeviceSelection::SystemDefault => None,
            DeviceSelection::Device(_) => Some(input.identity()),
        };
        self.prepared = Some(Prepared {
            input,
            resampler,
            running: false,
        });
        self.needs_rebuild = false;
        Ok(())
    }

    // stops and drops the input; with `drain_tail`, whatever it still holds
    // goes into the open capture first (AUD-11a).
    fn teardown(&mut self, drain_tail: bool, now: MonoTime) {
        self.standby_until = None;
        self.watchdog_at = None;
        self.awaiting_first_buffer = None;
        if let Some(prepared) = self.prepared.as_mut() {
            prepared.input.stop();
        }
        if drain_tail {
            self.pump(now);
            self.flush_into_capture();
        }
        self.prepared = None;
        self.pre_roll.clear();
    }

    // moves captured audio from the input into the capture, or into the
    // pre-roll while idling warm.
    fn pump(&mut self, now: MonoTime) {
        let Some(prepared) = self.prepared.as_mut() else {
            return;
        };
        self.raw.clear();
        prepared.input.read(&mut self.raw);
        if self.raw.is_empty() {
            return;
        }
        let channels = prepared.input.channels().max(1);
        self.mono.clear();
        self.mono.extend(self.raw.iter().step_by(channels));

        let Some(capture) = self.capture.as_mut() else {
            if !prepared.running {
                return;
            }
            self.converted.clear();
            match prepared.resampler.process(&self.mono, &mut self.converted) {
                Ok(()) => self.pre_roll.push(&self.converted),
                Err(err) => debug!(%err, "ignoring a pre-roll conversion failure"),
            }
            return;
        };

        if let Some(started) = self.awaiting_first_buffer.take() {
            let latency = now.saturating_sub(started);
            info!(
                latency_ms = latency.as_millis() as u64,
                "first audio arrived after capture start"
            );
            self.watchdog_at = None;
            capture.first_buffer_latency.get_or_insert(latency);
        }
        if capture.levels.should_report(now) {
            emit(&self.events, AudioEvent::Level(rms_level(&self.mono)));
        }
        if let Err(err) = prepared.resampler.process(&self.mono, &mut capture.samples) {
            error!(%err, "audio conversion failed");
            let capture_id = capture.start.capture_id;
            emit(
                &self.events,
                AudioEvent::ConversionFailed {
                    capture_id,
                    error: err,
                },
            );
        }
    }

    fn flush_into_capture(&mut self) {
        let (Some(prepared), Some(capture)) = (self.prepared.as_mut(), self.capture.as_mut())
        else {
            return;
        };
        if let Err(err) = prepared.resampler.flush(&mut capture.samples) {
            warn!(%err, "draining the resampler tail failed");
        }
    }

    fn arm_watchdog(&mut self, now: MonoTime) {
        self.awaiting_first_buffer = Some(now);
        self.watchdog_at = Some(now + FIRST_BUFFER_TIMEOUT);
    }

    // standby is over: close the microphone but keep the input built (AUD-5).
    fn retire(&mut self) {
        self.standby_until = None;
        if self.capture.is_some() {
            return;
        }
        let Some(prepared) = self.prepared.as_mut().filter(|p| p.running) else {
            return;
        };
        prepared.input.stop();
        prepared.running = false;
        prepared.resampler.reset();
        self.pre_roll.clear();
        info!("audio input stopped after standby");
    }

    fn first_buffer_overdue(&mut self, now: MonoTime) {
        self.watchdog_at = None;
        if self.capture.is_none() || self.awaiting_first_buffer.is_none() {
            return;
        }
        if self.retried_missing_first_buffer {
            error!("audio input delivered no buffers after a fresh rebuild");
            self.interrupt(AudioError::NoBuffers);
            return;
        }
        self.retried_missing_first_buffer = true;
        warn!("audio input delivered no buffers; rebuilding it once");
        self.request_rebuild(RebuildReason::MissingFirstBuffer, now);
    }

    fn request_rebuild(&mut self, reason: RebuildReason, now: MonoTime) {
        self.needs_rebuild = true;
        if self.prepared.is_none() && self.capture.is_none() {
            info!(%reason, "deferring input rebuild until capture is prepared");
            return;
        }
        if self.rebuild.is_some() {
            info!(%reason, "coalescing input rebuild");
            return;
        }
        if reason != RebuildReason::MissingFirstBuffer {
            self.retried_missing_first_buffer = false;
        }
        info!(%reason, "rebuilding audio input");
        let was_capturing = self.capture.is_some();
        self.teardown(was_capturing, now);
        self.rebuild = Some(Rebuild {
            reason,
            attempts: 0,
            next_at: now,
            was_capturing,
        });
        self.attempt_rebuild(now);
    }

    fn attempt_rebuild(&mut self, now: MonoTime) {
        let Some(mut rebuild) = self.rebuild.take() else {
            return;
        };
        let result = self.build().and_then(|()| self.resume_capture(now));
        let err = match result {
            Ok(()) => {
                let reason = rebuild.reason;
                info!(%reason, attempts = rebuild.attempts + 1, "rebuilt audio input");
                let device_name = self
                    .prepared
                    .as_ref()
                    .map(|p| p.input.name().to_owned())
                    .unwrap_or_default();
                emit(
                    &self.events,
                    AudioEvent::Rebuilt {
                        reason,
                        device_name,
                    },
                );
                return;
            }
            Err(err) => err,
        };
        rebuild.attempts += 1;
        warn!(attempt = rebuild.attempts, %err, "audio input rebuild attempt failed");
        self.teardown(false, now);
        self.needs_rebuild = true;
        if rebuild.attempts < REBUILD_ATTEMPTS {
            rebuild.next_at = now + REBUILD_RETRY_DELAY;
            self.rebuild = Some(rebuild);
            return;
        }
        let reason = rebuild.reason;
        if rebuild.was_capturing && self.capture.is_some() {
            error!(%reason, %err, "could not reconnect to the microphone");
            self.interrupt(AudioError::RecoveryFailed(Box::new(err)));
            return;
        }
        error!(%reason, %err, "could not prepare audio input");
    }

    // a rebuilt input keeps recording if a capture is open (AUD-11a).
    fn resume_capture(&mut self, now: MonoTime) -> Result<(), AudioError> {
        if self.capture.is_none() {
            return Ok(());
        }
        let Some(prepared) = self.prepared.as_mut() else {
            return Ok(());
        };
        prepared.input.start()?;
        prepared.running = true;
        self.arm_watchdog(now);
        Ok(())
    }

    fn interrupt(&mut self, error: AudioError) {
        let Some(capture) = &self.capture else {
            return;
        };
        let capture_id = capture.start.capture_id;
        emit(&self.events, AudioEvent::Interrupted { capture_id, error });
    }
}

fn emit(events: &Sender<AudioEvent>, event: AudioEvent) {
    if events.send(event).is_err() {
        debug!("audio event receiver is gone");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::mpsc::{self, Receiver};

    use onda_domain::pre_roll::PRE_ROLL_SAMPLES;

    use super::*;

    const TROY: &str = "troy-desk-mic";
    const ABED: &str = "abed-podcast-mic";

    #[derive(Default)]
    struct World {
        // (stable id, name, connection)
        devices: Vec<(String, String, u32)>,
        default: Option<String>,
        next_connection: u32,
        rate: u32,
        channels: usize,
        build_failures: VecDeque<AudioError>,
        start_failures: VecDeque<AudioError>,
        builds: Vec<String>,
        starts: usize,
        stops: usize,
        // the latest input's state
        running: bool,
        stalled: bool,
        queue: Vec<f32>,
        fault: Option<StreamFault>,
        live_input: u32,
    }

    impl World {
        fn plug(&mut self, id: &str, name: &str) {
            self.next_connection += 1;
            self.devices.retain(|(d, _, _)| d != id);
            self.devices
                .push((id.to_owned(), name.to_owned(), self.next_connection));
        }

        fn find(&self, selection: &DeviceSelection) -> Option<(String, u32)> {
            let id = match selection {
                DeviceSelection::SystemDefault => self.default.as_deref()?,
                DeviceSelection::Device(id) => id.as_str(),
            };
            self.devices
                .iter()
                .find(|(d, _, _)| d == id)
                .map(|(_, name, connection)| (name.clone(), *connection))
        }

        // the device delivers `frames` of `value` on every channel of ch0,
        // silence on the others, if its stream runs and is not stalled.
        fn deliver(&mut self, frames: usize, value: f32) {
            if !self.running || self.stalled {
                return;
            }
            for _ in 0..frames {
                self.queue.push(value);
                for _ in 1..self.channels {
                    self.queue.push(0.0);
                }
            }
        }
    }

    struct FakeBackend(Rc<RefCell<World>>);

    struct FakeInput {
        world: Rc<RefCell<World>>,
        serial: u32,
        name: String,
        connection: u32,
        rate: u32,
        channels: usize,
    }

    impl FakeInput {
        fn live(&self) -> bool {
            self.world.borrow().live_input == self.serial
        }
    }

    impl Backend for FakeBackend {
        type Identity = u32;
        type Input = FakeInput;

        fn build(&mut self, selection: &DeviceSelection) -> Result<FakeInput, AudioError> {
            let mut world = self.0.borrow_mut();
            world.builds.push(selection.as_setting().to_owned());
            if let Some(err) = world.build_failures.pop_front() {
                return Err(err);
            }
            let (name, connection) = match (world.find(selection), selection) {
                (Some(found), _) => found,
                (None, DeviceSelection::Device(_)) => {
                    return Err(AudioError::SelectedDeviceMissing);
                }
                (None, DeviceSelection::SystemDefault) => return Err(AudioError::InvalidFormat),
            };
            world.live_input += 1;
            world.running = false;
            world.queue.clear();
            world.fault = None;
            Ok(FakeInput {
                world: Rc::clone(&self.0),
                serial: world.live_input,
                name,
                connection,
                rate: world.rate,
                channels: world.channels,
            })
        }

        fn resolve(&mut self, selection: &DeviceSelection) -> Option<u32> {
            self.0.borrow().find(selection).map(|(_, c)| c)
        }
    }

    impl Input for FakeInput {
        type Identity = u32;

        fn identity(&self) -> u32 {
            self.connection
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn sample_rate(&self) -> u32 {
            self.rate
        }

        fn channels(&self) -> usize {
            self.channels
        }

        fn start(&mut self) -> Result<(), AudioError> {
            let mut world = self.world.borrow_mut();
            if let Some(err) = world.start_failures.pop_front() {
                return Err(err);
            }
            world.starts += 1;
            world.running = true;
            Ok(())
        }

        fn stop(&mut self) {
            if !self.live() {
                return;
            }
            let mut world = self.world.borrow_mut();
            if world.running {
                world.stops += 1;
            }
            world.running = false;
        }

        fn read(&mut self, out: &mut Vec<f32>) {
            if !self.live() {
                return;
            }
            out.append(&mut self.world.borrow_mut().queue);
        }

        fn take_fault(&mut self) -> Option<StreamFault> {
            if !self.live() {
                return None;
            }
            self.world.borrow_mut().fault.take()
        }
    }

    struct Harness {
        engine: Engine<FakeBackend>,
        world: Rc<RefCell<World>>,
        events: Receiver<AudioEvent>,
        now: MonoTime,
        // what the device is "hearing"
        signal: f32,
    }

    impl Harness {
        fn new(selection: DeviceSelection) -> Self {
            let mut world = World {
                rate: 16_000,
                channels: 1,
                ..World::default()
            };
            world.plug(TROY, "Troy's Desk Mic");
            world.plug(ABED, "Abed's Podcast Mic");
            world.default = Some(TROY.to_owned());
            let world = Rc::new(RefCell::new(world));
            let (tx, events) = mpsc::channel();
            let engine = Engine::new(FakeBackend(Rc::clone(&world)), selection, tx);
            Self {
                engine,
                world,
                events,
                now: Duration::ZERO,
                signal: 0.25,
            }
        }

        // advances in 10 ms steps, the device delivering audio as it goes
        fn advance(&mut self, by: Duration) {
            let step = Duration::from_millis(10);
            let end = self.now + by;
            while self.now < end {
                self.now += step;
                let frames = self.world.borrow().rate as usize / 100;
                self.world.borrow_mut().deliver(frames, self.signal);
                self.engine.tick(self.now);
            }
        }

        fn start(&mut self) -> Result<CaptureStart, AudioError> {
            self.engine.start(self.now)
        }

        fn stop(&mut self) -> Recording {
            self.engine.stop(self.now)
        }

        fn builds(&self) -> usize {
            self.world.borrow().builds.len()
        }

        fn events(&self) -> Vec<AudioEvent> {
            self.events.try_iter().collect()
        }

        fn rebuilds(&self) -> Vec<RebuildReason> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    AudioEvent::Rebuilt { reason, .. } => Some(reason),
                    _ => None,
                })
                .collect()
        }
    }

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn prepare_builds_without_opening_the_mic_aud_4_aud_12() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.engine.prepare();
        assert_eq!(h.builds(), 1);
        assert_eq!(h.world.borrow().starts, 0, "mic stays closed");

        let start = h.start().expect("start");
        assert_eq!(h.builds(), 1, "start reuses the prepared input");
        assert!(!start.warm, "a prepared input is still a cold start");
        assert_eq!(start.pre_roll_samples, 0);
        assert_eq!(start.device_name, "Troy's Desk Mic");
        h.advance(ms(500));
        let recording = h.stop();
        assert_eq!(recording.samples.len(), 8_000);
        assert!(recording.samples.capacity() >= SESSION_RESERVE_SAMPLES);
        assert_eq!(recording.first_buffer_latency, Some(ms(10)));
    }

    #[test]
    fn pre_roll_only_on_a_warm_start_aud_5_aud_6() {
        struct Case {
            name: &'static str,
            gap: Duration,
            want_warm: bool,
            want_pre_roll: usize,
        }
        let cases = [
            Case {
                name: "back-to-back start is warm with the newest 450 ms",
                gap: ms(1_000),
                want_warm: true,
                want_pre_roll: PRE_ROLL_SAMPLES,
            },
            Case {
                name: "a quick restart holds only what was heard",
                gap: ms(100),
                want_warm: true,
                want_pre_roll: 1_600,
            },
            Case {
                name: "after standby expires the start is cold",
                gap: ms(9_000),
                want_warm: false,
                want_pre_roll: 0,
            },
        ];
        for tc in cases {
            let mut h = Harness::new(DeviceSelection::SystemDefault);
            let first = h.start().expect("first start");
            assert!(!first.warm, "{}: first start is cold", tc.name);
            assert_eq!(first.pre_roll_samples, 0, "{}", tc.name);
            h.advance(ms(300));
            h.stop();

            h.signal = 0.5;
            h.advance(tc.gap);
            h.signal = 0.75;
            let second = h.start().expect("second start");
            assert_eq!(second.warm, tc.want_warm, "{}", tc.name);
            assert_eq!(second.pre_roll_samples, tc.want_pre_roll, "{}", tc.name);
            h.advance(ms(100));
            let recording = h.stop();
            assert_eq!(recording.pre_roll_samples, tc.want_pre_roll, "{}", tc.name);
            assert_eq!(recording.warm_start, tc.want_warm, "{}", tc.name);
            let (pre_roll, session) = recording.samples.split_at(tc.want_pre_roll);
            assert!(
                pre_roll.iter().all(|s| *s == 0.5),
                "{}: pre-roll is standby audio",
                tc.name
            );
            assert_eq!(session.len(), 1_600, "{}", tc.name);
            assert!(session.iter().all(|s| *s == 0.75), "{}", tc.name);
            assert_eq!(h.builds(), 1, "{}: never rebuilt", tc.name);
        }
    }

    #[test]
    fn standby_stops_the_mic_after_8_s_but_keeps_it_built_aud_5() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.start().expect("start");
        h.advance(ms(200));
        h.stop();
        h.advance(STANDBY - ms(10));
        assert!(h.world.borrow().running, "still warm just before 8 s");
        h.advance(ms(10));
        assert!(!h.world.borrow().running, "mic closed at 8 s");
        assert_eq!(h.world.borrow().stops, 1);

        let start = h.start().expect("restart");
        assert!(!start.warm);
        assert_eq!(h.builds(), 1, "restart reuses the built input");
    }

    #[test]
    fn stop_flushes_the_resampler_tail_aud_7() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.world.borrow_mut().rate = 48_000;
        h.start().expect("start");
        h.world.borrow_mut().deliver(48_000, 0.25);
        let released = h.engine.mark_release(h.now);
        h.world.borrow_mut().deliver(4_800, 0.25);
        let recording = h.stop();
        assert_eq!(
            recording.samples.len(),
            17_600,
            "every input frame comes out"
        );
        assert!(
            released > 15_000 && released <= 16_000,
            "released at {released}"
        );
        assert_eq!(recording.release_sample_count, Some(released));
        assert_eq!(recording.gate_samples().len(), released);
        // 100 samples from the end: lost without the flush
        let near_end = recording.samples[17_500];
        assert!((near_end - 0.25).abs() < 0.01, "tail is signal: {near_end}");
    }

    #[test]
    fn levels_come_from_channel_0_at_most_30_hz_while_capturing_aud_8() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.world.borrow_mut().channels = 2;
        h.start().expect("start");
        h.advance(ms(1_000));
        let levels: Vec<f32> = h
            .events()
            .into_iter()
            .filter_map(|e| match e {
                AudioEvent::Level(level) => Some(level),
                _ => None,
            })
            .collect();
        assert!((25..=30).contains(&levels.len()), "{} levels", levels.len());
        assert!(
            levels.iter().all(|l| (*l - 0.25).abs() < 1e-6),
            "channel 0 only"
        );
        let recording = h.stop();
        assert_eq!(recording.samples.len(), 16_000, "downmixed to mono");

        h.advance(ms(1_000));
        assert!(h.events().is_empty(), "no levels in standby");
    }

    #[test]
    fn watchdog_rebuilds_once_per_stall_aud_9() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.world.borrow_mut().stalled = true;
        let start = h.start().expect("start");
        h.advance(FIRST_BUFFER_TIMEOUT - ms(10));
        assert_eq!(h.builds(), 1, "no rebuild before 2 s");
        h.advance(ms(10));
        assert_eq!(h.builds(), 2, "one rebuild at 2 s");
        assert_eq!(h.rebuilds(), vec![RebuildReason::MissingFirstBuffer]);

        h.advance(FIRST_BUFFER_TIMEOUT);
        assert_eq!(h.builds(), 2, "no second rebuild for the same stall");
        assert_eq!(
            h.events(),
            vec![AudioEvent::Interrupted {
                capture_id: start.capture_id,
                error: AudioError::NoBuffers,
            }]
        );
    }

    #[test]
    fn another_rebuild_resets_the_watchdog_allowance_aud_9() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.world.borrow_mut().stalled = true;
        h.start().expect("start");
        h.advance(FIRST_BUFFER_TIMEOUT);
        assert_eq!(h.rebuilds(), vec![RebuildReason::MissingFirstBuffer]);

        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        assert_eq!(h.rebuilds(), vec![RebuildReason::DefaultDeviceChanged]);
        h.advance(FIRST_BUFFER_TIMEOUT);
        assert_eq!(
            h.rebuilds(),
            vec![RebuildReason::MissingFirstBuffer],
            "a new stall earns a new rebuild"
        );
        assert_eq!(h.builds(), 4);
    }

    #[test]
    fn audio_disarms_the_watchdog_aud_9() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.start().expect("start");
        h.advance(ms(5_000));
        assert_eq!(h.builds(), 1);
        assert!(h.rebuilds().is_empty());
    }

    #[test]
    fn rebuild_retries_6_times_200_ms_apart_then_stops_keeping_samples_aud_10() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        let start = h.start().expect("start");
        h.advance(ms(1_000));
        {
            let mut world = h.world.borrow_mut();
            world.build_failures = (0..REBUILD_ATTEMPTS)
                .map(|_| AudioError::InvalidFormat)
                .collect();
            world.fault = Some(StreamFault::DeviceLost);
        }
        h.advance(ms(10));
        assert_eq!(h.builds(), 2, "first attempt right away");
        h.advance(REBUILD_RETRY_DELAY * 4);
        assert_eq!(h.builds(), 6, "attempts 200 ms apart");
        assert!(h.events().iter().all(|e| matches!(e, AudioEvent::Level(_))));

        h.advance(REBUILD_RETRY_DELAY);
        assert_eq!(h.builds(), 7, "six attempts in all");
        let interruptions: Vec<AudioEvent> = h
            .events()
            .into_iter()
            .filter(|e| !matches!(e, AudioEvent::Level(_)))
            .collect();
        assert_eq!(
            interruptions,
            vec![AudioEvent::Interrupted {
                capture_id: start.capture_id,
                error: AudioError::RecoveryFailed(Box::new(AudioError::InvalidFormat)),
            }]
        );
        assert_eq!(
            AudioError::RecoveryFailed(Box::new(AudioError::InvalidFormat)).to_string(),
            "Could not reconnect to the microphone: Invalid audio format configuration"
        );

        h.advance(ms(1_000));
        assert_eq!(h.builds(), 7, "gives up");
        let recording = h.stop();
        assert_eq!(recording.samples.len(), 16_160, "captured audio is kept");
    }

    #[test]
    fn successful_rebuild_while_recording_keeps_the_session_and_coalesces_aud_10_aud_11a() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.start().expect("start");
        h.advance(ms(500));
        h.world.borrow_mut().build_failures =
            [AudioError::InvalidFormat, AudioError::InvalidFormat].into();
        h.world.borrow_mut().default = Some(ABED.to_owned());
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        assert_eq!(h.builds(), 2);

        // overlapping requests while the rebuild retries
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        h.advance(ms(100));
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        h.advance(REBUILD_RETRY_DELAY * 2 - ms(100));
        assert_eq!(h.builds(), 4, "three attempts, nothing extra");
        let rebuilt: Vec<AudioEvent> = h
            .events()
            .into_iter()
            .filter(|e| !matches!(e, AudioEvent::Level(_)))
            .collect();
        assert_eq!(
            rebuilt,
            vec![AudioEvent::Rebuilt {
                reason: RebuildReason::DefaultDeviceChanged,
                device_name: "Abed's Podcast Mic".to_owned(),
            }]
        );

        h.advance(ms(500));
        let recording = h.stop();
        assert_eq!(
            recording.samples.len(),
            16_000,
            "both inputs' audio, none lost"
        );
    }

    #[test]
    fn idle_rebuild_ends_standby_and_drops_pre_roll_aud_11() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.start().expect("start");
        h.advance(ms(300));
        h.stop();
        h.advance(ms(1_000));
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        assert_eq!(h.builds(), 2, "rebuilds right away");
        assert!(!h.world.borrow().running, "standby ended");

        let start = h.start().expect("start");
        assert!(!start.warm);
        assert_eq!(start.pre_roll_samples, 0);
        assert_eq!(h.builds(), 2, "the rebuilt input is used");
    }

    #[test]
    fn idle_rebuild_failure_is_only_logged_and_the_next_start_builds_aud_11() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.engine.prepare();
        h.world.borrow_mut().build_failures = (0..REBUILD_ATTEMPTS)
            .map(|_| AudioError::InvalidFormat)
            .collect();
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        h.advance(ms(2_000));
        assert_eq!(h.builds(), 1 + REBUILD_ATTEMPTS);
        assert!(h.events().is_empty(), "nothing reported while idle");

        h.start().expect("start builds from scratch");
        assert_eq!(h.builds(), 2 + REBUILD_ATTEMPTS);
    }

    #[test]
    fn route_change_during_capture_forces_a_fresh_build_at_next_start_aud_11() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.start().expect("start");
        h.advance(ms(300));
        // the rebuild's first attempt fails, so the capture stops while the
        // input is still stale
        h.world.borrow_mut().build_failures = [AudioError::InvalidFormat].into();
        h.world.borrow_mut().default = Some(ABED.to_owned());
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        assert_eq!(h.builds(), 2);
        h.stop();
        assert_eq!(
            h.builds(),
            3,
            "stop builds afresh instead of keeping it warm"
        );
        assert!(!h.world.borrow().running, "no stale warm standby");

        h.advance(ms(1_000));
        let start = h.start().expect("start");
        assert!(!start.warm, "a cold start on the fresh input");
        assert_eq!(start.pre_roll_samples, 0);
        assert_eq!(start.device_name, "Abed's Podcast Mic");
        assert_eq!(h.builds(), 3, "the fresh build is used");
    }

    #[test]
    fn request_while_not_prepared_only_marks_stale_aud_11() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.engine
            .device_changed(DeviceChange::DefaultInputChanged, h.now);
        h.advance(ms(1_000));
        assert_eq!(h.builds(), 0);
        h.start().expect("start");
        assert_eq!(h.builds(), 1);
    }

    #[test]
    fn device_changes_rebuild_only_when_they_matter_aud_10() {
        struct Case {
            name: &'static str,
            selection: DeviceSelection,
            change: fn(&mut World) -> DeviceChange,
            want_rebuild: Option<RebuildReason>,
        }
        let cases = [
            Case {
                name: "default change while following the default",
                selection: DeviceSelection::SystemDefault,
                change: |_| DeviceChange::DefaultInputChanged,
                want_rebuild: Some(RebuildReason::DefaultDeviceChanged),
            },
            Case {
                name: "default change with a specific device",
                selection: DeviceSelection::Device(ABED.to_owned()),
                change: |_| DeviceChange::DefaultInputChanged,
                want_rebuild: None,
            },
            Case {
                name: "unrelated device plugged in",
                selection: DeviceSelection::Device(ABED.to_owned()),
                change: |w| {
                    w.plug("greendale-study-room", "Study Room Mic");
                    DeviceChange::DeviceListChanged
                },
                want_rebuild: None,
            },
            Case {
                name: "selected device reconnected under the same id",
                selection: DeviceSelection::Device(ABED.to_owned()),
                change: |w| {
                    w.plug(ABED, "Abed's Podcast Mic");
                    DeviceChange::DeviceListChanged
                },
                want_rebuild: Some(RebuildReason::DeviceListChanged),
            },
            Case {
                name: "list change while following the default",
                selection: DeviceSelection::SystemDefault,
                change: |w| {
                    w.plug(ABED, "Abed's Podcast Mic");
                    DeviceChange::DeviceListChanged
                },
                want_rebuild: None,
            },
        ];
        for tc in cases {
            let mut h = Harness::new(tc.selection);
            h.engine.prepare();
            let change = (tc.change)(&mut h.world.borrow_mut());
            h.engine.device_changed(change, h.now);
            assert_eq!(h.rebuilds(), Vec::from_iter(tc.want_rebuild), "{}", tc.name);
        }
    }

    #[test]
    fn selection_change_rebuilds_against_the_new_device_aud_10() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.engine.prepare();
        h.engine
            .set_selection(DeviceSelection::Device(ABED.to_owned()), h.now);
        assert_eq!(h.rebuilds(), vec![RebuildReason::SelectionChanged]);
        assert_eq!(
            h.world.borrow().builds,
            vec!["".to_owned(), ABED.to_owned()]
        );
        let start = h.start().expect("start");
        assert_eq!(start.device_name, "Abed's Podcast Mic");
    }

    #[test]
    fn missing_selected_device_fails_without_falling_back_aud_3() {
        let mut h = Harness::new(DeviceSelection::Device("study-room-mic".to_owned()));
        let err = h.start().expect_err("must fail");
        assert_eq!(err, AudioError::SelectedDeviceMissing);
        assert_eq!(err.to_string(), "The selected microphone is not connected");
        assert_eq!(h.world.borrow().builds, vec!["study-room-mic".to_owned()]);
        assert_eq!(h.world.borrow().starts, 0, "no other input was opened");
    }

    #[test]
    fn selected_device_unplugged_while_recording_fails_recovery_aud_3_aud_10() {
        let mut h = Harness::new(DeviceSelection::Device(ABED.to_owned()));
        let start = h.start().expect("start");
        h.advance(ms(200));
        h.world.borrow_mut().devices.retain(|(id, _, _)| id != ABED);
        h.engine
            .device_changed(DeviceChange::DeviceListChanged, h.now);
        h.advance(REBUILD_RETRY_DELAY * 5);
        let interruptions: Vec<AudioEvent> = h
            .events()
            .into_iter()
            .filter(|e| !matches!(e, AudioEvent::Level(_)))
            .collect();
        assert_eq!(
            interruptions,
            vec![AudioEvent::Interrupted {
                capture_id: start.capture_id,
                error: AudioError::RecoveryFailed(Box::new(AudioError::SelectedDeviceMissing)),
            }]
        );
        assert!(
            h.world.borrow().builds.iter().all(|b| b == ABED),
            "never falls back to the default"
        );
        assert_eq!(h.stop().samples.len(), 3_200);
    }

    #[test]
    fn input_that_fails_to_start_is_rebuilt_next_time_aud_11b() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        h.engine.prepare();
        h.world
            .borrow_mut()
            .start_failures
            .push_back(AudioError::StartFailed("Chang's lock is busy".to_owned()));
        let err = h.start().expect_err("start fails");
        assert_eq!(err.to_string(), "Chang's lock is busy");
        h.start().expect("second start");
        assert_eq!(h.builds(), 2, "full rebuild on the next attempt");
    }

    #[test]
    fn late_interruption_names_its_capture() {
        let mut h = Harness::new(DeviceSelection::SystemDefault);
        let first = h.start().expect("first");
        h.advance(ms(100));
        h.stop();
        let second = h.start().expect("second");
        assert_ne!(first.capture_id, second.capture_id);
    }

    #[test]
    fn speech_gate_sees_only_samples_before_the_release_gate_6() {
        let speech = vec![0.1; 16_000];
        let cue = vec![0.5; 3_200];
        struct Case {
            name: &'static str,
            samples: Vec<f32>,
            release: Option<usize>,
            want: GateVerdict,
        }
        let cases = [
            Case {
                name: "no release judges everything",
                samples: [vec![0.001; 16_000], cue.clone()].concat(),
                release: None,
                want: GateVerdict::Accept,
            },
            Case {
                name: "stop cue in the tail cannot pass",
                samples: [vec![0.001; 16_000], cue.clone()].concat(),
                release: Some(16_000),
                want: GateVerdict::RejectWithWarning,
            },
            Case {
                name: "speech before the release passes",
                samples: [speech, cue].concat(),
                release: Some(16_000),
                want: GateVerdict::Accept,
            },
        ];
        for tc in cases {
            let recording = Recording {
                samples: tc.samples,
                release_sample_count: tc.release,
                ..Recording::default()
            };
            assert_eq!(recording.speech_gate(), tc.want, "{}", tc.name);
        }
    }
}
