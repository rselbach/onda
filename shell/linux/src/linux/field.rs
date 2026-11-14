//! The focused text field's value and caret, read over AT-SPI (OUT-2).
//!
//! AT-SPI answers only for an application that is on the accessibility bus
//! and keeps its focus there. The README says which toolkits were measured.
//! Everything else reads as no field, and the dictation is then delivered
//! without joining.

use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use gtk::{
    gio::{self, prelude::*},
    glib::{self, variant::ObjectPath},
};
use onda_app::ports::FieldText;

/// OUT-2: a read that takes longer is given up.
const LIMIT: Duration = Duration::from_millis(250);

/// How long one application may take to answer one call. Without a target
/// the search passes every application, and one that hangs must leave time
/// for the one that has the focus.
const CALL_TIMEOUT: Duration = Duration::from_millis(100);

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";

/// `ATSPI_STATE_ACTIVE` and `ATSPI_STATE_FOCUSED`, as bit positions.
const ACTIVE: u32 = 1;
const FOCUSED: u32 = 12;

/// `ATSPI_ROLE_PASSWORD_TEXT`.
const PASSWORD_TEXT: u32 = 40;

/// The characters GTK draws in place of hidden text, in the order it tries
/// them, and the asterisk it falls back to.
const MASKS: [char; 5] = ['\u{25CF}', '\u{2022}', '\u{2731}', '\u{273A}', '*'];

/// The text field that has the focus in the application with `process_id`,
/// or in whichever application has the active window when `None`. `None`
/// when no field exposes its text within [`LIMIT`], and for a password
/// field.
pub fn read(process_id: Option<u32>) -> Option<FieldText> {
    let cancellable = gio::Cancellable::new();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn({
        let cancellable = cancellable.clone();
        move || {
            // the receiver is gone once the limit passed
            let _ = sender.send(Bus::open(cancellable).and_then(|bus| bus.field(process_id)));
        }
    });
    match receiver.recv_timeout(LIMIT) {
        Ok(Ok(field)) => field,
        Ok(Err(error)) => {
            tracing::debug!(error = %format!("{error:#}"), "no field text");
            None
        }
        Err(_) => {
            cancellable.cancel();
            tracing::debug!("no field text within the limit");
            None
        }
    }
}

/// An object on the accessibility bus: the connection that owns it and its
/// path there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Accessible {
    owner: String,
    path: String,
}

/// An accessible's states, 64 bits in two words.
struct States(Vec<u32>);

impl States {
    fn has(&self, state: u32) -> bool {
        self.0
            .get((state / 32) as usize)
            .is_some_and(|word| word >> (state % 32) & 1 == 1)
    }
}

/// The accessibility bus for one read. Cancelling ends the call in flight
/// and every later one.
struct Bus {
    connection: gio::DBusConnection,
    cancellable: gio::Cancellable,
}

impl Bus {
    fn open(cancellable: gio::Cancellable) -> anyhow::Result<Self> {
        let session = Self {
            connection: gio::bus_get_sync(gio::BusType::Session, Some(&cancellable))
                .map_err(|error| failure("the session bus", &error))?,
            cancellable,
        };
        // the accessibility bus is a second bus, and the session bus knows
        // where it listens
        let (address,) = session
            .call(
                "org.a11y.Bus",
                "/org/a11y/bus",
                "org.a11y.Bus",
                "GetAddress",
                None,
            )?
            .get::<(String,)>()
            .context("unexpected GetAddress reply")?;
        let connection = gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            Some(&session.cancellable),
        )
        .map_err(|error| failure("the accessibility bus", &error))?;
        Ok(Self {
            connection,
            cancellable: session.cancellable,
        })
    }

    fn field(&self, process_id: Option<u32>) -> anyhow::Result<Option<FieldText>> {
        for application in self.applications(process_id)? {
            match self.focused_in(&application) {
                Ok(Some(focused)) => return self.field_text(&focused),
                Ok(None) => {}
                Err(error) if self.cancellable.is_cancelled() => return Err(error),
                // another application may hold the focus
                Err(error) => {
                    tracing::debug!(error = %format!("{error:#}"), "application passed over");
                }
            }
        }
        Ok(None)
    }

    /// The applications on the bus, or only those of `process_id`.
    fn applications(&self, process_id: Option<u32>) -> anyhow::Result<Vec<Accessible>> {
        let mut applications = self.children(&Accessible {
            owner: "org.a11y.atspi.Registry".into(),
            path: "/org/a11y/atspi/accessible/root".into(),
        })?;
        if let Some(process_id) = process_id {
            // the bus itself answers this, so no application is waited for
            applications
                .retain(|application| self.process_id(&application.owner).ok() == Some(process_id));
        }
        Ok(applications)
    }

    fn process_id(&self, owner: &str) -> anyhow::Result<u32> {
        let (process_id,) = self
            .call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                Some(&(owner,).to_variant()),
            )?
            .get::<(u32,)>()
            .context("unexpected GetConnectionUnixProcessID reply")?;
        Ok(process_id)
    }

    /// The accessible with the focus under the application's active window.
    /// AT-SPI has no call that names it, short of following focus events.
    fn focused_in(&self, application: &Accessible) -> anyhow::Result<Option<Accessible>> {
        for window in self.children(application)? {
            if !self.states(&window)?.has(ACTIVE) {
                continue;
            }
            let mut pending = vec![window];
            while let Some(current) = pending.pop() {
                if self.states(&current)?.has(FOCUSED) {
                    return Ok(Some(current));
                }
                pending.extend(self.children(&current)?.into_iter().rev());
            }
        }
        Ok(None)
    }

    fn field_text(&self, focused: &Accessible) -> anyhow::Result<Option<FieldText>> {
        // a password field is never asked for its text
        let (role,) = self
            .ask(focused, ACCESSIBLE, "GetRole", None)?
            .get::<(u32,)>()
            .context("unexpected GetRole reply")?;
        if role == PASSWORD_TEXT {
            return Ok(None);
        }
        let (interfaces,) = self
            .ask(focused, ACCESSIBLE, "GetInterfaces", None)?
            .get::<(Vec<String>,)>()
            .context("unexpected GetInterfaces reply")?;
        if !interfaces.iter().any(|interface| interface == TEXT) {
            return Ok(None);
        }
        let (text,) = self
            .ask(focused, TEXT, "GetText", Some(&(0i32, -1i32).to_variant()))?
            .get::<(String,)>()
            .context("unexpected GetText reply")?;
        if is_masked(&text) {
            return Ok(None);
        }
        Ok(usize::try_from(self.selection_start(focused)?)
            .ok()
            .map(|caret| FieldText {
                caret_utf16: utf16_offset(&text, caret),
                text,
            }))
    }

    /// Where a paste lands, in characters: the start of the selection, or
    /// the caret without one. Negative when the field has no caret.
    fn selection_start(&self, focused: &Accessible) -> anyhow::Result<i32> {
        let (selections,) = self
            .ask(focused, TEXT, "GetNSelections", None)?
            .get::<(i32,)>()
            .context("unexpected GetNSelections reply")?;
        if selections > 0 {
            let (start, end) = self
                .ask(focused, TEXT, "GetSelection", Some(&(0i32,).to_variant()))?
                .get::<(i32, i32)>()
                .context("unexpected GetSelection reply")?;
            return Ok(start.min(end));
        }
        let (caret,) = self
            .ask(
                focused,
                "org.freedesktop.DBus.Properties",
                "Get",
                Some(&(TEXT, "CaretOffset").to_variant()),
            )?
            .get::<(glib::Variant,)>()
            .context("unexpected CaretOffset reply")?;
        caret.get::<i32>().context("unexpected CaretOffset value")
    }

    fn children(&self, parent: &Accessible) -> anyhow::Result<Vec<Accessible>> {
        let (children,) = self
            .ask(parent, ACCESSIBLE, "GetChildren", None)?
            .get::<(Vec<(String, ObjectPath)>,)>()
            .context("unexpected GetChildren reply")?;
        Ok(children
            .into_iter()
            .map(|(owner, path)| Accessible {
                owner,
                path: path.as_str().into(),
            })
            .collect())
    }

    fn states(&self, accessible: &Accessible) -> anyhow::Result<States> {
        let (states,) = self
            .ask(accessible, ACCESSIBLE, "GetState", None)?
            .get::<(Vec<u32>,)>()
            .context("unexpected GetState reply")?;
        Ok(States(states))
    }

    fn ask(
        &self,
        accessible: &Accessible,
        interface: &str,
        method: &str,
        parameters: Option<&glib::Variant>,
    ) -> anyhow::Result<glib::Variant> {
        self.call(
            &accessible.owner,
            &accessible.path,
            interface,
            method,
            parameters,
        )
    }

    fn call(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        method: &str,
        parameters: Option<&glib::Variant>,
    ) -> anyhow::Result<glib::Variant> {
        self.connection
            .call_sync(
                Some(destination),
                path,
                interface,
                method,
                parameters,
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                CALL_TIMEOUT.as_millis() as i32,
                Some(&self.cancellable),
            )
            .map_err(|error| failure(method, &error))
    }
}

// LOG-2: an error's message is another application's wording and could
// quote the field, so only the kind of error is kept for the log.
fn failure(what: &str, error: &glib::Error) -> anyhow::Error {
    if let Some(kind) = error.kind::<gio::DBusError>() {
        return anyhow::anyhow!("{what}: {kind:?}");
    }
    match error.kind::<gio::IOErrorEnum>() {
        Some(kind) => anyhow::anyhow!("{what}: {kind:?}"),
        None => anyhow::anyhow!("{what} failed"),
    }
}

/// Whether `text` is what a toolkit shows for hidden input: one mask
/// character repeated. A GTK 4 entry whose text is hidden without being a
/// `GtkPasswordEntry` reports as an ordinary text field with this value.
fn is_masked(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| MASKS.contains(&first) && characters.all(|next| next == first))
}

/// AT-SPI counts in characters; the core counts in UTF-16 code units.
fn utf16_offset(text: &str, characters: usize) -> usize {
    text.chars().take(characters).map(char::len_utf16).sum()
}

#[cfg(test)]
mod bus_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caret_in_characters_becomes_utf16_units() {
        let cases = [
            ("ASCII", "Troy Barnes", 4, 4),
            ("after a character outside the BMP", "🎓🎓 Troy", 3, 5),
            ("after a character of two UTF-8 bytes", "Señor Chang", 4, 4),
            ("start of field", "Troy", 0, 0),
            ("past the end", "🎓", 9, 2),
        ];
        for (name, text, characters, want) in cases {
            assert_eq!(utf16_offset(text, characters), want, "{name}");
        }
    }

    #[test]
    fn hidden_input_is_recognized_by_its_mask() {
        let cases = [
            ("GTK's bullets", "••••••••••", true),
            ("GTK's black circles", "●●●", true),
            ("one asterisk", "*", true),
            ("an empty field", "", false),
            ("ordinary text", "Troy Barnes", false),
            ("a bullet before text", "• enroll", false),
            ("two different masks", "•●", false),
            ("a repeated letter", "aaaa", false),
        ];
        for (name, text, want) in cases {
            assert_eq!(is_masked(text), want, "{name}");
        }
    }

    #[test]
    fn states_are_read_across_both_words() {
        // what a focused GTK 4 text view reported, plus state 42 in the
        // second word
        let states = States(vec![1 << FOCUSED | 1 << 7, 1 << 10]);
        assert!(states.has(FOCUSED));
        assert!(states.has(42));
        assert!(!states.has(ACTIVE));
        assert!(!States(vec![]).has(FOCUSED));
    }
}
