//! Select, "Download & Use", delete, and re-download rules, plus model-load
//! decisions and copy (DL-7..DL-11, ASR-1, ASR-6, ASR-9).

use crate::languages::LanguageSetting;
use crate::messages::UserMessage;
use crate::migration::{
    DeclinedLoad, Migration, SelectionResolution, declined_load, pending_notice,
};
use crate::models::{Catalog, ModelError, ModelId};
use crate::routing::{default_model, supports};

/// Result of a selection attempt (DL-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectOutcome {
    /// Save the model as `selectedModel`, clear the declined migration key
    /// (DL-14), and reload (ASR-8).
    Selected,
    /// Already selected: no-op.
    AlreadySelected,
    /// Only downloaded models can be selected.
    NotDownloaded,
}

/// Decides a selection attempt (DL-7). `stored_selected` is the raw
/// `selectedModel` setting; `downloaded` comes from [`Catalog::downloaded`].
/// Every attempt, whatever the outcome, clears the pending selection
/// ([`PendingSelection::clear`], DL-8).
pub fn select_model(
    requested: ModelId,
    stored_selected: &str,
    downloaded: &[ModelId],
) -> SelectOutcome {
    if !downloaded.contains(&requested) {
        return SelectOutcome::NotDownloaded;
    }
    if requested.as_str() == stored_selected {
        return SelectOutcome::AlreadySelected;
    }
    SelectOutcome::Selected
}

/// The "Download & Use" model waiting for its download (DL-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PendingSelection(Option<ModelId>);

impl PendingSelection {
    /// No pending selection.
    pub fn new() -> PendingSelection {
        PendingSelection(None)
    }

    /// The model waiting for its download, if any.
    pub fn model(self) -> Option<ModelId> {
        self.0
    }

    /// "Download & Use" started for `model`; the caller also starts the
    /// download.
    pub fn download_and_use(&mut self, model: ModelId) {
        self.0 = Some(model);
    }

    /// Any explicit selection attempt, including the tray language switch
    /// even when it keeps the current model.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// `model`'s download failed or was cancelled.
    pub fn download_stopped(&mut self, model: ModelId) {
        if self.0 == Some(model) {
            self.0 = None;
        }
    }

    /// `model`'s download finished. Returns the model to select now when the
    /// session is idle; otherwise it keeps waiting for [`Self::session_idle`]
    /// (DICT-29).
    pub fn download_finished(&mut self, model: ModelId, session_idle: bool) -> Option<ModelId> {
        if self.0 != Some(model) || !session_idle {
            return None;
        }
        self.0.take()
    }

    /// The session returned to idle. Returns the model to select if its
    /// download has finished.
    pub fn session_idle(&mut self, downloaded: &[ModelId]) -> Option<ModelId> {
        let model = self.0?;
        if !downloaded.contains(&model) {
            return None;
        }
        self.0.take()
    }
}

/// What a permitted delete does after removing the folder (DL-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletePlan {
    /// When the selected model was deleted: the model to select and reload.
    pub reselect: Option<ModelId>,
}

/// Decides a delete request (DL-10). Built-in engines and models that are
/// not downloaded fail with [`ModelError::NotFound`]; deleting the only
/// downloaded model while it is selected fails with
/// [`ModelError::CannotDeleteActiveModel`]. Deleting the selected model
/// otherwise reselects the first other downloaded model that supports
/// `current_language`, else the first other downloaded model in catalog
/// order, else the default model.
pub fn delete_model(
    catalog: &Catalog,
    model: ModelId,
    stored_selected: &str,
    downloaded: &[ModelId],
    current_language: LanguageSetting,
    default_language: LanguageSetting,
) -> Result<DeletePlan, ModelError> {
    if catalog.is_built_in(model) || !downloaded.contains(&model) {
        return Err(ModelError::NotFound {
            model_id: model.as_str().to_string(),
        });
    }
    if model.as_str() != stored_selected {
        return Ok(DeletePlan { reselect: None });
    }
    let others: Vec<ModelId> = downloaded.iter().copied().filter(|m| *m != model).collect();
    if others.is_empty() {
        return Err(ModelError::CannotDeleteActiveModel);
    }
    let reselect = others
        .iter()
        .copied()
        .find(|m| supports(catalog, current_language, *m))
        .or_else(|| others.first().copied())
        .unwrap_or_else(|| default_model(catalog, default_language));
    Ok(DeletePlan {
        reselect: Some(reselect),
    })
}

/// Whether a model row shows a delete control (DL-9): never for built-in
/// engines.
pub fn has_delete_control(catalog: &Catalog, model: ModelId) -> bool {
    !catalog.is_built_in(model)
}

/// Delete confirmation alert copy (DL-9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteConfirmation {
    /// "Delete model".
    pub title: String,
    /// "Delete <Model>? ...".
    pub message: String,
    /// "Delete", destructive.
    pub confirm: &'static str,
    /// "Cancel".
    pub cancel: &'static str,
}

/// Builds the delete confirmation for `model_name`.
pub fn delete_confirmation(model_name: &str) -> DeleteConfirmation {
    DeleteConfirmation {
        title: "Delete model".to_string(),
        message: format!(
            "Delete {model_name}? This action removes local model files and cannot be undone."
        ),
        confirm: "Delete",
        cancel: "Cancel",
    }
}

/// Alert text when a delete fails (WIN-20).
pub fn delete_failed_message(model_name: &str, error: &str) -> String {
    format!("Failed to delete {model_name}: {error}")
}

/// Whether "Re-download Model" may run (DL-11): refused for built-in engines,
/// for models already downloading, and while a load is in flight.
pub fn can_redownload(
    catalog: &Catalog,
    model: ModelId,
    load_in_flight: bool,
    downloading: bool,
) -> bool {
    catalog.contains(model) && !catalog.is_built_in(model) && !load_in_flight && !downloading
}

/// The "Couldn't Load Speech Model" notification (ASR-6). `on_disk` lists
/// models whose folders passed validation (built-in engines never do).
pub fn load_failure_notification(model: ModelId, on_disk: &[ModelId], error: &str) -> UserMessage {
    UserMessage::CouldntLoadSpeechModel {
        error: error.to_owned(),
        files_on_disk: on_disk.contains(&model),
    }
}

/// What to load at launch, once the UI is ready (ASR-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchLoad {
    /// First-run onboarding is pending; it owns the first download.
    Nothing,
    /// Show the update notice after [`crate::migration::NOTICE_DELAY`]; load
    /// nothing until the user answers it.
    ShowNotice(Migration),
    /// The migration download was declined: load this temporary override.
    LoadFallback(ModelId),
    /// Declined with no downloaded fallback: open Speech settings.
    OpenSpeechSettings,
    /// Load the selected model (resolve it with
    /// [`crate::migration::load_target`]).
    LoadSelected,
}

impl LaunchLoad {
    /// Whether the tray shows the loading state immediately (ASR-1).
    pub fn starts_load(&self) -> bool {
        matches!(self, LaunchLoad::LoadFallback(_) | LaunchLoad::LoadSelected)
    }
}

/// Settings and facts the launch decision reads.
#[derive(Debug, Clone, Copy)]
pub struct LaunchState<'a> {
    /// First-run onboarding has not completed (SET-4).
    pub onboarding_pending: bool,
    /// `lastModelMigrationNoticeKey`.
    pub last_notice_key: &'a str,
    /// `declinedModelMigrationNoticeKey`.
    pub declined_notice_key: &'a str,
    /// `selectedLanguage`.
    pub selected_language: LanguageSetting,
    /// From [`Catalog::downloaded`].
    pub downloaded: &'a [ModelId],
}

/// Decides the launch-time load (ASR-1, DL-14). `resolution` comes from
/// [`crate::migration::resolve_selection`].
pub fn launch_load(
    catalog: &Catalog,
    resolution: &SelectionResolution,
    state: &LaunchState<'_>,
) -> LaunchLoad {
    if state.onboarding_pending {
        return LaunchLoad::Nothing;
    }
    if let Some(migration) = pending_notice(resolution, false, state.last_notice_key) {
        return LaunchLoad::ShowNotice(migration.clone());
    }
    match declined_load(
        catalog,
        resolution,
        state.declined_notice_key,
        state.selected_language,
        state.downloaded,
    ) {
        Some(DeclinedLoad::LoadFallback(model)) => LaunchLoad::LoadFallback(model),
        Some(DeclinedLoad::OpenSpeechSettings) => LaunchLoad::OpenSpeechSettings,
        None => LaunchLoad::LoadSelected,
    }
}

/// Whether a finished download should clear the temporary model override and
/// start a load (ASR-9): the selected model finished and no load is in
/// flight. Covers every download entry point.
pub fn reload_after_download(
    finished: ModelId,
    stored_selected: &str,
    load_in_flight: bool,
) -> bool {
    finished.as_str() == stored_selected && !load_in_flight
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::{with_system, without_system};
    use crate::privacy_naming::PrivacyNaming;
    use ModelId::*;

    fn lang(code: &str) -> LanguageSetting {
        LanguageSetting::parse(code).unwrap_or_else(|| panic!("{code} not offered"))
    }

    fn migration(from: &str, to: ModelId) -> Migration {
        Migration {
            from: from.to_string(),
            to,
        }
    }

    #[test]
    fn select_model_dl_7() {
        struct Case {
            name: &'static str,
            system: bool,
            requested: ModelId,
            stored: &'static str,
            on_disk: Vec<ModelId>,
            want: SelectOutcome,
        }
        let cases = [
            Case {
                name: "downloaded model selects",
                system: false,
                requested: ParakeetUltra,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetUltra],
                want: SelectOutcome::Selected,
            },
            Case {
                name: "not downloaded is refused",
                system: true,
                requested: ParakeetTdtV2,
                stored: "parakeet-unified",
                on_disk: vec![],
                want: SelectOutcome::NotDownloaded,
            },
            Case {
                name: "same model is a no-op",
                system: true,
                requested: ParakeetUnified,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetUnified],
                want: SelectOutcome::AlreadySelected,
            },
            Case {
                name: "built-in counts as downloaded",
                system: true,
                requested: SystemSpeech,
                stored: "parakeet-unified",
                on_disk: vec![],
                want: SelectOutcome::Selected,
            },
            Case {
                name: "system speech absent is refused",
                system: false,
                requested: SystemSpeech,
                stored: "parakeet-unified",
                on_disk: vec![],
                want: SelectOutcome::NotDownloaded,
            },
            Case {
                name: "replacing an unknown stored id",
                system: false,
                requested: ParakeetTdtV2,
                stored: "greendale-human-being",
                on_disk: vec![ParakeetTdtV2],
                want: SelectOutcome::Selected,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            let downloaded = catalog.downloaded(&tc.on_disk);
            assert_eq!(
                select_model(tc.requested, tc.stored, &downloaded),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn pending_selection_applies_once_installed_dl_8() {
        let mut pending = PendingSelection::new();
        assert_eq!(
            pending.session_idle(&[ParakeetTdtV2]),
            None,
            "nothing pending"
        );

        pending.download_and_use(ParakeetTdtV2);
        assert_eq!(pending.model(), Some(ParakeetTdtV2));
        assert_eq!(pending.session_idle(&[]), None, "not downloaded yet");
        assert_eq!(pending.model(), Some(ParakeetTdtV2), "still waiting");

        assert_eq!(pending.session_idle(&[ParakeetTdtV2]), Some(ParakeetTdtV2));
        assert_eq!(pending.model(), None);
        assert_eq!(
            pending.session_idle(&[ParakeetTdtV2]),
            None,
            "applied only once"
        );
    }

    #[test]
    fn pending_selection_download_finished_dl_8() {
        struct Case {
            name: &'static str,
            finished: ModelId,
            idle: bool,
            want: Option<ModelId>,
            want_pending: Option<ModelId>,
        }
        let cases = [
            Case {
                name: "finished while idle switches now",
                finished: ParakeetUltra,
                idle: true,
                want: Some(ParakeetUltra),
                want_pending: None,
            },
            Case {
                name: "finished mid-session waits for idle",
                finished: ParakeetUltra,
                idle: false,
                want: None,
                want_pending: Some(ParakeetUltra),
            },
            Case {
                name: "another model finishing is ignored",
                finished: ParakeetJapanese,
                idle: true,
                want: None,
                want_pending: Some(ParakeetUltra),
            },
        ];
        for tc in cases {
            let mut pending = PendingSelection::new();
            pending.download_and_use(ParakeetUltra);
            assert_eq!(
                pending.download_finished(tc.finished, tc.idle),
                tc.want,
                "{}",
                tc.name
            );
            assert_eq!(pending.model(), tc.want_pending, "{}", tc.name);
        }
    }

    #[test]
    fn pending_selection_dropped_by_other_choices_dl_8() {
        struct Case {
            name: &'static str,
            drop: fn(&mut PendingSelection),
            want: Option<ModelId>,
        }
        let cases = [
            Case {
                name: "picking a model explicitly",
                drop: |p| p.clear(),
                want: None,
            },
            Case {
                name: "cancelling or failing the download",
                drop: |p| p.download_stopped(ParakeetTdtV2),
                want: None,
            },
            Case {
                name: "another download stopping",
                drop: |p| p.download_stopped(ParakeetUltra),
                want: Some(ParakeetTdtV2),
            },
        ];
        for tc in cases {
            let mut pending = PendingSelection::new();
            pending.download_and_use(ParakeetTdtV2);
            (tc.drop)(&mut pending);
            assert_eq!(pending.model(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn delete_model_dl_10() {
        struct Case {
            name: &'static str,
            system: bool,
            model: ModelId,
            stored: &'static str,
            on_disk: Vec<ModelId>,
            current_language: &'static str,
            want: Result<DeletePlan, ModelError>,
        }
        let not_found = |id: &str| {
            Err(ModelError::NotFound {
                model_id: id.to_string(),
            })
        };
        let cases = [
            Case {
                name: "unselected model just deletes",
                system: false,
                model: ParakeetTdtV2,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetTdtV2, ParakeetUnified],
                current_language: "en",
                want: Ok(DeletePlan { reselect: None }),
            },
            Case {
                name: "only downloaded selected model is refused",
                system: false,
                model: ParakeetUnified,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetUnified],
                current_language: "en",
                want: Err(ModelError::CannotDeleteActiveModel),
            },
            Case {
                name: "with a system engine the selected model can always go",
                system: true,
                model: ParakeetUnified,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetUnified],
                current_language: "en",
                want: Ok(DeletePlan {
                    reselect: Some(SystemSpeech),
                }),
            },
            Case {
                name: "prefers a downloaded model that supports the language",
                system: true,
                model: ParakeetUltra,
                stored: "parakeet-ultra",
                on_disk: vec![ParakeetTdtV2, ParakeetUltra],
                current_language: "de",
                want: Ok(DeletePlan {
                    reselect: Some(SystemSpeech),
                }),
            },
            Case {
                name: "falls back to first other downloaded in catalog order",
                system: false,
                model: ParakeetUltra,
                stored: "parakeet-ultra",
                on_disk: vec![ParakeetUnified, ParakeetJapanese, ParakeetUltra],
                current_language: "de",
                want: Ok(DeletePlan {
                    reselect: Some(ParakeetJapanese),
                }),
            },
            Case {
                name: "built-in engine has no delete",
                system: true,
                model: SystemSpeech,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetUnified],
                current_language: "en",
                want: not_found("system-speech"),
            },
            Case {
                name: "not downloaded is not found",
                system: true,
                model: ParakeetJapanese,
                stored: "parakeet-unified",
                on_disk: vec![ParakeetUnified],
                current_language: "en",
                want: not_found("parakeet-ja"),
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            let downloaded = catalog.downloaded(&tc.on_disk);
            let got = delete_model(
                &catalog,
                tc.model,
                tc.stored,
                &downloaded,
                lang(tc.current_language),
                lang("en"),
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
        assert_eq!(
            ModelError::CannotDeleteActiveModel.to_string(),
            "Cannot delete the currently active model. Please select a different model first."
        );
    }

    #[test]
    fn delete_copy_dl_9() {
        let confirmation = delete_confirmation("Parakeet Ultra");
        assert_eq!(confirmation.title, "Delete model");
        assert_eq!(
            confirmation.message,
            "Delete Parakeet Ultra? This action removes local model files and cannot be undone."
        );
        assert_eq!(
            (confirmation.confirm, confirmation.cancel),
            ("Delete", "Cancel")
        );
        assert_eq!(
            delete_failed_message("Parakeet Ultra", "permission denied"),
            "Failed to delete Parakeet Ultra: permission denied"
        );
        let catalog = with_system();
        assert!(!has_delete_control(&catalog, SystemSpeech));
        assert!(has_delete_control(&catalog, ParakeetTdtV2));
    }

    #[test]
    fn redownload_eligibility_dl_11() {
        struct Case {
            name: &'static str,
            system: bool,
            model: ModelId,
            load_in_flight: bool,
            downloading: bool,
            want: bool,
        }
        let cases = [
            Case {
                name: "downloadable model after a failure",
                system: true,
                model: ParakeetTdtV2,
                load_in_flight: false,
                downloading: false,
                want: true,
            },
            Case {
                name: "built-in refused",
                system: true,
                model: SystemSpeech,
                load_in_flight: false,
                downloading: false,
                want: false,
            },
            Case {
                name: "absent system speech refused",
                system: false,
                model: SystemSpeech,
                load_in_flight: false,
                downloading: false,
                want: false,
            },
            Case {
                name: "ignored while a load is in flight",
                system: true,
                model: ParakeetTdtV2,
                load_in_flight: true,
                downloading: false,
                want: false,
            },
            Case {
                name: "refused while already downloading",
                system: true,
                model: ParakeetTdtV2,
                load_in_flight: false,
                downloading: true,
                want: false,
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            assert_eq!(
                can_redownload(&catalog, tc.model, tc.load_in_flight, tc.downloading),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn load_failure_notification_asr_6() {
        struct Case {
            name: &'static str,
            model: ModelId,
            on_disk: Vec<ModelId>,
            tray_term: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "files missing",
                model: ParakeetTdtV2,
                on_disk: vec![],
                tray_term: "menu bar icon",
                want: "Failed to load the transcription model Check your internet connection, then choose Retry Loading Model from the menu bar icon.",
            },
            Case {
                name: "built-in engine counts as not on disk",
                model: SystemSpeech,
                on_disk: vec![ParakeetTdtV2],
                tray_term: "menu bar icon",
                want: "Failed to load the transcription model Check your internet connection, then choose Retry Loading Model from the menu bar icon.",
            },
            Case {
                name: "files on disk offer re-download",
                model: ParakeetTdtV2,
                on_disk: vec![ParakeetTdtV2],
                tray_term: "menu bar icon",
                want: "Failed to load the transcription model Choose Retry Loading Model from the menu bar icon, or Re-download Model to replace its files.",
            },
            Case {
                name: "shell supplies the tray term",
                model: ParakeetTdtV2,
                on_disk: vec![],
                tray_term: "tray icon",
                want: "Failed to load the transcription model Check your internet connection, then choose Retry Loading Model from the tray icon.",
            },
        ];
        for tc in cases {
            let got = load_failure_notification(
                tc.model,
                &tc.on_disk,
                "Failed to load the transcription model",
            );
            let naming = PrivacyNaming::Accessibility;
            assert_eq!(
                got.title(naming),
                "Couldn't Load Speech Model",
                "{}",
                tc.name
            );
            assert_eq!(got.body(tc.tray_term, naming), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn launch_load_asr_1() {
        use SelectionResolution::*;
        struct Case {
            name: &'static str,
            resolution: SelectionResolution,
            onboarding_pending: bool,
            last_notice_key: &'static str,
            declined_notice_key: &'static str,
            on_disk: Vec<ModelId>,
            want: LaunchLoad,
        }
        let needs = || NeedsDownload(migration("old", ParakeetUltra));
        let key = "old->parakeet-ultra";
        let cases = [
            Case {
                name: "onboarding pending loads nothing",
                resolution: needs(),
                onboarding_pending: true,
                last_notice_key: "",
                declined_notice_key: "",
                on_disk: vec![ParakeetTdtV2],
                want: LaunchLoad::Nothing,
            },
            Case {
                name: "pending notice loads nothing",
                resolution: needs(),
                onboarding_pending: false,
                last_notice_key: "",
                declined_notice_key: "",
                on_disk: vec![ParakeetTdtV2],
                want: LaunchLoad::ShowNotice(migration("old", ParakeetUltra)),
            },
            Case {
                name: "declined loads the fallback",
                resolution: needs(),
                onboarding_pending: false,
                last_notice_key: key,
                declined_notice_key: key,
                on_disk: vec![ParakeetTdtV2],
                want: LaunchLoad::LoadFallback(ParakeetTdtV2),
            },
            Case {
                name: "declined without fallback opens speech settings",
                resolution: needs(),
                onboarding_pending: false,
                last_notice_key: key,
                declined_notice_key: key,
                on_disk: vec![],
                want: LaunchLoad::OpenSpeechSettings,
            },
            Case {
                name: "notice shown but not declined loads selected",
                resolution: needs(),
                onboarding_pending: false,
                last_notice_key: key,
                declined_notice_key: "",
                on_disk: vec![],
                want: LaunchLoad::LoadSelected,
            },
            Case {
                name: "current selection loads",
                resolution: Current(ParakeetTdtV2),
                onboarding_pending: false,
                last_notice_key: "",
                declined_notice_key: "",
                on_disk: vec![ParakeetTdtV2],
                want: LaunchLoad::LoadSelected,
            },
            Case {
                name: "silent switch loads",
                resolution: SwitchSilently(migration("old", ParakeetTdtV2)),
                onboarding_pending: false,
                last_notice_key: "",
                declined_notice_key: "old->parakeet-tdt-v2",
                on_disk: vec![ParakeetTdtV2],
                want: LaunchLoad::LoadSelected,
            },
        ];
        let catalog = without_system();
        for tc in cases {
            let downloaded = catalog.downloaded(&tc.on_disk);
            let state = LaunchState {
                onboarding_pending: tc.onboarding_pending,
                last_notice_key: tc.last_notice_key,
                declined_notice_key: tc.declined_notice_key,
                selected_language: lang("de"),
                downloaded: &downloaded,
            };
            let got = launch_load(&catalog, &tc.resolution, &state);
            assert_eq!(
                got.starts_load(),
                matches!(
                    tc.want,
                    LaunchLoad::LoadFallback(_) | LaunchLoad::LoadSelected
                ),
                "{}",
                tc.name
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn reload_after_download_asr_9() {
        struct Case {
            name: &'static str,
            finished: ModelId,
            stored: &'static str,
            load_in_flight: bool,
            want: bool,
        }
        let cases = [
            Case {
                name: "selected model finished",
                finished: ParakeetUltra,
                stored: "parakeet-ultra",
                load_in_flight: false,
                want: true,
            },
            Case {
                name: "another model finished",
                finished: ParakeetJapanese,
                stored: "parakeet-ultra",
                load_in_flight: false,
                want: false,
            },
            Case {
                name: "load already in flight",
                finished: ParakeetUltra,
                stored: "parakeet-ultra",
                load_in_flight: true,
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                reload_after_download(tc.finished, tc.stored, tc.load_in_flight),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
