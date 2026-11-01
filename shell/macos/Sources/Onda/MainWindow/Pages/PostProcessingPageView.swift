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
        PostProcessingApiKeyField(key: details.key, core: core)

        Picker(details.modelLabel, selection: Binding(
            get: { details.selectedModel },
            set: { core.setCleanupModel(slug: $0) }
        )) {
            ForEach(details.models, id: \.slug) { model in
                Text(model.name).tag(model.slug)
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

/// The OpenRouter API key field (CLN-25). The core owns loading and saving;
/// this view only forwards typing and end of editing. Never log the text.
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
        // turning post-processing off removes the field mid-edit
        .onDisappear { core.apiKeyCommit() }

        if let error = key.error {
            Text(error)
                .font(.caption)
                .foregroundStyle(.red)
        }
    }
}
