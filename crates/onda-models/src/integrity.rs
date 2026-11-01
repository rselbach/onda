//! Content verification of a model folder before an engine reads it.
//!
//! CoreML crashes the process, rather than failing, on some corrupt model
//! files, and a corrupt file of the right size passes launch validation
//! (DL-5). So every file is hashed against its manifest SHA-256 before a
//! load. Hashing ~600 MB on every load is wasteful, so a successful full
//! check writes a marker into the model folder recording each file's size
//! and modification time; while those are unchanged, the folder is trusted.

use std::io::{self, Read};
use std::path::Path;
use std::time::{Instant, SystemTime};

use onda_asr::ModelManifest;
use sha2::{Digest, Sha256};

use crate::validate::validate_folder;

/// File name of the verification marker inside a model folder (ARCH-12: it
/// never lives outside the models directory).
pub const VERIFIED_MARKER: &str = ".onda-verified";

/// First line of the marker; bump it when the format changes.
const MARKER_HEADER: &str = "onda-verified 1";
/// Smallest progress step reported while hashing.
const PROGRESS_STEP: f64 = 0.01;

/// Checks every file of `folder` against `manifest` by size and SHA-256,
/// unless the marker shows the folder was verified and has not changed since.
/// Blocking: hashing a large model takes a while. `progress` receives the
/// hashed share of the bytes, in `[0, 1]`, only when the files are hashed.
///
/// Returns failure details for
/// [`ModelError::IncompleteInstallation`](onda_domain::models::ModelError::IncompleteInstallation)
/// when a file is missing, has the wrong size or hash, or cannot be read.
pub(crate) fn verify_folder(
    folder: &Path,
    manifest: &ModelManifest,
    progress: &mut dyn FnMut(f64),
) -> Result<(), String> {
    if let Some(details) = validate_folder(folder, manifest).failure_details() {
        return Err(details);
    }
    let before =
        marker_text(folder, manifest).map_err(|e| format!("could not read folder: {e}"))?;
    if marker_is_current(folder, &before) {
        tracing::debug!("model folder unchanged since verification");
        return Ok(());
    }

    let start = Instant::now();
    let total: u64 = manifest
        .files
        .iter()
        .filter(|f| f.sha256.is_some())
        .map(|f| f.size)
        .sum();
    let mut done = 0u64;
    let mut reported = 0.0;
    progress(0.0);
    let mut corrupt = Vec::new();
    for file in manifest.files {
        let Some(want) = file.sha256 else {
            continue;
        };
        let mut on_read = |bytes: u64| {
            done += bytes;
            let fraction = match total {
                0 => 1.0,
                total => done as f64 / total as f64,
            };
            if fraction - reported >= PROGRESS_STEP {
                reported = fraction;
                progress(fraction);
            }
        };
        let got = sha256_file(&folder.join(file.path), &mut on_read)
            .map_err(|e| format!("could not read {}: {e}", file.path))?;
        if !got.eq_ignore_ascii_case(want) {
            tracing::error!(path = file.path, want, got, "model file checksum mismatch");
            corrupt.push(file.path);
        }
    }
    if reported < 1.0 {
        progress(1.0);
    }
    let elapsed_ms = start.elapsed().as_millis() as u64;
    if !corrupt.is_empty() {
        tracing::error!(
            elapsed_ms,
            corrupt = corrupt.len(),
            "model folder failed verification"
        );
        return Err(format!("corrupt files: {}", corrupt.join(", ")));
    }
    tracing::info!(elapsed_ms, bytes = total, "model folder verified");

    // a file rewritten while it was hashed must not be marked as verified
    match marker_text(folder, manifest) {
        Ok(after) if after == before => write_marker(folder, &after),
        Ok(_) => tracing::warn!("model files changed during verification; not marking verified"),
        Err(error) => tracing::warn!(%error, "could not read model folder after verification"),
    }
    Ok(())
}

/// Records `folder` as verified at its current file sizes and modification
/// times. Called by the downloader once every file was hashed. Failures are
/// logged: the next load then hashes the folder itself.
pub(crate) fn mark_verified(folder: &Path, manifest: &ModelManifest) {
    match marker_text(folder, manifest) {
        Ok(text) => write_marker(folder, &text),
        Err(error) => tracing::warn!(%error, "could not read model folder to mark it verified"),
    }
}

/// Hex SHA-256 of the file at `path`; `on_read` hears every chunk's length.
pub(crate) fn sha256_file(path: &Path, on_read: &mut dyn FnMut(u64)) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        on_read(read as u64);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

// the marker a verified folder in its current state would carry: one line
// per manifest file with its expected hash, current size, and modification
// time, so any change to a file or to the manifest invalidates it
fn marker_text(folder: &Path, manifest: &ModelManifest) -> io::Result<String> {
    let mut text = format!("{MARKER_HEADER}\n");
    for file in manifest.files {
        let meta = std::fs::metadata(folder.join(file.path))?;
        let modified = match meta.modified()?.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(since) => since.as_nanos().to_string(),
            Err(before) => format!("-{}", before.duration().as_nanos()),
        };
        let sha256 = file.sha256.unwrap_or("-");
        text.push_str(&format!(
            "{sha256} {} {modified} {}\n",
            meta.len(),
            file.path
        ));
    }
    Ok(text)
}

fn marker_is_current(folder: &Path, want: &str) -> bool {
    match std::fs::read_to_string(folder.join(VERIFIED_MARKER)) {
        Ok(text) => text == want,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(%error, "could not read the verification marker");
            false
        }
    }
}

// written to a temporary file and renamed, so a crash never leaves a
// truncated marker
fn write_marker(folder: &Path, text: &str) {
    let marker = folder.join(VERIFIED_MARKER);
    let temp = folder.join(format!("{VERIFIED_MARKER}.tmp"));
    let written = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, &marker));
    if let Err(error) = written {
        tracing::warn!(%error, "could not write the verification marker");
        if let Err(error) = std::fs::remove_file(&temp)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "could not delete the temporary verification marker");
        }
    }
}
