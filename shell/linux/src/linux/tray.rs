//! Optional StatusNotifierItem that renders the core's status menu. The
//! launcher remains usable without a tray.

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use onda_domain::languages::LanguageSetting;
use onda_domain::status_menu::{DictationMenuAction, StatusMenu};
use onda_domain::tray::{TrayIcon, TrayView};
use onda_domain::updates::UpdateAction;

use super::{Message, send};

/// What a tray choice asks for. `ui::Window::tray` maps each to its core call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A click on the icon itself.
    Show,
    MenuWillOpen,
    FixPermissions,
    ToggleDictation,
    CancelDictation,
    DeliverLastTranscript,
    DeliverTranscript(String),
    ShowAllHistory,
    RetryFailedDictation,
    DiscardFailedDictation,
    RetryModelLoad,
    RedownloadModel,
    SelectLanguage(LanguageSetting),
    ChooseOtherLanguage,
    OpenApp,
    OpenSettings,
    Update(UpdateAction),
    Quit,
}

pub struct Tray {
    events: async_channel::Sender<Message>,
    pub view: TrayView,
    /// `None` until the core sends its first menu.
    pub menu: Option<StatusMenu>,
}

fn icon_name(icon: TrayIcon) -> &'static str {
    match icon {
        TrayIcon::Ready => "audio-input-microphone-symbolic",
        TrayIcon::Recording => "media-record-symbolic",
        TrayIcon::Transcribing => "content-loading-symbolic",
        // a themed icon cannot draw the ring; the label carries the percentage
        TrayIcon::Preparing | TrayIcon::ProgressRing { .. } => "folder-download-symbolic",
        TrayIcon::Warning => "dialog-warning-symbolic",
    }
}

/// DBusMenu reads a single underscore as an access-key marker and hides it.
fn label(text: &str) -> String {
    text.replace('_', "__")
}

fn item(title: &str, enabled: bool, command: Command) -> ksni::MenuItem<Tray> {
    StandardItem {
        label: label(title),
        enabled,
        activate: Box::new(move |tray: &mut Tray| {
            send(&tray.events, Message::Tray(command.clone()))
        }),
        ..Default::default()
    }
    .into()
}

fn submenu(title: &str, submenu: Vec<ksni::MenuItem<Tray>>) -> ksni::MenuItem<Tray> {
    SubMenu {
        label: label(title),
        submenu,
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        super::APP_ID.into()
    }
    fn title(&self) -> String {
        self.view.accessibility_label.clone()
    }
    fn icon_name(&self) -> String {
        icon_name(self.view.icon).into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.view.accessibility_label.clone(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _: i32, _: i32) {
        send(&self.events, Message::Tray(Command::Show));
    }
    fn menu_about_to_show(&mut self) {
        send(&self.events, Message::Tray(Command::MenuWillOpen));
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::MenuItem::Separator;
        let header = |text: &str| {
            StandardItem {
                label: label(text),
                enabled: false,
                ..Default::default()
            }
            .into()
        };
        // the core has sent no menu yet, so the tray's own label stands in
        let Some(menu) = &self.menu else {
            return vec![
                header(&self.view.accessibility_label),
                item("Quit Onda", true, Command::Quit),
            ];
        };
        let mut items = vec![header(&menu.header)];
        if let Some(fix) = &menu.fix_permissions {
            items.push(item(fix.title, fix.enabled, Command::FixPermissions));
        }
        items.push(item(menu.dictation.title, true, Command::ToggleDictation));
        if menu.dictation.action == DictationMenuAction::Stop {
            // Escape is claimed on X11 only, so the tray always offers the
            // cancel
            items.push(item("Cancel Dictation", true, Command::CancelDictation));
        }
        items.push(item(
            menu.last_transcript.title,
            menu.last_transcript.enabled,
            Command::DeliverLastTranscript,
        ));
        if let Some(recent) = &menu.recent_transcripts {
            let mut entries: Vec<_> = recent
                .items
                .iter()
                .map(|entry| {
                    item(
                        &entry.title,
                        recent.items_enabled,
                        Command::DeliverTranscript(entry.text.clone()),
                    )
                })
                .collect();
            entries.push(Separator);
            entries.push(item(recent.show_all_title, true, Command::ShowAllHistory));
            items.push(submenu(recent.title, entries));
        }
        for (entry, command) in [
            (&menu.retry_failed_dictation, Command::RetryFailedDictation),
            (
                &menu.discard_failed_recording,
                Command::DiscardFailedDictation,
            ),
            (&menu.retry_model_load, Command::RetryModelLoad),
            (&menu.redownload_model, Command::RedownloadModel),
        ] {
            if let Some(entry) = entry {
                items.push(item(entry.title, entry.enabled, command));
            }
        }
        items.push(Separator);
        let language = &menu.language;
        let mut languages: Vec<_> = language
            .items
            .iter()
            .filter_map(|entry| {
                let command = Command::SelectLanguage(LanguageSetting::parse(&entry.code)?);
                Some(
                    CheckmarkItem {
                        label: label(&entry.name),
                        enabled: language.items_enabled,
                        checked: entry.checked,
                        activate: Box::new(move |tray: &mut Tray| {
                            send(&tray.events, Message::Tray(command.clone()))
                        }),
                        ..Default::default()
                    }
                    .into(),
                )
            })
            .collect();
        languages.push(Separator);
        languages.push(item(
            language.other_title,
            true,
            Command::ChooseOtherLanguage,
        ));
        // DBusMenu has no subtitles, so the parent names the current language
        items.push(submenu(
            &format!("{}: {}", language.title, language.subtitle),
            languages,
        ));
        items.push(Separator);
        items.push(item(
            menu.open_app.title,
            menu.open_app.enabled,
            Command::OpenApp,
        ));
        items.push(item(
            menu.settings.title,
            menu.settings.enabled,
            Command::OpenSettings,
        ));
        items.push(Separator);
        items.push(item(
            &menu.update.title,
            menu.update.enabled,
            Command::Update(menu.update.action),
        ));
        items.push(Separator);
        items.push(item(menu.quit.title, menu.quit.enabled, Command::Quit));
        items
    }
}

pub fn start(events: async_channel::Sender<Message>) -> Option<ksni::blocking::Handle<Tray>> {
    match (Tray {
        events,
        view: TrayView {
            icon: TrayIcon::Preparing,
            red_tint: false,
            accessibility_label: "Onda".into(),
        },
        menu: None,
    })
    .spawn()
    {
        Ok(handle) => Some(handle),
        Err(error) => {
            tracing::debug!(%error, "tray unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    use onda_domain::privacy_naming::PrivacyNaming;
    use onda_domain::setup::MissingPermission;
    use onda_domain::status_menu::{LanguageMenuItem, RecentTranscriptItem, StatusMenuInput};
    use onda_domain::tray::{AppStatus, tray_view};
    use onda_domain::types::{ModelState, OutputMode, SessionPhase};
    use onda_domain::updates::{UpdateStatus, update_control};

    use super::*;

    fn input(model_state: &ModelState) -> StatusMenuInput<'_> {
        StatusMenuInput {
            status: AppStatus::Ready,
            hotkey_display: "Ctrl+Alt+Space",
            first_run_pending: false,
            missing_permission: None,
            privacy_naming: PrivacyNaming::Accessibility,
            phase: SessionPhase::Idle,
            output_mode: OutputMode::Type,
            last_transcript_preview: Some("Troy Barnes joined the study group.".into()),
            history_enabled: true,
            recent_transcripts: [
                "Troy Barnes joined the study group.",
                "save it as troy_barnes",
            ]
            .into_iter()
            .map(|text| RecentTranscriptItem {
                title: text.into(),
                time: SystemTime::UNIX_EPOCH.try_into().unwrap(),
                app_name: Some("Greendale Messenger".into()),
                text: text.into(),
            })
            .collect(),
            has_failed_dictation: false,
            can_start_session: true,
            model_state,
            load_in_flight: false,
            selected_model_on_disk: false,
            current_language_name: "English".into(),
            languages: [("auto", "Automatic", false), ("en", "English", true)]
                .into_iter()
                .map(|(code, name, checked)| LanguageMenuItem {
                    code: code.into(),
                    name: name.into(),
                    checked,
                })
                .collect(),
            update: update_control(&UpdateStatus::Idle, false),
        }
    }

    /// Activates every item of the tray's menu and writes one line per row:
    /// the label, in brackets when disabled, then the command it sent.
    fn rows(tray: &mut Tray, sent: &async_channel::Receiver<Message>) -> Vec<String> {
        fn walk(
            items: Vec<ksni::MenuItem<Tray>>,
            indent: &str,
            tray: &mut Tray,
            sent: &async_channel::Receiver<Message>,
            rows: &mut Vec<String>,
        ) {
            for item in items {
                let (label, enabled) = match item {
                    ksni::MenuItem::Separator => {
                        rows.push(format!("{indent}---"));
                        continue;
                    }
                    ksni::MenuItem::SubMenu(menu) => {
                        rows.push(format!("{indent}{}", menu.label));
                        walk(menu.submenu, &format!("{indent}  "), tray, sent, rows);
                        continue;
                    }
                    ksni::MenuItem::Standard(item) => {
                        (item.activate)(tray);
                        (item.label, item.enabled)
                    }
                    ksni::MenuItem::Checkmark(item) => {
                        (item.activate)(tray);
                        let mark = if item.checked { "✓" } else { "·" };
                        (format!("{mark} {}", item.label), item.enabled)
                    }
                    ksni::MenuItem::RadioGroup(_) => panic!("the tray builds no radio groups"),
                };
                let label = if enabled { label } else { format!("[{label}]") };
                rows.push(match sent.try_recv() {
                    // a language's `Debug` output is its index in the catalog
                    Ok(Message::Tray(Command::SelectLanguage(language))) => {
                        format!("{indent}{label} => SelectLanguage({})", language.code())
                    }
                    Ok(Message::Tray(command)) => format!("{indent}{label} => {command:?}"),
                    Ok(_) => panic!("{label} sent something other than a tray command"),
                    Err(_) => format!("{indent}{label}"),
                });
            }
        }
        let mut rows = Vec::new();
        walk(ksni::Tray::menu(tray), "", tray, sent, &mut rows);
        rows
    }

    #[test]
    fn every_item_sends_its_command() {
        struct Case {
            status: AppStatus,
            adjust: fn(&mut StatusMenuInput),
            want: &'static [&'static str],
        }
        let failed = ModelState::Error {
            message: "Failed to load the transcription model".into(),
        };
        let cases = BTreeMap::from([
            (
                "idle, with every optional item",
                Case {
                    status: AppStatus::Ready,
                    adjust: |input| {
                        input.missing_permission = Some(MissingPermission::Microphone);
                        input.has_failed_dictation = true;
                        input.selected_model_on_disk = true;
                        input.update = update_control(
                            &UpdateStatus::Ready {
                                version: "3.3.0".into(),
                            },
                            false,
                        );
                    },
                    want: &[
                        "[Microphone Access Needed]",
                        "Fix Permissions… => FixPermissions",
                        "Start Dictation => ToggleDictation",
                        "Paste Last Transcript => DeliverLastTranscript",
                        "Recent Transcripts",
                        "  Troy Barnes joined the study group. => \
                         DeliverTranscript(\"Troy Barnes joined the study group.\")",
                        "  save it as troy__barnes => DeliverTranscript(\"save it as troy_barnes\")",
                        "  ---",
                        "  Show All History… => ShowAllHistory",
                        "Retry Failed Dictation => RetryFailedDictation",
                        "Discard Failed Recording => DiscardFailedDictation",
                        "Retry Loading Model => RetryModelLoad",
                        "Re-download Model => RedownloadModel",
                        "---",
                        "Language: English",
                        "  · Automatic => SelectLanguage(auto)",
                        "  ✓ English => SelectLanguage(en)",
                        "  ---",
                        "  Other Language… => ChooseOtherLanguage",
                        "---",
                        "Open Onda => OpenApp",
                        "Settings… => OpenSettings",
                        "---",
                        "Restart to Update => Update(Restart)",
                        "---",
                        "Quit Onda => Quit",
                    ],
                },
            ),
            (
                "recording",
                Case {
                    status: AppStatus::Recording,
                    adjust: |input| {
                        input.phase = SessionPhase::Recording { provisional: false };
                        input.output_mode = OutputMode::Clipboard;
                        input.has_failed_dictation = true;
                        input.can_start_session = false;
                    },
                    want: &[
                        "[Recording…]",
                        "Stop Dictation => ToggleDictation",
                        "Cancel Dictation => CancelDictation",
                        "[Copy Last Transcript] => DeliverLastTranscript",
                        "Recent Transcripts",
                        "  [Troy Barnes joined the study group.] => \
                         DeliverTranscript(\"Troy Barnes joined the study group.\")",
                        "  [save it as troy__barnes] => \
                         DeliverTranscript(\"save it as troy_barnes\")",
                        "  ---",
                        "  Show All History… => ShowAllHistory",
                        "[Retry Failed Dictation] => RetryFailedDictation",
                        "[Discard Failed Recording] => DiscardFailedDictation",
                        "---",
                        "Language: English",
                        "  [· Automatic] => SelectLanguage(auto)",
                        "  [✓ English] => SelectLanguage(en)",
                        "  ---",
                        "  Other Language… => ChooseOtherLanguage",
                        "---",
                        "Open Onda => OpenApp",
                        "Settings… => OpenSettings",
                        "---",
                        "Check for Updates… => Update(Check)",
                        "---",
                        "Quit Onda => Quit",
                    ],
                },
            ),
            (
                "transcribing, with nothing optional",
                Case {
                    status: AppStatus::Transcribing,
                    adjust: |input| {
                        input.phase = SessionPhase::Transcribing;
                        input.last_transcript_preview = None;
                        input.recent_transcripts.clear();
                        input.languages.clear();
                    },
                    want: &[
                        "[Transcribing…]",
                        "Cancel Dictation => ToggleDictation",
                        "[Paste Last Transcript] => DeliverLastTranscript",
                        "---",
                        "Language: English",
                        "  ---",
                        "  Other Language… => ChooseOtherLanguage",
                        "---",
                        "Open Onda => OpenApp",
                        "Settings… => OpenSettings",
                        "---",
                        "Check for Updates… => Update(Check)",
                        "---",
                        "Quit Onda => Quit",
                    ],
                },
            ),
        ]);
        for (name, tc) in cases {
            let ready = ModelState::Ready;
            let mut input = input(if tc.status == AppStatus::Ready {
                &failed
            } else {
                &ready
            });
            input.status = tc.status;
            (tc.adjust)(&mut input);
            let (events, sent) = async_channel::unbounded();
            let mut tray = Tray {
                events,
                view: tray_view(
                    input.status,
                    input.missing_permission,
                    input.first_run_pending,
                    input.privacy_naming,
                ),
                menu: Some(StatusMenu::resolve(input)),
            };
            assert_eq!(rows(&mut tray, &sent), tc.want, "{name}");
        }
    }

    #[test]
    fn the_first_row_is_the_cores_header() {
        let (events, sent) = async_channel::unbounded();
        let ready = ModelState::Ready;
        let input = input(&ready);
        let mut tray = Tray {
            events,
            view: tray_view(
                input.status,
                input.missing_permission,
                input.first_run_pending,
                input.privacy_naming,
            ),
            menu: Some(StatusMenu::resolve(input)),
        };
        assert_eq!(
            rows(&mut tray, &sent).first().map(String::as_str),
            Some("[Ready to Dictate — Ctrl+Alt+Space]")
        );
    }

    #[test]
    fn offers_only_quit_before_the_first_menu() {
        let (events, sent) = async_channel::unbounded();
        let mut tray = Tray {
            events,
            view: tray_view(
                AppStatus::Preparing { progress: None },
                None,
                false,
                PrivacyNaming::Accessibility,
            ),
            menu: None,
        };
        assert_eq!(
            rows(&mut tray, &sent),
            ["[Onda, preparing speech model]", "Quit Onda => Quit"]
        );
    }

    #[test]
    fn icon_click_and_menu_opening_reach_the_shell() {
        let (events, sent) = async_channel::unbounded();
        let mut tray = Tray {
            events,
            view: tray_view(AppStatus::Ready, None, false, PrivacyNaming::Accessibility),
            menu: None,
        };
        ksni::Tray::activate(&mut tray, 0, 0);
        ksni::Tray::menu_about_to_show(&mut tray);
        let mut got = Vec::new();
        while let Ok(Message::Tray(command)) = sent.try_recv() {
            got.push(command);
        }
        assert_eq!(got, [Command::Show, Command::MenuWillOpen]);
    }

    #[test]
    fn icon_and_tooltip_follow_the_app_status() {
        let cases = BTreeMap::from([
            (
                "ready",
                (
                    AppStatus::Ready,
                    None,
                    "audio-input-microphone-symbolic",
                    "Onda, ready to dictate",
                ),
            ),
            (
                "recording",
                (
                    AppStatus::Recording,
                    None,
                    "media-record-symbolic",
                    "Onda, recording",
                ),
            ),
            (
                "transcribing",
                (
                    AppStatus::Transcribing,
                    None,
                    "content-loading-symbolic",
                    "Onda, transcribing",
                ),
            ),
            (
                "loading a model",
                (
                    AppStatus::Preparing { progress: None },
                    None,
                    "folder-download-symbolic",
                    "Onda, preparing speech model",
                ),
            ),
            (
                "downloading a model",
                (
                    AppStatus::Preparing {
                        progress: Some(0.426),
                    },
                    None,
                    "folder-download-symbolic",
                    "Onda, preparing speech model, 42 percent",
                ),
            ),
            (
                "no model",
                (
                    AppStatus::Unavailable,
                    None,
                    "dialog-warning-symbolic",
                    "Onda, model unavailable",
                ),
            ),
            (
                "microphone access missing",
                (
                    AppStatus::Ready,
                    Some(MissingPermission::Microphone),
                    "dialog-warning-symbolic",
                    "Onda, microphone access needed",
                ),
            ),
        ]);
        for (name, (status, missing, want_icon, want_label)) in cases {
            let (events, _sent) = async_channel::unbounded();
            let tray = Tray {
                events,
                view: tray_view(status, missing, false, PrivacyNaming::Accessibility),
                menu: None,
            };
            assert_eq!(ksni::Tray::icon_name(&tray), want_icon, "{name}");
            assert_eq!(ksni::Tray::tool_tip(&tray).title, want_label, "{name}");
            assert_eq!(ksni::Tray::title(&tray), want_label, "{name}");
        }
    }
}
