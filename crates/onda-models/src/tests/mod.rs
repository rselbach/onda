mod integrity;
mod server;

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use onda_asr::{ManifestFile, ModelManifest};
use onda_domain::models::{ModelError, ModelId};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::remove::remove_descendant;
use crate::{DownloadEvent, DownloadEventKind, Error, ModelManager, validate_folder};
use server::{Behavior, Server};

const MODEL: ModelId = ModelId::ParakeetTdtV2;
const VOCAB: &str = "vocab.json";
const MARKER: &str = "Decoder.mlmodelc/coremldata.bin";
const WEIGHTS: &str = "Encoder.mlmodelc/weights/weight.bin";
const WEIGHTS_SIZE: usize = 256 * 1024;

fn fixture_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (VOCAB, br#"{"troy":"barnes","abed":"nadir"}"#.to_vec()),
        (MARKER, b"greendale community college".to_vec()),
        (
            WEIGHTS,
            (0..WEIGHTS_SIZE).map(|i| (i * 31 % 251) as u8).collect(),
        ),
    ]
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// A manifest for the fixture files; the vocabulary is a plain git file,
/// the others are hashed LFS files.
fn fixture_manifest(base_url: &str) -> &'static ModelManifest {
    let files: Vec<ManifestFile> = fixture_files()
        .into_iter()
        .map(|(path, content)| ManifestFile {
            path,
            size: content.len() as u64,
            sha256: (path != VOCAB).then(|| leak(hex_sha256(&content))),
        })
        .collect();
    Box::leak(Box::new(ModelManifest {
        base_url: leak(base_url.to_string()),
        files: Box::leak(files.into_boxed_slice()),
    }))
}

struct Fixture {
    _temp: tempfile::TempDir,
    server: Server,
    manager: ModelManager,
    events: UnboundedReceiver<DownloadEvent>,
    models_dir: PathBuf,
}

impl Fixture {
    async fn new() -> Fixture {
        let temp = tempfile::tempdir().expect("temp dir");
        let server = Server::start(&fixture_files()).await;
        let manifest = fixture_manifest(&server.base_url);
        let models_dir = temp.path().join("Models");
        let (manager, events) = ModelManager::build(
            models_dir.clone(),
            Box::new(move |model| (model == MODEL).then_some(manifest)),
            Duration::from_millis(5),
        )
        .expect("manager");
        Fixture {
            _temp: temp,
            server,
            manager,
            events,
            models_dir,
        }
    }

    fn model_dir(&self) -> PathBuf {
        self.models_dir.join(MODEL.as_str())
    }

    /// Events received so far, as kinds.
    fn drain(&mut self) -> Vec<DownloadEventKind> {
        let mut kinds = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            assert_eq!(event.model, MODEL);
            kinds.push(event.kind);
        }
        kinds
    }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

fn assert_installed(folder: &Path) {
    for (path, content) in fixture_files() {
        let got = std::fs::read(folder.join(path)).expect(path);
        assert!(got == content, "{path} content");
        assert!(
            !folder.join(format!("{path}.part")).exists(),
            "{path}.part left"
        );
    }
}

/// Start, progress events, end; returns the progress values.
fn assert_lifecycle(kinds: &[DownloadEventKind], end: &DownloadEventKind) -> Vec<f64> {
    assert_eq!(
        kinds.first(),
        Some(&DownloadEventKind::Started),
        "{kinds:?}"
    );
    assert_eq!(kinds.last(), Some(end), "{kinds:?}");
    kinds[1..kinds.len() - 1]
        .iter()
        .map(|kind| match kind {
            DownloadEventKind::Progress(p) => *p,
            other => panic!("unexpected event {other:?} in {kinds:?}"),
        })
        .collect()
}

async fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn fresh_download_dl_1_dl_2() {
    let mut f = Fixture::new().await;
    let folder = f.manager.download(MODEL).await.expect("download");
    assert_eq!(folder, f.model_dir());
    assert_installed(&folder);
    let progress = assert_lifecycle(&f.drain(), &DownloadEventKind::Finished);
    assert_eq!(progress.last(), Some(&1.0));
    assert!(f.manager.validate(MODEL).expect("manifest").is_complete());
    assert_eq!(f.manager.downloaded_models(), vec![MODEL]);
    assert!(!f.manager.is_downloading(MODEL));
    for request in f.server.requests() {
        assert_eq!(request.range, None, "{request:?}");
    }

    // already downloaded: returns at once, no request, no event
    let requests = f.server.requests().len();
    f.manager.download(MODEL).await.expect("second download");
    assert_eq!(f.server.requests().len(), requests);
    assert_eq!(f.drain(), vec![]);
}

#[tokio::test]
async fn resume_after_cancel_dl_3() {
    struct Case {
        name: &'static str,
        resume_behavior: Option<Behavior>,
        want_range: Option<String>,
        want_body_len: usize,
    }
    const STALL: usize = 100_000;
    let cases = [
        Case {
            name: "server honors range",
            resume_behavior: None,
            want_range: Some(format!("bytes={STALL}-")),
            want_body_len: WEIGHTS_SIZE - STALL,
        },
        Case {
            name: "server ignores range: file restarts",
            resume_behavior: Some(Behavior::IgnoreRange),
            want_range: Some(format!("bytes={STALL}-")),
            want_body_len: WEIGHTS_SIZE,
        },
    ];
    for tc in cases {
        let mut f = Fixture::new().await;
        f.server
            .set_behavior(WEIGHTS, Some(Behavior::StallAfter(STALL)));
        let manager = f.manager.clone();
        let task = tokio::spawn(async move { manager.download(MODEL).await });
        let part = f.model_dir().join(format!("{WEIGHTS}.part"));
        wait_until("partial weights", || file_len(&part) >= STALL as u64).await;
        assert!(f.manager.cancel(MODEL), "{}", tc.name);
        let result = task.await.expect("join");
        assert_eq!(result, Err(Error::Cancelled), "{}", tc.name);
        assert_lifecycle(&f.drain(), &DownloadEventKind::Cancelled);
        assert_eq!(file_len(&part), STALL as u64, "{}: partial kept", tc.name);
        assert!(f.model_dir().join(VOCAB).exists(), "{}", tc.name);

        f.server.set_behavior(WEIGHTS, tc.resume_behavior);
        f.manager.download(MODEL).await.expect(tc.name);
        assert_installed(&f.model_dir());
        let progress = assert_lifecycle(&f.drain(), &DownloadEventKind::Finished);
        assert_eq!(progress.last(), Some(&1.0), "{}", tc.name);
        let weights = f.server.requests_for(WEIGHTS);
        let resumed = weights.last().expect("resume request");
        assert_eq!(resumed.range, tc.want_range, "{}", tc.name);
        assert_eq!(resumed.body_len, tc.want_body_len, "{}", tc.name);
        assert_eq!(
            f.server.requests_for(VOCAB).len(),
            1,
            "{}: vocab refetched",
            tc.name
        );
    }
}

#[tokio::test]
async fn concurrent_requests_join_one_download_dl_1() {
    let mut f = Fixture::new().await;
    let (first, second) = tokio::join!(f.manager.download(MODEL), f.manager.download(MODEL));
    assert_eq!(first, Ok(f.model_dir()));
    assert_eq!(second, Ok(f.model_dir()));
    for (path, _) in fixture_files() {
        assert_eq!(f.server.requests_for(path).len(), 1, "{path}");
    }
    assert_lifecycle(&f.drain(), &DownloadEventKind::Finished);
}

#[tokio::test]
async fn cancelled_download_is_not_joined() {
    let mut f = Fixture::new().await;
    f.server
        .set_behavior(WEIGHTS, Some(Behavior::StallAfter(1)));
    let manager = f.manager.clone();
    let task = tokio::spawn(async move { manager.download(MODEL).await });
    let part = f.model_dir().join(format!("{WEIGHTS}.part"));
    wait_until("partial weights", || file_len(&part) >= 1).await;
    f.server.set_behavior(WEIGHTS, None);
    assert!(f.manager.cancel(MODEL));
    // a new request right after cancel starts a fresh download
    assert_eq!(f.manager.download(MODEL).await, Ok(f.model_dir()));
    assert_eq!(task.await.expect("join"), Err(Error::Cancelled));
    assert_installed(&f.model_dir());
    let kinds = f.drain();
    let ends: Vec<&DownloadEventKind> = kinds
        .iter()
        .filter(|k| !matches!(k, DownloadEventKind::Progress(_)))
        .collect();
    assert_eq!(
        ends,
        [
            &DownloadEventKind::Started,
            &DownloadEventKind::Cancelled,
            &DownloadEventKind::Started,
            &DownloadEventKind::Finished
        ]
    );
}

#[tokio::test]
async fn failure_deletes_partial_folder_dl_4() {
    struct Case {
        name: &'static str,
        behavior: Behavior,
        want: Error,
        want_requests: usize,
    }
    let cases = [
        Case {
            name: "server error after retries",
            behavior: Behavior::Fail,
            want: Error::Download(format!(
                "Could not download {WEIGHTS}: HTTP 500 Internal Server Error"
            )),
            want_requests: 4,
        },
        Case {
            name: "checksum mismatch is not retried",
            behavior: Behavior::Corrupt,
            want: Error::Download(format!("Checksum mismatch for {WEIGHTS}")),
            want_requests: 1,
        },
    ];
    for tc in cases {
        let mut f = Fixture::new().await;
        f.server.set_behavior(WEIGHTS, Some(tc.behavior));
        let got = f.manager.download(MODEL).await;
        assert_eq!(got, Err(tc.want.clone()), "{}", tc.name);
        assert!(!f.model_dir().exists(), "{}: partial folder kept", tc.name);
        assert!(f.models_dir.exists(), "{}: models dir deleted", tc.name);
        let failed = DownloadEventKind::Failed {
            error: tc.want.to_string(),
        };
        assert_lifecycle(&f.drain(), &failed);
        assert_eq!(
            f.server.requests_for(WEIGHTS).len(),
            tc.want_requests,
            "{}",
            tc.name
        );
        assert!(!f.manager.is_downloading(MODEL), "{}", tc.name);
    }
}

#[tokio::test]
async fn unknown_model_is_not_found() {
    let f = Fixture::new().await;
    let got = f.manager.download(ModelId::SystemSpeech).await;
    assert_eq!(
        got,
        Err(Error::Model(ModelError::NotFound {
            model_id: "system-speech".to_string()
        }))
    );
    assert_eq!(f.manager.validate(ModelId::SystemSpeech), None);
}

#[tokio::test]
async fn progress_is_monotonic_and_throttled_dl_2() {
    let mut f = Fixture::new().await;
    f.server.set_behavior(WEIGHTS, Some(Behavior::Slow));
    let start = Instant::now();
    f.manager.download(MODEL).await.expect("download");
    let elapsed = start.elapsed();
    let progress = assert_lifecycle(&f.drain(), &DownloadEventKind::Finished);
    assert!(
        progress.windows(2).all(|w| w[0] <= w[1]),
        "not monotonic: {progress:?}"
    );
    assert!(
        progress.iter().all(|p| (0.0..=1.0).contains(p)),
        "{progress:?}"
    );
    assert_eq!(progress.last(), Some(&1.0));
    let before_done = progress.iter().filter(|&&p| p < 1.0).count();
    let allowed = (elapsed.as_millis() / 100) as usize + 1;
    assert!(
        (2..=allowed).contains(&before_done),
        "{before_done} events in {elapsed:?}: {progress:?}"
    );
}

#[test]
fn validation_dl_5() {
    struct Case {
        name: &'static str,
        setup: fn(&Path),
        want: Option<&'static str>,
    }
    fn install(folder: &Path) {
        for (path, content) in fixture_files() {
            let file = folder.join(path);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
            std::fs::write(file, content).expect("write");
        }
    }
    let cases = [
        Case {
            name: "complete",
            setup: install,
            want: None,
        },
        Case {
            name: "missing plain file",
            setup: |folder| {
                install(folder);
                std::fs::remove_file(folder.join(VOCAB)).expect("remove");
            },
            want: Some("missing files: vocab.json"),
        },
        Case {
            name: "half-written weights",
            setup: |folder| {
                install(folder);
                std::fs::write(folder.join(WEIGHTS), [0; 1024]).expect("write");
            },
            want: Some("missing files: Encoder.mlmodelc/weights/weight.bin; missing model weights"),
        },
        Case {
            name: "zero-byte placeholder",
            setup: |folder| {
                install(folder);
                std::fs::write(folder.join(MARKER), []).expect("write");
            },
            // only weight.bin counts as weights, even though every file is hashed
            want: Some("missing files: Decoder.mlmodelc/coremldata.bin"),
        },
        Case {
            name: "partial file does not count",
            setup: |folder| {
                install(folder);
                std::fs::rename(folder.join(VOCAB), folder.join(format!("{VOCAB}.part")))
                    .expect("rename");
            },
            want: Some("missing files: vocab.json"),
        },
        Case {
            name: "missing folder",
            setup: |_| {},
            want: Some(
                "folder does not exist; missing files: vocab.json, Decoder.mlmodelc/coremldata.bin, Encoder.mlmodelc/weights/weight.bin; missing model weights",
            ),
        },
    ];
    let manifest = fixture_manifest("http://127.0.0.1:9/greendale");
    for tc in cases {
        let temp = tempfile::tempdir().expect("temp dir");
        let folder = temp.path().join(MODEL.as_str());
        if tc.name != "missing folder" {
            std::fs::create_dir(&folder).expect("mkdir");
        }
        (tc.setup)(&folder);
        let got = validate_folder(&folder, manifest);
        assert_eq!(got.failure_details().as_deref(), tc.want, "{}", tc.name);
    }
}

#[tokio::test]
async fn incomplete_folder_is_not_downloaded() {
    let f = Fixture::new().await;
    std::fs::create_dir_all(f.model_dir()).expect("mkdir");
    std::fs::write(f.model_dir().join(VOCAB), "{}").expect("write");
    assert_eq!(f.manager.downloaded_models(), vec![]);
}

#[tokio::test]
async fn delete_and_redownload_dl_9_dl_11() {
    let mut f = Fixture::new().await;
    assert_eq!(
        f.manager.delete(MODEL),
        Err(Error::Model(ModelError::NotFound {
            model_id: MODEL.as_str().to_string()
        }))
    );
    f.manager.download(MODEL).await.expect("download");
    f.manager.delete(MODEL).expect("delete");
    assert!(!f.model_dir().exists());
    assert!(f.models_dir.exists());

    f.manager.download(MODEL).await.expect("download again");
    f.drain();
    // a corrupt file passes validation; re-download replaces it
    std::fs::write(
        f.model_dir().join(VOCAB),
        br#"{"chang":"senor"}xxxxxxxxxxxxxxx"#,
    )
    .expect("corrupt");
    f.manager.redownload(MODEL).await.expect("redownload");
    assert_installed(&f.model_dir());
    assert_eq!(f.server.requests_for(VOCAB).len(), 3);
    assert_lifecycle(&f.drain(), &DownloadEventKind::Finished);
}

#[tokio::test]
async fn delete_refuses_symlink_out_of_models_dir_arch_12() {
    let f = Fixture::new().await;
    let outside = f._temp.path().join("dean-pelton");
    std::fs::create_dir_all(&outside).expect("mkdir");
    std::fs::write(outside.join("costume.txt"), "dalmatian").expect("write");
    std::fs::create_dir_all(&f.models_dir).expect("mkdir");
    symlink(&outside, f.model_dir()).expect("symlink");

    assert_eq!(
        f.manager.delete(MODEL),
        Err(Error::UnsafePath(f.model_dir()))
    );
    assert!(outside.join("costume.txt").exists());
    assert!(f.model_dir().symlink_metadata().is_ok(), "link removed");
}

#[test]
fn remove_descendant_arch_12() {
    struct Case {
        name: &'static str,
        target: fn(&Path, &Path) -> PathBuf,
        want_ok: bool,
    }
    let cases = [
        Case {
            name: "model folder",
            target: |root, _| root.join("parakeet-tdt-v2"),
            want_ok: true,
        },
        Case {
            name: "the models dir itself",
            target: |root, _| root.to_path_buf(),
            want_ok: false,
        },
        Case {
            name: "parent traversal",
            target: |root, _| root.join("parakeet-tdt-v2/../.."),
            want_ok: false,
        },
        Case {
            name: "sibling through traversal",
            target: |root, _| root.join("../dean-pelton"),
            want_ok: false,
        },
        Case {
            name: "outside path",
            target: |_, outside| outside.to_path_buf(),
            want_ok: false,
        },
        Case {
            name: "symlink to outside",
            target: |root, _| root.join("star-burns"),
            want_ok: false,
        },
        Case {
            name: "through a symlinked parent",
            target: |root, _| root.join("star-burns/costume.txt"),
            want_ok: false,
        },
        Case {
            name: "symlink to another model folder",
            target: |root, _| root.join("magnitude"),
            want_ok: false,
        },
    ];
    for tc in cases {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("Models");
        let outside = temp.path().join("dean-pelton");
        std::fs::create_dir_all(root.join("parakeet-tdt-v2")).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");
        std::fs::write(outside.join("costume.txt"), "dalmatian").expect("write");
        symlink(&outside, root.join("star-burns")).expect("symlink");
        symlink(root.join("parakeet-tdt-v2"), root.join("magnitude")).expect("symlink");

        let target = (tc.target)(&root, &outside);
        let got = remove_descendant(&root, &target);
        assert_eq!(got.is_ok(), tc.want_ok, "{}: {got:?}", tc.name);
        assert!(outside.join("costume.txt").exists(), "{}", tc.name);
        assert!(root.exists(), "{}", tc.name);
        if !tc.want_ok {
            assert!(root.join("parakeet-tdt-v2").exists(), "{}", tc.name);
        }
    }
}

#[tokio::test]
async fn remove_obsolete_dl_15() {
    let f = Fixture::new().await;
    let outside = f._temp.path().join("dean-pelton");
    std::fs::create_dir_all(&outside).expect("mkdir");
    for name in ["parakeet-tdt-v3", "parakeet-tdt-v2", "troy-and-abed"] {
        std::fs::create_dir_all(f.models_dir.join(name).join("weights")).expect("mkdir");
    }
    symlink(&outside, f.models_dir.join("nemotron")).expect("symlink");
    let removed = [
        ("parakeet-tdt-v3", ModelId::ParakeetUltra),
        ("nemotron", ModelId::ParakeetUnified),
        ("parakeet-tdt-v2", ModelId::ParakeetUltra),
    ];

    let deleted = f.manager.remove_obsolete(&removed);
    assert_eq!(deleted, vec!["parakeet-tdt-v3".to_string()]);
    assert!(!f.models_dir.join("parakeet-tdt-v3").exists());
    assert!(
        f.models_dir.join("parakeet-tdt-v2").exists(),
        "current id kept"
    );
    assert!(f.models_dir.join("troy-and-abed").exists(), "unknown kept");
    assert!(outside.exists(), "symlink target kept");
    assert!(f.models_dir.join("nemotron").symlink_metadata().is_ok());
}

#[test]
fn event_status_and_end() {
    use onda_domain::download_progress::{DownloadEnd, DownloadStatus, status_text};
    struct Case {
        kind: DownloadEventKind,
        want_text: &'static str,
        want_end: Option<DownloadEnd>,
    }
    let cases = [
        Case {
            kind: DownloadEventKind::Started,
            want_text: "Downloading Parakeet TDT v2... 0%",
            want_end: None,
        },
        Case {
            kind: DownloadEventKind::Progress(0.5),
            want_text: "Downloading Parakeet TDT v2... 50%",
            want_end: None,
        },
        Case {
            kind: DownloadEventKind::Finished,
            want_text: "Downloaded Parakeet TDT v2",
            want_end: Some(DownloadEnd::Finished),
        },
        Case {
            kind: DownloadEventKind::Cancelled,
            want_text: "Download cancelled: Parakeet TDT v2",
            want_end: Some(DownloadEnd::Cancelled),
        },
        Case {
            kind: DownloadEventKind::Failed {
                error: "HTTP 500".to_string(),
            },
            want_text: "Download failed: Parakeet TDT v2",
            want_end: Some(DownloadEnd::Failed {
                error: Some("HTTP 500".to_string()),
            }),
        },
    ];
    for tc in cases {
        let event = DownloadEvent {
            model: MODEL,
            kind: tc.kind,
        };
        let status: DownloadStatus = event.status();
        assert_eq!(status_text("Parakeet TDT v2", status), tc.want_text);
        assert_eq!(event.end(), tc.want_end, "{}", tc.want_text);
    }
}
