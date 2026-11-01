//! The status menu view model (07 "Status menu", UI-6 to UI-11, DICT-21,
//! DICT-25 to DICT-27, HIS-28, LANG-9, LANG-10).

use jiff::Timestamp;

use crate::setup::MissingPermission;
use crate::tray::{AppStatus, percent};
use crate::types::{ModelState, OutputMode, SessionPhase};

/// A saved transcript offered under "Recent Transcripts" (HIS-28). Built by
/// the history code: newest first, at most 5, previews already cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentTranscriptItem {
    /// One-line preview used as the title.
    pub title: String,
    /// When it was saved: the subtitle's date and time, which the shell
    /// formats in the user's locale.
    pub time: Timestamp,
    /// The app after the date ("<date and time> · <app>"), `None` when
    /// unknown.
    pub app_name: Option<String>,
    /// What choosing the item delivers.
    pub text: String,
}

/// One recent language in the Language submenu (LANG-9), already sorted by
/// display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageMenuItem {
    /// Language code passed to `select_language_from_menu`.
    pub code: String,
    /// Display name.
    pub name: String,
    /// The current language gets a check mark.
    pub checked: bool,
}

/// What the status menu is computed from.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusMenuInput<'a> {
    /// App status shared with the tray icon.
    pub status: AppStatus,
    /// Hotkey display string, e.g. "⌥ Space".
    pub hotkey_display: &'a str,
    /// First-run onboarding is pending (SET-4).
    pub first_run_pending: bool,
    /// Permission to warn about (UI-8).
    pub missing_permission: Option<MissingPermission>,
    /// Dictation session phase.
    pub phase: SessionPhase,
    /// Current output mode.
    pub output_mode: OutputMode,
    /// One-line preview of the last transcript, if there is one (DICT-26).
    pub last_transcript_preview: Option<String>,
    /// History is on (HIS-28).
    pub history_enabled: bool,
    /// Recent transcripts from history (HIS-28).
    pub recent_transcripts: Vec<RecentTranscriptItem>,
    /// Failed-dictation audio is kept (DICT-22).
    pub has_failed_dictation: bool,
    /// A new session could start now (DICT-25).
    pub can_start_session: bool,
    /// Transcription engine state.
    pub model_state: &'a ModelState,
    /// A model load is in flight.
    pub load_in_flight: bool,
    /// The selected model is downloadable and its files are on disk.
    pub selected_model_on_disk: bool,
    /// Current language display name ("Automatic" for `auto`).
    pub current_language_name: String,
    /// Recent languages (LANG-8, LANG-9).
    pub languages: Vec<LanguageMenuItem>,
    /// The updater can check now (UPD-1).
    pub can_check_for_updates: bool,
}

/// A plain menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItem {
    /// Title.
    pub title: &'static str,
    /// Clickable.
    pub enabled: bool,
}

impl MenuItem {
    fn enabled(title: &'static str) -> Self {
        Self {
            title,
            enabled: true,
        }
    }
}

/// What the dictation item does when chosen (UI-9, DICT-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationMenuAction {
    /// Start a non-provisional session with no tail.
    Start,
    /// Stop and transcribe.
    Stop,
    /// Cancel the transcription.
    Cancel,
}

/// The start/stop/cancel item (UI-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DictationItem {
    /// Title.
    pub title: &'static str,
    /// What choosing it does.
    pub action: DictationMenuAction,
}

/// "Paste Last Transcript" or "Copy Last Transcript" (DICT-26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastTranscriptItem {
    /// Title for the current output mode.
    pub title: &'static str,
    /// One-line preview.
    pub subtitle: Option<String>,
    /// Clickable.
    pub enabled: bool,
}

/// The "Recent Transcripts" submenu (HIS-28, DICT-27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentTranscriptsMenu {
    /// Parent item title.
    pub title: &'static str,
    /// Entries, newest first.
    pub items: Vec<RecentTranscriptItem>,
    /// Entries are clickable.
    pub items_enabled: bool,
    /// Trailing item, after a separator; opens History.
    pub show_all_title: &'static str,
}

/// The "Language" submenu (LANG-9, LANG-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageMenu {
    /// Parent item title.
    pub title: &'static str,
    /// Parent item subtitle: the current language.
    pub subtitle: String,
    /// Recent languages.
    pub items: Vec<LanguageMenuItem>,
    /// Language items are clickable (idle only).
    pub items_enabled: bool,
    /// Trailing item, after a separator; opens Speech settings.
    pub other_title: &'static str,
}

/// The status menu, top to bottom (07 "Status menu"). Optional items are
/// `None` when hidden. Separators sit before the language menu, before
/// `open_app`, before `check_for_updates`, and before `quit`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusMenu {
    /// Disabled header (UI-7).
    pub header: String,
    /// Opens the main window on General (UI-8).
    pub fix_permissions: Option<MenuItem>,
    /// Start, stop, or cancel (UI-9).
    pub dictation: DictationItem,
    /// Paste or copy the last transcript (DICT-26).
    pub last_transcript: LastTranscriptItem,
    /// Recent transcripts (HIS-28).
    pub recent_transcripts: Option<RecentTranscriptsMenu>,
    /// Retry the kept failed dictation (DICT-25).
    pub retry_failed_dictation: Option<MenuItem>,
    /// Drop the kept failed dictation (DICT-25).
    pub discard_failed_recording: Option<MenuItem>,
    /// Restart the failed model load (ASR-10).
    pub retry_model_load: Option<MenuItem>,
    /// Replace the failed model's files.
    pub redownload_model: Option<MenuItem>,
    /// Language submenu.
    pub language: LanguageMenu,
    /// Opens the main window on Getting Started (UI-11).
    pub open_app: MenuItem,
    /// Opens the main window on General (UI-11); shortcut ⌘, on macOS.
    pub settings: MenuItem,
    /// Follows the updater's can-check flag (UI-10).
    pub check_for_updates: MenuItem,
    /// Quits; shortcut ⌘Q on macOS.
    pub quit: MenuItem,
}

impl StatusMenu {
    /// Builds the menu.
    pub fn resolve(input: StatusMenuInput<'_>) -> Self {
        let idle = input.phase == SessionPhase::Idle;
        let model_load_failed =
            matches!(input.model_state, ModelState::Error { .. }) && !input.load_in_flight;
        let failed_dictation = input
            .has_failed_dictation
            .then_some(input.can_start_session);
        let recent_transcripts = (input.history_enabled && !input.recent_transcripts.is_empty())
            .then_some(RecentTranscriptsMenu {
                title: "Recent Transcripts",
                items: input.recent_transcripts,
                items_enabled: idle,
                show_all_title: "Show All History…",
            });
        Self {
            header: header(
                input.status,
                input.hotkey_display,
                input.first_run_pending,
                input.missing_permission,
            ),
            fix_permissions: input
                .missing_permission
                .map(|_| MenuItem::enabled("Fix Permissions…")),
            dictation: dictation_item(input.phase),
            last_transcript: LastTranscriptItem {
                title: match input.output_mode {
                    OutputMode::Type => "Paste Last Transcript",
                    OutputMode::Clipboard => "Copy Last Transcript",
                },
                enabled: idle && input.last_transcript_preview.is_some(),
                subtitle: input.last_transcript_preview,
            },
            recent_transcripts,
            retry_failed_dictation: failed_dictation.map(|enabled| MenuItem {
                title: "Retry Failed Dictation",
                enabled,
            }),
            discard_failed_recording: failed_dictation.map(|enabled| MenuItem {
                title: "Discard Failed Recording",
                enabled,
            }),
            retry_model_load: model_load_failed.then(|| MenuItem::enabled("Retry Loading Model")),
            redownload_model: (model_load_failed && input.selected_model_on_disk)
                .then(|| MenuItem::enabled("Re-download Model")),
            language: LanguageMenu {
                title: "Language",
                subtitle: input.current_language_name,
                items: input.languages,
                items_enabled: idle,
                other_title: "Other Language…",
            },
            open_app: MenuItem::enabled("Open Onda"),
            settings: MenuItem::enabled("Settings…"),
            check_for_updates: MenuItem {
                title: "Check for Updates…",
                enabled: input.can_check_for_updates,
            },
            quit: MenuItem::enabled("Quit Onda"),
        }
    }
}

fn header(
    status: AppStatus,
    hotkey_display: &str,
    first_run_pending: bool,
    missing_permission: Option<MissingPermission>,
) -> String {
    match status {
        AppStatus::Preparing { progress: None } => "Preparing Speech Model…".to_owned(),
        AppStatus::Preparing {
            progress: Some(progress),
        } => format!("Preparing Speech Model… {}%", percent(progress)),
        AppStatus::Ready => match missing_permission {
            Some(MissingPermission::Microphone) => "Microphone Access Needed".to_owned(),
            Some(MissingPermission::Accessibility) => "Accessibility Access Needed".to_owned(),
            None => format!("Ready to Dictate — {hotkey_display}"),
        },
        AppStatus::Recording => "Recording…".to_owned(),
        AppStatus::Transcribing => "Transcribing…".to_owned(),
        // before setup finishes no model was ever chosen, so "unavailable"
        // would read as a failure
        AppStatus::Unavailable if first_run_pending => "Finish Setup to Dictate".to_owned(),
        AppStatus::Unavailable => "Model Unavailable".to_owned(),
    }
}

fn dictation_item(phase: SessionPhase) -> DictationItem {
    let (title, action) = match phase {
        SessionPhase::Idle => ("Start Dictation", DictationMenuAction::Start),
        SessionPhase::Recording { .. } => ("Stop Dictation", DictationMenuAction::Stop),
        SessionPhase::Transcribing => ("Cancel Dictation", DictationMenuAction::Cancel),
    };
    DictationItem { title, action }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: ModelState = ModelState::Ready;

    fn error_state() -> ModelState {
        ModelState::Error {
            message: "Failed to load the transcription model".to_owned(),
        }
    }

    fn input(model_state: &ModelState) -> StatusMenuInput<'_> {
        StatusMenuInput {
            status: AppStatus::Ready,
            hotkey_display: "⌥ Space",
            first_run_pending: false,
            missing_permission: None,
            phase: SessionPhase::Idle,
            output_mode: OutputMode::Type,
            last_transcript_preview: None,
            history_enabled: true,
            recent_transcripts: vec![],
            has_failed_dictation: false,
            can_start_session: true,
            model_state,
            load_in_flight: false,
            selected_model_on_disk: false,
            current_language_name: "English".to_owned(),
            languages: vec![],
            can_check_for_updates: true,
        }
    }

    fn recent(title: &str) -> RecentTranscriptItem {
        RecentTranscriptItem {
            title: title.to_owned(),
            time: Timestamp::UNIX_EPOCH,
            app_name: Some("Greendale Messenger".to_owned()),
            text: title.to_owned(),
        }
    }

    #[test]
    fn ui_7_header_names_the_current_state() {
        struct Case {
            name: &'static str,
            status: AppStatus,
            pending: bool,
            missing: Option<MissingPermission>,
            want: &'static str,
            want_fix: bool,
        }
        let cases = [
            Case {
                name: "preparing a model",
                status: AppStatus::Preparing { progress: None },
                pending: false,
                missing: None,
                want: "Preparing Speech Model…",
                want_fix: false,
            },
            Case {
                name: "downloading with known progress",
                status: AppStatus::Preparing {
                    progress: Some(0.426),
                },
                pending: false,
                missing: None,
                want: "Preparing Speech Model… 42%",
                want_fix: false,
            },
            Case {
                name: "ready",
                status: AppStatus::Ready,
                pending: false,
                missing: None,
                want: "Ready to Dictate — ⌥ Space",
                want_fix: false,
            },
            Case {
                name: "recording",
                status: AppStatus::Recording,
                pending: false,
                missing: None,
                want: "Recording…",
                want_fix: false,
            },
            Case {
                name: "transcribing",
                status: AppStatus::Transcribing,
                pending: false,
                missing: None,
                want: "Transcribing…",
                want_fix: false,
            },
            Case {
                name: "no model after setup",
                status: AppStatus::Unavailable,
                pending: false,
                missing: None,
                want: "Model Unavailable",
                want_fix: false,
            },
            Case {
                name: "SET-4 no model before setup",
                status: AppStatus::Unavailable,
                pending: true,
                missing: None,
                want: "Finish Setup to Dictate",
                want_fix: false,
            },
            Case {
                name: "UI-8 microphone missing",
                status: AppStatus::Ready,
                pending: false,
                missing: Some(MissingPermission::Microphone),
                want: "Microphone Access Needed",
                want_fix: true,
            },
            Case {
                name: "UI-8 accessibility missing",
                status: AppStatus::Ready,
                pending: false,
                missing: Some(MissingPermission::Accessibility),
                want: "Accessibility Access Needed",
                want_fix: true,
            },
            Case {
                name: "recording keeps its header",
                status: AppStatus::Recording,
                pending: false,
                missing: Some(MissingPermission::Accessibility),
                want: "Recording…",
                want_fix: true,
            },
            Case {
                name: "abandoned setup still warns",
                status: AppStatus::Ready,
                pending: true,
                missing: Some(MissingPermission::Microphone),
                want: "Microphone Access Needed",
                want_fix: true,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                status: tc.status,
                first_run_pending: tc.pending,
                missing_permission: tc.missing,
                ..input(&READY)
            });
            assert_eq!(got.header, tc.want, "{}", tc.name);
            assert_eq!(got.fix_permissions.is_some(), tc.want_fix, "{}", tc.name);
            if let Some(item) = got.fix_permissions {
                assert_eq!(item.title, "Fix Permissions…", "{}", tc.name);
            }
        }
    }

    #[test]
    fn ui_9_dict_21_dictation_item_follows_the_session() {
        struct Case {
            name: &'static str,
            phase: SessionPhase,
            want_title: &'static str,
            want_action: DictationMenuAction,
        }
        let cases = [
            Case {
                name: "idle",
                phase: SessionPhase::Idle,
                want_title: "Start Dictation",
                want_action: DictationMenuAction::Start,
            },
            Case {
                name: "recording",
                phase: SessionPhase::Recording { provisional: false },
                want_title: "Stop Dictation",
                want_action: DictationMenuAction::Stop,
            },
            Case {
                name: "provisional recording",
                phase: SessionPhase::Recording { provisional: true },
                want_title: "Stop Dictation",
                want_action: DictationMenuAction::Stop,
            },
            Case {
                name: "transcribing",
                phase: SessionPhase::Transcribing,
                want_title: "Cancel Dictation",
                want_action: DictationMenuAction::Cancel,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                phase: tc.phase,
                ..input(&READY)
            })
            .dictation;
            assert_eq!(got.title, tc.want_title, "{}", tc.name);
            assert_eq!(got.action, tc.want_action, "{}", tc.name);
        }
    }

    #[test]
    fn dict_26_last_transcript_item() {
        struct Case {
            name: &'static str,
            phase: SessionPhase,
            preview: Option<&'static str>,
            mode: OutputMode,
            want_title: &'static str,
            want_enabled: bool,
        }
        let cases = [
            Case {
                name: "nothing dictated yet",
                phase: SessionPhase::Idle,
                preview: None,
                mode: OutputMode::Type,
                want_title: "Paste Last Transcript",
                want_enabled: false,
            },
            Case {
                name: "typing output",
                phase: SessionPhase::Idle,
                preview: Some("Troy Barnes"),
                mode: OutputMode::Type,
                want_title: "Paste Last Transcript",
                want_enabled: true,
            },
            Case {
                name: "clipboard output",
                phase: SessionPhase::Idle,
                preview: Some("Troy Barnes"),
                mode: OutputMode::Clipboard,
                want_title: "Copy Last Transcript",
                want_enabled: true,
            },
            Case {
                name: "during a recording",
                phase: SessionPhase::Recording { provisional: false },
                preview: Some("Troy Barnes"),
                mode: OutputMode::Type,
                want_title: "Paste Last Transcript",
                want_enabled: false,
            },
            Case {
                name: "during transcription",
                phase: SessionPhase::Transcribing,
                preview: Some("Troy Barnes"),
                mode: OutputMode::Type,
                want_title: "Paste Last Transcript",
                want_enabled: false,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                phase: tc.phase,
                last_transcript_preview: tc.preview.map(str::to_owned),
                output_mode: tc.mode,
                ..input(&READY)
            })
            .last_transcript;
            assert_eq!(got.title, tc.want_title, "{}", tc.name);
            assert_eq!(got.subtitle.as_deref(), tc.preview, "{}", tc.name);
            assert_eq!(got.enabled, tc.want_enabled, "{}", tc.name);
        }
    }

    #[test]
    fn his_28_dict_27_recent_transcripts() {
        struct Case {
            name: &'static str,
            history_enabled: bool,
            items: Vec<RecentTranscriptItem>,
            phase: SessionPhase,
            want_titles: Option<Vec<&'static str>>,
            want_enabled: bool,
        }
        let items = || {
            vec![
                recent("Troy and Abed in the morning"),
                recent("Six seasons and a movie"),
            ]
        };
        let cases = [
            Case {
                name: "entries while idle",
                history_enabled: true,
                items: items(),
                phase: SessionPhase::Idle,
                want_titles: Some(vec![
                    "Troy and Abed in the morning",
                    "Six seasons and a movie",
                ]),
                want_enabled: true,
            },
            Case {
                name: "history off hides",
                history_enabled: false,
                items: items(),
                phase: SessionPhase::Idle,
                want_titles: None,
                want_enabled: false,
            },
            Case {
                name: "nothing to show hides",
                history_enabled: true,
                items: vec![],
                phase: SessionPhase::Idle,
                want_titles: None,
                want_enabled: false,
            },
            Case {
                name: "busy recording disables",
                history_enabled: true,
                items: items(),
                phase: SessionPhase::Recording { provisional: false },
                want_titles: Some(vec![
                    "Troy and Abed in the morning",
                    "Six seasons and a movie",
                ]),
                want_enabled: false,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                history_enabled: tc.history_enabled,
                recent_transcripts: tc.items,
                phase: tc.phase,
                ..input(&READY)
            })
            .recent_transcripts;
            let titles = got
                .as_ref()
                .map(|m| m.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>());
            assert_eq!(titles, tc.want_titles, "{}", tc.name);
            assert_eq!(
                got.as_ref().is_some_and(|m| m.items_enabled),
                tc.want_enabled,
                "{}",
                tc.name
            );
            if let Some(menu) = got {
                assert_eq!(menu.title, "Recent Transcripts", "{}", tc.name);
                assert_eq!(menu.show_all_title, "Show All History…", "{}", tc.name);
            }
        }
    }

    #[test]
    fn dict_25_recovery_items_follow_failures() {
        struct Case {
            name: &'static str,
            failed: bool,
            can_start: bool,
            model_state: ModelState,
            load_in_flight: bool,
            want_failed_items: Option<bool>,
            want_retry_load: bool,
        }
        let cases = [
            Case {
                name: "nothing failed",
                failed: false,
                can_start: true,
                model_state: READY,
                load_in_flight: false,
                want_failed_items: None,
                want_retry_load: false,
            },
            Case {
                name: "failed dictation while idle",
                failed: true,
                can_start: true,
                model_state: READY,
                load_in_flight: false,
                want_failed_items: Some(true),
                want_retry_load: false,
            },
            Case {
                name: "failed dictation while a session runs",
                failed: true,
                can_start: false,
                model_state: READY,
                load_in_flight: false,
                want_failed_items: Some(false),
                want_retry_load: false,
            },
            Case {
                name: "model load failed",
                failed: false,
                can_start: true,
                model_state: error_state(),
                load_in_flight: false,
                want_failed_items: None,
                want_retry_load: true,
            },
            Case {
                name: "both failed",
                failed: true,
                can_start: true,
                model_state: error_state(),
                load_in_flight: false,
                want_failed_items: Some(true),
                want_retry_load: true,
            },
            Case {
                name: "retry hidden while a load is in flight",
                failed: false,
                can_start: true,
                model_state: error_state(),
                load_in_flight: true,
                want_failed_items: None,
                want_retry_load: false,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                has_failed_dictation: tc.failed,
                can_start_session: tc.can_start,
                load_in_flight: tc.load_in_flight,
                ..input(&tc.model_state)
            });
            assert_eq!(
                got.retry_failed_dictation.map(|i| i.enabled),
                tc.want_failed_items,
                "{}",
                tc.name
            );
            assert_eq!(
                got.discard_failed_recording.map(|i| i.enabled),
                tc.want_failed_items,
                "{}",
                tc.name
            );
            assert_eq!(
                got.retry_model_load.is_some(),
                tc.want_retry_load,
                "{}",
                tc.name
            );
            if let Some(item) = got.retry_failed_dictation {
                assert_eq!(item.title, "Retry Failed Dictation", "{}", tc.name);
            }
            if let Some(item) = got.discard_failed_recording {
                assert_eq!(item.title, "Discard Failed Recording", "{}", tc.name);
            }
            if let Some(item) = got.retry_model_load {
                assert_eq!(item.title, "Retry Loading Model", "{}", tc.name);
            }
        }
    }

    #[test]
    fn redownload_offered_only_for_a_downloaded_model_that_failed_to_load() {
        struct Case {
            name: &'static str,
            model_state: ModelState,
            on_disk: bool,
            want: bool,
        }
        let cases = [
            Case {
                name: "load failed on downloaded files",
                model_state: error_state(),
                on_disk: true,
                want: true,
            },
            Case {
                name: "load failed without files",
                model_state: error_state(),
                on_disk: false,
                want: false,
            },
            Case {
                name: "loaded fine",
                model_state: READY,
                on_disk: true,
                want: false,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                selected_model_on_disk: tc.on_disk,
                ..input(&tc.model_state)
            });
            assert_eq!(got.redownload_model.is_some(), tc.want, "{}", tc.name);
            if let Some(item) = got.redownload_model {
                assert_eq!(item.title, "Re-download Model", "{}", tc.name);
            }
        }
    }

    #[test]
    fn lang_9_lang_10_language_menu() {
        struct Case {
            name: &'static str,
            phase: SessionPhase,
            current: &'static str,
            want_enabled: bool,
        }
        let cases = [
            Case {
                name: "idle enables languages",
                phase: SessionPhase::Idle,
                current: "English",
                want_enabled: true,
            },
            Case {
                name: "recording disables languages",
                phase: SessionPhase::Recording { provisional: true },
                current: "English",
                want_enabled: false,
            },
            Case {
                name: "transcribing disables languages",
                phase: SessionPhase::Transcribing,
                current: "Automatic",
                want_enabled: false,
            },
        ];
        let languages = vec![
            LanguageMenuItem {
                code: "en".to_owned(),
                name: "English".to_owned(),
                checked: true,
            },
            LanguageMenuItem {
                code: "es".to_owned(),
                name: "Spanish".to_owned(),
                checked: false,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                phase: tc.phase,
                current_language_name: tc.current.to_owned(),
                languages: languages.clone(),
                ..input(&READY)
            })
            .language;
            assert_eq!(got.title, "Language", "{}", tc.name);
            assert_eq!(got.subtitle, tc.current, "{}", tc.name);
            assert_eq!(got.items, languages, "{}", tc.name);
            assert_eq!(got.items_enabled, tc.want_enabled, "{}", tc.name);
            assert_eq!(got.other_title, "Other Language…", "{}", tc.name);
        }
    }

    #[test]
    fn ui_10_ui_11_static_items_and_updates() {
        struct Case {
            name: &'static str,
            can_check: bool,
            want: bool,
        }
        let cases = [
            Case {
                name: "updater can check",
                can_check: true,
                want: true,
            },
            Case {
                name: "updater busy",
                can_check: false,
                want: false,
            },
        ];
        for tc in cases {
            let got = StatusMenu::resolve(StatusMenuInput {
                can_check_for_updates: tc.can_check,
                ..input(&READY)
            });
            assert_eq!(
                got.check_for_updates,
                MenuItem {
                    title: "Check for Updates…",
                    enabled: tc.want
                },
                "{}",
                tc.name
            );
            assert_eq!(
                got.open_app,
                MenuItem {
                    title: "Open Onda",
                    enabled: true
                },
                "{}",
                tc.name
            );
            assert_eq!(
                got.settings,
                MenuItem {
                    title: "Settings…",
                    enabled: true
                },
                "{}",
                tc.name
            );
            assert_eq!(
                got.quit,
                MenuItem {
                    title: "Quit Onda",
                    enabled: true
                },
                "{}",
                tc.name
            );
        }
    }
}
