//! What a downloadable model consists of (DL-5, DL-6).
//!
//! Backends own their manifests (MOD-1); the model manager downloads and
//! validates against them without knowing the runtime.

/// One file of a model, relative to the model folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestFile {
    /// Path relative to the model folder and to [`ModelManifest::base_url`].
    pub path: &'static str,
    /// Exact size in bytes; a file of any other size is incomplete.
    pub size: u64,
    /// Hex SHA-256, checked after download and before a load. Backends pin
    /// every file; a file without one is checked by size only.
    pub sha256: Option<&'static str>,
}

/// Every file a model needs, pinned to one upstream revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelManifest {
    /// URL prefix; a file downloads from `<base_url>/<path>`.
    pub base_url: &'static str,
    /// Required files.
    pub files: &'static [ManifestFile],
}

impl ModelManifest {
    /// Download URL for `file`.
    pub fn url(&self, file: &ManifestFile) -> String {
        format!("{}/{}", self.base_url, file.path)
    }

    /// Total download size in bytes (MOD-2).
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}
