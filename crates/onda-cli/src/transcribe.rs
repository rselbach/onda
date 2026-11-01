//! `transcribe`: run a WAV through a speech engine and time it (M3).
//!
//! `--bench` measures preview latency (ASR-17): it decodes 1, 5, 10, and
//! 15 s prefixes repeatedly and prints the median and p95. `--stream`
//! replays a live dictation instead: a preview every 300 ms of a growing
//! prefix, then the final decode, compared with an offline decode.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use onda_asr::SpeechEngine;
use onda_asr_coreml::ParakeetEngine;
use onda_domain::languages::{Language, LanguageSetting};
use onda_domain::models::ModelId;
use onda_domain::preview::{PREVIEW_INTERVAL, PREVIEW_MIN_SAMPLES};

use crate::models::parse_model;

const SAMPLE_RATE: u32 = 16_000;
const BENCH_PREFIXES: [usize; 4] = [1, 5, 10, 15];
const BENCH_WARMUP: usize = 3;
const BENCH_RUNS: usize = 30;

/// `transcribe` options.
#[derive(clap::Args)]
pub struct Args {
    /// 16 kHz WAV file.
    wav: PathBuf,
    /// Model id.
    #[arg(long, default_value = "parakeet-tdt-v2", value_parser = parse_model)]
    model: ModelId,
    /// Language code passed to the engine (ASR-11); `auto` passes none.
    #[arg(long, default_value = "auto", value_parser = parse_language)]
    language: LanguageSetting,
    /// Also measure preview-decode latency.
    #[arg(long)]
    bench: bool,
    /// Also replay live previews every 300 ms, then the final decode.
    #[arg(long)]
    stream: bool,
}

fn parse_language(code: &str) -> Result<LanguageSetting, String> {
    LanguageSetting::parse(code).ok_or_else(|| format!("unknown language '{code}'"))
}

pub fn run(args: &Args, models_dir: &Path) -> Result<()> {
    let samples = read_wav(&args.wav)?;
    let audio_seconds = samples.len() as f64 / f64::from(SAMPLE_RATE);
    let language = args.language.engine_language();
    let model_dir = models_dir.join(args.model.as_str());
    let Some(mut engine) = ParakeetEngine::new(args.model, &model_dir) else {
        bail!("{} has no engine yet", args.model);
    };

    let started = Instant::now();
    engine.load(&mut |progress| {
        eprintln!("  {:>3.0}% {}", progress.fraction * 100.0, progress.status)
    })?;
    println!("load: {:.2} s", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let text = engine.transcribe(&samples, language)?;
    let decode = started.elapsed().as_secs_f64();
    println!("audio: {audio_seconds:.2} s");
    println!("decode: {:.1} ms", decode * 1e3);
    println!("real-time factor: {:.4}", decode / audio_seconds);
    println!("transcript: {text}");

    if args.bench {
        bench(&mut engine, &samples, language)?;
    }
    if args.stream {
        stream(&mut engine, &samples, language, &text)?;
    }
    Ok(())
}

// reads a 16 kHz WAV as mono f32, averaging channels
fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).with_context(|| path.display().to_string())?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE {
        bail!(
            "{} is {} Hz; convert it first: afconvert -f WAVE -d LEI16@16000 -c 1 in.wav out.wav",
            path.display(),
            spec.sample_rate
        );
    }
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()
        }
    }
    .with_context(|| path.display().to_string())?;
    let channels = usize::from(spec.channels.max(1));
    Ok(interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect())
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn bench(engine: &mut ParakeetEngine, samples: &[f32], language: Option<Language>) -> Result<()> {
    println!("preview latency ({BENCH_RUNS} runs after {BENCH_WARMUP} warm-up):");
    for seconds in BENCH_PREFIXES {
        let Some(prefix) = samples.get(..seconds * SAMPLE_RATE as usize) else {
            println!("  {seconds:>2} s: skipped, the recording is shorter");
            continue;
        };
        for _ in 0..BENCH_WARMUP {
            engine.preview(prefix, language)?;
        }
        let mut times = Vec::with_capacity(BENCH_RUNS);
        for _ in 0..BENCH_RUNS {
            // a streaming engine would otherwise see no new audio
            engine.reset_preview();
            let started = Instant::now();
            engine.preview(prefix, language)?;
            times.push(started.elapsed());
        }
        engine.reset_preview();
        times.sort();
        println!(
            "  {seconds:>2} s: median {:.1} ms, p95 {:.1} ms",
            ms(percentile(&times, 0.5)),
            ms(percentile(&times, 0.95))
        );
    }
    Ok(())
}

// one preview per 300 ms of new audio, as the session's preview loop
// does in real time, then the final decode of the whole recording
fn stream(
    engine: &mut ParakeetEngine,
    samples: &[f32],
    language: Option<Language>,
    offline: &str,
) -> Result<()> {
    let tick = (PREVIEW_INTERVAL.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize;
    engine.reset_preview();
    let mut times = Vec::new();
    let mut last_preview = String::new();
    for len in (PREVIEW_MIN_SAMPLES..samples.len()).step_by(tick) {
        let started = Instant::now();
        last_preview = engine.preview(&samples[..len], language)?;
        times.push(started.elapsed());
    }
    let started = Instant::now();
    let text = engine.transcribe(samples, language)?;
    let final_decode = started.elapsed();
    let mut sorted = times.clone();
    sorted.sort();
    if sorted.is_empty() {
        println!("stream: the recording is too short for a preview");
    } else {
        println!(
            "stream: {} previews, median {:.1} ms, p95 {:.1} ms, max {:.1} ms, total {:.0} ms",
            sorted.len(),
            ms(percentile(&sorted, 0.5)),
            ms(percentile(&sorted, 0.95)),
            ms(percentile(&sorted, 1.0)),
            ms(times.iter().sum())
        );
    }
    println!("stream last preview: {last_preview}");
    println!("stream final decode: {:.1} ms", ms(final_decode));
    println!("stream transcript: {text}");
    println!(
        "stream final matches offline: {}",
        if text == offline { "yes" } else { "no" }
    );
    Ok(())
}
