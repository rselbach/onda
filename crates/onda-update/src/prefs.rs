//! What the updater remembers across launches, in `updater.json` under its
//! state folder: the automatic-check preference, the last finished check,
//! and the highest build any verified feed has named (UPD-5's replay
//! guard).

use std::path::{Path, PathBuf};
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

    /// Writes `file` atomically.
    pub(crate) fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = PathBuf::from(format!("{}.tmp", file.display()));
        // serializing this plain struct cannot fail
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).unwrap_or_default(),
        )?;
        std::fs::rename(&temporary, file)
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
