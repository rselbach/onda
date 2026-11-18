//! Model integrity checks at the `transcribe` command boundary.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

use onda_asr_coreml::manifest::manifest;
use onda_domain::models::ModelId;
use onda_models::{ModelManager, VERIFIED_MARKER};

const MODEL: ModelId = ModelId::ParakeetTdtV2;

fn assert_rejected_before_coreml(scratch: &Path, models: &Path, detail: &str) {
    let wav = scratch.join("greendale.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
    for _ in 0..spec.sample_rate {
        writer.write_sample(0_i16).unwrap();
    }
    writer.finalize().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_onda-cli"))
        .arg("--data-dir")
        .arg(scratch)
        .arg("--models-dir")
        .arg(models)
        .args(["transcribe", "--model", MODEL.as_str()])
        .arg(wav)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("error: Model '{MODEL}' is incomplete: {detail}")),
        "{stderr}"
    );
    assert!(!stderr.contains("Compiling "), "{stderr}");
}

#[test]
fn missing_model_is_rejected_before_coreml() {
    let scratch = tempfile::tempdir().unwrap();
    assert_rejected_before_coreml(
        scratch.path(),
        &scratch.path().join("models"),
        "folder does not exist",
    );
}

#[test]
fn corrupt_model_with_matching_marker_is_rejected_before_coreml() {
    let scratch = tempfile::tempdir().unwrap();
    let models = scratch.path().join("models");
    let model_dir = models.join(MODEL.as_str());
    // Simulate a marker left after corruption without installing real weights.
    let mut marker = String::from("onda-verified 1\n");
    for file in manifest(MODEL).unwrap().files {
        let path = model_dir.join(file.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let fixture = File::create(path).unwrap();
        fixture.set_len(file.size).unwrap();
        let modified = fixture
            .metadata()
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let hash = file.sha256.unwrap_or("-");
        writeln!(marker, "{hash} {} {modified} {}", file.size, file.path).unwrap();
    }
    let marker_path = model_dir.join(VERIFIED_MARKER);
    fs::write(&marker_path, marker).unwrap();
    let (manager, _events) = ModelManager::new(&models, manifest).unwrap();
    manager
        .verify(MODEL, &mut |_| {
            panic!("fixture marker should bypass hashing")
        })
        .unwrap();

    assert_rejected_before_coreml(scratch.path(), &models, "corrupt files:");
    assert!(!marker_path.exists());
}
