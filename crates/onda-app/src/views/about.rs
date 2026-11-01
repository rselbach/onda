//! The About page (WIN-21).

use onda_domain::models::Catalog;

/// The one-line description under the version (WIN-21).
pub const ABOUT_TAGLINE: &str = "Local speech-to-text. Audio is processed on-device; optional \
                                 transcript refinement can use a cloud provider.";

/// Third-party components Onda ships, with their licenses as their projects
/// declare them (WIN-21): the shell's Swift packages, the code the CoreML
/// decoder is ported from, and the significant crates linked into the core.
pub const LIBRARIES: &[(&str, &str, &str)] = &[
    (
        "aws-lc-rs",
        "ISC and (Apache-2.0 or ISC)",
        "https://github.com/aws/aws-lc-rs",
    ),
    (
        "aws-lc-sys",
        "ISC, Apache-2.0, MIT, and BSD-3-Clause",
        "https://github.com/aws/aws-lc-rs",
    ),
    ("cpal", "Apache-2.0", "https://github.com/RustAudio/cpal"),
    (
        "ed25519-dalek",
        "BSD-3-Clause",
        "https://github.com/dalek-cryptography/curve25519-dalek",
    ),
    (
        "FluidAudio",
        "Apache-2.0",
        "https://github.com/FluidInference/FluidAudio",
    ),
    (
        "jiff",
        "MIT or Unlicense",
        "https://github.com/BurntSushi/jiff",
    ),
    (
        "keyring",
        "MIT or Apache-2.0",
        "https://github.com/open-source-cooperative/keyring-rs",
    ),
    (
        "mediaremote-adapter",
        "BSD-3-Clause",
        "https://github.com/ejbills/mediaremote-adapter",
    ),
    ("objc2", "MIT", "https://github.com/madsmtm/objc2"),
    (
        "reqwest",
        "MIT or Apache-2.0",
        "https://github.com/seanmonstar/reqwest",
    ),
    (
        "rtrb",
        "MIT or Apache-2.0",
        "https://github.com/mgeier/rtrb",
    ),
    (
        "rubato",
        "MIT or Apache-2.0",
        "https://github.com/HEnquist/rubato",
    ),
    (
        "rustls",
        "Apache-2.0, ISC, or MIT",
        "https://github.com/rustls/rustls",
    ),
    (
        "security-framework",
        "MIT or Apache-2.0",
        "https://github.com/kornelski/rust-security-framework",
    ),
    (
        "serde",
        "MIT or Apache-2.0",
        "https://github.com/serde-rs/serde",
    ),
    (
        "serde_json",
        "MIT or Apache-2.0",
        "https://github.com/serde-rs/json",
    ),
    (
        "sha2",
        "MIT or Apache-2.0",
        "https://github.com/RustCrypto/hashes",
    ),
    (
        "tempfile",
        "MIT or Apache-2.0",
        "https://github.com/Stebalien/tempfile",
    ),
    ("tokio", "MIT", "https://github.com/tokio-rs/tokio"),
    ("tracing", "MIT", "https://github.com/tokio-rs/tracing"),
    (
        "tracing-oslog",
        "Zlib",
        "https://github.com/Absolucy/tracing-oslog",
    ),
    ("UniFFI", "MPL-2.0", "https://github.com/mozilla/uniffi-rs"),
    (
        "unicode-normalization",
        "(MIT or Apache-2.0) and Unicode-3.0",
        "https://github.com/unicode-rs/unicode-normalization",
    ),
    (
        "unicode-segmentation",
        "MIT or Apache-2.0",
        "https://github.com/unicode-rs/unicode-segmentation",
    ),
    (
        "uuid",
        "MIT or Apache-2.0",
        "https://github.com/uuid-rs/uuid",
    ),
];

/// A catalog model's credit (WIN-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutModel {
    /// Display name.
    pub name: String,
    /// Attribution line.
    pub attribution: String,
    /// License name, the link's title.
    pub license_name: String,
    /// License URL.
    pub license_url: String,
}

/// A third-party component (WIN-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutLibrary {
    /// Name.
    pub name: String,
    /// License.
    pub license: String,
    /// Project URL.
    pub url: String,
}

/// The About page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutPage {
    /// "Onda".
    pub app_name: String,
    /// "Version <x.y.z>".
    pub version: String,
    /// The tagline.
    pub tagline: String,
    /// "Models".
    pub models_title: String,
    /// Every catalog model.
    pub models: Vec<AboutModel>,
    /// "Libraries".
    pub libraries_title: String,
    /// Third-party components.
    pub libraries: Vec<AboutLibrary>,
}

/// Builds the About page for `version`.
pub fn about_page(catalog: &Catalog, version: &str) -> AboutPage {
    AboutPage {
        app_name: "Onda".to_owned(),
        version: format!("Version {version}"),
        tagline: ABOUT_TAGLINE.to_owned(),
        models_title: "Models".to_owned(),
        models: catalog
            .models()
            .into_iter()
            .map(|model| AboutModel {
                name: model.name.to_owned(),
                attribution: model.attribution.to_owned(),
                license_name: model.license_name.to_owned(),
                license_url: model.license_url.to_owned(),
            })
            .collect(),
        libraries_title: "Libraries".to_owned(),
        libraries: LIBRARIES
            .iter()
            .map(|(name, license, url)| AboutLibrary {
                name: (*name).to_owned(),
                license: (*license).to_owned(),
                url: (*url).to_owned(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_page_copy_models_and_libraries_win_21() {
        let page = about_page(&Catalog::new(None), "1.2.3");
        assert_eq!(page.app_name, "Onda");
        assert_eq!(page.version, "Version 1.2.3");
        assert_eq!(
            page.tagline,
            "Local speech-to-text. Audio is processed on-device; optional transcript \
             refinement can use a cloud provider."
        );
        // MOD-1: every conversion is credited
        for model in &page.models {
            assert!(
                model
                    .attribution
                    .ends_with("; CoreML conversion by FluidInference"),
                "{}",
                model.name
            );
        }

        let libraries: Vec<(&str, &str)> = page
            .libraries
            .iter()
            .map(|library| (library.name.as_str(), library.license.as_str()))
            .collect();
        let want = [
            ("aws-lc-rs", "ISC and (Apache-2.0 or ISC)"),
            ("aws-lc-sys", "ISC, Apache-2.0, MIT, and BSD-3-Clause"),
            ("cpal", "Apache-2.0"),
            ("ed25519-dalek", "BSD-3-Clause"),
            ("FluidAudio", "Apache-2.0"),
            ("jiff", "MIT or Unlicense"),
            ("keyring", "MIT or Apache-2.0"),
            ("mediaremote-adapter", "BSD-3-Clause"),
            ("objc2", "MIT"),
            ("reqwest", "MIT or Apache-2.0"),
            ("rtrb", "MIT or Apache-2.0"),
            ("rubato", "MIT or Apache-2.0"),
            ("rustls", "Apache-2.0, ISC, or MIT"),
            ("security-framework", "MIT or Apache-2.0"),
            ("serde", "MIT or Apache-2.0"),
            ("serde_json", "MIT or Apache-2.0"),
            ("sha2", "MIT or Apache-2.0"),
            ("tempfile", "MIT or Apache-2.0"),
            ("tokio", "MIT"),
            ("tracing", "MIT"),
            ("tracing-oslog", "Zlib"),
            ("UniFFI", "MPL-2.0"),
            (
                "unicode-normalization",
                "(MIT or Apache-2.0) and Unicode-3.0",
            ),
            ("unicode-segmentation", "MIT or Apache-2.0"),
            ("uuid", "MIT or Apache-2.0"),
        ];
        assert_eq!(libraries, want);
        assert!(
            page.libraries
                .iter()
                .all(|library| library.url.starts_with("https://")),
            "every library links to its project"
        );
    }
}
