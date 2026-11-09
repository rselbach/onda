//! Entry metadata (HIS-3, HIS-4), folder naming (HIS-5), path safety and
//! load-time cleanup (HIS-8), and the newest-first list (HIS-9).

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AUDIO_FILENAME, SAMPLE_RATE};

/// One path segment that cannot escape the folder it is joined to (HIS-8,
/// ARCH-12): never empty, `.`, or `..`, and never containing `/` or `\`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathComponent(String);

impl PathComponent {
    /// Returns `None` when `name` is unsafe to join to a folder path.
    pub fn new(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        let unsafe_name =
            name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']);
        match unsafe_name {
            true => None,
            false => Some(Self(name)),
        }
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PathComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A recording kept in an entry folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryAudio {
    /// File name inside the entry folder, normally [`AUDIO_FILENAME`].
    pub filename: PathComponent,
    /// Size of the file in bytes; counts toward the audio budget (HIS-14).
    pub byte_count: u64,
}

/// One saved dictation (HIS-3).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// Unique entry id.
    pub id: Uuid,
    /// When the entry was built, after cleanup and replacements.
    pub timestamp: Timestamp,
    /// Recording length in seconds (samples / 16,000).
    pub duration: f64,
    /// Sample rate of the session, in Hz.
    pub sample_rate: u32,
    /// Model actually loaded for the session.
    pub model_id: String,
    /// Model display name at save time, else the id.
    pub model_name: String,
    /// Language setting at save time (`auto` allowed).
    pub language: String,
    /// Final text after cleanup and replacements.
    pub transcript: String,
    /// ASR output, set only when cleanup was applied.
    pub raw_transcript: Option<String>,
    /// Whether cleanup was applied.
    pub was_post_processed: bool,
    /// Why cleanup fell back or was skipped.
    pub post_processing_error_description: Option<String>,
    /// Target app name, when known.
    pub app_name: Option<String>,
    /// Target app identifier (per-OS meaning), when known.
    pub app_bundle_id: Option<String>,
    /// Name of the entry folder under the history folder.
    pub directory_name: PathComponent,
    /// The kept recording, or `None` when audio was not kept or was dropped
    /// by the audio budget.
    pub audio: Option<EntryAudio>,
}

impl HistoryEntry {
    /// Whether the entry still has a recording.
    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// Bytes of audio this entry holds; 0 without audio.
    pub fn audio_byte_count(&self) -> u64 {
        self.audio.as_ref().map_or(0, |audio| audio.byte_count)
    }

    /// This entry once its recording is gone (HIS-14).
    pub fn without_audio(&self) -> Self {
        Self {
            audio: None,
            ..self.clone()
        }
    }
}

/// What a finished session contributes to a new entry (HIS-2, HIS-3).
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntry {
    /// Id for the entry; the caller generates it.
    pub id: Uuid,
    /// When the entry is built, after cleanup and replacements.
    pub timestamp: Timestamp,
    /// Number of 16 kHz samples in the session.
    pub sample_count: usize,
    /// Model actually loaded.
    pub model_id: String,
    /// Model display name, when the catalog knows the id.
    pub model_name: Option<String>,
    /// Language setting at save time.
    pub language: String,
    /// Final text after cleanup and replacements.
    pub transcript: String,
    /// ASR output, only when cleanup was applied.
    pub raw_transcript: Option<String>,
    /// Whether cleanup was applied.
    pub was_post_processed: bool,
    /// Why cleanup fell back or was skipped.
    pub post_processing_error_description: Option<String>,
    /// Target app name.
    pub app_name: Option<String>,
    /// Target app identifier.
    pub app_bundle_id: Option<String>,
    /// Size of the encoded WAV when audio is kept, else `None`.
    pub audio_byte_count: Option<u64>,
}

impl NewEntry {
    /// The entry to write: folder name from [`entry_directory_name`],
    /// duration from the sample count, model name falling back to the id,
    /// and [`AUDIO_FILENAME`] when audio is kept.
    pub fn into_entry(self) -> HistoryEntry {
        let audio = self.audio_byte_count.map(|byte_count| EntryAudio {
            filename: PathComponent(AUDIO_FILENAME.to_owned()),
            byte_count,
        });
        HistoryEntry {
            id: self.id,
            timestamp: self.timestamp,
            duration: self.sample_count as f64 / f64::from(SAMPLE_RATE),
            sample_rate: SAMPLE_RATE,
            model_name: self.model_name.unwrap_or_else(|| self.model_id.clone()),
            model_id: self.model_id,
            language: self.language,
            transcript: self.transcript,
            raw_transcript: self.raw_transcript,
            was_post_processed: self.was_post_processed,
            post_processing_error_description: self.post_processing_error_description,
            app_name: self.app_name,
            app_bundle_id: self.app_bundle_id,
            directory_name: entry_directory_name(self.timestamp, self.id),
            audio,
        }
    }
}

/// Entry folder name (HIS-5): the UTC timestamp with millisecond fraction and
/// `:` replaced by `-`, then `-` and the uppercase UUID, for example
/// `2025-01-01T00-00-00.000Z-00000000-0000-0000-0000-000000000001`.
pub fn entry_directory_name(timestamp: Timestamp, id: Uuid) -> PathComponent {
    let time = timestamp.strftime("%Y-%m-%dT%H-%M-%S%.3fZ");
    PathComponent(format!("{time}-{}", uppercase_uuid(id)))
}

fn uppercase_uuid(id: Uuid) -> String {
    format!("{id:X}")
}

/// Why `metadata.json` did not yield a usable entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataError {
    /// Not JSON, a required field is missing, or a value has the wrong type
    /// or format (including the id and timestamp).
    Malformed(String),
    /// `directoryName` or `audioFilename` could escape the history folder.
    UnsafeName(String),
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "malformed history metadata: {reason}"),
            Self::UnsafeName(name) => write!(f, "unsafe name in history metadata: '{name}'"),
        }
    }
}

impl std::error::Error for MetadataError {}

// on-disk shape. fields are declared in sorted key order so serde_json writes
// them sorted (HIS-5); optional keys are skipped when absent, like Jabber.
#[derive(Serialize, Deserialize)]
struct Metadata {
    #[serde(
        rename = "appBundleID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    app_bundle_id: Option<String>,
    #[serde(rename = "appName", default, skip_serializing_if = "Option::is_none")]
    app_name: Option<String>,
    #[serde(rename = "audioByteCount", default)]
    audio_byte_count: Option<u64>,
    #[serde(
        rename = "audioFilename",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    audio_filename: Option<String>,
    #[serde(rename = "directoryName")]
    directory_name: String,
    duration: f64,
    id: String,
    language: String,
    #[serde(rename = "modelID")]
    model_id: String,
    #[serde(rename = "modelName")]
    model_name: String,
    #[serde(
        rename = "postProcessingErrorDescription",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    post_processing_error_description: Option<String>,
    #[serde(
        rename = "rawTranscript",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    raw_transcript: Option<String>,
    #[serde(rename = "sampleRate")]
    sample_rate: u32,
    timestamp: String,
    transcript: String,
    #[serde(rename = "wasPostProcessed", default)]
    was_post_processed: Option<bool>,
}

/// `metadata.json` contents for `entry` (HIS-5): pretty-printed JSON with
/// sorted keys, absent optional fields omitted, the id as an uppercase UUID,
/// and the timestamp as ISO 8601 UTC without fractional seconds (Jabber's
/// format, so a reloaded entry's timestamp is truncated to the second).
pub fn encode_metadata(entry: &HistoryEntry) -> serde_json::Result<Vec<u8>> {
    let metadata = Metadata {
        app_bundle_id: entry.app_bundle_id.clone(),
        app_name: entry.app_name.clone(),
        audio_byte_count: Some(entry.audio_byte_count()),
        audio_filename: entry
            .audio
            .as_ref()
            .map(|audio| audio.filename.as_str().to_owned()),
        directory_name: entry.directory_name.as_str().to_owned(),
        duration: entry.duration,
        id: uppercase_uuid(entry.id),
        language: entry.language.clone(),
        model_id: entry.model_id.clone(),
        model_name: entry.model_name.clone(),
        post_processing_error_description: entry.post_processing_error_description.clone(),
        raw_transcript: entry.raw_transcript.clone(),
        sample_rate: entry.sample_rate,
        timestamp: entry.timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
        transcript: entry.transcript.clone(),
        was_post_processed: Some(entry.was_post_processed),
    };
    serde_json::to_vec_pretty(&metadata)
}

/// Parses `metadata.json` (HIS-4). Optional fields may be missing or null:
/// `audioFilename`, `rawTranscript`, `postProcessingErrorDescription`,
/// `appName`, and `appBundleID` become `None`, `audioByteCount` 0, and
/// `wasPostProcessed` false. Timestamps parse with or without fractional
/// seconds. Unsafe names are rejected (HIS-8).
pub fn decode_metadata(bytes: &[u8]) -> Result<HistoryEntry, MetadataError> {
    let metadata: Metadata =
        serde_json::from_slice(bytes).map_err(|err| MetadataError::Malformed(err.to_string()))?;
    let id = Uuid::parse_str(&metadata.id)
        .map_err(|err| MetadataError::Malformed(format!("id '{}': {err}", metadata.id)))?;
    let timestamp: Timestamp = metadata.timestamp.parse().map_err(|err| {
        MetadataError::Malformed(format!("timestamp '{}': {err}", metadata.timestamp))
    })?;
    let directory_name = safe_component(metadata.directory_name)?;
    let audio = match metadata.audio_filename {
        None => None,
        Some(filename) => Some(EntryAudio {
            filename: safe_component(filename)?,
            byte_count: metadata.audio_byte_count.unwrap_or(0),
        }),
    };
    Ok(HistoryEntry {
        id,
        timestamp,
        duration: metadata.duration,
        sample_rate: metadata.sample_rate,
        model_id: metadata.model_id,
        model_name: metadata.model_name,
        language: metadata.language,
        transcript: metadata.transcript,
        raw_transcript: metadata.raw_transcript,
        was_post_processed: metadata.was_post_processed.unwrap_or(false),
        post_processing_error_description: metadata.post_processing_error_description,
        app_name: metadata.app_name,
        app_bundle_id: metadata.app_bundle_id,
        directory_name,
        audio,
    })
}

fn safe_component(name: String) -> Result<PathComponent, MetadataError> {
    match PathComponent::new(name.clone()) {
        Some(component) => Ok(component),
        None => Err(MetadataError::UnsafeName(name)),
    }
}

/// What the storage layer found at `<entry folder>/metadata.json` while
/// scanning the history folder on first load. Plain files and hidden entries
/// in the history folder are not entry folders and are never classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataFile<'a> {
    /// The folder has no `metadata.json`.
    Missing,
    /// `metadata.json` exists but could not be read.
    Unreadable,
    /// The bytes of `metadata.json`.
    Contents(&'a [u8]),
}

/// What first load does with an entry folder (HIS-8).
#[derive(Debug, Clone, PartialEq)]
pub enum FolderVerdict {
    /// A valid entry; add it to the in-memory list.
    Keep(Box<HistoryEntry>),
    /// No `metadata.json`: what a write that died part-way leaves. Delete
    /// the whole entry folder.
    Prune,
    /// Leave the folder on disk and out of the list.
    Skip(SkipReason),
}

/// Why an entry folder is left out of the list, for logging (HIS-8). The
/// folder may hold a real entry this build cannot read right now, so it is
/// never deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `metadata.json` could not be read.
    UnreadableMetadata,
    /// `metadata.json` did not decode or names an unsafe path.
    Invalid(MetadataError),
}

/// Classifies one entry folder found on first load (HIS-8).
pub fn classify_entry_folder(metadata: MetadataFile<'_>) -> FolderVerdict {
    match metadata {
        MetadataFile::Missing => FolderVerdict::Prune,
        MetadataFile::Unreadable => FolderVerdict::Skip(SkipReason::UnreadableMetadata),
        MetadataFile::Contents(bytes) => match decode_metadata(bytes) {
            Ok(entry) => FolderVerdict::Keep(Box::new(entry)),
            Err(err) => FolderVerdict::Skip(SkipReason::Invalid(err)),
        },
    }
}

/// Orders loaded entries newest first (HIS-9).
pub fn sort_newest_first(entries: &mut [HistoryEntry]) {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp));
}

/// Inserts a just-saved entry into a newest-first list without rescanning
/// (HIS-9): before the first entry that is strictly older, so it lands after
/// entries with the same timestamp.
pub fn insert_newest_first(entries: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    let index = entries
        .iter()
        .position(|existing| existing.timestamp < entry.timestamp)
        .unwrap_or(entries.len());
    entries.insert(index, entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_support::{at, entry, with_audio};

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn path_component_safety_his_8() {
        struct Case {
            name: &'static str,
            input: &'static str,
            want: bool,
        }
        let cases = [
            Case {
                name: "entry folder name",
                input: "2025-01-01T00-00-00.000Z-00000000-0000-0000-0000-000000000001",
                want: true,
            },
            Case {
                name: "audio file",
                input: "audio.wav",
                want: true,
            },
            Case {
                name: "dots inside a name are fine",
                input: "..audio..wav",
                want: true,
            },
            Case {
                name: "empty",
                input: "",
                want: false,
            },
            Case {
                name: "current folder",
                input: ".",
                want: false,
            },
            Case {
                name: "parent folder",
                input: "..",
                want: false,
            },
            Case {
                name: "forward slash traversal",
                input: "../../escape.wav",
                want: false,
            },
            Case {
                name: "absolute path",
                input: "/etc/passwd",
                want: false,
            },
            Case {
                name: "backslash",
                input: "..\\escape.wav",
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(
                PathComponent::new(tc.input).is_some(),
                tc.want,
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn entry_directory_name_his_5() {
        struct Case {
            name: &'static str,
            timestamp: &'static str,
            id: u128,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "whole seconds keep a millisecond fraction",
                timestamp: "2025-01-01T00:00:00Z",
                id: 1,
                want: "2025-01-01T00-00-00.000Z-00000000-0000-0000-0000-000000000001",
            },
            Case {
                name: "fraction is cut to milliseconds and uuid is uppercase",
                timestamp: "2025-09-29T13:05:07.123456Z",
                id: 0xabcdef,
                want: "2025-09-29T13-05-07.123Z-00000000-0000-0000-0000-000000ABCDEF",
            },
            Case {
                name: "offsets are written as UTC",
                timestamp: "2025-09-29T09:00:00-04:00",
                id: 2,
                want: "2025-09-29T13-00-00.000Z-00000000-0000-0000-0000-000000000002",
            },
        ];
        for tc in cases {
            let timestamp: Timestamp = tc.timestamp.parse().expect("valid timestamp");
            let got = entry_directory_name(timestamp, uuid(tc.id));
            assert_eq!(got.as_str(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn new_entry_builds_derived_fields_his_3() {
        struct Case {
            name: &'static str,
            sample_count: usize,
            model_name: Option<&'static str>,
            audio_byte_count: Option<u64>,
            want_duration: f64,
            want_model_name: &'static str,
            want_audio: Option<(&'static str, u64)>,
        }
        let cases = [
            Case {
                name: "audio kept",
                sample_count: 4,
                model_name: Some("Parakeet TDT v2"),
                audio_byte_count: Some(52),
                want_duration: 4.0 / 16_000.0,
                want_model_name: "Parakeet TDT v2",
                want_audio: Some(("audio.wav", 52)),
            },
            Case {
                name: "transcript only",
                sample_count: 100,
                model_name: Some("Parakeet TDT v2"),
                audio_byte_count: None,
                want_duration: 100.0 / 16_000.0,
                want_model_name: "Parakeet TDT v2",
                want_audio: None,
            },
            Case {
                name: "unknown model falls back to its id",
                sample_count: 16_000,
                model_name: None,
                audio_byte_count: None,
                want_duration: 1.0,
                want_model_name: "parakeet-tdt-v2",
                want_audio: None,
            },
        ];
        for tc in cases {
            let timestamp = at(1_735_689_600);
            let got = NewEntry {
                id: uuid(7),
                timestamp,
                sample_count: tc.sample_count,
                model_id: "parakeet-tdt-v2".into(),
                model_name: tc.model_name.map(Into::into),
                language: "auto".into(),
                transcript: "cool cool cool".into(),
                raw_transcript: None,
                was_post_processed: false,
                post_processing_error_description: None,
                app_name: Some("Greendale Messenger".into()),
                app_bundle_id: Some("edu.greendale.messenger".into()),
                audio_byte_count: tc.audio_byte_count,
            }
            .into_entry();
            assert!(
                (got.duration - tc.want_duration).abs() < 1e-9,
                "{}: duration {}",
                tc.name,
                got.duration
            );
            assert_eq!(got.sample_rate, 16_000, "{}", tc.name);
            assert_eq!(got.model_name, tc.want_model_name, "{}", tc.name);
            assert_eq!(
                got.audio
                    .as_ref()
                    .map(|audio| (audio.filename.as_str(), audio.byte_count)),
                tc.want_audio,
                "{}",
                tc.name
            );
            assert_eq!(
                got.directory_name,
                entry_directory_name(timestamp, uuid(7)),
                "{}",
                tc.name
            );
            assert_eq!(
                got.app_bundle_id.as_deref(),
                Some("edu.greendale.messenger"),
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn encode_metadata_is_pretty_sorted_and_iso8601_his_5() {
        let mut full = with_audio(entry(1, "Hello.", at(1_735_689_600)), 84);
        full.timestamp = "2025-01-01T00:00:00.750Z".parse().expect("valid timestamp");
        full.duration = 0.5;
        full.raw_transcript = Some(" um hello ".into());
        full.was_post_processed = true;
        full.post_processing_error_description = Some("boom".into());
        full.app_name = Some("Greendale Messenger".into());
        full.app_bundle_id = Some("edu.greendale.messenger".into());
        full.directory_name = PathComponent::new("legacy").expect("safe");

        struct Case {
            name: &'static str,
            entry: HistoryEntry,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "every field",
                entry: full.clone(),
                want: r#"{
  "appBundleID": "edu.greendale.messenger",
  "appName": "Greendale Messenger",
  "audioByteCount": 84,
  "audioFilename": "audio.wav",
  "directoryName": "legacy",
  "duration": 0.5,
  "id": "00000000-0000-0000-0000-000000000001",
  "language": "en",
  "modelID": "parakeet-tdt-v2",
  "modelName": "Parakeet TDT v2",
  "postProcessingErrorDescription": "boom",
  "rawTranscript": " um hello ",
  "sampleRate": 16000,
  "timestamp": "2025-01-01T00:00:00Z",
  "transcript": "Hello.",
  "wasPostProcessed": true
}"#,
            },
            Case {
                name: "absent optionals are omitted and audio bytes are zero",
                entry: entry(0xab, "Troy Barnes", at(100)),
                want: r#"{
  "audioByteCount": 0,
  "directoryName": "entry-171",
  "duration": 1.0,
  "id": "00000000-0000-0000-0000-0000000000AB",
  "language": "en",
  "modelID": "parakeet-tdt-v2",
  "modelName": "Parakeet TDT v2",
  "sampleRate": 16000,
  "timestamp": "1970-01-01T00:01:40Z",
  "transcript": "Troy Barnes",
  "wasPostProcessed": false
}"#,
            },
        ];
        for tc in cases {
            let bytes = encode_metadata(&tc.entry).expect("encodes");
            let got = String::from_utf8(bytes).expect("utf-8");
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn metadata_round_trips_his_5() {
        let mut cleaned = with_audio(entry(2, "Hello.", at(1_735_689_600)), 52);
        cleaned.raw_transcript = Some(" um hello ".into());
        cleaned.was_post_processed = true;
        let mut fell_back = entry(3, "raw fallback", at(1_735_689_600));
        fell_back.post_processing_error_description = Some("boom".into());
        let mut with_app = entry(4, "pop pop", at(100));
        with_app.app_name = Some("Greendale Messenger".into());
        with_app.app_bundle_id = Some("edu.greendale.messenger".into());

        struct Case {
            name: &'static str,
            entry: HistoryEntry,
        }
        let cases = [
            Case {
                name: "post-processing metadata",
                entry: cleaned,
            },
            Case {
                name: "post-processing error description",
                entry: fell_back,
            },
            Case {
                name: "dictated app",
                entry: with_app,
            },
        ];
        for tc in cases {
            let bytes = encode_metadata(&tc.entry).expect("encodes");
            let got = decode_metadata(&bytes).expect("decodes");
            assert_eq!(got, tc.entry, "{}", tc.name);
        }
    }

    const LEGACY_JSON: &str = r#"{
        "id": "00000000-0000-0000-0000-000000000001",
        "timestamp": "2024-01-01T00:00:00Z",
        "duration": 0.5,
        "sampleRate": 16000,
        "modelID": "qwen3",
        "modelName": "Qwen3-ASR",
        "language": "en",
        "transcript": "cool cool cool",
        "directoryName": "legacy",
        "audioFilename": "audio.wav",
        "audioByteCount": 84
    }"#;

    /// `LEGACY_JSON` with keys removed (`None`) or set (`Some`).
    fn legacy(edits: &[(&str, Option<serde_json::Value>)]) -> String {
        let mut value: serde_json::Value =
            serde_json::from_str(LEGACY_JSON).expect("legacy fixture parses");
        let object = value.as_object_mut().expect("legacy fixture is an object");
        for (key, edit) in edits {
            match edit {
                None => object.remove(*key),
                Some(new) => object.insert((*key).to_owned(), new.clone()),
            };
        }
        value.to_string()
    }

    #[test]
    fn decode_metadata_is_lenient_his_4() {
        use serde_json::{Value, json};
        struct Case {
            name: &'static str,
            json: String,
            want_audio: Option<u64>,
            want_post_processed: bool,
            want_timestamp: &'static str,
        }
        let cases = [
            Case {
                name: "legacy entry missing post-processing and app fields",
                json: LEGACY_JSON.to_owned(),
                want_audio: Some(84),
                want_post_processed: false,
                want_timestamp: "2024-01-01T00:00:00Z",
            },
            Case {
                name: "missing audio fields",
                json: legacy(&[("audioFilename", None), ("audioByteCount", None)]),
                want_audio: None,
                want_post_processed: false,
                want_timestamp: "2024-01-01T00:00:00Z",
            },
            Case {
                name: "audio file without a byte count",
                json: legacy(&[("audioByteCount", None)]),
                want_audio: Some(0),
                want_post_processed: false,
                want_timestamp: "2024-01-01T00:00:00Z",
            },
            Case {
                name: "byte count without an audio file is ignored",
                json: legacy(&[("audioFilename", None)]),
                want_audio: None,
                want_post_processed: false,
                want_timestamp: "2024-01-01T00:00:00Z",
            },
            Case {
                name: "nulls count as missing",
                json: legacy(&[
                    ("audioByteCount", Some(Value::Null)),
                    ("wasPostProcessed", Some(Value::Null)),
                    ("rawTranscript", Some(Value::Null)),
                    ("appName", Some(Value::Null)),
                    ("appBundleID", Some(Value::Null)),
                    ("postProcessingErrorDescription", Some(Value::Null)),
                ]),
                want_audio: Some(0),
                want_post_processed: false,
                want_timestamp: "2024-01-01T00:00:00Z",
            },
            Case {
                name: "fractional seconds and offsets",
                json: legacy(&[("timestamp", Some(json!("2024-01-01T02:00:00.250+02:00")))]),
                want_audio: Some(84),
                want_post_processed: false,
                want_timestamp: "2024-01-01T00:00:00.25Z",
            },
            Case {
                name: "unknown keys are ignored",
                json: legacy(&[
                    ("wasPostProcessed", Some(json!(true))),
                    ("futureField", Some(json!([1, 2]))),
                ]),
                want_audio: Some(84),
                want_post_processed: true,
                want_timestamp: "2024-01-01T00:00:00Z",
            },
        ];
        for tc in cases {
            let got = match decode_metadata(tc.json.as_bytes()) {
                Ok(got) => got,
                Err(err) => panic!("{}: {err}", tc.name),
            };
            assert_eq!(got.transcript, "cool cool cool", "{}", tc.name);
            assert_eq!(got.id, uuid(1), "{}", tc.name);
            assert_eq!(got.app_name, None, "{}", tc.name);
            assert_eq!(got.raw_transcript, None, "{}", tc.name);
            assert_eq!(got.post_processing_error_description, None, "{}", tc.name);
            assert_eq!(
                got.was_post_processed, tc.want_post_processed,
                "{}",
                tc.name
            );
            assert_eq!(
                got.audio.as_ref().map(|audio| audio.byte_count),
                tc.want_audio,
                "{}",
                tc.name
            );
            assert_eq!(got.timestamp.to_string(), tc.want_timestamp, "{}", tc.name);
        }
    }

    #[test]
    fn classify_entry_folder_his_8() {
        use serde_json::json;
        let traversal_dir = legacy(&[("directoryName", Some(json!("..")))]);
        let dot_dir = legacy(&[("directoryName", Some(json!(".")))]);
        let empty_dir = legacy(&[("directoryName", Some(json!("")))]);
        let backslash_dir = legacy(&[("directoryName", Some(json!("a\\b")))]);
        let traversal_audio = legacy(&[("audioFilename", Some(json!("../../escape.wav")))]);
        let missing_transcript = legacy(&[("transcript", None)]);
        let missing_directory = legacy(&[("directoryName", None)]);
        let bad_id = legacy(&[("id", Some(json!("troy-and-abed")))]);
        let bad_timestamp = legacy(&[("timestamp", Some(json!("yesterday")))]);
        let no_zone = legacy(&[("timestamp", Some(json!("2024-01-01T00:00:00")))]);
        let wrong_type = legacy(&[("duration", Some(json!("half a second")))]);

        #[derive(Debug, PartialEq)]
        enum Want {
            Keep,
            Missing,
            Unreadable,
            Malformed,
            Unsafe(String),
        }
        struct Case<'a> {
            name: &'static str,
            input: MetadataFile<'a>,
            want: Want,
        }
        let cases = [
            Case {
                name: "valid metadata",
                input: MetadataFile::Contents(LEGACY_JSON.as_bytes()),
                want: Want::Keep,
            },
            Case {
                name: "orphan folder without metadata",
                input: MetadataFile::Missing,
                want: Want::Missing,
            },
            Case {
                name: "unreadable metadata",
                input: MetadataFile::Unreadable,
                want: Want::Unreadable,
            },
            Case {
                name: "corrupt json",
                input: MetadataFile::Contents(b"{ not valid json"),
                want: Want::Malformed,
            },
            Case {
                name: "required transcript missing",
                input: MetadataFile::Contents(missing_transcript.as_bytes()),
                want: Want::Malformed,
            },
            Case {
                name: "required directoryName missing",
                input: MetadataFile::Contents(missing_directory.as_bytes()),
                want: Want::Malformed,
            },
            Case {
                name: "wrong value type",
                input: MetadataFile::Contents(wrong_type.as_bytes()),
                want: Want::Malformed,
            },
            Case {
                name: "id is not a uuid",
                input: MetadataFile::Contents(bad_id.as_bytes()),
                want: Want::Malformed,
            },
            Case {
                name: "timestamp is not ISO 8601",
                input: MetadataFile::Contents(bad_timestamp.as_bytes()),
                want: Want::Malformed,
            },
            Case {
                name: "timestamp without a zone",
                input: MetadataFile::Contents(no_zone.as_bytes()),
                want: Want::Malformed,
            },
            Case {
                name: "directoryName traversal",
                input: MetadataFile::Contents(traversal_dir.as_bytes()),
                want: Want::Unsafe("..".into()),
            },
            Case {
                name: "directoryName is the current folder",
                input: MetadataFile::Contents(dot_dir.as_bytes()),
                want: Want::Unsafe(".".into()),
            },
            Case {
                name: "empty directoryName",
                input: MetadataFile::Contents(empty_dir.as_bytes()),
                want: Want::Unsafe(String::new()),
            },
            Case {
                name: "directoryName with a backslash",
                input: MetadataFile::Contents(backslash_dir.as_bytes()),
                want: Want::Unsafe("a\\b".into()),
            },
            Case {
                name: "audioFilename traversal",
                input: MetadataFile::Contents(traversal_audio.as_bytes()),
                want: Want::Unsafe("../../escape.wav".into()),
            },
        ];
        for tc in cases {
            let got = match classify_entry_folder(tc.input) {
                FolderVerdict::Keep(_) => Want::Keep,
                FolderVerdict::Prune => Want::Missing,
                FolderVerdict::Skip(SkipReason::UnreadableMetadata) => Want::Unreadable,
                FolderVerdict::Skip(SkipReason::Invalid(MetadataError::Malformed(_))) => {
                    Want::Malformed
                }
                FolderVerdict::Skip(SkipReason::Invalid(MetadataError::UnsafeName(name))) => {
                    Want::Unsafe(name)
                }
            };
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn newest_first_ordering_his_9() {
        struct Case {
            name: &'static str,
            existing: Vec<(&'static str, i64)>,
            insert: (&'static str, i64),
            want: Vec<&'static str>,
        }
        let cases = [
            Case {
                name: "saves out of order land by timestamp",
                existing: vec![("second", 200), ("first", 100)],
                insert: ("third", 150),
                want: vec!["second", "third", "first"],
            },
            Case {
                name: "newest goes first",
                existing: vec![("second", 200), ("first", 100)],
                insert: ("third", 300),
                want: vec!["third", "second", "first"],
            },
            Case {
                name: "oldest goes last",
                existing: vec![("second", 200)],
                insert: ("first", 100),
                want: vec!["second", "first"],
            },
            Case {
                name: "equal timestamps go after existing",
                existing: vec![("Troy", 100)],
                insert: ("Abed", 100),
                want: vec!["Troy", "Abed"],
            },
            Case {
                name: "empty list",
                existing: vec![],
                insert: ("Annie", 100),
                want: vec!["Annie"],
            },
        ];
        for tc in cases {
            let mut entries: Vec<HistoryEntry> = tc
                .existing
                .iter()
                .enumerate()
                .map(|(i, (text, secs))| entry(i as u128, text, at(*secs)))
                .collect();
            insert_newest_first(&mut entries, entry(99, tc.insert.0, at(tc.insert.1)));
            let got: Vec<&str> = entries.iter().map(|e| e.transcript.as_str()).collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }

        let mut loaded = vec![
            entry(1, "first", at(100)),
            entry(2, "second", at(200)),
            entry(3, "third", at(150)),
        ];
        sort_newest_first(&mut loaded);
        let got: Vec<&str> = loaded.iter().map(|e| e.transcript.as_str()).collect();
        assert_eq!(got, ["second", "third", "first"], "sort loaded entries");
    }

    #[test]
    fn without_audio_keeps_transcript_his_14() {
        let original = with_audio(entry(1, "Pierce", at(100)), 84);
        let got = original.without_audio();
        assert!(!got.has_audio(), "audio removed");
        assert_eq!(got.audio_byte_count(), 0, "byte count zeroed");
        assert_eq!(got.transcript, "Pierce", "transcript kept");
        assert_eq!(got.directory_name, original.directory_name, "folder kept");
    }
}
