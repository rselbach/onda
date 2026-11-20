use std::io;
use std::path::{Component, Path, PathBuf};

use onda_asr::ModelManifest;
use onda_domain::models::FolderValidation;

/// Resolves a manifest-relative file path strictly inside `folder`, rejecting
/// empty paths, `.`, `..`, roots, and platform prefixes.
pub(crate) fn manifest_file_path(folder: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() {
        return None;
    }
    let mut out = folder.to_path_buf();
    let mut count = 0;
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                count += 1;
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return None;
            }
        }
    }
    (count > 0).then_some(out)
}

/// Checks that `folder` (if it exists) and every existing path component of
/// `rel` under `folder` are not symlinks. Returns `Ok(true)` if the final file
/// exists as a regular file, `Ok(false)` if any component is missing, or
/// `Err(io::Error)` if any component is a symlink or unreadable.
pub(crate) fn check_no_symlinks(folder: &Path, rel: &str) -> io::Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(folder) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "symlink inside models directory",
            ));
        }
        Ok(meta) if !meta.is_dir() => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut current = folder.to_path_buf();
    let components: Vec<_> = Path::new(rel).components().collect();
    for (idx, component) in components.iter().enumerate() {
        let Component::Normal(part) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe manifest path",
            ));
        };
        current.push(part);
        let is_last = idx + 1 == components.len();
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "symlink inside models directory",
                ));
            }
            Ok(meta) if is_last => return Ok(Some(meta)),
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

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
    match std::fs::symlink_metadata(folder) {
        Ok(meta) if !meta.file_type().is_symlink() && meta.is_dir() => {}
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
        let complete = match manifest_file_path(folder, file.path) {
            None => false,
            Some(_) => match check_no_symlinks(folder, file.path) {
                Ok(Some(meta)) => meta.file_type().is_file() && meta.len() == file.size,
                Ok(None) => false,
                Err(error) => {
                    tracing::warn!(path = file.path, %error, "could not read model file");
                    false
                }
            },
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
