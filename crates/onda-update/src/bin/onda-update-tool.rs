//! Release tooling for the update feed (REL-6). Signing commands read the
//! base64 Ed25519 seed from standard input and refuse a key whose public
//! half is not the one Onda pins, so a wrong secret fails the release
//! instead of publishing a feed no client accepts.
//!
//! ```text
//! onda-update-tool sign-feed --version V --build N --minimum-os X
//!     --published-at RFC3339 --notes-url URL --zip FILE --zip-url URL
//!     --dmg FILE --dmg-url URL [--linux-dir DIR --linux-base-url URL] --out FILE
//! onda-update-tool sparkle-signature FILE
//! onda-update-tool verify-feed FILE [--version V --build N] [--download]
//! ```
//!
//! `verify-feed` opens a feed exactly as the app does; `--download` also
//! fetches every artifact through the app's transport and checks it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ed25519_dalek::SigningKey;
use onda_update::feed::{self, Artifact, KIND_APP_ZIP, KIND_DMG, Origins, Release};
use onda_update::transport::{Transport, sha256_file};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("sign-feed") => sign_feed(&args[1..]),
        Some("sparkle-signature") => sparkle_signature(&args[1..]),
        Some("verify-feed") => verify_feed(&args[1..]),
        _ => Err("usage: onda-update-tool sign-feed|sparkle-signature|verify-feed …".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `--name value` pairs and bare `--flag`s, plus positional arguments.
struct Args {
    options: HashMap<String, String>,
    flags: Vec<String>,
    positional: Vec<String>,
}

impl Args {
    fn parse(args: &[String], flags: &[&str]) -> Result<Self, String> {
        let mut parsed = Self {
            options: HashMap::new(),
            flags: Vec::new(),
            positional: Vec::new(),
        };
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let Some(name) = arg.strip_prefix("--") else {
                parsed.positional.push(arg.clone());
                continue;
            };
            if flags.contains(&name) {
                parsed.flags.push(name.to_owned());
                continue;
            }
            let value = iter
                .next()
                .ok_or_else(|| format!("--{name} needs a value"))?;
            parsed.options.insert(name.to_owned(), value.clone());
        }
        Ok(parsed)
    }

    fn get(&self, name: &str) -> Result<&str, String> {
        self.options
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing --{name}"))
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }
}

fn pinned_key() -> Result<SigningKey, String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| format!("could not read the key from stdin: {error}"))?;
    let key = feed::signing_key(&text)?;
    let pinned = feed::verifying_key(onda_update::PUBLIC_KEY)?;
    if key.verifying_key() != pinned {
        return Err("the signing key does not match onda_update::PUBLIC_KEY".to_owned());
    }
    Ok(key)
}

fn artifact(kind: &str, minimum_os: &str, file: &Path, url: &str) -> Result<Artifact, String> {
    let size = std::fs::metadata(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?
        .len();
    let sha256 =
        sha256_file(file).map_err(|error| format!("could not hash {}: {error}", file.display()))?;
    Ok(Artifact {
        os: onda_update::OS.to_owned(),
        arch: onda_update::ARCH.to_owned(),
        kind: kind.to_owned(),
        minimum_os: minimum_os.to_owned(),
        url: url.to_owned(),
        size,
        sha256,
    })
}

fn sign_feed(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &[])?;
    let build = args
        .get("build")?
        .parse()
        .map_err(|_| "--build is not a number".to_owned())?;
    let minimum_os = args.get("minimum-os")?;
    let mut release = Release {
        schema: feed::SCHEMA,
        app: onda_update::BUNDLE_ID.to_owned(),
        channel: feed::CHANNEL.to_owned(),
        version: args.get("version")?.to_owned(),
        build,
        published_at: args.get("published-at")?.to_owned(),
        notes_url: Some(args.get("notes-url")?.to_owned()),
        artifacts: vec![
            artifact(
                KIND_APP_ZIP,
                minimum_os,
                Path::new(args.get("zip")?),
                args.get("zip-url")?,
            )?,
            artifact(
                KIND_DMG,
                minimum_os,
                Path::new(args.get("dmg")?),
                args.get("dmg-url")?,
            )?,
        ],
    };
    if let Some(directory) = args.options.get("linux-dir") {
        release.artifacts.extend(linux_artifacts(
            Path::new(directory),
            &release.version,
            args.get("linux-base-url")?,
        )?);
    }
    let key = pinned_key()?;
    let sealed = feed::seal(&release, &key);
    // the published bytes must open exactly as the app opens them
    let opened = open_pinned(&sealed)?;
    if opened != release {
        return Err("the sealed feed does not round-trip".to_owned());
    }
    let out = PathBuf::from(args.get("out")?);
    std::fs::write(&out, &sealed)
        .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    println!(
        "signed {} build {} into {}",
        release.version,
        release.build,
        out.display()
    );
    Ok(())
}

fn linux_artifacts(directory: &Path, version: &str, base: &str) -> Result<Vec<Artifact>, String> {
    semver::Version::parse(version).map_err(|_| "invalid Linux release version".to_owned())?;
    let mut artifacts = Vec::new();
    for arch in ["x86_64", "arm64"] {
        for kind in ["deb", "rpm", "pkg.tar.zst", "tar.gz"] {
            let name = format!("Onda-{version}-linux-{arch}.{kind}");
            let mut item = artifact(kind, "0", &directory.join(&name), &format!("{base}/{name}"))?;
            item.os = "linux".into();
            item.arch = arch.into();
            artifacts.push(item);
        }
    }
    Ok(artifacts)
}

fn sparkle_signature(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &[])?;
    let [file] = args.positional.as_slice() else {
        return Err("usage: onda-update-tool sparkle-signature FILE".to_owned());
    };
    let bytes = std::fs::read(file).map_err(|error| format!("could not read {file}: {error}"))?;
    let key = pinned_key()?;
    println!("{}", feed::sparkle_signature(&bytes, &key));
    Ok(())
}

fn open_pinned(bytes: &[u8]) -> Result<Release, String> {
    let key = feed::verifying_key(onda_update::PUBLIC_KEY)?;
    feed::open(bytes, &key, &Origins::github()).map_err(|error| error.to_string())
}

fn verify_feed(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &["download"])?;
    let [file] = args.positional.as_slice() else {
        return Err("usage: onda-update-tool verify-feed FILE [--download]".to_owned());
    };
    let bytes = std::fs::read(file).map_err(|error| format!("could not read {file}: {error}"))?;
    let release = open_pinned(&bytes)?;
    if let Some(version) = args.options.get("version")
        && *version != release.version
    {
        return Err(format!(
            "the feed is version {}, not {version}",
            release.version
        ));
    }
    if let Some(build) = args.options.get("build")
        && *build != release.build.to_string()
    {
        return Err(format!("the feed is build {}, not {build}", release.build));
    }
    for kind in [KIND_APP_ZIP, KIND_DMG] {
        if release
            .artifact(onda_update::OS, onda_update::ARCH, kind)
            .is_none()
        {
            return Err(format!("the feed has no {kind} for this platform"));
        }
    }
    if args.flag("download") {
        download_all(&release)?;
    }
    println!("verified {} build {}", release.version, release.build);
    Ok(())
}

fn download_all(release: &Release) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start a runtime: {error}"))?;
    let transport = Transport::new(Origins::github(), "Onda-release-check")?;
    let dir = std::env::temp_dir().join(format!("onda-update-verify-{}", std::process::id()));
    let result = runtime.block_on(async {
        for artifact in &release.artifacts {
            let file = transport
                .download(artifact, &dir, &|_| {})
                .await
                .map_err(|error| format!("{}: {error}", artifact.url))?;
            println!("downloaded and verified {}", artifact.url);
            std::fs::remove_file(&file)
                .map_err(|error| format!("could not remove {}: {error}", file.display()))?;
        }
        Ok(())
    });
    if let Err(error) = std::fs::remove_dir_all(&dir)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("warning: could not remove {}: {error}", dir.display());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_feed_contains_every_format_and_architecture() {
        let directory = tempfile::tempdir().unwrap();
        let base = "https://github.com/rselbach/onda/releases/download/v3.2.0";
        for arch in ["x86_64", "arm64"] {
            for kind in ["deb", "rpm", "pkg.tar.zst", "tar.gz"] {
                std::fs::write(
                    directory
                        .path()
                        .join(format!("Onda-3.2.0-linux-{arch}.{kind}")),
                    b"Greendale",
                )
                .unwrap();
            }
        }
        let artifacts = linux_artifacts(directory.path(), "3.2.0", base).unwrap();
        assert_eq!(artifacts.len(), 8);
        for item in artifacts {
            assert_eq!(item.os, "linux");
            assert_eq!(item.size, 9);
            assert!(
                item.url
                    .ends_with(&format!("linux-{}.{}", item.arch, item.kind))
            );
            assert_eq!(
                item.sha256,
                sha256_file(&directory.path().join(item.url.rsplit('/').next().unwrap())).unwrap()
            );
        }
        std::fs::remove_file(directory.path().join("Onda-3.2.0-linux-arm64.pkg.tar.zst")).unwrap();
        assert!(linux_artifacts(directory.path(), "3.2.0", base).is_err());
    }
}
