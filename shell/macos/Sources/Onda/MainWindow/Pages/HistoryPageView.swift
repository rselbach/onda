import AppKit
import OndaCore
import SwiftUI

/// Renders `HistoryPage` and calls its commands (shell/macos/FFI.md).
/// Selection and expansion are the shell's; everything else is the core's.
struct HistoryPageView: View {
    let page: HistoryPage
    let core: OndaCore

    @State private var searchText = ""
    @State private var selection: String?
    // what is on screen; follows the core, cleared locally on dismissal
    @State private var shownDialog: HistoryDialog?
    @State private var shownError: AlertView?
    // a dialog button ran, so dismissal must not also cancel
    @State private var dialogAnswered = false

    var body: some View {
        VStack(spacing: 0) {
            HistorySettingsBar(page: page, core: core)
            Divider()
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .searchable(
            text: Binding(
                get: { searchText },
                set: { query in
                    guard query != searchText else { return }
                    searchText = query
                    core.searchHistory(query: query)
                }
            ),
            placement: .toolbar,
            prompt: Text(page.searchPrompt)
        )
        .toolbar {
            ToolbarItem {
                Button(page.clearButton.title, systemImage: "trash", role: .destructive) {
                    core.requestClearHistory()
                }
                .disabled(!page.clearButton.enabled)
                .help(page.clearTooltip)
            }
        }
        .onAppear { searchText = page.query }
        .onChange(of: page.dialog, initial: true) { _, dialog in
            shownDialog = dialog
        }
        .onChange(of: page.error, initial: true) { _, error in
            shownError = error
        }
        .confirmationDialog(
            shownDialog?.title ?? "",
            isPresented: Binding(
                get: { shownDialog != nil },
                set: { presented in
                    guard !presented else { return }
                    shownDialog = nil
                    // Escape or a click outside: cancel unless a button ran
                    Task { @MainActor in
                        if !dialogAnswered {
                            core.cancelHistoryDialog()
                        }
                        dialogAnswered = false
                    }
                }
            ),
            presenting: shownDialog
        ) { dialog in
            Button(dialog.confirmTitle, role: .destructive) {
                dialogAnswered = true
                core.confirmHistoryDialog()
            }
            Button(dialog.cancelTitle, role: .cancel) {
                dialogAnswered = true
                core.cancelHistoryDialog()
            }
        } message: { dialog in
            Text(dialog.message)
        }
        .alert(
            shownError?.title ?? "",
            isPresented: Binding(
                get: { shownError != nil },
                set: { presented in
                    guard !presented else { return }
                    shownError = nil
                    core.dismissHistoryError()
                }
            ),
            presenting: shownError
        ) { _ in
            Button("OK") {}
        } message: { error in
            Text(error.message)
        }
    }

    @ViewBuilder
    private var content: some View {
        switch page.content {
        case let .empty(title, message, action):
            ContentUnavailableView {
                Label(title, systemImage: "clock.arrow.circlepath")
            } description: {
                Text(message)
            } actions: {
                if let action {
                    Button(action) {
                        core.setHistoryEnabled(on: true)
                    }
                }
            }
        case let .noSearchResults(query):
            ContentUnavailableView.search(text: query)
        case let .sections(sections):
            List(selection: $selection) {
                ForEach(sections, id: \.heading) { section in
                    Section(HistoryDayTitle.text(section.heading)) {
                        ForEach(section.rows, id: \.id) { row in
                            HistoryRowView(
                                page: page,
                                row: row,
                                isExpanded: selection == row.id,
                                onCopy: { core.copyHistoryEntry(id: row.id) }
                            )
                            .tag(row.id)
                            .contextMenu {
                                HistoryContextMenu(row: row, core: core)
                            }
                        }
                    }
                }
            }
            .listStyle(.inset)
            .onCopyCommand {
                // the row's transcript is already trimmed (HIS-25); handing it
                // to SwiftUI makes it the only pasteboard write
                let rows = sections.lazy.flatMap(\.rows)
                guard let row = rows.first(where: { $0.id == selection }) else { return [] }
                return [NSItemProvider(object: (row.transcript ?? "") as NSString)]
            }
            .onDeleteCommand {
                if let selection {
                    core.deleteHistoryEntry(id: selection)
                }
            }
        }
    }
}

/// "Save transcripts", "Keep for", "Keep audio recordings", and the caption
/// (HIS-18).
private struct HistorySettingsBar: View {
    let page: HistoryPage
    let core: OndaCore

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 20) {
                Toggle(page.saveTranscripts.label, isOn: Binding(
                    get: { page.saveTranscripts.on },
                    set: { core.setHistoryEnabled(on: $0) }
                ))
                .disabled(!page.saveTranscripts.enabled)

                Picker(page.retentionLabel, selection: Binding(
                    get: { page.retention },
                    set: { core.setHistoryRetention(retention: $0) }
                )) {
                    ForEach(page.retentionOptions, id: \.retention) { option in
                        Text(option.title).tag(option.retention)
                    }
                }
                .fixedSize()
                .disabled(!page.retentionEnabled)

                Toggle(page.keepAudio.label, isOn: Binding(
                    get: { page.keepAudio.on },
                    set: { core.setHistoryKeepsAudio(on: $0) }
                ))
                .disabled(!page.keepAudio.enabled)
            }

            Text(page.caption)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One saved dictation (HIS-22, HIS-23). Selecting it shows the whole
/// transcript and, when cleanup changed it, the original.
private struct HistoryRowView: View {
    let page: HistoryPage
    let row: HistoryRow
    let isExpanded: Bool
    let onCopy: () -> Void

    @State private var showsOriginal = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                if let icon = row.appId.flatMap(HistoryAppIcons.icon(forBundleID:)) {
                    Image(nsImage: icon)
                        .resizable()
                        .frame(width: 14, height: 14)
                        .accessibilityHidden(true)
                }

                Text(verbatim: "\(row.time.formatted(date: .omitted, time: .shortened)) · \(row.details)")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Spacer()

                Button(action: onCopy) {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                        .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.borderless)
                .help(page.copyLabel)
                .accessibilityLabel(copied ? page.copiedLabel : page.copyLabel)
            }

            if let transcript = row.transcript {
                Text(transcript)
                    .lineLimit(isExpanded ? nil : 3)
            } else {
                Text(page.noTextLabel)
                    .italic()
                    .foregroundStyle(.secondary)
            }

            // a DisclosureGroup inside a List row draws no chevron and never
            // opens, so the disclosure is drawn here
            if isExpanded, let original = row.original {
                Button {
                    withAnimation(.snappy) { showsOriginal.toggle() }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .rotationEffect(.degrees(showsOriginal ? 90 : 0))
                        Text(page.originalLabel)
                            .font(.caption.weight(.medium))
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)

                if showsOriginal {
                    Text(original)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.leading, 14)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private var copied: Bool {
        page.copiedEntry == row.id
    }
}

/// The row's context menu, item for item from the core (HIS-24).
private struct HistoryContextMenu: View {
    let row: HistoryRow
    let core: OndaCore

    var body: some View {
        ForEach(Array(row.menu.enumerated()), id: \.offset) { _, entry in
            switch entry.item {
            case .divider:
                Divider()
            case .copy:
                Button(entry.title) { core.copyHistoryEntry(id: row.id) }
            case .copyOriginal:
                Button(entry.title) { core.copyHistoryOriginal(id: row.id) }
            case .playRecording:
                Button(entry.title) { core.playHistoryRecording(id: row.id) }
            case .showRecording:
                Button(entry.title) { core.showHistoryRecording(id: row.id) }
            case .delete:
                Button(entry.title, role: .destructive) { core.deleteHistoryEntry(id: row.id) }
            }
        }
    }
}

/// Day section titles in the user's locale (HIS-21).
enum HistoryDayTitle {
    static func text(_ heading: DayHeading, calendar: Calendar = .current, locale: Locale = .current) -> String {
        switch heading {
        case let .relative(text):
            return text
        case let .date(year, month, day, inCurrentYear):
            let components = DateComponents(year: Int(year), month: Int(month), day: Int(day), hour: 12)
            guard let date = calendar.date(from: components) else {
                return "\(year)-\(month)-\(day)"
            }
            var style = Date.FormatStyle(date: .omitted, time: .omitted, locale: locale, calendar: calendar)
            style = inCurrentYear
                ? style.weekday(.wide).month(.wide).day()
                : style.month(.wide).day().year()
            return date.formatted(style)
        }
    }
}

/// App icons by bundle identifier, looked up once per app (HIS-22).
@MainActor
private enum HistoryAppIcons {
    private static var icons: [String: NSImage?] = [:]

    static func icon(forBundleID bundleID: String) -> NSImage? {
        if let cached = icons[bundleID] {
            return cached
        }
        let icon = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID)
            .map { NSWorkspace.shared.icon(forFile: $0.path) }
        icons[bundleID] = icon
        return icon
    }
}
