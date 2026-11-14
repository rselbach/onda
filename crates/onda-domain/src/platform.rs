//! The facts that differ between the systems Onda runs on. The shell picks
//! its value at start and the core reads facts from it, so no rule asks
//! which system it is on.
//!
//! A new fact is a field here, set in [`Platform::macos`] and
//! [`Platform::linux`].

use crate::hotkey::{KeyNames, Shortcut};
use crate::privacy_naming::PrivacyNaming;

/// What the core knows about the system it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// The system's name in copy: "macOS", "Linux".
    pub os_name: &'static str,
    /// What copy calls the place Onda's menu lives: "menu bar icon",
    /// "application".
    pub tray_term: &'static str,
    /// What copy calls the computer Onda runs on, after "this": "Mac",
    /// "computer".
    pub computer_term: &'static str,
    /// What copy calls the place the system keeps API keys: "system
    /// keychain", "desktop keyring".
    pub secret_store_term: &'static str,
    /// Where copy says a revealed file shows: "Finder", "Folder".
    pub file_manager_term: &'static str,
    /// What copy calls the code the OS gives a failure: "OSStatus". `None`
    /// when the system's code tells the user nothing, so copy leaves it out.
    pub error_code_term: Option<&'static str>,
    /// Whether the system keeps what Onda saves out of its backups, which
    /// copy then says.
    pub excludes_from_backups: bool,
    /// What the OS calls the permission that lets Onda type into other
    /// apps. `None` when the system has no such permission.
    pub typing_permission: Option<PrivacyNaming>,
    /// The OS asks the user before an app may record from the microphone.
    pub microphone_permission: bool,
    /// How the system writes keys and shortcuts: `⌥ Space`,
    /// `Ctrl+Alt+Space`.
    pub key_names: &'static KeyNames,
    /// The dictation shortcut until the user picks one, and what Reset
    /// restores (HOT-2).
    pub default_shortcut: &'static Shortcut,
}

impl Platform {
    /// macOS, which names the typing permission by OS version.
    pub fn macos(typing_permission: PrivacyNaming) -> Self {
        Self {
            os_name: "macOS",
            tray_term: "menu bar icon",
            computer_term: "Mac",
            secret_store_term: "system keychain",
            file_manager_term: "Finder",
            error_code_term: Some("OSStatus"),
            excludes_from_backups: true,
            typing_permission: Some(typing_permission),
            microphone_permission: true,
            key_names: &KeyNames::MACOS,
            default_shortcut: &Shortcut::OPTION_SPACE,
        }
    }

    /// The Linux desktop. It has no privacy permission to grant ahead of
    /// time: the desktop asks for keyboard consent at the first paste.
    pub fn linux() -> Self {
        Self {
            os_name: "Linux",
            tray_term: "application",
            computer_term: "computer",
            secret_store_term: "desktop keyring",
            file_manager_term: "Folder",
            error_code_term: None,
            excludes_from_backups: false,
            typing_permission: None,
            microphone_permission: false,
            key_names: &KeyNames::LINUX,
            default_shortcut: &Shortcut::CONTROL_ALT_SPACE,
        }
    }

    /// The name copy gives the typing permission. A system without the
    /// permission never reports it missing, so the name it gets here is
    /// only read by copy it does not show.
    pub fn privacy_naming(self) -> PrivacyNaming {
        self.typing_permission
            .unwrap_or(PrivacyNaming::Accessibility)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shortcut_reads_the_platforms_way_hot_2() {
        let cases = [
            (
                "macOS",
                Platform::macos(PrivacyNaming::Accessibility),
                "⌥ Space",
            ),
            ("Linux", Platform::linux(), "Ctrl+Alt+Space"),
        ];
        for (name, platform, want) in cases {
            let got = platform.default_shortcut.display(platform.key_names);
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn privacy_naming_follows_the_typing_permission() {
        struct Case {
            name: &'static str,
            platform: Platform,
            want: PrivacyNaming,
        }
        let cases = [
            Case {
                name: "macOS 26",
                platform: Platform::macos(PrivacyNaming::Accessibility),
                want: PrivacyNaming::Accessibility,
            },
            Case {
                name: "macOS 27",
                platform: Platform::macos(PrivacyNaming::DeviceControlAndDataAccess),
                want: PrivacyNaming::DeviceControlAndDataAccess,
            },
            Case {
                name: "Linux has no typing permission",
                platform: Platform::linux(),
                want: PrivacyNaming::Accessibility,
            },
        ];
        for tc in cases {
            assert_eq!(tc.platform.privacy_naming(), tc.want, "{}", tc.name);
        }
    }
}
