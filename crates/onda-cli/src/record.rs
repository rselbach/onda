//! `devices` and `record`: microphone capture (M2).
//!
//! Each session ends like a hotkey stop: release, then a 200 ms tail that the
//! speech gate ignores (DICT-11, GATE-6). With `--gap-ms` under 8000 the next
//! session starts warm, with pre-roll (AUD-5, AUD-6). Device changes are
//! polled and logged while running.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use onda_audio::{
    AudioCapture, AudioDeviceEvents, AudioEvent, DeviceSelection, PollingDeviceEvents,
    input_devices,
};
use onda_domain::history::encode_wav;
use onda_domain::pre_roll::SAMPLE_RATE;

const TAIL: Duration = Duration::from_millis(200);

/// `record` options.
#[derive(clap::Args)]
pub struct Args {
    /// Input device id from `devices`; the system default when omitted.
    #[arg(long)]
    device: Option<String>,
    /// Seconds to record per session, before the tail.
    #[arg(long, default_value_t = 3.0)]
    seconds: f64,
    /// Number of sessions.
    #[arg(long, default_value_t = 1)]
    sessions: usize,
    /// Pause between sessions in milliseconds.
    #[arg(long, default_value_t = 1_000)]
    gap_ms: u64,
    /// Output prefix; session N goes to <PREFIX>-N.wav.
    #[arg(long, default_value = "recording")]
    out: String,
}

pub fn devices() -> Result<()> {
    for device in input_devices()? {
        let marker = if device.is_default { "*" } else { " " };
        println!("{marker} {}\t{}", device.id, device.name);
    }
    Ok(())
}

pub fn run(args: &Args) -> Result<()> {
    let selection = args
        .device
        .clone()
        .map_or(DeviceSelection::SystemDefault, DeviceSelection::Device);
    let (capture, events) = AudioCapture::new(selection)?;
    let mut devices = PollingDeviceEvents::new(Duration::from_millis(500));
    devices.subscribe(capture.device_change_sink());
    capture.prepare();

    for session in 1..=args.sessions {
        let start = capture
            .start()
            .map_err(|error| anyhow!("{} ({})", error, error.notice().title()))?;
        let mut peak_level = 0.0_f32;
        let wanted = Duration::from_secs_f64(args.seconds);
        let interrupted = watch(&events, wanted, start.capture_id, &mut peak_level);
        let tail = if interrupted { Duration::ZERO } else { TAIL };
        let released = capture.mark_release();
        watch(&events, tail, start.capture_id, &mut peak_level);
        let recording = capture.stop();

        let path = format!("{}-{session}.wav", args.out);
        std::fs::write(&path, encode_wav(&recording.samples)?).with_context(|| path.clone())?;
        let sample_peak = recording
            .samples
            .iter()
            .fold(0.0_f32, |peak, s| peak.max(s.abs()));
        let latency = recording
            .first_buffer_latency
            .map_or("none".to_owned(), |d| format!("{} ms", d.as_millis()));
        println!(
            "session {session}: {path} on {device}\n  duration {duration:.3} s ({samples} samples), warm start {warm}, pre-roll {pre_roll} samples ({pre_roll_ms} ms)\n  first buffer {latency}, peak level {peak_level:.4}, sample peak {sample_peak:.4}\n  released at {released} samples, speech gate {gate:?}",
            device = start.device_name,
            duration = recording.samples.len() as f64 / SAMPLE_RATE as f64,
            samples = recording.samples.len(),
            warm = recording.warm_start,
            pre_roll = recording.pre_roll_samples,
            pre_roll_ms = recording.pre_roll_samples * 1_000 / SAMPLE_RATE,
            gate = recording.speech_gate(),
        );

        if session < args.sessions {
            watch(&events, Duration::from_millis(args.gap_ms), 0, &mut 0.0);
        }
    }
    Ok(())
}

// prints events for `duration`; returns early with true if capture
// `capture_id` was interrupted
fn watch(
    events: &Receiver<AudioEvent>,
    duration: Duration,
    capture_id: u64,
    peak: &mut f32,
) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(AudioEvent::Level(level)) => *peak = peak.max(level),
            Ok(AudioEvent::Interrupted {
                capture_id: id,
                error,
            }) => {
                println!("  interrupted: {error} ({})", error.notice().title());
                if id == capture_id {
                    return true;
                }
            }
            Ok(AudioEvent::ConversionFailed { error, .. }) => println!("  {error}"),
            Ok(AudioEvent::Rebuilt {
                reason,
                device_name,
            }) => println!("  input rebuilt because {reason}; now {device_name}"),
            Err(RecvTimeoutError::Timeout) => return false,
            Err(RecvTimeoutError::Disconnected) => return true,
        }
    }
}
