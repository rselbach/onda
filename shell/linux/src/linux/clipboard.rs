//! Best-effort clipboard preservation, bounded by size and read time.

use super::process;
use std::time::{Duration, Instant};

use wl_clipboard_rs::copy::{ClipboardType, MimeSource, MimeType, Options, Seat, Source};
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xfixes, xproto};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub struct Snapshot {
    formats: Vec<(String, Vec<u8>)>,
    selection: Selection,
}

enum Selection {
    X11(Box<RustConnection>),
    Wayland(String),
}

impl Selection {
    fn watch() -> anyhow::Result<Self> {
        if process::wayland() {
            // The native provider claims every seat. With several seats or no
            // data control, leave delivery to wl-copy on its selected seat.
            anyhow::ensure!(
                process::wayland_globals_match(|globals| {
                    globals.iter().filter(|g| g.interface == "wl_seat").count() == 1
                        && globals.iter().any(|g| {
                            matches!(
                                g.interface.as_str(),
                                "zwlr_data_control_manager_v1" | "ext_data_control_manager_v1"
                            )
                        })
                }),
                "cannot preserve the Wayland selection"
            );
            return Ok(Self::Wayland(format!(
                "application/x-onda-{}",
                uuid::Uuid::new_v4()
            )));
        }
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        let selection = xproto::intern_atom(&connection, false, b"CLIPBOARD")?
            .reply()?
            .atom;
        xfixes::query_version(&connection, 5, 0)?.reply()?;
        xfixes::select_selection_input(
            &connection,
            root,
            selection,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER
                | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?
        .check()?;
        Ok(Self::X11(Box::new(connection)))
    }

    fn still_ours(&self) -> anyhow::Result<bool> {
        match self {
            Self::Wayland(marker) => {
                let types = process::run("wl-paste", &["--list-types"], &[])?;
                Ok(std::str::from_utf8(&types)?
                    .lines()
                    .any(|mime| mime == marker))
            }
            Self::X11(connection) => {
                // Our xclip must be the only selection change since the snapshot.
                connection.sync()?;
                let mut claimed = false;
                while let Some(event) = connection.poll_for_event()? {
                    if let Event::XfixesSelectionNotify(event) = event {
                        if claimed || event.owner == x11rb::NONE {
                            return Ok(false);
                        }
                        claimed = true;
                    }
                }
                Ok(claimed)
            }
        }
    }
}

pub(super) fn copy_text(text: &str) -> anyhow::Result<()> {
    if process::wayland() {
        process::write(
            "wl-copy",
            &["--type", "text/plain;charset=utf-8"],
            text.as_bytes(),
        )?;
    } else {
        process::write(
            "xclip",
            &["-selection", "clipboard", "-in"],
            text.as_bytes(),
        )?;
    }
    Ok(())
}

fn offer_wayland(formats: Vec<(String, Vec<u8>)>) -> anyhow::Result<()> {
    let mut options = Options::new();
    options.foreground(true);
    let prepared = options.prepare_copy_multi(
        formats
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
    Ok(())
}

impl Snapshot {
    pub fn read() -> anyhow::Result<Self> {
        let selection = Selection::watch()?;
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
        Ok(Self { formats, selection })
    }

    pub fn copy_for_paste(&self, text: &str) -> anyhow::Result<()> {
        match &self.selection {
            Selection::Wayland(marker) => offer_wayland(vec![
                ("text/plain;charset=utf-8".into(), text.as_bytes().to_vec()),
                (marker.clone(), Vec::new()),
            ])
            .or_else(|error| {
                tracing::debug!(%error, "copying without a Wayland restoration marker");
                copy_text(text)
            }),
            Selection::X11(_) => copy_text(text),
        }
    }

    pub fn restore_if_unchanged(self) -> anyhow::Result<()> {
        if !self.selection.still_ours()? {
            return Ok(());
        }
        if matches!(self.selection, Selection::Wayland(_)) {
            if self.formats.is_empty() {
                wl_clipboard_rs::copy::clear(ClipboardType::Regular, Seat::All)?;
            } else {
                offer_wayland(self.formats)?;
            }
        } else {
            let text = self
                .formats
                .iter()
                .find(|(mime, _)| mime == "UTF8_STRING")
                .or_else(|| self.formats.first())
                .map(|(_, data)| data.as_slice())
                .unwrap_or_default();
            process::write("xclip", &["-selection", "clipboard", "-in"], text)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "uses the private desktop clipboard"]
    fn restoring_preserves_a_new_copy_with_identical_text() {
        let read = || {
            if process::wayland() {
                process::run(
                    "wl-paste",
                    &["--no-newline", "--type", "text/plain;charset=utf-8"],
                    &[],
                )
            } else {
                process::run("xclip", &["-selection", "clipboard", "-out"], &[])
            }
            .unwrap()
        };
        for copied_again in [false, true] {
            copy_text("Greendale").unwrap();
            let snapshot = Snapshot::read().unwrap();
            snapshot.copy_for_paste("Troy Barnes").unwrap();
            assert_eq!(read(), b"Troy Barnes");
            if copied_again {
                if process::wayland() {
                    offer_wayland(vec![
                        ("text/plain;charset=utf-8".into(), b"Troy Barnes".to_vec()),
                        ("text/html".into(), b"<b>Troy Barnes</b>".to_vec()),
                    ])
                    .unwrap();
                } else {
                    copy_text("Troy Barnes").unwrap();
                }
            }
            snapshot.restore_if_unchanged().unwrap();
            if copied_again && process::wayland() {
                let html = process::run("wl-paste", &["--no-newline", "--type", "text/html"], &[])
                    .unwrap();
                assert_eq!(html, b"<b>Troy Barnes</b>");
            }
            assert_eq!(
                read(),
                if copied_again {
                    "Troy Barnes"
                } else {
                    "Greendale"
                }
                .as_bytes()
            );
        }
    }
}
