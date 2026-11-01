import OndaCore
import SwiftUI

/// Renders `HotkeyPage` plus the shortcut, recorder, and Fn warning from
/// `HotkeyState` (WIN-16, WIN-17, HOT-8..HOT-11, HOT-35). The app-wide
/// `RecorderKeyMonitor` listens while the core says so.
struct HotkeyPageView: View {
    let page: HotkeyPage
    let hotkey: HotkeyState
    let core: OndaCore

    var body: some View {
        Form {
            Section {
                HStack {
                    Text(page.pressToTalkLabel)
                    Spacer()
                    MainWindowKeycaps(labels: hotkey.keycaps)
                }
                if let warning = hotkey.fnWarning {
                    MainWindowFnWarning(message: warning, buttonTitle: hotkey.openKeyboardSettingsTitle) {
                        core.openKeyboardSettings()
                    }
                }
                ShortcutRecorderControls(hotkey: hotkey, core: core)
                Button(page.reset.title) {
                    core.resetHotkey()
                }
                .buttonStyle(.borderless)
                .disabled(!page.reset.enabled)
            } header: {
                Text("Shortcut")
            } footer: {
                Text(page.footer)
            }

            Section {
                Picker(page.activationLabel, selection: activation) {
                    ForEach(page.activationOptions, id: \.mode) { option in
                        Text(option.title).tag(option.mode)
                    }
                }
                MainWindowCaption(text: page.activationDescription)
            } header: {
                Text("Activation")
            }
        }
        .formStyle(.grouped)
    }

    private var activation: Binding<ActivationMode> {
        Binding(get: { page.activation }, set: { core.setActivationMode(mode: $0) })
    }
}
