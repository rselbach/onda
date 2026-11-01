import AppKit
import OndaCore
import SwiftUI

/// What the main window shows: the selected page and the latest view model
/// of every page. Each page event replaces its page whole.
@MainActor
@Observable
final class MainWindowModel {
    var selection: MainWindowSection = .gettingStarted
    var gettingStarted: GettingStartedPage?
    var general: GeneralPage?
    var hotkey: HotkeyState?
    var hotkeyPage: HotkeyPage?
    var speech: SpeechPage?
    var postProcessing: PostProcessingPage?
    var replacements: ReplacementsPage?
    var history: HistoryPage?
    var about: AboutPage?
    /// The Getting Started try-it field; local text only (WIN-6).
    var tryItText = ""
}

/// The main window (WIN-1..WIN-21): created once and kept (UI-3).
@MainActor
final class MainWindowController: NSObject, NSWindowDelegate {
    private static let frameName = "com.rselbach.onda.main"

    private let core: OndaCore
    private let model = MainWindowModel()
    private let sidebar = mainWindowSidebar()
    private var window: NSWindow?
    /// Between `mainWindowAppeared` and `mainWindowClosed`; a minimized
    /// window stays open.
    private var isOpen = false

    init(core: OndaCore) {
        self.core = core
    }

    /// Renders the events this window owns; ignores the rest.
    func handle(_ event: CoreEvent) {
        switch event {
        case let .openMainWindow(page):
            show(page)
        case let .gettingStartedChanged(page):
            model.gettingStarted = page
        case let .generalChanged(page):
            model.general = page
        case let .hotkeyChanged(hotkey):
            model.hotkey = hotkey
        case let .hotkeyPageChanged(page):
            model.hotkeyPage = page
        case let .speechChanged(page):
            model.speech = page
        case let .postProcessingChanged(page):
            model.postProcessing = page
        case let .replacementsChanged(page):
            model.replacements = page
        case let .historyPageChanged(page):
            model.history = page
        case let .aboutChanged(page):
            model.about = page
        default:
            break
        }
    }

    // MARK: - Window

    /// Shows the window on `page`, selecting it even when already open
    /// (UI-3, UI-4).
    private func show(_ page: MainWindowSection) {
        let window = window ?? makeWindow()
        select(page)
        if !isOpen {
            isOpen = true
            core.mainWindowAppeared()
            core.pageAppeared(page: model.selection)
        }
        if window.isMiniaturized {
            window.deminiaturize(nil)
        }
        // the request arrives through the core, after the menu click, so
        // the cooperative `activate()` is refused and the window opens
        // behind the frontmost app
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }

    private func makeWindow() -> NSWindow {
        let view = MainWindowView(
            model: model,
            core: core,
            sidebar: sidebar,
            actions: MainWindowActions(
                select: { [weak self] page in self?.select(page) }
            )
        )
        let hosting = NSHostingController(rootView: view)
        // SwiftUI drives the toolbar and title so the split view gets the
        // standard unified look
        hosting.sceneBridgingOptions = [.toolbars, .title]
        let window = NSWindow(contentViewController: hosting)
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbarStyle = .unified
        window.identifier = NSUserInterfaceItemIdentifier(Self.frameName)
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.contentMinSize = NSSize(width: 760, height: 540)
        window.setContentSize(NSSize(width: 880, height: 620))
        if !window.setFrameUsingName(Self.frameName) {
            window.center()
        }
        window.setFrameAutosaveName(Self.frameName)
        self.window = window
        return window
    }

    func windowWillClose(_: Notification) {
        isOpen = false
        // every visible page disappears with the window; the core ends a
        // running shortcut capture (HOT-8)
        core.mainWindowClosed()
    }

    // MARK: - Pages

    private func select(_ page: MainWindowSection) {
        let previous = model.selection
        guard page != previous else { return }
        model.selection = page
        guard isOpen else { return }
        core.pageDisappeared(page: previous)
        core.pageAppeared(page: page)
    }
}

/// What the window's views ask the controller to do.
struct MainWindowActions {
    let select: @MainActor (MainWindowSection) -> Void
}
