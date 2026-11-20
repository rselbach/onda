//! The speech model behind one serialized gate (ARCH-8, ASR-14) with a
//! single-flight load (ARCH-9) and the ASR-1..ASR-10 load lifecycle.
//!
//! The gate is a fair (FIFO) async mutex around the loaded engine. A gated
//! call moves the lock guard into its blocking closure, so a caller that
//! gives up (a cancelled session) frees nothing early: the engine call runs
//! to completion holding the gate, and its result is simply never read.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use onda_asr::{AsrError, LoadProgress, SpeechEngine};
use onda_domain::languages::Language;
use onda_domain::models::{ModelError, ModelId};
use onda_domain::non_dictation_ui::DEFAULT_LOADING_STATUS;
use onda_domain::types::ModelState;
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;

use crate::seams::ModelStore;

/// Waiting for another in-flight load gives up after this long (ASR-5).
pub(crate) const LOAD_WAIT_TIMEOUT: Duration = Duration::from_secs(600);

/// First loading status (ASR-2).
const PREPARING_STATUS: &str = "Preparing model...";

/// Why a load did not leave the desired model ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoadError {
    /// The load failed; the message is published as `error` (ASR-6).
    Failed(String),
    /// An unload cancelled it; the unload published `not_ready` (ASR-7).
    Cancelled,
    /// A newer selection superseded it; nothing is published (ASR-7).
    Stale,
}

pub(crate) type StatePublisher = Arc<dyn Fn(ModelState) + Send + Sync>;

#[derive(Default)]
struct Slot {
    model: Option<ModelId>,
    engine: Option<Box<dyn SpeechEngine>>,
}

struct Inner {
    gate: Arc<Mutex<Slot>>,
    load_lock: Mutex<()>,
    // serializes staleness checks with state publication so an unload or newer
    // selection cannot be overtaken by a stale load's status updates
    publish_lock: StdMutex<()>,
    // bumped by every unload; a load that sees it change was cancelled
    generation: AtomicU64,
    // bumped by every load or unload; a stale unload that sees it change skips
    // clearing a newer slot
    op_seq: AtomicU64,
    desired: StdMutex<Option<ModelId>>,
    loaded: StdMutex<Option<ModelId>>,
    supports_preview: AtomicBool,
    min_samples: AtomicUsize,
    store: Arc<dyn ModelStore>,
    publish: StatePublisher,
}

/// Handle to the transcription engine. Cheap to clone.
#[derive(Clone)]
pub(crate) struct TranscriptionService {
    inner: Arc<Inner>,
}

impl TranscriptionService {
    pub(crate) fn new(store: Arc<dyn ModelStore>, publish: StatePublisher) -> Self {
        Self {
            inner: Arc::new(Inner {
                gate: Arc::new(Mutex::new(Slot::default())),
                load_lock: Mutex::new(()),
                publish_lock: StdMutex::new(()),
                generation: AtomicU64::new(0),
                op_seq: AtomicU64::new(0),
                desired: StdMutex::new(None),
                loaded: StdMutex::new(None),
                supports_preview: AtomicBool::new(false),
                min_samples: AtomicUsize::new(0),
                store,
                publish,
            }),
        }
    }

    /// The model the next load should leave ready.
    pub(crate) fn set_desired(&self, model: ModelId) {
        *lock(&self.inner.desired) = Some(model);
    }

    /// The model actually loaded and ready (HIS-3 `modelID`).
    pub(crate) fn loaded_model(&self) -> Option<ModelId> {
        *lock(&self.inner.loaded)
    }

    /// Whether the loaded model streams a live preview.
    pub(crate) fn supports_preview(&self) -> bool {
        self.inner.supports_preview.load(Ordering::SeqCst)
    }

    /// The loaded engine's shortest decodable audio, in samples; 0 when
    /// nothing is loaded or the engine has no minimum.
    pub(crate) fn min_samples(&self) -> usize {
        self.inner.min_samples.load(Ordering::SeqCst)
    }

    /// Reserves a load (and optional leading unload) synchronously on the
    /// caller's thread so stale unloads cannot overtake it, returning the
    /// future that runs it.
    pub(crate) fn prepare_load(
        &self,
        unload_first: bool,
    ) -> impl Future<Output = Result<(), LoadError>> + Send + 'static {
        let (start_generation, start_seq, wait_model) = {
            let _pub = lock(&self.inner.publish_lock);
            if unload_first {
                self.inner.generation.fetch_add(1, Ordering::SeqCst);
            }
            let generation = self.inner.generation.load(Ordering::SeqCst);
            let seq = self.inner.op_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let desired = *lock(&self.inner.desired);
            (generation, seq, desired)
        };
        let service = self.clone();
        async move {
            if unload_first {
                service.finish_unload(start_generation, start_seq).await;
            }
            service.ensure_loaded_at(start_generation, wait_model).await
        }
    }

    /// Leaves the desired model loaded and ready (ASR-2, ASR-3, ARCH-9).
    ///
    /// Concurrent callers wait for the in-flight load (up to
    /// [`LOAD_WAIT_TIMEOUT`]) and then re-read the desired model, so only
    /// one load runs per selection. A load that finishes after an unload or
    /// a newer selection unloads what it just loaded.
    #[cfg(test)]
    pub(crate) async fn ensure_loaded(&self) -> Result<(), LoadError> {
        self.prepare_load(false).await
    }

    async fn ensure_loaded_at(
        &self,
        start_generation: u64,
        wait_model: Option<ModelId>,
    ) -> Result<(), LoadError> {
        let inner = &self.inner;
        let Ok(_load) = tokio::time::timeout(LOAD_WAIT_TIMEOUT, inner.load_lock.lock()).await
        else {
            let message = "Model loading timed out".to_owned();
            return Err(match wait_model {
                Some(model) => self.fail_if_current(start_generation, model, message),
                None => self.stale_error(start_generation),
            });
        };
        let (generation, model) = {
            let _pub = lock(&inner.publish_lock);
            let generation = inner.generation.load(Ordering::SeqCst);
            if generation != start_generation {
                return Err(LoadError::Cancelled);
            }
            let Some(model) = *lock(&inner.desired) else {
                return Err(LoadError::Stale);
            };
            (generation, model)
        };

        let mut slot = inner.gate.clone().lock_owned().await;
        if slot.model == Some(model) && slot.engine.is_some() {
            if self.is_stale(generation, model) {
                return Err(self.stale_error(generation));
            }
            return Ok(());
        }
        if slot.engine.is_some() {
            // a different model goes before the new one loads, so two are
            // never resident at once (ASR-3)
            let service = self.clone();
            blocking(move || {
                if let Some(engine) = slot.engine.as_mut() {
                    engine.unload();
                }
                *slot = Slot::default();
                service.mark_unloaded();
                let _pub = lock(&service.inner.publish_lock);
                if service.inner.generation.load(Ordering::SeqCst) == generation {
                    (service.inner.publish)(ModelState::NotReady);
                }
            })
            .await;
        } else {
            drop(slot);
        }
        if !self.publish_if_current(
            generation,
            model,
            ModelState::Loading {
                status: PREPARING_STATUS.to_owned(),
                progress: None,
            },
        ) {
            return Err(self.stale_error(generation));
        }

        let store = inner.store.clone();
        let downloaded = blocking(move || store.downloaded()).await;
        if !downloaded.contains(&model) {
            let message = ModelError::NotFound {
                model_id: model.as_str().to_owned(),
            }
            .to_string();
            return Err(self.fail_if_current(generation, model, message));
        }
        let store = inner.store.clone();
        let Some(mut engine) = blocking(move || store.engine(model)).await else {
            return Err(self.fail_if_current(
                generation,
                model,
                format!("No speech engine runs {model}."),
            ));
        };
        if !self.publish_if_current(
            generation,
            model,
            ModelState::Loading {
                status: DEFAULT_LOADING_STATUS.to_owned(),
                progress: None,
            },
        ) {
            return Err(self.stale_error(generation));
        }

        let progress_service = self.clone();
        let loaded = spawn_blocking(move || {
            let mut report = |progress: LoadProgress| {
                let status = match progress.status.trim() {
                    "" => DEFAULT_LOADING_STATUS.to_owned(),
                    status => status.to_owned(),
                };
                let progress = progress
                    .fraction
                    .is_finite()
                    .then(|| progress.fraction.clamp(0.0, 1.0));
                progress_service.publish_if_current(
                    generation,
                    model,
                    ModelState::Loading { status, progress },
                );
            };
            let result = engine.load(&mut report);
            (engine, result)
        })
        .await;
        let (engine, result) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => return Err(self.fail_if_current(generation, model, error.to_string())),
        };
        if let Err(error) = result {
            tracing::error!(model = model.as_str(), ?error, "model load failed");
            if error == AsrError::Cancelled {
                // ASR-7: the platform cancelled the load; not an error
                if self.publish_if_current(generation, model, ModelState::NotReady) {
                    return Err(LoadError::Cancelled);
                }
                return Err(self.stale_error(generation));
            }
            return Err(self.fail_if_current(generation, model, error.to_string()));
        }

        let mut slot = inner.gate.clone().lock_owned().await;
        let supports_preview = engine.supports_preview();
        let min_samples = engine.min_samples();
        // an unload bumps the generation before it queues on the gate, so
        // holding both the gate and publish_lock settles which of us goes last
        let stale_engine = {
            let _pub = lock(&inner.publish_lock);
            if self.is_stale(generation, model) {
                Some(engine)
            } else {
                slot.engine = Some(engine);
                slot.model = Some(model);
                *lock(&inner.loaded) = Some(model);
                inner
                    .supports_preview
                    .store(supports_preview, Ordering::SeqCst);
                inner.min_samples.store(min_samples, Ordering::SeqCst);
                (inner.publish)(ModelState::Ready);
                None
            }
        };
        if let Some(mut engine) = stale_engine {
            drop(slot);
            blocking(move || engine.unload()).await;
            return Err(self.stale_error(generation));
        }
        drop(slot);
        Ok(())
    }

    /// Reserves an unload synchronously on the caller's thread so in-flight
    /// loads are cancelled at once, returning the future that releases the
    /// engine after any in-flight call.
    pub(crate) fn prepare_unload(&self) -> impl Future<Output = ()> + Send + 'static {
        let (unload_generation, unload_seq) = {
            let _pub = lock(&self.inner.publish_lock);
            let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let seq = self.inner.op_seq.fetch_add(1, Ordering::SeqCst) + 1;
            (generation, seq)
        };
        let service = self.clone();
        async move {
            service.finish_unload(unload_generation, unload_seq).await;
        }
    }

    /// Unloads the model after any in-flight engine call and cancels any
    /// in-flight load; publishes `not_ready` (ASR-7, ASR-8).
    #[cfg(test)]
    pub(crate) async fn unload(&self) {
        self.prepare_unload().await;
    }

    async fn finish_unload(&self, unload_generation: u64, unload_seq: u64) {
        let mut slot = self.inner.gate.clone().lock_owned().await;
        let service = self.clone();
        blocking(move || {
            {
                let _pub = lock(&service.inner.publish_lock);
                if service.inner.op_seq.load(Ordering::SeqCst) != unload_seq {
                    return;
                }
            }
            if let Some(engine) = slot.engine.as_mut() {
                engine.unload();
            }
            *slot = Slot::default();
            service.mark_unloaded();
            let _pub = lock(&service.inner.publish_lock);
            if service.inner.generation.load(Ordering::SeqCst) == unload_generation
                && service.inner.op_seq.load(Ordering::SeqCst) == unload_seq
            {
                (service.inner.publish)(ModelState::NotReady);
            }
        })
        .await;
    }

    /// One live-preview decode of the recording so far (ASR-15..ASR-17).
    pub(crate) async fn preview(
        &self,
        samples: Vec<f32>,
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        self.run(move |engine| engine.preview(&samples, language))
            .await
    }

    /// The final transcript of a whole recording (ASR-20).
    pub(crate) async fn transcribe(
        &self,
        samples: Arc<[f32]>,
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        self.run(move |engine| engine.transcribe(&samples, language))
            .await
    }

    /// Starts a new preview stream (ASR-20). Errors are ignored: a model
    /// that is not ready has no stream.
    pub(crate) async fn reset_preview(&self) {
        let _ = self
            .run(|engine| {
                engine.reset_preview();
                Ok(())
            })
            .await;
    }

    async fn run<T: Send + 'static>(
        &self,
        op: impl FnOnce(&mut dyn SpeechEngine) -> Result<T, AsrError> + Send + 'static,
    ) -> Result<T, AsrError> {
        let mut slot = self.inner.gate.clone().lock_owned().await;
        spawn_blocking(move || match slot.engine.as_deref_mut() {
            Some(engine) => op(engine),
            None => Err(AsrError::NotReady),
        })
        .await
        .unwrap_or_else(|error| {
            Err(AsrError::TranscriptionFailed {
                detail: error.to_string(),
            })
        })
    }

    fn is_stale(&self, generation: u64, model: ModelId) -> bool {
        self.inner.generation.load(Ordering::SeqCst) != generation
            || *lock(&self.inner.desired) != Some(model)
    }

    fn publish_if_current(&self, generation: u64, model: ModelId, state: ModelState) -> bool {
        let _pub = lock(&self.inner.publish_lock);
        if self.is_stale(generation, model) {
            return false;
        }
        (self.inner.publish)(state);
        true
    }

    fn fail_if_current(&self, generation: u64, model: ModelId, message: String) -> LoadError {
        if self.publish_if_current(
            generation,
            model,
            ModelState::Error {
                message: message.clone(),
            },
        ) {
            LoadError::Failed(message)
        } else {
            self.stale_error(generation)
        }
    }

    fn stale_error(&self, generation: u64) -> LoadError {
        if self.inner.generation.load(Ordering::SeqCst) == generation {
            LoadError::Stale
        } else {
            LoadError::Cancelled
        }
    }

    fn mark_unloaded(&self) {
        *lock(&self.inner.loaded) = None;
        self.inner.supports_preview.store(false, Ordering::SeqCst);
        self.inner.min_samples.store(0, Ordering::SeqCst);
    }
}

fn lock<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// runs `f` on the blocking pool; a panic there is a bug in an engine, which
// is logged and otherwise ignored
async fn blocking<T: Default + Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match spawn_blocking(f).await {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "engine task failed");
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[derive(Default)]
    struct Counts {
        loads: AtomicU64,
        unloads: AtomicU64,
        active: AtomicU64,
        max_active: AtomicU64,
        order: StdMutex<Vec<&'static str>>,
    }

    struct Hold {
        started: mpsc::Sender<()>,
        release: StdMutex<mpsc::Receiver<()>>,
    }

    impl Hold {
        fn new() -> (Arc<Hold>, mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (started, started_rx) = mpsc::channel();
            let (release_tx, release) = mpsc::channel();
            let hold = Arc::new(Hold {
                started,
                release: StdMutex::new(release),
            });
            (hold, started_rx, release_tx)
        }

        fn wait(&self) {
            let _ = self.started.send(());
            let _ = lock(&self.release).recv();
        }
    }

    struct Engine {
        counts: Arc<Counts>,
        hold_load: Option<Arc<Hold>>,
        hold_transcribe: Option<Arc<Hold>>,
        fail_load: bool,
        nan_progress: bool,
        preview: bool,
        loaded: bool,
    }

    impl SpeechEngine for Engine {
        fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
            self.counts.loads.fetch_add(1, Ordering::SeqCst);
            if let Some(hold) = &self.hold_load {
                hold.wait();
            }
            if self.fail_load {
                return Err(AsrError::LoadFailed {
                    detail: "Greendale weights missing".to_owned(),
                });
            }
            progress(LoadProgress {
                status: String::new(),
                fraction: if self.nan_progress { f64::NAN } else { 1.5 },
            });
            self.loaded = true;
            Ok(())
        }

        fn unload(&mut self) {
            self.counts.unloads.fetch_add(1, Ordering::SeqCst);
            self.loaded = false;
        }

        fn supports_preview(&self) -> bool {
            self.preview
        }

        // the TDT fake has Parakeet's floor; the unified fake has none
        fn min_samples(&self) -> usize {
            if self.preview { 4_800 } else { 0 }
        }

        fn transcribe(&mut self, _: &[f32], _: Option<Language>) -> Result<String, AsrError> {
            if let Some(hold) = &self.hold_transcribe {
                hold.wait();
            }
            Ok("Troy and Abed in the morning".to_owned())
        }

        fn preview(&mut self, _: &[f32], _: Option<Language>) -> Result<String, AsrError> {
            Ok(String::new())
        }

        fn reset_preview(&mut self) {}
    }

    #[derive(Default)]
    struct Store {
        counts: StdMutex<Vec<(ModelId, Arc<Counts>)>>,
        downloaded: Vec<ModelId>,
        hold_load: StdMutex<Option<Arc<Hold>>>,
        hold_transcribe: StdMutex<Option<Arc<Hold>>>,
        fail_load: bool,
        nan_progress: bool,
    }

    impl Store {
        fn with(downloaded: &[ModelId]) -> Self {
            Self {
                downloaded: downloaded.to_vec(),
                ..Self::default()
            }
        }

        fn counts(&self, model: ModelId) -> Vec<Arc<Counts>> {
            lock(&self.counts)
                .iter()
                .filter(|(m, _)| *m == model)
                .map(|(_, c)| c.clone())
                .collect()
        }

        fn total(&self, model: ModelId, pick: fn(&Counts) -> &AtomicU64) -> u64 {
            self.counts(model)
                .iter()
                .map(|c| pick(c).load(Ordering::SeqCst))
                .sum()
        }
    }

    impl ModelStore for Store {
        fn downloaded(&self) -> Vec<ModelId> {
            self.downloaded.clone()
        }

        fn engine(&self, model: ModelId) -> Option<Box<dyn SpeechEngine>> {
            let counts = Arc::new(Counts::default());
            lock(&self.counts).push((model, counts.clone()));
            Some(Box::new(Engine {
                counts,
                hold_load: lock(&self.hold_load).take(),
                hold_transcribe: lock(&self.hold_transcribe).take(),
                fail_load: self.fail_load,
                nan_progress: self.nan_progress,
                preview: model == ModelId::ParakeetTdtV2,
                loaded: false,
            }))
        }

        fn start_download(&self, _: ModelId) {}

        fn cancel_download(&self, _: ModelId) {}

        fn delete(&self, _: ModelId) -> Result<(), String> {
            Ok(())
        }

        fn redownload(&self, _: ModelId) {}

        fn remove_obsolete(&self) {}
    }

    type States = Arc<StdMutex<Vec<ModelState>>>;

    fn service(store: Arc<Store>) -> (TranscriptionService, States) {
        let states: States = Arc::default();
        let sink = states.clone();
        let service =
            TranscriptionService::new(store, Arc::new(move |state| lock(&sink).push(state)));
        (service, states)
    }

    async fn eventually(what: &str, cond: impl Fn() -> bool) {
        for _ in 0..2_000 {
            if cond() {
                return;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("timed out waiting for {what}");
    }

    const V2: ModelId = ModelId::ParakeetTdtV2;
    const UNIFIED: ModelId = ModelId::ParakeetUnified;

    #[tokio::test]
    async fn asr_2_publishes_preparing_loading_progress_then_ready() {
        let store = Arc::new(Store::with(&[V2]));
        let (service, states) = service(store);
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        let loading = |status: &str, progress| ModelState::Loading {
            status: status.to_owned(),
            progress,
        };
        let want = vec![
            loading(PREPARING_STATUS, None),
            loading(DEFAULT_LOADING_STATUS, None),
            loading(DEFAULT_LOADING_STATUS, Some(1.0)),
            ModelState::Ready,
        ];
        assert_eq!(*lock(&states), want);
        assert_eq!(service.loaded_model(), Some(V2));
    }

    #[tokio::test]
    async fn asr_3_loading_the_ready_model_again_is_a_no_op() {
        let store = Arc::new(Store::with(&[V2]));
        let (service, states) = service(store.clone());
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        let published = lock(&states).len();
        assert_eq!(service.ensure_loaded().await, Ok(()));
        assert_eq!(lock(&states).len(), published);
        assert_eq!(store.total(V2, |c| &c.loads), 1);
    }

    #[tokio::test]
    async fn asr_6_load_failures_publish_an_error() {
        struct Case {
            name: &'static str,
            store: Store,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "files not on disk",
                store: Store::with(&[]),
                want: "Model 'parakeet-tdt-v2' not found or already deleted.",
            },
            Case {
                name: "engine load fails",
                store: Store {
                    fail_load: true,
                    ..Store::with(&[V2])
                },
                want: "Failed to load the transcription model",
            },
        ];
        for tc in cases {
            let (service, states) = service(Arc::new(tc.store));
            service.set_desired(V2);
            assert_eq!(
                service.ensure_loaded().await,
                Err(LoadError::Failed(tc.want.to_owned())),
                "{}",
                tc.name
            );
            assert_eq!(
                lock(&states).last(),
                Some(&ModelState::Error {
                    message: tc.want.to_owned()
                }),
                "{}",
                tc.name
            );
            assert_eq!(service.loaded_model(), None, "{}", tc.name);
        }
    }

    // Jabber: testConcurrentLoadModelCallersShareSingleProviderLoad
    #[tokio::test(flavor = "multi_thread")]
    async fn arch_9_concurrent_loads_share_one_load() {
        let store = Arc::new(Store::with(&[V2]));
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_load) = Some(hold);
        let (service, _) = service(store.clone());
        service.set_desired(V2);
        let first = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        let _ = started.recv();
        let second = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(store.total(V2, |c| &c.loads), 1);
        let _ = release.send(());
        assert_eq!(first.await.ok(), Some(Ok(())));
        assert_eq!(second.await.ok(), Some(Ok(())));
        assert_eq!(store.total(V2, |c| &c.loads), 1);
        assert_eq!(store.total(V2, |c| &c.unloads), 0);
        assert_eq!(service.loaded_model(), Some(V2));
    }

    // Jabber: testUnloadDuringLoadCancelsGenerationAndUnloadsNewProvider
    #[tokio::test(flavor = "multi_thread")]
    async fn asr_7_unload_during_load_unloads_the_fresh_engine() {
        let store = Arc::new(Store::with(&[V2]));
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_load) = Some(hold);
        let (service, states) = service(store.clone());
        service.set_desired(V2);
        let load = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        let _ = started.recv();
        service.unload().await;
        let _ = release.send(());
        assert_eq!(load.await.ok(), Some(Err(LoadError::Cancelled)));
        assert_eq!(store.total(V2, |c| &c.loads), 1);
        assert_eq!(store.total(V2, |c| &c.unloads), 1);
        assert_eq!(service.loaded_model(), None);
        assert_eq!(lock(&states).last(), Some(&ModelState::NotReady));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn arch_9_a_load_superseded_by_a_newer_selection_reports_nothing() {
        let store = Arc::new(Store::with(&[V2, UNIFIED]));
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_load) = Some(hold);
        let (service, states) = service(store.clone());
        service.set_desired(V2);
        let first = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        let _ = started.recv();
        service.set_desired(UNIFIED);
        let second = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        let _ = release.send(());
        assert_eq!(first.await.ok(), Some(Err(LoadError::Stale)));
        assert_eq!(second.await.ok(), Some(Ok(())));
        assert_eq!(store.total(V2, |c| &c.unloads), 1);
        assert_eq!(service.loaded_model(), Some(UNIFIED));
        assert_eq!(lock(&states).last(), Some(&ModelState::Ready));
        assert!(!service.supports_preview(), "unified fake has no preview");
    }

    // Jabber: testModelSwitchUnloadSuspensionDoesNotStartDuplicateLoad
    #[tokio::test(flavor = "multi_thread")]
    async fn asr_3_switch_unloads_after_the_in_flight_call_then_loads_once() {
        let store = Arc::new(Store::with(&[V2, UNIFIED]));
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_transcribe) = Some(hold);
        let (service, _) = service(store.clone());
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        let decode = tokio::spawn({
            let service = service.clone();
            async move { service.transcribe(Arc::from(vec![0.0; 16]), None).await }
        });
        let _ = started.recv();
        service.set_desired(UNIFIED);
        let loads: Vec<_> = (0..2)
            .map(|_| {
                let service = service.clone();
                tokio::spawn(async move { service.ensure_loaded().await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(store.total(V2, |c| &c.unloads), 0, "waits for the decode");
        let _ = release.send(());
        assert!(matches!(decode.await, Ok(Ok(_))));
        for load in loads {
            assert_eq!(load.await.ok(), Some(Ok(())));
        }
        assert_eq!(store.total(V2, |c| &c.unloads), 1);
        assert_eq!(store.total(UNIFIED, |c| &c.loads), 1);
        assert_eq!(store.total(UNIFIED, |c| &c.unloads), 0);
        assert_eq!(service.loaded_model(), Some(UNIFIED));
    }

    // Jabber: testProviderCallGateSerializesConcurrentOperations
    #[tokio::test(flavor = "multi_thread")]
    async fn arch_8_gate_runs_engine_calls_one_at_a_time_in_order() {
        let store = Arc::new(Store::with(&[V2]));
        let (service, _) = service(store);
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        let counts = Arc::new(Counts::default());
        let (hold, started, release) = Hold::new();
        let call = |name: &'static str, hold: Option<Arc<Hold>>| {
            let service = service.clone();
            let counts = counts.clone();
            tokio::spawn(async move {
                service
                    .run(move |_| {
                        let active = counts.active.fetch_add(1, Ordering::SeqCst) + 1;
                        counts.max_active.fetch_max(active, Ordering::SeqCst);
                        lock(&counts.order).push(name);
                        if let Some(hold) = hold {
                            hold.wait();
                        }
                        counts.active.fetch_sub(1, Ordering::SeqCst);
                        Ok(name)
                    })
                    .await
            })
        };
        let first = call("first", Some(hold));
        let _ = started.recv();
        let second = call("second", None);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(*lock(&counts.order), vec!["first"]);
        let _ = release.send(());
        assert_eq!(first.await.ok(), Some(Ok("first")));
        assert_eq!(second.await.ok(), Some(Ok("second")));
        assert_eq!(counts.max_active.load(Ordering::SeqCst), 1);
        assert_eq!(*lock(&counts.order), vec!["first", "second"]);
    }

    // Jabber: testStreamingCapabilityTracksLoadedProvider
    #[tokio::test]
    async fn preview_support_tracks_the_loaded_engine() {
        let (service, _) = service(Arc::new(Store::with(&[V2])));
        assert!(!service.supports_preview());
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        assert!(service.supports_preview());
        service.unload().await;
        assert!(!service.supports_preview());
        assert_eq!(
            service.transcribe(Arc::from(vec![0.0; 16]), None).await,
            Err(AsrError::NotReady)
        );
    }

    #[tokio::test]
    async fn readiness_publication_cannot_be_overtaken_by_engine_calls() {
        let inner = Arc::new(std::sync::OnceLock::<std::sync::Weak<Inner>>::new());
        let snapshots = Arc::new(StdMutex::new(Vec::new()));
        let publish = {
            let inner = inner.clone();
            let snapshots = snapshots.clone();
            Arc::new(move |state| {
                if !matches!(state, ModelState::Ready | ModelState::NotReady) {
                    return;
                }
                let inner = inner
                    .get()
                    .expect("service initialized")
                    .upgrade()
                    .expect("service alive");
                let serialized = inner.gate.try_lock().is_err();
                lock(&snapshots).push((
                    state,
                    serialized,
                    *lock(&inner.loaded),
                    inner.supports_preview.load(Ordering::SeqCst),
                    inner.min_samples.load(Ordering::SeqCst),
                ));
            })
        };
        let service = TranscriptionService::new(Arc::new(Store::with(&[V2, UNIFIED])), publish);
        inner
            .set(Arc::downgrade(&service.inner))
            .expect("set service");
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        service.set_desired(UNIFIED);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        service.unload().await;
        assert_eq!(
            *lock(&snapshots),
            vec![
                (ModelState::Ready, true, Some(V2), true, 4_800),
                (ModelState::NotReady, true, None, false, 0),
                (ModelState::Ready, true, Some(UNIFIED), false, 0),
                (ModelState::NotReady, true, None, false, 0),
            ]
        );
    }

    #[tokio::test]
    async fn min_samples_tracks_the_loaded_engine_gate_2() {
        let (service, _) = service(Arc::new(Store::with(&[V2, UNIFIED])));
        assert_eq!(service.min_samples(), 0, "nothing loaded");
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        assert_eq!(service.min_samples(), 4_800);
        service.set_desired(UNIFIED);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        assert_eq!(service.min_samples(), 0);
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        service.unload().await;
        assert_eq!(service.min_samples(), 0, "unloaded");
    }

    #[tokio::test(start_paused = true)]
    async fn asr_5_waiting_for_an_in_flight_load_times_out_after_600_s() {
        let store = Arc::new(Store::with(&[V2]));
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_load) = Some(hold);
        let (service, _) = service(store);
        service.set_desired(V2);
        let first = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        eventually("the first load", || started.try_recv().is_ok()).await;
        let second = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        tokio::task::yield_now().await;
        tokio::time::advance(LOAD_WAIT_TIMEOUT + Duration::from_millis(1)).await;
        eventually("the timeout", || second.is_finished()).await;
        assert_eq!(
            second.await.ok(),
            Some(Err(LoadError::Failed("Model loading timed out".to_owned())))
        );
        let _ = release.send(());
        assert_eq!(first.await.ok(), Some(Ok(())));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unload_during_load_error_does_not_overwrite_not_ready() {
        let store = Arc::new(Store {
            fail_load: true,
            ..Store::with(&[V2])
        });
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_load) = Some(hold);
        let (service, states) = service(store);
        service.set_desired(V2);
        let load = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        let _ = started.recv();
        service.unload().await;
        let _ = release.send(());
        assert_eq!(load.await.ok(), Some(Err(LoadError::Cancelled)));
        let published = lock(&states);
        assert_eq!(published.last(), Some(&ModelState::NotReady));
        assert!(
            !published
                .iter()
                .any(|state| matches!(state, ModelState::Error { .. })),
            "stale load failure must not overwrite NotReady: {published:?}"
        );
    }

    #[tokio::test]
    async fn non_finite_load_progress_omits_fraction() {
        let store = Arc::new(Store {
            nan_progress: true,
            ..Store::with(&[V2])
        });
        let (service, states) = service(store);
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));
        let published = lock(&states);
        assert!(published.iter().all(|state| match state {
            ModelState::Loading {
                progress: Some(fraction),
                ..
            } => fraction.is_finite(),
            _ => true,
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_unload_does_not_wipe_a_newer_loaded_engine() {
        let store = Arc::new(Store::with(&[V2, UNIFIED]));
        let (service, states) = service(store.clone());
        service.set_desired(V2);
        assert_eq!(service.ensure_loaded().await, Ok(()));

        // Reserve an unload, then supersede it with a newer load before the
        // stale unload runs its gate acquisition.
        let stale_unload = service.prepare_unload();
        service.set_desired(UNIFIED);
        let newer_load = service.prepare_load(true);
        assert_eq!(newer_load.await, Ok(()));
        assert_eq!(service.loaded_model(), Some(UNIFIED));

        // Running the stale unload now must leave the newer engine intact.
        stale_unload.await;
        assert_eq!(service.loaded_model(), Some(UNIFIED));
        assert_eq!(lock(&states).last(), Some(&ModelState::Ready));
        assert_eq!(
            service.transcribe(Arc::from(vec![0.0; 16]), None).await,
            Ok("Troy and Abed in the morning".to_owned())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn load_wait_timeout_does_not_publish_error_when_stale() {
        let store = Arc::new(Store::with(&[V2, UNIFIED]));
        let (hold, started, release) = Hold::new();
        *lock(&store.hold_load) = Some(hold);
        let (service, states) = service(store);
        service.set_desired(V2);
        let first = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        eventually("the first load", || started.try_recv().is_ok()).await;
        let waiter = tokio::spawn({
            let service = service.clone();
            async move { service.ensure_loaded().await }
        });
        tokio::task::yield_now().await;

        // Change desired model before the waiter times out.
        service.set_desired(UNIFIED);
        tokio::time::advance(LOAD_WAIT_TIMEOUT + Duration::from_millis(1)).await;
        eventually("the stale timeout", || waiter.is_finished()).await;
        assert_eq!(waiter.await.ok(), Some(Err(LoadError::Stale)));
        assert!(
            !lock(&states)
                .iter()
                .any(|state| matches!(state, ModelState::Error { .. })),
            "stale timeout must not publish Error"
        );
        let _ = release.send(());
        assert_eq!(first.await.ok(), Some(Err(LoadError::Stale)));
    }
}
