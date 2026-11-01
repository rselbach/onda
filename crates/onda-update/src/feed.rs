//! The signed update feed (UPD-5).
//!
//! The feed is one JSON file, so the release and its signature are always
//! published together:
//!
//! ```json
//! { "payload": "<base64 release JSON>", "signature": "<base64 Ed25519>" }
//! ```
//!
//! The signature covers [`SIGNING_CONTEXT`] followed by the decoded payload
//! bytes, so it can never pass for a Sparkle archive signature made with the
//! same key, or the reverse. Only a verified payload is parsed. The release
//! names Onda's bundle id, the channel, a monotonically increasing build
//! number (the CI run number, also `CFBundleVersion`), and one artifact per
//! OS, architecture, and kind, each with its size, SHA-256, minimum OS, and
//! a GitHub Releases URL. Unknown fields are ignored so a later schema can
//! add some; a known field with a bad value rejects the whole feed.

use std::cmp::Ordering;
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use reqwest::Url;
use serde::{Deserialize, Serialize};

/// Prefix of every signed message.
pub const SIGNING_CONTEXT: &[u8] = b"onda-update-feed-v1\n";

/// The only schema this client reads.
pub const SCHEMA: u32 = 1;

/// The only channel there is.
pub const CHANNEL: &str = "stable";

/// The archive the updater installs: a ZIP holding `Onda.app`.
pub const KIND_APP_ZIP: &str = "app-zip";

/// The disk image people install by hand.
pub const KIND_DMG: &str = "dmg";

/// Feeds larger than this are refused unread.
pub const MAX_FEED_BYTES: usize = 64 * 1024;

/// Archives larger than this are refused.
pub const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The signed envelope as published.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Base64 of the release JSON.
    pub payload: String,
    /// Base64 of the Ed25519 signature over [`SIGNING_CONTEXT`] + payload.
    pub signature: String,
}

/// One release, as the payload describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Always [`SCHEMA`].
    pub schema: u32,
    /// The bundle identifier, [`crate::BUNDLE_ID`].
    pub app: String,
    /// Always [`CHANNEL`].
    pub channel: String,
    /// `CFBundleShortVersionString`, e.g. "1.4.0".
    pub version: String,
    /// `CFBundleVersion`; releases are ordered by it alone.
    pub build: u64,
    /// RFC 3339 publication time; informational.
    pub published_at: String,
    /// The GitHub release page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_url: Option<String>,
    /// The files, one per OS, architecture, and kind.
    pub artifacts: Vec<Artifact>,
}

/// One downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// "macos".
    pub os: String,
    /// "arm64".
    pub arch: String,
    /// [`KIND_APP_ZIP`] or [`KIND_DMG`]; other kinds are ignored.
    pub kind: String,
    /// The lowest OS version the app runs on, e.g. "26.0".
    pub minimum_os: String,
    /// Where to download it.
    pub url: String,
    /// Exact size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

impl Release {
    /// The artifact of `kind` for `os` and `arch`, if the release has one.
    pub fn artifact(&self, os: &str, arch: &str, kind: &str) -> Option<&Artifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.os == os && artifact.arch == arch && artifact.kind == kind)
    }
}

/// Where feeds may send downloads and where downloads may redirect.
#[derive(Debug, Clone)]
pub struct Origins {
    /// Every artifact URL must start with this.
    pub artifact_prefix: String,
    /// Hosts a download may redirect to (GitHub serves release assets
    /// from a separate storage host).
    pub redirect_hosts: Vec<String>,
    /// Plain HTTP is refused; only tests turn this off.
    pub https_only: bool,
}

impl Origins {
    /// Onda's GitHub releases.
    pub fn github() -> Self {
        Self {
            artifact_prefix: "https://github.com/rselbach/onda/releases/download/".to_owned(),
            redirect_hosts: [
                "github.com",
                "objects.githubusercontent.com",
                "release-assets.githubusercontent.com",
                "rselbach.github.io",
            ]
            .map(str::to_owned)
            .to_vec(),
            https_only: true,
        }
    }

    /// `url` may be requested directly as an artifact.
    pub fn allows_artifact(&self, url: &str) -> bool {
        let Ok(parsed) = Url::parse(url) else {
            return false;
        };
        self.allows_scheme(&parsed)
            && parsed.as_str() == url
            && url.starts_with(&self.artifact_prefix)
            && url.len() > self.artifact_prefix.len()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && !url[self.artifact_prefix.len()..]
                .split('/')
                .any(|s| s == "..")
    }

    /// A redirect to `url` may be followed.
    pub fn allows_redirect(&self, url: &Url) -> bool {
        self.allows_scheme(url)
            && url.username().is_empty()
            && url.password().is_none()
            && url
                .host_str()
                .is_some_and(|host| self.redirect_hosts.iter().any(|allowed| allowed == host))
    }

    fn allows_scheme(&self, url: &Url) -> bool {
        match url.scheme() {
            "https" => true,
            "http" => !self.https_only,
            _ => false,
        }
    }
}

/// Why a feed was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    /// Too large, not JSON, not base64, or a field is missing or invalid.
    Malformed(String),
    /// The signature does not verify with the pinned key.
    BadSignature,
    /// The release names another app, channel, or schema.
    WrongIdentity(String),
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "malformed update feed: {why}"),
            Self::BadSignature => f.write_str("the update feed's signature does not verify"),
            Self::WrongIdentity(why) => write!(f, "the update feed is not Onda's: {why}"),
        }
    }
}

impl std::error::Error for FeedError {}

/// Parses the base64 public key.
pub fn verifying_key(base64: &str) -> Result<VerifyingKey, String> {
    let bytes = STANDARD
        .decode(base64.trim())
        .map_err(|error| format!("public key is not base64: {error}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "public key is not 32 bytes".to_owned())?;
    VerifyingKey::from_bytes(&bytes).map_err(|error| format!("invalid public key: {error}"))
}

/// Parses a base64 32-byte Ed25519 seed, the format `generate_keys -x`
/// exports and the release workflow's secret holds.
pub fn signing_key(base64: &str) -> Result<SigningKey, String> {
    let bytes = STANDARD
        .decode(base64.trim())
        .map_err(|_| "the signing key is not base64".to_owned())?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "the signing key is not a 32-byte Ed25519 seed".to_owned())?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Signs `release` and returns the published feed bytes.
pub fn seal(release: &Release, key: &SigningKey) -> Vec<u8> {
    // serializing these plain types cannot fail
    let payload = serde_json::to_vec(release).unwrap_or_default();
    let signature = key.sign(&signed_message(&payload));
    let envelope = Envelope {
        payload: STANDARD.encode(&payload),
        signature: STANDARD.encode(signature.to_bytes()),
    };
    let mut bytes = serde_json::to_vec_pretty(&envelope).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// The legacy Sparkle appcast's enclosure attributes for `archive`: a plain
/// Ed25519 signature over the file's bytes, which is what Sparkle's
/// `sign_update` prints.
pub fn sparkle_signature(archive: &[u8], key: &SigningKey) -> String {
    let signature = key.sign(archive);
    format!(
        "sparkle:edSignature=\"{}\" length=\"{}\"",
        STANDARD.encode(signature.to_bytes()),
        archive.len()
    )
}

/// Verifies `feed` with `key` and returns its release, checked against
/// Onda's identity and `origins`. Nothing in an unverified payload is read.
pub fn open(feed: &[u8], key: &VerifyingKey, origins: &Origins) -> Result<Release, FeedError> {
    let malformed = |why: &str| FeedError::Malformed(why.to_owned());
    if feed.len() > MAX_FEED_BYTES {
        return Err(malformed("too large"));
    }
    let envelope: Envelope =
        serde_json::from_slice(feed).map_err(|_| malformed("not an envelope"))?;
    let payload = STANDARD
        .decode(envelope.payload.as_bytes())
        .map_err(|_| malformed("payload is not base64"))?;
    let signature = STANDARD
        .decode(envelope.signature.as_bytes())
        .ok()
        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
        .ok_or_else(|| malformed("signature is not 64 base64 bytes"))?;
    key.verify_strict(
        &signed_message(&payload),
        &Signature::from_bytes(&signature),
    )
    .map_err(|_| FeedError::BadSignature)?;
    let release: Release =
        serde_json::from_slice(&payload).map_err(|error| malformed(&error.to_string()))?;
    check(&release, origins)?;
    Ok(release)
}

fn signed_message(payload: &[u8]) -> Vec<u8> {
    [SIGNING_CONTEXT, payload].concat()
}

fn check(release: &Release, origins: &Origins) -> Result<(), FeedError> {
    let identity = |why: String| Err(FeedError::WrongIdentity(why));
    let malformed = |why: String| Err(FeedError::Malformed(why));
    if release.schema != SCHEMA {
        return identity(format!("schema {}", release.schema));
    }
    if release.app != crate::BUNDLE_ID {
        return identity(format!("app {}", release.app));
    }
    if release.channel != CHANNEL {
        return identity(format!("channel {}", release.channel));
    }
    if !is_version(&release.version) {
        return malformed(format!("version {:?}", release.version));
    }
    if release.build == 0 {
        return malformed("build 0".to_owned());
    }
    if let Some(notes) = &release.notes_url
        && !notes.starts_with("https://github.com/rselbach/onda/")
    {
        return malformed(format!("notes URL {notes}"));
    }
    for artifact in &release.artifacts {
        if !is_sha256(&artifact.sha256) {
            return malformed(format!("SHA-256 of {}", artifact.url));
        }
        if artifact.size == 0 || artifact.size > MAX_ARTIFACT_BYTES {
            return malformed(format!("size of {}", artifact.url));
        }
        if OsVersion::parse(&artifact.minimum_os).is_none() {
            return malformed(format!("minimum OS {:?}", artifact.minimum_os));
        }
        if !origins.allows_artifact(&artifact.url) {
            return malformed(format!("artifact URL {}", artifact.url));
        }
    }
    Ok(())
}

/// A version is shown and compared with the bundle's for equality only;
/// builds order releases. It names staging folders, so it must be a safe
/// file name.
fn is_version(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text.as_bytes()[0].is_ascii_digit()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A dotted OS version such as "26.0.1"; missing parts count as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsVersion([u32; 3]);

impl OsVersion {
    /// Parses "26", "26.1", or "26.1.2".
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = [0; 3];
        for (index, part) in text.trim().split('.').enumerate() {
            let slot = parts.get_mut(index)?;
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        Some(Self(parts))
    }
}

impl PartialOrd for OsVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OsVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

/// The running install, as the update decision sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// `CFBundleVersion`.
    pub build: u64,
    /// The running OS version.
    pub os_version: OsVersion,
    /// The app can replace itself (UPD-7).
    pub replaceable: bool,
}

/// What to do about a verified release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The release is not newer, or has nothing for this platform.
    UpToDate,
    /// The release needs a newer OS.
    RequiresNewerOs {
        /// Its minimum OS, as published.
        minimum_os: String,
    },
    /// Download and stage this archive.
    Install(Artifact),
    /// This copy cannot replace itself; point the user at this download.
    DownloadOnly(Artifact),
}

/// Decides what `release` means for `install`. Only a strictly higher build
/// is an update, so a replayed older feed can never downgrade Onda.
pub fn decide(release: &Release, install: &Install) -> Decision {
    if release.build <= install.build {
        return Decision::UpToDate;
    }
    let zip = release.artifact(crate::OS, crate::ARCH, KIND_APP_ZIP);
    let dmg = release.artifact(crate::OS, crate::ARCH, KIND_DMG);
    let chosen = match install.replaceable {
        true => zip,
        false => dmg.or(zip),
    };
    let Some(artifact) = chosen else {
        return Decision::UpToDate;
    };
    // checked by `open`
    let minimum = OsVersion::parse(&artifact.minimum_os).unwrap_or(OsVersion([u32::MAX; 3]));
    if minimum > install.os_version {
        return Decision::RequiresNewerOs {
            minimum_os: artifact.minimum_os.clone(),
        };
    }
    match install.replaceable {
        true => Decision::Install(artifact.clone()),
        false => Decision::DownloadOnly(artifact.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn artifact(kind: &str, minimum_os: &str) -> Artifact {
        Artifact {
            os: "macos".to_owned(),
            arch: "arm64".to_owned(),
            kind: kind.to_owned(),
            minimum_os: minimum_os.to_owned(),
            url: format!(
                "https://github.com/rselbach/onda/releases/download/v2.0.0/Onda-2.0.0.{kind}"
            ),
            size: 1024,
            sha256: SHA.to_owned(),
        }
    }

    fn release() -> Release {
        Release {
            schema: SCHEMA,
            app: crate::BUNDLE_ID.to_owned(),
            channel: CHANNEL.to_owned(),
            version: "2.0.0".to_owned(),
            build: 42,
            published_at: "2026-11-01T12:00:00Z".to_owned(),
            notes_url: Some("https://github.com/rselbach/onda/releases/tag/v2.0.0".to_owned()),
            artifacts: vec![artifact(KIND_APP_ZIP, "26.0"), artifact(KIND_DMG, "26.0")],
        }
    }

    fn open_with(feed: &[u8], seed: u8) -> Result<Release, FeedError> {
        open(feed, &key(seed).verifying_key(), &Origins::github())
    }

    fn reseal_payload(feed: &[u8], edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut envelope: Envelope = serde_json::from_slice(feed).unwrap();
        let mut payload = STANDARD.decode(&envelope.payload).unwrap();
        edit(&mut payload);
        envelope.payload = STANDARD.encode(payload);
        serde_json::to_vec(&envelope).unwrap()
    }

    #[test]
    fn a_sealed_release_opens_with_the_matching_key_only() {
        let feed = seal(&release(), &key(1));
        assert_eq!(open_with(&feed, 1).unwrap(), release());
        assert_eq!(open_with(&feed, 2), Err(FeedError::BadSignature));
    }

    #[test]
    fn tampering_with_the_payload_breaks_the_signature() {
        let feed = seal(&release(), &key(1));
        let tampered = reseal_payload(&feed, |payload| {
            let text = String::from_utf8(payload.clone()).unwrap();
            *payload = text.replace("\"build\":42", "\"build\":43").into_bytes();
        });
        assert_eq!(open_with(&tampered, 1), Err(FeedError::BadSignature));
    }

    #[test]
    fn a_sparkle_signature_never_passes_as_a_feed_signature() {
        let payload = serde_json::to_vec(&release()).unwrap();
        let raw = key(1).sign(&payload);
        let envelope = Envelope {
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(raw.to_bytes()),
        };
        let feed = serde_json::to_vec(&envelope).unwrap();
        assert_eq!(open_with(&feed, 1), Err(FeedError::BadSignature));
    }

    #[test]
    fn identity_and_fields_are_checked_after_the_signature() {
        // the edit, and whether the release then names the wrong identity
        // (else it is malformed)
        type Edit = fn(&mut Release);
        let cases: HashMap<&str, (Edit, bool)> = HashMap::from([
            (
                "another app",
                (
                    (|r: &mut Release| r.app = "edu.greendale.onda".to_owned()) as fn(&mut _),
                    true,
                ),
            ),
            ("another channel", (|r| r.channel = "beta".to_owned(), true)),
            ("a future schema", (|r| r.schema = 2, true)),
            (
                "an unsafe version",
                (|r| r.version = "../2.0.0".to_owned(), false),
            ),
            (
                "a bad hash",
                (|r| r.artifacts[0].sha256 = "AB".repeat(32), false),
            ),
            ("an empty artifact", (|r| r.artifacts[0].size = 0, false)),
            (
                "a bad minimum OS",
                (|r| r.artifacts[0].minimum_os = "26.x".to_owned(), false),
            ),
            (
                "a download elsewhere",
                (
                    |r| {
                        r.artifacts[0].url =
                            "https://greendale.edu/rselbach/onda/releases/download/x.zip".to_owned()
                    },
                    false,
                ),
            ),
            (
                "notes elsewhere",
                (
                    |r| r.notes_url = Some("https://greendale.edu/notes".to_owned()),
                    false,
                ),
            ),
        ]);
        for (name, (edit, identity)) in cases {
            let mut tc = release();
            edit(&mut tc);
            let got = open_with(&seal(&tc, &key(1)), 1).unwrap_err();
            let want = match identity {
                true => matches!(got, FeedError::WrongIdentity(_)),
                false => matches!(got, FeedError::Malformed(_)),
            };
            assert!(want, "{name}: {got:?}");
        }
    }

    #[test]
    fn malformed_envelopes_are_refused() {
        let cases = HashMap::from([
            ("not json", b"Troy and Abed in the morning".to_vec()),
            ("empty object", b"{}".to_vec()),
            (
                "short signature",
                br#"{"payload":"e30=","signature":"AAAA"}"#.to_vec(),
            ),
            ("too large", vec![b' '; MAX_FEED_BYTES + 1]),
        ]);
        for (name, feed) in cases {
            assert!(
                matches!(open_with(&feed, 1), Err(FeedError::Malformed(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let feed = seal(&release(), &key(1));
        let extended = reseal_payload(&feed, |payload| {
            let mut value: serde_json::Value = serde_json::from_slice(payload).unwrap();
            value["rollout"] = serde_json::json!({"percent": 50});
            value["artifacts"][0]["delta"] = serde_json::json!(true);
            *payload = serde_json::to_vec(&value).unwrap();
        });
        // re-sign the edited payload the way a newer publisher would
        let envelope: Envelope = serde_json::from_slice(&extended).unwrap();
        let payload = STANDARD.decode(&envelope.payload).unwrap();
        let signature = key(1).sign(&signed_message(&payload));
        let signed = serde_json::to_vec(&Envelope {
            payload: envelope.payload,
            signature: STANDARD.encode(signature.to_bytes()),
        })
        .unwrap();
        assert_eq!(open_with(&signed, 1).unwrap(), release());
    }

    #[test]
    fn artifact_urls_stay_on_onda_releases() {
        let origins = Origins::github();
        let allowed = ["https://github.com/rselbach/onda/releases/download/v2.0.0/Onda-2.0.0.zip"];
        let refused = [
            "http://github.com/rselbach/onda/releases/download/v2.0.0/Onda.zip",
            "https://github.com/rselbach/onda/releases/download/",
            "https://github.com/rselbach/onda/releases/download/../../evil/x.zip",
            "https://github.com/rselbach/onda/releases/download/v2/x.zip?token=1",
            "https://github.com/rselbach/onda/releases/download/v2/x.zip#frag",
            "https://github.com.greendale.edu/rselbach/onda/releases/download/v2/x.zip",
            "https://github.com@greendale.edu/rselbach/onda/releases/download/v2/x.zip",
            "https://github.com/rselbach/onda-evil/releases/download/v2/x.zip",
            "https://github.com/rselbach/onda/releases/download/v2/%2e%2e/x.zip",
        ];
        for url in allowed {
            assert!(origins.allows_artifact(url), "{url}");
        }
        for url in refused {
            assert!(!origins.allows_artifact(url), "{url}");
        }
    }

    #[test]
    fn redirects_stay_on_https_github_hosts() {
        let origins = Origins::github();
        let allowed = [
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1",
        ];
        let refused = [
            "http://objects.githubusercontent.com/x",
            "https://greendale.edu/x",
            "https://user@objects.githubusercontent.com/x",
            "ftp://github.com/x",
        ];
        for url in allowed {
            assert!(origins.allows_redirect(&Url::parse(url).unwrap()), "{url}");
        }
        for url in refused {
            assert!(!origins.allows_redirect(&Url::parse(url).unwrap()), "{url}");
        }
    }

    #[test]
    fn decisions() {
        let install = |build, os: &str, replaceable| Install {
            build,
            os_version: OsVersion::parse(os).unwrap(),
            replaceable,
        };
        let mut newer_os = release();
        newer_os.artifacts[0].minimum_os = "27.1".to_owned();
        let mut dmg_only = release();
        dmg_only.artifacts.retain(|a| a.kind == KIND_DMG);
        let mut other_platform = release();
        for artifact in &mut other_platform.artifacts {
            artifact.arch = "riscv64".to_owned();
        }
        let cases = [
            (
                "same build",
                release(),
                install(42, "26.0", true),
                Decision::UpToDate,
            ),
            (
                "replayed older feed",
                release(),
                install(43, "26.0", true),
                Decision::UpToDate,
            ),
            (
                "newer build installs the zip",
                release(),
                install(41, "26.0", true),
                Decision::Install(artifact(KIND_APP_ZIP, "26.0")),
            ),
            (
                "read-only install gets the dmg",
                release(),
                install(41, "26.0", false),
                Decision::DownloadOnly(artifact(KIND_DMG, "26.0")),
            ),
            (
                "needs a newer OS",
                newer_os.clone(),
                install(41, "27.0.5", true),
                Decision::RequiresNewerOs {
                    minimum_os: "27.1".to_owned(),
                },
            ),
            (
                "a new enough OS",
                newer_os,
                install(41, "27.1", true),
                Decision::Install({
                    let mut a = artifact(KIND_APP_ZIP, "27.1");
                    a.minimum_os = "27.1".to_owned();
                    a
                }),
            ),
            (
                "no zip to install",
                dmg_only,
                install(41, "26.0", true),
                Decision::UpToDate,
            ),
            (
                "another platform",
                other_platform,
                install(41, "26.0", true),
                Decision::UpToDate,
            ),
        ];
        for (name, release, install, want) in cases {
            assert_eq!(decide(&release, &install), want, "{name}");
        }
    }

    #[test]
    fn os_versions_order_numerically() {
        let v = |text| OsVersion::parse(text).unwrap();
        assert!(v("26.10") > v("26.9"));
        assert_eq!(v("26"), v("26.0.0"));
        assert!(v("27.0") > v("26.99.99"));
        for bad in ["", "26.", "26.x", "1.2.3.4", "-1"] {
            assert_eq!(OsVersion::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn signing_keys_are_32_byte_seeds() {
        let seed = STANDARD.encode([7u8; 32]);
        assert_eq!(signing_key(&seed).unwrap().to_bytes(), [7u8; 32]);
        assert!(signing_key(&STANDARD.encode([7u8; 96])).is_err());
        assert!(signing_key("Señor Chang").is_err());
    }

    #[test]
    fn the_pinned_key_parses() {
        verifying_key(crate::PUBLIC_KEY).unwrap();
    }

    /// An independent vector from Sparkle 2.10.1's own tool, so the release
    /// key's seed format and the legacy archive signature are not only
    /// checked against this implementation:
    ///
    /// ```text
    /// printf 'Troy Barnes studies at Greendale Community College.\n' > archive.bin
    /// echo KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio= > key  # 32 × 0x2a, test only
    /// sign_update --ed-key-file key archive.bin
    /// ```
    #[test]
    fn sparkle_sign_update_agrees_on_the_key_and_the_archive_signature() {
        const SEED: &str = "KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio=";
        const ARCHIVE: &[u8] = b"Troy Barnes studies at Greendale Community College.\n";
        const SPARKLE: &str = "sparkle:edSignature=\"a9MFQo4pEzUdbld0uRTT0Xa1i7PWEgGqLL9G+LHd+ycBTxP43leyifKpFwJGb6Qrt5r6jNcwe42u/Fv7ZwLvCQ==\" length=\"52\"";
        let key = signing_key(SEED).unwrap();
        assert_eq!(sparkle_signature(ARCHIVE, &key), SPARKLE);
    }

    #[test]
    fn sparkle_signatures_are_plain_ed25519_over_the_archive() {
        let archive = b"Greendale Human Being archive";
        let text = sparkle_signature(archive, &key(3));
        let encoded = text
            .strip_prefix("sparkle:edSignature=\"")
            .and_then(|rest| rest.split_once('"'))
            .map(|(signature, _)| signature)
            .unwrap();
        let bytes: [u8; 64] = STANDARD.decode(encoded).unwrap().try_into().unwrap();
        key(3)
            .verifying_key()
            .verify_strict(archive, &Signature::from_bytes(&bytes))
            .unwrap();
        assert!(text.ends_with(&format!("length=\"{}\"", archive.len())));
    }
}
