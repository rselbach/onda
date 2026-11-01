//! Composes `onda-update` behind `onda-app`'s updater port (UPD-1..UPD-8).
//! The shell only describes the running bundle and does what needs
//! AppKit: open a URL and quit.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use onda_app::App;
use onda_app::ports::Updater;
use onda_app::views::UpdaterState;
use onda_update::feed::{self, Origins, OsVersion};
use onda_update::{Config, Host, MacInstaller, Schedule};
use tokio::runtime::Handle;

use crate::{UpdateHost, UpdaterPort, with_app};

/// The updater the core sees; `None` when this build cannot update itself
/// (UPD-4).
pub(crate) struct AppUpdater(Option<onda_update::Updater>);

impl AppUpdater {
    /// Starts the updater for the bundle `host` describes, keeping its state
    /// under `data_dir/updates`.
    pub(crate) fn start(
        host: Option<UpdateHost>,
        port: Arc<dyn UpdaterPort>,
        app: Arc<Mutex<Option<App>>>,
        data_dir: &Path,
        app_version: &str,
        runtime: &Handle,
    ) -> Self {
        let Some(host) = host else {
            tracing::info!("no app bundle; updates are off");
            return Self(None);
        };
        match config(&host, data_dir, app_version) {
            Ok(config) => {
                let open_args = match host.isolated {
                    // the relaunched copy keeps the run's data folder
                    true => vec![
                        OsString::from("--env"),
                        OsString::from(format!("ONDA_DATA_DIR={}", data_dir.display())),
                    ],
                    false => Vec::new(),
                };
                let installer = Arc::new(MacInstaller::new(
                    PathBuf::from(&host.bundle_path),
                    config.build,
                    config.os_version,
                    open_args,
                ));
                let shell = Arc::new(ShellHost { port, app });
                match onda_update::Updater::start(config, installer, shell, runtime) {
                    Ok(updater) => Self(Some(updater)),
                    Err(error) => {
                        tracing::error!(%error, "could not start the updater");
                        Self(None)
                    }
                }
            }
            Err(error) => {
                tracing::info!(%error, "updates are off");
                Self(None)
            }
        }
    }
}

fn config(host: &UpdateHost, data_dir: &Path, app_version: &str) -> Result<Config, String> {
    let build = host
        .build
        .parse()
        .map_err(|_| format!("build {:?} is not a number", host.build))?;
    let os_version = OsVersion::parse(&host.os_version)
        .ok_or_else(|| format!("OS version {:?} is not dotted numbers", host.os_version))?;
    let bundle = Path::new(&host.bundle_path);
    if !bundle.is_absolute() || bundle.extension().is_none_or(|ext| ext != "app") {
        return Err(format!("{} is not an app bundle", bundle.display()));
    }
    Ok(Config {
        feed_url: onda_update::FEED_URL.to_owned(),
        origins: Origins::github(),
        public_key: feed::verifying_key(onda_update::PUBLIC_KEY)?,
        build,
        os_version,
        state_dir: data_dir.join("updates"),
        background_checks: !host.isolated,
        automatic_default: host.legacy_automatic_checks.unwrap_or(true),
        schedule: Schedule::default(),
        user_agent: format!("Onda/{app_version}"),
    })
}

impl Updater for AppUpdater {
    fn state(&self) -> UpdaterState {
        let Some(updater) = &self.0 else {
            return UpdaterState::default();
        };
        let snapshot = updater.snapshot();
        UpdaterState {
            status: snapshot.status,
            automatic: snapshot.automatic,
            last_check: snapshot.last_check,
        }
    }

    fn check_now(&self) {
        if let Some(updater) = &self.0 {
            updater.check_now();
        }
    }

    fn set_automatic_checks(&self, enabled: bool) {
        if let Some(updater) = &self.0 {
            updater.set_automatic_checks(enabled);
        }
    }

    fn restart_to_update(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(onda_update::Updater::restart_to_update)
    }

    fn open_download(&self) {
        if let Some(updater) = &self.0 {
            updater.open_download();
        }
    }
}

struct ShellHost {
    port: Arc<dyn UpdaterPort>,
    app: Arc<Mutex<Option<App>>>,
}

impl Host for ShellHost {
    fn changed(&self) {
        with_app(&self.app, App::updater_changed);
    }

    fn open_url(&self, url: &str) {
        self.port.open_url(url.to_owned());
    }

    fn terminate(&self) {
        self.port.terminate();
    }
}
