//! The public capture handle: a worker thread that owns the engine and the
//! stream, driven by commands over a channel.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use onda_domain::pre_roll::SAMPLE_RATE;
use tracing::{debug, info, warn};

use crate::cpal_backend::CpalBackend;
use crate::engine::{AudioEvent, Backend, CaptureStart, Engine, Recording};
use crate::{AudioError, DeviceChange, DeviceChangeSink, DeviceSelection};

pub(crate) enum Command {
    Prepare,
    Start(Sender<Result<CaptureStart, AudioError>>),
    MarkRelease(Sender<usize>),
    Stop(Sender<Recording>),
    Samples(Sender<Vec<f32>>),
    SampleCount(Sender<usize>),
    SetSelection(DeviceSelection),
    DeviceChanged(DeviceChange),
    Shutdown,
}

/// Microphone capture to 16 kHz mono with warm standby, pre-roll, and input
/// recovery (03 "Audio capture").
///
/// A worker thread owns the stream; every method is a short request to it.
/// Events arrive on the receiver returned by [`AudioCapture::new`].
/// Dropping the handle closes the microphone and joins the worker.
pub struct AudioCapture {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Starts the capture worker for `selection` on the system audio host.
    /// Nothing is built until [`AudioCapture::prepare`] or
    /// [`AudioCapture::start`].
    pub fn new(selection: DeviceSelection) -> Result<(Self, Receiver<AudioEvent>), AudioError> {
        let (commands, inbox) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("onda-audio".to_owned())
            .spawn(move || run(Engine::new(CpalBackend::new(), selection, events_tx), inbox))
            .map_err(|err| AudioError::StartFailed(err.to_string()))?;
        let capture = Self {
            commands,
            worker: Some(worker),
        };
        Ok((capture, events))
    }

    /// Builds the input ahead of time without opening the microphone
    /// (AUD-4). Call only once microphone permission is granted.
    pub fn prepare(&self) {
        self.send(Command::Prepare);
    }

    /// Opens a capture; instant from warm standby, with pre-roll (AUD-5,
    /// AUD-6). A missing selected device fails with
    /// [`AudioError::SelectedDeviceMissing`] (AUD-3).
    pub fn start(&self) -> Result<CaptureStart, AudioError> {
        self.request(Command::Start)
            .unwrap_or_else(|| Err(worker_gone()))
    }

    /// Records the hotkey release and returns the sample count at that
    /// moment, so the speech gate skips the tail (GATE-6, DICT-11).
    pub fn mark_release(&self) -> usize {
        self.request(Command::MarkRelease).unwrap_or_default()
    }

    /// Closes the capture and returns everything captured, resampler tail
    /// included (AUD-7). The input idles warm for 8 s (AUD-5).
    pub fn stop(&self) -> Recording {
        self.request(Command::Stop).unwrap_or_default()
    }

    /// A copy of the open capture's samples so far.
    pub fn samples(&self) -> Vec<f32> {
        self.request(Command::Samples).unwrap_or_default()
    }

    /// The open capture's sample count so far, without copying.
    pub fn sample_count(&self) -> usize {
        self.request(Command::SampleCount).unwrap_or_default()
    }

    /// Switches to `selection`, rebuilding the input if it changed (AUD-10).
    pub fn set_selection(&self, selection: DeviceSelection) {
        self.send(Command::SetSelection(selection));
    }

    /// Where an [`AudioDeviceEvents`](crate::AudioDeviceEvents) source sends
    /// default-input and device-list changes.
    pub fn device_change_sink(&self) -> DeviceChangeSink {
        DeviceChangeSink::new(self.commands.clone())
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            warn!("audio worker is gone");
        }
    }

    fn request<T>(&self, command: impl FnOnce(Sender<T>) -> Command) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.send(command(reply));
        answer.recv().ok()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.send(Command::Shutdown);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            warn!("audio worker panicked");
        }
    }
}

fn worker_gone() -> AudioError {
    AudioError::StartFailed("The audio worker stopped".to_owned())
}

fn run<B: Backend>(mut engine: Engine<B>, inbox: Receiver<Command>) {
    let origin = Instant::now();
    loop {
        let wait = engine.next_wake(origin.elapsed());
        match inbox.recv_timeout(wait) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(command) => handle(&mut engine, command, origin),
            Err(RecvTimeoutError::Timeout) => {}
        }
        engine.tick(origin.elapsed());
    }
}

fn handle<B: Backend>(engine: &mut Engine<B>, command: Command, origin: Instant) {
    let now = origin.elapsed();
    match command {
        Command::Prepare => engine.prepare(),
        Command::Start(tx) => {
            let began = Instant::now();
            let result = engine.start(now);
            if let Ok(start) = &result {
                info!(
                    ready_ms = began.elapsed().as_millis() as u64,
                    pre_roll_ms = (start.pre_roll_samples * 1_000 / SAMPLE_RATE) as u64,
                    warm = start.warm,
                    "audio capture ready"
                );
            }
            reply(tx, result);
        }
        Command::MarkRelease(tx) => reply(tx, engine.mark_release(now)),
        Command::Stop(tx) => reply(tx, engine.stop(now)),
        Command::Samples(tx) => reply(tx, engine.samples(now)),
        Command::SampleCount(tx) => reply(tx, engine.sample_count(now)),
        Command::SetSelection(selection) => engine.set_selection(selection, now),
        Command::DeviceChanged(change) => engine.device_changed(change, now),
        Command::Shutdown => {}
    }
}

fn reply<T>(tx: Sender<T>, value: T) {
    if tx.send(value).is_err() {
        debug!("audio caller stopped waiting");
    }
}
