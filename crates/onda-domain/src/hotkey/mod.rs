//! Dictation hotkey rules (HOT-1 to HOT-35): the
//! shortcut model, presets, the recorder, the lone-modifier gesture,
//! registration policies, activation-mode press routing, and the Fn warning.
//!
//! Everything here is pure (ARCH-2). The shell turns OS events into the
//! platform-neutral inputs these reducers take and performs the effects they
//! return.

mod gesture;
mod globe;
mod key_names;
mod preset;
mod press;
mod recorder;
mod registration;
mod shortcut;
mod view;

pub use gesture::{
    DebounceToken, GestureAction, GestureInput, GesturePhase, LONE_MODIFIER_DEBOUNCE,
    LoneModifierGesture, ModifierFlagsChange, gesture_input_for_flags_change,
    gesture_input_for_key_down,
};
pub use globe::{GlobeKeyAction, OPEN_KEYBOARD_SETTINGS_TITLE, fn_key_warning};
pub use key_names::{KeyNames, ModifierNames};
pub use preset::HotkeyPreset;
pub use press::{
    AUTOMATIC_HOLD_THRESHOLD, HOLD_TAP_THRESHOLD, PressIntent, PressRouter, SessionSnapshot,
    provisional_confirmation_delay,
};
pub use recorder::{
    RECORD_BUTTON_TITLE, RECORDER_CANCEL_TITLE, RECORDING_BUTTON_TITLE, RecorderEvent,
    RecorderMessage, RecorderOutcome, ShortcutRecorder,
};
pub use registration::{
    ACCESSIBILITY_POLL_INTERVAL, CANCEL_HOTKEY_ID, DICTATION_HOTKEY_ID, FailureMailbox,
    HotkeyRoute, MAX_REENABLES, REENABLE_WINDOW, REGISTRATION_FAILURE_TITLE,
    RegistrationFailureNotice, RegistrationRetry, StreamDisableDecision, StreamReenablePolicy,
    cancel_hotkey_modifiers,
};
pub use shortcut::{
    HeldModifiers, KeyCode, Modifier, ModifierKey, ModifierSet, Shortcut, ShortcutSettingError,
    ValidationError,
};
pub use view::HotkeyView;
