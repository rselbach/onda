//! Onda's self-updater (UPD-1..UPD-8).
//!
//! GitHub Pages serves one signed JSON feed ([`FEED_URL`]); GitHub Releases
//! serve the versioned archives it names. The feed is an envelope whose
//! Ed25519 signature, made with the release key ([`PUBLIC_KEY`], also
//! `SUPublicEDKey` in Info.plist), covers the release JSON
//! ([`feed`]). A release newer than the running build is downloaded with
//! resume and checked against the signed size and SHA-256 ([`transport`]),
//! unpacked beside the installed app, checked for Onda's bundle id, version,
//! architecture, minimum macOS, Developer ID signature, and notarization,
//! and swapped in atomically when the user restarts ([`install`]). The
//! [`Updater`] runs the checks and owns the state the UI shows.
//!
//! Linux uses [`Updater::start_download_only`]: the same signed feed and
//! scheduling, with semantic-version comparison and package-specific browser
//! downloads. Installation remains with the user's package manager or installer.

pub mod feed;
pub mod install;
mod prefs;
pub mod transport;
mod updater;

pub use updater::{
    Config, DownloadTarget, Host, Installer, MacInstaller, Schedule, Snapshot, Updater,
};

/// The stable feed on GitHub Pages, next to the legacy Sparkle appcast.
pub const FEED_URL: &str = "https://rselbach.github.io/onda/update.json";

/// The release key's Ed25519 public key, base64. The same key signs the
/// legacy Sparkle appcast's archives; [`feed::SIGNING_CONTEXT`] keeps the two
/// kinds of signature apart.
pub const PUBLIC_KEY: &str = "2obyIySbPw8xSuXQwzSw2sCbOixUww08jk95se82unc=";

/// Onda's bundle identifier, which the feed and the update must name.
pub const BUNDLE_ID: &str = "com.rselbach.onda";

/// What an update's code signature must satisfy: a notarized Developer ID
/// application from Onda's team, identified as Onda. `scripts/release.sh`
/// checks release builds against the same file.
pub const CODE_REQUIREMENT: &str = include_str!("../update-requirement.txt");

/// This build's platform in feed terms.
pub const OS: &str = "macos";

/// This build's architecture in feed terms.
pub const ARCH: &str = match std::env::consts::ARCH.as_bytes() {
    b"aarch64" => "arm64",
    _ => std::env::consts::ARCH,
};
