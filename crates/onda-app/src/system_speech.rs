//! The system speech engine (SYS-1..SYS-7): a [`SpeechEngine`] over the
//! shell's [`SystemSpeech`] port, so the transcription service loads,
//! gates, and decodes it like any downloaded model. It downloads nothing
//! Onda manages, so nothing verifies its files.

use std::sync::Arc;

use onda_asr::{AsrError, LoadProgress, SpeechEngine};
use onda_domain::languages::{
    Language, LanguageSetting, system_speech_decode_locale, system_speech_locale,
    system_speech_unsupported_locale,
};

use crate::ports::{SystemSpeech, SystemSpeechError};
use crate::seams::SettingsSource;

/// Load status while the OS downloads a language asset (SYS-4).
pub const DOWNLOADING_STATUS: &str = "Downloading speech model...";

/// [`SpeechEngine`] for `system-speech`.
///
/// Load prepares the selected language, or the OS locale for `auto`
/// (SYS-2). A decode in another language re-prepares first (SYS-5); a
/// decode in `auto` keeps the locale prepared at load.
pub struct SystemSpeechEngine {
    port: Arc<dyn SystemSpeech>,
    settings: Arc<dyn SettingsSource>,
    // the locale the last successful prepare was for; none while unloaded
    prepared: Option<String>,
}

impl SystemSpeechEngine {
    /// An unloaded engine over `port`. `settings` supplies the selected
    /// language at load time.
    pub fn new(port: Arc<dyn SystemSpeech>, settings: Arc<dyn SettingsSource>) -> Self {
        Self {
            port,
            settings,
            prepared: None,
        }
    }
}

impl SpeechEngine for SystemSpeechEngine {
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        if self.prepared.is_some() {
            return Ok(());
        }
        let language = self.settings.snapshot().language;
        let locale = system_speech_locale(language, &self.port.os_locale());
        let mut report = |fraction: f64| {
            progress(LoadProgress {
                status: DOWNLOADING_STATUS.to_owned(),
                fraction,
            });
        };
        self.port
            .prepare(&locale, &mut report)
            .map_err(load_error)?;
        self.prepared = Some(locale);
        Ok(())
    }

    fn unload(&mut self) {
        self.prepared = None;
    }

    fn supports_preview(&self) -> bool {
        false
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        let Some(prepared) = self.prepared.clone() else {
            return Err(AsrError::NotReady);
        };
        let language = language.map_or(LanguageSetting::Auto, LanguageSetting::Specific);
        // something is always prepared here, so the OS locale is never read
        let locale = system_speech_decode_locale(language, Some(&prepared), &prepared);
        if locale != prepared {
            // SYS-5: an asset download now happens inside this decode,
            // without progress
            self.port
                .prepare(&locale, &mut |_| {})
                .map_err(reprepare_error)?;
            self.prepared = Some(locale.clone());
        }
        let results = self
            .port
            .transcribe(samples, &locale)
            .map_err(decode_error)?;
        Ok(join_results(&results))
    }

    fn preview(&mut self, _: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        // SYS-7: never called, since the engine has no preview
        Ok(String::new())
    }

    fn reset_preview(&mut self) {}
}

fn load_error(error: SystemSpeechError) -> AsrError {
    match error {
        SystemSpeechError::UnsupportedLocale { display_name } => unsupported(&display_name),
        SystemSpeechError::Cancelled => AsrError::Cancelled,
        SystemSpeechError::Failed { detail } => AsrError::LoadFailed { detail },
    }
}

// a decode's re-prepare names the locale it cannot serve, as Jabber does
fn reprepare_error(error: SystemSpeechError) -> AsrError {
    match error {
        SystemSpeechError::UnsupportedLocale { display_name } => unsupported(&display_name),
        error => decode_error(error),
    }
}

// SYS-7: every decode failure but cancellation is logged and reported as
// "Transcription failed"
fn decode_error(error: SystemSpeechError) -> AsrError {
    let detail = match error {
        SystemSpeechError::Cancelled => return AsrError::Cancelled,
        SystemSpeechError::UnsupportedLocale { display_name } => {
            format!("unsupported locale {display_name}")
        }
        SystemSpeechError::Failed { detail } => detail,
    };
    tracing::error!(%detail, "system speech decode failed");
    AsrError::TranscriptionFailed { detail }
}

// SYS-6
fn unsupported(display_name: &str) -> AsrError {
    AsrError::Unsupported {
        message: system_speech_unsupported_locale(display_name),
    }
}

// SYS-7: final results joined with single spaces; the whole is trimmed
// (ASR-13). (Deviation: no space where either side is written without
// spaces, so Chinese reads "你好，欢迎" rather than "你好 ，欢迎"; Jabber
// and SYS-7 always insert one.)
fn join_results(results: &[String]) -> String {
    let mut joined = String::new();
    for result in results.iter().map(|result| result.trim()) {
        if result.is_empty() {
            continue;
        }
        let spaced = match (joined.chars().last(), result.chars().next()) {
            (Some(before), Some(after)) => !unspaced(before) && !unspaced(after),
            _ => false,
        };
        if spaced {
            joined.push(' ');
        }
        joined.push_str(result);
    }
    joined
}

// characters of scripts and punctuation that take no space around them:
// CJK punctuation, kana, Han, and full-width forms. Hangul and Thai (which
// separates phrases with spaces) stay spaced.
fn unspaced(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF00}'..='\u{FF65}'
            | '\u{20000}'..='\u{2FA1F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_join_with_single_spaces_sys_7() {
        struct Case {
            name: &'static str,
            results: Vec<&'static str>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "none",
                results: vec![],
                want: "",
            },
            Case {
                name: "one",
                results: vec!["Troy and Abed in the morning."],
                want: "Troy and Abed in the morning.",
            },
            Case {
                name: "several",
                results: vec!["Cool.", "Cool cool cool."],
                want: "Cool. Cool cool cool.",
            },
            Case {
                name: "chinese takes no space at the seam",
                results: vec!["你好", "，欢迎来到格林戴尔社区学院", "。"],
                want: "你好，欢迎来到格林戴尔社区学院。",
            },
            Case {
                name: "japanese takes no space",
                results: vec!["こんにちは。", "グリーンデール"],
                want: "こんにちは。グリーンデール",
            },
            Case {
                name: "korean keeps its spaces",
                results: vec!["안녕하세요.", "그린데일입니다."],
                want: "안녕하세요. 그린데일입니다.",
            },
            Case {
                name: "latin next to han is still unspaced",
                results: vec!["Greendale", "社区学院"],
                want: "Greendale社区学院",
            },
            Case {
                name: "edge whitespace and empty results",
                results: vec![" Six seasons ", "", "  ", "and a movie. "],
                want: "Six seasons and a movie.",
            },
        ];
        for tc in cases {
            let results: Vec<String> = tc.results.iter().map(|r| (*r).to_owned()).collect();
            assert_eq!(join_results(&results), tc.want, "{}", tc.name);
        }
    }
}
