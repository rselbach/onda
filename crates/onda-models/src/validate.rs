use std::io;
use std::path::Path;

use onda_asr::ModelManifest;
use onda_domain::models::FolderValidation;

/// Checks a model folder against its manifest (DL-5): every file must exist
/// with exactly the manifest size. Only metadata is read, so this is cheap
/// enough for launch; hashes are checked when a file is downloaded and
/// before a load ([`ModelManager::verify`](crate::ModelManager::verify)).
///
/// Files that are absent or have the wrong size (a zero-byte placeholder, a
/// half-written file) are listed as missing. Weight files (`weight.bin`
/// inside a compiled model) that fail the check also clear `has_weights`.
pub fn validate_folder(folder: &Path, manifest: &ModelManifest) -> FolderValidation {
    let all_files = || manifest.files.iter().map(|f| f.path.to_string()).collect();
    let has_weights = !manifest.files.iter().any(|f| is_weight(f.path));
    match std::fs::metadata(folder) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return missing_folder(all_files(), has_weights),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return missing_folder(all_files(), has_weights);
        }
        Err(error) => {
            return FolderValidation {
                folder_exists: true,
                read_error: Some(error.to_string()),
                missing_files: all_files(),
                has_weights,
            };
        }
    }

    let mut validation = FolderValidation {
        folder_exists: true,
        read_error: None,
        missing_files: Vec::new(),
        has_weights: true,
    };
    for file in manifest.files {
        let complete = match std::fs::metadata(folder.join(file.path)) {
            Ok(meta) => meta.is_file() && meta.len() == file.size,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                tracing::warn!(path = file.path, %error, "could not read model file");
                false
            }
        };
        if complete {
            continue;
        }
        validation.missing_files.push(file.path.to_string());
        if is_weight(file.path) {
            validation.has_weights = false;
        }
    }
    validation
}

// every file now carries a hash, so weights are recognized by name: CoreML
// compiled models keep their weights in `weights/weight.bin`
fn is_weight(path: &str) -> bool {
    path.ends_with("weight.bin")
}

// a missing folder misses everything, weights included (when there are any)
fn missing_folder(missing_files: Vec<String>, no_weights_needed: bool) -> FolderValidation {
    FolderValidation {
        folder_exists: false,
        read_error: None,
        missing_files,
        has_weights: no_weights_needed,
    }
}
