import AppKit
import OndaCore

/// The status menu, rebuilt from the core's `MenuState` each time it opens
/// and whenever a new one arrives while it is open (UI-6..UI-11). Every item
/// and its enablement come from the core; choosing one calls a command.
@MainActor
final class StatusMenu: NSObject, NSMenuDelegate {
    let menu = NSMenu()
    private let core: OndaCore
    private var state: MenuState?
    private var isOpen = false

    init(core: OndaCore) {
        self.core = core
        super.init()
        menu.autoenablesItems = false
        menu.delegate = self
    }

    func render(_ state: MenuState) {
        self.state = state
        if isOpen {
            rebuild()
        }
    }

    func menuNeedsUpdate(_: NSMenu) {
        rebuild()
    }

    func menuWillOpen(_: NSMenu) {
        isOpen = true
        // UI-6: re-read permissions whenever the menu opens
        core.menuWillOpen()
    }

    func menuDidClose(_: NSMenu) {
        isOpen = false
    }

    private func rebuild() {
        menu.removeAllItems()
        guard let state else {
            menu.addItem(quitItem(title: "Quit Onda"))
            return
        }
        let header = NSMenuItem(title: state.header, action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)
        if let fix = state.fixPermissions {
            menu.addItem(item(fix.title, enabled: fix.enabled) { $0.fixPermissions() })
        }

        menu.addItem(item(state.dictation.title) { $0.toggleDictationFromMenu() })
        let last = item(state.lastTranscript.title, enabled: state.lastTranscript.enabled) {
            $0.deliverLastTranscript()
        }
        last.subtitle = state.lastTranscript.subtitle
        menu.addItem(last)
        if let recent = state.recentTranscripts {
            menu.addItem(recentTranscripts(recent))
        }
        if let retry = state.retryFailedDictation {
            menu.addItem(item(retry.title, enabled: retry.enabled) { $0.retryFailedDictation() })
        }
        if let discard = state.discardFailedRecording {
            menu.addItem(item(discard.title, enabled: discard.enabled) { $0.discardFailedDictation() })
        }
        if let reload = state.retryModelLoad {
            menu.addItem(item(reload.title, enabled: reload.enabled) { $0.retryModelLoad() })
        }
        if let redownload = state.redownloadModel {
            menu.addItem(item(redownload.title, enabled: redownload.enabled) { $0.redownloadModel() })
        }

        menu.addItem(.separator())
        menu.addItem(language(state.language))

        menu.addItem(.separator())
        menu.addItem(item(state.openApp.title, enabled: state.openApp.enabled) { $0.openApp() })
        let settings = item(state.settings.title, enabled: state.settings.enabled) { $0.openSettings() }
        settings.keyEquivalent = ","
        menu.addItem(settings)

        menu.addItem(.separator())
        menu.addItem(item(state.checkForUpdates.title, enabled: state.checkForUpdates.enabled) {
            $0.checkForUpdates()
        })

        menu.addItem(.separator())
        menu.addItem(quitItem(title: state.quit.title))
    }

    private func recentTranscripts(_ recent: RecentTranscriptsSubmenu) -> NSMenuItem {
        let parent = NSMenuItem(title: recent.title, action: nil, keyEquivalent: "")
        let submenu = NSMenu()
        submenu.autoenablesItems = false
        for entry in recent.items {
            let text = entry.text
            let child = item(entry.title, enabled: recent.itemsEnabled) { $0.deliverTranscript(text: text) }
            child.subtitle = Self.recentSubtitle(entry)
            submenu.addItem(child)
        }
        submenu.addItem(.separator())
        submenu.addItem(item(recent.showAllTitle) { $0.showAllHistory() })
        parent.submenu = submenu
        return parent
    }

    /// "<abbreviated date and short time> · <app>" in the user's locale, the
    /// app omitted when the core sends none (HIS-28).
    static func recentSubtitle(_ entry: RecentTranscriptEntry) -> String {
        let date = entry.time.formatted(date: .abbreviated, time: .shortened)
        guard let app = entry.app else { return date }
        return "\(date) · \(app)"
    }

    private func language(_ language: LanguageSubmenu) -> NSMenuItem {
        let parent = NSMenuItem(title: language.title, action: nil, keyEquivalent: "")
        parent.subtitle = language.subtitle
        let submenu = NSMenu()
        submenu.autoenablesItems = false
        for entry in language.items {
            let code = entry.code
            let child = item(entry.name, enabled: language.itemsEnabled) {
                $0.selectLanguageFromMenu(code: code)
            }
            child.state = entry.checked ? .on : .off
            submenu.addItem(child)
        }
        submenu.addItem(.separator())
        submenu.addItem(item(language.otherTitle) { $0.chooseOtherLanguage() })
        parent.submenu = submenu
        return parent
    }

    private func item(
        _ title: String,
        enabled: Bool = true,
        action: @escaping @MainActor (OndaCore) -> Void
    ) -> NSMenuItem {
        let core = core
        let item = ActionMenuItem(title: title) { action(core) }
        item.isEnabled = enabled
        return item
    }

    private func quitItem(title: String) -> NSMenuItem {
        NSMenuItem(title: title, action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
    }
}

/// A menu item that runs a closure.
@MainActor
final class ActionMenuItem: NSMenuItem {
    private let handler: @MainActor () -> Void

    init(title: String, handler: @escaping @MainActor () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(fire), keyEquivalent: "")
        target = self
    }

    @available(*, unavailable)
    required init(coder _: NSCoder) {
        fatalError("not used")
    }

    @objc private func fire() {
        handler()
    }
}
