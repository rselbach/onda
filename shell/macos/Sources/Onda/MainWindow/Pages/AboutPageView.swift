import AppKit
import OndaCore
import SwiftUI

/// Renders `AboutPage` (WIN-21; shell/macos/FFI.md).
struct AboutPageView: View {
    let page: AboutPage
    let core: OndaCore

    var body: some View {
        Form {
            Section {
                HStack(spacing: 12) {
                    Image(nsImage: NSApplication.shared.applicationIconImage)
                        .resizable()
                        .frame(width: 44, height: 44)
                        .accessibilityHidden(true)

                    VStack(alignment: .leading, spacing: 2) {
                        Text(page.appName)
                            .font(.headline)
                        Text(page.version)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                Text(page.tagline)
                    .foregroundStyle(.secondary)
            } header: {
                Text(verbatim: "About")
            }

            Section {
                ForEach(page.models, id: \.self) { model in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.name)
                        Text(model.attribution)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        AboutLink(title: model.licenseName, url: model.licenseUrl)
                            .font(.caption)
                    }
                    .padding(.vertical, 4)
                }
            } header: {
                Text(page.modelsTitle)
            }

            Section {
                ForEach(page.libraries, id: \.self) { library in
                    LabeledContent {
                        Text(library.license)
                    } label: {
                        AboutLink(title: library.name, url: library.url)
                    }
                }
            } header: {
                Text(page.librariesTitle)
            }
        }
        .formStyle(.grouped)
    }
}

/// A link, or plain text when the core's URL does not parse.
private struct AboutLink: View {
    let title: String
    let url: String

    var body: some View {
        if let destination = URL(string: url) {
            Link(title, destination: destination)
        } else {
            Text(title)
        }
    }
}
