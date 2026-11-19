//! Keys by their place on the keyboard, which is how a shortcut names its
//! key. The same place has the same name in every keyboard layout.

/// X11 and GDK number a key this far above the kernel's number for it.
const XKB_OFFSET: u32 = 8;

/// The W3C code the core gets for a key this table does not hold.
const UNIDENTIFIED: &str = "Unidentified";

/// The kernel's number for each key, from `linux/input-event-codes.h`, and
/// the key's W3C `KeyboardEvent.code`.
#[rustfmt::skip]
const KEYS: &[(u32, &str)] = &[
    (1, "Escape"), (2, "Digit1"), (3, "Digit2"), (4, "Digit3"), (5, "Digit4"), (6, "Digit5"),
    (7, "Digit6"), (8, "Digit7"), (9, "Digit8"), (10, "Digit9"), (11, "Digit0"), (12, "Minus"),
    (13, "Equal"), (14, "Backspace"), (15, "Tab"), (16, "KeyQ"), (17, "KeyW"), (18, "KeyE"),
    (19, "KeyR"), (20, "KeyT"), (21, "KeyY"), (22, "KeyU"), (23, "KeyI"), (24, "KeyO"),
    (25, "KeyP"), (26, "BracketLeft"), (27, "BracketRight"), (28, "Enter"), (29, "ControlLeft"),
    (30, "KeyA"), (31, "KeyS"), (32, "KeyD"), (33, "KeyF"), (34, "KeyG"), (35, "KeyH"),
    (36, "KeyJ"), (37, "KeyK"), (38, "KeyL"), (39, "Semicolon"), (40, "Quote"),
    (41, "Backquote"), (42, "ShiftLeft"), (43, "Backslash"), (44, "KeyZ"), (45, "KeyX"),
    (46, "KeyC"), (47, "KeyV"), (48, "KeyB"), (49, "KeyN"), (50, "KeyM"), (51, "Comma"),
    (52, "Period"), (53, "Slash"), (54, "ShiftRight"), (55, "NumpadMultiply"), (56, "AltLeft"),
    (57, "Space"), (58, "CapsLock"), (59, "F1"), (60, "F2"), (61, "F3"), (62, "F4"), (63, "F5"),
    (64, "F6"), (65, "F7"), (66, "F8"), (67, "F9"), (68, "F10"), (69, "NumLock"),
    (70, "ScrollLock"), (71, "Numpad7"), (72, "Numpad8"), (73, "Numpad9"),
    (74, "NumpadSubtract"), (75, "Numpad4"), (76, "Numpad5"), (77, "Numpad6"),
    (78, "NumpadAdd"), (79, "Numpad1"), (80, "Numpad2"), (81, "Numpad3"), (82, "Numpad0"),
    (83, "NumpadDecimal"), (86, "IntlBackslash"), (87, "F11"), (88, "F12"), (89, "IntlRo"),
    (96, "NumpadEnter"), (97, "ControlRight"), (98, "NumpadDivide"), (99, "PrintScreen"),
    (100, "AltRight"), (102, "Home"), (103, "ArrowUp"), (104, "PageUp"), (105, "ArrowLeft"),
    (106, "ArrowRight"), (107, "End"), (108, "ArrowDown"), (109, "PageDown"), (110, "Insert"),
    (111, "Delete"), (117, "NumpadEqual"), (119, "Pause"), (121, "NumpadComma"),
    (124, "IntlYen"), (125, "MetaLeft"), (126, "MetaRight"), (127, "ContextMenu"), (183, "F13"),
    (184, "F14"), (185, "F15"), (186, "F16"), (187, "F17"), (188, "F18"), (189, "F19"),
    (190, "F20"), (191, "F21"), (192, "F22"), (193, "F23"), (194, "F24"),
];

/// The W3C code of the key that GDK and X11 report as `keycode`.
pub fn code(keycode: u32) -> &'static str {
    KEYS.iter()
        .find(|(key, _)| Some(*key) == keycode.checked_sub(XKB_OFFSET))
        .map_or(UNIDENTIFIED, |(_, code)| code)
}

/// The keycode X11 reports for the key with the W3C code `code`.
pub fn keycode(code: &str) -> Option<u8> {
    let (key, _) = KEYS.iter().find(|(_, known)| *known == code)?;
    u8::try_from(key + XKB_OFFSET).ok()
}

/// Every W3C `KeyboardEvent.code` in the table.
#[cfg(test)]
pub fn codes() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|(_, code)| *code)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use onda_domain::hotkey::{KeyCode, ModifierKey};

    use super::*;

    #[test]
    fn a_key_keeps_its_code_whichever_way_it_is_looked_up() {
        let cases = [
            ("the key a US keyboard labels G", 42, "KeyG"),
            ("the key a French keyboard labels A", 24, "KeyQ"),
            ("Space", 65, "Space"),
            ("the last key of the table", 202, "F24"),
        ];
        for (name, hardware, w3c) in cases {
            assert_eq!(code(hardware), w3c, "{name}");
            assert_eq!(keycode(w3c), u8::try_from(hardware).ok(), "{name}");
        }
        for unknown in [0, 3, 248, u32::MAX] {
            assert_eq!(code(unknown), UNIDENTIFIED, "{unknown}");
        }
        assert_eq!(keycode(UNIDENTIFIED), None);
    }

    #[test]
    fn each_key_has_one_place_and_one_code() {
        let places: BTreeSet<u32> = KEYS.iter().map(|(place, _)| *place).collect();
        let codes: BTreeSet<&str> = KEYS.iter().map(|(_, code)| *code).collect();
        assert_eq!(places.len(), KEYS.len());
        assert_eq!(codes.len(), KEYS.len());
        for (_, code) in KEYS {
            assert!(KeyCode::parse(code).is_some(), "{code}");
        }
        for modifier in ModifierKey::ALL {
            assert_eq!(
                codes.contains(modifier.code()),
                modifier != ModifierKey::Fn,
                "{modifier:?}"
            );
        }
    }
}
