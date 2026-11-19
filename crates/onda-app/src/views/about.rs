//! The About page (WIN-21).

use onda_domain::models::Catalog;
use onda_domain::platform::Platform;

/// The one-line description under the version (WIN-21).
pub const ABOUT_TAGLINE: &str = "Local speech-to-text. Audio is processed on-device; optional \
                                 transcript refinement can use a cloud provider.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryTarget {
    All,
    MacOs,
    Linux,
}

impl LibraryTarget {
    fn included_on(self, platform: Platform) -> bool {
        match self {
            Self::All => true,
            Self::MacOs => platform.os_name != "Linux",
            Self::Linux => platform.os_name == "Linux",
        }
    }
}

struct LibraryEntry {
    name: &'static str,
    license: &'static str,
    url: &'static str,
    target: LibraryTarget,
}

const fn lib(
    name: &'static str,
    license: &'static str,
    url: &'static str,
    target: LibraryTarget,
) -> LibraryEntry {
    LibraryEntry {
        name,
        license,
        url,
        target,
    }
}

/// Third-party components Onda ships, with their licenses as their projects
/// declare them (WIN-21), tagged by target platform.
const LIBRARIES: &[LibraryEntry] = &[
    lib(
        "aws-lc-rs",
        "ISC and (Apache-2.0 or ISC)",
        "https://github.com/aws/aws-lc-rs",
        LibraryTarget::All,
    ),
    lib(
        "aws-lc-sys",
        "ISC, Apache-2.0, MIT, and BSD-3-Clause",
        "https://github.com/aws/aws-lc-rs",
        LibraryTarget::All,
    ),
    lib(
        "cpal",
        "Apache-2.0",
        "https://github.com/RustAudio/cpal",
        LibraryTarget::All,
    ),
    lib(
        "ed25519-dalek",
        "BSD-3-Clause",
        "https://github.com/dalek-cryptography/curve25519-dalek",
        LibraryTarget::All,
    ),
    lib(
        "FluidAudio",
        "Apache-2.0",
        "https://github.com/FluidInference/FluidAudio",
        LibraryTarget::MacOs,
    ),
    lib(
        "gtk4-rs",
        "MIT",
        "https://github.com/gtk-rs/gtk4-rs",
        LibraryTarget::Linux,
    ),
    lib(
        "Hex desktop integration",
        "MIT",
        "https://github.com/anomalyco/hex",
        LibraryTarget::Linux,
    ),
    lib(
        "jiff",
        "MIT or Unlicense",
        "https://github.com/BurntSushi/jiff",
        LibraryTarget::All,
    ),
    lib(
        "keyring",
        "MIT or Apache-2.0",
        "https://github.com/open-source-cooperative/keyring-rs",
        LibraryTarget::MacOs,
    ),
    lib(
        "libadwaita-rs",
        "MIT",
        "https://gitlab.gnome.org/World/Rust/libadwaita-rs",
        LibraryTarget::Linux,
    ),
    lib(
        "mediaremote-adapter",
        "BSD-3-Clause",
        "https://github.com/ejbills/mediaremote-adapter",
        LibraryTarget::MacOs,
    ),
    lib(
        "objc2",
        "MIT",
        "https://github.com/madsmtm/objc2",
        LibraryTarget::MacOs,
    ),
    lib(
        "reqwest",
        "MIT or Apache-2.0",
        "https://github.com/seanmonstar/reqwest",
        LibraryTarget::All,
    ),
    lib(
        "rtrb",
        "MIT or Apache-2.0",
        "https://github.com/mgeier/rtrb",
        LibraryTarget::All,
    ),
    lib(
        "rubato",
        "MIT or Apache-2.0",
        "https://github.com/HEnquist/rubato",
        LibraryTarget::All,
    ),
    lib(
        "rustls",
        "Apache-2.0, ISC, or MIT",
        "https://github.com/rustls/rustls",
        LibraryTarget::All,
    ),
    lib(
        "security-framework",
        "MIT or Apache-2.0",
        "https://github.com/kornelski/rust-security-framework",
        LibraryTarget::MacOs,
    ),
    lib(
        "serde",
        "MIT or Apache-2.0",
        "https://github.com/serde-rs/serde",
        LibraryTarget::All,
    ),
    lib(
        "serde_json",
        "MIT or Apache-2.0",
        "https://github.com/serde-rs/json",
        LibraryTarget::All,
    ),
    lib(
        "sha2",
        "MIT or Apache-2.0",
        "https://github.com/RustCrypto/hashes",
        LibraryTarget::All,
    ),
    lib(
        "tempfile",
        "MIT or Apache-2.0",
        "https://github.com/Stebalien/tempfile",
        LibraryTarget::All,
    ),
    lib(
        "tokio",
        "MIT",
        "https://github.com/tokio-rs/tokio",
        LibraryTarget::All,
    ),
    lib(
        "tracing",
        "MIT",
        "https://github.com/tokio-rs/tracing",
        LibraryTarget::All,
    ),
    lib(
        "tracing-oslog",
        "Zlib",
        "https://github.com/Absolucy/tracing-oslog",
        LibraryTarget::MacOs,
    ),
    lib(
        "transcribe.cpp",
        "MIT",
        "https://github.com/handy-computer/transcribe.cpp",
        LibraryTarget::Linux,
    ),
    lib(
        "UniFFI",
        "MPL-2.0",
        "https://github.com/mozilla/uniffi-rs",
        LibraryTarget::MacOs,
    ),
    lib(
        "unicode-normalization",
        "(MIT or Apache-2.0) and Unicode-3.0",
        "https://github.com/unicode-rs/unicode-normalization",
        LibraryTarget::All,
    ),
    lib(
        "unicode-segmentation",
        "MIT or Apache-2.0",
        "https://github.com/unicode-rs/unicode-segmentation",
        LibraryTarget::All,
    ),
    lib(
        "uuid",
        "MIT or Apache-2.0",
        "https://github.com/uuid-rs/uuid",
        LibraryTarget::All,
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

/// Builds the About page for `version` and `platform`.
pub fn about_page(catalog: &Catalog, version: &str, platform: Platform) -> AboutPage {
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
            .filter(|entry| entry.target.included_on(platform))
            .map(|entry| AboutLibrary {
                name: entry.name.to_owned(),
                license: entry.license.to_owned(),
                url: entry.url.to_owned(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use onda_domain::privacy_naming::PrivacyNaming;

    use super::*;

    #[test]
    fn about_page_copy_models_and_libraries_win_21() {
        let page = about_page(
            &Catalog::new(None),
            "1.2.3",
            Platform::macos(PrivacyNaming::Accessibility),
        );
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

    #[test]
    fn linux_about_page_uses_linux_libraries_and_gguf_credits() {
        let page = about_page(&Catalog::gguf(), "1.2.3", Platform::linux());
        let names: Vec<&str> = page.libraries.iter().map(|l| l.name.as_str()).collect();
        for linux_lib in [
            "gtk4-rs",
            "Hex desktop integration",
            "libadwaita-rs",
            "transcribe.cpp",
        ] {
            assert!(names.contains(&linux_lib), "missing {linux_lib}: {names:?}");
        }
        for mac_lib in [
            "FluidAudio",
            "keyring",
            "mediaremote-adapter",
            "objc2",
            "security-framework",
            "tracing-oslog",
            "UniFFI",
        ] {
            assert!(
                !names.contains(&mac_lib),
                "unexpected macOS lib {mac_lib}: {names:?}"
            );
        }
        for model in &page.models {
            assert!(
                !model.attribution.contains("CoreML"),
                "GGUF attribution leaked CoreML: {}",
                model.attribution
            );
            assert!(
                model.attribution.contains("GGUF"),
                "expected GGUF attribution: {}",
                model.attribution
            );
        }
    }
}
