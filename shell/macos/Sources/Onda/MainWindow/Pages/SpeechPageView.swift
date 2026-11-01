import OndaCore
import SwiftUI

/// Renders `SpeechPage`: language picker, compatibility note, model rows,
/// and the core's alert queue one at a time (WIN-18..WIN-20).
struct SpeechPageView: View {
    let page: SpeechPage
    let core: OndaCore
    /// The alert just answered, hidden until the core sends the next page
    /// so it is not shown twice.
    @State private var answeredAlert: UInt64?

    var body: some View {
        Form {
            Section {
                Picker(page.languageLabel, selection: language) {
                    ForEach(page.languages, id: \.code) { option in
                        Text(option.name).tag(option.code)
                        if option.dividerAfter {
                            Divider()
                        }
                    }
                }
                if let note = page.compatibility {
                    SpeechCompatibilityNote(note: note, core: core)
                }
            } header: {
                Text("Transcription Language")
            }

            Section {
                ForEach(page.models, id: \.id) { row in
                    SpeechModelRow(row: row, core: core)
                }
            } header: {
                Text("Speech Models")
            }
        }
        .formStyle(.grouped)
        .alert(page.alert?.title ?? "", isPresented: alertPresented, presenting: page.alert) { alert in
            if let confirm = alert.confirm {
                Button(confirm, role: .destructive) {
                    answeredAlert = alert.id
                    core.confirmSpeechAlert(id: alert.id)
                }
            }
            Button(alert.dismiss, role: .cancel) {
                answeredAlert = alert.id
                core.dismissSpeechAlert(id: alert.id)
            }
        } message: { alert in
            Text(alert.message)
        }
    }

    private var language: Binding<String> {
        Binding(get: { page.selectedLanguage }, set: { core.selectLanguage(code: $0) })
    }

    /// Buttons answer the alert; the core's next page clears or replaces it.
    private var alertPresented: Binding<Bool> {
        Binding(
            get: { page.alert.map { $0.id != answeredAlert } ?? false },
            set: { _ in }
        )
    }
}

/// The note under the language picker, with its fix button.
private struct SpeechCompatibilityNote: View {
    let note: CompatibilityView
    let core: OndaCore

    var body: some View {
        if note.warning {
            HStack {
                Label(note.text, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
                if let action = note.action {
                    Button(action.title) {
                        perform(action)
                    }
                    .disabled(!action.enabled)
                }
            }
        } else {
            MainWindowCaption(text: note.text)
        }
    }

    private func perform(_ action: NoteActionView) {
        switch action.kind {
        case .use:
            core.selectModel(id: action.modelId)
        case .downloadAndUse:
            core.downloadAndUse(id: action.modelId)
        case .downloading:
            break
        }
    }
}

/// One catalog model with its trailing control (WIN-19).
private struct SpeechModelRow: View {
    let row: ModelRow
    let core: OndaCore

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(row.name)
                        .fontWeight(row.selected ? .semibold : .regular)
                    Text(row.sizeHint)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if let pill = row.recommended {
                        Text(pill)
                            .font(.caption2.weight(.medium))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .background(Color.accentColor.opacity(0.15), in: Capsule())
                            .foregroundStyle(Color.accentColor)
                    }
                }
                MainWindowCaption(text: row.description)
                if let unsupported = row.unsupported {
                    MainWindowCaption(text: unsupported, warning: true)
                }
            }
            Spacer()
            control
            if let tooltip = row.deleteTooltip {
                Button {
                    core.requestDeleteModel(id: row.id)
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .help(tooltip)
                .accessibilityLabel(tooltip)
            }
        }
    }

    @ViewBuilder
    private var control: some View {
        switch row.control {
        case let .downloading(progress, percent, cancelTitle):
            ProgressView(value: progress)
                .progressViewStyle(.linear)
                .frame(width: 80)
            Text(percent)
                .font(.caption)
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .frame(width: 35, alignment: .trailing)
            Button(cancelTitle) {
                core.cancelDownload(id: row.id)
            }
            .buttonStyle(.borderless)
        case .selected:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.tint)
        case let .select(title):
            Button(title) {
                core.selectModel(id: row.id)
            }
            .buttonStyle(.borderless)
        case let .downloadAndUse(title):
            Button(title) {
                core.downloadAndUse(id: row.id)
            }
            .buttonStyle(.bordered)
        }
    }
}
