//! `onda-app` events in UniFFI shapes. The mapping is mechanical: every
//! field the core computed crosses as is, and core constants the shell must
//! show (tooltips, labels) ride along so no copy lives in the shell.

use std::sync::Arc;
use std::time::SystemTime;

use onda_app::views::{
    AboutPage, GeneralPage, GettingStartedPage, HistoryPage, HotkeyPage, OnboardingView,
    PostProcessingPage, ReplacementsPage, SpeechPage, UpdateNotice,
};
use onda_app::{Event, EventSink, MainWindowSection};
use onda_domain::hotkey::HotkeyView;
use onda_domain::non_dictation_ui::{
    DownloadOverlayView, HIDE_ACCESSIBILITY_LABEL, OverlayProgress, hide_tooltip,
};
use onda_domain::overlay::{
    CANCEL_ACCESSIBILITY_LABEL, CANCEL_TOOLTIP, OverlayContent, OverlayIcon, OverlayView,
};
use onda_domain::status_menu::{self, DictationMenuAction, StatusMenu};
use onda_domain::tray;
use onda_domain::updates::UpdateControl;

/// Receives core events in emission order (ARCH-5), on a core thread.
#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    /// Handles one event.
    fn on_event(&self, event: CoreEvent);
}

/// A core → shell event.
// lowered into a foreign value at once, so boxing would buy nothing
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum CoreEvent {
    /// The tray icon changed.
    TrayChanged {
        /// The new tray view model.
        tray: TrayState,
    },
    /// The status menu changed.
    MenuChanged {
        /// The new menu.
        menu: MenuState,
    },
    /// The recording overlay changed.
    OverlayChanged {
        /// The new overlay.
        overlay: OverlayState,
    },
    /// The download overlay changed.
    DownloadOverlayChanged {
        /// The new download overlay.
        overlay: DownloadOverlayState,
    },
    /// History changed; re-query.
    HistoryChanged,
    /// The shortcut, its recorder, or the Fn warning changed.
    HotkeyChanged {
        /// The new hotkey view model.
        hotkey: HotkeyState,
    },
    /// Show the onboarding window; its content arrives in
    /// `OnboardingChanged` (ONB-13, WIN-4, DICT-5).
    OpenOnboarding,
    /// Close the onboarding window (ONB-10).
    CloseOnboarding,
    /// The onboarding view model.
    OnboardingChanged {
        /// The new view.
        view: OnboardingView,
    },
    /// Open the main window on `page`, selecting it even when the window is
    /// already open (UI-4, UI-11, DICT-5).
    OpenMainWindow {
        /// Page to select.
        page: MainWindowSection,
    },
    /// Show the model update notice (DL-14).
    ShowUpdateNotice {
        /// Its content.
        notice: UpdateNotice,
    },
    /// Close the model update notice.
    CloseUpdateNotice,
    /// Behave as a regular app (dock icon, app switcher) while a window is
    /// open, else tray-only (UI-2).
    AppPresenceChanged {
        /// A window is open.
        regular: bool,
    },
    /// Bring Onda to the front (CLN-25).
    ActivateApp,
    /// Show a blocking modal alert with one button (critical
    /// messages).
    ShowAlert {
        /// The title.
        title: String,
        /// The body.
        message: String,
        /// The only button's label.
        button: String,
    },
    /// The Getting Started page.
    GettingStartedChanged {
        /// The new page.
        page: GettingStartedPage,
    },
    /// The General page.
    GeneralChanged {
        /// The new page.
        page: GeneralPage,
    },
    /// The Hotkey page beyond the recorder.
    HotkeyPageChanged {
        /// The new page.
        page: HotkeyPage,
    },
    /// The Speech page.
    SpeechChanged {
        /// The new page.
        page: SpeechPage,
    },
    /// The Post-Processing page.
    PostProcessingChanged {
        /// The new page.
        page: PostProcessingPage,
    },
    /// The Replacements page.
    ReplacementsChanged {
        /// The new page.
        page: ReplacementsPage,
    },
    /// The History page.
    HistoryPageChanged {
        /// The new page.
        page: HistoryPage,
    },
    /// The About page.
    AboutChanged {
        /// The new page.
        page: AboutPage,
    },
}

/// The hotkey page's view model (HOT-6, HOT-7, HOT-11, HOT-35).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HotkeyState {
    /// Display string, e.g. `⌥ Space`.
    pub display: String,
    /// One keycap per modifier glyph plus one for the key.
    pub keycaps: Vec<String>,
    /// The shortcut needs Accessibility (lone modifiers).
    pub needs_accessibility: bool,
    /// A recording is in progress: show Cancel next to the record button.
    pub recording: bool,
    /// Keep exactly one local key monitor installed while true and hand
    /// every key event to `recorder_flags_changed` / `recorder_key_down`
    /// (HOT-9, HOT-10).
    pub listening: bool,
    /// The record button's title.
    pub record_button_title: String,
    /// The cancel button's title, shown while recording.
    pub cancel_button_title: String,
    /// The help text under the recorder.
    pub help_text: String,
    /// The validation or start-failure message, if any.
    pub message: Option<String>,
    /// The Fn warning, if any.
    pub fn_warning: Option<String>,
    /// The title of the button next to the Fn warning.
    pub open_keyboard_settings_title: String,
}

impl From<HotkeyView> for HotkeyState {
    fn from(view: HotkeyView) -> Self {
        Self {
            display: view.display,
            keycaps: view.keycaps,
            needs_accessibility: view.needs_accessibility,
            recording: view.recording,
            listening: view.listening,
            record_button_title: view.record_button_title.to_owned(),
            cancel_button_title: view.cancel_button_title.to_owned(),
            help_text: view.help_text,
            message: view.message.map(str::to_owned),
            fn_warning: view.fn_warning.map(str::to_owned),
            open_keyboard_settings_title: view.open_keyboard_settings_title.to_owned(),
        }
    }
}

/// Tray icon view model.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrayState {
    /// Icon to show.
    pub icon: TrayIcon,
    /// Tint the icon red.
    pub red_tint: bool,
    /// What screen readers announce.
    pub accessibility_label: String,
}

/// Platform-neutral tray icon.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum TrayIcon {
    /// Ready to dictate.
    Ready,
    /// Recording.
    Recording,
    /// Transcribing.
    Transcribing,
    /// Preparing a model, progress unknown.
    Preparing,
    /// Preparing a model with known progress (UI-5).
    ProgressRing {
        /// Fraction in `[0, 1]`.
        progress: f64,
    },
    /// Model unavailable, setup unfinished, or a permission missing.
    Warning,
}

/// A plain menu item.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuEntry {
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
}

/// What the dictation item does (UI-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DictationAction {
    /// Start a session.
    Start,
    /// Stop and transcribe.
    Stop,
    /// Cancel the transcription.
    Cancel,
}

/// The start/stop/cancel item.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DictationEntry {
    /// Title.
    pub title: String,
    /// What choosing it does.
    pub action: DictationAction,
}

/// "Paste Last Transcript" / "Copy Last Transcript" (DICT-26).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LastTranscriptEntry {
    /// Title.
    pub title: String,
    /// One-line preview.
    pub subtitle: Option<String>,
    /// Clickable.
    pub enabled: bool,
}

/// A "Recent Transcripts" entry (HIS-28).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecentTranscriptEntry {
    /// Preview title.
    pub title: String,
    /// When it was saved. The shell writes the subtitle as the localized
    /// abbreviated date and short time, then " · " and `app` when present.
    pub time: SystemTime,
    /// The app, `None` when unknown (the subtitle is then just the date).
    pub app: Option<String>,
    /// What choosing it delivers.
    pub text: String,
}

/// The "Recent Transcripts" submenu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecentTranscriptsSubmenu {
    /// Parent title.
    pub title: String,
    /// Entries, newest first.
    pub items: Vec<RecentTranscriptEntry>,
    /// Entries are clickable.
    pub items_enabled: bool,
    /// Trailing "show all" item title.
    pub show_all_title: String,
}

/// A recent language (LANG-9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LanguageEntry {
    /// Language code.
    pub code: String,
    /// Display name.
    pub name: String,
    /// Current language.
    pub checked: bool,
}

/// The "Language" submenu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LanguageSubmenu {
    /// Parent title.
    pub title: String,
    /// Parent subtitle: the current language.
    pub subtitle: String,
    /// Recent languages.
    pub items: Vec<LanguageEntry>,
    /// Language items are clickable.
    pub items_enabled: bool,
    /// Trailing item title.
    pub other_title: String,
}

/// The status menu, top to bottom. Separators sit before
/// `language`, `open_app`, `update`, and `quit`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuState {
    /// Disabled header (UI-7).
    pub header: String,
    /// "Fix Permissions…" (UI-8).
    pub fix_permissions: Option<MenuEntry>,
    /// Start / Stop / Cancel Dictation (UI-9).
    pub dictation: DictationEntry,
    /// Paste or copy the last transcript.
    pub last_transcript: LastTranscriptEntry,
    /// Recent transcripts.
    pub recent_transcripts: Option<RecentTranscriptsSubmenu>,
    /// "Retry Failed Dictation".
    pub retry_failed_dictation: Option<MenuEntry>,
    /// "Discard Failed Recording".
    pub discard_failed_recording: Option<MenuEntry>,
    /// "Retry Loading Model".
    pub retry_model_load: Option<MenuEntry>,
    /// "Re-download Model".
    pub redownload_model: Option<MenuEntry>,
    /// Language submenu.
    pub language: LanguageSubmenu,
    /// "Open Onda".
    pub open_app: MenuEntry,
    /// "Settings…".
    pub settings: MenuEntry,
    /// Check for updates, restart to update, or download.
    pub update: UpdateControl,
    /// "Quit Onda".
    pub quit: MenuEntry,
}

/// The overlay clock (DICT-9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OverlayClock {
    /// "m:ss" or "Stops in m:ss".
    pub text: String,
    /// Render in the warning color.
    pub warning: bool,
}

/// Clock and cancel button (OVL-7).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OverlayTrailing {
    /// Clock, while recording.
    pub clock: Option<OverlayClock>,
    /// Cancel button tooltip.
    pub cancel_tooltip: String,
    /// Cancel button accessibility label.
    pub cancel_accessibility_label: String,
}

/// The overlay's single content mode (OVL-5).
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OverlayContentState {
    /// A notice (OVL-12).
    Notice {
        /// Title.
        title: String,
        /// Message.
        message: String,
    },
    /// Processing (OVL-10).
    Processing {
        /// "Transcribing..." or "Refining...".
        label: String,
        /// Last preview text.
        preview: Option<String>,
    },
    /// Silent microphone (AUD-13).
    SilentMicrophone {
        /// Title.
        title: String,
        /// Hint.
        message: String,
    },
    /// Preview above a short waveform (OVL-9).
    Preview {
        /// Preview text.
        text: String,
        /// Levels, oldest first.
        levels: Vec<f32>,
    },
    /// The level meter (OVL-8).
    Waveform {
        /// Levels, oldest first.
        levels: Vec<f32>,
    },
}

/// The recording overlay's leading icon (OVL-6, OUT-12).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OverlayIconState {
    /// No icon.
    None,
    /// The target app's icon.
    App {
        /// Bundle identifier.
        bundle_id: String,
    },
    /// The frontmost app's icon, unless that is Onda itself.
    FrontmostApp,
}

/// The recording overlay view model.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OverlayState {
    /// On screen.
    pub visible: bool,
    /// Leading icon (OVL-6).
    pub icon: OverlayIconState,
    /// Content mode.
    pub content: OverlayContentState,
    /// Clock and cancel button.
    pub trailing: Option<OverlayTrailing>,
}

/// The download overlay view model (OVL-16, OVL-17).
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum DownloadOverlayState {
    /// Off screen.
    Hidden,
    /// On screen.
    Shown {
        /// Status line.
        status: String,
        /// Fraction in `[0, 1]`, or none for an indeterminate bar.
        progress: Option<f64>,
        /// Close button tooltip.
        hide_tooltip: String,
        /// Close button accessibility label.
        hide_accessibility_label: String,
    },
}

impl From<tray::TrayView> for TrayState {
    fn from(view: tray::TrayView) -> Self {
        let icon = match view.icon {
            tray::TrayIcon::Ready => TrayIcon::Ready,
            tray::TrayIcon::Recording => TrayIcon::Recording,
            tray::TrayIcon::Transcribing => TrayIcon::Transcribing,
            tray::TrayIcon::Preparing => TrayIcon::Preparing,
            tray::TrayIcon::ProgressRing { progress } => TrayIcon::ProgressRing { progress },
            tray::TrayIcon::Warning => TrayIcon::Warning,
        };
        Self {
            icon,
            red_tint: view.red_tint,
            accessibility_label: view.accessibility_label,
        }
    }
}

fn entry(item: status_menu::MenuItem) -> MenuEntry {
    MenuEntry {
        title: item.title.to_owned(),
        enabled: item.enabled,
    }
}

impl From<StatusMenu> for MenuState {
    fn from(menu: StatusMenu) -> Self {
        Self {
            header: menu.header,
            fix_permissions: menu.fix_permissions.map(entry),
            dictation: DictationEntry {
                title: menu.dictation.title.to_owned(),
                action: match menu.dictation.action {
                    DictationMenuAction::Start => DictationAction::Start,
                    DictationMenuAction::Stop => DictationAction::Stop,
                    DictationMenuAction::Cancel => DictationAction::Cancel,
                },
            },
            last_transcript: LastTranscriptEntry {
                title: menu.last_transcript.title.to_owned(),
                subtitle: menu.last_transcript.subtitle,
                enabled: menu.last_transcript.enabled,
            },
            recent_transcripts: menu
                .recent_transcripts
                .map(|recent| RecentTranscriptsSubmenu {
                    title: recent.title.to_owned(),
                    items: recent
                        .items
                        .into_iter()
                        .map(|item| RecentTranscriptEntry {
                            title: item.title,
                            time: SystemTime::from(item.time),
                            app: item.app_name,
                            text: item.text,
                        })
                        .collect(),
                    items_enabled: recent.items_enabled,
                    show_all_title: recent.show_all_title.to_owned(),
                }),
            retry_failed_dictation: menu.retry_failed_dictation.map(entry),
            discard_failed_recording: menu.discard_failed_recording.map(entry),
            retry_model_load: menu.retry_model_load.map(entry),
            redownload_model: menu.redownload_model.map(entry),
            language: LanguageSubmenu {
                title: menu.language.title.to_owned(),
                subtitle: menu.language.subtitle,
                items: menu
                    .language
                    .items
                    .into_iter()
                    .map(|item| LanguageEntry {
                        code: item.code,
                        name: item.name,
                        checked: item.checked,
                    })
                    .collect(),
                items_enabled: menu.language.items_enabled,
                other_title: menu.language.other_title.to_owned(),
            },
            open_app: entry(menu.open_app),
            settings: entry(menu.settings),
            update: menu.update,
            quit: entry(menu.quit),
        }
    }
}

impl From<OverlayView> for OverlayState {
    fn from(view: OverlayView) -> Self {
        let content = match view.content {
            OverlayContent::Notice(notice) => OverlayContentState::Notice {
                title: notice.title,
                message: notice.message,
            },
            OverlayContent::Processing { label, preview } => OverlayContentState::Processing {
                label: label.to_owned(),
                preview,
            },
            OverlayContent::SilentMicrophone { title, message } => {
                OverlayContentState::SilentMicrophone {
                    title,
                    message: message.to_owned(),
                }
            }
            OverlayContent::Preview { text, levels } => {
                OverlayContentState::Preview { text, levels }
            }
            OverlayContent::Waveform { levels } => OverlayContentState::Waveform { levels },
        };
        Self {
            visible: view.visible,
            icon: match view.icon {
                OverlayIcon::None => OverlayIconState::None,
                OverlayIcon::App(bundle_id) => OverlayIconState::App { bundle_id },
                OverlayIcon::FrontmostApp => OverlayIconState::FrontmostApp,
            },
            content,
            trailing: view.trailing.map(|trailing| OverlayTrailing {
                clock: trailing.clock.map(|clock| OverlayClock {
                    text: clock.text,
                    warning: clock.warning,
                }),
                cancel_tooltip: CANCEL_TOOLTIP.to_owned(),
                cancel_accessibility_label: CANCEL_ACCESSIBILITY_LABEL.to_owned(),
            }),
        }
    }
}

fn download_overlay(view: DownloadOverlayView, tray_term: &str) -> DownloadOverlayState {
    match view {
        DownloadOverlayView::Hidden => DownloadOverlayState::Hidden,
        DownloadOverlayView::Shown { status, progress } => DownloadOverlayState::Shown {
            status,
            progress: match progress {
                OverlayProgress::Determinate(fraction) => Some(fraction),
                OverlayProgress::Indeterminate => None,
            },
            hide_tooltip: hide_tooltip(tray_term),
            hide_accessibility_label: HIDE_ACCESSIBILITY_LABEL.to_owned(),
        },
    }
}

fn core_event(event: Event, tray_term: &str) -> CoreEvent {
    match event {
        Event::TrayChanged(view) => CoreEvent::TrayChanged { tray: view.into() },
        Event::MenuChanged(menu) => CoreEvent::MenuChanged {
            menu: (*menu).into(),
        },
        Event::OverlayChanged(view) => CoreEvent::OverlayChanged {
            overlay: view.into(),
        },
        Event::DownloadOverlayChanged(view) => CoreEvent::DownloadOverlayChanged {
            overlay: download_overlay(view, tray_term),
        },
        Event::HistoryChanged => CoreEvent::HistoryChanged,
        Event::HotkeyChanged(view) => CoreEvent::HotkeyChanged {
            hotkey: view.into(),
        },
        Event::OpenOnboarding => CoreEvent::OpenOnboarding,
        Event::CloseOnboarding => CoreEvent::CloseOnboarding,
        Event::OnboardingChanged(view) => CoreEvent::OnboardingChanged { view: *view },
        Event::OpenMainWindow(page) => CoreEvent::OpenMainWindow { page },
        Event::ShowUpdateNotice(notice) => CoreEvent::ShowUpdateNotice { notice: *notice },
        Event::CloseUpdateNotice => CoreEvent::CloseUpdateNotice,
        Event::AppPresenceChanged { regular } => CoreEvent::AppPresenceChanged { regular },
        Event::ActivateApp => CoreEvent::ActivateApp,
        Event::ShowAlert {
            title,
            message,
            button,
        } => CoreEvent::ShowAlert {
            title,
            message,
            button: button.to_owned(),
        },
        Event::GettingStartedChanged(page) => CoreEvent::GettingStartedChanged { page: *page },
        Event::GeneralChanged(page) => CoreEvent::GeneralChanged { page: *page },
        Event::HotkeyPageChanged(page) => CoreEvent::HotkeyPageChanged { page: *page },
        Event::SpeechChanged(page) => CoreEvent::SpeechChanged { page: *page },
        Event::PostProcessingChanged(page) => CoreEvent::PostProcessingChanged { page: *page },
        Event::ReplacementsChanged(page) => CoreEvent::ReplacementsChanged { page: *page },
        Event::HistoryPageChanged(page) => CoreEvent::HistoryPageChanged { page: *page },
        Event::AboutChanged(page) => CoreEvent::AboutChanged { page: *page },
    }
}

/// Forwards core events to the shell's listener.
pub(crate) struct ListenerSink {
    pub(crate) listener: Arc<dyn CoreListener>,
    pub(crate) tray_term: &'static str,
}

impl EventSink for ListenerSink {
    fn emit(&self, event: Event) {
        self.listener.on_event(core_event(event, self.tray_term));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Critical messages reach the shell as a modal alert with "OK"; the
    /// start failure's copy comes from the core too
    /// (MSG-3).
    #[test]
    fn critical_alert_copy_crosses_the_ffi_msg_3() {
        let event = Event::ShowAlert {
            title: "Greendale Is Closed".to_owned(),
            message: "The study room flooded.".to_owned(),
            button: "OK",
        };
        assert_eq!(
            core_event(event, "menu bar icon"),
            CoreEvent::ShowAlert {
                title: "Greendale Is Closed".to_owned(),
                message: "The study room flooded.".to_owned(),
                button: "OK".to_owned(),
            }
        );
        assert_eq!(
            crate::start_failure_alert(),
            crate::StartFailureAlert {
                title: "Onda Could Not Start".to_owned(),
                button: "Quit".to_owned(),
            }
        );
    }
}
