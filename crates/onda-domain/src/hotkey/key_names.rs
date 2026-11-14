//! How a system writes keys and shortcuts (HOT-6). macOS draws a glyph per
//! modifier; a Linux desktop spells the names and joins them with `+`.
//!
//! A system is one table here. Display code reads the table, so no rule
//! asks which system it is on.

use super::shortcut::{Modifier, ModifierKey};

/// What a system calls one modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierNames {
    /// Inside a combo and on a keycap: `⌥`, `Alt`.
    pub label: &'static str,
    /// The left key on its own: `Option`, `Alt`.
    pub key: &'static str,
    /// The right key on its own: `Right Option`, `Right Alt`.
    pub right_key: &'static str,
}

/// A system's names for keys, and the copy that spells those names out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyNames {
    /// Control.
    pub control: ModifierNames,
    /// Option on macOS, Alt elsewhere.
    pub alt: ModifierNames,
    /// Shift.
    pub shift: ModifierNames,
    /// Command on macOS, Super elsewhere.
    pub meta: ModifierNames,
    /// Written between two modifier labels of a combo.
    pub modifier_separator: &'static str,
    /// Written between a combo's modifiers and its key.
    pub key_separator: &'static str,
    /// The key W3C calls `Enter`.
    pub enter: &'static str,
    /// The key W3C calls `Backspace`.
    pub backspace: &'static str,
    /// The paste shortcut as the system's menus write it.
    pub paste: &'static str,
    /// The message for a combo without a required modifier (HOT-10).
    pub missing_modifier_message: &'static str,
    /// The recorder's help text while recording (HOT-11).
    pub recording_help: &'static str,
    /// The footer under the Hotkey page's shortcut section (WIN-16).
    pub hotkey_footer: &'static str,
}

impl KeyNames {
    /// macOS: modifier glyphs, then a space, then the key (`⌃⌥ Space`).
    pub const MACOS: Self = Self {
        control: ModifierNames {
            label: "⌃",
            key: "Control",
            right_key: "Right Control",
        },
        alt: ModifierNames {
            label: "⌥",
            key: "Option",
            right_key: "Right Option",
        },
        shift: ModifierNames {
            label: "⇧",
            key: "Shift",
            right_key: "Right Shift",
        },
        meta: ModifierNames {
            label: "⌘",
            key: "Command",
            right_key: "Right Command",
        },
        modifier_separator: "",
        key_separator: " ",
        enter: "Return",
        backspace: "Delete",
        paste: "⌘V",
        missing_modifier_message: "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn.",
        recording_help: "Press a key with Command, Control, or Option. A lone modifier like Right Option or Fn also works. Escape cancels.",
        hotkey_footer: "Shortcuts must include Command, Control, or Option — or use a single \
                        modifier key like Right Option or Fn on its own — so Onda does not steal \
                        every innocent keystroke like a gremlin.",
    };

    /// A Linux desktop: names joined with `+` (`Ctrl+Alt+Space`). Its copy
    /// offers no lone modifier, which a Linux desktop cannot register.
    pub const LINUX: Self = Self {
        control: ModifierNames {
            label: "Ctrl",
            key: "Ctrl",
            right_key: "Right Ctrl",
        },
        alt: ModifierNames {
            label: "Alt",
            key: "Alt",
            right_key: "Right Alt",
        },
        shift: ModifierNames {
            label: "Shift",
            key: "Shift",
            right_key: "Right Shift",
        },
        meta: ModifierNames {
            label: "Super",
            key: "Super",
            right_key: "Right Super",
        },
        modifier_separator: "+",
        key_separator: "+",
        enter: "Enter",
        backspace: "Backspace",
        paste: "Ctrl+V",
        missing_modifier_message: "Shortcut must include Ctrl, Alt, or Super.",
        recording_help: "Press a key with Ctrl, Alt, or Super. Escape cancels.",
        hotkey_footer: "Shortcuts must include Ctrl, Alt, or Super, so Onda does not steal \
                        every innocent keystroke like a gremlin.",
    };

    /// The names of `modifier`.
    pub fn modifier(&self, modifier: Modifier) -> &ModifierNames {
        match modifier {
            Modifier::Control => &self.control,
            Modifier::Alt => &self.alt,
            Modifier::Shift => &self.shift,
            Modifier::Meta => &self.meta,
        }
    }

    /// The name of a modifier key on its own (HOT-6), e.g. `Right Option`.
    /// Fn has one name everywhere.
    pub fn modifier_key(&self, key: ModifierKey) -> &'static str {
        match key {
            ModifierKey::ControlLeft => self.control.key,
            ModifierKey::ControlRight => self.control.right_key,
            ModifierKey::AltLeft => self.alt.key,
            ModifierKey::AltRight => self.alt.right_key,
            ModifierKey::ShiftLeft => self.shift.key,
            ModifierKey::ShiftRight => self.shift.right_key,
            ModifierKey::MetaLeft => self.meta.key,
            ModifierKey::MetaRight => self.meta.right_key,
            ModifierKey::Fn => "Fn",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_key_names_hot_6() {
        let cases = [
            (ModifierKey::ControlLeft, "Control", "Ctrl"),
            (ModifierKey::ControlRight, "Right Control", "Right Ctrl"),
            (ModifierKey::AltLeft, "Option", "Alt"),
            (ModifierKey::AltRight, "Right Option", "Right Alt"),
            (ModifierKey::ShiftLeft, "Shift", "Shift"),
            (ModifierKey::ShiftRight, "Right Shift", "Right Shift"),
            (ModifierKey::MetaLeft, "Command", "Super"),
            (ModifierKey::MetaRight, "Right Command", "Right Super"),
            (ModifierKey::Fn, "Fn", "Fn"),
        ];
        for (key, want_macos, want_linux) in cases {
            assert_eq!(KeyNames::MACOS.modifier_key(key), want_macos, "{key:?}");
            assert_eq!(KeyNames::LINUX.modifier_key(key), want_linux, "{key:?}");
        }
    }
}
