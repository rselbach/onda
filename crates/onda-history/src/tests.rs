use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use onda_domain::history::{
    AUDIO_FILENAME, EntryAudio, HistoryEntry, MAX_AUDIO_BYTES, MAX_AUDIO_RECORDINGS, MAX_ENTRIES,
    METADATA_FILENAME, MISSING_RECORDING_ERROR, NewEntry, PathComponent, decode_metadata,
    encode_metadata, encode_wav, entry_directory_name,
};
use onda_domain::types::HistoryRetention;
use uuid::Uuid;

use crate::fs::remove_descendant;
use crate::{HistoryError, HistoryHooks, HistoryPreferences, HistoryStore};

const DAY: i64 = 24 * 60 * 60;
const NOW: i64 = 100 * DAY;

#[derive(Default)]
struct Recorder {
    changes: AtomicUsize,
    exclusions: Mutex<Vec<PathBuf>>,
}

struct Hooks(Arc<Recorder>);

impl HistoryHooks for Hooks {
    fn history_changed(&self) {
        self.0.changes.fetch_add(1, Ordering::SeqCst);
    }

    fn exclude_from_backup(&self, directory: &Path) -> std::io::Result<()> {
        self.0
            .exclusions
            .lock()
            .expect("exclusions lock")
            .push(directory.to_path_buf());
        Ok(())
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    history: PathBuf,
    outside: PathBuf,
    recorder: Arc<Recorder>,
    store: HistoryStore,
}

impl Fixture {
    fn new() -> Fixture {
        let temp = tempfile::tempdir().expect("temp dir");
        let history = temp.path().join("Onda").join("History");
        let outside = temp.path().join("Greendale");
        std::fs::create_dir_all(&outside).expect("outside dir");
        std::fs::write(outside.join("escape.wav"), b"keep me").expect("outside file");
        let recorder = Arc::new(Recorder::default());
        let store = HistoryStore::new(&history, Hooks(recorder.clone()));
        Fixture {
            _temp: temp,
            history,
            outside,
            recorder,
            store,
        }
    }

    // a second store over the same folder, as after a relaunch.
    fn reopen(&self) -> HistoryStore {
        HistoryStore::new(&self.history, Hooks(Arc::new(Recorder::default())))
    }

    fn changes(&self) -> usize {
        self.recorder.changes.load(Ordering::SeqCst)
    }

    fn exclusions(&self) -> Vec<PathBuf> {
        self.recorder.exclusions.lock().expect("lock").clone()
    }

    fn save(
        &self,
        n: u128,
        age: i64,
        keep_audio: bool,
        retention: HistoryRetention,
    ) -> HistoryEntry {
        self.store
            .save(
                new_entry(n, age),
                &SAMPLES,
                prefs(keep_audio, retention),
                at(NOW),
            )
            .expect("save")
            .expect("history on")
    }

    // writes an entry folder directly, as an earlier run would have.
    fn seed(&self, n: u128, age: i64, claimed_audio_bytes: Option<u64>) {
        let mut entry = new_entry(n, age).into_entry();
        entry.audio = claimed_audio_bytes.map(|byte_count| EntryAudio {
            filename: PathComponent::new(AUDIO_FILENAME).expect("safe"),
            byte_count,
        });
        let folder = self.history.join(entry.directory_name.as_str());
        std::fs::create_dir_all(&folder).expect("seed folder");
        std::fs::write(
            folder.join(METADATA_FILENAME),
            encode_metadata(&entry).expect("encode"),
        )
        .expect("seed metadata");
        if entry.audio.is_some() {
            std::fs::write(folder.join(AUDIO_FILENAME), b"RIFF").expect("seed audio");
        }
    }

    fn assert_outside_intact(&self) {
        let got = std::fs::read(self.outside.join("escape.wav")).expect("outside file survives");
        assert_eq!(got, b"keep me");
    }
}

const SAMPLES: [f32; 4] = [0.0, 0.5, -0.5, 1.0];

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn prefs(keep_audio: bool, retention: HistoryRetention) -> HistoryPreferences {
    HistoryPreferences {
        enabled: true,
        keep_audio,
        retention,
    }
}

fn timestamp(age: i64) -> Timestamp {
    at(NOW - age)
}

fn new_entry(n: u128, age: i64) -> NewEntry {
    NewEntry {
        id: Uuid::from_u128(n),
        timestamp: timestamp(age),
        sample_count: SAMPLES.len(),
        model_id: "parakeet-tdt-v2".into(),
        model_name: Some("Parakeet TDT v2".into()),
        language: "en".into(),
        transcript: format!("Troy and Abed in the morning #{n}"),
        raw_transcript: None,
        was_post_processed: false,
        post_processing_error_description: None,
        app_name: Some("Greendale Messenger".into()),
        app_bundle_id: Some("edu.greendale.messenger".into()),
        audio_byte_count: None,
    }
}

fn entry_folder(history: &Path, n: u128, age: i64) -> PathBuf {
    history.join(entry_directory_name(timestamp(age), Uuid::from_u128(n)).as_str())
}

fn ids(entries: &[HistoryEntry]) -> Vec<u128> {
    entries.iter().map(|entry| entry.id.as_u128()).collect()
}

// metadata bytes with `key` set to `value`.
fn replace_key(bytes: &[u8], key: &str, value: &str) -> Vec<u8> {
    let mut json: serde_json::Value = serde_json::from_slice(bytes).expect("json");
    json[key] = value.into();
    serde_json::to_vec(&json).expect("json")
}

fn read_entry(folder: &Path) -> HistoryEntry {
    let bytes = std::fs::read(folder.join(METADATA_FILENAME)).expect("metadata");
    decode_metadata(&bytes).expect("decodes")
}

#[test]
fn retention_retries_a_failed_metadata_write() {
    let f = Fixture::new();
    f.seed(1, 60, Some(1));
    f.seed(2, 120, Some(MAX_AUDIO_BYTES));
    let entries = f.store.entries().expect("load the cache");
    let folder = entry_folder(&f.history, 2, 120);
    let metadata = folder.join(METADATA_FILENAME);
    let bytes = std::fs::read(&metadata).expect("read metadata");
    std::fs::remove_file(&metadata).expect("remove metadata");
    std::fs::create_dir(&metadata).expect("block the metadata write");
    assert!(
        f.store
            .apply_retention(HistoryRetention::Month, at(NOW))
            .is_err()
    );
    assert!(
        !folder.join(AUDIO_FILENAME).exists(),
        "audio was removed first"
    );
    assert_eq!(f.store.entries().expect("cached entries"), entries);

    std::fs::remove_dir(&metadata).expect("unblock the metadata write");
    std::fs::write(&metadata, bytes).expect("restore the old metadata");
    assert!(
        f.store
            .apply_retention(HistoryRetention::Month, at(NOW))
            .expect("retry")
    );
    assert!(!read_entry(&folder).has_audio());
    assert!(!f.reopen().entries().expect("reloaded entries")[1].has_audio());
    assert!(
        !f.store
            .apply_retention(HistoryRetention::Month, at(NOW))
            .expect("no more work")
    );
}

#[test]
fn store_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<HistoryStore>();
}

#[test]
fn save_and_reload_round_trip_his_5_his_9() {
    let f = Fixture::new();
    let older = f.save(1, 2 * DAY, false, HistoryRetention::Month);
    let newer = f.save(2, DAY, false, HistoryRetention::Month);
    let oldest = f.save(3, 3 * DAY, false, HistoryRetention::Month);

    let got = f.store.entries().expect("entries");
    assert_eq!(ids(&got), [2, 1, 3], "newest first, without rescanning");
    assert_eq!(got, vec![newer.clone(), older.clone(), oldest.clone()]);
    assert_eq!(f.changes(), 3, "one change per save");
    assert_eq!(
        f.exclusions(),
        std::slice::from_ref(&f.history),
        "backup exclusion requested once"
    );

    let folder = entry_folder(&f.history, 2, DAY);
    assert_eq!(
        std::fs::read(folder.join(METADATA_FILENAME)).expect("metadata"),
        encode_metadata(&newer).expect("encode"),
        "metadata.json is the domain encoding"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&folder)
        .expect("folder")
        .map(|item| item.expect("item").file_name())
        .collect();
    assert_eq!(leftovers, [METADATA_FILENAME], "no temp files left");

    let reloaded = f.reopen().entries().expect("reload");
    assert_eq!(reloaded, vec![newer, older, oldest], "reload matches");
}

#[test]
fn audio_kept_only_when_preference_on_his_1_his_5() {
    struct Case {
        name: &'static str,
        keep_audio: bool,
        want_audio: Option<Vec<u8>>,
    }
    let cases = [
        Case {
            name: "keep audio",
            keep_audio: true,
            want_audio: Some(encode_wav(&SAMPLES).expect("wav")),
        },
        Case {
            name: "transcript only",
            keep_audio: false,
            want_audio: None,
        },
    ];
    for tc in cases {
        let f = Fixture::new();
        let mut new = new_entry(1, DAY);
        new.audio_byte_count = Some(999_999);
        let saved = f
            .store
            .save(
                new,
                &SAMPLES,
                prefs(tc.keep_audio, HistoryRetention::Month),
                at(NOW),
            )
            .expect("save")
            .expect("history on");
        let folder = entry_folder(&f.history, 1, DAY);
        let got_audio = std::fs::read(folder.join(AUDIO_FILENAME)).ok();
        assert_eq!(got_audio, tc.want_audio, "{}: file", tc.name);
        let want_bytes = tc.want_audio.as_ref().map_or(0, |wav| wav.len() as u64);
        assert_eq!(
            saved.audio_byte_count(),
            want_bytes,
            "{}: byte count",
            tc.name
        );
        assert_eq!(read_entry(&folder), saved, "{}: metadata", tc.name);
    }
}

#[test]
fn history_off_saves_nothing_his_1() {
    let f = Fixture::new();
    let got = f
        .store
        .save(
            new_entry(1, DAY),
            &SAMPLES,
            HistoryPreferences {
                enabled: false,
                keep_audio: true,
                retention: HistoryRetention::Month,
            },
            at(NOW),
        )
        .expect("save");
    assert_eq!(got, None);
    assert!(!f.history.exists(), "folder not created");
    assert_eq!(f.changes(), 0);
}

#[test]
fn failed_write_removes_partial_folder_his_7() {
    let f = Fixture::new();
    std::fs::create_dir_all(&f.history).expect("history");
    let folder = f.history.join("2025-01-01T00-00-00.000Z-TROY");
    let got = f.store.write_entry_folder(&folder, |folder| {
        std::fs::write(folder.join(AUDIO_FILENAME), b"RIFF").expect("partial audio");
        Err(HistoryError::Io("No space left on device".into()))
    });
    assert_eq!(got, Err(HistoryError::Io("No space left on device".into())));
    assert!(!folder.exists(), "partial folder removed");
    assert!(f.history.exists(), "history folder kept");
}

#[test]
fn save_never_removes_a_folder_it_did_not_create_his_7() {
    let f = Fixture::new();
    // loaded first, so the load-time prune does not see the folder
    f.store.entries().expect("loaded");
    let folder = entry_folder(&f.history, 1, DAY);
    std::fs::create_dir_all(&folder).expect("existing folder");
    std::fs::write(folder.join("pierce.txt"), b"streets ahead").expect("existing file");

    let got = f.store.save(
        new_entry(1, DAY),
        &SAMPLES,
        prefs(true, HistoryRetention::Month),
        at(NOW),
    );
    assert!(matches!(got, Err(HistoryError::Io(_))), "{got:?}");
    assert!(
        folder.join("pierce.txt").exists(),
        "existing folder untouched"
    );
    assert_eq!(f.store.entries().expect("entries"), vec![]);
}

#[test]
fn first_load_prunes_only_folders_without_metadata_his_8() {
    struct Case {
        name: &'static str,
        setup: fn(&Fixture, &Path),
        want_exists: bool,
        want_loaded: bool,
    }
    fn write_metadata(folder: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(folder).expect("folder");
        std::fs::write(folder.join(METADATA_FILENAME), bytes).expect("metadata");
    }
    fn valid(n: u128) -> Vec<u8> {
        encode_metadata(&new_entry(n, DAY).into_entry()).expect("encode")
    }
    let cases = [
        Case {
            name: "valid entry",
            setup: |_, path| write_metadata(path, &valid(1)),
            want_exists: true,
            want_loaded: true,
        },
        Case {
            name: "orphan folder without metadata",
            setup: |_, path| std::fs::create_dir_all(path).expect("folder"),
            want_exists: false,
            want_loaded: false,
        },
        Case {
            name: "corrupt metadata",
            setup: |_, path| write_metadata(path, b"{ not json"),
            want_exists: true,
            want_loaded: false,
        },
        Case {
            name: "unreadable metadata (a folder)",
            setup: |_, path| {
                std::fs::create_dir_all(path.join(METADATA_FILENAME)).expect("metadata folder")
            },
            want_exists: true,
            want_loaded: false,
        },
        Case {
            name: "unsafe directoryName",
            setup: |_, path| write_metadata(path, &replace_key(&valid(2), "directoryName", "..")),
            want_exists: true,
            want_loaded: false,
        },
        Case {
            name: "unsafe audioFilename",
            setup: |_, path| {
                let bytes = replace_key(&valid(3), "audioFilename", "../../Greendale/escape.wav");
                write_metadata(path, &bytes);
            },
            want_exists: true,
            want_loaded: false,
        },
        Case {
            name: "plain file",
            setup: |_, path| std::fs::write(path, b"Senor Chang").expect("file"),
            want_exists: true,
            want_loaded: false,
        },
        Case {
            name: "hidden folder",
            setup: |_, path| std::fs::create_dir_all(path).expect("folder"),
            want_exists: true,
            want_loaded: false,
        },
        Case {
            name: "symlink to an outside folder with valid metadata",
            setup: |f, path| {
                write_metadata(&f.outside, &valid(4));
                symlink(&f.outside, path).expect("symlink");
            },
            want_exists: true,
            want_loaded: false,
        },
    ];
    for tc in cases {
        let f = Fixture::new();
        std::fs::create_dir_all(&f.history).expect("history");
        let name = match tc.name {
            "hidden folder" => ".Trashes".to_owned(),
            other => other.replace(' ', "-"),
        };
        let path = f.history.join(name);
        (tc.setup)(&f, &path);

        let got = f.store.entries().expect("entries");
        assert_eq!(
            path.symlink_metadata().is_ok(),
            tc.want_exists,
            "{}: exists",
            tc.name
        );
        assert_eq!(!got.is_empty(), tc.want_loaded, "{}: loaded", tc.name);
        f.assert_outside_intact();
        assert!(f.outside.exists(), "{}: outside folder kept", tc.name);
    }
}

#[test]
fn loaded_entries_point_at_their_own_folder() {
    let f = Fixture::new();
    // metadata copied into a folder whose name differs from its directoryName
    let folder = f.history.join("copied-by-hand");
    std::fs::create_dir_all(&folder).expect("folder");
    let entry = new_entry(1, DAY).into_entry();
    std::fs::write(
        folder.join(METADATA_FILENAME),
        encode_metadata(&entry).expect("encode"),
    )
    .expect("metadata");

    let got = f.store.entries().expect("entries");
    assert_eq!(got[0].directory_name.as_str(), "copied-by-hand");
    f.store.delete(entry.id).expect("delete");
    assert!(!folder.exists(), "the folder it was loaded from is deleted");
}

#[test]
fn retention_runs_on_disk_after_save_his_12_to_15() {
    struct Seed {
        n: u128,
        age: i64,
        audio: Option<u64>,
    }
    struct Case {
        name: &'static str,
        seeds: Vec<Seed>,
        save_age: i64,
        retention: HistoryRetention,
        want_deleted: Vec<u128>,
        want_stripped: Vec<u128>,
    }
    let cases = [
        Case {
            name: "expired entries go (HIS-13)",
            seeds: vec![
                Seed {
                    n: 1,
                    age: DAY,
                    audio: None,
                },
                Seed {
                    n: 2,
                    age: 10 * DAY,
                    audio: Some(100),
                },
                Seed {
                    n: 3,
                    age: 40 * DAY,
                    audio: None,
                },
            ],
            save_age: 0,
            retention: HistoryRetention::Week,
            want_deleted: vec![2, 3],
            want_stripped: vec![],
        },
        Case {
            name: "saved entry is exempt from age (HIS-15)",
            seeds: vec![Seed {
                n: 1,
                age: 10 * DAY,
                audio: None,
            }],
            save_age: 40 * DAY,
            retention: HistoryRetention::Week,
            want_deleted: vec![1],
            want_stripped: vec![],
        },
        Case {
            name: "entry cap drops the oldest (HIS-12)",
            seeds: (1..=MAX_ENTRIES as u128)
                .map(|n| Seed {
                    n,
                    age: n as i64 + 60,
                    audio: None,
                })
                .collect(),
            save_age: 0,
            retention: HistoryRetention::Forever,
            want_deleted: vec![MAX_ENTRIES as u128],
            want_stripped: vec![],
        },
        Case {
            name: "recording count budget strips the oldest audio (HIS-14)",
            seeds: (1..=MAX_AUDIO_RECORDINGS as u128)
                .map(|n| Seed {
                    n,
                    age: n as i64 * 60,
                    audio: Some(1_000),
                })
                .collect(),
            save_age: 0,
            retention: HistoryRetention::Month,
            want_deleted: vec![],
            want_stripped: vec![MAX_AUDIO_RECORDINGS as u128],
        },
        Case {
            name: "byte budget strips older audio, even small ones (HIS-14)",
            seeds: vec![
                Seed {
                    n: 1,
                    age: 60,
                    audio: Some(499_999_990),
                },
                Seed {
                    n: 2,
                    age: 120,
                    audio: Some(10),
                },
                Seed {
                    n: 3,
                    age: 180,
                    audio: None,
                },
            ],
            save_age: 0,
            retention: HistoryRetention::Month,
            want_deleted: vec![],
            want_stripped: vec![1, 2],
        },
    ];
    for tc in cases {
        let f = Fixture::new();
        for seed in &tc.seeds {
            f.seed(seed.n, seed.age, seed.audio);
        }
        let saved = f.save(0, tc.save_age, true, tc.retention);

        let saved_folder = entry_folder(&f.history, 0, tc.save_age);
        assert!(
            saved_folder.join(AUDIO_FILENAME).exists(),
            "{}: saved audio kept",
            tc.name
        );
        let list = f.store.entries().expect("entries");
        assert!(list.contains(&saved), "{}: saved entry listed", tc.name);
        for seed in &tc.seeds {
            let folder = entry_folder(&f.history, seed.n, seed.age);
            let listed = list.iter().find(|entry| entry.id.as_u128() == seed.n);
            if tc.want_deleted.contains(&seed.n) {
                assert!(!folder.exists(), "{}: {} folder deleted", tc.name, seed.n);
                assert!(listed.is_none(), "{}: {} unlisted", tc.name, seed.n);
                continue;
            }
            let listed = listed.unwrap_or_else(|| panic!("{}: {} listed", tc.name, seed.n));
            let on_disk = read_entry(&folder);
            assert_eq!(
                &on_disk, listed,
                "{}: {} memory matches disk",
                tc.name, seed.n
            );
            let want_audio = seed.audio.is_some() && !tc.want_stripped.contains(&seed.n);
            assert_eq!(
                folder.join(AUDIO_FILENAME).exists(),
                want_audio,
                "{}: {} audio file",
                tc.name,
                seed.n
            );
            assert_eq!(
                on_disk.has_audio(),
                want_audio,
                "{}: {} metadata",
                tc.name,
                seed.n
            );
            assert_eq!(
                on_disk.transcript,
                format!("Troy and Abed in the morning #{}", seed.n),
                "{}: {} transcript kept",
                tc.name,
                seed.n
            );
        }
        assert_eq!(f.changes(), 1, "{}: one change per save", tc.name);
    }
}

#[test]
fn exempt_entry_goes_at_the_next_save_his_15() {
    let f = Fixture::new();
    f.save(1, 40 * DAY, false, HistoryRetention::Week);
    assert!(
        entry_folder(&f.history, 1, 40 * DAY).exists(),
        "exempt while saved"
    );
    f.save(2, 0, false, HistoryRetention::Week);
    assert!(
        !entry_folder(&f.history, 1, 40 * DAY).exists(),
        "expired on next save"
    );
    assert_eq!(ids(&f.store.entries().expect("entries")), [2]);
}

#[test]
fn apply_retention_on_demand_his_16() {
    let f = Fixture::new();
    f.seed(1, DAY, None);
    f.seed(2, 10 * DAY, None);

    let changed = f
        .store
        .apply_retention(HistoryRetention::Month, at(NOW))
        .expect("month");
    assert!(!changed, "nothing expired (HIS-10)");
    assert_eq!(f.changes(), 0, "nothing expired, no change");
    assert_eq!(
        ids(&f.store.entries().expect("entries")),
        [1, 2],
        "preloaded"
    );

    let changed = f
        .store
        .apply_retention(HistoryRetention::Week, at(NOW))
        .expect("week");
    assert!(changed, "an entry expired (HIS-10)");
    assert_eq!(f.changes(), 1);
    assert!(!entry_folder(&f.history, 2, 10 * DAY).exists());
    assert_eq!(ids(&f.store.entries().expect("entries")), [1]);
}

#[test]
fn delete_one_delete_all_and_recreate_his_16a() {
    let f = Fixture::new();
    f.save(1, DAY, true, HistoryRetention::Month);
    f.save(2, 2 * DAY, true, HistoryRetention::Month);
    std::fs::write(f.history.join("notes.txt"), b"Dean Pelton").expect("plain file");

    f.store.delete(Uuid::from_u128(2)).expect("delete");
    assert!(
        !entry_folder(&f.history, 2, 2 * DAY).exists(),
        "folder gone"
    );
    assert!(f.history.join("notes.txt").exists(), "plain file untouched");
    assert_eq!(ids(&f.store.entries().expect("entries")), [1]);
    assert_eq!(f.changes(), 3);

    f.store.delete(Uuid::from_u128(42)).expect("unknown id");
    assert_eq!(f.changes(), 3, "unknown id changes nothing");

    f.store.delete_all().expect("delete all");
    assert!(!f.history.exists(), "whole folder removed");
    assert_eq!(f.store.entries().expect("entries"), vec![]);
    assert_eq!(f.changes(), 4);

    f.save(3, 0, false, HistoryRetention::Month);
    assert!(entry_folder(&f.history, 3, 0).exists(), "folder recreated");
    assert_eq!(
        f.exclusions(),
        [f.history.clone(), f.history.clone()],
        "excluded again"
    );
    assert_eq!(ids(&f.reopen().entries().expect("reload")), [3]);
}

#[test]
fn audio_path_his_24() {
    let f = Fixture::new();
    f.save(1, DAY, true, HistoryRetention::Month);
    f.save(2, DAY, false, HistoryRetention::Month);

    let got = f.store.audio_path(Uuid::from_u128(1)).expect("audio path");
    assert_eq!(
        std::fs::read(&got).expect("audio"),
        encode_wav(&SAMPLES).expect("wav")
    );

    struct Case {
        name: &'static str,
        id: u128,
    }
    std::fs::remove_file(got).expect("remove audio");
    let cases = [
        Case {
            name: "file deleted behind our back",
            id: 1,
        },
        Case {
            name: "no audio kept",
            id: 2,
        },
        Case {
            name: "unknown entry",
            id: 3,
        },
    ];
    for tc in cases {
        let err = f
            .store
            .audio_path(Uuid::from_u128(tc.id))
            .expect_err(tc.name);
        assert_eq!(err, HistoryError::MissingRecording, "{}", tc.name);
        assert_eq!(err.to_string(), MISSING_RECORDING_ERROR, "{}", tc.name);
    }
}

#[test]
fn symlinked_entry_folders_are_refused_arch_12() {
    let f = Fixture::new();
    f.save(1, DAY, true, HistoryRetention::Month);
    f.store.entries().expect("loaded");
    // swap the entry folder for a symlink to an outside folder holding audio
    let folder = entry_folder(&f.history, 1, DAY);
    std::fs::remove_dir_all(&folder).expect("remove folder");
    std::fs::write(f.outside.join(AUDIO_FILENAME), b"outside audio").expect("outside audio");
    symlink(&f.outside, &folder).expect("symlink");

    let got = f.store.delete(Uuid::from_u128(1));
    assert_eq!(
        got,
        Err(HistoryError::UnsafePath(folder.clone())),
        "delete refused"
    );
    assert_eq!(
        ids(&f.store.entries().expect("entries")),
        [1],
        "entry still listed"
    );

    let got = f.store.audio_path(Uuid::from_u128(1));
    assert!(
        matches!(got, Err(HistoryError::UnsafePath(_))),
        "audio path refused: {got:?}"
    );

    // 50 newer recordings push entry 1 past the audio budget
    for n in 2..=MAX_AUDIO_RECORDINGS as u128 + 1 {
        f.save(n, 0, true, HistoryRetention::Month);
    }
    assert!(
        f.outside.join(AUDIO_FILENAME).exists(),
        "outside audio kept"
    );
    assert!(folder.symlink_metadata().is_ok(), "symlink left in place");
    f.assert_outside_intact();
}

#[test]
fn symlinked_history_folder_is_not_cleared_arch_12() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.history.parent().expect("parent")).expect("data dir");
    symlink(&f.outside, &f.history).expect("symlink");
    let got = f.store.delete_all();
    assert_eq!(got, Err(HistoryError::UnsafePath(f.history.clone())));
    assert_eq!(f.changes(), 0, "refused clear changes nothing");
    f.assert_outside_intact();
}

#[test]
fn remove_descendant_arch_12() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path().join("History");
    let outside = temp.path().join("Greendale");
    std::fs::create_dir_all(root.join("entry")).expect("entry");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("escape.wav"), b"keep me").expect("outside file");
    symlink(&outside, root.join("link")).expect("symlink");

    struct Case {
        name: &'static str,
        target: PathBuf,
        want_refused: bool,
    }
    let cases = [
        Case {
            name: "the root itself",
            target: root.clone(),
            want_refused: true,
        },
        Case {
            name: "a sibling of the root",
            target: outside.clone(),
            want_refused: true,
        },
        Case {
            name: "parent traversal",
            target: root.join("..").join("Greendale").join("escape.wav"),
            want_refused: true,
        },
        Case {
            name: "symlink to outside",
            target: root.join("link"),
            want_refused: true,
        },
        Case {
            name: "through a symlinked folder",
            target: root.join("link").join("escape.wav"),
            want_refused: true,
        },
        Case {
            name: "missing child is fine",
            target: root.join("Chang"),
            want_refused: false,
        },
        Case {
            name: "entry folder",
            target: root.join("entry"),
            want_refused: false,
        },
    ];
    for tc in cases {
        let got = remove_descendant(&root, &tc.target);
        assert_eq!(
            matches!(got, Err(HistoryError::UnsafePath(_))),
            tc.want_refused,
            "{}: {got:?}",
            tc.name
        );
        assert!(
            outside.join("escape.wav").exists(),
            "{}: outside kept",
            tc.name
        );
        assert!(root.exists(), "{}: root kept", tc.name);
    }
    assert!(!root.join("entry").exists(), "entry folder removed");
}

#[test]
fn concurrent_saves_all_land() {
    let f = Fixture::new();
    std::thread::scope(|scope| {
        for thread in 0..4u128 {
            let store = &f.store;
            scope.spawn(move || {
                for i in 0..5u128 {
                    let n = thread * 100 + i;
                    store
                        .save(
                            new_entry(n, n as i64),
                            &SAMPLES,
                            prefs(false, HistoryRetention::Month),
                            at(NOW),
                        )
                        .expect("save");
                    store.entries().expect("read while saving");
                }
            });
        }
    });
    assert_eq!(f.store.entries().expect("entries").len(), 20);
    assert_eq!(f.reopen().entries().expect("reload").len(), 20);
    assert_eq!(f.changes(), 20);
}
