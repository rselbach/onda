//! The shortcut recorder (HOT-8..HOT-11). The core runs the recorder. This
//! is what a place that records a shortcut shows, and the one key monitor
//! that hears the keys for every such place.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, glib};
use onda_domain::hotkey::{
    HeldModifiers, HotkeyView, KeyCode, Modifier, ModifierKey, ModifierSet, RecorderEvent,
};

use super::super::hotkeys::keys;
use super::{Window, clear};

fn modifiers(state: gdk::ModifierType) -> ModifierSet {
    [
        (gdk::ModifierType::CONTROL_MASK, Modifier::Control),
        (gdk::ModifierType::ALT_MASK, Modifier::Alt),
        (gdk::ModifierType::SHIFT_MASK, Modifier::Shift),
        (gdk::ModifierType::SUPER_MASK, Modifier::Meta),
    ]
    .into_iter()
    .filter(|(mask, _)| state.contains(*mask))
    .map(|(_, modifier)| modifier)
    .collect()
}

/// What the core's recorder hears for one GDK key event. `state` is the
/// modifier state before the event, which is what GDK reports. The release
/// of a key that is no modifier is nothing to the recorder.
fn recorder_event(pressed: bool, keycode: u32, state: gdk::ModifierType) -> Option<RecorderEvent> {
    let code = keys::code(keycode);
    let key = KeyCode::parse(code)?;
    let held = modifiers(state);
    let Some(own) = ModifierKey::from_code(code).map(ModifierKey::modifier) else {
        return pressed.then_some(RecorderEvent::KeyDown {
            key,
            modifiers: held,
        });
    };
    // the event changes the key's own modifier, and nothing says whether the
    // key on the other side still holds it down after a release
    let held = held
        .iter()
        .filter(|modifier| Some(*modifier) != own)
        .chain(own.filter(|_| pressed))
        .collect();
    Some(RecorderEvent::FlagsChanged {
        key,
        held: HeldModifiers::new(held, false),
    })
}

fn forward(
    owner: &Weak<Window>,
    pressed: bool,
    keycode: u32,
    state: gdk::ModifierType,
) -> glib::Propagation {
    let swallow = owner
        .upgrade()
        .zip(recorder_event(pressed, keycode, state))
        .is_some_and(|(owner, event)| owner.core.app.recorder_event(event));
    if swallow {
        return glib::Propagation::Stop;
    }
    glib::Propagation::Proceed
}

/// The recorder's key monitor, one for the whole application. It listens on
/// every window exactly while the core says so, hands each key event to the
/// core, and swallows what the core says to swallow. Every place that shows
/// [`Controls`] shares it.
#[derive(Default)]
pub(super) struct KeyMonitor {
    listening: RefCell<Vec<(gtk::Widget, gtk::EventControllerKey)>>,
}

impl KeyMonitor {
    /// Follows the core's `listening` flag.
    pub fn render(&self, owner: &Rc<Window>, view: &HotkeyView) {
        let installed = !self.listening.borrow().is_empty();
        match (view.listening, installed) {
            (true, false) => self.install(owner),
            (false, true) => {
                for (window, keys) in self.listening.take() {
                    window.remove_controller(&keys);
                }
            }
            _ => {}
        }
    }

    fn install(&self, owner: &Rc<Window>) {
        let windows = gtk::Window::list_toplevels();
        if windows.is_empty() {
            tracing::error!("no window can listen for the shortcut recorder");
            owner.core.app.hotkey_capture_failed();
            return;
        }
        let listening = windows.into_iter().map(|window| {
            let keys = gtk::EventControllerKey::new();
            // ahead of the focused widget and the window's own shortcuts, so
            // Space records instead of pressing the record button again
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(owner);
            keys.connect_key_pressed(move |_, _, keycode, state| {
                forward(&weak, true, keycode, state)
            });
            let weak = Rc::downgrade(owner);
            keys.connect_key_released(move |_, _, keycode, state| {
                forward(&weak, false, keycode, state);
            });
            window.add_controller(keys.clone());
            (window, keys)
        });
        self.listening.replace(listening.collect());
    }
}

/// The record button, Cancel while recording, the help text, and the
/// message for a rejected combination, all from `HotkeyChanged`.
pub(super) struct Controls {
    pub widget: gtk::Box,
    record: gtk::Button,
    cancel: gtk::Button,
    help: gtk::Label,
    message: gtk::Label,
}

impl Controls {
    pub fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let record = gtk::Button::new();
        let cancel = gtk::Button::new();
        cancel.add_css_class("flat");
        buttons.append(&record);
        buttons.append(&cancel);
        widget.append(&buttons);
        let help = caption();
        help.add_css_class("dim-label");
        widget.append(&help);
        let message = caption();
        message.add_css_class("warning");
        widget.append(&message);
        Self {
            widget,
            record,
            cancel,
            help,
            message,
        }
    }

    pub fn connect(&self, owner: &Rc<Window>) {
        let weak = Rc::downgrade(owner);
        self.record.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.toggle_hotkey_capture();
            }
        });
        let weak = Rc::downgrade(owner);
        self.cancel.connect_clicked(move |_| {
            if let Some(owner) = weak.upgrade() {
                owner.core.app.end_hotkey_capture();
            }
        });
    }

    pub fn render(&self, view: &HotkeyView) {
        self.record.set_label(view.record_button_title);
        self.cancel.set_label(view.cancel_button_title);
        self.cancel.set_visible(view.recording);
        self.help.set_text(&view.help_text);
        self.message.set_text(view.message.unwrap_or_default());
        self.message.set_visible(view.message.is_some());
    }
}

fn caption() -> gtk::Label {
    gtk::Label::builder().wrap(true).xalign(0.0).build()
}

/// An empty row of keycaps for [`set_keycaps`] to fill.
pub(super) fn keycaps() -> gtk::Box {
    gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        // the system theme draws a `.keycap` only inside the node of its
        // own shortcut label
        .css_name("shortcut")
        .build()
}

/// Shows a shortcut as one keycap per key: `Ctrl` `Alt` `Space`.
pub(super) fn set_keycaps(keycaps: &gtk::Box, labels: &[String]) {
    clear(keycaps);
    for text in labels {
        let keycap = gtk::Label::new(Some(text));
        keycap.add_css_class("keycap");
        keycaps.append(&keycap);
    }
}

#[cfg(test)]
mod tests {
    use gtk::gdk::ModifierType;

    use super::*;

    const CONTROL_LEFT: u32 = 37;
    const CONTROL_RIGHT: u32 = 105;
    const ALT_LEFT: u32 = 64;
    const SUPER_LEFT: u32 = 133;
    const SPACE: u32 = 65;
    const KEY_G: u32 = 42;
    const KEY_Q: u32 = 24;
    const ESCAPE: u32 = 9;
    const CAPS_LOCK: u32 = 66;

    fn held(modifiers: &[Modifier]) -> ModifierSet {
        modifiers.iter().copied().collect()
    }

    fn key(code: &str) -> KeyCode {
        KeyCode::parse(code).unwrap()
    }

    #[test]
    fn a_key_press_reaches_the_recorder_as_its_place_on_the_keyboard() {
        use Modifier::{Alt, Control, Meta, Shift};
        struct Case {
            keycode: u32,
            state: ModifierType,
            want_key: &'static str,
            want_modifiers: &'static [Modifier],
        }
        let cases = [
            (
                "the key a US keyboard labels G",
                Case {
                    keycode: KEY_G,
                    state: ModifierType::CONTROL_MASK | ModifierType::ALT_MASK,
                    want_key: "KeyG",
                    want_modifiers: &[Control, Alt],
                },
            ),
            (
                "the key a French keyboard labels A",
                Case {
                    keycode: KEY_Q,
                    state: ModifierType::SUPER_MASK,
                    want_key: "KeyQ",
                    want_modifiers: &[Meta],
                },
            ),
            (
                "a key alone",
                Case {
                    keycode: SPACE,
                    state: ModifierType::empty(),
                    want_key: "Space",
                    want_modifiers: &[],
                },
            ),
            (
                "locks and pointer buttons are no modifiers of a shortcut",
                Case {
                    keycode: SPACE,
                    state: ModifierType::SHIFT_MASK
                        | ModifierType::LOCK_MASK
                        | ModifierType::BUTTON1_MASK,
                    want_key: "Space",
                    want_modifiers: &[Shift],
                },
            ),
            (
                "Escape",
                Case {
                    keycode: ESCAPE,
                    state: ModifierType::empty(),
                    want_key: "Escape",
                    want_modifiers: &[],
                },
            ),
            (
                "Caps Lock is a key like any other",
                Case {
                    keycode: CAPS_LOCK,
                    state: ModifierType::CONTROL_MASK,
                    want_key: "CapsLock",
                    want_modifiers: &[Control],
                },
            ),
            (
                "a key the table does not hold",
                Case {
                    keycode: 248,
                    state: ModifierType::CONTROL_MASK,
                    want_key: "Unidentified",
                    want_modifiers: &[Control],
                },
            ),
            (
                "a keycode below the first key",
                Case {
                    keycode: 3,
                    state: ModifierType::ALT_MASK,
                    want_key: "Unidentified",
                    want_modifiers: &[Alt],
                },
            ),
        ];
        for (name, tc) in cases {
            let want = RecorderEvent::KeyDown {
                key: key(tc.want_key),
                modifiers: held(tc.want_modifiers),
            };
            assert_eq!(
                recorder_event(true, tc.keycode, tc.state),
                Some(want),
                "{name}"
            );
            assert_eq!(recorder_event(false, tc.keycode, tc.state), None, "{name}");
        }
    }

    #[test]
    fn a_modifier_key_reports_what_is_held_after_it_moved() {
        use Modifier::{Alt, Control};
        struct Case {
            pressed: bool,
            keycode: u32,
            state: ModifierType,
            want_key: &'static str,
            want_held: &'static [Modifier],
        }
        let cases = [
            (
                "the first modifier goes down",
                Case {
                    pressed: true,
                    keycode: CONTROL_LEFT,
                    state: ModifierType::empty(),
                    want_key: "ControlLeft",
                    want_held: &[Control],
                },
            ),
            (
                "a second modifier goes down",
                Case {
                    pressed: true,
                    keycode: ALT_LEFT,
                    state: ModifierType::CONTROL_MASK,
                    want_key: "AltLeft",
                    want_held: &[Control, Alt],
                },
            ),
            (
                "one of two modifiers goes up",
                Case {
                    pressed: false,
                    keycode: CONTROL_RIGHT,
                    state: ModifierType::CONTROL_MASK | ModifierType::ALT_MASK,
                    want_key: "ControlRight",
                    want_held: &[Alt],
                },
            ),
            (
                "the last modifier goes up",
                Case {
                    pressed: false,
                    keycode: SUPER_LEFT,
                    state: ModifierType::SUPER_MASK,
                    want_key: "MetaLeft",
                    want_held: &[],
                },
            ),
        ];
        for (name, tc) in cases {
            let want = RecorderEvent::FlagsChanged {
                key: key(tc.want_key),
                held: HeldModifiers::new(held(tc.want_held), false),
            };
            assert_eq!(
                recorder_event(tc.pressed, tc.keycode, tc.state),
                Some(want),
                "{name}"
            );
        }
    }
}
