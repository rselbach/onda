//! Runs the Parakeet models for real. Needs macOS `say` and
//! `ONDA_TEST_MODEL_DIR` set to one model's folder, or to the models folder
//! holding several; tests for models not found there are skipped:
//!
//! ```text
//! ONDA_TEST_MODEL_DIR=target/dev-models \
//!     cargo test -p onda-asr-coreml --test real_model -- --ignored
//! ```

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use onda_asr::{AsrError, SpeechEngine};
use onda_asr_coreml::ParakeetEngine;
use onda_domain::languages::Language;
use onda_domain::models::ModelId;

/// The folder of `model` under `ONDA_TEST_MODEL_DIR`, or `None` (skip)
/// when it is not there.
fn model_dir(model: ModelId) -> Option<PathBuf> {
    // relative paths are from the workspace root, where cargo is usually run
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = workspace.join(std::env::var("ONDA_TEST_MODEL_DIR").expect("ONDA_TEST_MODEL_DIR"));
    let found = if dir.ends_with(model.as_str()) {
        Some(dir.clone())
    } else {
        Some(dir.join(model.as_str())).filter(|d| d.is_dir())
    };
    if found.is_none() {
        eprintln!("skipped: no {} in {}", model.as_str(), dir.display());
    }
    found
}

fn loaded(model: ModelId) -> Option<ParakeetEngine> {
    let dir = model_dir(model)?;
    let mut engine = ParakeetEngine::new(model, dir).expect("supported model");
    engine.load(&mut |_| {}).expect("load");
    Some(engine)
}

fn language(code: &str) -> Option<Language> {
    Some(Language::from_code(code).expect("offered language"))
}

/// Speaks `text` with `voice` into a 16 kHz mono WAV and reads it back.
fn speak(voice: &str, text: &str) -> Vec<f32> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "onda-asr-test-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let status = Command::new("say")
        .args([
            "-v",
            voice,
            "--file-format=WAVE",
            "--data-format=LEI16@16000",
            "-o",
        ])
        .arg(&path)
        .arg(text)
        .status()
        .expect("run say");
    assert!(status.success(), "say failed");
    let mut reader = hound::WavReader::open(&path).expect("open wav");
    assert_eq!(reader.spec().sample_rate, 16_000);
    let samples = reader
        .samples::<i16>()
        .map(|s| f32::from(s.expect("sample")) / 32_768.0)
        .collect();
    std::fs::remove_file(&path).expect("remove wav");
    samples
}

const GREENDALE: &str = "Troy Barnes and Abed Nadir study Spanish at Greendale Community College.";

#[test]
#[ignore = "needs ONDA_TEST_MODEL_DIR and the downloaded model"]
fn transcribes_speech_with_the_real_model() {
    let Some(dir) = model_dir(ModelId::ParakeetTdtV2) else {
        return;
    };
    let mut engine = ParakeetEngine::new(ModelId::ParakeetTdtV2, dir).expect("v2 supported");
    let samples = speak("Samantha", GREENDALE);
    assert_eq!(engine.transcribe(&samples, None), Err(AsrError::NotReady));

    let mut fractions = Vec::new();
    engine
        .load(&mut |progress| fractions.push(progress.fraction))
        .expect("load");
    assert_eq!(fractions.last(), Some(&1.0));

    let text = engine.transcribe(&samples, None).expect("transcribe");
    let lower = text.to_lowercase();
    assert!(lower.contains("troy barnes"), "{text}");
    assert!(lower.contains("greendale community college"), "{text}");
    assert_eq!(text, text.trim());

    // 3 s of silence after the speech pushes it past one 15 s window
    let mut long = samples.clone();
    long.resize(long.len() + 48_000, 0.0);
    while long.len() <= 240_000 {
        long.extend_from_slice(&samples);
    }
    let text = engine.preview(&long, None).expect("preview");
    let greendales = text.to_lowercase().matches("greendale").count();
    assert!(greendales >= 2, "{text}");

    engine.unload();
    assert_eq!(engine.preview(&samples, None), Err(AsrError::NotReady));
}

/// The session engine previews the whole recording every 300 ms (ASR-15),
/// so one engine decodes hundreds of growing prefixes, most of them past
/// one window. Also replays the sequence reported to abort the process:
/// 240,000 samples growing by 5,920 to past 470,880.
#[test]
#[ignore = "needs ONDA_TEST_MODEL_DIR and the downloaded model"]
fn survives_growing_preview_prefixes() {
    let Some(mut engine) = loaded(ModelId::ParakeetTdtV2) else {
        return;
    };
    let speech = speak("Samantha", GREENDALE);
    let mut recording = Vec::new();
    while recording.len() < 960_000 {
        recording.extend_from_slice(&speech);
        recording.resize(recording.len() + 24_000, 0.0);
    }
    let live = (8_000..=960_000).step_by(4_800);
    let reported = (240_000..=480_000).step_by(5_920);
    for len in live.chain(reported) {
        let text = engine.preview(&recording[..len], None).expect("preview");
        if len >= speech.len() {
            assert!(text.to_lowercase().contains("greendale"), "{len}: {text}");
        }
    }
}

/// ASR-17: a preview decodes again only the windows that changed since the
/// last one, and reads exactly as a preview of the same audio decoded from
/// scratch.
#[test]
#[ignore = "needs ONDA_TEST_MODEL_DIR and the downloaded model"]
fn previews_reuse_unchanged_windows() {
    let cases = [
        (ModelId::ParakeetTdtV2, None),
        (ModelId::ParakeetUltra, language("en")),
    ];
    for (model, language) in cases {
        let Some(mut engine) = loaded(model) else {
            continue;
        };
        let speech = speak(
            "Samantha",
            "Troy Barnes and Abed Nadir study Spanish at Greendale Community \
             College. Annie Edison keeps a color coded binder for every class. \
             Dean Craig Pelton announces a new mascot every semester.",
        );
        let mut recording = Vec::new();
        while recording.len() < 70 * 16_000 {
            recording.extend_from_slice(&speech);
            recording.resize(recording.len() + 8_000, 0.0);
        }
        // a preview every 0.4 s of audio, checked every 4 s
        let mut checked = Vec::new();
        for (tick, len) in (8_000..=recording.len()).step_by(6_400).enumerate() {
            let text = engine
                .preview(&recording[..len], language)
                .expect("preview");
            if tick % 10 == 0 {
                checked.push((len, text));
            }
        }
        for (len, want) in checked {
            engine.reset_preview();
            let got = engine
                .preview(&recording[..len], language)
                .expect("preview");
            assert_eq!(got, want, "{}: {len}", model.as_str());
        }
    }
}

/// ASR-12: the language is a script hint. Polish keeps Latin letters and
/// Russian Cyrillic ones; a recording past one window is merged.
#[test]
#[ignore = "needs ONDA_TEST_MODEL_DIR and the downloaded model"]
fn ultra_transcribes_european_languages() {
    let Some(mut engine) = loaded(ModelId::ParakeetUltra) else {
        return;
    };
    struct Case {
        voice: &'static str,
        language: &'static str,
        text: &'static str,
        want: &'static str,
        script: fn(char) -> bool,
    }
    let latin = |c: char| !matches!(c, '\u{400}'..='\u{4FF}');
    let cyrillic = |c: char| !c.is_ascii_alphabetic();
    let cases = [
        Case {
            voice: "Zosia",
            language: "pl",
            text: "Troy i Abed uczą się hiszpańskiego w college'u Greendale.",
            want: "hiszpańskiego",
            script: latin,
        },
        Case {
            voice: "Milena",
            language: "ru",
            text: "Трой и Абед изучают испанский язык в колледже Гриндейл.",
            want: "испанский",
            script: cyrillic,
        },
        Case {
            voice: "Anna",
            language: "de",
            text: "Troy und Abed studieren Spanisch am Greendale Community College.",
            want: "spanisch",
            script: latin,
        },
    ];
    for tc in cases {
        let samples = speak(tc.voice, tc.text);
        let text = engine
            .transcribe(&samples, language(tc.language))
            .expect("transcribe");
        assert!(
            text.to_lowercase().contains(tc.want),
            "{}: {text}",
            tc.language
        );
        assert!(text.chars().all(tc.script), "{}: {text}", tc.language);

        let mut long = samples.clone();
        while long.len() <= 300_000 {
            long.resize(long.len() + 16_000, 0.0);
            long.extend_from_slice(&samples);
        }
        let text = engine
            .preview(&long, language(tc.language))
            .expect("preview");
        let repeats = text.to_lowercase().matches(tc.want).count();
        assert!(repeats >= 3, "{}: {text}", tc.language);
    }
}

#[test]
#[ignore = "needs ONDA_TEST_MODEL_DIR and the downloaded model"]
fn japanese_transcribes_and_merges_windows() {
    let Some(mut engine) = loaded(ModelId::ParakeetJapanese) else {
        return;
    };
    let samples = speak(
        "Kyoko",
        "トロイはグリーンデールでスペイン語を勉強しています。",
    );
    // the language is ignored, so passing it changes nothing
    let text = engine
        .transcribe(&samples, language("ja"))
        .expect("transcribe");
    assert!(text.contains("スペイン語"), "{text}");
    assert_eq!(engine.transcribe(&samples, None), Ok(text));

    let mut long = samples.clone();
    while long.len() <= 300_000 {
        long.resize(long.len() + 16_000, 0.0);
        long.extend_from_slice(&samples);
    }
    let text = engine.transcribe(&long, None).expect("transcribe");
    assert!(text.matches("スペイン語").count() >= 3, "{text}");
}

/// ASR-17, ASR-20: previews stream only new audio, the final decode
/// flushes the tail of the same stream and matches an offline decode, and
/// a shorter recording restarts the stream.
#[test]
#[ignore = "needs ONDA_TEST_MODEL_DIR and the downloaded model"]
fn unified_streams_previews_into_the_final_transcript() {
    let Some(mut engine) = loaded(ModelId::ParakeetUnified) else {
        return;
    };
    let speech = speak("Samantha", GREENDALE);
    let offline = engine.transcribe(&speech, None).expect("transcribe");
    assert!(
        offline.to_lowercase().contains("community college"),
        "{offline}"
    );

    let mut previews = Vec::new();
    for len in (8_000..speech.len()).step_by(4_800) {
        previews.push(engine.preview(&speech[..len], None).expect("preview"));
    }
    assert!(previews.iter().any(|p| !p.is_empty()), "{previews:?}");
    let last = previews.last().cloned().unwrap_or_default();
    assert!(offline.starts_with(&last), "{last} / {offline}");
    assert_eq!(engine.transcribe(&speech, None), Ok(offline.clone()));

    // a new, shorter recording restarts instead of continuing
    let half = &speech[..speech.len() / 2];
    engine.preview(&speech, None).expect("preview");
    let restarted = engine.preview(half, None).expect("preview");
    assert!(offline.starts_with(&restarted), "{restarted} / {offline}");

    // after a reset, the next call starts a new stream too
    engine.reset_preview();
    assert_eq!(engine.transcribe(&speech, None), Ok(offline));
}
