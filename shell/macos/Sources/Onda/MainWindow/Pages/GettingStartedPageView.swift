import OndaCore
import SwiftUI

/// Renders `GettingStartedPage`: the setup checklist and the try-it field
/// (WIN-4..WIN-7).
struct GettingStartedPageView: View {
    let page: GettingStartedPage
    let core: OndaCore
    @Binding var tryItText: String

    var body: some View {
        Form {
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    if let ready = page.readyLine {
                        Label(ready, systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                            .font(.subheadline)
                    }
                    ForEach(page.steps, id: \.step) { row in
                        GettingStartedStepRow(row: row) { action in
                            core.setupAction(action: action)
                        }
                    }
                }
                Button(page.runSetupAgainTitle) {
                    core.runSetupAgain()
                }
            } header: {
                Text("Setup")
            }

            Section {
                HStack(spacing: 6) {
                    Text(page.tryItPrefix)
                        .foregroundStyle(.secondary)
                    MainWindowKeycaps(labels: page.keycaps)
                    Text(page.tryItSuffix)
                        .foregroundStyle(.secondary)
                }
                TextEditor(text: $tryItText)
                    .font(.body)
                    .frame(minHeight: 110)
                    .accessibilityLabel(page.tryItPlaceholder)
                    .overlay(alignment: .topLeading) {
                        if tryItText.isEmpty {
                            Text(page.tryItPlaceholder)
                                .foregroundStyle(.tertiary)
                                .padding(.top, 1)
                                .padding(.leading, 5)
                                .allowsHitTesting(false)
                        }
                    }
            } header: {
                Text("Try It")
            }
        }
        .formStyle(.grouped)
    }
}

/// One checklist row: status icon, title, message while incomplete, and
/// the action button when it needs one (WIN-5).
private struct GettingStartedStepRow: View {
    let row: ChecklistRow
    let perform: (SetupAction) -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: icon)
                .foregroundStyle(color)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 3) {
                Text(row.title)
                    .font(.subheadline)
                    .fontWeight(.medium)
                if let message = row.message {
                    MainWindowCaption(text: message)
                }
                if let button = row.button {
                    Button(button.title) {
                        perform(button.action)
                    }
                    .buttonStyle(.borderless)
                    .font(.caption)
                }
            }
            Spacer(minLength: 0)
        }
    }

    private var icon: String {
        switch row.status {
        case .complete: "checkmark.circle.fill"
        case .needsAction: "exclamationmark.circle.fill"
        case .inProgress: "arrow.down.circle.fill"
        }
    }

    private var color: Color {
        switch row.status {
        case .complete: .green
        case .needsAction: .orange
        case .inProgress: .blue
        }
    }
}
