//! Shortcut model: key ids, modifier sets, validation, display, and the
//! settings JSON form (HOT-1 to HOT-7).

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::platform::Platform;

/// A key identified by its W3C `KeyboardEvent.code` name (HOT-1), such as
/// `Space`, `KeyA`, `F5`, `AltRight`, or `Fn`.
///
/// Every W3C code is a non-empty run of ASCII letters and digits; anything
/// else is rejected so junk from a hand-edited settings file never reaches
/// the display or the shell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCode(Cow<'static, str>);

impl KeyCode {
    /// The space bar.
    pub const SPACE: Self = Self(Cow::Borrowed("Space"));
    /// Escape, reserved for cancelling (HOT-4, HOT-34).
    pub const ESCAPE: Self = Self(Cow::Borrowed("Escape"));

    /// Parses a W3C `KeyboardEvent.code` name, or returns `None` when `code`
    /// is empty or contains anything but ASCII letters and digits.
    pub fn parse(code: &str) -> Option<Self> {
        if code.is_empty() || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        Some(Self(Cow::Owned(code.to_owned())))
    }

    /// The W3C code name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key's display name (HOT-6): letters and digits as themselves,
    /// named keys spelled out, arrows and punctuation as glyphs, and
    /// `Key <code>` for anything unknown.
    pub fn display_name(&self) -> String {
        if let Some(name) = named_key(self.as_str()) {
            return name.to_owned();
        }
        if let Some(modifier) = ModifierKey::from_code(self.as_str()) {
            return modifier.display_name().to_owned();
        }
        let code = self.as_str();
        if let Some(rest) = code.strip_prefix("Key")
            && rest.len() == 1
            && rest.bytes().all(|b| b.is_ascii_uppercase())
        {
            return rest.to_owned();
        }
        if let Some(rest) = code.strip_prefix("Digit")
            && rest.len() == 1
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            return rest.to_owned();
        }
        if let Some(rest) = code.strip_prefix('F')
            && let Ok(n) = rest.parse::<u8>()
            && (1..=12).contains(&n)
            && !rest.starts_with('0')
        {
            return code.to_owned();
        }
        format!("Key {code}")
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ModifierKey> for KeyCode {
    fn from(key: ModifierKey) -> Self {
        Self(Cow::Borrowed(key.code()))
    }
}

// display names for named keys and punctuation, keyed by W3C code (HOT-6).
// on macOS the key W3C calls `Backspace` is labelled Delete.
fn named_key(code: &str) -> Option<&'static str> {
    let name = match code {
        "Enter" => "Return",
        "Tab" => "Tab",
        "Space" => "Space",
        "Backspace" => "Delete",
        "Escape" => "Escape",
        "CapsLock" => "Caps Lock",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "Page Up",
        "PageDown" => "Page Down",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowDown" => "↓",
        "ArrowUp" => "↑",
        "Equal" => "=",
        "Minus" => "-",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Quote" => "'",
        "Semicolon" => ";",
        "Backslash" => "\\",
        "Comma" => ",",
        "Slash" => "/",
        "Period" => ".",
        "Backquote" => "`",
        _ => return None,
    };
    Some(name)
}

/// One member of a shortcut's modifier set (HOT-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// Control (`⌃`).
    Control,
    /// Option on macOS, Alt elsewhere (`⌥`).
    Alt,
    /// Shift (`⇧`).
    Shift,
    /// Command on macOS, the Windows/Super key elsewhere (`⌘`).
    Meta,
}

impl Modifier {
    /// Every modifier in display order (HOT-6).
    pub const ALL: [Self; 4] = [Self::Control, Self::Alt, Self::Shift, Self::Meta];

    /// The settings name: `control`, `alt`, `shift`, or `meta`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Alt => "alt",
            Self::Shift => "shift",
            Self::Meta => "meta",
        }
    }

    /// Parses a settings name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The macOS glyph shown in shortcut displays and keycaps.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Control => "⌃",
            Self::Alt => "⌥",
            Self::Shift => "⇧",
            Self::Meta => "⌘",
        }
    }

    fn bit(self) -> u8 {
        match self {
            Self::Control => 1,
            Self::Alt => 1 << 1,
            Self::Shift => 1 << 2,
            Self::Meta => 1 << 3,
        }
    }
}

/// A set of [`Modifier`]s. Fn is deliberately not a member: it is never part
/// of a combo (HOT-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModifierSet(u8);

impl ModifierSet {
    /// No modifiers.
    pub const EMPTY: Self = Self(0);

    /// Returns this set plus `modifier`.
    #[must_use]
    pub fn with(self, modifier: Modifier) -> Self {
        Self(self.0 | modifier.bit())
    }

    /// Whether `modifier` is in the set.
    pub fn contains(self, modifier: Modifier) -> bool {
        self.0 & modifier.bit() != 0
    }

    /// Whether the set has no modifiers.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The members in display order (`⌃ ⌥ ⇧ ⌘`).
    pub fn iter(self) -> impl Iterator<Item = Modifier> {
        Modifier::ALL.into_iter().filter(move |m| self.contains(*m))
    }
}

impl FromIterator<Modifier> for ModifierSet {
    fn from_iter<I: IntoIterator<Item = Modifier>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

/// A modifier key that can be a shortcut on its own (HOT-3). Left and right
/// are distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    /// Left Control (`ControlLeft`).
    ControlLeft,
    /// Right Control (`ControlRight`).
    ControlRight,
    /// Left Option/Alt (`AltLeft`).
    AltLeft,
    /// Right Option/Alt (`AltRight`).
    AltRight,
    /// Left Shift (`ShiftLeft`).
    ShiftLeft,
    /// Right Shift (`ShiftRight`).
    ShiftRight,
    /// Left Command/Meta (`MetaLeft`).
    MetaLeft,
    /// Right Command/Meta (`MetaRight`).
    MetaRight,
    /// Fn, the 🌐 Globe key on Macs (`Fn`).
    Fn,
}

impl ModifierKey {
    /// Every lone-modifier key.
    pub const ALL: [Self; 9] = [
        Self::ControlLeft,
        Self::ControlRight,
        Self::AltLeft,
        Self::AltRight,
        Self::ShiftLeft,
        Self::ShiftRight,
        Self::MetaLeft,
        Self::MetaRight,
        Self::Fn,
    ];

    /// The W3C `KeyboardEvent.code` name.
    pub fn code(self) -> &'static str {
        match self {
            Self::ControlLeft => "ControlLeft",
            Self::ControlRight => "ControlRight",
            Self::AltLeft => "AltLeft",
            Self::AltRight => "AltRight",
            Self::ShiftLeft => "ShiftLeft",
            Self::ShiftRight => "ShiftRight",
            Self::MetaLeft => "MetaLeft",
            Self::MetaRight => "MetaRight",
            Self::Fn => "Fn",
        }
    }

    /// The lone-modifier key with W3C code `code`, if any.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == code)
    }

    /// The modifier family both sides share, or `None` for Fn, which has no
    /// combo modifier.
    pub fn modifier(self) -> Option<Modifier> {
        match self {
            Self::ControlLeft | Self::ControlRight => Some(Modifier::Control),
            Self::AltLeft | Self::AltRight => Some(Modifier::Alt),
            Self::ShiftLeft | Self::ShiftRight => Some(Modifier::Shift),
            Self::MetaLeft | Self::MetaRight => Some(Modifier::Meta),
            Self::Fn => None,
        }
    }

    /// The display name (HOT-6), e.g. `Option` or `Right Option`.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ControlLeft => "Control",
            Self::ControlRight => "Right Control",
            Self::AltLeft => "Option",
            Self::AltRight => "Right Option",
            Self::ShiftLeft => "Shift",
            Self::ShiftRight => "Right Shift",
            Self::MetaLeft => "Command",
            Self::MetaRight => "Right Command",
            Self::Fn => "Fn",
        }
    }
}

/// Why a shortcut is not allowed (HOT-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationError {
    /// The key is Escape.
    EscapeReserved,
    /// A combo without Command, Control, or Option.
    MissingRequiredModifier,
}

impl ValidationError {
    /// The message shown under the recorder (HOT-10).
    pub fn message(self) -> &'static str {
        match self {
            Self::EscapeReserved => "Escape is reserved for cancelling shortcut recording.",
            Self::MissingRequiredModifier => {
                "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn."
            }
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ValidationError {}

/// A valid dictation shortcut: a key plus a modifier set (HOT-1).
///
/// Construction validates (HOT-4), so every `Shortcut` value is usable.
/// It is a **lone modifier** when the key is a [`ModifierKey`] and the set is
/// empty, and a **combo** otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shortcut {
    key: KeyCode,
    modifiers: ModifierSet,
}

impl Default for Shortcut {
    /// `⌥ Space` (HOT-2).
    fn default() -> Self {
        Self {
            key: KeyCode::SPACE,
            modifiers: ModifierSet::EMPTY.with(Modifier::Alt),
        }
    }
}

impl Shortcut {
    /// Builds a shortcut, or reports the first HOT-4 rule it breaks.
    pub fn new(key: KeyCode, modifiers: ModifierSet) -> Result<Self, ValidationError> {
        validate(&key, modifiers)?;
        Ok(Self { key, modifiers })
    }

    /// A lone-modifier shortcut, which is always valid (HOT-3, HOT-4).
    pub fn lone(key: ModifierKey) -> Self {
        Self {
            key: key.into(),
            modifiers: ModifierSet::EMPTY,
        }
    }

    /// The key.
    pub fn key(&self) -> &KeyCode {
        &self.key
    }

    /// The modifier set.
    pub fn modifiers(&self) -> ModifierSet {
        self.modifiers
    }

    /// The modifier key when this is a lone-modifier shortcut.
    pub fn lone_modifier(&self) -> Option<ModifierKey> {
        if !self.modifiers.is_empty() {
            return None;
        }
        ModifierKey::from_code(self.key.as_str())
    }

    /// Whether this is a lone-modifier shortcut.
    pub fn is_lone_modifier(&self) -> bool {
        self.lone_modifier().is_some()
    }

    /// Whether the shortcut needs the Accessibility permission (HOT-7):
    /// true exactly for lone modifiers, which macOS observes through an
    /// event tap. A system without the permission needs it for nothing.
    pub fn needs_accessibility(&self, platform: Platform) -> bool {
        platform.typing_permission.is_some() && self.is_lone_modifier()
    }

    /// The display string (HOT-6), e.g. `⌃⌥ Space` or `Right Option`.
    pub fn display(&self) -> String {
        let glyphs: String = self.modifiers.iter().map(Modifier::glyph).collect();
        let key = self.key.display_name();
        if glyphs.is_empty() {
            return key;
        }
        format!("{glyphs} {key}")
    }

    /// One keycap label per modifier glyph plus one for the whole key name
    /// (HOT-6): `⌃⌥ Space` gives `["⌃", "⌥", "Space"]`.
    pub fn keycaps(&self) -> Vec<String> {
        self.modifiers
            .iter()
            .map(|m| m.glyph().to_owned())
            .chain(std::iter::once(self.key.display_name()))
            .collect()
    }

    /// Parses the settings form `{"key":"Space","modifiers":["alt"]}`
    /// strictly, so the settings layer can log why a value was rejected
    /// (SET-2). Use the lenient [`Deserialize`] impl to read settings.
    pub fn from_setting(value: &serde_json::Value) -> Result<Self, ShortcutSettingError> {
        let wire: ShortcutWire =
            serde_json::from_value(value.clone()).map_err(|_| ShortcutSettingError::Malformed)?;
        let key = KeyCode::parse(&wire.key).ok_or(ShortcutSettingError::Malformed)?;
        let modifiers = wire
            .modifiers
            .iter()
            .map(|name| Modifier::from_name(name))
            .collect::<Option<ModifierSet>>()
            .ok_or(ShortcutSettingError::Malformed)?;
        Self::new(key, modifiers).map_err(ShortcutSettingError::Invalid)
    }

    fn to_wire(&self) -> ShortcutWire {
        ShortcutWire {
            key: self.key.as_str().to_owned(),
            modifiers: self.modifiers.iter().map(|m| m.name().to_owned()).collect(),
        }
    }
}

fn validate(key: &KeyCode, modifiers: ModifierSet) -> Result<(), ValidationError> {
    if *key == KeyCode::ESCAPE {
        return Err(ValidationError::EscapeReserved);
    }
    if modifiers.is_empty() && ModifierKey::from_code(key.as_str()).is_some() {
        return Ok(());
    }
    let required = [Modifier::Meta, Modifier::Control, Modifier::Alt];
    if required.into_iter().any(|m| modifiers.contains(m)) {
        return Ok(());
    }
    Err(ValidationError::MissingRequiredModifier)
}

/// Why a stored shortcut could not be read (SET-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutSettingError {
    /// Not the `{key, modifiers}` shape, an invalid key code, or an unknown
    /// modifier name.
    Malformed,
    /// Well formed but rejected by validation (HOT-4).
    Invalid(ValidationError),
}

impl fmt::Display for ShortcutSettingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => f.write_str("malformed hotkey setting"),
            Self::Invalid(err) => write!(f, "invalid hotkey setting: {err}"),
        }
    }
}

impl std::error::Error for ShortcutSettingError {}

#[derive(Serialize, Deserialize)]
struct ShortcutWire {
    key: String,
    modifiers: Vec<String>,
}

impl Serialize for Shortcut {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_wire().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    /// Lenient: any well-formed input that is not a valid shortcut reads back
    /// as the default (HOT-5, SET-2).
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self::from_setting(&value).unwrap_or_default())
    }
}

/// The modifier keys held according to a modifier-flags event, including Fn.
///
/// Only the shortcut recorder uses this (HOT-9); combos never include Fn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HeldModifiers {
    modifiers: ModifierSet,
    fn_key: bool,
}

impl HeldModifiers {
    /// Nothing held.
    pub const NONE: Self = Self {
        modifiers: ModifierSet::EMPTY,
        fn_key: false,
    };

    /// The held set from an event's modifier flags.
    pub fn new(modifiers: ModifierSet, fn_key: bool) -> Self {
        Self { modifiers, fn_key }
    }

    /// Exactly `key`'s family held and nothing else.
    pub fn only(key: ModifierKey) -> Self {
        match key.modifier() {
            Some(m) => Self::new(ModifierSet::EMPTY.with(m), false),
            None => Self::new(ModifierSet::EMPTY, true),
        }
    }

    /// Whether `key`'s family flag is held. Both sides share one flag.
    pub fn contains(self, key: ModifierKey) -> bool {
        match key.modifier() {
            Some(m) => self.modifiers.contains(m),
            None => self.fn_key,
        }
    }

    /// Whether nothing is held.
    pub fn is_empty(self) -> bool {
        self.modifiers.is_empty() && !self.fn_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy_naming::PrivacyNaming;

    fn key(code: &str) -> KeyCode {
        KeyCode::parse(code).expect("test key code")
    }

    fn mods(list: &[Modifier]) -> ModifierSet {
        list.iter().copied().collect()
    }

    fn combo(code: &str, list: &[Modifier]) -> Shortcut {
        Shortcut::new(key(code), mods(list)).expect("test shortcut is valid")
    }

    use Modifier::{Alt, Control, Meta, Shift};

    #[test]
    fn default_shortcut_is_option_space_hot_2() {
        let got = Shortcut::default();
        assert_eq!(got.key(), &KeyCode::SPACE);
        assert_eq!(got.modifiers(), mods(&[Alt]));
        assert_eq!(got.display(), "⌥ Space");
        assert_eq!(Shortcut::new(got.key().clone(), got.modifiers()), Ok(got));
    }

    #[test]
    fn validation_hot_4() {
        struct Case {
            name: &'static str,
            key: &'static str,
            modifiers: &'static [Modifier],
            want: Result<(), ValidationError>,
        }
        let cases = [
            Case {
                name: "no modifier",
                key: "KeyA",
                modifiers: &[],
                want: Err(ValidationError::MissingRequiredModifier),
            },
            Case {
                name: "shift only",
                key: "KeyA",
                modifiers: &[Shift],
                want: Err(ValidationError::MissingRequiredModifier),
            },
            Case {
                name: "escape with option is reserved",
                key: "Escape",
                modifiers: &[Alt],
                want: Err(ValidationError::EscapeReserved),
            },
            Case {
                name: "bare escape checks escape first",
                key: "Escape",
                modifiers: &[],
                want: Err(ValidationError::EscapeReserved),
            },
            Case {
                name: "command",
                key: "KeyA",
                modifiers: &[Meta],
                want: Ok(()),
            },
            Case {
                name: "control",
                key: "F5",
                modifiers: &[Control],
                want: Ok(()),
            },
            Case {
                name: "shift plus option",
                key: "KeyD",
                modifiers: &[Shift, Alt],
                want: Ok(()),
            },
            Case {
                name: "lone right option",
                key: "AltRight",
                modifiers: &[],
                want: Ok(()),
            },
            Case {
                name: "lone fn",
                key: "Fn",
                modifiers: &[],
                want: Ok(()),
            },
            Case {
                name: "modifier key with shift only is a combo missing a required modifier",
                key: "AltRight",
                modifiers: &[Shift],
                want: Err(ValidationError::MissingRequiredModifier),
            },
        ];
        for tc in cases {
            let got = Shortcut::new(key(tc.key), mods(tc.modifiers)).map(|_| ());
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn validation_messages_hot_4() {
        let cases = [
            (
                "escape",
                ValidationError::EscapeReserved,
                "Escape is reserved for cancelling shortcut recording.",
            ),
            (
                "missing modifier",
                ValidationError::MissingRequiredModifier,
                "Shortcut must include Command, Control, or Option — or use a lone modifier like Right Option or Fn.",
            ),
        ];
        for (name, err, want) in cases {
            assert_eq!(err.message(), want, "{name}");
            assert_eq!(err.to_string(), want, "{name}");
        }
    }

    #[test]
    fn every_lone_modifier_key_is_valid_and_lone_hot_3() {
        for k in ModifierKey::ALL {
            let got = Shortcut::new(k.into(), ModifierSet::EMPTY);
            assert_eq!(got, Ok(Shortcut::lone(k)), "{}", k.code());
            assert_eq!(Shortcut::lone(k).lone_modifier(), Some(k), "{}", k.code());
        }
    }

    #[test]
    fn lone_modifier_requires_no_extra_modifiers() {
        struct Case {
            name: &'static str,
            shortcut: Shortcut,
            want: Option<ModifierKey>,
        }
        let cases = [
            Case {
                name: "right option alone",
                shortcut: Shortcut::lone(ModifierKey::AltRight),
                want: Some(ModifierKey::AltRight),
            },
            Case {
                name: "right option with command is a combo",
                shortcut: combo("AltRight", &[Meta]),
                want: None,
            },
            Case {
                name: "option space",
                shortcut: Shortcut::default(),
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(tc.shortcut.lone_modifier(), tc.want, "{}", tc.name);
            assert_eq!(
                tc.shortcut.is_lone_modifier(),
                tc.want.is_some(),
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn only_lone_modifiers_need_accessibility_hot_7() {
        let macos = Platform::macos(PrivacyNaming::Accessibility);
        let macos_27 = Platform::macos(PrivacyNaming::DeviceControlAndDataAccess);
        let linux = Platform::linux();
        let right_option = Shortcut::lone(ModifierKey::AltRight);
        let cases = [
            ("option space", macos, Shortcut::default(), false),
            ("right option", macos, right_option.clone(), true),
            (
                "right option on macOS 27",
                macos_27,
                right_option.clone(),
                true,
            ),
            ("fn", macos, Shortcut::lone(ModifierKey::Fn), true),
            (
                "control option d",
                macos,
                combo("KeyD", &[Control, Alt]),
                false,
            ),
            ("right option on Linux", linux, right_option, false),
            ("option space on Linux", linux, Shortcut::default(), false),
        ];
        for (name, platform, shortcut, want) in cases {
            assert_eq!(shortcut.needs_accessibility(platform), want, "{name}");
        }
    }

    #[test]
    fn display_hot_6() {
        let cases = [
            (
                "all modifiers in stable order",
                combo("KeyA", &[Meta, Shift, Alt, Control]),
                "⌃⌥⇧⌘ A",
            ),
            ("option space", Shortcut::default(), "⌥ Space"),
            (
                "control option space",
                combo("Space", &[Control, Alt]),
                "⌃⌥ Space",
            ),
            (
                "right option",
                Shortcut::lone(ModifierKey::AltRight),
                "Right Option",
            ),
            ("fn", Shortcut::lone(ModifierKey::Fn), "Fn"),
            ("unknown key", combo("Numpad1", &[Meta]), "⌘ Key Numpad1"),
        ];
        for (name, shortcut, want) in cases {
            assert_eq!(shortcut.display(), want, "{name}");
        }
    }

    #[test]
    fn keycaps_split_modifiers_and_keep_key_whole_hot_6() {
        let cases = [
            (
                "modifiers plus key",
                combo("Space", &[Control, Alt]),
                vec!["⌃", "⌥", "Space"],
            ),
            (
                "lone modifier keeps multi-word name",
                Shortcut::lone(ModifierKey::AltRight),
                vec!["Right Option"],
            ),
            ("fn on its own", Shortcut::lone(ModifierKey::Fn), vec!["Fn"]),
            (
                "page down",
                combo("PageDown", &[Meta]),
                vec!["⌘", "Page Down"],
            ),
        ];
        for (name, shortcut, want) in cases {
            assert_eq!(shortcut.keycaps(), want, "{name}");
        }
    }

    #[test]
    fn key_display_names_hot_6() {
        let cases = [
            ("KeyA", "A"),
            ("KeyZ", "Z"),
            ("Digit0", "0"),
            ("Digit9", "9"),
            ("Enter", "Return"),
            ("Tab", "Tab"),
            ("Space", "Space"),
            ("Backspace", "Delete"),
            ("Escape", "Escape"),
            ("CapsLock", "Caps Lock"),
            ("F1", "F1"),
            ("F12", "F12"),
            ("Home", "Home"),
            ("End", "End"),
            ("PageUp", "Page Up"),
            ("PageDown", "Page Down"),
            ("ArrowLeft", "←"),
            ("ArrowRight", "→"),
            ("ArrowDown", "↓"),
            ("ArrowUp", "↑"),
            ("Equal", "="),
            ("Minus", "-"),
            ("BracketLeft", "["),
            ("BracketRight", "]"),
            ("Quote", "'"),
            ("Semicolon", ";"),
            ("Backslash", "\\"),
            ("Comma", ","),
            ("Slash", "/"),
            ("Period", "."),
            ("Backquote", "`"),
            ("MetaLeft", "Command"),
            ("MetaRight", "Right Command"),
            ("ShiftLeft", "Shift"),
            ("ShiftRight", "Right Shift"),
            ("AltLeft", "Option"),
            ("AltRight", "Right Option"),
            ("ControlLeft", "Control"),
            ("ControlRight", "Right Control"),
            ("Fn", "Fn"),
            ("F13", "Key F13"),
            ("F0", "Key F0"),
            ("F01", "Key F01"),
            ("KeyAB", "Key KeyAB"),
            ("Delete", "Key Delete"),
            ("IntlYen", "Key IntlYen"),
        ];
        for (code, want) in cases {
            assert_eq!(key(code).display_name(), want, "{code}");
        }
    }

    #[test]
    fn key_code_parse_rejects_non_w3c_names() {
        let cases = [
            ("space", "Space", Some("Space")),
            ("digits", "Digit1", Some("Digit1")),
            ("empty", "", None),
            ("whitespace", "Key A", None),
            ("punctuation", "Key-A", None),
            ("unicode", "Kéy", None),
        ];
        for (name, input, want) in cases {
            let got = KeyCode::parse(input);
            assert_eq!(got.as_ref().map(KeyCode::as_str), want, "{name}");
        }
    }

    #[test]
    fn settings_round_trip_hot_1() {
        let cases = [
            (
                "default",
                Shortcut::default(),
                r#"{"key":"Space","modifiers":["alt"]}"#,
            ),
            (
                "lone modifier",
                Shortcut::lone(ModifierKey::AltRight),
                r#"{"key":"AltRight","modifiers":[]}"#,
            ),
            (
                "canonical modifier order",
                combo("KeyD", &[Meta, Control, Shift, Alt]),
                r#"{"key":"KeyD","modifiers":["control","alt","shift","meta"]}"#,
            ),
        ];
        for (name, shortcut, want) in cases {
            let got = serde_json::to_string(&shortcut).expect("serialize");
            assert_eq!(got, want, "{name}");
            let back: Shortcut = serde_json::from_str(&got).expect("deserialize");
            assert_eq!(back, shortcut, "{name}");
        }
    }

    #[test]
    fn settings_read_is_lenient_hot_5_set_2() {
        let cases = [
            (
                "valid combo",
                r#"{"key":"KeyD","modifiers":["control","alt"]}"#,
                combo("KeyD", &[Control, Alt]),
            ),
            (
                "modifiers in any order",
                r#"{"key":"KeyD","modifiers":["alt","control"]}"#,
                combo("KeyD", &[Control, Alt]),
            ),
            (
                "duplicate modifiers",
                r#"{"key":"KeyD","modifiers":["alt","alt"]}"#,
                combo("KeyD", &[Alt]),
            ),
            (
                "extra fields ignored",
                r#"{"key":"Fn","modifiers":[],"by":"Troy Barnes"}"#,
                Shortcut::lone(ModifierKey::Fn),
            ),
            (
                "fails validation",
                r#"{"key":"KeyA","modifiers":["shift"]}"#,
                Shortcut::default(),
            ),
            (
                "escape",
                r#"{"key":"Escape","modifiers":["alt"]}"#,
                Shortcut::default(),
            ),
            (
                "unknown modifier",
                r#"{"key":"KeyA","modifiers":["hyper","alt"]}"#,
                Shortcut::default(),
            ),
            (
                "fn as a modifier",
                r#"{"key":"KeyA","modifiers":["fn","alt"]}"#,
                Shortcut::default(),
            ),
            (
                "empty key",
                r#"{"key":"","modifiers":["alt"]}"#,
                Shortcut::default(),
            ),
            (
                "junk key",
                r#"{"key":"Greendale Community College","modifiers":["alt"]}"#,
                Shortcut::default(),
            ),
            (
                "missing modifiers",
                r#"{"key":"AltRight"}"#,
                Shortcut::default(),
            ),
            (
                "missing key",
                r#"{"modifiers":["alt"]}"#,
                Shortcut::default(),
            ),
            (
                "jabber integer key code",
                r#"{"key":49,"modifiers":["alt"]}"#,
                Shortcut::default(),
            ),
            ("huge number", "5000000000", Shortcut::default()),
            ("string", r#""alt+Space""#, Shortcut::default()),
            ("null", "null", Shortcut::default()),
            ("array", r#"["Space","alt"]"#, Shortcut::default()),
        ];
        for (name, input, want) in cases {
            let got: Shortcut =
                serde_json::from_str(input).expect("lenient read never fails on valid JSON");
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn strict_settings_parse_reports_why_set_2() {
        let cases = [
            (
                "valid",
                r#"{"key":"Space","modifiers":["alt"]}"#,
                Ok(Shortcut::default()),
            ),
            (
                "malformed",
                r#"{"key":1}"#,
                Err(ShortcutSettingError::Malformed),
            ),
            (
                "invalid",
                r#"{"key":"KeyA","modifiers":[]}"#,
                Err(ShortcutSettingError::Invalid(
                    ValidationError::MissingRequiredModifier,
                )),
            ),
        ];
        for (name, input, want) in cases {
            let value: serde_json::Value = serde_json::from_str(input).expect("json");
            assert_eq!(Shortcut::from_setting(&value), want, "{name}");
        }
    }

    #[test]
    fn held_modifiers_track_families_and_fn() {
        struct Case {
            name: &'static str,
            held: HeldModifiers,
            key: ModifierKey,
            want_contains: bool,
            want_only: bool,
        }
        let cases = [
            Case {
                name: "nothing held",
                held: HeldModifiers::NONE,
                key: ModifierKey::AltRight,
                want_contains: false,
                want_only: false,
            },
            Case {
                name: "option alone covers both sides",
                held: HeldModifiers::new(mods(&[Alt]), false),
                key: ModifierKey::AltLeft,
                want_contains: true,
                want_only: true,
            },
            Case {
                name: "fn alone",
                held: HeldModifiers::new(ModifierSet::EMPTY, true),
                key: ModifierKey::Fn,
                want_contains: true,
                want_only: true,
            },
            Case {
                name: "fn with option",
                held: HeldModifiers::new(mods(&[Alt]), true),
                key: ModifierKey::Fn,
                want_contains: true,
                want_only: false,
            },
        ];
        for tc in cases {
            assert_eq!(tc.held.contains(tc.key), tc.want_contains, "{}", tc.name);
            assert_eq!(
                tc.held == HeldModifiers::only(tc.key),
                tc.want_only,
                "{}",
                tc.name
            );
        }
    }
}
