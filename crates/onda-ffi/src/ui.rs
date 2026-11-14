//! Pure core helpers the shell calls directly, so rules it needs while
//! rendering or typing (fade bookkeeping, hotkey routing, Unicode chunking,
//! waveform layout) stay in `onda-domain`.

use std::sync::{Mutex, PoisonError};

use onda_domain::hotkey::{
    CANCEL_HOTKEY_ID, DICTATION_HOTKEY_ID, HotkeyRoute, KeyNames, Modifier, ModifierSet, Shortcut,
};
use onda_domain::level::{self, BAR_SPACING, BAR_WIDTH};
use onda_domain::non_dictation_ui::DOWNLOAD_OVERLAY_FADE;
use onda_domain::overlay::{self, FadeCommand, OVERLAY_FADE};
use onda_domain::typing;

/// A raw dictation-hotkey event (HOT-20a).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HotkeyInput {
    /// The combo shortcut went down.
    ComboDown,
    /// The combo shortcut went up.
    ComboUp,
    /// Escape fired while a session runs (HOT-34).
    Cancel,
    /// A modifier-flags change from the lone-modifier event stream, read
    /// from the event's own flags (HOT-16).
    ModifierFlags {
        /// W3C code of the key whose flag changed.
        key: String,
        /// The changed key's family flag is set after the event.
        family_flag_set: bool,
        /// The changed key's per-side device bit is set after the event.
        side_flag_set: bool,
    },
    /// A key-down from the lone-modifier event stream; `Unidentified` for a
    /// key without a W3C code (HOT-14a).
    KeyDown {
        /// W3C code of the key.
        key: String,
    },
    /// The OS disabled the event stream (HOT-17).
    StreamDisabled {
        /// The OS status to report if the core gives up.
        status: i32,
    },
}

/// A modifier set in the platform-neutral form (HOT-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Record)]
pub struct HotkeyModifiers {
    /// Control.
    pub control: bool,
    /// Option/Alt.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Command/Meta.
    pub meta: bool,
}

impl From<ModifierSet> for HotkeyModifiers {
    fn from(set: ModifierSet) -> Self {
        Self {
            control: set.contains(Modifier::Control),
            alt: set.contains(Modifier::Alt),
            shift: set.contains(Modifier::Shift),
            meta: set.contains(Modifier::Meta),
        }
    }
}

impl From<HotkeyModifiers> for ModifierSet {
    fn from(modifiers: HotkeyModifiers) -> Self {
        [
            (modifiers.control, Modifier::Control),
            (modifiers.alt, Modifier::Alt),
            (modifiers.shift, Modifier::Shift),
            (modifiers.meta, Modifier::Meta),
        ]
        .into_iter()
        .filter_map(|(held, modifier)| held.then_some(modifier))
        .collect()
    }
}

/// The dictation shortcut in its platform-neutral form (HOT-1). The shell
/// maps the W3C key code to its own key codes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HotkeyShortcut {
    /// W3C `KeyboardEvent.code`, e.g. `Space`.
    pub key: String,
    /// Control is held.
    pub control: bool,
    /// Option/Alt is held.
    pub alt: bool,
    /// Shift is held.
    pub shift: bool,
    /// Command/Meta is held.
    pub meta: bool,
    /// A lone-modifier shortcut, observed through an event stream instead
    /// of the global-hotkey API (HOT-14).
    pub lone_modifier: bool,
    /// Display string (HOT-6), e.g. `⌥ Space`.
    pub display: String,
}

impl From<&Shortcut> for HotkeyShortcut {
    fn from(shortcut: &Shortcut) -> Self {
        let modifiers = shortcut.modifiers();
        Self {
            key: shortcut.key().as_str().to_owned(),
            control: modifiers.contains(Modifier::Control),
            alt: modifiers.contains(Modifier::Alt),
            shift: modifiers.contains(Modifier::Shift),
            meta: modifiers.contains(Modifier::Meta),
            lone_modifier: shortcut.is_lone_modifier(),
            display: shortcut.display(&KeyNames::MACOS),
        }
    }
}

/// Id to register the dictation shortcut under.
#[uniffi::export]
pub fn dictation_hotkey_id() -> u32 {
    DICTATION_HOTKEY_ID
}

/// Id to register Escape under while a session runs (HOT-34).
#[uniffi::export]
pub fn cancel_hotkey_id() -> u32 {
    CANCEL_HOTKEY_ID
}

/// What a global-hotkey press or release of registered hotkey `id` means;
/// none when it means nothing (Escape's release, unknown ids).
#[uniffi::export]
pub fn route_global_hotkey(id: u32, pressed: bool) -> Option<HotkeyInput> {
    match HotkeyRoute::route(id, pressed) {
        HotkeyRoute::DictationDown => Some(HotkeyInput::ComboDown),
        HotkeyRoute::DictationUp => Some(HotkeyInput::ComboUp),
        HotkeyRoute::Cancel => Some(HotkeyInput::Cancel),
        HotkeyRoute::Ignore => None,
    }
}

/// Splits `text` into UTF-16 chunks for synthetic key events, never
/// splitting a surrogate pair (OUT-3).
#[uniffi::export]
pub fn unicode_chunks(text: String) -> Vec<Vec<u16>> {
    typing::unicode_chunks(&text)
}

/// The overlay waveform laid out in a box (OVL-8).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WaveformBars {
    /// Bar width in points.
    pub bar_width: f64,
    /// Gap between bars in points.
    pub spacing: f64,
    /// Bar heights in points, left to right, newest on the right; each
    /// bar is centered on the box's middle line.
    pub heights: Vec<f64>,
}

/// Lays out `levels` (oldest first) as waveform bars in a `width` ×
/// `height` point box: the most recent levels that fit, left-padded with
/// silence (OVL-8).
#[uniffi::export]
pub fn waveform_bars(levels: Vec<f32>, width: f64, height: f64) -> WaveformBars {
    WaveformBars {
        bar_width: BAR_WIDTH,
        spacing: BAR_SPACING,
        heights: level::bar_heights(&levels, width, height),
    }
}

/// What a panel should animate (OVL-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PanelFadeCommand {
    /// Nothing.
    None,
    /// Reposition, then fade in; from the current opacity when interrupting
    /// a fade-out.
    FadeIn {
        /// Start from the current opacity.
        from_current: bool,
    },
    /// Fade out, then call [`PanelFade::fade_out_finished`].
    FadeOut,
    /// Take the panel off screen.
    OrderOut,
}

impl From<FadeCommand> for PanelFadeCommand {
    fn from(command: FadeCommand) -> Self {
        match command {
            FadeCommand::None => Self::None,
            FadeCommand::FadeIn { from_current } => Self::FadeIn { from_current },
            FadeCommand::FadeOut => Self::FadeOut,
            FadeCommand::OrderOut => Self::OrderOut,
        }
    }
}

/// Fade bookkeeping for one overlay panel (OVL-4, OVL-16).
#[derive(uniffi::Object)]
pub struct PanelFade {
    fade: Mutex<overlay::PanelFade>,
    duration_seconds: f64,
}

#[uniffi::export]
impl PanelFade {
    /// The recording overlay's fade (0.2 s).
    #[uniffi::constructor]
    pub fn recording_overlay() -> Self {
        Self::with_duration(OVERLAY_FADE.as_secs_f64())
    }

    /// The download overlay's fade (0.3 s).
    #[uniffi::constructor]
    pub fn download_overlay() -> Self {
        Self::with_duration(DOWNLOAD_OVERLAY_FADE.as_secs_f64())
    }

    /// Fade duration in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// Show the panel.
    pub fn show(&self) -> PanelFadeCommand {
        self.apply(overlay::PanelFade::show)
    }

    /// Hide the panel.
    pub fn hide(&self) -> PanelFadeCommand {
        self.apply(overlay::PanelFade::hide)
    }

    /// The fade-out animation finished.
    pub fn fade_out_finished(&self) -> PanelFadeCommand {
        self.apply(overlay::PanelFade::fade_out_finished)
    }
}

impl PanelFade {
    fn with_duration(duration_seconds: f64) -> Self {
        Self {
            fade: Mutex::new(overlay::PanelFade::new()),
            duration_seconds,
        }
    }

    fn apply(&self, step: impl FnOnce(&mut overlay::PanelFade) -> FadeCommand) -> PanelFadeCommand {
        let mut fade = self.fade.lock().unwrap_or_else(PoisonError::into_inner);
        step(&mut fade).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_carbon_hotkeys() {
        let cases = [
            (DICTATION_HOTKEY_ID, true, Some(HotkeyInput::ComboDown)),
            (DICTATION_HOTKEY_ID, false, Some(HotkeyInput::ComboUp)),
            (CANCEL_HOTKEY_ID, true, Some(HotkeyInput::Cancel)),
            (CANCEL_HOTKEY_ID, false, None),
            (99, true, None),
        ];
        for (id, pressed, want) in cases {
            assert_eq!(route_global_hotkey(id, pressed), want, "{id} {pressed}");
        }
    }

    #[test]
    fn default_shortcut_crosses_as_option_space() {
        let shortcut = HotkeyShortcut::from(&Shortcut::OPTION_SPACE);
        assert_eq!(
            shortcut,
            HotkeyShortcut {
                key: "Space".to_owned(),
                control: false,
                alt: true,
                shift: false,
                meta: false,
                lone_modifier: false,
                display: "⌥ Space".to_owned(),
            }
        );
    }

    #[test]
    fn waveform_bars_carry_the_domain_layout() {
        let bars = waveform_bars(vec![0.1], 9.0, 24.0);
        assert_eq!(bars.bar_width, 3.0);
        assert_eq!(bars.spacing, 3.0);
        assert_eq!(bars.heights, vec![3.0, 24.0]);
    }

    #[test]
    fn fade_interrupting_a_hide_fades_up_from_current() {
        let fade = PanelFade::recording_overlay();
        assert_eq!(
            fade.show(),
            PanelFadeCommand::FadeIn {
                from_current: false
            }
        );
        assert_eq!(fade.hide(), PanelFadeCommand::FadeOut);
        assert_eq!(fade.hide(), PanelFadeCommand::None);
        assert_eq!(fade.show(), PanelFadeCommand::FadeIn { from_current: true });
        assert_eq!(fade.fade_out_finished(), PanelFadeCommand::None);
    }
}
