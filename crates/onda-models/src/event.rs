use onda_domain::download_progress::{DownloadEnd, DownloadStatus};
use onda_domain::models::ModelId;

/// One download event, delivered in order on the manager's channel.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadEvent {
    /// The model being downloaded.
    pub model: ModelId,
    /// What happened.
    pub kind: DownloadEventKind,
}

/// What happened to a download.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadEventKind {
    /// The download task started (or resumed).
    Started,
    /// Fraction of the total bytes on disk, filtered by
    /// [`onda_domain::download_progress::ProgressThrottle`] (DL-2).
    Progress(f64),
    /// Every file is on disk and verified.
    Finished,
    /// Cancelled by the user; partial data is kept (DL-3).
    Cancelled,
    /// Failed after retries; the partial folder is gone (DL-4).
    Failed {
        /// User-facing error description.
        error: String,
    },
}

impl DownloadEvent {
    /// The phase for [`onda_domain::download_progress::status_text`].
    pub fn status(&self) -> DownloadStatus {
        match &self.kind {
            DownloadEventKind::Started => DownloadStatus::Downloading(0.0),
            DownloadEventKind::Progress(progress) => DownloadStatus::Downloading(*progress),
            DownloadEventKind::Finished => DownloadStatus::Finished,
            DownloadEventKind::Cancelled => DownloadStatus::Cancelled,
            DownloadEventKind::Failed { .. } => DownloadStatus::Failed,
        }
    }

    /// How the download ended, for
    /// [`onda_domain::download_progress::failure_report`]; `None` while it
    /// runs.
    pub fn end(&self) -> Option<DownloadEnd> {
        match &self.kind {
            DownloadEventKind::Started | DownloadEventKind::Progress(_) => None,
            DownloadEventKind::Finished => Some(DownloadEnd::Finished),
            DownloadEventKind::Cancelled => Some(DownloadEnd::Cancelled),
            DownloadEventKind::Failed { error } => Some(DownloadEnd::Failed {
                error: Some(error.clone()),
            }),
        }
    }
}
