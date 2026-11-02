//! The updater's worker and the state it shows (UPD-1..UPD-7).
//!
//! One task runs every check, download, and restart in order, so they never
//! overlap. With automatic checks on, it checks shortly after launch and
//! then once the last check is a day old; an isolated verification run
//! (`ONDA_DATA_DIR`) only checks when asked. A newer release on a
//! replaceable install downloads and stages at once and waits as
//! [`UpdateStatus::Ready`] for the user's restart. Checks keep running while
//! an update is staged: a newer release replaces it once that one is
//! verified, and a failed check leaves it ready. Without a staged update a
//! failure lands in [`UpdateStatus::Failed`], and the next check (manual or
//! scheduled) starts over from the feed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc as std_mpsc};
use std::time::{Duration, SystemTime};

use ed25519_dalek::VerifyingKey;
use onda_domain::updates::{UpdateFailure, UpdateStatus};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::feed::{self, Artifact, Decision, Install, Origins, OsVersion, Release};
use crate::install::{self, ApplyError, Expected, InstallError, Staged};
use crate::prefs::Prefs;
use crate::transport::{TransferError, Transport};

/// When automatic checks run.
#[derive(Debug, Clone, Copy)]
pub struct Schedule {
    /// The first check after launch.
    pub launch_delay: Duration,
    /// How often the worker wakes to see whether a check is due.
    pub poll: Duration,
    /// How old the last check must be before the next one.
    pub interval: Duration,
    /// How long a restart waits for the feed before installing the staged
    /// update anyway.
    pub restart_refresh: Duration,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            launch_delay: Duration::from_secs(10),
            poll: Duration::from_secs(60 * 60),
            interval: Duration::from_secs(24 * 60 * 60),
            restart_refresh: Duration::from_secs(5),
        }
    }
}

/// How the updater is set up.
#[derive(Debug, Clone)]
pub struct Config {
    /// The signed feed.
    pub feed_url: String,
    /// Where downloads may come from.
    pub origins: Origins,
    /// The key the feed must be signed with.
    pub public_key: VerifyingKey,
    /// The running `CFBundleVersion`; unused for download-only checks.
    pub build: u64,
    /// The running macOS version; unused for download-only checks.
    pub os_version: OsVersion,
    /// Holds `updater.json` and partial downloads.
    pub state_dir: PathBuf,
    /// Automatic checks may run; off for isolated verification runs.
    pub background_checks: bool,
    /// The automatic-check preference before one is saved.
    pub automatic_default: bool,
    /// When automatic checks run.
    pub schedule: Schedule,
    /// Sent with every request.
    pub user_agent: String,
}

/// A package managed outside Onda. Checks offer a download without replacing files.
#[derive(Debug, Clone)]
pub struct DownloadTarget {
    /// Installed semantic version; equal or older versions are never offered.
    pub version: semver::Version,
    /// Feed platform, such as `linux`.
    pub os: String,
    /// Feed architecture, such as `arm64` or `x86_64`.
    pub arch: String,
    /// Package kind: `deb`, `rpm`, or `tar.gz`.
    pub kind: String,
}

impl DownloadTarget {
    fn decide(&self, release: &Release) -> Result<Option<Artifact>, UpdateFailure> {
        let version =
            semver::Version::parse(&release.version).map_err(|_| UpdateFailure::Untrusted)?;
        if !version.cmp_precedence(&self.version).is_gt() {
            return Ok(None);
        }
        release
            .artifact(&self.os, &self.arch, &self.kind)
            .cloned()
            .map(Some)
            .ok_or(UpdateFailure::Check)
    }
}

/// What the updater needs from the app.
pub trait Host: Send + Sync {
    /// The snapshot changed.
    fn changed(&self);
    /// Opens `url` in the browser.
    fn open_url(&self, url: &str);
    /// Quits the app; the update is in place and the relauncher waits.
    fn terminate(&self);
}

/// Stages and applies verified archives. [`MacInstaller`] is the real one.
pub trait Installer: Send + Sync {
    /// The installed app can replace itself.
    fn replaceable(&self) -> bool;
    /// Removes what earlier updates left behind.
    fn remove_leftovers(&self);
    /// Unpacks and verifies `zip`, the archive of `release`.
    fn stage(&self, zip: &Path, release: &Release) -> Result<Staged, InstallError>;
    /// Checks `staged` again, swaps it in, and arranges the relaunch.
    fn apply(&self, staged: &Staged) -> Result<(), ApplyError>;
    /// Drops `staged` without installing it.
    fn discard(&self, staged: &Staged);
}

/// The installer for the running `Onda.app`.
pub struct MacInstaller {
    bundle: PathBuf,
    build: u64,
    os_version: OsVersion,
    open_args: Vec<OsString>,
}

impl MacInstaller {
    /// Replaces `bundle`, which runs as `build`, on `os_version`;
    /// `open_args` go to `open` before the bundle path when it relaunches.
    pub fn new(
        bundle: PathBuf,
        build: u64,
        os_version: OsVersion,
        open_args: Vec<OsString>,
    ) -> Self {
        Self {
            bundle,
            build,
            os_version,
            open_args,
        }
    }
}

impl Installer for MacInstaller {
    fn replaceable(&self) -> bool {
        install::replaceable(&self.bundle)
    }

    fn remove_leftovers(&self) {
        install::remove_leftovers(&self.bundle);
    }

    fn stage(&self, zip: &Path, release: &Release) -> Result<Staged, InstallError> {
        let expected = Expected::for_release(release, self.os_version, crate::CODE_REQUIREMENT);
        install::stage(zip, &self.bundle, &expected)
    }

    fn apply(&self, staged: &Staged) -> Result<(), ApplyError> {
        install::apply(staged, self.build, std::process::id(), &self.open_args)
    }

    fn discard(&self, staged: &Staged) {
        install::discard(staged);
    }
}

/// What the UI shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// What the updater is doing.
    pub status: UpdateStatus,
    /// "Check for updates automatically".
    pub automatic: bool,
    /// The last check that reached a verified feed.
    pub last_check: Option<SystemTime>,
}

enum Command {
    Check,
    /// Answered with whether Onda is quitting to relaunch.
    Restart(std_mpsc::Sender<bool>),
}

struct State {
    status: UpdateStatus,
    prefs: Prefs,
    /// A verified update waiting for the restart. While it is set, the
    /// status is [`UpdateStatus::Ready`] whenever the worker is idle.
    staged: Option<Staged>,
    /// Where [`UpdateStatus::Available`] points.
    download_url: Option<String>,
    /// A swap went in but neither relaunched nor undid; nothing more is
    /// tried until Onda quits.
    stranded: bool,
    /// The last build whose download or staging failed; a restart does not
    /// wait for it again.
    failed_build: Option<u64>,
}

struct Shared {
    state: Mutex<State>,
    prefs_file: PathBuf,
    host: Arc<dyn Host>,
    /// Commands the worker finished; tests wait on it.
    finished: AtomicUsize,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_status(&self, status: UpdateStatus) {
        {
            let mut state = self.lock();
            if state.status == status {
                return;
            }
            state.status = status;
        }
        self.host.changed();
    }

    /// Changes the preferences and saves them under the state lock, so
    /// saves land in the order the changes happened and the file always
    /// matches memory. `change` returning `Err` saves nothing.
    fn update_prefs<E>(&self, change: impl FnOnce(&mut Prefs) -> Result<(), E>) -> Result<(), E> {
        let mut state = self.lock();
        change(&mut state.prefs)?;
        if let Err(error) = state.prefs.save(&self.prefs_file) {
            tracing::error!(%error, "could not save the updater state");
        }
        Ok(())
    }
}

/// The running updater. Cheap to clone.
#[derive(Clone)]
pub struct Updater {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedSender<Command>,
}

impl Updater {
    /// Starts the worker on `runtime`.
    pub fn start(
        config: Config,
        installer: Arc<dyn Installer>,
        host: Arc<dyn Host>,
        runtime: &Handle,
    ) -> Result<Self, String> {
        Self::start_worker(config, Some(installer), None, host, runtime)
    }

    /// Starts signed checks for a package installed outside Onda.
    pub fn start_download_only(
        config: Config,
        target: DownloadTarget,
        host: Arc<dyn Host>,
        runtime: &Handle,
    ) -> Result<Self, String> {
        Self::start_worker(config, None, Some(target), host, runtime)
    }

    fn start_worker(
        config: Config,
        installer: Option<Arc<dyn Installer>>,
        download_target: Option<DownloadTarget>,
        host: Arc<dyn Host>,
        runtime: &Handle,
    ) -> Result<Self, String> {
        let transport = Transport::new(config.origins.clone(), &config.user_agent)?;
        let prefs_file = config.state_dir.join("updater.json");
        let prefs = Prefs::load(&prefs_file, config.automatic_default);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                status: UpdateStatus::Idle,
                prefs,
                staged: None,
                download_url: None,
                stranded: false,
                failed_build: None,
            }),
            prefs_file,
            host,
            finished: AtomicUsize::new(0),
        });
        let (commands, receiver) = mpsc::unbounded_channel();
        let worker = Worker {
            shared: shared.clone(),
            config,
            transport,
            installer,
            download_target,
        };
        tracing::info!(
            build = worker.config.build,
            background = worker.config.background_checks,
            "updater started"
        );
        runtime.spawn(worker.run(receiver));
        Ok(Self { shared, commands })
    }

    /// The current state.
    pub fn snapshot(&self) -> Snapshot {
        let state = self.shared.lock();
        Snapshot {
            status: state.status.clone(),
            automatic: state.prefs.automatic_checks,
            last_check: state.prefs.last_check_time(),
        }
    }

    /// "Check for Updates…" (UPD-1).
    pub fn check_now(&self) {
        let _ = self.commands.send(Command::Check);
    }

    /// "Check for updates automatically".
    pub fn set_automatic_checks(&self, enabled: bool) {
        let saved = self.shared.update_prefs(|prefs| {
            prefs.automatic_checks = enabled;
            Ok::<(), ()>(())
        });
        if saved.is_ok() {
            self.shared.host.changed();
        }
    }

    /// "Restart to Update" (UPD-6). Blocks until Onda is quitting to
    /// relaunch (`true`) or the restart did not happen (`false`): nothing
    /// was ready, a newer update replaces the staged one first, or the
    /// install failed. Call it off any async runtime thread.
    ///
    /// Claiming [`UpdateStatus::Ready`] here, under the state lock, keeps
    /// checks from replacing the staged update while it installs, and keeps
    /// a restart from waiting behind a long download.
    pub fn restart_to_update(&self) -> bool {
        {
            let mut state = self.shared.lock();
            let UpdateStatus::Ready { version } = &state.status else {
                return false;
            };
            if state.staged.is_none() {
                return false;
            }
            state.status = UpdateStatus::Installing {
                version: version.clone(),
            };
        }
        self.shared.host.changed();
        let (reply, answer) = std_mpsc::channel();
        if self.commands.send(Command::Restart(reply)).is_err() {
            return false;
        }
        answer.recv().unwrap_or(false)
    }

    /// "Download Onda …" (UPD-7); ignored unless one is available.
    pub fn open_download(&self) {
        let url = {
            let state = self.shared.lock();
            match state.status {
                UpdateStatus::Available { .. } => state.download_url.clone(),
                _ => None,
            }
        };
        if let Some(url) = url {
            self.shared.host.open_url(&url);
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    config: Config,
    transport: Transport,
    installer: Option<Arc<dyn Installer>>,
    download_target: Option<DownloadTarget>,
}

impl Worker {
    async fn run(self, mut commands: mpsc::UnboundedReceiver<Command>) {
        if let Some(installer) = self.installer.clone()
            && let Err(error) =
                tokio::task::spawn_blocking(move || installer.remove_leftovers()).await
        {
            tracing::warn!(%error, "could not clean up earlier updates");
        }
        let schedule = self.config.schedule;
        let mut next = Instant::now() + schedule.launch_delay;
        let mut launch_check = true;
        loop {
            let command = tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => command,
                    None => return,
                },
                () = tokio::time::sleep_until(next), if self.config.background_checks => {
                    next = Instant::now() + schedule.poll;
                    let due = self.background_check_due(launch_check);
                    launch_check = false;
                    if !due {
                        continue;
                    }
                    tracing::info!("checking for updates in the background");
                    Command::Check
                }
            };
            match command {
                Command::Check => self.check().await,
                Command::Restart(reply) => {
                    let relaunching = self.restart().await;
                    let _ = reply.send(relaunching);
                }
            }
            self.shared.finished.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Automatic checks are on and this is the launch check or the last
    /// one is old enough. A staged update does not stop them: a newer
    /// release replaces it. A last check in the future counts as old.
    fn background_check_due(&self, launch_check: bool) -> bool {
        let state = self.shared.lock();
        let old_enough = state.prefs.last_check_time().is_none_or(|last| {
            last.elapsed()
                .map_or(true, |age| age >= self.config.schedule.interval)
        });
        state.prefs.automatic_checks && (launch_check || old_enough)
    }

    async fn check(&self) {
        {
            let mut state = self.shared.lock();
            // a restart claimed the staged update, or a swap is stranded
            if state.stranded || matches!(state.status, UpdateStatus::Installing { .. }) {
                return;
            }
            state.status = UpdateStatus::Checking;
        }
        self.shared.host.changed();
        let outcome = match self.fetch().await {
            Ok(release) => self.act(&release).await,
            Err(failure) => Err(failure),
        };
        let status = self.settle(outcome);
        self.shared.set_status(status);
    }

    /// The status after a check: a staged update outlives a failed check,
    /// and stays ready unless the feed says it is no longer wanted.
    fn settle(&self, outcome: Result<UpdateStatus, UpdateFailure>) -> UpdateStatus {
        let (status, dropped) = {
            let mut state = self.shared.lock();
            let Some(version) = state.staged.as_ref().map(|s| s.version().to_owned()) else {
                return outcome.unwrap_or_else(UpdateStatus::Failed);
            };
            match outcome {
                // nothing newer applies to this build, or it cannot replace
                // itself any more
                Ok(status @ (UpdateStatus::UpToDate | UpdateStatus::Available { .. })) => {
                    (status, state.staged.take())
                }
                Ok(status @ UpdateStatus::Ready { .. }) => (status, None),
                Ok(_) => (UpdateStatus::Ready { version }, None),
                Err(failure) => {
                    tracing::warn!(?failure, "the check failed; the staged update stays ready");
                    (UpdateStatus::Ready { version }, None)
                }
            }
        };
        if let Some(staged) = dropped {
            tracing::info!(build = staged.build(), "dropping the staged update");
            if let Some(installer) = &self.installer {
                installer.discard(&staged);
            }
        }
        status
    }

    /// Fetches and verifies the feed, refuses one older than a feed already
    /// verified, and records the check.
    async fn fetch(&self) -> Result<Release, UpdateFailure> {
        let bytes = self
            .transport
            .fetch_feed(&self.config.feed_url)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "could not fetch the update feed");
                UpdateFailure::Check
            })?;
        let release =
            feed::open(&bytes, &self.config.public_key, &self.config.origins).map_err(|error| {
                tracing::error!(%error, "refused the update feed");
                UpdateFailure::Untrusted
            })?;
        self.shared.update_prefs(|prefs| {
            if release.build < prefs.highest_build_seen {
                tracing::error!(
                    build = release.build,
                    seen = prefs.highest_build_seen,
                    "refused an update feed older than one already seen"
                );
                return Err(UpdateFailure::Untrusted);
            }
            prefs.highest_build_seen = release.build;
            prefs.set_last_check(SystemTime::now());
            Ok(())
        })?;
        Ok(release)
    }

    fn install(&self) -> Install {
        Install {
            build: self.config.build,
            os_version: self.config.os_version,
            replaceable: self
                .installer
                .as_ref()
                .is_some_and(|installer| installer.replaceable()),
        }
    }

    /// Decides what `release` means for this install and acts on it.
    async fn act(&self, release: &Release) -> Result<UpdateStatus, UpdateFailure> {
        if let Some(target) = &self.download_target {
            let artifact = target.decide(release)?;
            self.shared.lock().download_url =
                artifact.as_ref().map(|artifact| artifact.url.clone());
            return Ok(match artifact {
                Some(_) => UpdateStatus::Available {
                    version: release.version.clone(),
                },
                None => UpdateStatus::UpToDate,
            });
        }
        let install = self.install();
        tracing::info!(
            version = release.version,
            build = release.build,
            replaceable = install.replaceable,
            "read the update feed"
        );
        match feed::decide(release, &install) {
            Decision::UpToDate => Ok(UpdateStatus::UpToDate),
            Decision::RequiresNewerOs { minimum_os } => Err(UpdateFailure::RequiresNewerOs {
                version: release.version.clone(),
                minimum_os,
            }),
            Decision::DownloadOnly(artifact) => {
                self.shared.lock().download_url = Some(artifact.url);
                Ok(UpdateStatus::Available {
                    version: release.version.clone(),
                })
            }
            Decision::Install(artifact) => {
                let result = self.download_and_stage(release, &artifact).await;
                self.shared.lock().failed_build = result.is_err().then_some(release.build);
                result
            }
        }
    }

    /// Downloads and stages `release` unless it is already staged. A staged
    /// older update stays until the new one is verified and staged.
    async fn download_and_stage(
        &self,
        release: &Release,
        artifact: &Artifact,
    ) -> Result<UpdateStatus, UpdateFailure> {
        let ready = UpdateStatus::Ready {
            version: release.version.clone(),
        };
        let already = self
            .shared
            .lock()
            .staged
            .as_ref()
            .is_some_and(|staged| staged.build() == release.build);
        if already {
            return Ok(ready);
        }
        let downloads = self.config.state_dir.join("downloads");
        prune(&downloads, &artifact.sha256);
        let shared = self.shared.clone();
        let version = release.version.clone();
        let size = artifact.size.max(1);
        let shown = AtomicU8::new(u8::MAX);
        let progress = move |written: u64| {
            let percent = u8::try_from(written.saturating_mul(100) / size).unwrap_or(100);
            if shown.swap(percent, Ordering::Relaxed) != percent {
                shared.set_status(UpdateStatus::Downloading {
                    version: version.clone(),
                    percent,
                });
            }
        };
        progress(0);
        let zip = self
            .transport
            .download(artifact, &downloads, &progress)
            .await
            .map_err(|error| {
                tracing::error!(%error, "update download failed");
                match error {
                    TransferError::Mismatch(_) => UpdateFailure::Verify,
                    TransferError::Network(_) | TransferError::Io(_) => UpdateFailure::Download,
                }
            })?;
        let installer = self.installer.clone().ok_or(UpdateFailure::Install)?;
        let archive = zip.clone();
        let owned = release.clone();
        let staged = tokio::task::spawn_blocking(move || installer.stage(&archive, &owned))
            .await
            .unwrap_or_else(|error| Err(InstallError::Failed(error.to_string())));
        if let Err(error) = std::fs::remove_file(&zip) {
            tracing::warn!(%error, "could not delete the update archive");
        }
        let staged = staged.map_err(|error| {
            tracing::error!(%error, "could not stage the update");
            match error {
                InstallError::Rejected(_) => UpdateFailure::Verify,
                InstallError::Failed(_) => UpdateFailure::Install,
            }
        })?;
        tracing::info!(version = release.version, path = %staged.app.display(), "update ready");
        let earlier = self.shared.lock().staged.replace(staged);
        if let Some(earlier) = earlier
            && let Some(installer) = &self.installer
        {
            installer.discard(&earlier);
        }
        Ok(ready)
    }

    /// `release` is newer than `staged`, would install, and has not just
    /// failed to download or stage, so a restart waits for it.
    fn replaces(&self, release: &Release, staged: &Staged) -> bool {
        release.build > staged.build()
            && self.shared.lock().failed_build != Some(release.build)
            && matches!(feed::decide(release, &self.install()), Decision::Install(_))
    }

    /// Installs the update [`Updater::restart_to_update`] claimed. First a
    /// short feed refresh: a newer release replaces the staged one instead
    /// (the user restarts again once it is ready). A refresh that fails or
    /// times out (offline) does not hold the restart back: the staged app
    /// was verified when staged and is verified again before the swap.
    async fn restart(&self) -> bool {
        let Some(installer) = self.installer.clone() else {
            return false;
        };
        let staged = {
            let state = self.shared.lock();
            match (&state.status, &state.staged) {
                (UpdateStatus::Installing { .. }, Some(staged)) => staged.clone(),
                _ => return false,
            }
        };
        match tokio::time::timeout(self.config.schedule.restart_refresh, self.fetch()).await {
            Ok(Ok(release))
                if matches!(
                    feed::decide(&release, &self.install()),
                    Decision::UpToDate | Decision::DownloadOnly(_)
                ) =>
            {
                let outcome = self.act(&release).await;
                let status = self.settle(outcome);
                self.shared.set_status(status);
                return false;
            }
            Ok(Ok(release)) if self.replaces(&release, &staged) => {
                tracing::info!(
                    build = release.build,
                    "a newer update replaces the staged one before restarting"
                );
                self.shared.set_status(UpdateStatus::Checking);
                let outcome = self.act(&release).await;
                let status = self.settle(outcome);
                self.shared.set_status(status);
                return false;
            }
            Ok(Ok(_)) => {}
            Ok(Err(failure)) => {
                tracing::warn!(
                    ?failure,
                    "could not refresh the feed; installing the staged update"
                );
            }
            Err(_) => tracing::warn!("the feed refresh timed out; installing the staged update"),
        }
        let applying = installer.clone();
        let target = staged.clone();
        let applied = tokio::task::spawn_blocking(move || applying.apply(&target))
            .await
            .unwrap_or_else(|error| {
                Err(ApplyError::NotApplied(InstallError::Failed(
                    error.to_string(),
                )))
            });
        let failure = match applied {
            Ok(()) => {
                tracing::info!(
                    version = staged.version(),
                    "update installed; quitting to relaunch"
                );
                self.shared.host.terminate();
                return true;
            }
            Err(ApplyError::NotApplied(error)) => {
                tracing::error!(%error, "could not install the update");
                // nothing changed, so the staging folder holds only the update
                installer.discard(&staged);
                self.shared.lock().staged = None;
                match error {
                    InstallError::Rejected(_) => UpdateFailure::Verify,
                    InstallError::Failed(_) => UpdateFailure::Install,
                }
            }
            Err(error @ ApplyError::Stranded { .. }) => {
                // the staging folder holds the running app now: keep it
                tracing::error!(%error, "installed the update but could not relaunch");
                self.shared.lock().stranded = true;
                UpdateFailure::Relaunch {
                    version: staged.version().to_owned(),
                }
            }
        };
        self.shared.set_status(UpdateStatus::Failed(failure));
        false
    }
}

/// Removes downloads other than the one for `keep` (a SHA-256).
fn prune(downloads: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(downloads) else {
        return;
    };
    for entry in entries.flatten() {
        let current = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(keep));
        if !current && let Err(error) = std::fs::remove_file(entry.path()) {
            tracing::warn!(%error, "could not delete an old update download");
        }
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::feed::{CHANNEL, KIND_APP_ZIP, KIND_DMG, SCHEMA};
    use crate::transport::tests::{Behavior, Server, artifact_for};

    const ZIP_PATH: &str = "/releases/v2.0.0/Onda-2.0.0.zip";
    const DMG_PATH: &str = "/releases/v2.0.0/Onda-2.0.0.dmg";

    #[derive(Default)]
    struct FakeHost {
        changes: AtomicUsize,
        opened: Mutex<Vec<String>>,
        terminated: AtomicUsize,
    }

    impl Host for FakeHost {
        fn changed(&self) {
            self.changes.fetch_add(1, Ordering::SeqCst);
        }

        fn open_url(&self, url: &str) {
            self.opened.lock().unwrap().push(url.to_owned());
        }

        fn terminate(&self) {
            self.terminated.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeInstaller {
        replaceable: bool,
        stage_error: Mutex<Option<InstallError>>,
        apply_error: Mutex<Option<ApplyError>>,
        log: Mutex<Vec<String>>,
    }

    impl FakeInstaller {
        fn new(replaceable: bool) -> Self {
            Self {
                replaceable,
                stage_error: Mutex::new(None),
                apply_error: Mutex::new(None),
                log: Mutex::new(Vec::new()),
            }
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Installer for FakeInstaller {
        fn replaceable(&self) -> bool {
            self.replaceable
        }

        fn remove_leftovers(&self) {
            self.log.lock().unwrap().push("clean".to_owned());
        }

        fn stage(&self, zip: &Path, release: &Release) -> Result<Staged, InstallError> {
            let bytes = std::fs::read(zip).unwrap();
            self.log
                .lock()
                .unwrap()
                .push(format!("stage {} {}", release.build, bytes.len()));
            if let Some(error) = self.stage_error.lock().unwrap().take() {
                return Err(error);
            }
            Ok(Staged::detached(
                PathBuf::from(format!("/Applications/.onda-update-{}", release.build)),
                PathBuf::from("/Applications/Onda.app"),
                Expected::for_release(release, OsVersion::parse("26.1").unwrap(), "anything"),
            ))
        }

        fn apply(&self, staged: &Staged) -> Result<(), ApplyError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("apply {}", staged.build()));
            match self.apply_error.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn discard(&self, staged: &Staged) {
            self.log
                .lock()
                .unwrap()
                .push(format!("discard {}", staged.build()));
        }
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn zip_body() -> Vec<u8> {
        b"Troy and Abed's update".repeat(1000)
    }

    #[tokio::test]
    async fn package_checks_verify_feeds_preserve_preferences_and_never_install() {
        let server = Server::start().await;
        let directory = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let target = DownloadTarget {
            version: "3.2.0".parse().unwrap(),
            os: "linux".into(),
            arch: "arm64".into(),
            kind: "rpm".into(),
        };
        let config = Config {
            feed_url: server.url("/update.json"),
            origins: server.origins(),
            public_key: signing_key().verifying_key(),
            build: 0,
            os_version: OsVersion::parse("0").unwrap(),
            state_dir: directory.path().to_owned(),
            background_checks: false,
            automatic_default: true,
            schedule: Schedule::default(),
            user_agent: "Onda/test".into(),
        };
        let start = || {
            Updater::start_download_only(
                config.clone(),
                target.clone(),
                host.clone(),
                &Handle::current(),
            )
            .unwrap()
        };
        let updater = start();
        let mut artifacts = Vec::new();
        for arch in ["arm64", "x86_64"] {
            for kind in ["rpm", "deb", "tar.gz"] {
                let mut item = artifact_for(
                    &server,
                    &format!("/releases/Greendale-{arch}.{kind}"),
                    b"Troy Barnes",
                );
                item.os = "linux".into();
                item.arch = arch.into();
                item.kind = kind.into();
                artifacts.push(item);
            }
        }
        let mut release = Release {
            schema: SCHEMA,
            app: crate::BUNDLE_ID.into(),
            channel: CHANNEL.into(),
            version: "3.3.0".into(),
            build: 20,
            published_at: "2026-11-01T12:00:00Z".into(),
            notes_url: None,
            artifacts,
        };
        for arch in ["arm64", "x86_64"] {
            for kind in ["rpm", "deb", "tar.gz"] {
                let selected = DownloadTarget {
                    arch: arch.into(),
                    kind: kind.into(),
                    ..target.clone()
                };
                let item = selected.decide(&release).unwrap().unwrap();
                assert_eq!((&item.arch, &item.kind), (&selected.arch, &selected.kind));
            }
        }
        server.put(
            "/update.json",
            feed::seal(&release, &signing_key()),
            Behavior::Serve,
        );
        assert_eq!(
            package_check(&updater).await,
            UpdateStatus::Available {
                version: "3.3.0".into()
            }
        );
        assert!(updater.snapshot().last_check.is_some());
        updater.open_download();
        assert_eq!(
            *host.opened.lock().unwrap(),
            [server.url("/releases/Greendale-arm64.rpm")]
        );
        assert!(!updater.restart_to_update());
        assert_eq!(host.terminated.load(Ordering::SeqCst), 0);
        assert!(!directory.path().join("downloads").exists());
        assert!(server.requests("/releases/Greendale-arm64.rpm").is_empty());
        updater.set_automatic_checks(false);
        drop(updater);
        let updater = start();
        assert!(!updater.snapshot().automatic);
        assert!(updater.snapshot().last_check.is_some());

        release.build = 19;
        server.put(
            "/update.json",
            feed::seal(&release, &signing_key()),
            Behavior::Serve,
        );
        assert_eq!(
            package_check(&updater).await,
            UpdateStatus::Failed(UpdateFailure::Untrusted)
        );
        release.build = 21;
        server.put(
            "/update.json",
            feed::seal(&release, &SigningKey::from_bytes(&[7; 32])),
            Behavior::Serve,
        );
        assert_eq!(
            package_check(&updater).await,
            UpdateStatus::Failed(UpdateFailure::Untrusted)
        );
        updater.open_download();
        assert_eq!(host.opened.lock().unwrap().len(), 1);

        for version in ["3.1.9", "3.2.0-rc.1", "3.2.0", "3.2.0+greendale"] {
            release.version = version.into();
            assert!(target.decide(&release).unwrap().is_none(), "{version}");
        }
        release.version = "3.3.0".into();
        release.artifacts.retain(|item| item.kind != "rpm");
        server.put(
            "/update.json",
            feed::seal(&release, &signing_key()),
            Behavior::Serve,
        );
        assert_eq!(
            package_check(&updater).await,
            UpdateStatus::Failed(UpdateFailure::Check)
        );
    }

    async fn package_check(updater: &Updater) -> UpdateStatus {
        let before = updater.shared.finished.load(Ordering::SeqCst);
        updater.check_now();
        for _ in 0..500 {
            if updater.shared.finished.load(Ordering::SeqCst) > before {
                return updater.snapshot().status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("package check did not finish");
    }

    struct Fixture {
        server: Server,
        host: Arc<FakeHost>,
        installer: Arc<FakeInstaller>,
        updater: Updater,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        async fn new(build: u64, replaceable: bool) -> Self {
            Self::with(build, replaceable, false, tempfile::tempdir().unwrap()).await
        }

        async fn with(
            build: u64,
            replaceable: bool,
            background_checks: bool,
            dir: tempfile::TempDir,
        ) -> Self {
            let schedule = Schedule {
                launch_delay: Duration::from_millis(50),
                poll: Duration::from_millis(50),
                interval: Duration::from_secs(3600),
                restart_refresh: Duration::from_secs(5),
            };
            Self::scheduled(build, replaceable, background_checks, dir, schedule).await
        }

        async fn scheduled(
            build: u64,
            replaceable: bool,
            background_checks: bool,
            dir: tempfile::TempDir,
            schedule: Schedule,
        ) -> Self {
            let server = Server::start().await;
            server.put(ZIP_PATH, zip_body(), Behavior::Serve);
            server.put(DMG_PATH, b"dmg".to_vec(), Behavior::Serve);
            let fixture_host = Arc::new(FakeHost::default());
            let installer = Arc::new(FakeInstaller::new(replaceable));
            let config = Config {
                feed_url: server.url("/update.json"),
                origins: server.origins(),
                public_key: signing_key().verifying_key(),
                build,
                os_version: OsVersion::parse("26.1").unwrap(),
                state_dir: dir.path().to_path_buf(),
                background_checks,
                automatic_default: true,
                schedule,
                user_agent: "Onda/test".to_owned(),
            };
            let updater = Updater::start(
                config,
                installer.clone(),
                fixture_host.clone(),
                &Handle::current(),
            )
            .unwrap();
            Self {
                server,
                host: fixture_host,
                installer,
                updater,
                _dir: dir,
            }
        }

        fn publish(&self, build: u64) {
            self.publish_signed(build, &signing_key());
        }

        fn publish_signed(&self, build: u64, key: &SigningKey) {
            let mut zip = artifact_for(&self.server, ZIP_PATH, &zip_body());
            zip.kind = KIND_APP_ZIP.to_owned();
            let mut dmg = artifact_for(&self.server, DMG_PATH, b"dmg");
            dmg.kind = KIND_DMG.to_owned();
            let release = Release {
                schema: SCHEMA,
                app: crate::BUNDLE_ID.to_owned(),
                channel: CHANNEL.to_owned(),
                version: format!("2.0.{build}"),
                build,
                published_at: "2026-11-01T12:00:00Z".to_owned(),
                notes_url: None,
                artifacts: vec![zip, dmg],
            };
            self.server
                .put("/update.json", feed::seal(&release, key), Behavior::Serve);
        }

        async fn settle(&self, done: impl Fn(&UpdateStatus) -> bool) -> UpdateStatus {
            for _ in 0..500 {
                let status = self.updater.snapshot().status;
                if done(&status) {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("status stuck at {:?}", self.updater.snapshot().status);
        }

        /// Runs `send` and waits for the worker to finish that command.
        async fn finish(&self, send: impl FnOnce(&Updater)) -> UpdateStatus {
            let before = self.updater.shared.finished.load(Ordering::SeqCst);
            send(&self.updater);
            for _ in 0..500 {
                if self.updater.shared.finished.load(Ordering::SeqCst) > before {
                    return self.updater.snapshot().status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("command never finished");
        }

        async fn check(&self) -> UpdateStatus {
            self.finish(Updater::check_now).await
        }

        /// Restarts from a blocking thread, as the app does.
        async fn restart(&self) -> bool {
            let updater = self.updater.clone();
            tokio::task::spawn_blocking(move || updater.restart_to_update())
                .await
                .unwrap()
        }

        fn ready(version: &str) -> UpdateStatus {
            UpdateStatus::Ready {
                version: version.to_owned(),
            }
        }

        fn logged(&self, prefix: &str) -> Vec<String> {
            self.installer
                .log()
                .into_iter()
                .filter(|line| line.starts_with(prefix))
                .collect()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_newer_release_downloads_stages_and_restarts() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        let status = fixture.check().await;
        assert_eq!(
            status,
            UpdateStatus::Ready {
                version: "2.0.42".to_owned()
            }
        );
        assert!(fixture.updater.snapshot().last_check.is_some());

        assert!(fixture.restart().await);
        assert_eq!(
            fixture.updater.snapshot().status,
            UpdateStatus::Installing {
                version: "2.0.42".to_owned()
            }
        );
        assert_eq!(fixture.host.terminated.load(Ordering::SeqCst), 1);
        let log = fixture.installer.log();
        assert_eq!(log.last().map(String::as_str), Some("apply 42"));
        assert!(
            log.contains(&format!("stage 42 {}", zip_body().len())),
            "{log:?}"
        );
        assert!(fixture.host.changes.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_same_or_an_older_build_is_up_to_date() {
        let fixture = Fixture::new(42, true).await;
        fixture.publish(42);
        assert_eq!(fixture.check().await, UpdateStatus::UpToDate);
        assert!(fixture.server.requests(ZIP_PATH).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_does_nothing_without_a_staged_update() {
        let fixture = Fixture::new(42, true).await;
        fixture.publish(42);
        fixture.check().await;
        assert!(!fixture.restart().await);
        assert_eq!(fixture.updater.snapshot().status, UpdateStatus::UpToDate);
        assert_eq!(fixture.host.terminated.load(Ordering::SeqCst), 0);
        assert!(
            !fixture
                .installer
                .log()
                .iter()
                .any(|l| l.starts_with("apply"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_corrupt_download_fails_and_a_retry_recovers() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.server.behave(ZIP_PATH, Behavior::Corrupt);
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Failed(UpdateFailure::Verify)
        );
        assert!(
            !fixture
                .installer
                .log()
                .iter()
                .any(|l| l.starts_with("stage"))
        );

        fixture.server.behave(ZIP_PATH, Behavior::Serve);
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Ready {
                version: "2.0.42".to_owned()
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn feed_failures_map_to_their_causes() {
        let fixture = Fixture::new(41, true).await;
        fixture
            .server
            .put("/update.json", Vec::new(), Behavior::Fail);
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Failed(UpdateFailure::Check)
        );

        fixture.publish_signed(42, &SigningKey::from_bytes(&[8; 32]));
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Failed(UpdateFailure::Untrusted)
        );
        assert!(fixture.updater.snapshot().last_check.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_replayed_older_feed_is_refused() {
        let fixture = Fixture::new(40, false).await;
        fixture.publish(42);
        fixture.check().await;
        fixture.publish(41);
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Failed(UpdateFailure::Untrusted)
        );

        // a staged update stays ready and is not replaced
        let staged = Fixture::new(40, true).await;
        staged.publish(42);
        staged.check().await;
        staged.publish(41);
        assert_eq!(staged.check().await, Fixture::ready("2.0.42"));
        assert_eq!(staged.logged("stage").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rejected_bundle_is_a_verification_failure() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        *fixture.installer.stage_error.lock().unwrap() =
            Some(InstallError::Rejected("not Onda".to_owned()));
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Failed(UpdateFailure::Verify)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_swap_reports_and_discards_the_update() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;
        *fixture.installer.apply_error.lock().unwrap() = Some(ApplyError::NotApplied(
            InstallError::Failed("disk full".to_owned()),
        ));
        assert!(!fixture.restart().await);
        assert_eq!(
            fixture.updater.snapshot().status,
            UpdateStatus::Failed(UpdateFailure::Install)
        );
        assert_eq!(fixture.host.terminated.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture.installer.log().last().map(String::as_str),
            Some("discard 42")
        );
        assert!(!fixture.restart().await, "nothing is staged any more");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stranded_swap_keeps_the_previous_app_and_stops() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;
        *fixture.installer.apply_error.lock().unwrap() = Some(ApplyError::Stranded {
            previous: PathBuf::from("/Applications/.onda-update-42/Onda.app"),
            why: "no launcher".to_owned(),
        });
        assert!(!fixture.restart().await);
        let stranded = UpdateStatus::Failed(UpdateFailure::Relaunch {
            version: "2.0.42".to_owned(),
        });
        assert_eq!(fixture.updater.snapshot().status, stranded);
        assert!(
            fixture.updater.shared.lock().staged.is_some(),
            "retain the staging receipt lock until this process exits"
        );
        assert!(
            fixture.logged("discard").is_empty(),
            "the previous app stays"
        );

        fixture.publish(43);
        assert_eq!(fixture.check().await, stranded, "nothing more is tried");
        assert_eq!(fixture.logged("stage").len(), 1);
        assert_eq!(fixture.host.terminated.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_update_survives_failed_checks() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;
        fixture
            .server
            .put("/update.json", Vec::new(), Behavior::Fail);
        assert_eq!(fixture.check().await, Fixture::ready("2.0.42"));
        assert!(fixture.logged("discard").is_empty());

        // a newer release whose download fails leaves the staged one
        fixture.publish(43);
        fixture.server.behave(ZIP_PATH, Behavior::Corrupt);
        assert_eq!(fixture.check().await, Fixture::ready("2.0.42"));
        assert!(fixture.logged("discard").is_empty());

        // and a good one replaces it once staged
        fixture.server.behave(ZIP_PATH, Behavior::Serve);
        assert_eq!(fixture.check().await, Fixture::ready("2.0.43"));
        assert_eq!(fixture.logged("stage").len(), 2);
        assert_eq!(fixture.logged("discard"), ["discard 42"]);
        assert!(fixture.restart().await);
        assert_eq!(fixture.logged("apply"), ["apply 43"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn background_checks_continue_while_an_update_is_staged() {
        let schedule = Schedule {
            launch_delay: Duration::from_millis(20),
            poll: Duration::from_millis(20),
            interval: Duration::ZERO,
            restart_refresh: Duration::from_secs(5),
        };
        let fixture =
            Fixture::scheduled(41, true, true, tempfile::tempdir().unwrap(), schedule).await;
        fixture.publish(42);
        fixture
            .settle(|status| *status == Fixture::ready("2.0.42"))
            .await;
        fixture.publish(43);
        fixture
            .settle(|status| *status == Fixture::ready("2.0.43"))
            .await;
        assert_eq!(fixture.logged("discard"), ["discard 42"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_refreshes_the_feed_first() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;

        // a newer release takes the restart's place
        fixture.publish(43);
        assert!(!fixture.restart().await);
        assert_eq!(fixture.updater.snapshot().status, Fixture::ready("2.0.43"));
        assert!(fixture.logged("apply").is_empty());

        // offline: the staged update installs anyway
        fixture
            .server
            .put("/update.json", Vec::new(), Behavior::Fail);
        assert!(fixture.restart().await);
        assert_eq!(fixture.logged("apply"), ["apply 43"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_does_not_wait_again_for_a_newer_update_that_failed() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;
        fixture.publish(43);
        fixture.server.behave(ZIP_PATH, Behavior::Corrupt);
        assert!(!fixture.restart().await, "tries the newer one first");
        assert_eq!(fixture.updater.snapshot().status, Fixture::ready("2.0.42"));
        assert!(fixture.restart().await, "then installs what is staged");
        assert_eq!(fixture.logged("apply"), ["apply 42"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_respects_a_feed_that_removes_this_platform() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;
        let release = Release {
            schema: SCHEMA,
            app: crate::BUNDLE_ID.to_owned(),
            channel: CHANNEL.to_owned(),
            version: "2.0.43".to_owned(),
            build: 43,
            published_at: "2026-11-01T12:00:00Z".to_owned(),
            notes_url: None,
            artifacts: Vec::new(),
        };
        fixture.server.put(
            "/update.json",
            feed::seal(&release, &signing_key()),
            Behavior::Serve,
        );
        assert!(!fixture.restart().await);
        assert!(fixture.logged("apply").is_empty());
        assert_eq!(fixture.logged("discard"), ["discard 42"]);
        assert_eq!(fixture.updater.snapshot().status, UpdateStatus::UpToDate);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_claimed_restart_keeps_checks_away() {
        let fixture = Fixture::new(41, true).await;
        fixture.publish(42);
        fixture.check().await;
        fixture
            .server
            .put("/update.json", Vec::new(), Behavior::Fail);
        // claimed but not yet run: a check queued now must not touch it
        fixture.updater.shared.lock().status = UpdateStatus::Installing {
            version: "2.0.42".to_owned(),
        };
        let before = fixture.server.requests("/update.json").len();
        fixture.check().await;
        assert_eq!(fixture.server.requests("/update.json").len(), before);
        assert!(!fixture.restart().await, "only Ready can be claimed");
        fixture.updater.shared.lock().status = Fixture::ready("2.0.42");
        assert!(fixture.restart().await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_only_install_offers_the_download() {
        let fixture = Fixture::new(41, false).await;
        fixture.publish(42);
        assert_eq!(
            fixture.check().await,
            UpdateStatus::Available {
                version: "2.0.42".to_owned()
            }
        );
        assert!(fixture.server.requests(ZIP_PATH).is_empty());
        fixture.updater.open_download();
        assert_eq!(
            *fixture.host.opened.lock().unwrap(),
            vec![fixture.server.url(DMG_PATH)]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn background_checks_follow_the_preference_and_isolation() {
        let isolated = Fixture::new(41, true).await;
        isolated.publish(42);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(isolated.server.requests("/update.json").is_empty());

        let dir = tempfile::tempdir().unwrap();
        Prefs {
            automatic_checks: false,
            last_check: None,
            highest_build_seen: 0,
        }
        .save(&dir.path().join("updater.json"))
        .unwrap();
        let off = Fixture::with(41, true, true, dir).await;
        off.publish(42);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(off.server.requests("/update.json").is_empty());
        assert!(!off.updater.snapshot().automatic);

        let on = Fixture::with(41, true, true, tempfile::tempdir().unwrap()).await;
        on.publish(42);
        on.settle(|s| matches!(s, UpdateStatus::Ready { .. })).await;
        assert_eq!(on.server.requests("/update.json").len(), 1);
        assert_eq!(
            on.installer.log().first().map(String::as_str),
            Some("clean")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_automatic_preference_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let fixture = Fixture::with(41, true, false, dir).await;
        assert!(fixture.updater.snapshot().automatic);
        fixture.updater.set_automatic_checks(false);
        assert!(!fixture.updater.snapshot().automatic);
        assert!(!Prefs::load(&path.join("updater.json"), true).automatic_checks);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_preference_changes_and_checks_keep_both_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("updater.json");
        let fixture = Fixture::with(40, true, false, dir).await;
        for build in 41..=50 {
            fixture.publish(build);
            let updater = fixture.updater.clone();
            let toggles = std::thread::spawn(move || {
                for index in 0..50 {
                    updater.set_automatic_checks(index % 2 == 0);
                }
                updater.set_automatic_checks(build % 2 == 0);
            });
            fixture.check().await;
            toggles.join().unwrap();
            let on_disk = Prefs::load(&file, true);
            assert_eq!(on_disk.highest_build_seen, build);
            assert_eq!(on_disk.automatic_checks, build % 2 == 0);
            assert_eq!(on_disk, fixture.updater.shared.lock().prefs);
        }
    }
}
