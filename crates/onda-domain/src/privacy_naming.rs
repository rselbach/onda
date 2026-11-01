//! What copy calls the permission that lets Onda type into other apps
//! (08 "Permissions"). macOS 27 renamed the Accessibility privacy pane to
//! "Device Control and Data Access"; copy uses the name the running OS shows.

/// The name the running OS gives the typing permission. The shell reports
/// it at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrivacyNaming {
    /// "Accessibility" (macOS 26).
    Accessibility,
    /// "Device Control and Data Access" (macOS 27 and later).
    DeviceControlAndDataAccess,
}

impl PrivacyNaming {
    /// The permission's name: the setup checklist row and the General page
    /// permission row title (WIN-5, WIN-8).
    pub fn name(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility",
            Self::DeviceControlAndDataAccess => "Device Control and Data Access",
        }
    }

    /// The setup checklist row's message while the permission is missing
    /// (WIN-5).
    pub fn allow_message(self) -> &'static str {
        match self {
            Self::Accessibility => "Allow Accessibility so Onda can type into other apps.",
            Self::DeviceControlAndDataAccess => {
                "Allow Device Control and Data Access so Onda can type into other apps."
            }
        }
    }

    /// Title of the message shown when typing needs the permission.
    pub fn required_title(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility Permission Required",
            Self::DeviceControlAndDataAccess => "Device Control and Data Access Required",
        }
    }

    /// Body of the message shown when typing needs the permission.
    pub fn required_body(self) -> &'static str {
        match self {
            Self::Accessibility => {
                "Grant accessibility permission before dictating into the active app, or switch output to Copy to clipboard in Settings."
            }
            Self::DeviceControlAndDataAccess => {
                "Grant Device Control and Data Access before dictating into the active app, or switch output to Copy to clipboard in Settings."
            }
        }
    }

    /// Body of the notice after a transcript was copied because the
    /// permission is missing (OUT-1).
    pub fn copied_to_clipboard_body(self) -> &'static str {
        match self {
            Self::Accessibility => {
                "Grant Accessibility permission in Privacy & Security to enable direct typing into apps."
            }
            Self::DeviceControlAndDataAccess => {
                "Grant Device Control and Data Access in Privacy & Security to enable direct typing into apps."
            }
        }
    }

    /// The sentence a lone-modifier registration failure ends with, before
    /// the OS status (HOT-19).
    pub fn lone_modifier_requirement(self) -> &'static str {
        match self {
            Self::Accessibility => {
                "Lone modifier keys require Accessibility permission in System Settings."
            }
            Self::DeviceControlAndDataAccess => {
                "Lone modifier keys require Device Control and Data Access in System Settings."
            }
        }
    }

    /// Onboarding's Continue hint while the permission is missing (ONB-6).
    pub fn onboarding_hint(self) -> &'static str {
        match self {
            Self::Accessibility => "Enable Accessibility or choose clipboard output",
            Self::DeviceControlAndDataAccess => {
                "Enable Device Control and Data Access or choose clipboard output"
            }
        }
    }

    /// The status menu header while the permission is missing (UI-7).
    pub fn needed_header(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility Access Needed",
            Self::DeviceControlAndDataAccess => "Device Control and Data Access Needed",
        }
    }

    /// The tray icon's accessibility label while the permission is missing
    /// (UI-5).
    pub fn needed_label(self) -> &'static str {
        match self {
            Self::Accessibility => "Onda, accessibility access needed",
            Self::DeviceControlAndDataAccess => "Onda, device control and data access needed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases_follow_the_os_naming() {
        type Phrase = fn(PrivacyNaming) -> &'static str;
        struct Case {
            name: &'static str,
            phrase: Phrase,
            want_accessibility: &'static str,
            want_device_control: &'static str,
        }
        let cases = [
            Case {
                name: "name",
                phrase: PrivacyNaming::name,
                want_accessibility: "Accessibility",
                want_device_control: "Device Control and Data Access",
            },
            Case {
                name: "allow message",
                phrase: PrivacyNaming::allow_message,
                want_accessibility: "Allow Accessibility so Onda can type into other apps.",
                want_device_control: "Allow Device Control and Data Access so Onda can type into other apps.",
            },
            Case {
                name: "required title",
                phrase: PrivacyNaming::required_title,
                want_accessibility: "Accessibility Permission Required",
                want_device_control: "Device Control and Data Access Required",
            },
            Case {
                name: "required body",
                phrase: PrivacyNaming::required_body,
                want_accessibility: "Grant accessibility permission before dictating into the active app, or switch output to Copy to clipboard in Settings.",
                want_device_control: "Grant Device Control and Data Access before dictating into the active app, or switch output to Copy to clipboard in Settings.",
            },
            Case {
                name: "OUT-1 copied to clipboard body",
                phrase: PrivacyNaming::copied_to_clipboard_body,
                want_accessibility: "Grant Accessibility permission in Privacy & Security to enable direct typing into apps.",
                want_device_control: "Grant Device Control and Data Access in Privacy & Security to enable direct typing into apps.",
            },
            Case {
                name: "HOT-19 lone modifier requirement",
                phrase: PrivacyNaming::lone_modifier_requirement,
                want_accessibility: "Lone modifier keys require Accessibility permission in System Settings.",
                want_device_control: "Lone modifier keys require Device Control and Data Access in System Settings.",
            },
            Case {
                name: "onboarding hint",
                phrase: PrivacyNaming::onboarding_hint,
                want_accessibility: "Enable Accessibility or choose clipboard output",
                want_device_control: "Enable Device Control and Data Access or choose clipboard output",
            },
            Case {
                name: "status menu header",
                phrase: PrivacyNaming::needed_header,
                want_accessibility: "Accessibility Access Needed",
                want_device_control: "Device Control and Data Access Needed",
            },
            Case {
                name: "tray label",
                phrase: PrivacyNaming::needed_label,
                want_accessibility: "Onda, accessibility access needed",
                want_device_control: "Onda, device control and data access needed",
            },
        ];
        for tc in cases {
            assert_eq!(
                (tc.phrase)(PrivacyNaming::Accessibility),
                tc.want_accessibility,
                "{}",
                tc.name
            );
            let got = (tc.phrase)(PrivacyNaming::DeviceControlAndDataAccess);
            assert_eq!(got, tc.want_device_control, "{}", tc.name);
            assert!(!got.contains("Access Access"), "{}: {got}", tc.name);
        }
    }
}
