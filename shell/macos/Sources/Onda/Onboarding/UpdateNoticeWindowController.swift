import AppKit
import Observation
import OndaCore
import SwiftUI

/// The notice the window shows.
@MainActor
@Observable
final class UpdateNoticeModel {
    var notice: UpdateNotice?
}

/// The model update notice window (DL-14, 07 "Model update notice").
/// Every button and closing the window go to the core, which closes it.
@MainActor
final class UpdateNoticeWindowController: NSObject, NSWindowDelegate {
    private let core: OndaCore
    private let model = UpdateNoticeModel()
    private var window: NSWindow?
    private var closingFromCore = false

    init(core: OndaCore) {
        self.core = core
        super.init()
    }

    /// Renders the events this window owns; ignores the rest.
    func handle(_ event: CoreEvent) {
        switch event {
        case let .showUpdateNotice(notice):
            show(notice)
        case .closeUpdateNotice:
            close()
        default:
            break
        }
    }

    func windowWillClose(_: Notification) {
        guard !closingFromCore else { return }
        // closing the window is "Not Now" (DL-14)
        core.updateNoticeChoice(choice: .notNow)
    }

    private func show(_ notice: UpdateNotice) {
        model.notice = notice
        let window = window ?? makeWindow()
        self.window = window
        window.title = notice.title
        if !window.isVisible {
            window.center()
        }
        // the notice arrives through the core 750 ms after launch,
        // so the cooperative `activate()` would be refused
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }

    private func close() {
        guard let window, window.isVisible else { return }
        closingFromCore = true
        window.close()
        closingFromCore = false
    }

    private func makeWindow() -> NSWindow {
        let core = core
        let root = UpdateNoticeContentView(model: model) { choice in
            core.updateNoticeChoice(choice: choice)
        }
        let hosting = NSHostingController(rootView: root)
        hosting.sizingOptions = .preferredContentSize
        let window = NSWindow(contentViewController: hosting)
        window.styleMask = [.titled, .closable, .fullSizeContentView]
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true
        window.identifier = NSUserInterfaceItemIdentifier("com.rselbach.onda.update-notice")
        window.isReleasedWhenClosed = false
        window.delegate = self
        return window
    }
}

/// Icon, bold title, selectable body with the core's Markdown, and
/// right-aligned buttons; Return is the prominent default.
struct UpdateNoticeContentView: View {
    let model: UpdateNoticeModel
    let choose: (NoticeChoice) -> Void

    var body: some View {
        if let notice = model.notice {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .top, spacing: 14) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                        .font(.system(size: 30, weight: .medium))
                        .foregroundStyle(.tint)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 6) {
                        Text(notice.title)
                            .font(.title2)
                            .fontWeight(.bold)
                            .accessibilityAddTraits(.isHeader)
                        Text(Self.markdown(notice.body))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                }
                HStack(spacing: 8) {
                    Spacer()
                    ForEach(notice.buttons, id: \.choice) { button in
                        Button(button.title) {
                            choose(button.choice)
                        }
                        .modifier(UpdateNoticeShortcut(shortcut: button.shortcut))
                    }
                }
            }
            .padding(24)
            .frame(width: 440)
        }
    }

    /// The body's inline Markdown (bold model name), else the raw text.
    static func markdown(_ body: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .inlineOnlyPreservingWhitespace
        )
        do {
            return try AttributedString(markdown: body, options: options)
        } catch {
            Log.app.error("Could not parse the update notice body: \(error.localizedDescription, privacy: .public)")
            return AttributedString(body)
        }
    }
}

/// Escape cancels; Return is the default, prominent button.
private struct UpdateNoticeShortcut: ViewModifier {
    let shortcut: NoticeShortcut?

    func body(content: Content) -> some View {
        switch shortcut {
        case .return:
            content
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
        case .escape:
            content.keyboardShortcut(.cancelAction)
        case nil:
            content
        }
    }
}
