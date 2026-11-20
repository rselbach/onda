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
        guard self.state != state else { return }
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
        guard let state else {
            reconcile(menu, with: [
                .quit(key: "quit", title: "Quit Onda")
            ])
            return
        }
        var entries: [MenuEntry] = [
            .header(key: "header", title: state.header)
        ]
        if let fix = state.fixPermissions {
            entries.append(.action(key: "fixPermissions", title: fix.title, enabled: fix.enabled) {
                $0.fixPermissions()
            })
        }
        entries.append(.action(key: "dictation", title: state.dictation.title) {
            $0.toggleDictationFromMenu()
        })
        entries.append(.action(
            key: "lastTranscript",
            title: state.lastTranscript.title,
            subtitle: state.lastTranscript.subtitle,
            enabled: state.lastTranscript.enabled
        ) {
            $0.deliverLastTranscript()
        })
        if let recent = state.recentTranscripts {
            var children: [MenuEntry] = recent.items.enumerated().map { idx, entry in
                let text = entry.text
                return .action(
                    key: "recent-\(idx)",
                    title: entry.title,
                    subtitle: Self.recentSubtitle(entry),
                    enabled: recent.itemsEnabled
                ) {
                    $0.deliverTranscript(text: text)
                }
            }
            children.append(.separator(key: "recent-sep"))
            children.append(.action(key: "recent-showAll", title: recent.showAllTitle) {
                $0.showAllHistory()
            })
            entries.append(.submenu(key: "recentTranscripts", title: recent.title, subtitle: nil, items: children))
        }
        if let retry = state.retryFailedDictation {
            entries.append(.action(key: "retryFailedDictation", title: retry.title, enabled: retry.enabled) {
                $0.retryFailedDictation()
            })
        }
        if let discard = state.discardFailedRecording {
            entries.append(.action(key: "discardFailedRecording", title: discard.title, enabled: discard.enabled) {
                $0.discardFailedDictation()
            })
        }
        if let reload = state.retryModelLoad {
            entries.append(.action(key: "retryModelLoad", title: reload.title, enabled: reload.enabled) {
                $0.retryModelLoad()
            })
        }
        if let redownload = state.redownloadModel {
            entries.append(.action(key: "redownloadModel", title: redownload.title, enabled: redownload.enabled) {
                $0.redownloadModel()
            })
        }

        entries.append(.separator(key: "sep-lang"))
        var langChildren: [MenuEntry] = state.language.items.map { entry in
            let code = entry.code
            return .action(
                key: "lang-\(code)",
                title: entry.name,
                enabled: state.language.itemsEnabled,
                checked: entry.checked
            ) {
                $0.selectLanguageFromMenu(code: code)
            }
        }
        langChildren.append(.separator(key: "lang-sep"))
        langChildren.append(.action(key: "lang-other", title: state.language.otherTitle) {
            $0.chooseOtherLanguage()
        })
        entries.append(.submenu(
            key: "language",
            title: state.language.title,
            subtitle: state.language.subtitle,
            items: langChildren
        ))

        entries.append(.separator(key: "sep-app"))
        entries.append(.action(key: "openApp", title: state.openApp.title, enabled: state.openApp.enabled) {
            $0.openApp()
        })
        entries.append(.action(
            key: "settings",
            title: state.settings.title,
            enabled: state.settings.enabled,
            keyEquivalent: ","
        ) {
            $0.openSettings()
        })

        entries.append(.separator(key: "sep-update"))
        let update = state.update
        entries.append(.action(key: "update", title: update.title, enabled: update.enabled) {
            $0.updateAction(action: update.action)
        })

        entries.append(.separator(key: "sep-quit"))
        entries.append(.quit(key: "quit", title: state.quit.title))

        reconcile(menu, with: entries)
    }

    private func reconcile(_ targetMenu: NSMenu, with entries: [MenuEntry]) {
        let wantedKeys = Set(entries.map(\.key))
        var existingByKey: [String: NSMenuItem] = [:]
        for existing in targetMenu.items {
            if let key = existing.representedObject as? String, wantedKeys.contains(key) {
                existingByKey[key] = existing
            } else {
                targetMenu.removeItem(existing)
            }
        }
        for (index, entry) in entries.enumerated() {
            if let existing = existingByKey[entry.key] {
                let currentIndex = targetMenu.index(of: existing)
                if currentIndex != index {
                    targetMenu.removeItem(existing)
                    targetMenu.insertItem(existing, at: index)
                }
                apply(entry, to: existing)
            } else {
                let created = makeItem(for: entry)
                targetMenu.insertItem(created, at: index)
            }
        }
        while targetMenu.numberOfItems > entries.count {
            targetMenu.removeItem(at: targetMenu.numberOfItems - 1)
        }
    }

    private func makeItem(for entry: MenuEntry) -> NSMenuItem {
        let item: NSMenuItem
        switch entry {
        case .separator:
            item = NSMenuItem.separator()
        case let .header(_, title):
            item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            item.isEnabled = false
        case let .action(_, title, subtitle, enabled, checked, keyEquivalent, action):
            let core = core
            let actionItem = ActionMenuItem(title: title) { action(core) }
            actionItem.subtitle = subtitle
            actionItem.isEnabled = enabled
            actionItem.state = checked ? .on : .off
            actionItem.keyEquivalent = keyEquivalent
            item = actionItem
        case let .submenu(_, title, subtitle, children):
            let parent = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            parent.subtitle = subtitle
            let submenu = NSMenu()
            submenu.autoenablesItems = false
            reconcile(submenu, with: children)
            parent.submenu = submenu
            item = parent
        case let .quit(_, title):
            item = quitItem(title: title)
        }
        item.representedObject = entry.key
        return item
    }

    private func apply(_ entry: MenuEntry, to item: NSMenuItem) {
        switch entry {
        case .separator:
            break
        case let .header(_, title):
            item.title = title
            item.isEnabled = false
        case let .action(_, title, subtitle, enabled, checked, keyEquivalent, action):
            item.title = title
            item.subtitle = subtitle
            item.isEnabled = enabled
            item.state = checked ? .on : .off
            item.keyEquivalent = keyEquivalent
            if let actionItem = item as? ActionMenuItem {
                let core = core
                actionItem.handler = { action(core) }
            }
        case let .submenu(_, title, subtitle, children):
            item.title = title
            item.subtitle = subtitle
            let submenu = item.submenu ?? {
                let created = NSMenu()
                created.autoenablesItems = false
                item.submenu = created
                return created
            }()
            reconcile(submenu, with: children)
        case let .quit(_, title):
            item.title = title
        }
    }

    private enum MenuEntry {
        case separator(key: String)
        case header(key: String, title: String)
        case action(
            key: String,
            title: String,
            subtitle: String? = nil,
            enabled: Bool = true,
            checked: Bool = false,
            keyEquivalent: String = "",
            action: @MainActor (OndaCore) -> Void
        )
        case submenu(key: String, title: String, subtitle: String?, items: [MenuEntry])
        case quit(key: String, title: String)

        var key: String {
            switch self {
            case let .separator(key),
                 let .header(key, _),
                 let .action(key, _, _, _, _, _, _),
                 let .submenu(key, _, _, _),
                 let .quit(key, _):
                key
            }
        }
    }

    /// "<abbreviated date and short time> · <app>" in the user's locale, the
    /// app omitted when the core sends none (HIS-28).
    static func recentSubtitle(_ entry: RecentTranscriptEntry) -> String {
        let date = entry.time.formatted(date: .abbreviated, time: .shortened)
        guard let app = entry.app else { return date }
        return "\(date) · \(app)"
    }

    private func quitItem(title: String) -> NSMenuItem {
        NSMenuItem(title: title, action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
    }
}

/// A menu item that runs a closure.
@MainActor
final class ActionMenuItem: NSMenuItem {
    var handler: @MainActor () -> Void

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
