//! What Onda shows outside a dictation session: the OVL-19 resolver, the
//! download tracker that keeps the overlay from jumping between downloads,
//! and the download overlay view model (OVL-16 to OVL-18).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::types::ModelState;

/// Status text when a load has none of its own (OVL-16, ASR-2).
pub const DEFAULT_LOADING_STATUS: &str = "Loading model...";

/// Download overlay fade duration (OVL-16).
pub const DOWNLOAD_OVERLAY_FADE: Duration = Duration::from_millis(300);

/// Accessibility label of the download overlay's close button (OVL-17).
pub const HIDE_ACCESSIBILITY_LABEL: &str = "Hide";

/// Tooltip of the download overlay's close button (OVL-17). `tray_term` is
/// the platform's name for the tray icon, "menu bar icon" on macOS (09).
pub fn hide_tooltip(tray_term: &str) -> String {
    format!("Hide — progress stays in the {tray_term}")
}

/// A model download in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDownload {
    /// Catalog id of the model being downloaded.
    pub model_id: String,
    /// Fraction in `[0, 1]`.
    pub progress: f64,
    /// Status line, e.g. "Downloading Parakeet TDT v2... 37%".
    pub status: String,
}

/// What the tray and download overlay show outside a session (OVL-19).
#[derive(Debug, Clone, PartialEq)]
pub enum NonDictationState {
    /// The model is loaded; no overlay.
    Ready,
    /// A download is in progress.
    Downloading(ModelDownload),
    /// The model is loading.
    Loading {
        /// Status line; never empty.
        status: String,
        /// Fraction in `[0, 1]`, or `None` for an indeterminate bar.
        progress: Option<f64>,
    },
    /// No usable model and nothing preparing one; no overlay, error icon.
    Error,
}

/// Resolves the non-dictation state (OVL-19), in priority order: model
/// error, force-loading, a download, ready, not ready (loading only with a
/// load in flight), loading.
///
/// `force_loading` is true when a selected-model download just finished, or
/// a load started at launch and no state has arrived yet.
pub fn resolve(
    model_state: &ModelState,
    download: Option<&ModelDownload>,
    load_in_flight: bool,
    force_loading: bool,
) -> NonDictationState {
    if let ModelState::Error { .. } = model_state {
        return NonDictationState::Error;
    }
    if force_loading {
        return loading(model_state);
    }
    if let Some(download) = download {
        return NonDictationState::Downloading(download.clone());
    }
    match model_state {
        ModelState::Ready => NonDictationState::Ready,
        // do not fake a spinner when nothing is preparing the model
        ModelState::NotReady if load_in_flight => loading(model_state),
        ModelState::NotReady | ModelState::Error { .. } => NonDictationState::Error,
        ModelState::Loading { .. } => loading(model_state),
    }
}

fn loading(model_state: &ModelState) -> NonDictationState {
    match model_state {
        ModelState::Loading { status, progress } if !status.is_empty() => {
            NonDictationState::Loading {
                status: status.clone(),
                progress: *progress,
            }
        }
        ModelState::Loading { progress, .. } => NonDictationState::Loading {
            status: DEFAULT_LOADING_STATUS.to_owned(),
            progress: *progress,
        },
        ModelState::NotReady | ModelState::Ready | ModelState::Error { .. } => {
            NonDictationState::Loading {
                status: DEFAULT_LOADING_STATUS.to_owned(),
                progress: None,
            }
        }
    }
}

/// Tracks running downloads and which one the overlay follows (OVL-19 step
/// 3): the selected model's download, else the one already being followed
/// until it ends, else the first remaining one by id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadTracker {
    downloads: BTreeMap<String, ModelDownload>,
    followed: Option<String>,
}

impl DownloadTracker {
    /// Creates an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// A download started or reported progress.
    pub fn progressed(&mut self, download: ModelDownload) {
        let id = download.model_id.clone();
        self.downloads.insert(id, download);
        if !self.is_followed_running() {
            self.followed = self.first_by_id();
        }
    }

    /// A download finished, failed, or was cancelled.
    pub fn ended(&mut self, model_id: &str) {
        self.downloads.remove(model_id);
        if self.followed.as_deref() == Some(model_id) {
            self.followed = self.first_by_id();
        }
    }

    /// The download to show, given the selected model.
    pub fn current(&self, selected_model_id: Option<&str>) -> Option<&ModelDownload> {
        selected_model_id
            .and_then(|id| self.downloads.get(id))
            .or_else(|| self.followed.as_ref().and_then(|id| self.downloads.get(id)))
            .or_else(|| self.downloads.values().next())
    }

    /// The running download of `model_id`, if any (WIN-19, ONB-8).
    pub fn get(&self, model_id: &str) -> Option<&ModelDownload> {
        self.downloads.get(model_id)
    }

    /// Any download is running (WIN-5 "In progress").
    pub fn any_running(&self) -> bool {
        !self.downloads.is_empty()
    }

    fn is_followed_running(&self) -> bool {
        self.followed
            .as_ref()
            .is_some_and(|id| self.downloads.contains_key(id))
    }

    fn first_by_id(&self) -> Option<String> {
        self.downloads.keys().next().cloned()
    }
}

/// Progress bar of the download overlay (OVL-16).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayProgress {
    /// Fraction in `[0, 1]`.
    Determinate(f64),
    /// Progress unknown.
    Indeterminate,
}

/// The download overlay view model (OVL-16).
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadOverlayView {
    /// Not on screen.
    Hidden,
    /// On screen with a status line and a progress bar.
    Shown {
        /// Status line.
        status: String,
        /// Progress bar.
        progress: OverlayProgress,
    },
}

/// Download overlay visibility, including the user's "hide" (OVL-17, OVL-18).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadOverlay {
    dismissed: bool,
}

impl DownloadOverlay {
    /// Creates an overlay the user has not hidden.
    pub fn new() -> Self {
        Self::default()
    }

    /// The user clicked the close button: stay hidden until the download or
    /// load ends (OVL-17).
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Applies a non-dictation state and returns what to show. Never shown
    /// during a session or while an overlay notice is up (OVL-18). The
    /// user's hide lasts until the state becomes ready or error.
    pub fn apply(
        &mut self,
        state: &NonDictationState,
        session_active: bool,
        notice_up: bool,
    ) -> DownloadOverlayView {
        if session_active {
            return DownloadOverlayView::Hidden;
        }
        let (status, progress) = match state {
            NonDictationState::Ready | NonDictationState::Error => {
                self.dismissed = false;
                return DownloadOverlayView::Hidden;
            }
            NonDictationState::Downloading(download) => (
                download.status.clone(),
                OverlayProgress::Determinate(download.progress),
            ),
            NonDictationState::Loading { status, progress } => (
                status.clone(),
                progress.map_or(OverlayProgress::Indeterminate, OverlayProgress::Determinate),
            ),
        };
        if self.dismissed || notice_up {
            return DownloadOverlayView::Hidden;
        }
        DownloadOverlayView::Shown { status, progress }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download(id: &str) -> ModelDownload {
        ModelDownload {
            model_id: id.to_owned(),
            progress: 0.5,
            status: format!("Downloading {id}... 50%"),
        }
    }

    fn loading_state(status: &str, progress: Option<f64>) -> NonDictationState {
        NonDictationState::Loading {
            status: status.to_owned(),
            progress,
        }
    }

    fn model_loading(status: &str, progress: Option<f64>) -> ModelState {
        ModelState::Loading {
            status: status.to_owned(),
            progress,
        }
    }

    #[test]
    fn ovl_19_resolve() {
        struct Case {
            name: &'static str,
            model: ModelState,
            download: Option<ModelDownload>,
            load_in_flight: bool,
            force_loading: bool,
            want: NonDictationState,
        }
        let cases = [
            Case {
                name: "not ready with a load in flight is loading",
                model: ModelState::NotReady,
                download: None,
                load_in_flight: true,
                force_loading: false,
                want: loading_state("Loading model...", None),
            },
            Case {
                name: "not ready with nothing in flight is an error",
                model: ModelState::NotReady,
                download: None,
                load_in_flight: false,
                force_loading: false,
                want: NonDictationState::Error,
            },
            Case {
                name: "ready without a download",
                model: ModelState::Ready,
                download: None,
                load_in_flight: false,
                force_loading: false,
                want: NonDictationState::Ready,
            },
            Case {
                name: "model error wins over a download",
                model: ModelState::Error {
                    message: "boom".to_owned(),
                },
                download: Some(download("base")),
                load_in_flight: false,
                force_loading: false,
                want: NonDictationState::Error,
            },
            Case {
                name: "model error wins over force-loading",
                model: ModelState::Error {
                    message: "boom".to_owned(),
                },
                download: None,
                load_in_flight: true,
                force_loading: true,
                want: NonDictationState::Error,
            },
            Case {
                name: "download while ready",
                model: ModelState::Ready,
                download: Some(download("base")),
                load_in_flight: false,
                force_loading: false,
                want: NonDictationState::Downloading(download("base")),
            },
            Case {
                name: "download wins over generic loading",
                model: model_loading("Preparing model...", None),
                download: Some(download("base")),
                load_in_flight: true,
                force_loading: false,
                want: NonDictationState::Downloading(download("base")),
            },
            Case {
                name: "force-loading overrides a download",
                model: ModelState::Ready,
                download: Some(download("base")),
                load_in_flight: false,
                force_loading: true,
                want: loading_state("Loading model...", None),
            },
            Case {
                name: "force-loading keeps the load's own progress",
                model: model_loading("Loading weights...", Some(0.3)),
                download: None,
                load_in_flight: true,
                force_loading: true,
                want: loading_state("Loading weights...", Some(0.3)),
            },
            Case {
                name: "loading with progress",
                model: model_loading("Loading text decoder weights...", Some(0.92)),
                download: None,
                load_in_flight: true,
                force_loading: false,
                want: loading_state("Loading text decoder weights...", Some(0.92)),
            },
            Case {
                name: "loading with empty status falls back",
                model: model_loading("", None),
                download: None,
                load_in_flight: true,
                force_loading: false,
                want: loading_state("Loading model...", None),
            },
        ];
        for tc in cases {
            let got = resolve(
                &tc.model,
                tc.download.as_ref(),
                tc.load_in_flight,
                tc.force_loading,
            );
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn ovl_19_download_tracker_does_not_jump() {
        enum Event {
            Progress(&'static str),
            End(&'static str),
        }
        struct Case {
            name: &'static str,
            events: Vec<Event>,
            selected: Option<&'static str>,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "nothing running",
                events: vec![],
                selected: None,
                want: None,
            },
            Case {
                name: "selected model's download wins",
                events: vec![Event::Progress("alpha"), Event::Progress("zulu")],
                selected: Some("zulu"),
                want: Some("zulu"),
            },
            Case {
                name: "keeps following the first download when a lower id starts",
                events: vec![
                    Event::Progress("mike"),
                    Event::Progress("alpha"),
                    Event::Progress("mike"),
                ],
                selected: None,
                want: Some("mike"),
            },
            Case {
                name: "followed download ends, next by id",
                events: vec![
                    Event::Progress("mike"),
                    Event::Progress("zulu"),
                    Event::Progress("bravo"),
                    Event::End("mike"),
                ],
                selected: None,
                want: Some("bravo"),
            },
            Case {
                name: "selected model not downloading falls back to followed",
                events: vec![Event::Progress("mike"), Event::Progress("alpha")],
                selected: Some("parakeet-tdt-v2"),
                want: Some("mike"),
            },
            Case {
                name: "all ended",
                events: vec![Event::Progress("mike"), Event::End("mike")],
                selected: None,
                want: None,
            },
        ];
        for tc in cases {
            let mut tracker = DownloadTracker::new();
            for event in tc.events {
                match event {
                    Event::Progress(id) => tracker.progressed(download(id)),
                    Event::End(id) => tracker.ended(id),
                }
            }
            let got = tracker.current(tc.selected).map(|d| d.model_id.as_str());
            assert_eq!(got, tc.want, "{}", tc.name);
            assert_eq!(tracker.any_running(), tc.want.is_some(), "{}", tc.name);
            if let Some(id) = tc.want {
                assert_eq!(
                    tracker.get(id).map(|d| d.model_id.as_str()),
                    Some(id),
                    "{}",
                    tc.name
                );
            }
        }
    }

    #[test]
    fn tracker_get_reports_only_running_downloads() {
        let mut tracker = DownloadTracker::new();
        tracker.progressed(download("alpha"));
        tracker.progressed(download("bravo"));
        tracker.ended("alpha");
        let cases = [("ended", "alpha", false), ("running", "bravo", true)];
        for (name, id, want) in cases {
            assert_eq!(tracker.get(id).is_some(), want, "{name}");
        }
    }

    #[test]
    fn ovl_16_to_18_download_overlay() {
        struct Case {
            name: &'static str,
            state: NonDictationState,
            session_active: bool,
            notice_up: bool,
            want: DownloadOverlayView,
        }
        let shown = |status: &str, progress| DownloadOverlayView::Shown {
            status: status.to_owned(),
            progress,
        };
        let cases = [
            Case {
                name: "download shows determinate progress",
                state: NonDictationState::Downloading(download("base")),
                session_active: false,
                notice_up: false,
                want: shown("Downloading base... 50%", OverlayProgress::Determinate(0.5)),
            },
            Case {
                name: "load without progress is indeterminate",
                state: loading_state("Loading model...", None),
                session_active: false,
                notice_up: false,
                want: shown("Loading model...", OverlayProgress::Indeterminate),
            },
            Case {
                name: "load with progress",
                state: loading_state("Loading weights...", Some(0.25)),
                session_active: false,
                notice_up: false,
                want: shown("Loading weights...", OverlayProgress::Determinate(0.25)),
            },
            Case {
                name: "ready hides",
                state: NonDictationState::Ready,
                session_active: false,
                notice_up: false,
                want: DownloadOverlayView::Hidden,
            },
            Case {
                name: "error hides",
                state: NonDictationState::Error,
                session_active: false,
                notice_up: false,
                want: DownloadOverlayView::Hidden,
            },
            Case {
                name: "never during a session",
                state: NonDictationState::Downloading(download("base")),
                session_active: true,
                notice_up: false,
                want: DownloadOverlayView::Hidden,
            },
            Case {
                name: "never while a notice is up",
                state: NonDictationState::Downloading(download("base")),
                session_active: false,
                notice_up: true,
                want: DownloadOverlayView::Hidden,
            },
        ];
        for tc in cases {
            let got = DownloadOverlay::new().apply(&tc.state, tc.session_active, tc.notice_up);
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn ovl_17_hidden_until_download_or_load_ends() {
        struct Case {
            name: &'static str,
            after_dismiss: Vec<(NonDictationState, bool)>,
            want_last_shown: bool,
        }
        let downloading = || NonDictationState::Downloading(download("base"));
        let cases = [
            Case {
                name: "stays hidden through progress",
                after_dismiss: vec![(downloading(), false), (downloading(), false)],
                want_last_shown: false,
            },
            Case {
                name: "stays hidden from download into load",
                after_dismiss: vec![(loading_state("Loading model...", None), false)],
                want_last_shown: false,
            },
            Case {
                name: "a session does not clear the hide",
                after_dismiss: vec![(downloading(), true), (downloading(), false)],
                want_last_shown: false,
            },
            Case {
                name: "ready ends it, next download shows",
                after_dismiss: vec![(NonDictationState::Ready, false), (downloading(), false)],
                want_last_shown: true,
            },
            Case {
                name: "error ends it, next download shows",
                after_dismiss: vec![(NonDictationState::Error, false), (downloading(), false)],
                want_last_shown: true,
            },
        ];
        for tc in cases {
            let mut overlay = DownloadOverlay::new();
            assert_ne!(
                overlay.apply(&downloading(), false, false),
                DownloadOverlayView::Hidden,
                "{}",
                tc.name
            );
            overlay.dismiss();
            let mut last = DownloadOverlayView::Hidden;
            for (state, session_active) in &tc.after_dismiss {
                last = overlay.apply(state, *session_active, false);
            }
            assert_eq!(
                last != DownloadOverlayView::Hidden,
                tc.want_last_shown,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn ovl_17_hide_copy() {
        struct Case {
            name: &'static str,
            term: &'static str,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "macOS",
                term: "menu bar icon",
                want: "Hide — progress stays in the menu bar icon",
            },
            Case {
                name: "Windows",
                term: "tray icon",
                want: "Hide — progress stays in the tray icon",
            },
        ];
        for tc in cases {
            assert_eq!(hide_tooltip(tc.term), tc.want, "{}", tc.name);
        }
    }
}
