//! Best-effort clipboard preservation, bounded by size and read time.

use super::process;
use std::cell::Cell;
use std::time::{Duration, Instant};

use wl_clipboard_rs::copy::{ClipboardType, MimeSource, MimeType, Options, Seat, Source};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::{Event, xfixes, xkb, xproto, xtest};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::{ConnectionExt as _, GrabServer};

const WAYLAND_TEXT_TYPE: &str = "text/plain;charset=utf-8";
const X11_PASTE_SYMBOLS: [xproto::Keysym; 2] = [0x76, 0x56];
const X11_SKIP_TARGETS: &[&str] = &[
    "TARGETS",
    "TIMESTAMP",
    "MULTIPLE",
    "SAVE_TARGETS",
    "DELETE",
    "INSERT_SELECTION",
    "INSERT_PROPERTY",
];
const X11_TEXT_TARGETS: &[&str] = &[
    "UTF8_STRING",
    "STRING",
    "TEXT",
    "text/plain",
    "text/plain;charset=utf-8",
];

pub struct Paste<'a> {
    text: &'a str,
    previous: Option<Vec<(String, Vec<u8>)>>,
    selection: Option<Selection>,
}

enum Selection {
    X11(Box<X11Selection>),
    Wayland(String),
}

struct X11Selection {
    connection: RustConnection,
    ownership: Cell<X11Ownership>,
}

#[derive(Clone, Copy)]
enum X11Ownership {
    AwaitingClaim,
    Claimed,
    Lost,
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
        Ok(Self::X11(Box::new(X11Selection {
            connection,
            ownership: Cell::new(X11Ownership::AwaitingClaim),
        })))
    }

    fn still_ours(&self) -> anyhow::Result<bool> {
        match self {
            Self::Wayland(marker) => {
                let types = process::run("wl-paste", &["--list-types"], &[])?;
                Ok(std::str::from_utf8(&types)?
                    .lines()
                    .any(|mime| mime == marker))
            }
            Self::X11(selection) => {
                if matches!(selection.ownership.get(), X11Ownership::Lost) {
                    return Ok(false);
                }
                selection.connection.sync().inspect_err(|_| {
                    selection.ownership.set(X11Ownership::Lost);
                })?;
                selection.consume_ownership_events()
            }
        }
    }
}

impl X11Ownership {
    fn observe(self, event: xfixes::SelectionNotifyEvent) -> Self {
        if matches!(self, Self::AwaitingClaim)
            && event.subtype == xfixes::SelectionEvent::SET_SELECTION_OWNER
            && event.owner != x11rb::NONE
        {
            return Self::Claimed;
        }
        Self::Lost
    }
}

impl X11Selection {
    // The caller must round-trip first so notifications already sent by the server are visible.
    fn consume_ownership_events(&self) -> anyhow::Result<bool> {
        let mut state = self.ownership.replace(X11Ownership::Lost);
        while !matches!(state, X11Ownership::Lost) {
            let Some(event) = self.connection.poll_for_event()? else {
                break;
            };
            if let Event::XfixesSelectionNotify(event) = event {
                state = state.observe(event);
            }
        }
        self.ownership.set(state);
        Ok(matches!(state, X11Ownership::Claimed))
    }

    fn paste(&self, shift: bool) -> anyhow::Result<bool> {
        if matches!(self.ownership.get(), X11Ownership::Lost) {
            return Ok(false);
        }
        let connection = &self.connection;
        anyhow::ensure!(
            xkb::use_extension(connection, 1, 0)?.reply()?.supported,
            "XKB is unavailable"
        );
        xtest::get_version(connection, 2, 2)?.reply()?;
        let device = xkb::ID::USE_CORE_KBD.into();
        let requests = self.with_server_grab(|| {
            // Read the layout and drain ownership changes before dispatching on this connection.
            let map = connection.send_trait_request_with_reply(xkb::GetMapRequest {
                device_spec: device,
                full: xkb::MapPart::KEY_SYMS
                    | xkb::MapPart::KEY_TYPES
                    | xkb::MapPart::KEY_ACTIONS
                    | xkb::MapPart::MODIFIER_MAP,
                ..Default::default()
            })?;
            let state = xkb::get_state(connection, device)?;
            let state = state.reply()?;
            if !self.consume_ownership_events()? {
                return Ok(None);
            }
            let map = map.reply()?;
            anyhow::ensure!(
                map.first_key_sym == map.first_key_action,
                "incomplete XKB map"
            );
            let keys = X11PasteKeys::resolve(&map.map, map.first_key_sym, &state, shift)?;
            let requests = keys
                .events()
                .map(|(kind, key)| {
                    xtest::fake_input(
                        connection,
                        kind,
                        key,
                        x11rb::CURRENT_TIME,
                        x11rb::NONE,
                        0,
                        0,
                        0,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(requests))
        })?;
        let Some(requests) = requests else {
            return Ok(false);
        };
        for request in requests {
            request.check()?;
        }
        Ok(true)
    }

    fn with_server_grab<T>(&self, action: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let grab = GrabServer::grab(&self.connection)?;
        let result = action();
        // Flush the ungrab on errors too, before checking requests or waiting on a receiver.
        drop(grab);
        self.connection.flush()?;
        result
    }
}

struct X11PasteKeys {
    ctrl: xproto::Keycode,
    shift: Option<xproto::Keycode>,
    v: xproto::Keycode,
}

impl X11PasteKeys {
    fn resolve(
        map: &xkb::GetMapMap,
        first_key: xproto::Keycode,
        state: &xkb::GetStateReply,
        shift: bool,
    ) -> anyhow::Result<Self> {
        use xproto::ModMask;

        anyhow::ensure!(
            state.base_mods == ModMask::default()
                && state.latched_mods == ModMask::default()
                && state.latched_group == 0,
            "X11 modifiers are held or latched"
        );
        let types = map.types_rtrn.as_deref().unwrap_or_default();
        let symbols = map.syms_rtrn.as_deref().unwrap_or_default();
        let group = usize::from(u8::from(state.group));
        let translate = |code: xproto::Keycode, mods| {
            let offset = usize::from(code.checked_sub(first_key)?);
            let key = symbols.get(offset)?;
            let group_count = usize::from(key.group_info & 0x0f);
            if group_count == 0 {
                return None;
            }
            let group = if group < group_count {
                group
            } else {
                match key.group_info & 0xc0 {
                    0x40 => group_count - 1,
                    0x80 => {
                        let redirect = usize::from((key.group_info >> 4) & 0x03);
                        if redirect < group_count { redirect } else { 0 }
                    }
                    _ => group % group_count,
                }
            };
            let key_type = types.get(usize::from(*key.kt_index.get(group)?))?;
            let level = key_type
                .map
                .iter()
                .find(|entry| entry.active && entry.mods_mask == (mods & key_type.mods_mask))
                .map_or(0, |entry| entry.level);
            let index = group * usize::from(key.width) + usize::from(level);
            let action = map.key_actions.as_ref().and_then(|actions| {
                let (prior, current) = actions.acts_rtrn_count.split_at_checked(offset)?;
                if index >= usize::from(*current.first()?) {
                    return None;
                }
                let start: usize = prior.iter().copied().map(usize::from).sum();
                actions.acts_rtrn_acts.get(start + index)
            });
            Some((*key.syms.get(index)?, action))
        };
        let modifiers = map.modmap_rtrn.as_deref().unwrap_or_default();
        let modifier = |mask, mods| {
            // A mapped modifier can select a different action, such as switching groups.
            modifiers
                .iter()
                .find(|entry| {
                    entry.mods == mask
                        && translate(entry.keycode, mods)
                            .and_then(|(_, action)| action)
                            .is_some_and(|action| {
                                action.as_type() == xkb::SAType::SET_MODS
                                    && action.as_setmods().mask == mask
                            })
                })
                .map(|entry| entry.keycode)
                .ok_or_else(|| {
                    anyhow::anyhow!("the current X11 layout does not map the paste modifier")
                })
        };
        let ctrl = modifier(ModMask::CONTROL, state.lookup_mods)?;
        let mods = state.lookup_mods | ModMask::CONTROL;
        let shift = shift.then(|| modifier(ModMask::SHIFT, mods)).transpose()?;
        let mods = if shift.is_some() {
            mods | ModMask::SHIFT
        } else {
            mods
        };
        for v in first_key..=xproto::Keycode::MAX {
            if translate(v, mods).is_some_and(|(sym, action)| {
                X11_PASTE_SYMBOLS.contains(&sym)
                    && action.is_none_or(|action| action.as_type() == xkb::SAType::NO_ACTION)
            }) && !modifiers.iter().any(|entry| entry.keycode == v)
            {
                return Ok(Self { ctrl, shift, v });
            }
        }
        anyhow::bail!("the current X11 layout does not map the paste shortcut")
    }

    fn events(&self) -> impl Iterator<Item = (u8, xproto::Keycode)> {
        let keys = [Some(self.ctrl), self.shift, Some(self.v)];
        keys.into_iter()
            .flatten()
            .map(|key| (xproto::KEY_PRESS_EVENT, key))
            .chain(
                keys.into_iter()
                    .rev()
                    .flatten()
                    .map(|key| (xproto::KEY_RELEASE_EVENT, key)),
            )
    }
}

pub(super) fn copy_text(text: &str) -> anyhow::Result<()> {
    if process::wayland() {
        process::write("wl-copy", &["--type", WAYLAND_TEXT_TYPE], text.as_bytes())?;
    } else {
        process::write(
            "xclip",
            &["-selection", "clipboard", "-in"],
            text.as_bytes(),
        )?;
    }
    Ok(())
}

fn filter_x11_targets(types: &str) -> anyhow::Result<Vec<&str>> {
    let mut targets = Vec::new();
    for target in types
        .lines()
        .map(str::trim)
        .filter(|target| !target.is_empty() && !X11_SKIP_TARGETS.contains(target))
    {
        anyhow::ensure!(
            X11_TEXT_TARGETS.contains(&target),
            "X11 clipboard contains non-text formats; preserving the transcript instead"
        );
        targets.push(target);
    }
    Ok(targets)
}

fn is_empty_clipboard_error(
    wayland: bool,
    status: std::process::ExitStatus,
    stderr: &[u8],
) -> bool {
    if status.code() != Some(1) {
        return false;
    }
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if wayland {
        message.contains("nothing is copied") || message.contains("no selection")
    } else {
        message.contains("target targets not available")
            || message.contains("selection clipboard not available")
            || message.trim().is_empty()
    }
}

fn validate_wayland_mime(mime: &str) -> anyhow::Result<&str> {
    anyhow::ensure!(
        !mime.is_empty() && !mime.starts_with('-'),
        "invalid clipboard MIME type"
    );
    Ok(mime)
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

impl<'a> Paste<'a> {
    pub fn prepare(text: &'a str) -> Self {
        let previous = Self::read_formats()
            .inspect_err(|error| tracing::debug!(%error, "clipboard cannot be preserved"))
            .ok();
        let selection = Selection::watch()
            .inspect_err(|error| tracing::debug!(%error, "clipboard cannot be monitored"))
            .ok();
        Self {
            text,
            previous: selection.as_ref().and(previous),
            selection,
        }
    }

    fn read_formats() -> anyhow::Result<Vec<(String, Vec<u8>)>> {
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
        let wayland = process::wayland();
        let types_output = if wayland {
            process::run_for_output(
                "wl-paste",
                &["--list-types"],
                &[],
                deadline.saturating_duration_since(Instant::now()),
            )?
        } else {
            process::run_for_output(
                "xclip",
                &["-selection", "clipboard", "-out", "-target", "TARGETS"],
                &[],
                deadline.saturating_duration_since(Instant::now()),
            )?
        };
        if !types_output.status.success() {
            if is_empty_clipboard_error(wayland, types_output.status, &types_output.stderr) {
                return Ok(Vec::new());
            }
            anyhow::bail!(
                "{} failed ({})",
                if wayland { "wl-paste" } else { "xclip" },
                types_output.status
            );
        }
        let types = String::from_utf8(types_output.stdout)?;
        let mut formats: Vec<(String, Vec<u8>)> = Vec::new();
        let mut size = 0;
        if wayland {
            for mime in types
                .lines()
                .filter(|mime| !mime.is_empty() && !X11_SKIP_TARGETS.contains(mime))
            {
                let mime = validate_wayland_mime(mime)?;
                let type_arg = format!("--type={mime}");
                let bytes = read(&["--no-newline", &type_arg])?;
                size += bytes.len();
                anyhow::ensure!(size <= 32 * 1024 * 1024, "clipboard exceeds 32 MiB");
                formats.push((mime.into(), bytes));
            }
        } else {
            for target in filter_x11_targets(&types)? {
                let bytes = read(&["-selection", "clipboard", "-out", "-target", target])?;
                size += bytes.len();
                anyhow::ensure!(size <= 32 * 1024 * 1024, "clipboard exceeds 32 MiB");
                formats.push((target.into(), bytes));
            }
        }
        Ok(formats)
    }

    pub fn copy_for_paste(&mut self) -> anyhow::Result<()> {
        if let Some(Selection::Wayland(marker)) = &self.selection {
            let marker = marker.clone();
            match offer_wayland(vec![
                (WAYLAND_TEXT_TYPE.into(), self.text.as_bytes().to_vec()),
                (marker, Vec::new()),
            ]) {
                Ok(()) => {
                    if let Some(selection) = &self.selection {
                        for _ in 0..25 {
                            if matches!(selection.still_ours(), Ok(true)) {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    return Ok(());
                }
                Err(error) => {
                    tracing::debug!(%error, "copying without a Wayland restoration marker");
                    self.selection = None;
                }
            }
        }
        copy_text(self.text)
    }

    pub fn paste_on_x11(&self, shift: bool) -> anyhow::Result<bool> {
        match &self.selection {
            Some(Selection::X11(selection)) => selection.paste(shift),
            _ => Ok(false),
        }
    }

    pub fn ready_to_paste(&self) -> anyhow::Result<bool> {
        match &self.selection {
            Some(selection) => selection.still_ours(),
            None if process::wayland() => {
                let contents = process::run(
                    "wl-paste",
                    &["--no-newline", "--type", WAYLAND_TEXT_TYPE],
                    &[],
                )?;
                Ok(contents == self.text.as_bytes())
            }
            None => Ok(false),
        }
    }

    pub fn can_restore(&self) -> bool {
        self.previous.is_some() && self.selection.is_some()
    }

    pub fn restore_if_unchanged(self) -> anyhow::Result<()> {
        let (Some(formats), Some(selection)) = (self.previous, self.selection) else {
            return Ok(());
        };
        // A new copy can still arrive after this check and before the restore writes.
        if !selection.still_ours()? {
            return Ok(());
        }
        if matches!(selection, Selection::Wayland(_)) {
            if formats.is_empty() {
                wl_clipboard_rs::copy::clear(ClipboardType::Regular, Seat::All)?;
            } else {
                offer_wayland(formats)?;
            }
        } else {
            let text = formats
                .iter()
                .find(|(mime, _)| mime == "UTF8_STRING")
                .or_else(|| formats.first())
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
    fn an_x11_claim_cannot_be_recovered_after_loss() {
        use xfixes::SelectionEvent;
        let claim = |subtype, owner| xfixes::SelectionNotifyEvent {
            subtype,
            owner,
            ..Default::default()
        };
        let offered = claim(SelectionEvent::SET_SELECTION_OWNER, 1);
        let state = X11Ownership::AwaitingClaim.observe(offered);
        assert!(matches!(state, X11Ownership::Claimed));
        for event in [
            offered,
            claim(SelectionEvent::SET_SELECTION_OWNER, 2),
            claim(SelectionEvent::SET_SELECTION_OWNER, x11rb::NONE),
            claim(SelectionEvent::SELECTION_WINDOW_DESTROY, 1),
            claim(SelectionEvent::SELECTION_CLIENT_CLOSE, 1),
        ] {
            let lost = state.observe(event);
            assert!(matches!(lost, X11Ownership::Lost));
            assert!(matches!(lost.observe(offered), X11Ownership::Lost));
        }
        assert!(matches!(
            X11Ownership::AwaitingClaim
                .observe(claim(SelectionEvent::SET_SELECTION_OWNER, x11rb::NONE)),
            X11Ownership::Lost
        ));
    }

    #[test]
    fn paste_requires_v_in_the_active_x11_group() {
        use xkb::SAType;
        use xproto::ModMask;

        let key = |syms: Vec<_>| xkb::KeySymMap {
            group_info: syms.len() as u8,
            width: 1,
            syms,
            ..Default::default()
        };
        let map = xkb::GetMapMap {
            modmap_rtrn: Some(vec![
                xkb::KeyModMap {
                    keycode: 37,
                    mods: ModMask::CONTROL,
                },
                xkb::KeyModMap {
                    keycode: 38,
                    mods: ModMask::SHIFT,
                },
            ]),
            types_rtrn: Some(vec![xkb::KeyType::default()]),
            syms_rtrn: Some(vec![
                key(vec![0xffe3]),
                key(vec![0xfe08]),
                key(vec![0x76, 0x06d7]),
            ]),
            key_actions: Some(xkb::GetMapMapKeyActions {
                acts_rtrn_count: vec![1, 1, 0],
                acts_rtrn_acts: vec![
                    xkb::SASetMods {
                        type_: SAType::SET_MODS,
                        mask: ModMask::CONTROL,
                        ..Default::default()
                    }
                    .into(),
                    xkb::SALockGroup {
                        type_: SAType::LOCK_GROUP,
                        ..Default::default()
                    }
                    .into(),
                ],
            }),
            ..Default::default()
        };
        let mut state = xkb::GetStateReply::default();
        assert_eq!(
            X11PasteKeys::resolve(&map, 37, &state, false).unwrap().v,
            39
        );
        assert!(X11PasteKeys::resolve(&map, 37, &state, true).is_err());
        state.group = xkb::Group::M2;
        assert!(X11PasteKeys::resolve(&map, 37, &state, false).is_err());
    }

    #[test]
    #[ignore = "uses the private X11 clipboard and keyboard"]
    fn x11_dispatch_sends_a_balanced_chord_only_while_owning_the_clipboard() {
        use xproto::{
            ConnectionExt as _, CreateWindowAux, EventMask, InputFocus, KeyButMask, WindowClass,
        };
        let (receiver, screen) = x11rb::connect(None).unwrap();
        let root = receiver.setup().roots[screen].root;
        let window = receiver.generate_id().unwrap();
        receiver
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                root,
                0,
                0,
                50,
                50,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE),
            )
            .unwrap()
            .check()
            .unwrap();
        receiver.map_window(window).unwrap().check().unwrap();
        receiver
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        let events = || {
            receiver.sync().unwrap();
            std::iter::from_fn(|| receiver.poll_for_event().unwrap())
                .filter_map(|event| match event {
                    Event::KeyPress(key) => Some((true, key.detail, key.state)),
                    Event::KeyRelease(key) => Some((false, key.detail, key.state)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let read = || process::run("xclip", &["-selection", "clipboard", "-out"], &[]).unwrap();
        let mapping = || {
            let setup = receiver.setup();
            receiver
                .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
                .unwrap()
                .reply()
                .unwrap()
        };
        xkb::use_extension(&receiver, 1, 0)
            .unwrap()
            .reply()
            .unwrap();
        let group = || {
            u8::from(
                xkb::get_state(&receiver, xkb::ID::USE_CORE_KBD.into())
                    .unwrap()
                    .reply()
                    .unwrap()
                    .group,
            )
        };
        let before = mapping();
        let original_group = group();
        for shift in [false, true] {
            copy_text("Greendale").unwrap();
            let mut paste = Paste::prepare("Troy Barnes");
            paste.copy_for_paste().unwrap();
            assert!(paste.paste_on_x11(shift).unwrap());
            let keys = events();
            let chord_size = if shift { 3 } else { 2 };
            assert_eq!(keys.len(), chord_size * 2);
            let (presses, releases) = keys.split_at(chord_size);
            for (down, up) in presses.iter().zip(releases.iter().rev()) {
                assert!(down.0 && !up.0);
                assert_eq!(down.1, up.1);
            }
            let (_, key, modifiers) = presses[chord_size - 1];
            assert!(modifiers.contains(KeyButMask::CONTROL));
            assert_eq!(modifiers.contains(KeyButMask::SHIFT), shift);
            assert_eq!(group(), original_group);
            assert_eq!(mapping().keysyms, before.keysyms);
            let v_key = receiver
                .get_keyboard_mapping(key, 1)
                .unwrap()
                .reply()
                .unwrap();
            assert!(
                v_key
                    .keysyms
                    .iter()
                    .any(|sym| X11_PASTE_SYMBOLS.contains(sym))
            );
            assert_eq!(read(), b"Troy Barnes");
            paste.restore_if_unchanged().unwrap();
            assert_eq!(read(), b"Greendale");
        }

        let mut paste = Paste::prepare("Troy Barnes");
        paste.copy_for_paste().unwrap();
        assert!(paste.ready_to_paste().unwrap());
        copy_text("Abed Nadir").unwrap();
        assert!(!paste.paste_on_x11(false).unwrap());
        assert!(events().is_empty());
        paste.restore_if_unchanged().unwrap();
        assert_eq!(read(), b"Abed Nadir");
    }

    #[test]
    #[ignore = "uses the private desktop clipboard"]
    fn restoring_preserves_a_new_copy_with_identical_text() {
        let read = || {
            if process::wayland() {
                process::run(
                    "wl-paste",
                    &["--no-newline", "--type", WAYLAND_TEXT_TYPE],
                    &[],
                )
            } else {
                process::run("xclip", &["-selection", "clipboard", "-out"], &[])
            }
            .unwrap()
        };
        for copied_again in [false, true] {
            copy_text("Greendale").unwrap();
            let mut paste = Paste::prepare("Troy Barnes");
            assert!(paste.can_restore());
            paste.copy_for_paste().unwrap();
            assert_eq!(read(), b"Troy Barnes");
            assert!(paste.ready_to_paste().unwrap());
            if copied_again {
                if process::wayland() {
                    offer_wayland(vec![
                        (WAYLAND_TEXT_TYPE.into(), b"Troy Barnes".to_vec()),
                        ("text/html".into(), b"<b>Troy Barnes</b>".to_vec()),
                    ])
                    .unwrap();
                } else {
                    copy_text("Troy Barnes").unwrap();
                }
                assert!(!paste.ready_to_paste().unwrap());
            }
            paste.restore_if_unchanged().unwrap();
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

    #[test]
    #[ignore = "uses the private X11 clipboard"]
    fn newer_x11_copies_before_preflight_stop_paste_and_restore() {
        assert!(!process::wayland());
        copy_text("Greendale").unwrap();
        let mut paste = Paste::prepare("Troy Barnes");
        assert!(paste.can_restore());
        paste.copy_for_paste().unwrap();
        copy_text("Britta Perry").unwrap();
        copy_text("Abed Nadir").unwrap();

        assert!(!paste.paste_on_x11(false).unwrap());
        assert!(!paste.ready_to_paste().unwrap());
        assert!(!paste.ready_to_paste().unwrap());
        paste.restore_if_unchanged().unwrap();
        assert_eq!(
            process::run("xclip", &["-selection", "clipboard", "-out"], &[]).unwrap(),
            b"Abed Nadir"
        );
    }

    #[test]
    fn an_x11_paste_without_a_watcher_does_not_dispatch() {
        let paste = Paste {
            text: "Troy Barnes",
            previous: None,
            selection: None,
        };
        assert!(!paste.paste_on_x11(false).unwrap());
        assert!(!paste.can_restore());
    }

    #[test]
    #[ignore = "uses the private Wayland clipboard"]
    fn wayland_paste_without_a_marker_checks_the_current_text() {
        assert!(process::wayland());
        let mut paste = Paste {
            text: "Troy Barnes\n",
            previous: None,
            selection: None,
        };
        paste.copy_for_paste().unwrap();
        assert!(paste.ready_to_paste().unwrap());
        copy_text("Greendale").unwrap();
        assert!(!paste.ready_to_paste().unwrap());
        paste.restore_if_unchanged().unwrap();
        assert_eq!(
            process::run("wl-paste", &["--no-newline"], &[]).unwrap(),
            b"Greendale"
        );
    }

    #[test]
    fn x11_target_filter_skips_icccm_metadata_and_rejects_non_text() {
        assert_eq!(
            filter_x11_targets(
                "TARGETS\nTIMESTAMP\nDELETE\nINSERT_SELECTION\nUTF8_STRING\nSTRING\n"
            )
            .unwrap(),
            ["UTF8_STRING", "STRING"]
        );
        assert!(filter_x11_targets("TARGETS\nUTF8_STRING\nimage/png\n").is_err());
        assert!(filter_x11_targets("text/html\nUTF8_STRING\n").is_err());
    }

    #[test]
    fn wayland_mime_validation_rejects_flag_like_or_empty_types() {
        assert_eq!(
            validate_wayland_mime("text/plain;charset=utf-8").unwrap(),
            "text/plain;charset=utf-8"
        );
        assert!(validate_wayland_mime("").is_err());
        assert!(validate_wayland_mime("--clear").is_err());
        assert!(validate_wayland_mime("-n").is_err());
    }

    #[test]
    fn xkb_out_of_range_group_handling_wraps_clamps_and_redirects() {
        use xkb::SAType;
        use xproto::ModMask;

        let make_map = |v_group_info: u8, v_syms: Vec<u32>| xkb::GetMapMap {
            modmap_rtrn: Some(vec![xkb::KeyModMap {
                keycode: 37,
                mods: ModMask::CONTROL,
            }]),
            types_rtrn: Some(vec![xkb::KeyType::default()]),
            syms_rtrn: Some(vec![
                xkb::KeySymMap {
                    group_info: 1,
                    width: 1,
                    syms: vec![0xffe3],
                    ..Default::default()
                },
                xkb::KeySymMap {
                    group_info: v_group_info,
                    width: 1,
                    syms: v_syms,
                    ..Default::default()
                },
            ]),
            key_actions: Some(xkb::GetMapMapKeyActions {
                acts_rtrn_count: vec![1, 0],
                acts_rtrn_acts: vec![
                    xkb::SASetMods {
                        type_: SAType::SET_MODS,
                        mask: ModMask::CONTROL,
                        ..Default::default()
                    }
                    .into(),
                ],
            }),
            ..Default::default()
        };

        let state = xkb::GetStateReply {
            // index 2, out of range for 2-group key
            group: xkb::Group::M3,
            ..Default::default()
        };

        // WrapIntoRange (0x00): 2 % 2 = 0 -> 'v' (0x76)
        let wrap_map = make_map(0x02, vec![0x76, 0x06d7]);
        assert_eq!(
            X11PasteKeys::resolve(&wrap_map, 37, &state, false)
                .unwrap()
                .v,
            38
        );

        // ClampIntoRange (0x40): group_count - 1 = 1 -> 0x06d7 (not 'v')
        let clamp_map = make_map(0x40 | 0x02, vec![0x76, 0x06d7]);
        assert!(X11PasteKeys::resolve(&clamp_map, 37, &state, false).is_err());

        // RedirectIntoRange (0x80) to group 0 -> 'v' (0x76)
        let redirect_map = make_map(0x80 | 0x02, vec![0x76, 0x06d7]);
        assert_eq!(
            X11PasteKeys::resolve(&redirect_map, 37, &state, false)
                .unwrap()
                .v,
            38
        );

        // Zero groups: unmapped
        let empty_map = make_map(0x00, vec![]);
        assert!(X11PasteKeys::resolve(&empty_map, 37, &state, false).is_err());
    }
}
