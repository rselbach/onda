//! The speech-engine interface every inference backend implements
//! (ASR-1..ASR-21).
//!
//! Engines are synchronous and blocking. The session engine runs every call
//! on a blocking thread behind one serialized gate (ARCH-8, ASR-14), so an
//! engine never sees two calls at once and needs `Send` but not `Sync`.

use std::fmt;

use onda_domain::languages::Language;

/// Runtime progress while an engine loads (ASR-2).
///
/// The caller publishes "Preparing model..." and the indeterminate "Loading
/// model..." itself; engines report only what their runtime is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadProgress {
    /// Runtime-specific status, e.g. "Compiling Encoder.mlmodelc...". Empty
    /// means "Loading model...".
    pub status: String,
    /// Share of the load done, in `[0, 1]`.
    pub fraction: f64,
}

/// Why an engine call failed. `Display` is the user-facing copy; `detail`
/// is for logs only (LOG-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsrError {
    /// The model files could not be read or compiled (ASR-6).
    LoadFailed {
        /// What went wrong, for logs.
        detail: String,
    },
    /// A decode was requested while no model is loaded.
    NotReady,
    /// The runtime failed while decoding.
    TranscriptionFailed {
        /// What went wrong, for logs.
        detail: String,
    },
    /// The engine cannot serve the request and says why in user-facing
    /// copy, e.g. the system engine's unsupported locale (SYS-6).
    Unsupported {
        /// What to show the user.
        message: String,
    },
    /// The platform cancelled the call. Not a failure: a cancelled load
    /// reports `not_ready` (ASR-7) and a cancelled decode reports nothing
    /// (SYS-7).
    Cancelled,
}

impl fmt::Display for AsrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsrError::LoadFailed { .. } => f.write_str("Failed to load the transcription model"),
            AsrError::NotReady => f.write_str("Transcription model is not ready"),
            AsrError::TranscriptionFailed { .. } => f.write_str("Transcription failed"),
            AsrError::Unsupported { message } => f.write_str(message),
            AsrError::Cancelled => f.write_str("Transcription cancelled"),
        }
    }
}

impl std::error::Error for AsrError {}

/// A speech-to-text engine for one model.
///
/// Audio is always 16 kHz mono `f32` in `[-1, 1]`. `language` is the selected
/// language, `None` for `auto` (ASR-11, see
/// [`LanguageSetting::engine_language`](onda_domain::languages::LanguageSetting::engine_language));
/// engines that cannot use it ignore it (ASR-12). Returned text is trimmed
/// (ASR-13) and may be empty.
pub trait SpeechEngine: Send {
    /// Loads the model, reporting runtime progress. Loading an engine that
    /// is already loaded is a no-op.
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError>;

    /// Releases the model. Decodes fail with [`AsrError::NotReady`] until
    /// the next [`load`](Self::load).
    fn unload(&mut self);

    /// Whether the model streams a live preview (catalog `live_preview`).
    fn supports_preview(&self) -> bool;

    /// Shortest final audio, in samples, the engine can decode. Shorter
    /// audio is never sent to [`transcribe`](Self::transcribe); the session
    /// drops it like a too-short recording (GATE-2). Engines without a
    /// minimum keep the default of 0.
    fn min_samples(&self) -> usize {
        0
    }

    /// Final transcript of a whole recording (ASR-20). A streaming engine
    /// feeds the audio its preview stream has not seen and finalizes that
    /// stream; every other engine decodes `samples` from scratch.
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
    ) -> Result<String, AsrError>;

    /// Streaming decode for the live preview (ASR-15..ASR-17). Called
    /// repeatedly with the whole recording so far. Parakeet TDT decodes
    /// again only the windows whose audio changed since its last preview; a
    /// streaming engine decodes only the audio it has not seen. Both start
    /// over when `samples` is shorter than the last call (a new recording;
    /// see [`UnifiedStream`](onda_domain::preview::UnifiedStream)).
    fn preview(&mut self, samples: &[f32], language: Option<Language>) -> Result<String, AsrError>;

    /// Forgets the preview stream so the next call starts a new recording
    /// (ASR-20). A no-op for engines that keep no stream.
    fn reset_preview(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference copy; details stay out of the user-facing text.
    #[test]
    fn errors_display_reference_copy() {
        let cases = [
            (
                AsrError::LoadFailed {
                    detail: "Encoder.mlmodelc missing".to_owned(),
                },
                "Failed to load the transcription model",
            ),
            (AsrError::NotReady, "Transcription model is not ready"),
            (
                AsrError::TranscriptionFailed {
                    detail: "joint prediction failed".to_owned(),
                },
                "Transcription failed",
            ),
            (
                AsrError::Unsupported {
                    message: "System Speech can't transcribe Persian (Iran) on this Mac".to_owned(),
                },
                "System Speech can't transcribe Persian (Iran) on this Mac",
            ),
            (AsrError::Cancelled, "Transcription cancelled"),
        ];
        for (error, want) in cases {
            assert_eq!(error.to_string(), want, "{error:?}");
        }
    }
}
