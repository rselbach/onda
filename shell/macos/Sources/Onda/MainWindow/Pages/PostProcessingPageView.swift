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
                    PostProcessingDetailsView(details: details, core: core)
                }
            } header: {
                Text(verbatim: "Post-Processing")
            }
        }
        .formStyle(.grouped)
    }
}

/// The rows shown while post-processing is on (CLN-26, CLN-27).
private struct PostProcessingDetailsView: View {
    let details: PostProcessingDetails
    let core: OndaCore

    /// the alert on screen; follows the core, cleared locally on dismissal
    @State private var shownError: AlertView?

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
                        ProviderLogo(id: provider.id)
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

/// A provider's mark, `<id>-light.svg` or `<id>-dark.svg` from the bundle
/// (assets/providers). The brand's own light and dark versions are used
/// as is, never tinted; a missing file draws nothing.
private struct ProviderLogo: View {
    let id: String

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let variant = colorScheme == .dark ? "dark" : "light"
        if let url = Bundle.main.url(forResource: "\(id)-\(variant)", withExtension: "svg"),
           let image = NSImage(contentsOf: url) {
            Image(nsImage: image)
                .resizable()
                .scaledToFit()
                .frame(width: 16, height: 16)
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
            Section {
                PostProcessingApiKeyField(key: dialog.key, core: core)
            }

            Section {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                    TextField("", text: $query, prompt: Text(dialog.searchPlaceholder))
                        .labelsHidden()
                        .textFieldStyle(.plain)
                        .onSubmit(useTypedID)
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
