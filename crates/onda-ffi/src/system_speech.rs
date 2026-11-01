//! The optional `SystemSpeech` port as a UniFFI foreign trait, adapted to
//! `onda-app`'s [`SystemSpeech`] (SYS-1..SYS-7). The adapter
//! only converts types and bridges progress; the core decides locales,
//! copy, and error handling.

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;

use onda_app::ports::{SystemSpeech, SystemSpeechError};
use onda_domain::models::SystemSpeechInfo;

/// Catalog facts the shell supplies for its speech engine (MOD-4, SYS-1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SystemSpeechDetails {
    /// E.g. "Built-in macOS speech recognition — no download required".
    pub description: String,
    /// Language codes the engine supports (SYS-1).
    pub languages: Vec<String>,
    /// E.g. "Apple System".
    pub license_name: String,
    /// E.g. "https://www.apple.com/legal/sla/".
    pub license_url: String,
    /// E.g. "Apple Speech Framework".
    pub attribution: String,
}

impl From<SystemSpeechDetails> for SystemSpeechInfo {
    fn from(details: SystemSpeechDetails) -> Self {
        Self {
            description: details.description,
            languages: details.languages,
            license_name: details.license_name,
            license_url: details.license_url,
            attribution: details.attribution,
        }
    }
}

/// Why a system speech call failed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum SpeechError {
    /// The OS has no recognizer for the locale (SYS-6).
    UnsupportedLocale {
        /// The OS's localized locale name, e.g. "Polish (Poland)".
        display_name: String,
    },
    /// The OS work was cancelled.
    Cancelled,
    /// Anything else; never transcript text.
    Failed {
        /// What went wrong, for the log.
        message: String,
    },
}

impl fmt::Display for SpeechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLocale { display_name } => {
                write!(f, "unsupported locale {display_name}")
            }
            Self::Cancelled => f.write_str("cancelled"),
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for SpeechError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for SpeechError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

impl From<SpeechError> for SystemSpeechError {
    fn from(error: SpeechError) -> Self {
        match error {
            SpeechError::UnsupportedLocale { display_name } => {
                Self::UnsupportedLocale { display_name }
            }
            SpeechError::Cancelled => Self::Cancelled,
            SpeechError::Failed { message } => Self::Failed { detail: message },
        }
    }
}

/// Where the shell reports a language-asset download while
/// [`SystemSpeechPort::prepare`] runs. Calls after `prepare` returned are
/// ignored.
#[derive(uniffi::Object)]
pub struct SpeechProgress {
    sender: Sender<Prepare>,
}

#[uniffi::export]
impl SpeechProgress {
    /// The download's fraction in `[0, 1]`.
    pub fn report(&self, fraction: f64) {
        // the receiver is gone once prepare returned
        let _ = self.sender.send(Prepare::Progress(fraction));
    }
}

/// The platform speech recognizer (macOS: the Speech framework). The core
/// calls `prepare` and `transcribe` from its own background threads, never
/// the main thread, and they block until the OS finishes.
#[uniffi::export(with_foreign)]
pub trait SystemSpeechPort: Send + Sync {
    /// Catalog facts; constant for the run.
    fn details(&self) -> SystemSpeechDetails;

    /// The OS locale identifier, read fresh (SYS-2).
    fn current_locale(&self) -> String;

    /// Gets `locale` ready, installing its speech assets if needed and
    /// reporting that download to `progress` (SYS-4).
    fn prepare(&self, locale: String, progress: Arc<SpeechProgress>) -> Result<(), SpeechError>;

    /// Transcribes 16 kHz mono `samples` in the prepared `locale`; returns
    /// the final results in order (SYS-7).
    fn transcribe(&self, samples: Vec<f32>, locale: String) -> Result<Vec<String>, SpeechError>;
}

enum Prepare {
    Progress(f64),
    Done(Result<(), SpeechError>),
}

/// The shell's [`SystemSpeechPort`] behind `onda-app`'s [`SystemSpeech`].
pub(crate) struct ShellSpeech(pub(crate) Arc<dyn SystemSpeechPort>);

impl SystemSpeech for ShellSpeech {
    fn info(&self) -> SystemSpeechInfo {
        self.0.details().into()
    }

    fn os_locale(&self) -> String {
        self.0.current_locale()
    }

    // the shell reports progress from its own threads while this one waits,
    // so the port runs on a scoped thread and this one relays progress to
    // the borrowed callback
    fn prepare(
        &self,
        locale: &str,
        progress: &mut dyn FnMut(f64),
    ) -> Result<(), SystemSpeechError> {
        let (sender, events) = mpsc::channel();
        let reporter = Arc::new(SpeechProgress {
            sender: sender.clone(),
        });
        let port = self.0.clone();
        let locale = locale.to_owned();
        thread::scope(|scope| {
            thread::Builder::new()
                .name("onda-system-speech".to_owned())
                .spawn_scoped(scope, move || {
                    let result = port.prepare(locale, reporter);
                    let _ = sender.send(Prepare::Done(result));
                })
                .map_err(|error| SystemSpeechError::Failed {
                    detail: format!("could not start the prepare thread: {error}"),
                })?;
            for event in events {
                match event {
                    Prepare::Progress(fraction) => progress(fraction),
                    Prepare::Done(result) => return result.map_err(SystemSpeechError::from),
                }
            }
            Err(SystemSpeechError::Failed {
                detail: "prepare ended without a result".to_owned(),
            })
        })
    }

    fn transcribe(&self, samples: &[f32], locale: &str) -> Result<Vec<String>, SystemSpeechError> {
        self.0
            .transcribe(samples.to_vec(), locale.to_owned())
            .map_err(SystemSpeechError::from)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Reports progress from another thread, as the Speech framework does.
    struct Port {
        result: Mutex<Option<Result<(), SpeechError>>>,
    }

    impl SystemSpeechPort for Port {
        fn details(&self) -> SystemSpeechDetails {
            SystemSpeechDetails {
                description: "Built-in Greendale speech".to_owned(),
                languages: vec!["ko".to_owned()],
                license_name: "Greendale".to_owned(),
                license_url: "https://greendale.example".to_owned(),
                attribution: "Greendale Speech".to_owned(),
            }
        }

        fn current_locale(&self) -> String {
            "en_US".to_owned()
        }

        fn prepare(&self, _: String, progress: Arc<SpeechProgress>) -> Result<(), SpeechError> {
            let reporter = thread::spawn(move || {
                progress.report(0.25);
                progress.report(1.0);
            });
            reporter.join().map_err(|_| SpeechError::Cancelled)?;
            let result = self.result.lock().ok().and_then(|mut r| r.take());
            result.unwrap_or(Ok(()))
        }

        fn transcribe(&self, _: Vec<f32>, _: String) -> Result<Vec<String>, SpeechError> {
            Ok(vec!["Cool.".to_owned()])
        }
    }

    #[test]
    fn prepare_relays_progress_and_maps_errors() {
        struct Case {
            name: &'static str,
            result: Result<(), SpeechError>,
            want: Result<(), SystemSpeechError>,
        }
        let cases = [
            Case {
                name: "ready",
                result: Ok(()),
                want: Ok(()),
            },
            Case {
                name: "unsupported locale",
                result: Err(SpeechError::UnsupportedLocale {
                    display_name: "Korean (South Korea)".to_owned(),
                }),
                want: Err(SystemSpeechError::UnsupportedLocale {
                    display_name: "Korean (South Korea)".to_owned(),
                }),
            },
            Case {
                name: "cancelled",
                result: Err(SpeechError::Cancelled),
                want: Err(SystemSpeechError::Cancelled),
            },
            Case {
                name: "failed",
                result: Err(SpeechError::Failed {
                    message: "asset server unreachable".to_owned(),
                }),
                want: Err(SystemSpeechError::Failed {
                    detail: "asset server unreachable".to_owned(),
                }),
            },
        ];
        for tc in cases {
            let speech = ShellSpeech(Arc::new(Port {
                result: Mutex::new(Some(tc.result)),
            }));
            let mut fractions = Vec::new();
            let got = speech.prepare("ko-KR", &mut |fraction| fractions.push(fraction));
            assert_eq!(got, tc.want, "{}", tc.name);
            assert_eq!(fractions, vec![0.25, 1.0], "{}", tc.name);
        }
    }
}
