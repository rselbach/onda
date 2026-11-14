//! The facts that differ between the systems Onda runs on. The shell picks
//! its value at start and the core reads facts from it, so no rule asks
//! which system it is on.
//!
//! A new fact is a field here, set in [`Platform::macos`] and
//! [`Platform::linux`].

use crate::privacy_naming::PrivacyNaming;

/// What the core knows about the system it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// The system's name in copy: "macOS", "Linux".
    pub os_name: &'static str,
    /// What copy calls the place Onda's menu lives: "menu bar icon",
    /// "application".
    pub tray_term: &'static str,
    /// What the OS calls the permission that lets Onda type into other
    /// apps. `None` when the system has no such permission.
    pub typing_permission: Option<PrivacyNaming>,
}

impl Platform {
    /// macOS, which names the typing permission by OS version.
    pub fn macos(typing_permission: PrivacyNaming) -> Self {
        Self {
            os_name: "macOS",
            tray_term: "menu bar icon",
            typing_permission: Some(typing_permission),
        }
    }

    /// The Linux desktop.
    pub fn linux() -> Self {
        Self {
            os_name: "Linux",
            tray_term: "application",
            typing_permission: None,
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
