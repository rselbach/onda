use std::path::Path;

use crate::error::{Error, describe};

/// Removes `target` and everything under it, but only when it is a strict
/// descendant of `root` (ARCH-12). Parent directories are canonicalized
/// before the check, and a symlink at `target` itself is refused rather than
/// followed. `remove_dir_all` never follows symlinks inside the tree.
pub(crate) fn remove_descendant(root: &Path, target: &Path) -> Result<(), Error> {
    let unsafe_path = || {
        tracing::error!(path = %target.display(), "refusing to delete outside the models directory");
        Error::UnsafePath(target.to_path_buf())
    };
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(unsafe_path());
    };
    let root = canonicalize(root)?;
    let resolved = canonicalize(parent)?.join(name);
    if resolved == root || !resolved.starts_with(&root) {
        return Err(unsafe_path());
    }
    let meta = std::fs::symlink_metadata(&resolved).map_err(|e| io_error("read", target, &e))?;
    if meta.file_type().is_symlink() {
        return Err(unsafe_path());
    }
    let result = match meta.is_dir() {
        true => std::fs::remove_dir_all(&resolved),
        false => std::fs::remove_file(&resolved),
    };
    result.map_err(|e| io_error("delete", target, &e))
}

fn canonicalize(path: &Path) -> Result<std::path::PathBuf, Error> {
    path.canonicalize()
        .map_err(|e| io_error("resolve", path, &e))
}

fn io_error(action: &str, path: &Path, error: &std::io::Error) -> Error {
    Error::Io(format!(
        "Could not {action} {}: {}",
        path.display(),
        describe(error)
    ))
}
