//! Guarded deletes (ARCH-12) and atomic writes (HIS-7).

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::error::{HistoryError, io_error};

/// Removes `target` (a folder and everything under it, or a file) when it is
/// a strict descendant of `root` (ARCH-12). A missing target is not an error.
/// The parent is canonicalized before the check and a symlink at `target` is
/// refused, so no symlink leads the delete out of `root`; `remove_dir_all`
/// never follows symlinks inside the tree.
pub(crate) fn remove_descendant(root: &Path, target: &Path) -> Result<(), HistoryError> {
    match std::fs::symlink_metadata(target) {
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(io_error("read", target, &err)),
    }
    let (resolved, is_dir) = resolve_descendant(root, target)?;
    let result = match is_dir {
        true => std::fs::remove_dir_all(&resolved),
        false => std::fs::remove_file(&resolved),
    };
    result.map_err(|err| io_error("delete", target, &err))
}

/// The canonical path of an existing `target` and whether it is a folder,
/// when `target` is a strict descendant of `root` and not itself a symlink
/// (ARCH-12).
pub(crate) fn resolve_descendant(
    root: &Path,
    target: &Path,
) -> Result<(PathBuf, bool), HistoryError> {
    let unsafe_path = || {
        tracing::error!(path = %target.display(), "refusing a path outside the history folder");
        HistoryError::UnsafePath(target.to_path_buf())
    };
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(unsafe_path());
    };
    let root = canonicalize(root)?;
    let resolved = canonicalize(parent)?.join(name);
    if resolved == root || !resolved.starts_with(&root) {
        return Err(unsafe_path());
    }
    let meta =
        std::fs::symlink_metadata(&resolved).map_err(|err| io_error("read", target, &err))?;
    if meta.file_type().is_symlink() {
        return Err(unsafe_path());
    }
    Ok((resolved, meta.is_dir()))
}

/// Writes `bytes` to a hidden temporary file next to `path`, then renames it
/// over `path`, so readers see the old file or the new one, never a partial
/// one (HIS-7).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), HistoryError> {
    let folder = path.parent().unwrap_or(Path::new("."));
    let mut file =
        tempfile::NamedTempFile::new_in(folder).map_err(|err| io_error("write", path, &err))?;
    file.write_all(bytes)
        .map_err(|err| io_error("write", path, &err))?;
    file.persist(path)
        .map_err(|err| io_error("write", path, &err.error))?;
    Ok(())
}

fn canonicalize(path: &Path) -> Result<PathBuf, HistoryError> {
    path.canonicalize()
        .map_err(|err| io_error("resolve", path, &err))
}
