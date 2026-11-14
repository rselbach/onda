//! The facts that differ between the systems Onda runs on. The shell picks
//! its value at start and the core reads facts from it, so no rule asks
//! which system it is on.
//!
//! A new fact is a field here, set in [`Platform::macos`] and
//! [`Platform::linux`].

use crate::hotkey::{HotkeyPreset, KeyNames, Shortcut};
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
    /// A modifier key on its own, such as Right Option or Fn, can be the
    /// dictation shortcut. Where it cannot, the recorder never commits one
    /// (HOT-9) and the Fn warning never shows (HOT-35).
    pub lone_modifier_hotkeys: bool,
    /// The shortcuts onboarding offers as one click, in display order
    /// (ONB-7).
    pub hotkey_presets: &'static [HotkeyPreset],
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
            lone_modifier_hotkeys: true,
            hotkey_presets: &HotkeyPreset::MACOS,
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
            lone_modifier_hotkeys: false,
            hotkey_presets: &HotkeyPreset::LINUX,
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
    fn presets_are_shortcuts_the_platform_registers_onb_7() {
        let cases = [
            (
                Platform::macos(PrivacyNaming::Accessibility),
                vec!["⌥ Space", "Right Option", "Fn"],
            ),
            (
                Platform::linux(),
                vec!["Ctrl+Alt+Space", "Alt+Super+Space", "Ctrl+Super+Space"],
            ),
        ];
        for (platform, want) in cases {
            let shortcuts: Vec<Shortcut> = platform
                .hotkey_presets
                .iter()
                .map(|preset| preset.shortcut())
                .collect();
            let got: Vec<String> = shortcuts
                .iter()
                .map(|shortcut| shortcut.display(platform.key_names))
                .collect();
            assert_eq!(got, want, "{}", platform.os_name);
            assert_eq!(
                shortcuts.first(),
                Some(platform.default_shortcut),
                "{}: the default leads",
                platform.os_name
            );
            let lone = shortcuts.iter().any(Shortcut::is_lone_modifier);
            assert_eq!(lone, platform.lone_modifier_hotkeys, "{}", platform.os_name);
        }
    }

    #[test]
    fn copy_offers_a_lone_modifier_only_where_one_registers_hot_10() {
        for platform in [
            Platform::macos(PrivacyNaming::Accessibility),
            Platform::linux(),
        ] {
            let names = platform.key_names;
            for copy in [
                names.missing_modifier_message,
                names.recording_help,
                names.hotkey_footer,
            ] {
                let offers = ["lone modifier", "single modifier", "Fn"]
                    .iter()
                    .any(|phrase| copy.contains(phrase));
                assert_eq!(
                    offers, platform.lone_modifier_hotkeys,
                    "{}: {copy}",
                    platform.os_name
                );
            }
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
