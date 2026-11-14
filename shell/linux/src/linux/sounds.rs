//! Onda's start and stop cues (SND-1, SND-2).
//!
//! The cues are the macOS ones as WAV, which needs no codec, and they play
//! through ALSA, the one audio library every package depends on. A thread
//! owns the output streams. Each cue has a stream of its own, open only
//! while the cue sounds: an idle open stream keeps the output device awake.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use gtk::glib;
use onda_app::ports::{Cue, SoundPlayer};

/// How long a stream stays open past its cue's length. Opening the stream
/// takes part of it, and the audio server and the device play what they
/// buffered in the rest.
const LINGER: Duration = Duration::from_millis(500);

pub struct Sounds {
    cues: mpsc::Sender<Cue>,
}

/// Cue playback from the sound files installed with this binary.
pub fn start() -> Sounds {
    let (cues, inbox) = mpsc::channel();
    let clips = Clips::load(&folders());
    let spawned = std::thread::Builder::new()
        .name("onda-sounds".into())
        .spawn(move || run(&clips, &inbox));
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start sound playback");
    }
    Sounds { cues }
}

impl SoundPlayer for Sounds {
    fn play(&self, cue: Cue) {
        if self.cues.send(cue).is_err() {
            tracing::debug!("sound playback is not running");
        }
    }
}

/// Where the cue files may be, best first: the prefix this binary is
/// installed in, then the XDG data folders, because the archive installer
/// puts `share/` in `XDG_DATA_HOME` when that is set.
fn folders() -> Vec<PathBuf> {
    let prefix = std::env::current_exe()
        .ok()
        .and_then(|binary| Some(binary.parent()?.parent()?.join("share")));
    prefix
        .into_iter()
        .chain([glib::user_data_dir()])
        .chain(glib::system_data_dirs())
        .map(|data| data.join("onda/sounds"))
        .collect()
}

fn file_name(cue: Cue) -> &'static str {
    match cue {
        Cue::Start => "dictation_start.wav",
        Cue::Stop => "dictation_stop.wav",
    }
}

/// A cue that is sounding.
struct Playing {
    cue: Cue,
    /// dropping it stops the sound
    _stream: cpal::Stream,
    /// when the cue has been heard in full
    heard: Instant,
}

fn run(clips: &Clips, inbox: &mpsc::Receiver<Cue>) {
    let mut playing: Vec<Playing> = Vec::new();
    loop {
        let now = Instant::now();
        playing.retain(|sound| sound.heard > now);
        let next = match playing.iter().map(|sound| sound.heard).min() {
            Some(heard) => inbox.recv_timeout(heard - now),
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let cue = match next {
            Ok(cue) => cue,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // a cue that is still sounding starts over (SND-1)
        playing.retain(|sound| sound.cue != cue);
        let Some(clip) = clips.of(cue) else { continue };
        match play(cue, clip) {
            Ok(sound) => playing.push(sound),
            Err(error) => {
                tracing::warn!(?cue, error = %format!("{error:#}"), "could not play a sound cue");
            }
        }
    }
}

/// Opens a stream on the default output and starts `clip` on it. The stream
/// asks for the file's own format, which the ALSA default device converts.
fn play(cue: Cue, clip: &Clip) -> anyhow::Result<Playing> {
    let device = cpal::default_host()
        .default_output_device()
        .context("no audio output")?;
    let config = cpal::StreamConfig {
        channels: clip.channels,
        sample_rate: clip.rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let samples = Arc::clone(&clip.samples);
    let mut next = 0;
    let stream = device.build_output_stream::<i16, _, _>(
        config,
        // realtime: no locks, no allocation
        move |out: &mut [i16], _| {
            let rest = &samples[next..];
            let (sound, silence) = out.split_at_mut(rest.len().min(out.len()));
            sound.copy_from_slice(&rest[..sound.len()]);
            silence.fill(0);
            next += sound.len();
        },
        |error| {
            if error.kind() != cpal::ErrorKind::Xrun {
                tracing::warn!(%error, "sound cue playback failed");
            }
        },
        None,
    )?;
    stream.play()?;
    Ok(Playing {
        cue,
        _stream: stream,
        heard: Instant::now() + clip.length() + LINGER,
    })
}

/// The cue files, read once at launch: a cue then starts without the disk,
/// and a file that is missing is reported once and never fatal (SND-2).
struct Clips {
    start: Option<Clip>,
    stop: Option<Clip>,
}

impl Clips {
    fn load(folders: &[PathBuf]) -> Self {
        let load = |cue| match Clip::load(folders, file_name(cue)) {
            Ok(clip) => Some(clip),
            Err(error) => {
                tracing::warn!(?cue, error = %format!("{error:#}"), "sound cue will stay silent");
                None
            }
        };
        Self {
            start: load(Cue::Start),
            stop: load(Cue::Stop),
        }
    }

    fn of(&self, cue: Cue) -> Option<&Clip> {
        match cue {
            Cue::Start => self.start.as_ref(),
            Cue::Stop => self.stop.as_ref(),
        }
    }
}

/// A decoded cue file: 16-bit samples, interleaved by channel.
struct Clip {
    channels: u16,
    rate: u32,
    samples: Arc<[i16]>,
}

impl Clip {
    /// Reads `name` from the first of `folders` that has it.
    fn load(folders: &[PathBuf], name: &str) -> anyhow::Result<Self> {
        let path = folders
            .iter()
            .map(|folder| folder.join(name))
            .find(|path| path.is_file())
            .with_context(|| format!("missing sound {name}, looked in {folders:?}"))?;
        let mut reader = hound::WavReader::open(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let spec = reader.spec();
        anyhow::ensure!(
            spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
            "{} is not 16-bit PCM",
            path.display()
        );
        let samples = reader
            .samples::<i16>()
            .collect::<Result<Arc<[i16]>, _>>()
            .with_context(|| format!("could not read {}", path.display()))?;
        Ok(Self {
            channels: spec.channels,
            rate: spec.sample_rate,
            samples,
        })
    }

    fn length(&self) -> Duration {
        let frames = self.samples.len() / usize::from(self.channels.max(1));
        Duration::from_secs_f64(frames as f64 / f64::from(self.rate.max(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the cue files as the repository ships them
    fn shipped() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds"))
    }

    // The macOS cues are AAC with 10,804 frames at 44.1 kHz between the
    // encoder's priming and padding. The WAV files must hold those frames.
    #[test]
    fn shipped_cues_hold_the_macos_cues() {
        for cue in [Cue::Start, Cue::Stop] {
            let clip = Clip::load(&[shipped()], file_name(cue)).unwrap();
            assert_eq!((clip.channels, clip.rate), (1, 44_100), "{cue:?}");
            assert_eq!(clip.samples.len(), 10_804, "{cue:?}");
            assert_eq!(clip.length(), Duration::from_secs_f64(10_804.0 / 44_100.0));
            let peak = clip.samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(peak > 3_000, "{cue:?} is close to silent: peak {peak}");
        }
    }

    #[test]
    fn a_cue_comes_from_the_first_folder_that_has_it() {
        let empty = tempfile::tempdir().unwrap();
        let folders = [empty.path().to_owned(), shipped()];
        assert!(Clip::load(&folders, file_name(Cue::Stop)).is_ok());
    }

    #[test]
    fn a_missing_or_broken_cue_is_an_error_that_names_the_file() {
        let folder = tempfile::tempdir().unwrap();
        let folders = [folder.path().to_owned()];
        let missing = Clip::load(&folders, "dictation_start.wav").err().unwrap();
        assert!(format!("{missing:#}").contains("missing sound dictation_start.wav"));
        std::fs::write(folder.path().join("dictation_start.wav"), b"Troy Barnes").unwrap();
        let broken = Clip::load(&folders, "dictation_start.wav").err().unwrap();
        assert!(format!("{broken:#}").contains("could not read"));
    }

    // SND-2: the thread that plays cues outlives the cues it has no file for
    #[test]
    fn missing_cue_files_do_not_stop_playback() {
        let (cues, inbox) = mpsc::channel();
        let worker = std::thread::spawn(move || run(&Clips::load(&[]), &inbox));
        cues.send(Cue::Start).unwrap();
        cues.send(Cue::Stop).unwrap();
        drop(cues);
        worker.join().unwrap();
    }
}
