import OndaCore
import SwiftUI

/// Renders `ReplacementsPage` and calls its commands (shell/macos/FFI.md).
/// The core debounces and persists every edit (REP-12).
struct ReplacementsPageView: View {
    let page: ReplacementsPage
    let core: OndaCore

    /// field text as typed, by rule id; replaced only when `revision` changes
    @State private var drafts: [String: ReplacementsDraft] = [:]

    var body: some View {
        Form {
            Section {
                if let caption = page.emptyCaption {
                    Text(caption)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                ForEach(page.rows, id: \.id) { row in
                    ReplacementsRuleRow(
                        page: page,
                        triggers: Binding(
                            get: { drafts[row.id]?.triggers ?? row.triggers },
                            set: { text in
                                // fields write back text they were given; only typing counts
                                guard text != (drafts[row.id]?.triggers ?? row.triggers) else { return }
                                drafts[row.id, default: ReplacementsDraft(row)].triggers = text
                                core.setReplacementTriggers(id: row.id, text: text)
                            }
                        ),
                        replacement: Binding(
                            get: { drafts[row.id]?.replacement ?? row.replacement },
                            set: { text in
                                guard text != (drafts[row.id]?.replacement ?? row.replacement) else { return }
                                drafts[row.id, default: ReplacementsDraft(row)].replacement = text
                                core.setReplacementText(id: row.id, text: text)
                            }
                        ),
                        onDelete: { core.deleteReplacementRule(id: row.id) }
                    )
                }
                Button {
                    core.addReplacementRule()
                } label: {
                    Label(page.addTitle, systemImage: "plus")
                }
                .buttonStyle(.borderless)
            } header: {
                Text(page.title)
            } footer: {
                Text(page.footer)
            }
        }
        .formStyle(.grouped)
        .onChange(of: page.revision, initial: true) {
            drafts = Dictionary(
                page.rows.map { ($0.id, ReplacementsDraft($0)) },
                uniquingKeysWith: { first, _ in first }
            )
        }
    }
}

/// One rule's fields as typed.
private struct ReplacementsDraft {
    var triggers: String
    var replacement: String

    init(_ row: ReplacementRowView) {
        triggers = row.triggers
        replacement = row.replacement
    }
}

/// One rule (REP-9): comma-separated triggers above, the replacement below,
/// with a delete button.
private struct ReplacementsRuleRow: View {
    let page: ReplacementsPage
    @Binding var triggers: String
    @Binding var replacement: String
    let onDelete: () -> Void

    var body: some View {
        // A grouped Form turns a field's title into a leading label column
        // and right-aligns the text, so the captions are drawn here and the
        // field titles stay as accessibility labels.
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                Text(page.replaceLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                TextField(page.replaceLabel, text: $triggers, prompt: Text(page.triggersPlaceholder))
                    .textFieldStyle(.roundedBorder)
                    .labelsHidden()
                    .multilineTextAlignment(.leading)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(page.withLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                HStack {
                    TextField(page.withLabel, text: $replacement, prompt: Text(page.replacementPlaceholder))
                        .textFieldStyle(.roundedBorder)
                        .labelsHidden()
                        .multilineTextAlignment(.leading)
                    Button(action: onDelete) {
                        Label(page.deleteTitle, systemImage: "trash")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .help(page.deleteTitle)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
