//! Models inside the session actor: the launch decision (ASR-1, DL-12..DL-15),
//! downloads end to end (DL-1..DL-4, ASR-2, ASR-9), delete and re-download
//! (DL-9..DL-11), the update notice (DL-14), and the language switches
//! (LANG-8..LANG-10).

use onda_domain::download_progress::{
    DOWNLOAD_ALERT_TITLE, DownloadEnd, DownloadStatus, FailureReport, failure_report, status_text,
};
use onda_domain::language_menu::{
    MenuModelChoice, encode_recent, recent_languages, switch_language_from_menu,
};
use onda_domain::languages::{LanguageSetting, default_language};
use onda_domain::migration::{
    DeclinedLoad, NoticeAction, NoticeChoice, REMOVED_MODEL_IDS, SelectionResolution,
    declined_load, is_declined, resolve_selection, respond_to_notice, should_present_notice,
};
use onda_domain::model_selection::{
    LaunchLoad, LaunchState, can_redownload, delete_confirmation, delete_failed_message,
    delete_model, launch_load, load_failure_notification,
};
use onda_domain::models::ModelId;
use onda_domain::non_dictation_ui::ModelDownload;
use onda_domain::onboarding::{OnboardingDownloadUpdate, launch_plan};
use onda_domain::routing::routes;
use onda_domain::types::ModelState;
use tokio::task::spawn_blocking;

use super::windows::PendingAlertAction;
use super::{Actor, Msg, Phase, UiMsg, blocking_send};
use crate::effects::Effect;
use crate::events::Event;
use crate::seams::{DownloadUpdate, DownloadUpdateKind, SettingChange};
use crate::views::{MainWindowSection, SpeechAlert, update_notice};

/// First loading status, also shown while a load downloads first (ASR-2).
const PREPARING_STATUS: &str = "Preparing model...";

impl Actor {
    /// The model loads use: the declined-migration fallback while it
    /// applies, else the selection (DL-14).
    pub(super) fn desired_model(&self) -> ModelId {
        self.override_model.unwrap_or(self.settings.selected_model)
    }

    /// A load, or a download a load is waiting for, is running.
    pub(super) fn load_in_flight(&self) -> bool {
        self.loads_in_flight > 0 || self.load_download.is_some()
    }

    // --- launch (ASR-1, ONB-12, ONB-13, DL-12..DL-14) ---

    pub(super) fn launch_checked(&mut self, downloaded: Vec<ModelId>) {
        self.downloaded = downloaded;
        self.refresh_settings();
        // HIS-16: entries may have expired while Onda was not running; this
        // also preloads the list for the tray menu
        let history = self.ports.history.clone();
        let retention = self.settings.history_retention;
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            match history.apply_retention(retention, jiff::Timestamp::now()) {
                // HIS-10: announce what expired; the menu re-reads
                Ok(true) => return Msg::Ui(UiMsg::HistoryOpDone(Ok(()))),
                Ok(false) => {}
                Err(error) => {
                    tracing::error!(%error, "could not apply history retention at launch");
                }
            }
            Msg::Recent(history.recent_transcripts())
        }));
        let plan = launch_plan(self.settings.onboarding);
        if let Some(delay) = plan.show_onboarding_after {
            self.ui.launch_onboarding = Some(self.timer(delay, Msg::Ui(UiMsg::AutoOnboarding)));
        }
        if plan.request_notifications {
            self.effects.send(Effect::RequestNotifications);
        }
        // first-run onboarding owns the first download (ASR-1)
        if plan.skip_model_load {
            return;
        }
        let resolution = self.resolution();
        if let SelectionResolution::SwitchSilently(migration) = &resolution {
            // DL-13: the replacement is installed, so switch without asking
            self.apply_setting(SettingChange::SelectedModel(migration.to));
        }
        let settings = self.settings.clone();
        let decision = launch_load(
            &self.catalog,
            &resolution,
            &LaunchState {
                onboarding_pending: settings.first_run_pending,
                last_notice_key: &settings.last_notice_key,
                declined_notice_key: &settings.declined_notice_key,
                selected_language: settings.language,
                downloaded: &self.downloaded,
            },
        );
        match decision {
            LaunchLoad::Nothing => {}
            LaunchLoad::ShowNotice(migration) => {
                self.ui.notice.pending = Some(migration);
                self.ui.notice.timer = Some(self.timer(
                    onda_domain::migration::NOTICE_DELAY,
                    Msg::Ui(UiMsg::ShowNotice),
                ));
            }
            LaunchLoad::LoadFallback(model) => {
                self.override_model = Some(model);
                self.force_loading = true;
                self.start_load(false);
            }
            LaunchLoad::OpenSpeechSettings => self.open_section(MainWindowSection::Speech),
            LaunchLoad::LoadSelected => {
                // the tray shows loading at once (ASR-1)
                self.force_loading = true;
                self.start_load(false);
            }
        }
    }

    /// What the stored selection resolves to (DL-12).
    pub(super) fn resolution(&self) -> SelectionResolution {
        let settings = &self.settings;
        resolve_selection(
            &self.catalog,
            &settings.stored_model,
            REMOVED_MODEL_IDS,
            default_language(settings.os_language.as_deref()),
            &self.downloaded,
        )
    }

    /// The user declined the migration download (DL-14).
    pub(super) fn declined(&self) -> bool {
        !self.settings.declined_notice_key.is_empty()
            && is_declined(&self.resolution(), &self.settings.declined_notice_key)
    }

    /// Ends the declined state: any explicit selection, download start, or
    /// model change (DL-14).
    pub(super) fn end_declined(&mut self) {
        if !self.settings.declined_notice_key.is_empty() {
            self.apply_setting(SettingChange::DeclinedNoticeKey(String::new()));
        }
    }

    // loads the declined fallback without touching the selection, or opens
    // Speech when none is installed (DL-14, ONB-11)
    pub(super) fn load_declined_fallback(&mut self) -> bool {
        let settings = self.settings.clone();
        match declined_load(
            &self.catalog,
            &self.resolution(),
            &settings.declined_notice_key,
            settings.language,
            &self.downloaded,
        ) {
            Some(DeclinedLoad::LoadFallback(model)) => {
                self.override_model = Some(model);
                self.force_loading = true;
                self.start_load(false);
                true
            }
            Some(DeclinedLoad::OpenSpeechSettings) => {
                self.open_section(MainWindowSection::Speech);
                true
            }
            None => false,
        }
    }

    // --- the update notice (DL-14) ---

    pub(super) fn show_pending_notice(&mut self) {
        self.ui.notice.timer = None;
        let Some(migration) = self.ui.notice.pending.take() else {
            return;
        };
        let onboarding_open = self.ui.onboarding.is_some();
        if !should_present_notice(onboarding_open, self.ui.notice.open.is_some()) {
            return;
        }
        let view = update_notice(&self.catalog, &migration);
        self.ui.notice.open = Some(migration);
        self.events.emit(Event::ShowUpdateNotice(Box::new(view)));
    }

    pub(super) fn notice_choice(&mut self, choice: NoticeChoice) {
        let Some(migration) = self.ui.notice.open.take() else {
            return;
        };
        let response = respond_to_notice(&migration, choice);
        self.apply_setting(SettingChange::LastNoticeKey(response.last_shown_key));
        self.apply_setting(SettingChange::DeclinedNoticeKey(
            response.declined_key.unwrap_or_default(),
        ));
        self.events.emit(Event::CloseUpdateNotice);
        match response.action {
            NoticeAction::DownloadReplacement(model) => {
                self.apply_setting(SettingChange::SelectedModel(model));
                self.begin_download(model);
            }
            NoticeAction::OpenSpeechSettings => self.open_section(MainWindowSection::Speech),
            NoticeAction::LoadDeclinedFallback => {
                self.load_declined_fallback();
            }
        }
    }

    // --- downloads (DL-1..DL-4, ASR-2, ASR-9) ---

    /// Starts or joins `model`'s download. Any download start ends a
    /// declined migration (DL-14).
    pub(super) fn begin_download(&mut self, model: ModelId) {
        if self.catalog.is_built_in(model) {
            return;
        }
        self.end_declined();
        let models = self.ports.models.clone();
        spawn_blocking(move || models.start_download(model));
    }

    // ASR-2: the load waits for the download; ASR-9 loads once it finishes
    pub(super) fn download_then_load(&mut self, model: ModelId) {
        // the tray and overlay follow the download's progress (OVL-19)
        self.force_loading = false;
        self.load_download = Some(model);
        self.model_state = ModelState::Loading {
            status: PREPARING_STATUS.to_owned(),
            progress: None,
        };
        self.begin_download(model);
    }

    pub(super) fn cancel_download(&mut self, model: ModelId) {
        if let Some(onboarding) = &mut self.ui.onboarding {
            onboarding.wizard.cancel_download();
        }
        let models = self.ports.models.clone();
        spawn_blocking(move || models.cancel_download(model));
    }

    pub(super) fn on_download(&mut self, update: DownloadUpdate) {
        let model = update.model;
        let name = self.catalog.name(model).to_owned();
        let compatible = routes(&self.catalog, self.settings.language)
            .iter()
            .any(|route| route.model == model);
        let onboarding_update = match &update.kind {
            DownloadUpdateKind::Started | DownloadUpdateKind::Progress(_) => {
                let progress = match update.kind {
                    DownloadUpdateKind::Progress(progress) => progress,
                    _ => 0.0,
                };
                self.downloads.progressed(ModelDownload {
                    model_id: model.as_str().to_owned(),
                    progress,
                    status: status_text(&name, DownloadStatus::Downloading(progress)),
                });
                Some(OnboardingDownloadUpdate::Active)
            }
            DownloadUpdateKind::Finished => {
                self.downloads.ended(model.as_str());
                self.refresh_downloaded();
                self.download_finished(model);
                Some(OnboardingDownloadUpdate::Active)
            }
            DownloadUpdateKind::Cancelled => {
                self.downloads.ended(model.as_str());
                self.pending.download_stopped(model);
                if self.load_download == Some(model) {
                    // a cancelled load reports not_ready (ASR-7)
                    self.load_download = None;
                    self.model_state = ModelState::NotReady;
                }
                Some(OnboardingDownloadUpdate::Failed {
                    cancelled: true,
                    error: String::new(),
                })
            }
            DownloadUpdateKind::Failed { error } => {
                self.downloads.ended(model.as_str());
                self.pending.download_stopped(model);
                let end = DownloadEnd::Failed {
                    error: (!error.is_empty()).then(|| error.clone()),
                };
                let shown = match &end {
                    DownloadEnd::Failed { error: Some(error) } => error.clone(),
                    _ => status_text(&name, DownloadStatus::Failed),
                };
                if self.load_download == Some(model) {
                    // the load failed: error state and ASR-6
                    self.load_download = None;
                    self.model_state = ModelState::Error {
                        message: shown.clone(),
                    };
                    let message = load_failure_notification(model, &self.downloaded, &shown);
                    self.notify(message);
                } else {
                    let speech_visible = self.ui.is_visible(MainWindowSection::Speech);
                    match failure_report(&name, &end, speech_visible) {
                        Some(FailureReport::Alert(message)) => {
                            self.push_speech_alert(DOWNLOAD_ALERT_TITLE, message, None);
                        }
                        Some(FailureReport::Notification(message)) => {
                            self.notify(message);
                        }
                        None => {}
                    }
                }
                Some(OnboardingDownloadUpdate::Failed {
                    cancelled: false,
                    error: shown,
                })
            }
        };
        if let (Some(onboarding), Some(update), true) =
            (&mut self.ui.onboarding, onboarding_update, compatible)
        {
            onboarding.wizard.download_update(update);
        }
    }

    // --- delete and re-download (DL-9..DL-11) ---

    pub(super) fn request_delete_model(&mut self, model: ModelId) {
        if !self.downloaded.contains(&model) || self.catalog.is_built_in(model) {
            return;
        }
        let confirmation = delete_confirmation(self.catalog.name(model));
        let id = self.push_speech_alert(
            &confirmation.title,
            confirmation.message,
            Some(confirmation.confirm),
        );
        self.ui
            .speech
            .actions
            .insert(id, PendingAlertAction::Delete(model));
    }

    pub(super) fn delete_confirmed(&mut self, model: ModelId) {
        let settings = self.settings.clone();
        let plan = delete_model(
            &self.catalog,
            model,
            &settings.stored_model,
            &self.downloaded,
            settings.language,
            default_language(settings.os_language.as_deref()),
        );
        let name = self.catalog.name(model).to_owned();
        let reselect = match plan {
            Ok(plan) => plan.reselect,
            Err(error) => {
                let message = delete_failed_message(&name, &error.to_string());
                self.push_speech_alert(DOWNLOAD_ALERT_TITLE, message, None);
                return;
            }
        };
        let models = self.ports.models.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::Ui(UiMsg::ModelDeleted {
                model,
                reselect,
                result: models.delete(model),
            })
        }));
    }

    pub(super) fn model_deleted(
        &mut self,
        model: ModelId,
        reselect: Option<ModelId>,
        result: Result<(), String>,
    ) {
        self.refresh_downloaded();
        if let Err(error) = result {
            let message = delete_failed_message(self.catalog.name(model), &error);
            self.push_speech_alert(DOWNLOAD_ALERT_TITLE, message, None);
            return;
        }
        self.downloaded.retain(|m| *m != model);
        if let Some(next) = reselect {
            // the selected model is gone: select and reload another (DL-10)
            self.refresh_settings();
            self.pending.clear();
            self.switch_model(next, true);
        }
    }

    // DL-11: replace the files of a model that failed to load
    pub(super) fn redownload_model(&mut self) {
        let model = self.desired_model();
        let downloading = self.downloads.get(model.as_str()).is_some();
        if !can_redownload(&self.catalog, model, self.load_in_flight(), downloading) {
            return;
        }
        self.end_declined();
        self.load_download = Some(model);
        self.model_state = ModelState::Loading {
            status: PREPARING_STATUS.to_owned(),
            progress: None,
        };
        let models = self.ports.models.clone();
        spawn_blocking(move || models.redownload(model));
    }

    // --- languages (LANG-8..LANG-10) ---

    /// LANG-8: opening the menu records the current language first.
    pub(super) fn record_recent_languages(&mut self) {
        let settings = self.settings.clone();
        let recents = recent_languages(settings.language, &settings.stored_recent_languages);
        if encode_recent(&recents) != settings.stored_recent_languages {
            self.apply_setting(SettingChange::RecentLanguages(recents));
        }
    }

    pub(super) fn select_language_from_menu(&mut self, language: LanguageSetting) {
        let settings = self.settings.clone();
        let Some(plan) = switch_language_from_menu(
            &self.catalog,
            language,
            matches!(self.phase, Phase::Idle),
            &settings.stored_recent_languages,
            &settings.stored_model,
            &self.downloaded,
        ) else {
            return;
        };
        self.apply_setting(SettingChange::Language(plan.language));
        self.apply_setting(SettingChange::RecentLanguages(recent_languages(
            plan.language,
            &settings.stored_recent_languages,
        )));
        // the tray switch is an explicit selection attempt, even when it
        // keeps the current model (DL-8)
        self.pending.clear();
        match plan.model {
            MenuModelChoice::Use(model) => self.switch_model(model, true),
            MenuModelChoice::OpenSpeechSettings => self.open_section(MainWindowSection::Speech),
        }
    }

    // --- the Speech page alert queue (WIN-20) ---

    pub(super) fn push_speech_alert(
        &mut self,
        title: &str,
        message: String,
        confirm: Option<&str>,
    ) -> u64 {
        self.ui.speech.next_id += 1;
        let id = self.ui.speech.next_id;
        self.ui.speech.alerts.push_back(SpeechAlert {
            id,
            title: title.to_owned(),
            message,
            confirm: confirm.map(str::to_owned),
            dismiss: match confirm {
                Some(_) => "Cancel".to_owned(),
                None => "OK".to_owned(),
            },
        });
        id
    }

    pub(super) fn close_speech_alert(&mut self, id: u64, confirmed: bool) {
        self.ui.speech.alerts.retain(|alert| alert.id != id);
        let action = self.ui.speech.actions.remove(&id);
        if let (true, Some(PendingAlertAction::Delete(model))) = (confirmed, action) {
            self.delete_confirmed(model);
        }
    }
}
