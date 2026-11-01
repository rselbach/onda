//! Content verification before a load (ASR-6, DL-11) and its marker.

use std::path::Path;
use std::time::{Duration, SystemTime};

use onda_domain::models::ModelError;

use super::{Fixture, MARKER, MODEL, VOCAB, WEIGHTS, fixture_files, fixture_manifest};
use crate::{Error, ModelManager, VERIFIED_MARKER};

struct Installed {
    _temp: tempfile::TempDir,
    manager: ModelManager,
    folder: std::path::PathBuf,
}

/// A manager over a temp models dir with the fixture model installed intact.
fn installed() -> Installed {
    let temp = tempfile::tempdir().expect("temp dir");
    let manifest = fixture_manifest("http://127.0.0.1:9/greendale");
    let (manager, _events) = ModelManager::new(temp.path().join("Models"), move |model| {
        (model == MODEL).then_some(manifest)
    })
    .expect("manager");
    let folder = manager.model_dir(MODEL);
    for (path, content) in fixture_files() {
        let file = folder.join(path);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(file, content).expect("write");
    }
    Installed {
        _temp: temp,
        manager,
        folder,
    }
}

/// Verifies and returns the result with every progress fraction reported.
fn verify(manager: &ModelManager) -> (Result<(), Error>, Vec<f64>) {
    let mut fractions = Vec::new();
    let result = manager.verify(MODEL, &mut |fraction| fractions.push(fraction));
    (result, fractions)
}

/// Flips the first byte of `path`, keeping its size and modification time.
fn corrupt_in_place(path: &Path) {
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .expect("mtime");
    let mut bytes = std::fs::read(path).expect("read");
    bytes[0] ^= 0xff;
    std::fs::write(path, &bytes).expect("write");
    set_modified(path, modified);
}

fn set_modified(path: &Path, time: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(time))
        .expect("set mtime");
}

fn incomplete(details: &str) -> Error {
    Error::Model(ModelError::IncompleteInstallation {
        model_id: MODEL.as_str().to_owned(),
        details: details.to_owned(),
    })
}

#[test]
fn verification_before_load_asr_6() {
    struct Case {
        name: &'static str,
        damage: fn(&Path),
        want: Result<(), Error>,
        want_marker: bool,
    }
    let cases = [
        Case {
            name: "intact folder passes and is marked",
            damage: |_| {},
            want: Ok(()),
            want_marker: true,
        },
        Case {
            name: "corrupt weights of the right size",
            damage: |folder| corrupt_in_place(&folder.join(WEIGHTS)),
            want: Err(incomplete(&format!("corrupt files: {WEIGHTS}"))),
            want_marker: false,
        },
        Case {
            name: "every corrupt file is listed",
            damage: |folder| {
                corrupt_in_place(&folder.join(MARKER));
                corrupt_in_place(&folder.join(WEIGHTS));
            },
            want: Err(incomplete(&format!("corrupt files: {MARKER}, {WEIGHTS}"))),
            want_marker: false,
        },
        Case {
            name: "missing file fails before hashing",
            damage: |folder| std::fs::remove_file(folder.join(VOCAB)).expect("remove"),
            want: Err(incomplete("missing files: vocab.json")),
            want_marker: false,
        },
    ];
    for tc in cases {
        let f = installed();
        (tc.damage)(&f.folder);
        let (got, fractions) = verify(&f.manager);
        assert_eq!(got, tc.want, "{}", tc.name);
        assert_eq!(
            f.folder.join(VERIFIED_MARKER).exists(),
            tc.want_marker,
            "{}",
            tc.name
        );
        if tc.name != "missing file fails before hashing" {
            assert_eq!(fractions.first(), Some(&0.0), "{}", tc.name);
            assert_eq!(fractions.last(), Some(&1.0), "{}", tc.name);
            assert!(
                fractions.windows(2).all(|w| w[0] < w[1]),
                "{}: {fractions:?}",
                tc.name
            );
        }
    }
}

/// The user-facing copy is the domain's "incomplete" error, which the app
/// turns into the ASR-6 notification offering Re-download Model.
#[test]
fn corrupt_folder_error_copy_is_the_domains() {
    let f = installed();
    corrupt_in_place(&f.folder.join(WEIGHTS));
    let (got, _) = verify(&f.manager);
    let want = ModelError::IncompleteInstallation {
        model_id: MODEL.as_str().to_owned(),
        details: format!("corrupt files: {WEIGHTS}"),
    }
    .to_string();
    assert_eq!(got.map_err(|e| e.to_string()), Err(want));
}

#[test]
fn marker_skips_rehash_until_a_file_changes() {
    struct Case {
        name: &'static str,
        change: fn(&Path),
        want: Result<(), Error>,
        want_hashed: bool,
    }
    let cases = [
        Case {
            name: "unchanged folder is trusted",
            change: |_| {},
            want: Ok(()),
            want_hashed: false,
        },
        Case {
            // proves the marker skips hashing: the bytes changed, the
            // metadata did not
            name: "same size and mtime is not re-hashed",
            change: |folder| corrupt_in_place(&folder.join(WEIGHTS)),
            want: Ok(()),
            want_hashed: false,
        },
        Case {
            name: "touched file is re-hashed",
            change: |folder| {
                let later = SystemTime::now() + Duration::from_secs(60);
                set_modified(&folder.join(VOCAB), later);
            },
            want: Ok(()),
            want_hashed: true,
        },
        Case {
            name: "touched corrupt file fails",
            change: |folder| {
                let path = folder.join(WEIGHTS);
                corrupt_in_place(&path);
                set_modified(&path, SystemTime::now() + Duration::from_secs(60));
            },
            want: Err(incomplete(&format!("corrupt files: {WEIGHTS}"))),
            want_hashed: true,
        },
        Case {
            name: "garbled marker is re-hashed",
            change: |folder| {
                std::fs::write(folder.join(VERIFIED_MARKER), "Chang was here").expect("write");
            },
            want: Ok(()),
            want_hashed: true,
        },
    ];
    for tc in cases {
        let f = installed();
        let (first, fractions) = verify(&f.manager);
        assert_eq!(first, Ok(()), "{}", tc.name);
        assert!(!fractions.is_empty(), "{}", tc.name);

        (tc.change)(&f.folder);
        let (got, fractions) = verify(&f.manager);
        assert_eq!(got, tc.want, "{}", tc.name);
        assert_eq!(!fractions.is_empty(), tc.want_hashed, "{}", tc.name);
    }
}

#[tokio::test]
async fn fresh_download_writes_marker() {
    let f = Fixture::new().await;
    f.manager.download(MODEL).await.expect("download");
    assert!(f.model_dir().join(VERIFIED_MARKER).is_file());
    let mut hashed = false;
    let result = f.manager.verify(MODEL, &mut |_| hashed = true);
    assert_eq!(result, Ok(()));
    assert!(!hashed, "a fresh download must not be hashed again");
}

#[tokio::test]
async fn download_keeping_earlier_files_leaves_verification_to_load() {
    let f = Fixture::new().await;
    let (path, content) = fixture_files()
        .into_iter()
        .find(|(path, _)| *path == WEIGHTS)
        .expect("weights fixture");
    let kept = f.model_dir().join(path);
    std::fs::create_dir_all(kept.parent().expect("parent")).expect("mkdir");
    std::fs::write(&kept, content).expect("write");

    f.manager.download(MODEL).await.expect("download");
    assert!(f.server.requests_for(WEIGHTS).is_empty());
    assert!(!f.model_dir().join(VERIFIED_MARKER).exists());
    let mut hashed = false;
    let result = f.manager.verify(MODEL, &mut |_| hashed = true);
    assert_eq!(result, Ok(()));
    assert!(hashed);
    assert!(f.model_dir().join(VERIFIED_MARKER).is_file());
}

#[test]
fn models_without_manifest_pass() {
    let f = installed();
    let mut hashed = false;
    let result = f
        .manager
        .verify(onda_domain::models::ModelId::SystemSpeech, &mut |_| {
            hashed = true
        });
    assert_eq!(result, Ok(()));
    assert!(!hashed);
}
