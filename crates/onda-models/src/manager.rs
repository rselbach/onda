use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use onda_asr::{ManifestFile, ModelManifest};
use onda_domain::download_progress::ProgressThrottle;
use onda_domain::migration::removed_download_folders;
use onda_domain::models::{FolderValidation, ModelError, ModelId};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::error::{Error, describe};
use crate::event::{DownloadEvent, DownloadEventKind};
use crate::integrity::{mark_verified, sha256_file, verify_folder};
use crate::remove::remove_descendant;
use crate::validate::validate_folder;

/// Retries per file after the first attempt (DL-4 "the downloader's own
/// retries").
const RETRIES: u32 = 3;
/// First retry delay; doubles on every retry.
const RETRY_DELAY: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A stalled transfer fails (and retries) after this long without data.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Suffix of a file still downloading; renamed away once verified.
const PART_SUFFIX: &str = ".part";

type ManifestLookup = Box<dyn Fn(ModelId) -> Option<&'static ModelManifest> + Send + Sync>;
/// `None` while the download runs.
type Outcome = Option<Result<PathBuf, Error>>;

/// Downloads, validates, and deletes model folders under one models
/// directory. Cheap to clone; clones share downloads and the event channel.
#[derive(Clone)]
pub struct ModelManager {
    inner: Arc<Inner>,
}

struct Inner {
    models_dir: PathBuf,
    manifests: ManifestLookup,
    client: reqwest::Client,
    events: mpsc::UnboundedSender<DownloadEvent>,
    active: Mutex<HashMap<ModelId, Active>>,
    retry_delay: Duration,
}

struct Active {
    token: CancellationToken,
    outcome: watch::Receiver<Outcome>,
}

impl ModelManager {
    /// Creates a manager for `models_dir` (normally `<data>/Models`) and the
    /// receiver of its download events. `manifests` returns the files of a
    /// model, or `None` for models that download nothing; the speech backend
    /// owns it (MOD-1). Touches neither the disk nor the network.
    pub fn new(
        models_dir: impl Into<PathBuf>,
        manifests: impl Fn(ModelId) -> Option<&'static ModelManifest> + Send + Sync + 'static,
    ) -> Result<(ModelManager, mpsc::UnboundedReceiver<DownloadEvent>), Error> {
        ModelManager::build(models_dir.into(), Box::new(manifests), RETRY_DELAY)
    }

    pub(crate) fn build(
        models_dir: PathBuf,
        manifests: ManifestLookup,
        retry_delay: Duration,
    ) -> Result<(ModelManager, mpsc::UnboundedReceiver<DownloadEvent>), Error> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Onda/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .map_err(|e| {
                Error::Io(format!(
                    "Could not create the HTTP client: {}",
                    describe(&e)
                ))
            })?;
        let (events, receiver) = mpsc::unbounded_channel();
        let inner = Inner {
            models_dir,
            manifests,
            client,
            events,
            active: Mutex::new(HashMap::new()),
            retry_delay,
        };
        Ok((
            ModelManager {
                inner: Arc::new(inner),
            },
            receiver,
        ))
    }

    /// The models directory. The app passes it to the shell's `Files` port to
    /// exclude it from backups.
    pub fn models_dir(&self) -> &Path {
        &self.inner.models_dir
    }

    /// Creates the models directory if needed, so it can be excluded from
    /// backups before the first download.
    pub fn create_models_dir(&self) -> Result<(), Error> {
        std::fs::create_dir_all(self.models_dir()).map_err(|e| {
            Error::Io(format!(
                "Could not create {}: {}",
                self.models_dir().display(),
                describe(&e)
            ))
        })
    }

    /// The folder of `model`: `<models_dir>/<model-id>`.
    pub fn model_dir(&self, model: ModelId) -> PathBuf {
        self.inner.model_dir(model)
    }

    /// Validates the folder of `model` against its manifest (DL-5); `None`
    /// for models that download nothing.
    pub fn validate(&self, model: ModelId) -> Option<FolderValidation> {
        let manifest = (self.inner.manifests)(model)?;
        Some(validate_folder(&self.model_dir(model), manifest))
    }

    /// Verifies the content of `model`'s folder against its manifest hashes
    /// before an engine reads it (ASR-6), unless the folder carries a
    /// current verification marker. Blocking; run it off the async runtime.
    /// `progress` receives the hashed share of the bytes while hashing. Models
    /// that download nothing pass.
    ///
    /// Fails with [`ModelError::IncompleteInstallation`] when a file is
    /// missing, corrupt, or unreadable, so the user is offered "Re-download
    /// Model" (DL-11).
    pub fn verify(&self, model: ModelId, progress: &mut dyn FnMut(f64)) -> Result<(), Error> {
        let Some(manifest) = (self.inner.manifests)(model) else {
            return Ok(());
        };
        verify_folder(&self.model_dir(model), manifest, progress).map_err(|details| {
            tracing::error!(%model, %details, "model failed verification");
            ModelError::IncompleteInstallation {
                model_id: model.as_str().to_string(),
                details,
            }
            .into()
        })
    }

    /// Models whose folders pass validation, in catalog order (DL-5).
    /// Incomplete folders are logged and left out. Built-in engines are not
    /// included; `Catalog::downloaded` adds them.
    pub fn downloaded_models(&self) -> Vec<ModelId> {
        ModelId::ALL
            .into_iter()
            .filter(|&model| {
                let Some(validation) = self.validate(model) else {
                    return false;
                };
                if let (true, Some(details)) =
                    (validation.folder_exists, validation.failure_details())
                {
                    tracing::warn!(%model, %details, "ignoring incomplete model folder");
                }
                validation.is_complete()
            })
            .collect()
    }

    /// Whether `model` has a download task running.
    pub fn is_downloading(&self, model: ModelId) -> bool {
        self.inner.lock_active().contains_key(&model)
    }

    /// Downloads `model` and returns its folder once every file is on disk
    /// and verified (DL-1..DL-4). A model that already passes validation
    /// returns at once. A request for a model already downloading joins that
    /// download. Progress and the outcome are also reported as events.
    ///
    /// Dropping the returned future does not stop the download; use
    /// [`ModelManager::cancel`].
    pub async fn download(&self, model: ModelId) -> Result<PathBuf, Error> {
        let manifest = (self.inner.manifests)(model).ok_or_else(|| ModelError::NotFound {
            model_id: model.as_str().to_string(),
        })?;
        loop {
            let (outcome, joinable) = {
                let mut active = self.inner.lock_active();
                match active.get(&model) {
                    Some(running) => (running.outcome.clone(), !running.token.is_cancelled()),
                    None => {
                        let folder = self.model_dir(model);
                        if validate_folder(&folder, manifest).is_complete() {
                            return Ok(folder);
                        }
                        let token = CancellationToken::new();
                        let (sender, outcome) = watch::channel(None);
                        active.insert(
                            model,
                            Active {
                                token: token.clone(),
                                outcome: outcome.clone(),
                            },
                        );
                        tokio::spawn(Arc::clone(&self.inner).run(model, manifest, token, sender));
                        (outcome, true)
                    }
                }
            };
            let result = wait(outcome).await;
            // a cancelled download still winding down is not joined: wait for
            // it to end, then start a fresh one
            if joinable {
                return result;
            }
        }
    }

    /// Cancels the download of `model`, keeping partial data so the next
    /// attempt resumes (DL-3). Returns whether a download was running.
    pub fn cancel(&self, model: ModelId) -> bool {
        let active = self.inner.lock_active();
        let Some(running) = active.get(&model) else {
            return false;
        };
        tracing::info!(%model, "cancelling download");
        running.token.cancel();
        true
    }

    /// Deletes the folder of `model` (DL-9..DL-11), refusing anything that is
    /// not a strict descendant of the models directory (ARCH-12). Fails with
    /// [`ModelError::NotFound`] when there is no folder.
    pub fn delete(&self, model: ModelId) -> Result<(), Error> {
        if self.is_downloading(model) {
            return Err(Error::Downloading(model));
        }
        let folder = self.model_dir(model);
        match std::fs::symlink_metadata(&folder) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ModelError::NotFound {
                    model_id: model.as_str().to_string(),
                }
                .into());
            }
            Err(error) => {
                return Err(Error::Io(format!(
                    "Could not read {}: {}",
                    folder.display(),
                    describe(&error)
                )));
            }
        }
        remove_descendant(self.models_dir(), &folder)?;
        tracing::info!(%model, "deleted model folder");
        Ok(())
    }

    /// Deletes the folder of `model`, if any, and downloads it again
    /// (DL-11).
    pub async fn redownload(&self, model: ModelId) -> Result<PathBuf, Error> {
        match self.delete(model) {
            Ok(()) | Err(Error::Model(ModelError::NotFound { .. })) => {}
            Err(error) => return Err(error),
        }
        self.download(model).await
    }

    /// Deletes the folders of removed models (DL-15), as selected by
    /// [`removed_download_folders`] from `removed` (normally
    /// `onda_domain::migration::REMOVED_MODEL_IDS`). Failures are logged.
    /// Returns the folder names deleted.
    pub fn remove_obsolete(&self, removed: &[(&str, ModelId)]) -> Vec<String> {
        let entries = match std::fs::read_dir(self.models_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                tracing::warn!(%error, "could not list the models directory");
                return Vec::new();
            }
        };
        let names: Vec<String> = entries
            .filter_map(|entry| match entry {
                Ok(entry) => entry.file_name().into_string().ok(),
                Err(error) => {
                    tracing::warn!(%error, "could not list the models directory");
                    None
                }
            })
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut deleted = Vec::new();
        for name in removed_download_folders(&names, removed) {
            match remove_descendant(self.models_dir(), &self.models_dir().join(name)) {
                Ok(()) => {
                    tracing::info!(folder = name, "deleted folder of a removed model");
                    deleted.push(name.to_string());
                }
                Err(error) => {
                    tracing::error!(folder = name, %error, "could not delete folder of a removed model");
                }
            }
        }
        deleted
    }
}

async fn wait(mut outcome: watch::Receiver<Outcome>) -> Result<PathBuf, Error> {
    match outcome.wait_for(Option::is_some).await {
        Ok(result) => result.clone().unwrap_or(Err(Error::Cancelled)),
        Err(_) => Err(Error::Download(
            "The download stopped unexpectedly.".to_string(),
        )),
    }
}

/// How one HTTP attempt at a file ended badly.
enum Attempt {
    Cancelled,
    /// Transient; the next attempt resumes from the partial file.
    Retry {
        message: String,
        timed_out: bool,
    },
    Fatal(Error),
}

impl Attempt {
    fn retry(error: &reqwest::Error) -> Attempt {
        Attempt::Retry {
            message: describe(error),
            timed_out: error.is_timeout(),
        }
    }
}

fn io_attempt(action: &str, path: &Path, error: &std::io::Error) -> Attempt {
    Attempt::Fatal(Error::Io(format!(
        "Could not {action} {}: {}",
        path.display(),
        describe(error)
    )))
}

impl Inner {
    fn model_dir(&self, model: ModelId) -> PathBuf {
        self.models_dir.join(model.as_str())
    }

    fn lock_active(&self) -> MutexGuard<'_, HashMap<ModelId, Active>> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, model: ModelId, kind: DownloadEventKind) {
        if self.events.send(DownloadEvent { model, kind }).is_err() {
            tracing::debug!(%model, "download event receiver dropped");
        }
    }

    /// The download task: one per model (DL-1).
    async fn run(
        self: Arc<Inner>,
        model: ModelId,
        manifest: &'static ModelManifest,
        token: CancellationToken,
        sender: watch::Sender<Outcome>,
    ) {
        let folder = self.model_dir(model);
        tracing::info!(%model, total_bytes = manifest.total_size(), "download started");
        self.emit(model, DownloadEventKind::Started);
        let mut job = Job {
            inner: &self,
            model,
            token: &token,
            total: manifest.total_size(),
            start: Instant::now(),
            throttle: ProgressThrottle::new(),
            network_gave_out: false,
        };
        let result = job.fetch_all(manifest, &folder).await;
        let network_gave_out = job.network_gave_out;
        let elapsed_ms = job.start.elapsed().as_millis() as u64;
        let (result, event) = match result {
            Ok(()) => {
                tracing::info!(%model, elapsed_ms, "download finished");
                (Ok(folder), DownloadEventKind::Finished)
            }
            Err(Error::Cancelled) => {
                tracing::info!(%model, elapsed_ms, "download cancelled; partial data kept");
                (Err(Error::Cancelled), DownloadEventKind::Cancelled)
            }
            Err(error) => {
                tracing::error!(%model, elapsed_ms, %error, network_gave_out, "download failed");
                // what a dropped connection leaves is a clean prefix the next
                // download resumes; any other failure may have left bad data
                if !network_gave_out
                    && let Err(cleanup) = remove_descendant(&self.models_dir, &folder)
                {
                    tracing::error!(%model, error = %cleanup, "could not delete partial download");
                }
                let event = DownloadEventKind::Failed {
                    error: error.to_string(),
                };
                (Err(error), event)
            }
        };
        // under the lock, so a new download of this model cannot emit its
        // start before this one's end
        {
            let mut active = self.lock_active();
            active.remove(&model);
            self.emit(model, event);
        }
        sender.send_replace(Some(result));
    }
}

/// One running download: its cancellation token and progress over the
/// model's total bytes, fed through the domain throttle (DL-2).
struct Job<'a> {
    inner: &'a Inner,
    model: ModelId,
    token: &'a CancellationToken,
    total: u64,
    start: Instant,
    throttle: ProgressThrottle,
    /// The failure was the network running out of retries, so the partial
    /// data is intact and kept for a resume.
    network_gave_out: bool,
}

impl Job<'_> {
    fn report(&mut self, bytes: u64) {
        let fraction = match self.total {
            0 => 1.0,
            total => bytes as f64 / total as f64,
        };
        if let Some(published) = self.throttle.accept(fraction, self.start.elapsed()) {
            self.inner
                .emit(self.model, DownloadEventKind::Progress(published));
        }
    }

    /// Fetches every file. When this run hashed all of them, marks the
    /// folder verified so the first load skips a second hash; files kept from
    /// an earlier run leave that to the load.
    async fn fetch_all(&mut self, manifest: &ModelManifest, folder: &Path) -> Result<(), Error> {
        let mut done = 0;
        let mut fetched_all = true;
        for file in manifest.files {
            fetched_all &= self.fetch_file(manifest, file, folder, done).await?;
            done += file.size;
            self.report(done);
        }
        if let Some(details) = validate_folder(folder, manifest).failure_details() {
            return Err(ModelError::IncompleteInstallation {
                model_id: self.model.as_str().to_string(),
                details,
            }
            .into());
        }
        if fetched_all {
            mark_verified(folder, manifest);
        }
        Ok(())
    }

    /// Downloads one file to `<path>.part`, retrying with backoff, verifies
    /// its hash, and renames it into place. A file already in place with the
    /// right size was verified by an earlier run and is skipped. `done` is
    /// the byte count of the files before this one. Returns whether the file
    /// was fetched (and so verified) by this call.
    async fn fetch_file(
        &mut self,
        manifest: &ModelManifest,
        file: &ManifestFile,
        folder: &Path,
        done: u64,
    ) -> Result<bool, Error> {
        let model = self.model;
        let dest = folder.join(file.path);
        if file_len(&dest).await == Some(file.size) {
            return Ok(false);
        }
        let part = part_path(&dest);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Io(format!("Could not create {}: {e}", parent.display())))?;
        }
        let url = manifest.url(file);
        let mut retries = 0;
        loop {
            let (message, timed_out) = match self.fetch_attempt(&url, file, &part, done).await {
                Ok(()) => break,
                Err(Attempt::Cancelled) => return Err(Error::Cancelled),
                Err(Attempt::Fatal(error)) => return Err(error),
                Err(Attempt::Retry { message, timed_out }) => (message, timed_out),
            };
            if retries == RETRIES {
                self.network_gave_out = true;
                return Err(match timed_out {
                    true => ModelError::DownloadTimedOut {
                        model_id: model.as_str().to_string(),
                    }
                    .into(),
                    false => {
                        Error::Download(format!("Could not download {}: {message}", file.path))
                    }
                });
            }
            let delay = self.inner.retry_delay * 2u32.pow(retries);
            retries += 1;
            tracing::warn!(%model, path = file.path, retry = retries, delay_ms = delay.as_millis() as u64, %message, "retrying file");
            tokio::select! {
                biased;
                () = self.token.cancelled() => return Err(Error::Cancelled),
                () = tokio::time::sleep(delay) => {}
            }
        }
        if let Some(want) = file.sha256 {
            verify_sha256(model, &part, file.path, want, self.token).await?;
        }
        tokio::fs::rename(&part, &dest)
            .await
            .map_err(|e| Error::Io(format!("Could not move {}: {e}", dest.display())))?;
        Ok(true)
    }

    /// One HTTP request for the rest of a file, resuming from the partial
    /// file with a Range request.
    async fn fetch_attempt(
        &mut self,
        url: &str,
        file: &ManifestFile,
        part: &Path,
        done: u64,
    ) -> Result<(), Attempt> {
        let model = self.model;
        let mut offset = file_len(part).await.unwrap_or(0);
        if offset == file.size {
            // a previous run stopped between the last byte and the hash check
            return Ok(());
        }
        if offset > file.size {
            offset = 0;
        }
        let mut request = self.inner.client.get(url);
        if offset > 0 {
            tracing::info!(%model, path = file.path, offset, size = file.size, "resuming file");
            request = request.header(RANGE, format!("bytes={offset}-"));
        } else {
            tracing::debug!(%model, path = file.path, size = file.size, "downloading file");
        }
        let response = tokio::select! {
            biased;
            () = self.token.cancelled() => return Err(Attempt::Cancelled),
            response = request.send() => response.map_err(|e| Attempt::retry(&e))?,
        };
        let status = response.status();
        match status {
            StatusCode::PARTIAL_CONTENT if offset > 0 => {
                let want = format!("bytes {offset}-");
                let content_range = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                if !content_range.starts_with(&want) {
                    return Err(Attempt::Fatal(Error::Download(format!(
                        "Could not download {}: the server resumed at the wrong offset ({content_range})",
                        file.path
                    ))));
                }
            }
            StatusCode::OK => {
                if offset > 0 {
                    tracing::warn!(%model, path = file.path, "server ignored the range request; restarting file");
                }
                offset = 0;
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                tracing::warn!(%model, path = file.path, offset, "partial file rejected by the server; restarting file");
                tokio::fs::remove_file(part)
                    .await
                    .map_err(|e| io_attempt("delete", part, &e))?;
                return Err(Attempt::Retry {
                    message: format!("HTTP {status}"),
                    timed_out: false,
                });
            }
            s if s.is_server_error()
                || s == StatusCode::TOO_MANY_REQUESTS
                || s == StatusCode::REQUEST_TIMEOUT =>
            {
                return Err(Attempt::Retry {
                    message: format!("HTTP {status}"),
                    timed_out: false,
                });
            }
            _ => {
                return Err(Attempt::Fatal(Error::Download(format!(
                    "Could not download {}: HTTP {status}",
                    file.path
                ))));
            }
        }
        self.write_body(response, file, part, offset, done).await
    }

    async fn write_body(
        &mut self,
        mut response: reqwest::Response,
        file: &ManifestFile,
        part: &Path,
        offset: u64,
        done: u64,
    ) -> Result<(), Attempt> {
        let opened = match offset {
            0 => tokio::fs::File::create(part).await,
            _ => tokio::fs::OpenOptions::new().append(true).open(part).await,
        };
        let mut out = opened.map_err(|e| io_attempt("open", part, &e))?;
        let mut written = offset;
        loop {
            let chunk = tokio::select! {
                biased;
                () = self.token.cancelled() => {
                    // let the in-flight write land so the partial file is a
                    // clean prefix for the next resume
                    if let Err(error) = out.flush().await {
                        tracing::warn!(path = %part.display(), %error, "could not flush partial file");
                    }
                    return Err(Attempt::Cancelled);
                }
                chunk = response.chunk() => chunk,
            };
            let bytes = match chunk {
                Ok(Some(bytes)) => bytes,
                Ok(None) => break,
                Err(error) => {
                    out.flush()
                        .await
                        .map_err(|e| io_attempt("write", part, &e))?;
                    return Err(Attempt::retry(&error));
                }
            };
            if written + bytes.len() as u64 > file.size {
                return Err(Attempt::Fatal(Error::Download(format!(
                    "Could not download {}: the server sent more than {} bytes",
                    file.path, file.size
                ))));
            }
            out.write_all(&bytes)
                .await
                .map_err(|e| io_attempt("write", part, &e))?;
            written += bytes.len() as u64;
            self.report(done + written);
        }
        out.flush()
            .await
            .map_err(|e| io_attempt("write", part, &e))?;
        if written < file.size {
            return Err(Attempt::Retry {
                message: format!("connection closed after {written} of {} bytes", file.size),
                timed_out: false,
            });
        }
        Ok(())
    }
}

async fn verify_sha256(
    model: ModelId,
    part: &Path,
    path: &str,
    want: &str,
    token: &CancellationToken,
) -> Result<(), Error> {
    let start = Instant::now();
    let to_hash = part.to_path_buf();
    let hashing = tokio::task::spawn_blocking(move || sha256_file(&to_hash, &mut |_| {}));
    // a cancelled hash keeps running to the end, read-only, then is dropped
    let joined = tokio::select! {
        biased;
        () = token.cancelled() => return Err(Error::Cancelled),
        joined = hashing => joined,
    };
    let got = joined
        .map_err(|e| Error::Io(format!("Could not check {path}: {e}")))?
        .map_err(|e| Error::Io(format!("Could not check {path}: {}", describe(&e))))?;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    if !got.eq_ignore_ascii_case(want) {
        tracing::error!(%model, path, want, got, "checksum mismatch");
        return Err(Error::Download(format!("Checksum mismatch for {path}")));
    }
    tracing::debug!(%model, path, elapsed_ms, "checksum verified");
    Ok(())
}

async fn file_len(path: &Path) -> Option<u64> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    meta.is_file().then_some(meta.len())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}
