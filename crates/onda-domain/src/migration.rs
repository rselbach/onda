//! Catalog evolution: removed or unknown selected models, the update notice,
//! the declined state, and removed-model cleanup (DL-12..DL-15, ASR-4).
//!
//! The declined state lives in the `declinedModelMigrationNoticeKey` setting.
//! It ends when the caller clears that key, which it does on any explicit
//! model selection, any download start, any model change, and the notice's
//! Download or Choose Another ([`respond_to_notice`]). It also ends on its
//! own once the replacement is downloaded, because [`resolve_selection`] then
//! switches silently instead of reporting [`SelectionResolution::NeedsDownload`].

use crate::languages::LanguageSetting;
use crate::models::{Catalog, ModelId};
use crate::routing::{default_model, recommended_model};
use std::time::Duration;

/// Removed model ids mapped to their replacements (DL-12). Empty until a
/// release drops a model.
pub const REMOVED_MODEL_IDS: &[(&str, ModelId)] = &[];

/// Delay before the update notice window appears (DL-14).
pub const NOTICE_DELAY: Duration = Duration::from_millis(750);

/// A stored selection that must change: `from` is the stored id, `to` the
/// replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    /// The stored, removed or unknown id.
    pub from: String,
    /// The replacement model.
    pub to: ModelId,
}

impl Migration {
    /// The `old->new` key stored in `lastModelMigrationNoticeKey` and
    /// `declinedModelMigrationNoticeKey`.
    pub fn notice_key(&self) -> String {
        format!("{}->{}", self.from, self.to.as_str())
    }
}

/// What reading the stored selection found (DL-12..DL-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionResolution {
    /// The stored id is in the catalog.
    Current(ModelId),
    /// The replacement is downloaded or built in: save `to` as the selection
    /// without telling the user (DL-13).
    SwitchSilently(Migration),
    /// The replacement needs a download: leave the stored selection untouched
    /// (DL-14).
    NeedsDownload(Migration),
}

impl SelectionResolution {
    /// The migration, if the stored id is removed or unknown.
    pub fn migration(&self) -> Option<&Migration> {
        match self {
            SelectionResolution::Current(_) => None,
            SelectionResolution::SwitchSilently(m) | SelectionResolution::NeedsDownload(m) => {
                Some(m)
            }
        }
    }
}

/// Resolves the stored `selectedModel` (DL-12, DL-13). A removed or unknown
/// id is replaced by its mapped id from `removed` (normally
/// [`REMOVED_MODEL_IDS`]) if that model is in the catalog, else by the
/// recommended model for the default language. `downloaded` comes from
/// [`Catalog::downloaded`], so built-in engines count.
pub fn resolve_selection(
    catalog: &Catalog,
    stored: &str,
    removed: &[(&str, ModelId)],
    default_language: LanguageSetting,
    downloaded: &[ModelId],
) -> SelectionResolution {
    if let Some(id) = catalog.resolve(stored) {
        return SelectionResolution::Current(id);
    }
    let to = removed
        .iter()
        .find(|(old, _)| *old == stored)
        .map(|(_, new)| *new)
        .filter(|new| catalog.contains(*new))
        .unwrap_or_else(|| default_model(catalog, default_language));
    let migration = Migration {
        from: stored.to_string(),
        to,
    };
    if downloaded.contains(&to) {
        return SelectionResolution::SwitchSilently(migration);
    }
    SelectionResolution::NeedsDownload(migration)
}

/// The migration whose update notice should show at launch (DL-14): one that
/// needs a download, unless first-run onboarding is pending or the last
/// notice shown had the same key.
pub fn pending_notice<'a>(
    resolution: &'a SelectionResolution,
    onboarding_pending: bool,
    last_shown_key: &str,
) -> Option<&'a Migration> {
    match resolution {
        SelectionResolution::NeedsDownload(m)
            if !onboarding_pending && m.notice_key() != last_shown_key =>
        {
            Some(m)
        }
        _ => None,
    }
}

/// Whether the delayed notice should still appear (DL-14): skipped silently
/// if onboarding or another notice window opened meanwhile.
pub fn should_present_notice(onboarding_open: bool, notice_open: bool) -> bool {
    !onboarding_open && !notice_open
}

/// Whether the user declined this migration's download (DL-14). False once
/// the replacement is downloaded.
pub fn is_declined(resolution: &SelectionResolution, declined_key: &str) -> bool {
    matches!(resolution, SelectionResolution::NeedsDownload(m) if m.notice_key() == declined_key)
}

/// The temporary override to load while declined (DL-14): the recommended
/// model for the *selected* language if downloaded, else the first downloaded
/// model in catalog order.
pub fn declined_fallback(
    catalog: &Catalog,
    selected_language: LanguageSetting,
    downloaded: &[ModelId],
) -> Option<ModelId> {
    recommended_model(catalog, selected_language)
        .filter(|r| downloaded.contains(r))
        .or_else(|| downloaded.first().copied())
}

/// What to load while the migration download is declined (DL-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclinedLoad {
    /// Load this model as a temporary override; keep the stored selection.
    LoadFallback(ModelId),
    /// No fallback is downloaded: open Speech settings.
    OpenSpeechSettings,
}

/// The declined-state load, or `None` when not declined. Used at launch
/// (ASR-1), after Not Now, and when onboarding closes early (ONB-11).
pub fn declined_load(
    catalog: &Catalog,
    resolution: &SelectionResolution,
    declined_key: &str,
    selected_language: LanguageSetting,
    downloaded: &[ModelId],
) -> Option<DeclinedLoad> {
    if !is_declined(resolution, declined_key) {
        return None;
    }
    Some(
        declined_fallback(catalog, selected_language, downloaded)
            .map_or(DeclinedLoad::OpenSpeechSettings, DeclinedLoad::LoadFallback),
    )
}

/// A button the user can press on the update notice. Closing the window
/// counts as [`NoticeChoice::NotNow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeChoice {
    /// Select the replacement and download it.
    Download,
    /// Open Speech settings.
    ChooseAnother,
    /// Decline for now.
    NotNow,
}

/// Keyboard shortcut bound to a notice button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeShortcut {
    /// Escape.
    Escape,
    /// Return; also the default (prominent) button.
    Return,
}

/// One notice button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoticeButton {
    /// Label.
    pub label: &'static str,
    /// What it does.
    pub choice: NoticeChoice,
    /// Bound key, if any.
    pub shortcut: Option<NoticeShortcut>,
}

/// Copy for the update notice window (DL-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeCopy {
    /// "Onda was updated".
    pub title: String,
    /// Body with the new model name in Markdown bold (`**...**`).
    pub body: String,
    /// Buttons left to right.
    pub buttons: [NoticeButton; 3],
}

/// Builds the update notice copy for `migration`.
pub fn notice_copy(catalog: &Catalog, migration: &Migration) -> NoticeCopy {
    NoticeCopy {
        title: "Onda was updated".to_string(),
        body: format!(
            "Your previous speech model is no longer available, so we've switched you to **{}**.",
            catalog.name(migration.to)
        ),
        buttons: [
            NoticeButton {
                label: "Not Now",
                choice: NoticeChoice::NotNow,
                shortcut: Some(NoticeShortcut::Escape),
            },
            NoticeButton {
                label: "Choose Another",
                choice: NoticeChoice::ChooseAnother,
                shortcut: None,
            },
            NoticeButton {
                label: "Download",
                choice: NoticeChoice::Download,
                shortcut: Some(NoticeShortcut::Return),
            },
        ],
    }
}

/// The follow-up action after a notice choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAction {
    /// Save this as `selectedModel`, start its download, close the window.
    DownloadReplacement(ModelId),
    /// Open the main window on Speech, then close the notice.
    OpenSpeechSettings,
    /// Load the declined fallback ([`declined_load`]), close the window.
    LoadDeclinedFallback,
}

/// Settings writes and the action for a notice choice (DL-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeResponse {
    /// New `lastModelMigrationNoticeKey`; every choice records it.
    pub last_shown_key: String,
    /// New `declinedModelMigrationNoticeKey`: `Some` to store, `None` to
    /// clear.
    pub declined_key: Option<String>,
    /// What to do next.
    pub action: NoticeAction,
}

/// Responds to the user's notice choice.
pub fn respond_to_notice(migration: &Migration, choice: NoticeChoice) -> NoticeResponse {
    let key = migration.notice_key();
    let (declined_key, action) = match choice {
        NoticeChoice::Download => (None, NoticeAction::DownloadReplacement(migration.to)),
        NoticeChoice::ChooseAnother => (None, NoticeAction::OpenSpeechSettings),
        NoticeChoice::NotNow => (Some(key.clone()), NoticeAction::LoadDeclinedFallback),
    };
    NoticeResponse {
        last_shown_key: key,
        declined_key,
        action,
    }
}

/// Download folders to delete at launch (DL-15): names under the models
/// directory that belong to removed model ids. Never selects a current
/// catalog id or a name not in `removed`.
pub fn removed_download_folders<'a>(
    folder_names: &[&'a str],
    removed: &[(&str, ModelId)],
) -> Vec<&'a str> {
    folder_names
        .iter()
        .copied()
        .filter(|name| ModelId::parse(name).is_none())
        .filter(|name| removed.iter().any(|(old, _)| old == name))
        .collect()
}

/// The model a load actually uses (ASR-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadTarget {
    /// Model to load.
    pub model: ModelId,
    /// True when the stored id was unknown: save `model` as `selectedModel`.
    pub save_selection: bool,
}

/// Resolves the stored selection for a load (ASR-4): an unknown id falls back
/// to the recommended model for the default language, which is saved.
pub fn load_target(
    catalog: &Catalog,
    stored: &str,
    default_language: LanguageSetting,
) -> LoadTarget {
    match catalog.resolve(stored) {
        Some(model) => LoadTarget {
            model,
            save_selection: false,
        },
        None => LoadTarget {
            model: default_model(catalog, default_language),
            save_selection: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::{with_system, without_system};
    use ModelId::*;

    const TEST_REMOVED: &[(&str, ModelId)] = &[
        ("parakeet-tdt-v3", ParakeetUltra),
        ("nemotron", ParakeetUnified),
        ("chang-speech", SystemSpeech),
    ];

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
    fn production_map_is_empty_dl_12() {
        assert!(REMOVED_MODEL_IDS.is_empty());
    }

    #[test]
    fn notice_key_is_stable() {
        assert_eq!(
            migration("medium", ParakeetTdtV2).notice_key(),
            "medium->parakeet-tdt-v2"
        );
    }

    #[test]
    fn resolve_selection_dl_12_dl_13() {
        use SelectionResolution::*;
        struct Case {
            name: &'static str,
            system: bool,
            stored: &'static str,
            default_language: &'static str,
            on_disk: Vec<ModelId>,
            want: SelectionResolution,
        }
        let cases = [
            Case {
                name: "valid id is current",
                system: true,
                stored: "parakeet-unified",
                default_language: "en",
                on_disk: vec![],
                want: Current(ParakeetUnified),
            },
            Case {
                name: "removed id maps to its replacement, which needs a download",
                system: true,
                stored: "parakeet-tdt-v3",
                default_language: "en",
                on_disk: vec![],
                want: NeedsDownload(migration("parakeet-tdt-v3", ParakeetUltra)),
            },
            Case {
                name: "nemotron becomes unified",
                system: true,
                stored: "nemotron",
                default_language: "en",
                on_disk: vec![],
                want: NeedsDownload(migration("nemotron", ParakeetUnified)),
            },
            Case {
                name: "downloaded replacement switches silently",
                system: true,
                stored: "parakeet-tdt-v3",
                default_language: "en",
                on_disk: vec![ParakeetUltra],
                want: SwitchSilently(migration("parakeet-tdt-v3", ParakeetUltra)),
            },
            Case {
                name: "built-in replacement switches silently",
                system: true,
                stored: "chang-speech",
                default_language: "en",
                on_disk: vec![],
                want: SwitchSilently(migration("chang-speech", SystemSpeech)),
            },
            Case {
                name: "unknown id uses the default language recommendation",
                system: true,
                stored: "totally-unknown-model",
                default_language: "en",
                on_disk: vec![],
                want: NeedsDownload(migration("totally-unknown-model", ParakeetTdtV2)),
            },
            Case {
                name: "unknown id with built-in recommendation switches silently",
                system: true,
                stored: "parakeet",
                default_language: "auto",
                on_disk: vec![],
                want: SwitchSilently(migration("parakeet", SystemSpeech)),
            },
            Case {
                name: "MOD-4 system speech without engine is unknown",
                system: false,
                stored: "system-speech",
                default_language: "de",
                on_disk: vec![],
                want: NeedsDownload(migration("system-speech", ParakeetUltra)),
            },
            Case {
                name: "replacement missing from catalog falls back to default",
                system: false,
                stored: "chang-speech",
                default_language: "ja",
                on_disk: vec![ParakeetJapanese],
                want: SwitchSilently(migration("chang-speech", ParakeetJapanese)),
            },
            Case {
                name: "LANG-5 system-only default language without engine",
                system: false,
                stored: "greendale-human-being",
                default_language: "ko",
                on_disk: vec![],
                want: NeedsDownload(migration("greendale-human-being", ParakeetUltra)),
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            let downloaded = catalog.downloaded(&tc.on_disk);
            let got = resolve_selection(
                &catalog,
                tc.stored,
                TEST_REMOVED,
                lang(tc.default_language),
                &downloaded,
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn pending_notice_dl_14() {
        use SelectionResolution::*;
        struct Case {
            name: &'static str,
            resolution: SelectionResolution,
            onboarding_pending: bool,
            last_shown_key: &'static str,
            want: Option<Migration>,
        }
        let cases = [
            Case {
                name: "no migration",
                resolution: Current(ParakeetTdtV2),
                onboarding_pending: false,
                last_shown_key: "",
                want: None,
            },
            Case {
                name: "silent switch never notifies",
                resolution: SwitchSilently(migration("old", ParakeetUltra)),
                onboarding_pending: false,
                last_shown_key: "",
                want: None,
            },
            Case {
                name: "empty last-shown key means never prompted",
                resolution: NeedsDownload(migration("old", ParakeetUltra)),
                onboarding_pending: false,
                last_shown_key: "",
                want: Some(migration("old", ParakeetUltra)),
            },
            Case {
                name: "already shown for this key",
                resolution: NeedsDownload(migration("old", ParakeetUltra)),
                onboarding_pending: false,
                last_shown_key: "old->parakeet-ultra",
                want: None,
            },
            Case {
                name: "different migration shows again",
                resolution: NeedsDownload(migration("new", ParakeetUnified)),
                onboarding_pending: false,
                last_shown_key: "old->parakeet-ultra",
                want: Some(migration("new", ParakeetUnified)),
            },
            Case {
                name: "onboarding pending owns the flow",
                resolution: NeedsDownload(migration("old", ParakeetUltra)),
                onboarding_pending: true,
                last_shown_key: "",
                want: None,
            },
        ];
        for tc in cases {
            let got = pending_notice(&tc.resolution, tc.onboarding_pending, tc.last_shown_key);
            assert_eq!(got, tc.want.as_ref(), "{}", tc.name);
        }
    }

    #[test]
    fn present_notice_only_without_other_windows_dl_14() {
        struct Case {
            name: &'static str,
            onboarding_open: bool,
            notice_open: bool,
            want: bool,
        }
        let cases = [
            Case {
                name: "nothing open",
                onboarding_open: false,
                notice_open: false,
                want: true,
            },
            Case {
                name: "onboarding open",
                onboarding_open: true,
                notice_open: false,
                want: false,
            },
            Case {
                name: "another notice open",
                onboarding_open: false,
                notice_open: true,
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                should_present_notice(tc.onboarding_open, tc.notice_open),
                tc.want,
                "{}",
                tc.name
            );
        }
        assert_eq!(NOTICE_DELAY, Duration::from_millis(750));
    }

    #[test]
    fn declined_matches_current_migration_dl_14() {
        use SelectionResolution::*;
        struct Case {
            name: &'static str,
            resolution: SelectionResolution,
            declined_key: &'static str,
            want: bool,
        }
        let cases = [
            Case {
                name: "matches current key",
                resolution: NeedsDownload(migration("medium", ParakeetTdtV2)),
                declined_key: "medium->parakeet-tdt-v2",
                want: true,
            },
            Case {
                name: "different migration key",
                resolution: NeedsDownload(migration("small", ParakeetTdtV2)),
                declined_key: "medium->parakeet-tdt-v2",
                want: false,
            },
            Case {
                name: "replacement downloaded ends the declined state",
                resolution: SwitchSilently(migration("medium", ParakeetTdtV2)),
                declined_key: "medium->parakeet-tdt-v2",
                want: false,
            },
            Case {
                name: "cleared key",
                resolution: NeedsDownload(migration("medium", ParakeetTdtV2)),
                declined_key: "",
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                is_declined(&tc.resolution, tc.declined_key),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn declined_fallback_dl_14() {
        struct Case {
            name: &'static str,
            system: bool,
            selected_language: &'static str,
            on_disk: Vec<ModelId>,
            want: Option<ModelId>,
        }
        let cases = [
            Case {
                name: "prefers the downloaded recommended model",
                system: true,
                selected_language: "en",
                on_disk: vec![ParakeetTdtV2],
                want: Some(ParakeetTdtV2),
            },
            Case {
                name: "uses first downloaded when recommended missing",
                system: true,
                selected_language: "en",
                on_disk: vec![ParakeetUnified],
                want: Some(ParakeetUnified),
            },
            Case {
                name: "built-in counts as downloaded",
                system: true,
                selected_language: "de",
                on_disk: vec![],
                want: Some(SystemSpeech),
            },
            Case {
                name: "nothing downloaded",
                system: false,
                selected_language: "en",
                on_disk: vec![],
                want: None,
            },
            Case {
                name: "LANG-5 no recommendation uses first downloaded",
                system: false,
                selected_language: "ko",
                on_disk: vec![ParakeetJapanese],
                want: Some(ParakeetJapanese),
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
                declined_fallback(&catalog, lang(tc.selected_language), &downloaded),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn declined_load_dl_14() {
        let declined = SelectionResolution::NeedsDownload(migration("old", ParakeetUltra));
        struct Case {
            name: &'static str,
            declined_key: &'static str,
            on_disk: Vec<ModelId>,
            want: Option<DeclinedLoad>,
        }
        let cases = [
            Case {
                name: "not declined",
                declined_key: "",
                on_disk: vec![ParakeetTdtV2],
                want: None,
            },
            Case {
                name: "declined with fallback",
                declined_key: "old->parakeet-ultra",
                on_disk: vec![ParakeetTdtV2],
                want: Some(DeclinedLoad::LoadFallback(ParakeetTdtV2)),
            },
            Case {
                name: "declined without fallback opens speech settings",
                declined_key: "old->parakeet-ultra",
                on_disk: vec![],
                want: Some(DeclinedLoad::OpenSpeechSettings),
            },
        ];
        let catalog = without_system();
        for tc in cases {
            let downloaded = catalog.downloaded(&tc.on_disk);
            let got = declined_load(
                &catalog,
                &declined,
                tc.declined_key,
                lang("en"),
                &downloaded,
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn notice_copy_dl_14() {
        let copy = notice_copy(&with_system(), &migration("old", ParakeetUltra));
        assert_eq!(copy.title, "Onda was updated");
        assert_eq!(
            copy.body,
            "Your previous speech model is no longer available, so we've switched you to **Parakeet Ultra**."
        );
        let buttons: Vec<(&str, NoticeChoice, Option<NoticeShortcut>)> = copy
            .buttons
            .iter()
            .map(|b| (b.label, b.choice, b.shortcut))
            .collect();
        assert_eq!(
            buttons,
            [
                (
                    "Not Now",
                    NoticeChoice::NotNow,
                    Some(NoticeShortcut::Escape)
                ),
                ("Choose Another", NoticeChoice::ChooseAnother, None),
                (
                    "Download",
                    NoticeChoice::Download,
                    Some(NoticeShortcut::Return)
                ),
            ]
        );
    }

    #[test]
    fn respond_to_notice_dl_14() {
        struct Case {
            name: &'static str,
            choice: NoticeChoice,
            want: NoticeResponse,
        }
        let key = "old->parakeet-ultra".to_string();
        let cases = [
            Case {
                name: "download selects and downloads the replacement",
                choice: NoticeChoice::Download,
                want: NoticeResponse {
                    last_shown_key: key.clone(),
                    declined_key: None,
                    action: NoticeAction::DownloadReplacement(ParakeetUltra),
                },
            },
            Case {
                name: "choose another opens speech settings",
                choice: NoticeChoice::ChooseAnother,
                want: NoticeResponse {
                    last_shown_key: key.clone(),
                    declined_key: None,
                    action: NoticeAction::OpenSpeechSettings,
                },
            },
            Case {
                name: "not now declines and loads the fallback",
                choice: NoticeChoice::NotNow,
                want: NoticeResponse {
                    last_shown_key: key.clone(),
                    declined_key: Some(key.clone()),
                    action: NoticeAction::LoadDeclinedFallback,
                },
            },
        ];
        for tc in cases {
            assert_eq!(
                respond_to_notice(&migration("old", ParakeetUltra), tc.choice),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn removed_download_folders_dl_15() {
        struct Case {
            name: &'static str,
            folders: Vec<&'static str>,
            removed: &'static [(&'static str, ModelId)],
            want: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "removed folders selected, current kept",
                folders: vec!["parakeet-tdt-v2", "parakeet-tdt-v3", "nemotron"],
                removed: TEST_REMOVED,
                want: vec!["parakeet-tdt-v3", "nemotron"],
            },
            Case {
                name: "unexpected folders are left alone",
                folders: vec!["troy-and-abed", ".DS_Store"],
                removed: TEST_REMOVED,
                want: vec![],
            },
            Case {
                name: "a current id in the map is never deleted",
                folders: vec!["parakeet-ultra"],
                removed: &[("parakeet-ultra", ParakeetTdtV2)],
                want: vec![],
            },
            Case {
                name: "empty production map deletes nothing",
                folders: vec!["parakeet-tdt-v3"],
                removed: REMOVED_MODEL_IDS,
                want: vec![],
            },
        ];
        for tc in cases {
            assert_eq!(
                removed_download_folders(&tc.folders, tc.removed),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn unknown_id_load_target_asr_4() {
        struct Case {
            name: &'static str,
            system: bool,
            stored: &'static str,
            default_language: &'static str,
            want: LoadTarget,
        }
        let cases = [
            Case {
                name: "known id loads as is",
                system: true,
                stored: "parakeet-ja",
                default_language: "en",
                want: LoadTarget {
                    model: ParakeetJapanese,
                    save_selection: false,
                },
            },
            Case {
                name: "unknown id falls back and saves",
                system: true,
                stored: "greendale-human-being",
                default_language: "en",
                want: LoadTarget {
                    model: ParakeetTdtV2,
                    save_selection: true,
                },
            },
            Case {
                name: "unknown id with auto default and system engine",
                system: true,
                stored: "greendale-human-being",
                default_language: "auto",
                want: LoadTarget {
                    model: SystemSpeech,
                    save_selection: true,
                },
            },
            Case {
                name: "system speech without engine falls back",
                system: false,
                stored: "system-speech",
                default_language: "auto",
                want: LoadTarget {
                    model: ParakeetUltra,
                    save_selection: true,
                },
            },
        ];
        for tc in cases {
            let catalog = if tc.system {
                with_system()
            } else {
                without_system()
            };
            assert_eq!(
                load_target(&catalog, tc.stored, lang(tc.default_language)),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
