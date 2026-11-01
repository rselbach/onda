use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use jiff::Timestamp;
use onda_domain::history::{
    FolderVerdict, HistoryEntry, METADATA_FILENAME, MetadataFile, NewEntry, PathComponent,
    classify_entry_folder, encode_metadata, encode_wav, insert_newest_first, plan_retention,
    sort_newest_first,
};
use onda_domain::types::HistoryRetention;
use uuid::Uuid;

use crate::error::{HistoryError, io_error};
use crate::fs::{remove_descendant, resolve_descendant, write_atomic};

/// History settings as read at save time (HIS-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryPreferences {
    /// "Save transcripts": when off, nothing new is saved.
    pub enabled: bool,
    /// "Keep audio recordings": when on, new entries keep `audio.wav`.
    pub keep_audio: bool,
    /// "Keep for": the retention age applied after the save.
    pub retention: HistoryRetention,
}

/// What the app supplies to the store.
pub trait HistoryHooks: Send + Sync {
    /// The in-memory list changed: save, delete, delete all, or retention
    /// (HIS-10). Called with no lock held, so it may read the store.
    fn history_changed(&self);

    /// Marks `directory` as excluded from backups (HIS-6). Called when the
    /// store first prepares the history folder for writing, and again after
    /// "Clear History" removed it (HIS-16a). Called with the store locked, so
    /// it must not call back into the store. A failure is logged and does not
    /// stop the save.
    fn exclude_from_backup(&self, directory: &Path) -> std::io::Result<()>;
}

/// The history folder and its newest-first entry list (HIS-5..HIS-16a).
///
/// The list is loaded from disk on first use, pruning bad entry folders
/// (HIS-8), then kept in step with every change, so saving never rescans the
/// folder (HIS-9). All methods are synchronous and serialized by one lock.
pub struct HistoryStore {
    directory: PathBuf,
    hooks: Box<dyn HistoryHooks>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    // `None` until the first load.
    entries: Option<Vec<HistoryEntry>>,
    // the folder exists and backup exclusion was requested.
    directory_prepared: bool,
    // the list changed during the current operation; reported once the lock
    // is released.
    changed: bool,
}

impl HistoryStore {
    /// A store over `directory` (normally `<data>/History`). Nothing is read
    /// until the first call that needs the list.
    pub fn new(directory: impl Into<PathBuf>, hooks: impl HistoryHooks + 'static) -> Self {
        Self {
            directory: directory.into(),
            hooks: Box::new(hooks),
            state: Mutex::new(State::default()),
        }
    }

    /// The history folder.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Entries, newest first (HIS-9). Loads and prunes the folder on first
    /// call (HIS-8).
    pub fn entries(&self) -> Result<Vec<HistoryEntry>, HistoryError> {
        self.with_state(|state| {
            self.load_once(&mut state.entries)
                .map(|entries| entries.clone())
        })
    }

    /// Saves a finished session when history is on (HIS-1, HIS-2), then
    /// applies retention with the new entry exempt (HIS-12..HIS-15).
    ///
    /// `preferences` must be read from settings right before the call. When
    /// audio is kept, `samples` are written as `audio.wav` and
    /// `new.audio_byte_count` is set from the encoded file; otherwise it is
    /// cleared. Returns the saved entry, or `None` when history is off. A
    /// write failure removes the partial folder (HIS-7); a retention failure
    /// after a successful write is logged and does not fail the save. Every
    /// failure is logged.
    pub fn save(
        &self,
        new: NewEntry,
        samples: &[f32],
        preferences: HistoryPreferences,
        now: Timestamp,
    ) -> Result<Option<HistoryEntry>, HistoryError> {
        if !preferences.enabled {
            return Ok(None);
        }
        let result =
            self.with_state(|state| self.save_locked(state, new, samples, preferences, now));
        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(err) => {
                tracing::error!(error = %err, "could not save history entry");
                Err(err)
            }
        }
    }

    /// Applies retention to the whole list (HIS-16): at launch (which also
    /// preloads the list), when the History page appears, and when the
    /// retention setting changes. `Ok(true)` when it deleted entries or
    /// removed recordings (HIS-10).
    pub fn apply_retention(
        &self,
        retention: HistoryRetention,
        now: Timestamp,
    ) -> Result<bool, HistoryError> {
        self.with_state(|state| self.enforce_retention(state, retention, now, None))
    }

    /// Deletes one entry's folder. An unknown id is not an error.
    pub fn delete(&self, id: Uuid) -> Result<(), HistoryError> {
        self.with_state(|state| {
            let State {
                entries, changed, ..
            } = state;
            let entries = self.load_once(entries)?;
            let Some(index) = entries.iter().position(|entry| entry.id == id) else {
                return Ok(());
            };
            self.remove_entry_folder(&entries[index])?;
            entries.remove(index);
            *changed = true;
            tracing::info!(%id, "deleted history entry");
            Ok(())
        })
    }

    /// "Clear History": removes the whole history folder (HIS-16a). The next
    /// save recreates it and requests backup exclusion again. A symlinked
    /// history folder is refused rather than unlinked, since unlinking would
    /// leave the transcripts on disk (ARCH-12).
    pub fn delete_all(&self) -> Result<(), HistoryError> {
        self.with_state(|state| {
            let removal = match std::fs::symlink_metadata(&self.directory) {
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
                Err(err) => return Err(io_error("read", &self.directory, &err)),
                Ok(meta) if meta.file_type().is_symlink() => {
                    tracing::error!(path = %self.directory.display(), "refusing to clear a symlinked history folder");
                    return Err(HistoryError::UnsafePath(self.directory.clone()));
                }
                Ok(_) => std::fs::remove_dir_all(&self.directory),
            };
            state.directory_prepared = false;
            state.changed = true;
            if let Err(err) = removal {
                // part of the folder may be gone; rescan on next use.
                state.entries = None;
                return Err(io_error("delete", &self.directory, &err));
            }
            state.entries = Some(Vec::new());
            tracing::info!("cleared history");
            Ok(())
        })
    }

    /// The recording of entry `id`, for "Play Recording" and "Show Recording
    /// in Finder" (HIS-24). Fails with [`HistoryError::MissingRecording`]
    /// when the entry is unknown, has no audio, or its file is gone, and
    /// with [`HistoryError::UnsafePath`] when the file resolves outside the
    /// history folder or is a symlink.
    pub fn audio_path(&self, id: Uuid) -> Result<PathBuf, HistoryError> {
        self.with_state(|state| {
            let entries = self.load_once(&mut state.entries)?;
            let audio = entries
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| self.audio_file(entry))
                .ok_or(HistoryError::MissingRecording)?;
            match std::fs::symlink_metadata(&audio) {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    return Err(HistoryError::MissingRecording);
                }
                Err(err) => return Err(io_error("read", &audio, &err)),
            }
            let (resolved, _) = resolve_descendant(&self.directory, &audio)?;
            Ok(resolved)
        })
    }

    // runs `op` under the lock, then reports a change with the lock released.
    fn with_state<T>(&self, op: impl FnOnce(&mut State) -> T) -> T {
        let (result, changed) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let result = op(&mut state);
            (result, std::mem::take(&mut state.changed))
        };
        if changed {
            self.hooks.history_changed();
        }
        result
    }

    fn save_locked(
        &self,
        state: &mut State,
        mut new: NewEntry,
        samples: &[f32],
        preferences: HistoryPreferences,
        now: Timestamp,
    ) -> Result<HistoryEntry, HistoryError> {
        self.load_once(&mut state.entries)?;
        self.prepare_directory(state)?;

        let wav = match preferences.keep_audio {
            true => Some(encode_wav(samples).map_err(|_| HistoryError::AudioTooLarge)?),
            false => None,
        };
        new.audio_byte_count = wav.as_ref().map(|wav| wav.len() as u64);
        let entry = new.into_entry();
        let folder = self.entry_folder(&entry);
        self.write_entry_folder(&folder, |folder| {
            if let (Some(wav), Some(audio)) = (&wav, &entry.audio) {
                write_atomic(&folder.join(audio.filename.as_str()), wav)?;
            }
            write_metadata(folder, &entry)
        })?;

        insert_newest_first(state.entries.get_or_insert_default(), entry.clone());
        state.changed = true;
        tracing::info!(
            id = %entry.id,
            audio_bytes = entry.audio_byte_count(),
            "saved history entry"
        );

        if let Err(err) = self.enforce_retention(state, preferences.retention, now, Some(entry.id))
        {
            tracing::error!(error = %err, "could not apply history retention after save");
        }
        Ok(entry)
    }

    // creates the folder (failing if it already exists, so a folder this
    // store did not create is never removed), runs `write`, and removes the
    // folder again when `write` fails (HIS-7).
    pub(crate) fn write_entry_folder(
        &self,
        folder: &Path,
        write: impl FnOnce(&Path) -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        std::fs::create_dir(folder).map_err(|err| io_error("create", folder, &err))?;
        let Err(err) = write(folder) else {
            return Ok(());
        };
        tracing::error!(path = %folder.display(), error = %err, "could not write history entry");
        if let Err(cleanup) = remove_descendant(&self.directory, folder) {
            tracing::error!(
                path = %folder.display(),
                error = %cleanup,
                "could not remove partial history entry"
            );
        }
        Err(err)
    }

    fn prepare_directory(&self, state: &mut State) -> Result<(), HistoryError> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|err| io_error("create", &self.directory, &err))?;
        if state.directory_prepared {
            return Ok(());
        }
        if let Err(err) = self.hooks.exclude_from_backup(&self.directory) {
            tracing::error!(error = %err, "could not exclude history from backups");
        }
        state.directory_prepared = true;
        Ok(())
    }

    // executes the plan in its documented order, keeping the list in step
    // with each step so a failure part way leaves it matching the disk.
    fn enforce_retention(
        &self,
        state: &mut State,
        retention: HistoryRetention,
        now: Timestamp,
        exempt: Option<Uuid>,
    ) -> Result<bool, HistoryError> {
        let State {
            entries, changed, ..
        } = state;
        let entries = self.load_once(entries)?;
        let plan = plan_retention(entries, now, retention, exempt);
        if plan.is_noop() {
            return Ok(false);
        }
        for doomed in &plan.delete {
            self.remove_entry_folder(doomed)?;
            entries.retain(|entry| entry.id != doomed.id);
            *changed = true;
        }
        for stripped in &plan.strip_audio {
            if let Some(audio) = self.audio_file(stripped) {
                remove_descendant(&self.directory, &audio)?;
            }
            let updated = stripped.without_audio();
            if let Some(slot) = entries.iter_mut().find(|entry| entry.id == stripped.id) {
                *slot = updated.clone();
            }
            *changed = true;
            write_metadata(&self.entry_folder(&updated), &updated)?;
        }
        tracing::info!(
            deleted = plan.delete.len(),
            audio_removed = plan.strip_audio.len(),
            "applied history retention"
        );
        Ok(true)
    }

    fn load_once<'a>(
        &self,
        slot: &'a mut Option<Vec<HistoryEntry>>,
    ) -> Result<&'a mut Vec<HistoryEntry>, HistoryError> {
        let entries = match slot.take() {
            Some(entries) => entries,
            None => self.load()?,
        };
        Ok(slot.insert(entries))
    }

    // scans the folder once (HIS-8). plain files, symlinks, and hidden
    // entries are not entry folders and are left alone.
    fn load(&self) -> Result<Vec<HistoryEntry>, HistoryError> {
        let listing = match std::fs::read_dir(&self.directory) {
            Ok(listing) => listing,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_error("read", &self.directory, &err)),
        };
        let mut entries = Vec::new();
        for item in listing {
            let item = item.map_err(|err| io_error("read", &self.directory, &err))?;
            let path = item.path();
            let is_dir = match item.file_type() {
                Ok(file_type) => file_type.is_dir(),
                Err(err) => {
                    tracing::warn!(path = %path.display(), error = %err, "skipping unreadable history item");
                    continue;
                }
            };
            let file_name = item.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if !is_dir || name.starts_with('.') {
                continue;
            }
            let Some(name) = PathComponent::new(name) else {
                continue;
            };
            let contents = std::fs::read(path.join(METADATA_FILENAME));
            let metadata = match &contents {
                Ok(bytes) => MetadataFile::Contents(bytes),
                Err(err) if err.kind() == ErrorKind::NotFound => MetadataFile::Missing,
                Err(_) => MetadataFile::Unreadable,
            };
            match classify_entry_folder(metadata) {
                // the folder the metadata came from is the one to delete
                // later, whatever its `directoryName` says.
                FolderVerdict::Keep(entry) => entries.push(HistoryEntry {
                    directory_name: name,
                    ..*entry
                }),
                FolderVerdict::Prune(reason) => {
                    tracing::warn!(path = %path.display(), ?reason, "removing unreadable history entry");
                    if let Err(err) = remove_descendant(&self.directory, &path) {
                        tracing::error!(path = %path.display(), error = %err, "could not remove history entry");
                    }
                }
            }
        }
        sort_newest_first(&mut entries);
        tracing::info!(count = entries.len(), "loaded history");
        Ok(entries)
    }

    fn entry_folder(&self, entry: &HistoryEntry) -> PathBuf {
        self.directory.join(entry.directory_name.as_str())
    }

    fn audio_file(&self, entry: &HistoryEntry) -> Option<PathBuf> {
        let audio = entry.audio.as_ref()?;
        Some(self.entry_folder(entry).join(audio.filename.as_str()))
    }

    fn remove_entry_folder(&self, entry: &HistoryEntry) -> Result<(), HistoryError> {
        remove_descendant(&self.directory, &self.entry_folder(entry))
    }
}

fn write_metadata(folder: &Path, entry: &HistoryEntry) -> Result<(), HistoryError> {
    let path = folder.join(METADATA_FILENAME);
    let bytes = encode_metadata(entry)
        .map_err(|err| HistoryError::Io(format!("Could not encode {}: {err}", path.display())))?;
    write_atomic(&path, &bytes)
}
