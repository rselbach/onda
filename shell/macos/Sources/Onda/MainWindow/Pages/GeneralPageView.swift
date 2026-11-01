import OndaCore
import SwiftUI

/// Renders `GeneralPage` (WIN-8..WIN-15). Controls show the core's values
/// and send every change back as a command.
struct GeneralPageView: View {
    let page: GeneralPage
    let core: OndaCore

    var body: some View {
        Form {
            Section {
                MainWindowPermissionRow(row: page.microphone, fix: fix)
                MainWindowPermissionRow(row: page.accessibility, fix: fix)
            } header: {
                Text("Permissions")
            }

            Section {
                Picker(page.input.label, selection: inputSelection) {
                    ForEach(page.input.options, id: \.uid) { option in
                        Text(option.name).tag(option.uid)
                    }
                }
                if let warning = page.input.warning {
                    MainWindowCaption(text: warning, warning: true)
                }
                if let caption = page.input.caption {
                    MainWindowCaption(text: caption)
                }
            } header: {
                Text("Microphone")
            }

            Section {
                Picker(page.output.label, selection: outputSelection) {
                    ForEach(page.output.options, id: \.mode) { option in
                        Text(option.title).tag(option.mode)
                    }
                }
                .pickerStyle(.radioGroup)
                if let caption = page.output.caption {
                    MainWindowCaption(text: caption)
                }
            } header: {
                Text("Output")
            }

            Section {
                toggle(page.pauseMedia) { core.setPauseMedia(on: $0) }
                MainWindowCaption(text: page.pauseMediaCaption)
            } header: {
                Text("Media")
            }

            Section {
                toggle(page.sounds) { core.setSounds(on: $0) }
            } header: {
                Text("Feedback")
            }

            Section {
                toggle(page.launchAtLogin) { core.setLaunchAtLogin(on: $0) }
                if let caption = page.launchAtLoginCaption {
                    HStack {
                        MainWindowCaption(text: caption.text, warning: caption.tone == .warning)
                        Spacer()
                        if let button = caption.button {
                            Button(button) {
                                core.openLoginItems()
                            }
                        }
                    }
                }
            } header: {
                Text("Startup")
            }

            Section {
                toggle(page.automaticUpdates) { core.setAutomaticUpdateChecks(on: $0) }
                Button(page.checkForUpdates.title) {
                    core.checkForUpdates()
                }
                .disabled(!page.checkForUpdates.enabled)
            } header: {
                Text("Updates")
            } footer: {
                Text(page.versionFooter)
            }
        }
        .formStyle(.grouped)
    }

    private func fix(_ fix: PermissionFix) {
        core.fixPermission(fix: fix)
    }

    private var inputSelection: Binding<String> {
        Binding(get: { page.input.selectedUid }, set: { core.selectInputDevice(uid: $0) })
    }

    private var outputSelection: Binding<OutputMode> {
        Binding(get: { page.output.selected }, set: { core.setOutputMode(mode: $0) })
    }

    private func toggle(_ view: ToggleView, set: @escaping @Sendable (Bool) -> Void) -> some View {
        Toggle(view.label, isOn: Binding(get: { view.on }, set: set))
            .disabled(!view.enabled)
    }
}
