//! What the updater remembers across launches, in `updater.json` under its
//! state folder: the automatic-check preference, the last finished check,
//! and the highest build any verified feed has named (UPD-5's replay
//! guard).

use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Prefs {
    pub(crate) automatic_checks: bool,
    /// Seconds since the Unix epoch.
    #[serde(default)]
    pub(crate) last_check: Option<u64>,
    #[serde(default)]
    pub(crate) highest_build_seen: u64,
}

impl Prefs {
    /// Reads `file`; a missing or unreadable file starts over with
    /// automatic checks set to `automatic`.
    pub(crate) fn load(file: &Path, automatic: bool) -> Self {
        let fresh = Self {
            automatic_checks: automatic,
            last_check: None,
            highest_build_seen: 0,
        };
        match std::fs::read(file) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "unreadable updater state; starting over");
                fresh
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fresh,
            Err(error) => {
                tracing::warn!(%error, "could not read the updater state");
                fresh
            }
        }
    }

    /// Acquires an exclusive cross-process lock on `<file>.lock`, reloads
    /// `file` from disk, merges monotonic fields (`highest_build_seen`,
    /// `last_check`) with `in_memory`, applies `change`, and persists the
    /// result atomically.
    pub(crate) fn update_locked<E>(
        file: &Path,
        in_memory: &Self,
        change: impl FnOnce(&mut Self) -> Result<(), E>,
    ) -> Result<Self, E> {
        let _lock = lock_prefs(file);
        let mut merged = if file.exists() {
            let mut on_disk = Self::load(file, in_memory.automatic_checks);
            on_disk.highest_build_seen =
                on_disk.highest_build_seen.max(in_memory.highest_build_seen);
            on_disk.last_check = match (on_disk.last_check, in_memory.last_check) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            on_disk
        } else {
            in_memory.clone()
        };
        change(&mut merged)?;
        if let Err(error) = merged.save(file) {
            tracing::error!(%error, "could not save the updater state");
        }
        Ok(merged)
    }

    /// Writes `file` atomically.
    pub(crate) fn save(&self, file: &Path) -> std::io::Result<()> {
        let parent = file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".updater-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        // serializing this plain struct cannot fail
        temporary.write_all(&serde_json::to_vec_pretty(self).unwrap_or_default())?;
        temporary.as_file().sync_all()?;
        temporary.persist(file).map_err(|error| error.error)?;
        Ok(())
    }

    /// The last check, or `None` when there was none or the saved value
    /// is not a time this clock can represent.
    pub(crate) fn last_check_time(&self) -> Option<SystemTime> {
        self.last_check
            .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
    }

    pub(crate) fn set_last_check(&mut self, time: SystemTime) {
        self.last_check = time
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|since| since.as_secs());
    }
}

fn lock_prefs(file: &Path) -> Option<std::fs::File> {
    let parent = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if let Err(error) = std::fs::create_dir_all(parent) {
        tracing::warn!(%error, "could not create updater state directory for lock");
        return None;
    }
    let mut lock_name = file.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_path = std::path::PathBuf::from(lock_name);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let lock_file = match options.open(&lock_path) {
        Ok(lock_file) => lock_file,
        Err(error) => {
            tracing::warn!(%error, path = %lock_path.display(), "could not open updater prefs lock");
            return None;
        }
    };
    if let Err(error) = lock_file.lock() {
        tracing::warn!(%error, path = %lock_path.display(), "could not acquire updater prefs lock");
        return None;
    }
    Some(lock_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_survive_damage() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("updates/updater.json");
        assert!(Prefs::load(&file, true).automatic_checks);
        assert!(!Prefs::load(&file, false).automatic_checks);

        let mut prefs = Prefs::load(&file, true);
        prefs.automatic_checks = false;
        prefs.highest_build_seen = 42;
        prefs.set_last_check(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        prefs.save(&file).unwrap();
        assert_eq!(Prefs::load(&file, true), prefs);

        std::fs::write(&file, b"Pierce").unwrap();
        assert_eq!(Prefs::load(&file, true).highest_build_seen, 0);

        std::fs::write(
            &file,
            format!(r#"{{"automatic_checks":true,"last_check":{}}}"#, u64::MAX),
        )
        .unwrap();
        assert_eq!(Prefs::load(&file, true).last_check_time(), None);
    }
}
