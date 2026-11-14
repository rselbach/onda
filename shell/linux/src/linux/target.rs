//! The application that holds the keyboard focus, on X11.
//!
//! Wayland tells an ordinary client nothing about the focused application,
//! so a Wayland session has no target; see the README.

use gio_unix::{DesktopAppInfo, prelude::*};
use gtk::gio;
use onda_app::ports::Target;
use x11rb::{
    connection::{Connection, RequestConnection},
    properties::WmClass,
    protocol::{
        res::{self, ConnectionExt as _},
        xproto::{AtomEnum, ConnectionExt as _, Window},
    },
    rust_connection::RustConnection,
};

/// What a window says about the application it belongs to.
#[derive(Debug, Default)]
struct Hints {
    /// `_GTK_APPLICATION_ID`, the name GTK gives the application's desktop
    /// entry.
    gtk_id: Option<String>,
    /// The two halves of `WM_CLASS`.
    instance: Option<String>,
    class: Option<String>,
}

/// An installed desktop entry, as far as matching a window needs it.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    /// The desktop file's id without `.desktop`.
    id: String,
    name: String,
    /// `StartupWMClass`.
    wm_class: Option<String>,
}

/// The focused application: `Ok(None)` when the server answered and no
/// application it can name has the focus, an error when it did not answer.
pub fn focused() -> anyhow::Result<Option<Target>> {
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let Some(window) = focused_window(&connection, root)? else {
        return Ok(None);
    };
    let Some(process_id) = process_id(&connection, window)? else {
        return Ok(None);
    };
    let hints = hints(&connection, window)?;
    let entries = entries();
    let entry = matching(&hints, &entries);
    Ok(Some(Target {
        process_id,
        name: entry
            .map(|entry| entry.name.clone())
            .or(hints.class)
            .or(hints.instance),
        app_id: entry.map(|entry| entry.id.clone()),
    }))
}

/// Whether the focus is no longer in `target`: it closed, or another
/// application or nothing has the focus. A focus that cannot be read counts
/// as still there, so a failed read never withholds a paste.
pub fn left(target: &Target) -> bool {
    match focused() {
        Ok(Some(now)) if now == *target => false,
        Ok(now) => {
            tracing::info!(
                from = target.process_id,
                to = now.map(|now| now.process_id),
                "the focus left the target; copying instead"
            );
            true
        }
        Err(error) => {
            tracing::debug!(%error, "focused application unreadable");
            false
        }
    }
}

/// The icon of the desktop entry [`focused`] reported as `app_id`.
pub fn icon(app_id: &str) -> Option<gio::Icon> {
    DesktopAppInfo::new(&format!("{app_id}.desktop"))?.icon()
}

fn focused_window(connection: &RustConnection, root: Window) -> anyhow::Result<Option<Window>> {
    let active = connection
        .get_property(
            false,
            root,
            atom(connection, b"_NET_ACTIVE_WINDOW")?,
            AtomEnum::ANY,
            0,
            1,
        )?
        .reply()?;
    if let Some(window) = active.value32().and_then(|mut value| value.next()) {
        return Ok((window != x11rb::NONE).then_some(window));
    }
    // no window manager publishes the active window, so follow the keyboard
    let mut window = connection.get_input_focus()?.reply()?.focus;
    // None and PointerRoot name no window
    if window <= 1 || window == root {
        return Ok(None);
    }
    // toolkits may focus a child of the window that carries the hints
    loop {
        if WmClass::get(connection, window)?.reply()?.is_some() {
            return Ok(Some(window));
        }
        let parent = connection.query_tree(window)?.reply()?.parent;
        if parent == root || parent == x11rb::NONE {
            return Ok(Some(window));
        }
        window = parent;
    }
}

fn process_id(connection: &RustConnection, window: Window) -> anyhow::Result<Option<u32>> {
    // The server knows the process behind the window's connection. The
    // window's own `_NET_WM_PID` is a claim: inside a sandbox it is a
    // process id of the sandbox, and a remote client's is of another host.
    if connection
        .extension_information(res::X11_EXTENSION_NAME)?
        .is_some()
    {
        let version = connection.res_query_version(1, 2)?.reply()?;
        if (version.server_major, version.server_minor) >= (1, 2) {
            let spec = res::ClientIdSpec {
                client: window,
                mask: res::ClientIdMask::LOCAL_CLIENT_PID,
            };
            let reply = connection.res_query_client_ids(&[spec])?.reply()?;
            let known = reply
                .ids
                .iter()
                .find_map(|id| id.value.first().copied())
                .filter(|pid| *pid != 0);
            if known.is_some() {
                return Ok(known);
            }
        }
    }
    let claimed = connection
        .get_property(
            false,
            window,
            atom(connection, b"_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            0,
            1,
        )?
        .reply()?;
    Ok(claimed.value32().and_then(|mut value| value.next()))
}

fn hints(connection: &RustConnection, window: Window) -> anyhow::Result<Hints> {
    let text = |bytes: &[u8]| {
        Some(String::from_utf8_lossy(bytes).into_owned()).filter(|text| !text.is_empty())
    };
    let class = WmClass::get(connection, window)?.reply()?;
    let gtk_id = connection
        .get_property(
            false,
            window,
            atom(connection, b"_GTK_APPLICATION_ID")?,
            AtomEnum::ANY,
            0,
            1024,
        )?
        .reply()?;
    Ok(Hints {
        gtk_id: text(&gtk_id.value),
        instance: class.as_ref().and_then(|class| text(class.instance())),
        class: class.as_ref().and_then(|class| text(class.class())),
    })
}

fn atom(connection: &RustConnection, name: &[u8]) -> anyhow::Result<u32> {
    Ok(connection.intern_atom(false, name)?.reply()?.atom)
}

fn entries() -> Vec<Entry> {
    gio::AppInfo::all()
        .into_iter()
        .filter_map(|app| app.downcast::<DesktopAppInfo>().ok())
        .filter_map(|app| {
            Some(Entry {
                id: app.id()?.strip_suffix(".desktop")?.to_owned(),
                name: app.display_name().into(),
                wm_class: app.startup_wm_class().map(Into::into),
            })
        })
        .collect()
}

/// The entry of the window's application, tried in GNOME Shell's order: the
/// GTK application id, then an entry that declares the window's instance or
/// class as its `StartupWMClass`, then an entry named after either.
fn matching<'a>(hints: &Hints, entries: &'a [Entry]) -> Option<&'a Entry> {
    let named = |name: &Option<String>| {
        let name = name.as_deref()?;
        entries
            .iter()
            .find(|entry| entry.id.eq_ignore_ascii_case(name))
    };
    let declaring = |name: &Option<String>| {
        let name = name.as_deref()?;
        entries
            .iter()
            .find(|entry| entry.wm_class.as_deref() == Some(name))
    };
    named(&hints.gtk_id)
        .or_else(|| declaring(&hints.instance))
        .or_else(|| declaring(&hints.class))
        .or_else(|| named(&hints.instance))
        .or_else(|| named(&hints.class))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, wm_class: Option<&str>) -> Entry {
        Entry {
            id: id.into(),
            name: id.into(),
            wm_class: wm_class.map(Into::into),
        }
    }

    #[test]
    fn a_window_finds_its_desktop_entry() {
        let entries = [
            entry("edu.greendale.Messenger", None),
            entry("greendale-browser", None),
            entry("greendale-browser-study-room", Some("crx_studyroom")),
            entry("edu.greendale.Paintball", Some("Paintball")),
            entry("paintball", None),
        ];
        struct Case {
            name: &'static str,
            gtk_id: Option<&'static str>,
            instance: Option<&'static str>,
            class: Option<&'static str>,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "the GTK application id names the entry",
                gtk_id: Some("edu.greendale.Messenger"),
                instance: Some("messenger"),
                class: Some("messenger"),
                want: Some("edu.greendale.Messenger"),
            },
            Case {
                name: "an entry declaring the instance beats one named after the class",
                gtk_id: None,
                instance: Some("crx_studyroom"),
                class: Some("Greendale-browser"),
                want: Some("greendale-browser-study-room"),
            },
            Case {
                name: "an entry declaring the class beats one named after the instance",
                gtk_id: None,
                instance: Some("paintball"),
                class: Some("Paintball"),
                want: Some("edu.greendale.Paintball"),
            },
            Case {
                name: "an entry named after the class, whatever its case",
                gtk_id: None,
                instance: Some("Navigator"),
                class: Some("Greendale-Browser"),
                want: Some("greendale-browser"),
            },
            Case {
                name: "a GTK application id without an entry falls back to the class",
                gtk_id: Some("edu.greendale.Unlisted"),
                instance: Some("greendale-browser"),
                class: Some("Unlisted"),
                want: Some("greendale-browser"),
            },
            Case {
                name: "no entry",
                gtk_id: None,
                instance: Some("troy"),
                class: Some("Troy"),
                want: None,
            },
            Case {
                name: "no hints",
                gtk_id: None,
                instance: None,
                class: None,
                want: None,
            },
        ];
        for case in cases {
            let hints = Hints {
                gtk_id: case.gtk_id.map(Into::into),
                instance: case.instance.map(Into::into),
                class: case.class.map(Into::into),
            };
            let got = matching(&hints, &entries).map(|entry| entry.id.as_str());
            assert_eq!(got, case.want, "{}", case.name);
        }
    }
}
