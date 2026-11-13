//! Signed update checks with downloads matching the installed package format.

use std::{path::Path, sync::Arc};

use onda_app::{ports::Updater, views::UpdaterState};
use onda_update::{
    Config, DownloadTarget, Schedule,
    feed::{self, Origins, OsVersion},
};

use super::{Message, process, send};

pub struct Updates(onda_update::Updater);

impl Updates {
    pub fn new(
        data: &Path,
        events: async_channel::Sender<Message>,
        runtime: &tokio::runtime::Handle,
    ) -> anyhow::Result<Self> {
        let executable = std::env::current_exe()?;
        let kind = package_kind(&executable);
        let config = Config {
            feed_url: onda_update::FEED_URL.into(),
            origins: Origins::github(),
            public_key: feed::verifying_key(onda_update::PUBLIC_KEY).map_err(anyhow::Error::msg)?,
            build: 0,
            os_version: OsVersion::parse("0").expect("constant OS version"),
            state_dir: data.join("updates"),
            background_checks: std::env::var_os("ONDA_DATA_DIR").is_none(),
            automatic_default: true,
            schedule: Schedule::default(),
            user_agent: concat!("Onda/", env!("ONDA_VERSION"), " (Linux)").into(),
        };
        let target = DownloadTarget {
            version: env!("ONDA_VERSION").parse()?,
            os: "linux".into(),
            arch: onda_update::ARCH.into(),
            kind: kind.into(),
        };
        Ok(Self(
            onda_update::Updater::start_download_only(
                config,
                target,
                Arc::new(DesktopHost(events)),
                runtime,
            )
            .map_err(anyhow::Error::msg)?,
        ))
    }
}

fn package_kind(executable: &Path) -> &'static str {
    let kind = executable
        .parent()
        .and_then(Path::parent)
        .and_then(|prefix| std::fs::read_to_string(prefix.join("lib/onda/package-kind")).ok());
    match kind.as_deref().map(str::trim) {
        Some("deb") => "deb",
        Some("rpm") => "rpm",
        Some("pkg.tar.zst") => "pkg.tar.zst",
        _ => "tar.gz",
    }
}

impl Updater for Updates {
    fn state(&self) -> UpdaterState {
        let state = self.0.snapshot();
        UpdaterState {
            status: state.status,
            automatic: state.automatic,
            last_check: state.last_check,
        }
    }
    fn check_now(&self) {
        self.0.check_now();
    }
    fn set_automatic_checks(&self, enabled: bool) {
        self.0.set_automatic_checks(enabled);
    }
    fn restart_to_update(&self) -> bool {
        false
    }
    fn open_download(&self) {
        self.0.open_download();
    }
}

struct DesktopHost(async_channel::Sender<Message>);

impl onda_update::Host for DesktopHost {
    fn changed(&self) {
        send(&self.0, Message::UpdaterChanged);
    }
    fn open_url(&self, url: &str) {
        if let Err(error) = process::open_uri(url) {
            send(
                &self.0,
                Message::Notice("Could not open update download".into(), error.to_string()),
            );
        }
    }
    fn terminate(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_identity_follows_the_executable_prefix() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("bin/onda");
        assert_eq!(package_kind(&executable), "tar.gz");
        let library = root.path().join("lib/onda");
        std::fs::create_dir_all(&library).unwrap();
        for kind in ["deb", "rpm", "pkg.tar.zst", "tar.gz"] {
            std::fs::write(library.join("package-kind"), format!("{kind}\n")).unwrap();
            assert_eq!(package_kind(&executable), kind);
        }
        std::fs::write(library.join("package-kind"), "unknown").unwrap();
        assert_eq!(package_kind(&executable), "tar.gz");
    }

    #[test]
    fn package_identity_survives_documentation_removal() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("lib/onda");
        let docs = root.path().join("share/doc/onda");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(library.join("package-kind"), "pkg.tar.zst\n").unwrap();
        std::fs::write(docs.join("README.txt"), "Greendale installation guide").unwrap();
        std::fs::remove_dir_all(root.path().join("share/doc")).unwrap();
        assert_eq!(package_kind(&root.path().join("bin/onda")), "pkg.tar.zst");
    }
}
