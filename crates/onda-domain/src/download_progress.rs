//! Download progress publishing, status text, and failure reporting
//! (DL-2, DL-4).

use crate::messages::UserMessage;
use crate::routing::percent;
use crate::types::MonoTime;
use std::time::Duration;

/// Minimum spacing between progress events, except 100% (DL-2).
pub const PUBLISH_INTERVAL: Duration = Duration::from_millis(100);

/// Filters one download's raw progress reports into events (DL-2):
/// monotonic, clamped to `[0, 1]`, at most every [`PUBLISH_INTERVAL`], and
/// 100% always publishes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProgressThrottle {
    published: f64,
    last_publish: Option<MonoTime>,
}

impl ProgressThrottle {
    /// A throttle for a download that just started at 0%.
    pub fn new() -> ProgressThrottle {
        ProgressThrottle::default()
    }

    /// The last published fraction.
    pub fn published(&self) -> f64 {
        self.published
    }

    /// Feeds a raw report. Returns the fraction to publish, or `None` to drop
    /// it (backwards, NaN, or too soon).
    pub fn accept(&mut self, incoming: f64, now: MonoTime) -> Option<f64> {
        if incoming.is_nan() {
            return None;
        }
        let progress = incoming.clamp(0.0, 1.0);
        if progress < self.published {
            return None;
        }
        let too_soon = self
            .last_publish
            .is_some_and(|last| now.saturating_sub(last) < PUBLISH_INTERVAL);
        if progress < 1.0 && too_soon {
            return None;
        }
        self.published = progress;
        self.last_publish = Some(now);
        Some(progress)
    }
}

/// Phase of a download for its status text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DownloadStatus {
    /// Started or progressing; fraction in `[0, 1]`.
    Downloading(f64),
    /// Finished.
    Finished,
    /// Cancelled by the user.
    Cancelled,
    /// Failed for real.
    Failed,
}

/// Status text naming the model (DL-2).
pub fn status_text(model_name: &str, status: DownloadStatus) -> String {
    match status {
        DownloadStatus::Downloading(progress) => {
            format!("Downloading {model_name}... {}%", percent(progress))
        }
        DownloadStatus::Finished => format!("Downloaded {model_name}"),
        DownloadStatus::Cancelled => format!("Download cancelled: {model_name}"),
        DownloadStatus::Failed => format!("Download failed: {model_name}"),
    }
}

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEnd {
    /// Finished.
    Finished,
    /// Cancelled by the user (partial data kept, DL-3).
    Cancelled,
    /// Failed after the downloader's own retries.
    Failed {
        /// Error description, if any.
        error: Option<String>,
    },
}

/// Title of the Speech page alert for a failed download (DL-4).
pub const DOWNLOAD_ALERT_TITLE: &str = "Error";

/// Body shared by the Speech page alert and the notification (DL-4).
pub fn download_failed_body(model_name: &str, error: &str) -> String {
    format!("Failed to download {model_name}: {error}")
}

/// Where a download failure is reported (DL-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureReport {
    /// Speech page visible: queue an alert there titled
    /// [`DOWNLOAD_ALERT_TITLE`] with this body.
    Alert(String),
    /// Otherwise: post a non-critical notification.
    Notification(UserMessage),
}

/// Decides the failure report (DL-4). `None` unless the download failed;
/// cancellations are not reported. A missing error description falls back
/// to the status text.
pub fn failure_report(
    model_name: &str,
    end: &DownloadEnd,
    speech_page_visible: bool,
) -> Option<FailureReport> {
    let DownloadEnd::Failed { error } = end else {
        return None;
    };
    let error = error
        .clone()
        .unwrap_or_else(|| status_text(model_name, DownloadStatus::Failed));
    if speech_page_visible {
        return Some(FailureReport::Alert(download_failed_body(
            model_name, &error,
        )));
    }
    Some(FailureReport::Notification(
        UserMessage::ModelDownloadFailed {
            model: model_name.to_owned(),
            error,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> MonoTime {
        Duration::from_millis(n)
    }

    #[test]
    fn throttle_dl_2() {
        struct Case {
            name: &'static str,
            reports: Vec<(f64, u64)>,
            want: Vec<Option<f64>>,
        }
        let cases = [
            Case {
                name: "first report publishes",
                reports: vec![(0.1, 5)],
                want: vec![Some(0.1)],
            },
            Case {
                name: "drops backward reports",
                reports: vec![(0.98, 0), (0.97, 500)],
                want: vec![Some(0.98), None],
            },
            Case {
                name: "clamps forward reports",
                reports: vec![(0.98, 0), (1.2, 500)],
                want: vec![Some(0.98), Some(1.0)],
            },
            Case {
                name: "clamps negative reports",
                reports: vec![(-0.5, 0)],
                want: vec![Some(0.0)],
            },
            Case {
                name: "at most every 100 ms",
                reports: vec![(0.1, 0), (0.2, 50), (0.3, 99), (0.4, 100), (0.5, 150)],
                want: vec![Some(0.1), None, None, Some(0.4), None],
            },
            Case {
                name: "100% always publishes",
                reports: vec![(0.5, 0), (1.0, 10)],
                want: vec![Some(0.5), Some(1.0)],
            },
            Case {
                name: "throttled report does not raise the floor",
                reports: vec![(0.5, 0), (0.9, 10), (0.6, 120)],
                want: vec![Some(0.5), None, Some(0.6)],
            },
            Case {
                name: "nan dropped",
                reports: vec![(f64::NAN, 0), (0.2, 1)],
                want: vec![None, Some(0.2)],
            },
        ];
        for tc in cases {
            let mut throttle = ProgressThrottle::new();
            let got: Vec<Option<f64>> = tc
                .reports
                .iter()
                .map(|&(progress, at)| throttle.accept(progress, ms(at)))
                .collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn status_text_names_the_model_dl_2() {
        struct Case {
            name: &'static str,
            status: DownloadStatus,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "started",
                status: DownloadStatus::Downloading(0.0),
                want: "Downloading Parakeet Ultra... 0%",
            },
            Case {
                name: "progress truncates",
                status: DownloadStatus::Downloading(0.429),
                want: "Downloading Parakeet Ultra... 42%",
            },
            Case {
                name: "finished",
                status: DownloadStatus::Finished,
                want: "Downloaded Parakeet Ultra",
            },
            Case {
                name: "cancelled",
                status: DownloadStatus::Cancelled,
                want: "Download cancelled: Parakeet Ultra",
            },
            Case {
                name: "failed",
                status: DownloadStatus::Failed,
                want: "Download failed: Parakeet Ultra",
            },
        ];
        for tc in cases {
            assert_eq!(
                status_text("Parakeet Ultra", tc.status),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn failure_report_dl_4() {
        struct Case {
            name: &'static str,
            end: DownloadEnd,
            speech_page_visible: bool,
            want: Option<FailureReport>,
        }
        let notification = |error: &str| {
            FailureReport::Notification(UserMessage::ModelDownloadFailed {
                model: "Parakeet Ultra".to_owned(),
                error: error.to_owned(),
            })
        };
        let offline = || DownloadEnd::Failed {
            error: Some("The Internet connection appears to be offline.".to_string()),
        };
        let cases = [
            Case {
                name: "speech page visible shows an alert",
                end: offline(),
                speech_page_visible: true,
                want: Some(FailureReport::Alert(
                    "Failed to download Parakeet Ultra: The Internet connection appears to be offline."
                        .to_owned(),
                )),
            },
            Case {
                name: "other page or closed window posts a notification",
                end: offline(),
                speech_page_visible: false,
                want: Some(notification(
                    "The Internet connection appears to be offline.",
                )),
            },
            Case {
                name: "missing description falls back to status text",
                end: DownloadEnd::Failed { error: None },
                speech_page_visible: false,
                want: Some(notification("Download failed: Parakeet Ultra")),
            },
            Case {
                name: "cancellation not reported",
                end: DownloadEnd::Cancelled,
                speech_page_visible: true,
                want: None,
            },
            Case {
                name: "success not reported",
                end: DownloadEnd::Finished,
                speech_page_visible: false,
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(
                failure_report("Parakeet Ultra", &tc.end, tc.speech_page_visible),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
