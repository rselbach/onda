//! Best-effort clipboard preservation, bounded by size and read time.

use super::process;
use std::time::{Duration, Instant};

pub struct Snapshot(Vec<(String, Vec<u8>)>);

impl Snapshot {
    pub fn read() -> anyhow::Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let read = |args: &[&str]| {
            process::run_for(
                if process::wayland() {
                    "wl-paste"
                } else {
                    "xclip"
                },
                args,
                &[],
                deadline.saturating_duration_since(Instant::now()),
            )
        };
        let types = if process::wayland() {
            read(&["--list-types"])?
        } else {
            read(&["-selection", "clipboard", "-out", "-target", "TARGETS"])?
        };
        let types = String::from_utf8(types)?;
        let mut formats: Vec<(String, Vec<u8>)> = Vec::new();
        let mut size = 0;
        for mime in types
            .lines()
            .filter(|mime| !matches!(*mime, "TARGETS" | "TIMESTAMP" | "MULTIPLE" | "SAVE_TARGETS"))
        {
            let bytes = if process::wayland() {
                read(&["--no-newline", "--type", mime])?
            } else {
                read(&["-selection", "clipboard", "-out", "-target", mime])?
            };
            size += bytes.len();
            anyhow::ensure!(size <= 32 * 1024 * 1024, "clipboard exceeds 32 MiB");
            formats.push((mime.into(), bytes));
        }
        if !process::wayland() {
            anyhow::ensure!(
                formats.iter().all(|(mime, _)| matches!(
                    mime.as_str(),
                    "UTF8_STRING" | "STRING" | "TEXT" | "text/plain" | "text/plain;charset=utf-8"
                )),
                "X11 clipboard contains non-text formats; preserving the transcript instead"
            );
        }
        Ok(Self(formats))
    }

    pub fn restore_if_unchanged(self, transcript: &str) -> anyhow::Result<()> {
        let current = if process::wayland() {
            process::run(
                "wl-paste",
                &["--no-newline", "--type", "text/plain;charset=utf-8"],
                &[],
            )?
        } else {
            process::run("xclip", &["-selection", "clipboard", "-out"], &[])?
        };
        if current != transcript.as_bytes() {
            return Ok(());
        }
        if process::wayland() {
            use wl_clipboard_rs::copy::{
                ClipboardType, MimeSource, MimeType, Options, Seat, Source,
            };
            if self.0.is_empty() {
                wl_clipboard_rs::copy::clear(ClipboardType::Regular, Seat::All)?;
            } else {
                let mut options = Options::new();
                options.foreground(true);
                let prepared = options.prepare_copy_multi(
                    self.0
                        .into_iter()
                        .map(|(mime, bytes)| MimeSource {
                            source: Source::Bytes(bytes.into_boxed_slice()),
                            mime_type: MimeType::Specific(mime),
                        })
                        .collect(),
                )?;
                std::thread::spawn(move || {
                    if let Err(error) = prepared.serve() {
                        tracing::debug!(%error, "clipboard ownership ended");
                    }
                });
            }
        } else {
            let text = self
                .0
                .iter()
                .find(|(mime, _)| mime == "UTF8_STRING")
                .or_else(|| self.0.first())
                .map(|(_, data)| data.as_slice())
                .unwrap_or_default();
            process::write("xclip", &["-selection", "clipboard", "-in"], text)?;
        }
        Ok(())
    }
}
