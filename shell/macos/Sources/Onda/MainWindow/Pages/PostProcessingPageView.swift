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

/// The model sheet: a provider sidebar, and the selected provider's key and
/// models; clicking a model uses it.
private struct ProvidersSheet: View {
    let dialog: ProvidersDialog
    let core: OndaCore

    var body: some View {
        NavigationSplitView {
            List(selection: Binding(
                get: { dialog.selectedProvider },
                set: { id in
                    if let id {
                        core.selectDialogProvider(id: id)
                    }
                }
            )) {
                ForEach(dialog.providers, id: \.id) { provider in
                    Label {
                        Text(provider.name)
                    } icon: {
                        ProviderLogo(id: provider.id, systemSymbol: provider.systemSymbol)
                    }
                    .tag(provider.id)
                }
            }
            .navigationSplitViewColumnWidth(min: 140, ideal: 160)
        } detail: {
            ProviderPane(dialog: dialog, core: core)
        }
        .navigationTitle(dialog.title)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button(dialog.doneTitle) {
                    core.closeProvidersDialog()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .frame(minWidth: 680, minHeight: 520)
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

    /// a List builds only the rows on screen; a grouped Form builds all of
    /// them, which stalls the sheet with a provider's full model list
    var body: some View {
        List {
            if let key = dialog.key {
                Section {
                    PostProcessingApiKeyField(key: key, core: core)
                }
            }

            if let status = dialog.statusMessage {
                Section {
                    Label(status, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.secondary)
                }
            }

            Section {
                // a provider with one fixed model has nothing to search
                if dialog.refreshModels != nil {
                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass")
                            .foregroundStyle(.secondary)
                        TextField("", text: $query, prompt: Text(dialog.searchPlaceholder))
                            .labelsHidden()
                            .textFieldStyle(.plain)
                            .onSubmit(useTypedID)
                    }
                }

                if let id = typedID {
                    Button(action: useTypedID) {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(id)
                                Text(dialog.useIdCaption)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Image(systemName: "return")
                                .foregroundStyle(.secondary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }

                if dialog.modelsLoading {
                    HStack(spacing: 6) {
                        ProgressView()
                            .controlSize(.small)
                        Text(dialog.loadingTitle)
                            .foregroundStyle(.secondary)
                    }
                }

                if let error = dialog.modelsError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            } header: {
                HStack {
                    Text(dialog.modelsLabel)
                    Spacer()
                    if let refresh = dialog.refreshModels {
                        Button(refresh.title) {
                            core.refreshCleanupModels()
                        }
                        .disabled(!refresh.enabled)
                    }
                }
            }

            ForEach(dialog.modelGroups, id: \.title) { group in
                let models = group.models.filter(matches)
                if !models.isEmpty {
                    Section(group.title) {
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
                        }
                    }
                }
            }
        }
        .listStyle(.inset)
    }

    private func useTypedID() {
        guard let id = typedID else { return }
        core.selectCleanupModel(provider: dialog.selectedProvider, slug: id)
        query = ""
    }
}

/// One clickable model row, checked when in use.
private struct ModelRowButton: View {
    let model: CleanupModelOption
    let selected: Bool
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.name)
                    if model.name != model.slug {
                        Text(model.slug)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                if selected {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.tint)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
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
        LabeledContent {
            SecureField(key.placeholder, text: Binding(
                get: { text },
                set: { newValue in
                    // the field writes back text it was given; only typing counts
                    guard newValue != text else { return }
                    text = newValue
                    core.apiKeyEdited(text: newValue)
                }
            ))
            .textContentType(.password)
            .labelsHidden()
            .focused($focused)
            .onSubmit { core.apiKeyCommit() }
        } label: {
            HStack(spacing: 6) {
                Text(key.placeholder)
                if key.loading {
                    ProgressView()
                        .controlSize(.small)
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

        if let error = key.error {
            Text(error)
                .font(.caption)
                .foregroundStyle(.red)
        }
    }
}
