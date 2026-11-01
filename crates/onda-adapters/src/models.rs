//! Installed models through `onda-models`, with an injected inference
//! backend. Every downloaded model's engine verifies its folder before the
//! backend reads it, so a corrupt file fails the load instead of crashing
//! the runtime (ASR-6). Built-in engines have no folder and skip it.

use std::path::Path;
use std::sync::Arc;

use onda_app::seams::{DownloadUpdate, DownloadUpdateKind, ModelStore};
use onda_asr::{AsrError, LoadProgress, SpeechEngine};
use onda_domain::languages::Language;
use onda_domain::migration::REMOVED_MODEL_IDS;
use onda_domain::models::{Catalog, ModelId};
use onda_models::{DownloadEvent, DownloadEventKind, ModelManager};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;

/// Builds an unloaded engine for a model from its folder, or `None` when the
/// backend cannot run it.
pub type EngineFactory = Box<dyn Fn(ModelId, &Path) -> Option<Box<dyn SpeechEngine>> + Send + Sync>;

/// Receives every download's progress and outcome, in order. The
/// composition root forwards these to `App::download_event`.
pub type DownloadListener = Arc<dyn Fn(DownloadUpdate) + Send + Sync>;

/// [`ModelStore`] over a [`ModelManager`].
pub struct Models {
    manager: ModelManager,
    catalog: Catalog,
    engines: EngineFactory,
    runtime: Handle,
    listener: DownloadListener,
}

impl Models {
    /// Serves models from `manager`; `runtime` runs downloads. The manager's
    /// own events go to `listener` through [`forward_download_events`];
    /// `listener` also hears what the manager does not report itself (a
    /// model that was already complete, a model with no download).
    pub fn new(
        manager: ModelManager,
        catalog: Catalog,
        engines: EngineFactory,
        runtime: Handle,
        listener: DownloadListener,
    ) -> Self {
        Self {
            manager,
            catalog,
            engines,
            runtime,
            listener,
        }
    }

    fn run(&self, model: ModelId, redownload: bool) {
        // the manager reports nothing for a model that needs no download
        let complete = self.manager.validate(model).map(|v| v.is_complete());
        let listener = self.listener.clone();
        match complete {
            None => {
                listener(DownloadUpdate {
                    model,
                    kind: DownloadUpdateKind::Failed {
                        error: format!("Model '{model}' cannot be downloaded."),
                    },
                });
                return;
            }
            Some(true) if !redownload => {
                listener(DownloadUpdate {
                    model,
                    kind: DownloadUpdateKind::Finished,
                });
                return;
            }
            Some(_) => {}
        }
        let manager = self.manager.clone();
        self.runtime.spawn(async move {
            let result = match redownload {
                true => manager.redownload(model).await,
                false => manager.download(model).await,
            };
            if let Err(error) = result {
                // the manager already reported it through its events
                tracing::warn!(%error, model = model.as_str(), "download ended without a model");
            }
        });
    }
}

impl ModelStore for Models {
    fn downloaded(&self) -> Vec<ModelId> {
        self.catalog.downloaded(&self.manager.downloaded_models())
    }

    fn engine(&self, model: ModelId) -> Option<Box<dyn SpeechEngine>> {
        let inner = (self.engines)(model, &self.manager.model_dir(model))?;
        if self.catalog.is_built_in(model) {
            // the platform owns a built-in engine's files; there is nothing
            // to verify
            return Some(inner);
        }
        Some(Box::new(VerifiedEngine::new(
            self.manager.clone(),
            model,
            inner,
        )))
    }

    fn start_download(&self, model: ModelId) {
        self.run(model, false);
    }

    fn cancel_download(&self, model: ModelId) {
        self.manager.cancel(model);
    }

    fn delete(&self, model: ModelId) -> Result<(), String> {
        self.manager
            .delete(model)
            .map_err(|error| error.to_string())
    }

    fn redownload(&self, model: ModelId) {
        self.run(model, true);
    }

    fn remove_obsolete(&self) {
        // the manager logs each folder it deletes or fails to delete
        self.manager.remove_obsolete(REMOVED_MODEL_IDS);
    }
}

/// Status shown while the model's files are hashed (ASR-2 runtime progress).
const VERIFYING_STATUS: &str = "Verifying model...";

/// A [`SpeechEngine`] that verifies its model folder against the manifest
/// hashes before the wrapped engine loads it ([`ModelManager::verify`]).
/// A corrupt or tampered file fails the load with
/// [`AsrError::LoadFailed`] carrying the domain's "incomplete" copy, and the
/// wrapped engine never sees it: CoreML crashes on some corrupt files rather
/// than failing. Every other call is delegated.
struct VerifiedEngine {
    manager: ModelManager,
    model: ModelId,
    inner: Box<dyn SpeechEngine>,
    loaded: bool,
}

impl VerifiedEngine {
    /// Wraps `inner`, the engine for `model`, whose folder `manager` owns.
    fn new(manager: ModelManager, model: ModelId, inner: Box<dyn SpeechEngine>) -> Self {
        Self {
            manager,
            model,
            inner,
            loaded: false,
        }
    }
}

impl SpeechEngine for VerifiedEngine {
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        if !self.loaded {
            let mut report = |fraction: f64| {
                progress(LoadProgress {
                    status: VERIFYING_STATUS.to_owned(),
                    fraction,
                });
            };
            self.manager
                .verify(self.model, &mut report)
                .map_err(|error| AsrError::LoadFailed {
                    detail: error.to_string(),
                })?;
        }
        self.inner.load(progress)?;
        self.loaded = true;
        Ok(())
    }

    fn unload(&mut self) {
        self.inner.unload();
        self.loaded = false;
    }

    fn supports_preview(&self) -> bool {
        self.inner.supports_preview()
    }

    fn min_samples(&self) -> usize {
        self.inner.min_samples()
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        self.inner.transcribe(samples, language)
    }

    fn preview(&mut self, samples: &[f32], language: Option<Language>) -> Result<String, AsrError> {
        self.inner.preview(samples, language)
    }

    fn reset_preview(&mut self) {
        self.inner.reset_preview();
    }
}

/// The core's view of one manager event.
pub fn download_update(event: DownloadEvent) -> DownloadUpdate {
    let kind = match event.kind {
        DownloadEventKind::Started => DownloadUpdateKind::Started,
        DownloadEventKind::Progress(progress) => DownloadUpdateKind::Progress(progress),
        DownloadEventKind::Finished => DownloadUpdateKind::Finished,
        DownloadEventKind::Cancelled => DownloadUpdateKind::Cancelled,
        DownloadEventKind::Failed { error } => DownloadUpdateKind::Failed { error },
    };
    DownloadUpdate {
        model: event.model,
        kind,
    }
}

/// Forwards the manager's download events to `listener` on `runtime` until
/// the manager is gone.
pub fn forward_download_events(
    runtime: &Handle,
    mut events: UnboundedReceiver<DownloadEvent>,
    listener: DownloadListener,
) {
    runtime.spawn(async move {
        while let Some(event) = events.recv().await {
            tracing::debug!(?event, "model download");
            listener(download_update(event));
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use onda_asr::{ManifestFile, ModelManifest};
    use onda_domain::model_selection::load_failure_notification;
    use onda_domain::models::{ModelError, SystemSpeechInfo};
    use onda_models::VERIFIED_MARKER;

    use super::*;

    const MODEL: ModelId = ModelId::ParakeetTdtV2;
    const WEIGHTS: &str = "Encoder.mlmodelc/model.mil";
    const WEIGHTS_CONTENT: &[u8] = b"Troy and Abed in the morning";
    const VOCAB: &str = "vocab.json";
    const VOCAB_CONTENT: &[u8] = br#"{"greendale":"human being"}"#;
    static MANIFEST: ModelManifest = ModelManifest {
        base_url: "http://127.0.0.1:9/greendale",
        files: &[
            ManifestFile {
                path: WEIGHTS,
                size: 28,
                sha256: Some("2bd31818080084d111c56833b8ccba5992c2cc81d985b94404a42b0d42f12212"),
            },
            ManifestFile {
                path: VOCAB,
                size: 27,
                sha256: Some("1cd3bc0f7a2d416b6d9be397d57970fac8ea4809990a32240af0694767cb253c"),
            },
        ],
    };

    /// Stands in for CoreML: counts loads and reports one runtime step.
    struct FakeEngine(Arc<AtomicU32>);

    impl SpeechEngine for FakeEngine {
        fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            progress(LoadProgress {
                status: "Compiling Encoder.mlmodelc...".to_owned(),
                fraction: 0.5,
            });
            Ok(())
        }

        fn unload(&mut self) {}

        fn supports_preview(&self) -> bool {
            true
        }

        fn min_samples(&self) -> usize {
            4_800
        }

        fn transcribe(&mut self, _: &[f32], _: Option<Language>) -> Result<String, AsrError> {
            Ok("Pop pop!".to_owned())
        }

        fn preview(&mut self, _: &[f32], _: Option<Language>) -> Result<String, AsrError> {
            Ok(String::new())
        }

        fn reset_preview(&mut self) {}
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        _runtime: tokio::runtime::Runtime,
        models: Models,
        folder: std::path::PathBuf,
        backend_loads: Arc<AtomicU32>,
    }

    /// A store over a temp models dir with the fixture model installed.
    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().expect("temp dir");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let (manager, _events) = ModelManager::new(temp.path().join("Models"), |model| {
            (model == MODEL).then_some(&MANIFEST)
        })
        .expect("manager");
        let folder = manager.model_dir(MODEL);
        for (path, content) in [(WEIGHTS, WEIGHTS_CONTENT), (VOCAB, VOCAB_CONTENT)] {
            let file = folder.join(path);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
            std::fs::write(file, content).expect("write");
        }
        let backend_loads = Arc::new(AtomicU32::new(0));
        let engine_loads = backend_loads.clone();
        let models = Models::new(
            manager,
            Catalog::new(None),
            Box::new(move |_, _| Some(Box::new(FakeEngine(engine_loads.clone())))),
            runtime.handle().clone(),
            Arc::new(|_| {}),
        );
        Fixture {
            _temp: temp,
            _runtime: runtime,
            models,
            folder,
            backend_loads,
        }
    }

    fn load(models: &Models) -> (Result<(), AsrError>, Vec<LoadProgress>) {
        let mut engine = models.engine(MODEL).expect("engine");
        let mut steps = Vec::new();
        let result = engine.load(&mut |step| steps.push(step));
        (result, steps)
    }

    fn statuses(steps: &[LoadProgress]) -> Vec<&str> {
        let mut statuses: Vec<&str> = steps.iter().map(|s| s.status.as_str()).collect();
        statuses.dedup();
        statuses
    }

    /// A corrupt file of the right size passes the launch check (DL-5) but
    /// must fail the load before the backend reads it, with the copy that
    /// offers Re-download Model (ASR-6, DL-11).
    #[test]
    fn corrupt_model_fails_load_without_reaching_backend() {
        struct Case {
            name: &'static str,
            damage: fn(&std::path::Path),
            want_detail: Option<String>,
            want_backend_loads: u32,
            want_statuses: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "intact model verifies, then loads",
                damage: |_| {},
                want_detail: None,
                want_backend_loads: 1,
                want_statuses: vec![VERIFYING_STATUS, "Compiling Encoder.mlmodelc..."],
            },
            Case {
                name: "same-size corrupt model.mil",
                damage: |folder| {
                    std::fs::write(folder.join(WEIGHTS), b"Troy and Abed in the mornin!")
                        .expect("corrupt");
                },
                want_detail: Some(
                    ModelError::IncompleteInstallation {
                        model_id: MODEL.as_str().to_owned(),
                        details: format!("corrupt files: {WEIGHTS}"),
                    }
                    .to_string(),
                ),
                want_backend_loads: 0,
                want_statuses: vec![VERIFYING_STATUS],
            },
        ];
        for tc in cases {
            let f = fixture();
            (tc.damage)(&f.folder);
            assert!(f.models.downloaded().contains(&MODEL), "{}", tc.name);

            let (got, steps) = load(&f.models);
            let want = tc.want_detail.map(|detail| AsrError::LoadFailed { detail });
            assert_eq!(got.err(), want, "{}", tc.name);
            assert_eq!(
                f.backend_loads.load(Ordering::SeqCst),
                tc.want_backend_loads,
                "{}",
                tc.name
            );
            assert_eq!(statuses(&steps), tc.want_statuses, "{}", tc.name);
        }
    }

    /// The verifying wrapper reports the backend's minimum, so the session
    /// drops clips Parakeet cannot decode (GATE-2).
    #[test]
    fn verified_engine_reports_the_backend_minimum() {
        let f = fixture();
        let engine = f.models.engine(MODEL).expect("engine");
        assert_eq!(engine.min_samples(), 4_800);
    }

    /// DL-15: nothing in the (empty) removed map touches installed models.
    #[test]
    fn remove_obsolete_keeps_current_models_dl_15() {
        let f = fixture();
        f.models.remove_obsolete();
        assert!(f.folder.exists());
        assert!(f.models.downloaded().contains(&MODEL));
    }

    /// The app turns the failed load into the ASR-6 notification; the files
    /// are on disk, so it offers Re-download Model.
    #[test]
    fn corrupt_model_notification_offers_redownload() {
        let f = fixture();
        std::fs::write(f.folder.join(VOCAB), br#"{"greendale":"human beinG"}"#).expect("corrupt");
        let (got, _) = load(&f.models);
        let error = got.expect_err("load must fail");
        let message = load_failure_notification(MODEL, &f.models.downloaded(), &error.to_string());
        assert_eq!(
            message.body("menu bar icon"),
            "Failed to load the transcription model Choose Retry Loading Model from the menu bar icon, or Re-download Model to replace its files."
        );
    }

    /// Verified once, the folder is trusted until a file changes, and an
    /// already loaded engine is not verified again.
    #[test]
    fn verified_folder_loads_without_rehash() {
        let f = fixture();
        let mut engine = f.models.engine(MODEL).expect("engine");
        let mut steps = Vec::new();
        engine
            .load(&mut |step| steps.push(step))
            .expect("first load");
        assert!(statuses(&steps).contains(&VERIFYING_STATUS));
        assert!(f.folder.join(VERIFIED_MARKER).is_file());

        steps.clear();
        engine.load(&mut |step| steps.push(step)).expect("reload");
        assert!(!statuses(&steps).contains(&VERIFYING_STATUS));

        let (got, steps) = load(&f.models);
        assert_eq!(got, Ok(()));
        assert_eq!(statuses(&steps), vec!["Compiling Encoder.mlmodelc..."]);
        assert_eq!(f.backend_loads.load(Ordering::SeqCst), 3);
    }

    /// A built-in engine counts as downloaded and loads without
    /// verification: it has no folder or manifest (DL-10).
    #[test]
    fn built_in_engine_skips_verification() {
        let temp = tempfile::tempdir().expect("temp dir");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let (manager, _events) =
            ModelManager::new(temp.path().join("Models"), |_| None).expect("manager");
        let catalog = Catalog::new(Some(SystemSpeechInfo {
            description: "Built-in Greendale speech recognition".to_owned(),
            languages: vec!["en".to_owned(), "ko".to_owned()],
            license_name: "Greendale System".to_owned(),
            license_url: "https://greendale.example/license".to_owned(),
            attribution: "Greendale Speech Framework".to_owned(),
        }));
        let backend_loads = Arc::new(AtomicU32::new(0));
        let engine_loads = backend_loads.clone();
        let models = Models::new(
            manager,
            catalog,
            Box::new(move |_, _| Some(Box::new(FakeEngine(engine_loads.clone())))),
            runtime.handle().clone(),
            Arc::new(|_| {}),
        );
        assert_eq!(models.downloaded(), vec![ModelId::SystemSpeech]);

        let mut engine = models.engine(ModelId::SystemSpeech).expect("engine");
        let mut steps = Vec::new();
        assert_eq!(engine.load(&mut |step| steps.push(step)), Ok(()));
        assert_eq!(statuses(&steps), vec!["Compiling Encoder.mlmodelc..."]);
        assert_eq!(backend_loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn manager_events_map_to_core_updates() {
        let cases = [
            (DownloadEventKind::Started, DownloadUpdateKind::Started),
            (
                DownloadEventKind::Progress(0.5),
                DownloadUpdateKind::Progress(0.5),
            ),
            (DownloadEventKind::Finished, DownloadUpdateKind::Finished),
            (DownloadEventKind::Cancelled, DownloadUpdateKind::Cancelled),
            (
                DownloadEventKind::Failed {
                    error: "Greendale is offline".to_owned(),
                },
                DownloadUpdateKind::Failed {
                    error: "Greendale is offline".to_owned(),
                },
            ),
        ];
        for (kind, want) in cases {
            let got = download_update(DownloadEvent {
                model: ModelId::ParakeetTdtV2,
                kind,
            });
            assert_eq!(got.kind, want);
            assert_eq!(got.model, ModelId::ParakeetTdtV2);
        }
    }
}
