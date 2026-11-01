//! macOS installs (UPD-6, UPD-7).
//!
//! The verified ZIP is unpacked into a new `.onda-update-…` folder beside
//! the installed bundle, so the swap stays on one volume. The folder holds a
//! receipt naming the installed bundle's canonical path; the process that
//! staged it keeps the receipt `flock`ed for as long as it holds the
//! [`Staged`] value, so a held lock means the folder is in use and a free
//! one means its owner is gone. Cleanup removes only folders whose receipt
//! names this bundle and whose lock is free: never anything without a
//! readable receipt, another copy's folders, or a folder another process
//! still holds.
//!
//! The unpacked app must be a real directory whose Info.plist names Onda's
//! bundle id, the signed version and build, an arm64 executable, and a
//! minimum macOS this Mac meets, and whose code signature satisfies
//! [`crate::CODE_REQUIREMENT`] (Onda's notarized Developer ID). A staged app
//! can wait for days, so the restart checks it again, and checks that the
//! installed bundle is still the running build, while holding a lock on the
//! installed bundle. The two bundles then trade places in one
//! `renameatx_np(RENAME_SWAP)`, and a detached waiter opens the new app once
//! this process has exited. If the waiter cannot start, the swap is undone;
//! if that fails too, the new version stays installed, the previous bundle
//! stays in the staging folder, and [`ApplyError::Stranded`] says where.
//! The next launch removes the previous bundle with the folder.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustix::fs::{Access, CWD, FlockOperation, RenameFlags};
use serde::{Deserialize, Serialize};

use crate::feed::{OsVersion, Release};

/// Staging folders beside the installed bundle start with this.
pub const STAGING_PREFIX: &str = ".onda-update-";

/// The ownership receipt inside a staging folder.
const RECEIPT: &str = "receipt.json";

/// How many times, 50 ms apart, [`try_lock`] tries.
const LOCK_TRIES: u32 = 10;

/// How long the waiter waits for the old process before giving up, in
/// tenths of a second.
const RELAUNCH_WAIT_TENTHS: u32 = 600;

/// The waiter: polls the old process, then runs the given command. Its
/// arguments arrive as positional parameters, never as shell text.
const WAITER: &str = r#"pid="$1"; shift; i=0
while /bin/kill -0 "$pid" 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -gt "$WAIT_TENTHS" ]; then exit 1; fi
  /bin/sleep 0.1
done
exec "$@""#;

/// Why staging or installing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// The unpacked app is not the update the feed promised.
    Rejected(String),
    /// The disk or a system tool failed.
    Failed(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for InstallError {}

/// Why a restart did not install the update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// Nothing changed: the installed app is as it was and the staging
    /// folder still holds the update, which may be discarded.
    NotApplied(InstallError),
    /// The update is installed but the relaunch could not be scheduled and
    /// the swap could not be undone. The staging folder now holds the
    /// running (previous) app and must not be discarded.
    Stranded {
        /// The previous app's bundle.
        previous: PathBuf,
        /// What failed.
        why: String,
    },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotApplied(error) => error.fmt(f),
            Self::Stranded { previous, why } => write!(
                f,
                "{why}; the update is installed and the previous app is at {}",
                previous.display()
            ),
        }
    }
}

impl std::error::Error for ApplyError {}

/// What the unpacked app must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// `CFBundleIdentifier`.
    pub bundle_id: String,
    /// `CFBundleShortVersionString`.
    pub version: String,
    /// `CFBundleVersion`.
    pub build: u64,
    /// The running macOS; the app's `LSMinimumSystemVersion` must not
    /// exceed it.
    pub os_version: OsVersion,
    /// A code requirement the signature must satisfy.
    pub requirement: String,
}

impl Expected {
    /// The expectations for `release` on this Mac.
    pub fn for_release(release: &Release, os_version: OsVersion, requirement: &str) -> Self {
        Self {
            bundle_id: crate::BUNDLE_ID.to_owned(),
            version: release.version.clone(),
            build: release.build,
            os_version,
            requirement: requirement.to_owned(),
        }
    }
}

/// A verified app beside the installed one, waiting for the swap. Its
/// staging folder stays locked to this process until the last clone drops.
#[derive(Debug, Clone)]
pub struct Staged {
    /// The staging folder.
    pub folder: PathBuf,
    /// The unpacked app inside it.
    pub app: PathBuf,
    /// The installed app it replaces, canonical.
    pub installed: PathBuf,
    /// What the app was verified against; the restart checks it again.
    pub expected: Expected,
    /// The locked receipt; `None` only for test doubles.
    receipt: Option<Arc<fs::File>>,
}

impl Staged {
    /// The staged build.
    pub fn build(&self) -> u64 {
        self.expected.build
    }

    /// The staged version.
    pub fn version(&self) -> &str {
        &self.expected.version
    }

    /// A staged update that owns nothing on disk, for test doubles.
    #[cfg(test)]
    pub(crate) fn detached(folder: PathBuf, installed: PathBuf, expected: Expected) -> Self {
        Self {
            app: folder.join("Onda.app"),
            folder,
            installed,
            expected,
            receipt: None,
        }
    }
}

/// Who a staging folder belongs to.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Receipt {
    /// The installed bundle's canonical path.
    installed: PathBuf,
    build: u64,
}

/// The app can replace itself: not translocated, and both the bundle and
/// its folder are writable by this user on a writable volume. Anything
/// else gets the download instead of an escalation (UPD-7).
pub fn replaceable(bundle: &Path) -> bool {
    let translocated = bundle
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation");
    let writable = |path: &Path| rustix::fs::access(path, Access::WRITE_OK).is_ok();
    !translocated
        && bundle.is_absolute()
        && bundle.parent().is_some_and(writable)
        && writable(bundle)
}

/// Unpacks `zip` (already checked against the feed) into a new staging
/// folder beside `installed` and verifies the app inside. A failure removes
/// the new folder; nothing that was already there is touched.
pub fn stage(zip: &Path, installed: &Path, expected: &Expected) -> Result<Staged, InstallError> {
    let installed =
        fs::canonicalize(installed).map_err(|error| failed("resolve", installed, &error))?;
    let parent = installed
        .parent()
        .ok_or_else(|| InstallError::Failed("the app has no parent folder".to_owned()))?;
    let (folder, receipt) = claim_folder(parent, &installed, expected.build)?;
    let staged = Staged {
        app: folder.join("Onda.app"),
        folder,
        installed,
        expected: expected.clone(),
        receipt: Some(Arc::new(receipt)),
    };
    let result = unpack(zip, &staged.folder).and_then(|()| verify_bundle(&staged.app, expected));
    if let Err(error) = result {
        discard(&staged);
        return Err(error);
    }
    Ok(staged)
}

/// Creates a new staging folder in `parent` and its locked receipt.
fn claim_folder(
    parent: &Path,
    installed: &Path,
    build: u64,
) -> Result<(PathBuf, fs::File), InstallError> {
    static ATTEMPT: AtomicU32 = AtomicU32::new(0);
    let receipt = serde_json::to_vec(&Receipt {
        installed: installed.to_path_buf(),
        build,
    })
    .map_err(|error| InstallError::Failed(format!("could not write the receipt: {error}")))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    for _ in 0..16 {
        let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "{STAGING_PREFIX}{build}-{}-{nanos:x}-{attempt}",
            std::process::id()
        );
        let folder = parent.join(name);
        match fs::create_dir(&folder) {
            Ok(()) => {}
            // never reuse a folder that was already there
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(failed("create", &folder, &error)),
        }
        let path = folder.join(RECEIPT);
        let written = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| {
                // lock before writing: a reader that sees the receipt
                // empty or locked leaves the folder alone
                rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive)?;
                file.write_all(&receipt)?;
                file.sync_all()?;
                Ok(file)
            });
        return match written {
            Ok(file) => Ok((folder, file)),
            Err(error) => {
                if let Err(cleanup) = remove_staging_folder(&folder) {
                    tracing::warn!(error = %cleanup, "could not remove a new staging folder");
                }
                Err(failed("write", &path, &error))
            }
        };
    }
    Err(InstallError::Failed(
        "could not create a staging folder".to_owned(),
    ))
}

/// Opens and locks `folder`'s receipt and returns it if it names
/// `installed`: the folder is this install's and no process holds it.
fn owned_and_free(folder: &Path, installed: &Path) -> Option<fs::File> {
    let path = folder.join(RECEIPT);
    let is_file = fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file());
    if !is_file {
        return None;
    }
    let mut file = fs::File::open(&path).ok()?;
    if !try_lock(&file) {
        return None;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let receipt: Receipt = serde_json::from_slice(&bytes).ok()?;
    (receipt.installed == installed).then_some(file)
}

/// Takes an exclusive `flock` on `file`. A child spawned at the wrong
/// moment holds a copy of every lock until it execs, so contention gets a
/// moment to clear before this gives up.
fn try_lock(file: &fs::File) -> bool {
    for _ in 1..LOCK_TRIES {
        if rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive).is_ok()
}

/// Removes a staged update that will not be installed.
pub fn discard(staged: &Staged) {
    if let Err(error) = check_owned(staged) {
        tracing::warn!(%error, "leaving a staging folder whose ownership changed");
        return;
    }
    if let Err(error) = remove_staging_folder(&staged.folder) {
        tracing::warn!(%error, "could not remove a staged update");
    }
}

/// Removes this install's staging folders that no process holds: the
/// previous bundle after an update, or an update that was never applied.
/// Folders of other copies, folders still in use, and anything without
/// this install's receipt stay.
pub fn remove_leftovers(installed: &Path) {
    let Ok(installed) = fs::canonicalize(installed) else {
        return;
    };
    let Some(parent) = installed.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let named = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if !named || !is_dir {
            continue;
        }
        let folder = entry.path();
        // held until the folder is gone, so nobody claims it meanwhile
        let Some(_receipt) = owned_and_free(&folder, &installed) else {
            continue;
        };
        if let Err(error) = remove_staging_folder(&folder) {
            tracing::warn!(%error, "could not remove an earlier update");
        }
    }
}

/// Swaps the staged app in and starts the waiter that opens it once process
/// `pid` exits, with `open_args` before the bundle path. `running_build` is
/// this process's build; the installed bundle must still be it.
pub fn apply(
    staged: &Staged,
    running_build: u64,
    pid: u32,
    open_args: &[OsString],
) -> Result<(), ApplyError> {
    let mut command = vec![OsString::from("/usr/bin/open")];
    command.extend(open_args.iter().cloned());
    command.push(staged.installed.clone().into_os_string());
    apply_with(staged, running_build, &swap, || {
        run_after_exit(pid, &command)
    })
}

/// [`apply`] with the swap and the relaunch injected.
fn apply_with(
    staged: &Staged,
    running_build: u64,
    swap: &dyn Fn(&Path, &Path) -> Result<(), InstallError>,
    relaunch: impl FnOnce() -> Result<(), InstallError>,
) -> Result<(), ApplyError> {
    let _installed =
        lock_installed(&staged.installed, running_build).map_err(ApplyError::NotApplied)?;
    check_owned(staged).map_err(ApplyError::NotApplied)?;
    verify_bundle(&staged.app, &staged.expected).map_err(ApplyError::NotApplied)?;
    swap(&staged.app, &staged.installed).map_err(ApplyError::NotApplied)?;
    let Err(error) = relaunch() else {
        return Ok(());
    };
    tracing::error!(%error, "could not schedule the relaunch; undoing the swap");
    match swap(&staged.app, &staged.installed) {
        Ok(()) => Err(ApplyError::NotApplied(error)),
        Err(undo) => Err(ApplyError::Stranded {
            previous: staged.app.clone(),
            why: format!("{error}, and undoing the swap failed: {undo}"),
        }),
    }
}

/// Locks the installed bundle against another process applying an update
/// to it and checks that it is still `running_build`: if another instance
/// already replaced it, or someone else did, installing would go backwards.
fn lock_installed(installed: &Path, running_build: u64) -> Result<fs::File, InstallError> {
    let bundle = fs::File::open(installed).map_err(|error| failed("open", installed, &error))?;
    if !try_lock(&bundle) {
        return Err(InstallError::Failed(
            "another Onda is updating this copy".to_owned(),
        ));
    }
    let locked = bundle
        .metadata()
        .map_err(|error| failed("inspect", installed, &error))?;
    let current =
        fs::symlink_metadata(installed).map_err(|error| failed("inspect", installed, &error))?;
    let build = plist_value(&installed.join("Contents/Info.plist"), "CFBundleVersion");
    let same = (locked.dev(), locked.ino()) == (current.dev(), current.ino())
        && build.as_deref() == Ok(running_build.to_string().as_str());
    if !same {
        return Err(InstallError::Failed(format!(
            "{} is no longer build {running_build}",
            installed.display()
        )));
    }
    Ok(bundle)
}

/// The staging folder is still the one this process staged and locked,
/// with its receipt naming the installed bundle.
fn check_owned(staged: &Staged) -> Result<(), InstallError> {
    let refused = |why: &str| {
        Err(InstallError::Rejected(format!(
            "the staged update at {} {why}",
            staged.folder.display()
        )))
    };
    let Some(receipt) = &staged.receipt else {
        return refused("has no receipt");
    };
    let held = receipt
        .metadata()
        .map_err(|error| failed("inspect", &staged.folder, &error))?;
    let Ok(on_disk) = fs::symlink_metadata(staged.folder.join(RECEIPT)) else {
        return refused("lost its receipt");
    };
    if (held.dev(), held.ino()) != (on_disk.dev(), on_disk.ino()) {
        return refused("has a replaced receipt");
    }
    let mut bytes = Vec::new();
    fs::File::open(staged.folder.join(RECEIPT))
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| failed("read", &staged.folder, &error))?;
    let want = Receipt {
        installed: staged.installed.clone(),
        build: staged.expected.build,
    };
    match serde_json::from_slice::<Receipt>(&bytes) {
        Ok(receipt) if receipt == want => Ok(()),
        _ => refused("belongs to another install"),
    }
}

/// Starts a detached waiter that runs `command` once process `pid` exits,
/// or gives up after a minute.
pub fn run_after_exit(pid: u32, command: &[OsString]) -> Result<(), InstallError> {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(WAITER)
        .arg("onda-relaunch")
        .arg(pid.to_string())
        .args(command)
        .env("WAIT_TENTHS", RELAUNCH_WAIT_TENTHS.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // its own process group, so nothing aimed at Onda's reaches it
        .process_group(0)
        .spawn()
        .map(drop)
        .map_err(|error| InstallError::Failed(format!("could not start the relauncher: {error}")))
}

/// Exchanges two paths in one step.
fn swap(a: &Path, b: &Path) -> Result<(), InstallError> {
    rustix::fs::renameat_with(CWD, a, CWD, b, RenameFlags::EXCHANGE).map_err(|error| {
        InstallError::Failed(format!(
            "could not swap {} and {}: {error}",
            a.display(),
            b.display()
        ))
    })
}

fn unpack(zip: &Path, folder: &Path) -> Result<(), InstallError> {
    let output = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(zip)
        .arg(folder)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| InstallError::Failed(format!("could not run ditto: {error}")))?;
    if !output.status.success() {
        return Err(InstallError::Rejected(format!(
            "could not unpack the update: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Checks the unpacked app before anything runs from it.
pub fn verify_bundle(app: &Path, expected: &Expected) -> Result<(), InstallError> {
    let rejected = |why: String| Err(InstallError::Rejected(why));
    let is_dir = fs::symlink_metadata(app).is_ok_and(|meta| meta.file_type().is_dir());
    if !is_dir {
        return rejected("the update has no Onda.app folder".to_owned());
    }
    let plist = app.join("Contents/Info.plist");
    let value = |key: &str| plist_value(&plist, key);
    let bundle_id = value("CFBundleIdentifier")?;
    if bundle_id != expected.bundle_id {
        return rejected(format!(
            "the update is {bundle_id}, not {}",
            expected.bundle_id
        ));
    }
    let version = value("CFBundleShortVersionString")?;
    if version != expected.version {
        return rejected(format!(
            "the update is version {version}, not {}",
            expected.version
        ));
    }
    let build = value("CFBundleVersion")?;
    if build != expected.build.to_string() {
        return rejected(format!(
            "the update is build {build}, not {}",
            expected.build
        ));
    }
    let minimum = value("LSMinimumSystemVersion")?;
    match OsVersion::parse(&minimum) {
        Some(minimum) if minimum <= expected.os_version => {}
        _ => return rejected(format!("the update needs macOS {minimum}")),
    }
    let executable = value("CFBundleExecutable")?;
    if executable.contains('/') || executable.starts_with('.') {
        return rejected(format!("the update's executable is {executable:?}"));
    }
    let header = read_head(&app.join("Contents/MacOS").join(&executable))?;
    if !has_arm64(&header) {
        return rejected("the update has no arm64 executable".to_owned());
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(format!("-R={}", expected.requirement.trim()))
        .arg(app)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| InstallError::Failed(format!("could not run codesign: {error}")))?;
    if !output.status.success() {
        return rejected(format!(
            "the update's code signature is not Onda's notarized Developer ID: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn plist_value(plist: &Path, key: &str) -> Result<String, InstallError> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(plist)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| InstallError::Failed(format!("could not run plutil: {error}")))?;
    if !output.status.success() {
        return Err(InstallError::Rejected(format!(
            "the update's Info.plist has no {key}"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn read_head(path: &Path) -> Result<Vec<u8>, InstallError> {
    let mut head = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(4096).read_to_end(&mut head))
        .map_err(|_| InstallError::Rejected("the update's executable is unreadable".to_owned()))?;
    Ok(head)
}

const CPU_TYPE_ARM64: u32 = 0x0100_000c;

/// The Mach-O header (thin or universal) lists an arm64 slice.
fn has_arm64(head: &[u8]) -> bool {
    let word = |at: usize, big: bool| -> Option<u32> {
        let bytes: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
        Some(match big {
            true => u32::from_be_bytes(bytes),
            false => u32::from_le_bytes(bytes),
        })
    };
    match word(0, true) {
        // MH_MAGIC_64 stored little-endian
        Some(0xcffa_edfe) => word(4, false) == Some(CPU_TYPE_ARM64),
        // FAT_MAGIC / FAT_MAGIC_64: big-endian arch table
        Some(magic @ (0xcafe_babe | 0xcafe_babf)) => {
            let stride = if magic == 0xcafe_babe { 20 } else { 32 };
            let count = word(4, true).unwrap_or(0).min(16) as usize;
            (0..count).any(|index| word(8 + index * stride, true) == Some(CPU_TYPE_ARM64))
        }
        _ => false,
    }
}

/// Removes a staging folder, which must be named like one.
fn remove_staging_folder(folder: &Path) -> Result<(), InstallError> {
    let named = folder
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(STAGING_PREFIX));
    if !named {
        return Err(InstallError::Failed(format!(
            "refused to remove {}",
            folder.display()
        )));
    }
    match fs::symlink_metadata(folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed("inspect", folder, &error)),
        Ok(meta) if meta.file_type().is_dir() => {
            make_writable(folder);
            fs::remove_dir_all(folder).map_err(|error| failed("remove", folder, &error))
        }
        Ok(_) => fs::remove_file(folder).map_err(|error| failed("remove", folder, &error)),
    }
}

/// Bundles can carry read-only folders, which `remove_dir_all` cannot
/// empty; give this user write access to the folders inside first.
fn make_writable(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    if let Ok(meta) = fs::symlink_metadata(folder) {
        let mut permissions = meta.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        let _ = fs::set_permissions(folder, permissions);
    }
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            make_writable(&entry.path());
        }
    }
}

fn failed(action: &str, path: &Path, error: &std::io::Error) -> InstallError {
    InstallError::Failed(format!("could not {action} {}: {error}", path.display()))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Instant;

    use super::*;

    const TEST_ID: &str = "com.rselbach.onda";
    const TEST_REQUIREMENT: &str = "identifier \"com.rselbach.onda\"";

    /// A minimal ad hoc signed Onda.app in `root`.
    pub(crate) fn bundle(root: &Path, id: &str, version: &str, build: u64) -> PathBuf {
        let app = root.join("Onda.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::copy("/usr/bin/true", app.join("Contents/MacOS/Onda")).unwrap();
        fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>{id}</string>\
                 <key>CFBundleExecutable</key><string>Onda</string>\
                 <key>CFBundleShortVersionString</key><string>{version}</string>\
                 <key>CFBundleVersion</key><string>{build}</string>\
                 <key>LSMinimumSystemVersion</key><string>26.0</string>\
                 </dict></plist>"
            ),
        )
        .unwrap();
        let status = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--identifier", id])
            .arg(&app)
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        app
    }

    /// Zips `app` the way the release does.
    pub(crate) fn zip(app: &Path, out: &Path) {
        let status = Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(app)
            .arg(out)
            .status()
            .unwrap();
        assert!(status.success());
    }

    pub(crate) fn expected(version: &str, build: u64) -> Expected {
        Expected {
            bundle_id: TEST_ID.to_owned(),
            version: version.to_owned(),
            build,
            os_version: OsVersion::parse("26.0").unwrap(),
            requirement: TEST_REQUIREMENT.to_owned(),
        }
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn the_developer_id_requirement_compiles() {
        let status = Command::new("/usr/bin/csreq")
            .arg(format!("-r={}", crate::CODE_REQUIREMENT.trim()))
            .arg("-t")
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn only_the_promised_app_passes() {
        let root = scratch();
        let app = bundle(&root.path().join("good"), TEST_ID, "2.0.0", 42);
        verify_bundle(&app, &expected("2.0.0", 42)).unwrap();

        let cases = [
            ("another version", expected("2.0.1", 42)),
            ("another build", expected("2.0.0", 43)),
            (
                "too old an OS",
                Expected {
                    os_version: OsVersion::parse("25.4").unwrap(),
                    ..expected("2.0.0", 42)
                },
            ),
            (
                "an ad hoc signature is not Onda's Developer ID",
                Expected {
                    requirement: crate::CODE_REQUIREMENT.to_owned(),
                    ..expected("2.0.0", 42)
                },
            ),
        ];
        for (name, tc) in cases {
            let got = verify_bundle(&app, &tc);
            assert!(
                matches!(got, Err(InstallError::Rejected(_))),
                "{name}: {got:?}"
            );
        }

        let other = bundle(
            &root.path().join("other"),
            "edu.greendale.onda",
            "2.0.0",
            42,
        );
        assert!(verify_bundle(&other, &expected("2.0.0", 42)).is_err());

        fs::write(app.join("Contents/MacOS/Onda"), b"Chang was here").unwrap();
        assert!(verify_bundle(&app, &expected("2.0.0", 42)).is_err());
    }

    #[test]
    fn mach_o_headers_name_their_architectures() {
        let thin = |cpu: u32| {
            let mut head = 0xfeed_facf_u32.to_le_bytes().to_vec();
            head.extend(cpu.to_le_bytes());
            head
        };
        let fat = |cpus: &[u32]| {
            let mut head = 0xcafe_babe_u32.to_be_bytes().to_vec();
            head.extend((cpus.len() as u32).to_be_bytes());
            for cpu in cpus {
                head.extend(cpu.to_be_bytes());
                head.extend([0; 16]);
            }
            head
        };
        assert!(has_arm64(&thin(CPU_TYPE_ARM64)));
        assert!(!has_arm64(&thin(0x0100_0007)));
        assert!(has_arm64(&fat(&[0x0100_0007, CPU_TYPE_ARM64])));
        assert!(!has_arm64(&fat(&[0x0100_0007])));
        assert!(!has_arm64(b"#!/bin/sh\n"));
        assert!(has_arm64(&fs::read("/usr/bin/true").unwrap()));
    }

    fn version(app: &Path) -> String {
        plist_value(&app.join("Contents/Info.plist"), "CFBundleVersion").unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// An installed build 41 in `root/Applications` and a zip of build 42.
    fn installed_and_update(root: &Path) -> (PathBuf, PathBuf) {
        let installed = bundle(&root.join("Applications"), TEST_ID, "1.0.0", 41);
        let update = bundle(&root.join("source"), TEST_ID, "2.0.0", 42);
        let archive = root.join("Onda.zip");
        zip(&update, &archive);
        (installed, archive)
    }

    fn no_relaunch() -> Result<(), InstallError> {
        Ok(())
    }

    #[test]
    fn a_zip_stages_beside_the_app_and_swaps_in() {
        let root = scratch();
        let (installed, archive) = installed_and_update(root.path());
        // a folder that merely looks like staging is never reused or emptied
        let lookalike = installed.with_file_name(format!("{STAGING_PREFIX}42"));
        fs::create_dir_all(lookalike.join("Troy")).unwrap();

        let staged = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        assert_ne!(staged.folder, lookalike);
        assert!(lookalike.join("Troy").exists());
        assert_eq!(staged.installed, fs::canonicalize(&installed).unwrap());

        apply_with(&staged, 41, &swap, no_relaunch).unwrap();
        assert_eq!(version(&installed), "42");
        assert_eq!(version(&staged.app), "41");

        remove_leftovers(&installed);
        assert!(staged.folder.exists(), "still held by this process");
        let folder = staged.folder.clone();
        drop(staged);
        remove_leftovers(&installed);
        assert!(!folder.exists());
        assert!(lookalike.join("Troy").exists());
        assert_eq!(version(&installed), "42");
    }

    #[test]
    fn a_rejected_update_leaves_nothing_behind() {
        let root = scratch();
        let installed = bundle(&root.path().join("Applications"), TEST_ID, "1.0.0", 41);
        let update = bundle(
            &root.path().join("source"),
            "edu.greendale.onda",
            "2.0.0",
            42,
        );
        let archive = root.path().join("Onda.zip");
        zip(&update, &archive);
        let got = stage(&archive, &installed, &expected("2.0.0", 42));
        assert!(matches!(got, Err(InstallError::Rejected(_))), "{got:?}");
        assert_eq!(names(installed.parent().unwrap()), ["Onda.app"]);

        fs::write(&archive, b"not a zip").unwrap();
        assert!(stage(&archive, &installed, &expected("2.0.0", 42)).is_err());
        assert_eq!(names(installed.parent().unwrap()), ["Onda.app"]);
    }

    #[test]
    fn cleanup_removes_only_this_installs_free_staging_folders() {
        let root = scratch();
        let (installed, archive) = installed_and_update(root.path());
        let parent = installed.parent().unwrap().to_path_buf();
        let copy = parent.join("Onda Copy.app");
        fs::create_dir_all(&copy).unwrap();
        fs::create_dir_all(parent.join("Greendale.app/Contents")).unwrap();
        // prefix collisions: no receipt, an unreadable receipt, a file
        fs::create_dir_all(parent.join(".onda-update-7/Contents")).unwrap();
        fs::create_dir_all(parent.join(".onda-update-8")).unwrap();
        fs::write(parent.join(".onda-update-8").join(RECEIPT), b"Pierce").unwrap();
        fs::write(parent.join(".onda-update-9"), b"file").unwrap();

        let ours = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        let theirs = stage(&archive, &copy, &expected("2.0.0", 42)).unwrap();
        let held = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        let (ours_folder, theirs_folder) = (ours.folder.clone(), theirs.folder.clone());
        drop(ours);
        drop(theirs);
        // another process holding its receipt keeps a folder alive
        let other = fs::File::open(held.folder.join(RECEIPT)).unwrap();
        let held_folder = held.folder.clone();
        drop(held);
        rustix::fs::flock(&other, FlockOperation::NonBlockingLockExclusive).unwrap();

        remove_leftovers(&installed);
        assert!(!ours_folder.exists());
        assert!(theirs_folder.exists(), "another copy's update");
        assert!(held_folder.exists(), "held by another updater");
        for name in [".onda-update-7", ".onda-update-8", ".onda-update-9"] {
            assert!(parent.join(name).exists(), "{name}");
        }

        drop(other);
        remove_leftovers(&installed);
        assert!(!held_folder.exists());
        remove_leftovers(&copy);
        assert!(!theirs_folder.exists());
        assert_eq!(
            names(&parent),
            [
                ".onda-update-7",
                ".onda-update-8",
                ".onda-update-9",
                "Greendale.app",
                "Onda Copy.app",
                "Onda.app"
            ]
        );
        assert!(remove_staging_folder(&installed).is_err());
    }

    #[test]
    fn a_tampered_or_replaced_staged_app_is_refused_at_restart() {
        let root = scratch();
        let (installed, archive) = installed_and_update(root.path());

        let tampered = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        fs::write(tampered.app.join("Contents/MacOS/Onda"), b"Chang was here").unwrap();
        let got = apply_with(&tampered, 41, &swap, no_relaunch);
        assert!(
            matches!(got, Err(ApplyError::NotApplied(InstallError::Rejected(_)))),
            "{got:?}"
        );
        assert_eq!(version(&installed), "41");

        let replaced = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        fs::remove_dir_all(&replaced.app).unwrap();
        bundle(&replaced.folder, TEST_ID, "2.0.0", 43);
        let got = apply_with(&replaced, 41, &swap, no_relaunch);
        assert!(matches!(got, Err(ApplyError::NotApplied(_))), "{got:?}");
        assert_eq!(version(&installed), "41");

        let relabeled = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        fs::remove_file(relabeled.folder.join(RECEIPT)).unwrap();
        fs::write(relabeled.folder.join(RECEIPT), b"{}").unwrap();
        let got = apply_with(&relabeled, 41, &swap, no_relaunch);
        assert!(matches!(got, Err(ApplyError::NotApplied(_))), "{got:?}");
        assert_eq!(version(&installed), "41");
        discard(&relabeled);
        assert!(
            relabeled.folder.exists(),
            "a replaced receipt loses cleanup authority"
        );

        let staged = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        let got = apply_with(&staged, 40, &swap, no_relaunch);
        assert!(
            matches!(got, Err(ApplyError::NotApplied(_))),
            "the installed app is no longer the running build: {got:?}"
        );
        assert_eq!(version(&installed), "41");
    }

    #[test]
    fn a_failed_relaunch_undoes_the_swap_or_reports_the_stranded_app() {
        let root = scratch();
        let (installed, archive) = installed_and_update(root.path());
        let no_launcher = || Err(InstallError::Failed("no launcher".to_owned()));

        let staged = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        let got = apply_with(&staged, 41, &swap, no_launcher);
        assert!(matches!(got, Err(ApplyError::NotApplied(_))), "{got:?}");
        assert_eq!(version(&installed), "41");
        assert_eq!(version(&staged.app), "42");

        let swaps = AtomicU32::new(0);
        let undo_fails = |a: &Path, b: &Path| match swaps.fetch_add(1, Ordering::SeqCst) {
            0 => swap(a, b),
            _ => Err(InstallError::Failed("volume went away".to_owned())),
        };
        let got = apply_with(&staged, 41, &undo_fails, no_launcher);
        assert_eq!(
            got.as_ref().err().and_then(|error| match error {
                ApplyError::Stranded { previous, .. } => Some(previous),
                ApplyError::NotApplied(_) => None,
            }),
            Some(&staged.app),
            "{got:?}"
        );
        assert_eq!(version(&installed), "42");
        assert_eq!(version(&staged.app), "41", "the previous app survives");
    }

    #[test]
    fn a_second_restart_of_the_same_copy_waits_its_turn() {
        let root = scratch();
        let (installed, archive) = installed_and_update(root.path());
        let staged = stage(&archive, &installed, &expected("2.0.0", 42)).unwrap();
        let other = lock_installed(&staged.installed, 41).unwrap();
        let got = apply_with(&staged, 41, &swap, no_relaunch);
        assert!(matches!(got, Err(ApplyError::NotApplied(_))), "{got:?}");
        assert_eq!(version(&installed), "41");
        drop(other);
        apply_with(&staged, 41, &swap, no_relaunch).unwrap();
        assert_eq!(version(&installed), "42");
    }

    #[test]
    fn translocated_and_read_only_apps_are_not_replaceable() {
        let root = scratch();
        let app = root.path().join("Onda.app");
        fs::create_dir_all(&app).unwrap();
        assert!(replaceable(&app));
        assert!(!replaceable(
            &root.path().join("AppTranslocation/x/Onda.app")
        ));
        assert!(!replaceable(Path::new(
            "/System/Applications/Calculator.app"
        )));
        assert!(!replaceable(Path::new("Onda.app")));
    }

    #[test]
    fn the_relauncher_waits_for_the_old_process() {
        let root = scratch();
        let marker = root.path().join("relaunched");
        let mut old = Command::new("/bin/sleep").arg("1").spawn().unwrap();
        let started = Instant::now();
        run_after_exit(
            old.id(),
            &[
                OsString::from("/usr/bin/touch"),
                marker.clone().into_os_string(),
            ],
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!marker.exists(), "ran before the old process exited");
        old.wait().unwrap();
        while !marker.exists() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(marker.exists(), "never ran after the old process exited");
    }
}
