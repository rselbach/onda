import OndaCore
import SwiftUI

/// Renders `PostProcessingPage` and calls its commands (shell/macos/FFI.md).
struct PostProcessingPageView: View {
    let page: PostProcessingPage
    let core: OndaCore

    var body: some View {
        Form {
            Section {
                Toggle(page.enabled.label, isOn: Binding(
                    get: { page.enabled.on },
                    set: { core.setCleanupEnabled(on: $0) }
                ))
                .disabled(!page.enabled.enabled)

                if let details = page.details {
                    PostProcessingModelRows(details: details, core: core)
                }
            } header: {
                Text(verbatim: "Post-Processing")
            }

            if let details = page.details {
                PostProcessingPromptSection(prompt: details.prompt, core: core)

                Section {
                    PostProcessingTestRows(details: details, core: core)
                } footer: {
                    if let status = details.prompt.status {
                        Text(status)
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

/// The model in use and its privacy caption (CLN-26).
private struct PostProcessingModelRows: View {
    let details: PostProcessingDetails
    let core: OndaCore

    var body: some View {
        Button {
            core.openProvidersDialog()
        } label: {
            HStack {
                Text(details.modelLabel)
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    Text(details.modelName)
                        .fontWeight(.medium)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(details.providerName)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Image(systemName: "chevron.right")
                    .foregroundStyle(.secondary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // the core owns whether the sheet is open; dismissing it reports back
        .sheet(isPresented: Binding(
            get: { details.dialog != nil },
            set: { presented in
                if !presented {
                    core.closeProvidersDialog()
                }
            }
        )) {
            if let dialog = details.dialog {
                ProvidersSheet(dialog: dialog, core: core)
            }
        }

        Text(details.caption)
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

/// The prompt in use: a row that opens the prompt sheet, over the start of
/// the prompt. One prompt serves every provider and model.
private struct PostProcessingPromptSection: View {
    let prompt: PromptSummaryView
    let core: OndaCore

    var body: some View {
        Section {
            Button {
                core.openPromptEditor()
            } label: {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(prompt.name)
                                .fontWeight(.medium)
                            Text(prompt.subtitle)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Image(systemName: "chevron.right")
                            .foregroundStyle(.secondary)
                    }
                    // an excerpt of what the sheet shows in full; left out
                    // of the row's spoken label, which would run to 400
                    // characters
                    Text(prompt.preview)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                        .accessibilityHidden(true)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            // the core owns the draft; dismissing the sheet discards it
            .sheet(isPresented: Binding(
                get: { prompt.editor != nil },
                set: { presented in
                    if !presented {
                        core.cancelPromptEditor()
                    }
                }
            )) {
                if let editor = prompt.editor {
                    PromptEditorSheet(editor: editor, core: core)
                }
            }
        } header: {
            Text(prompt.title)
        } footer: {
            Text(prompt.caption)
        }
    }
}

/// The prompt sheet: the whole prompt in an editor. The core holds the
/// draft and decides what the buttons may do; this view forwards typing and
/// takes the core's text only when it replaces the draft.
private struct PromptEditorSheet: View {
    let editor: PromptEditorView
    let core: OndaCore

    @State private var text = ""
    @State private var showsHelp = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(editor.title)
                .font(.title3.weight(.semibold))
            Text(editor.intro)
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 6)

            HStack {
                Text(editor.label)
                    .font(.callout.weight(.semibold))
                Spacer()
                Text(editor.badge)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 5))
            }
            .padding(.top, 18)

            TextEditor(text: Binding(
                get: { text },
                set: { newValue in
                    // the editor writes back text it was given; only typing counts
                    guard newValue != text else { return }
                    text = newValue
                    core.promptEdited(text: newValue)
                }
            ))
            .font(.system(size: 12, design: .monospaced))
            .autocorrectionDisabled()
            .frame(height: 280)
            .overlay(
                RoundedRectangle(cornerRadius: MainWindowRadius.small)
                    .stroke(.separator)
            )
            .accessibilityLabel(editor.label)
            .padding(.top, 8)

            HStack(alignment: .firstTextBaseline) {
                Text(editor.status)
                    .foregroundStyle(editor.statusIsError ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
                Text(editor.characterCount)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            .font(.caption)
            .padding(.top, 7)

            DisclosureGroup(editor.helpTitle, isExpanded: $showsHelp) {
                Text(editor.help)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 4)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.top, 14)

            Divider()
                .padding(.top, 18)

            HStack {
                Button(editor.restore.title) {
                    core.restoreDefaultPrompt()
                }
                .disabled(!editor.restore.enabled)
                Spacer()
                Button(editor.cancelTitle) {
                    core.cancelPromptEditor()
                }
                .keyboardShortcut(.cancelAction)
                Button(editor.save.title) {
                    core.savePrompt()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!editor.save.enabled)
            }
            .padding(.top, 14)
        }
        .padding(22)
        .frame(width: 580)
        // replace the text only when the core says so, never while typing
        .onChange(of: editor.revision, initial: true) {
            text = editor.text
        }
    }
}

/// The sample test: its button, result, and error alert (CLN-27).
private struct PostProcessingTestRows: View {
    let details: PostProcessingDetails
    let core: OndaCore

    /// the alert on screen; follows the core, cleared locally on dismissal
    @State private var shownError: AlertView?

    var body: some View {
        HStack {
            Button(details.testButton.title) {
                core.testCleanup()
            }
            .disabled(!details.testButton.enabled)

            if details.testRunning {
                ProgressView()
                    .controlSize(.small)
            }
        }
        .onChange(of: details.testError, initial: true) { _, error in
            shownError = error
        }
        .alert(
            shownError?.title ?? "",
            isPresented: Binding(
                get: { shownError != nil },
                set: { presented in
                    guard !presented else { return }
                    shownError = nil
                    core.dismissCleanupTestError()
                }
            ),
            presenting: shownError
        ) { _ in
            Button("OK") {}
        } message: { error in
            Text(error.message)
        }

        if let result = details.testResult {
            VStack(alignment: .leading, spacing: 4) {
                Text(result.said)
                    .foregroundStyle(.secondary)
                Text(result.result)
            }
            .font(.caption)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The model sheet: a provider sidebar, the selected provider's key and
/// models, and a footer naming what is in use; clicking a model uses it.
private struct ProvidersSheet: View {
    let dialog: ProvidersDialog
    let core: OndaCore

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                ProviderSidebar(dialog: dialog, core: core)
                    .frame(width: 200)
                Divider()
                ProviderPane(dialog: dialog, core: core)
            }
            Divider()
            HStack(spacing: 4) {
                Text(verbatim: "\(dialog.inUseLabel):")
                    .foregroundStyle(.secondary)
                Text(dialog.inUseModel)
                    .fontWeight(.medium)
                    .truncationMode(.middle)
                Text(verbatim: "· \(dialog.inUseProvider)")
                    .foregroundStyle(.secondary)
                    .layoutPriority(1)
                Spacer()
                Button(dialog.doneTitle) {
                    core.closeProvidersDialog()
                }
                .keyboardShortcut(.defaultAction)
            }
            .lineLimit(1)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
        }
        .frame(width: 700, height: 560)
        .navigationTitle(dialog.title)
    }
}

/// The providers this Mac offers; the one in use carries a checkmark.
private struct ProviderSidebar: View {
    let dialog: ProvidersDialog
    let core: OndaCore

    var body: some View {
        List(selection: Binding(
            get: { dialog.selectedProvider },
            set: { id in
                if let id {
                    core.selectDialogProvider(id: id)
                }
            }
        )) {
            ForEach(dialog.providers, id: \.id) { provider in
                HStack {
                    Label {
                        Text(provider.name)
                    } icon: {
                        ProviderLogo(id: provider.id, systemSymbol: provider.systemSymbol)
                    }
                    Spacer()
                    if provider.inUse {
                        Image(systemName: "checkmark")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .accessibilityLabel(dialog.inUseLabel)
                    }
                }
                .tag(provider.id)
            }
        }
        .listStyle(.sidebar)
    }
}

/// A provider's mark from the bundle (assets/providers): `<id>-light.svg`
/// or `<id>-dark.svg`, else `<id>.svg` for a mark that suits both, else
/// the system symbol the core names. Brand marks are used as is, never
/// tinted; with none of these nothing is drawn.
private struct ProviderLogo: View {
    let id: String
    let systemSymbol: String?

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let variant = colorScheme == .dark ? "dark" : "light"
        let url = Bundle.main.url(forResource: "\(id)-\(variant)", withExtension: "svg")
            ?? Bundle.main.url(forResource: id, withExtension: "svg")
        if let url, let image = NSImage(contentsOf: url) {
            Image(nsImage: image)
                .resizable()
                .scaledToFit()
                .frame(width: 16, height: 16)
        } else if let systemSymbol {
            Image(systemName: systemSymbol)
        }
    }
}

/// One provider's key field and its models, one of them in use. A single
/// field filters the list and doubles as the way to use an unlisted ID.
private struct ProviderPane: View {
    let dialog: ProvidersDialog
    let core: OndaCore

    @State private var query = ""

    private var trimmedQuery: String {
        query.trimmingCharacters(in: .whitespaces)
    }

    /// an ID-shaped query that no row has, offered as a model to use
    private var typedID: String? {
        guard dialog.refreshModels != nil else { return nil }
        let id = trimmedQuery
        guard !id.isEmpty, !id.contains(where: \.isWhitespace) else { return nil }
        let listed = dialog.modelGroups.contains { group in
            group.models.contains { $0.slug == id }
        }
        return listed ? nil : id
    }

    private func matches(_ model: CleanupModelOption) -> Bool {
        trimmedQuery.isEmpty
            || model.name.localizedCaseInsensitiveContains(trimmedQuery)
            || model.slug.localizedCaseInsensitiveContains(trimmedQuery)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(dialog.providerName)
                        .font(.title3.weight(.semibold))
                    Text(dialog.privacyNote)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if let key = dialog.key {
                    PostProcessingApiKeyField(key: key, core: core)
                }

                if let status = dialog.statusMessage {
                    Label(status, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }

                // a provider with one fixed model has nothing to search
                if let refresh = dialog.refreshModels {
                    HStack(spacing: 8) {
                        SearchField(
                            text: $query,
                            placeholder: dialog.searchPlaceholder,
                            submit: useTypedID
                        )
                        Button {
                            core.refreshCleanupModels()
                        } label: {
                            Image(systemName: "arrow.clockwise")
                        }
                        .disabled(!refresh.enabled)
                        .help(refresh.title)
                        .accessibilityLabel(refresh.title)
                    }
                }

                switch dialog.modelsNotice {
                case let .needsKey(text):
                    Label(text, systemImage: "key")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                case let .failed(text):
                    Label(text, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.red)
                case nil:
                    EmptyView()
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 6)

            models
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        // each provider starts with its whole list
        .onChange(of: dialog.selectedProvider) {
            query = ""
        }
    }

    /// a List builds only the rows on screen; a provider's full model list
    /// built eagerly stalls the sheet
    private var models: some View {
        List {
            if dialog.modelsLoading {
                HStack(spacing: 6) {
                    ProgressView()
                        .controlSize(.small)
                    Text(dialog.loadingTitle)
                        .foregroundStyle(.secondary)
                }
                .padding(.leading, 8)
                .modelListRow()
            }

            if let id = typedID {
                Button {
                    _ = useTypedID()
                } label: {
                    HStack(spacing: 8) {
                        Text(id)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer()
                        Text(dialog.useIdCaption)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Image(systemName: "return")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.leading, 22)
                    .modelRowHighlight(selected: false)
                }
                .buttonStyle(.plain)
                .modelListRow()
            }

            // headings are rows, not section headers, which pin and draw a rule
            ForEach(dialog.modelGroups, id: \.title) { group in
                let models = group.models.filter(matches)
                if !models.isEmpty {
                    Text(group.title)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.leading, 8)
                        .padding(.top, 8)
                        .accessibilityAddTraits(.isHeader)
                        .modelListRow()
                    ForEach(models, id: \.slug) { model in
                        ModelRowButton(
                            model: model,
                            selected: model.slug == dialog.selectedModel
                        ) {
                            core.selectCleanupModel(
                                provider: dialog.selectedProvider,
                                slug: model.slug
                            )
                        }
                        .modelListRow()
                    }
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    /// Uses the typed ID; false when the field holds none, so Return falls
    /// through to "Done".
    private func useTypedID() -> Bool {
        guard let id = typedID else { return false }
        core.selectCleanupModel(provider: dialog.selectedProvider, slug: id)
        query = ""
        return true
    }
}

/// AppKit's search field, which SwiftUI offers only in a toolbar.
private struct SearchField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    /// Return; true when it was handled.
    let submit: () -> Bool

    func makeNSView(context: Context) -> NSSearchField {
        let field = Field()
        field.delegate = context.coordinator
        // the clear button changes the text without a delegate call
        field.target = context.coordinator
        field.action = #selector(Coordinator.changed(_:))
        return field
    }

    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.parent = self
        field.placeholderString = placeholder
        if field.stringValue != text {
            field.stringValue = text
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    /// Assistive apps set the text through accessibility, which tells
    /// neither the delegate nor the target.
    private final class Field: NSSearchField {
        override func setAccessibilityValue(_ value: Any?) {
            guard let text = value as? String else {
                super.setAccessibilityValue(value)
                return
            }
            // unlike super, this also replaces text being edited
            stringValue = text
            sendAction(action, to: target)
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var parent: SearchField

        init(parent: SearchField) {
            self.parent = parent
        }

        @objc func changed(_ field: NSSearchField) {
            parent.text = field.stringValue
        }

        func controlTextDidChange(_ notification: Notification) {
            if let field = notification.object as? NSSearchField {
                parent.text = field.stringValue
            }
        }

        func control(
            _ control: NSControl,
            textView: NSTextView,
            doCommandBy selector: Selector
        ) -> Bool {
            selector == #selector(NSResponder.insertNewline(_:)) && parent.submit()
        }
    }
}

/// One clickable model row, checked and tinted when in use. The whole slug
/// is its tooltip, since the row shows only the short ID.
private struct ModelRowButton: View {
    let model: CleanupModelOption
    let selected: Bool
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack(spacing: 8) {
                Image(systemName: "checkmark")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tint)
                    .opacity(selected ? 1 : 0)
                    .frame(width: 14)
                Text(model.name)
                    .fontWeight(selected ? .semibold : .regular)
                    .lineLimit(1)
                    .layoutPriority(1)
                if let id = model.shortId {
                    // a long name leaves the ID its start, where the vendor is
                    Text(id)
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .frame(minWidth: 90, alignment: .leading)
                }
                Spacer(minLength: 0)
            }
            .modelRowHighlight(selected: selected)
        }
        .buttonStyle(.plain)
        .help(model.slug)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// A model row's shape: tinted when in use, shaded under the pointer.
private struct ModelRowHighlight: ViewModifier {
    let selected: Bool

    @State private var hovered = false

    private var fill: Color {
        if selected {
            return Color.accentColor.opacity(0.14)
        }
        return hovered ? Color.primary.opacity(0.06) : .clear
    }

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 8)
            .frame(height: 28)
            .background(fill, in: RoundedRectangle(cornerRadius: MainWindowRadius.small))
            .contentShape(Rectangle())
            .onHover { hovered = $0 }
    }
}

private extension View {
    func modelRowHighlight(selected: Bool) -> some View {
        modifier(ModelRowHighlight(selected: selected))
    }

    /// a row of the model list: no separator, and the pane's side margin
    func modelListRow() -> some View {
        listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: 1, leading: 4, bottom: 1, trailing: 4))
    }
}

/// The provider's API key field (CLN-25). The core owns loading and saving;
/// this view only forwards typing and end of editing, and the core saves
/// when the sheet closes. Never log the text.
private struct PostProcessingApiKeyField: View {
    let key: ApiKeyFieldView
    let core: OndaCore

    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(key.label)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 6) {
                    SecureField(key.placeholder, text: Binding(
                        get: { text },
                        set: { newValue in
                            // the field writes back text it was given; only typing counts
                            guard newValue != text else { return }
                            text = newValue
                            core.apiKeyEdited(text: newValue)
                        }
                    ), prompt: Text(key.placeholder))
                        .textContentType(.password)
                        .labelsHidden()
                        .textFieldStyle(.roundedBorder)
                        .focused($focused)
                        .onSubmit { core.apiKeyCommit() }
                    if key.loading {
                        ProgressView()
                            .controlSize(.small)
                    }
                }
                HStack {
                    Text(key.note)
                        .foregroundStyle(.secondary)
                    Spacer()
                    if let url = URL(string: key.getKeyUrl) {
                        Link(destination: url) {
                            Text("\(key.getKeyTitle) \(Image(systemName: "arrow.up.right"))")
                        }
                    }
                }
                .font(.caption)
                if let error = key.error {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }
        }
        // replace the text only when the core says so, never while typing
        .onChange(of: key.revision, initial: true) {
            text = key.text
        }
        .onChange(of: focused) { _, isFocused in
            if !isFocused {
                core.apiKeyCommit()
            }
        }
    }
}
