//! The model update notice window (DL-14).

use onda_domain::migration::{Migration, NoticeChoice, NoticeShortcut, notice_copy};
use onda_domain::models::Catalog;

/// One notice button, left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeButtonView {
    /// Title.
    pub title: String,
    /// Pass to `update_notice_choice`.
    pub choice: NoticeChoice,
    /// Escape or Return; Return is also the default, prominent button.
    pub shortcut: Option<NoticeShortcut>,
}

/// The update notice view model. Closing the window counts as "Not Now".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotice {
    /// "Onda was updated".
    pub title: String,
    /// Selectable body; the new model name is Markdown bold (`**...**`).
    pub body: String,
    /// "Not Now", "Choose Another", "Download".
    pub buttons: Vec<NoticeButtonView>,
}

/// Builds the notice for `migration`.
pub fn update_notice(catalog: &Catalog, migration: &Migration) -> UpdateNotice {
    let copy = notice_copy(catalog, migration);
    UpdateNotice {
        title: copy.title,
        body: copy.body,
        buttons: copy
            .buttons
            .iter()
            .map(|button| NoticeButtonView {
                title: button.label.to_owned(),
                choice: button.choice,
                shortcut: button.shortcut,
            })
            .collect(),
    }
}
