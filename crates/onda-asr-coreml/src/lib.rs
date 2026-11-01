//! Parakeet speech engines on CoreML, using FluidInference's conversions
//! (MOD-1). The only platform-specific crate in the core (ARCH-1).
//!
//! The decode logic is a port of FluidAudio's Parakeet pipeline: the TDT
//! models (v2, Ultra, Japanese) and Parakeet Unified's streaming RNNT.
//! Unsafe code is confined to the `coreml` module.
#![deny(unsafe_code)]

mod chunk;
mod coreml;
mod engine;
mod language;
pub mod manifest;
mod mel;
mod recovery;
mod tdt;
mod tensor;
mod unified;
mod vocab;

pub use engine::ParakeetEngine;

#[cfg(test)]
mod tests {
    use onda_domain::models::{Catalog, ModelId};

    use crate::manifest::manifest;

    /// MOD-2: catalog size hints match the real downloads, in decimal MB.
    #[test]
    fn size_hints_match_manifests() {
        let catalog = Catalog::new(None);
        for model in ModelId::ALL {
            let Some(manifest) = manifest(model) else {
                continue;
            };
            let Some(info) = catalog.model(model) else {
                panic!("{} missing from catalog", model.as_str());
            };
            let want = format!("~{}MB", (manifest.total_size() as f64 / 1e6).round());
            assert_eq!(info.size_hint, want, "{}", model.as_str());
        }
    }

    /// Every file is pinned by a hex SHA-256, so a corrupt or swapped file of
    /// the right size is caught before CoreML reads it (DL-5, ASR-6).
    #[test]
    fn every_file_is_hashed() {
        for model in ModelId::ALL {
            let Some(manifest) = manifest(model) else {
                continue;
            };
            for file in manifest.files {
                let Some(sha256) = file.sha256 else {
                    panic!("{} has no SHA-256", file.path);
                };
                let hex = sha256.len() == 64
                    && sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                assert!(hex, "{}: {sha256}", file.path);
            }
        }
    }
}
