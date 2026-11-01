//! The release tool refuses keys and feeds the app would not accept.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::SigningKey;
use onda_update::feed::{self, Artifact, Release};

fn tool(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_onda-update-tool"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stranger() -> String {
    STANDARD.encode([4u8; 32])
}

#[test]
fn signing_with_a_key_onda_does_not_pin_fails() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("Onda.dmg");
    std::fs::write(&archive, b"Greendale").unwrap();
    let out = tool(
        &["sparkle-signature", archive.to_str().unwrap()],
        &stranger(),
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not match"));

    let zip = dir.path().join("Onda.zip");
    std::fs::write(&zip, b"Greendale").unwrap();
    let feed = dir.path().join("update.json");
    let url = "https://github.com/rselbach/onda/releases/download/v2.0.0/Onda";
    let out = tool(
        &[
            "sign-feed",
            "--version",
            "2.0.0",
            "--build",
            "42",
            "--minimum-os",
            "26.0",
            "--published-at",
            "2026-11-01T12:00:00Z",
            "--notes-url",
            "https://github.com/rselbach/onda/releases/tag/v2.0.0",
            "--zip",
            zip.to_str().unwrap(),
            "--zip-url",
            &format!("{url}.zip"),
            "--dmg",
            archive.to_str().unwrap(),
            "--dmg-url",
            &format!("{url}.dmg"),
            "--out",
            feed.to_str().unwrap(),
        ],
        &stranger(),
    );
    assert!(!out.status.success());
    assert!(!feed.exists());
}

#[test]
fn verify_feed_refuses_a_feed_signed_by_another_key() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://github.com/rselbach/onda/releases/download/v2.0.0/Onda";
    let artifact = |kind: &str| Artifact {
        os: onda_update::OS.to_owned(),
        arch: onda_update::ARCH.to_owned(),
        kind: kind.to_owned(),
        minimum_os: "26.0".to_owned(),
        url: format!("{url}.{kind}"),
        size: 9,
        sha256: "b".repeat(64),
    };
    let release = Release {
        schema: feed::SCHEMA,
        app: onda_update::BUNDLE_ID.to_owned(),
        channel: feed::CHANNEL.to_owned(),
        version: "2.0.0".to_owned(),
        build: 42,
        published_at: "2026-11-01T12:00:00Z".to_owned(),
        notes_url: None,
        artifacts: vec![artifact(feed::KIND_APP_ZIP), artifact(feed::KIND_DMG)],
    };
    let path = dir.path().join("update.json");
    std::fs::write(
        &path,
        feed::seal(&release, &SigningKey::from_bytes(&[4; 32])),
    )
    .unwrap();
    let out = tool(&["verify-feed", path.to_str().unwrap()], "");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("signature"));
}

#[test]
fn the_pinned_key_matches_info_plist() {
    let plist = concat!(env!("CARGO_MANIFEST_DIR"), "/../../shell/macos/Info.plist");
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", "SUPublicEDKey", "raw", "-o", "-", plist])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        onda_update::PUBLIC_KEY
    );
}
