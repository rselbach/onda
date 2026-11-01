import AppKit
import OndaCore

/// Wires the shell's ports and views to the core. Renders what the core
/// sends; decides nothing (ARCH-3).
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var core: OndaCore?
    private var hotkeys: GlobalHotkeys?
    private var statusMenu: StatusMenu?
    private var overlay: RecordingOverlay?
    private var downloadOverlay: DownloadOverlay?
    private var recorderMonitor: RecorderKeyMonitor?
    private var mainWindow: MainWindowController?
    private var onboarding: OnboardingWindowController?
    private var updateNotice: UpdateNoticeWindowController?
    private let notifier = Notifier()
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)

    func applicationDidFinishLaunching(_: Notification) {
        let dataDir = Self.dataDirectory()
        let hotkeyBridge = HotkeyBridge()
        let permissions = Permissions()
        Log.system.info(
            "Permissions at launch: microphone \(String(describing: permissions.microphone()), privacy: .public), accessibility \(permissions.accessibilityTrusted(), privacy: .public)"
        )
        // an ONDA_DATA_DIR verification run never checks for updates on its own
        let updater = Updater(isolated: ProcessInfo.processInfo.environment["ONDA_DATA_DIR"]?.isEmpty == false)
        let ports = ShellPorts(
            textOutput: TextOutput(),
            permissions: permissions,
            sounds: Sounds(),
            power: Power(),
            hotkeys: hotkeyBridge,
            notifier: notifier,
            files: Files(),
            loginItem: LoginItem(),
            updater: updater,
            media: MediaControl(),
            systemSpeech: SystemSpeech()
        )
        let relay = EventRelay { [weak self] event in
            self?.handle(event)
        }
        let core: OndaCore
        do {
            core = try OndaCore.start(
                ports: ports,
                dataDir: dataDir.path,
                osLanguage: Locale.current.language.languageCode?.identifier,
                appVersion: Self.appVersion(),
                trayTerm: "menu bar icon",
                privacyNaming: Self.privacyNaming(),
                listener: relay
            )
        } catch {
            Log.app.fault("Could not start the core: \(error.localizedDescription, privacy: .public)")
            Self.failToLaunch(error)
            return
        }
        self.core = core
        updater.onChange = { core.updaterChanged() }
        Log.app.info("Onda started with data in \(dataDir.path, privacy: .public)")

        NSApp.mainMenu = AppMenu.make(core: core)
        let menu = StatusMenu(core: core)
        statusItem.menu = menu.menu
        statusMenu = menu
        overlay = RecordingOverlay { core.cancelDictation() }
        downloadOverlay = DownloadOverlay { core.dismissDownloadOverlay() }
        recorderMonitor = RecorderKeyMonitor(core: core)
        mainWindow = MainWindowController(core: core)
        onboarding = OnboardingWindowController(core: core)
        updateNotice = UpdateNoticeWindowController(core: core)

        // the core registers the shortcut once this main-thread turn ends
        let hotkeys = GlobalHotkeys(core: core)
        hotkeyBridge.hotkeys = hotkeys
        hotkeys.start()
        self.hotkeys = hotkeys

        NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification,
            object: nil,
            queue: .main
        ) { _ in
            core.appBecameActive()
        }
        // ONB-13: no onboarding or main window yet, so the UI is ready on
        // the next main-loop turn
        DispatchQueue.main.async {
            core.uiReady()
        }
    }

    func applicationWillTerminate(_: Notification) {
        hotkeys?.stop()
        // DICT-28a: cancels any session; nothing is delivered
        core?.shutdown()
    }

    private func handle(_ event: CoreEvent) {
        // every window sees every event and renders the ones it owns
        mainWindow?.handle(event)
        onboarding?.handle(event)
        updateNotice?.handle(event)
        switch event {
        case let .trayChanged(tray):
            render(tray)
        case let .menuChanged(menu):
            statusMenu?.render(menu)
        case let .overlayChanged(state):
            overlay?.render(state)
        case let .downloadOverlayChanged(state):
            downloadOverlay?.render(state)
        case let .hotkeyChanged(hotkey):
            // the windows render the recorder; one monitor serves both
            recorderMonitor?.render(hotkey)
        case let .appPresenceChanged(regular):
            // UI-2: the core counts the windows; a regular app gets the dock
            // icon, the app switcher, and the app menu
            NSApp.setActivationPolicy(regular ? .regular : .accessory)
        case .activateApp:
            // CLN-25: the Keychain prompt took the focus; the cooperative
            // `activate()` would be refused
            NSApp.activate(ignoringOtherApps: true)
        case let .showAlert(title, message, button):
            // critical messages: a blocking modal alert
            let alert = NSAlert()
            alert.alertStyle = .critical
            alert.messageText = title
            alert.informativeText = message
            alert.addButton(withTitle: button)
            alert.runModal()
        case .historyChanged, .openOnboarding, .closeOnboarding, .onboardingChanged,
             .openMainWindow, .showUpdateNotice, .closeUpdateNotice, .gettingStartedChanged,
             .generalChanged, .hotkeyPageChanged, .speechChanged, .postProcessingChanged,
             .replacementsChanged, .historyPageChanged, .aboutChanged:
            // the window controllers above render these
            break
        }
    }

    /// The bundle's short version (WIN-14, WIN-21).
    private static func appVersion() -> String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
    }

    /// macOS 27 renamed the Accessibility privacy pane to "Device Control
    /// and Data Access"; copy uses the name this OS shows.
    private static func privacyNaming() -> PrivacyNaming {
        ProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 27
            ? .deviceControlAndDataAccess
            : .accessibility
    }

    private func render(_ tray: TrayState) {
        guard let button = statusItem.button else { return }
        button.image = TrayImage.make(for: tray.icon, label: tray.accessibilityLabel)
        button.contentTintColor = tray.redTint ? .systemRed : nil
        button.toolTip = tray.accessibilityLabel
        button.setAccessibilityLabel(tray.accessibilityLabel)
    }

    /// `~/Library/Application Support/Onda`, or `ONDA_DATA_DIR` for isolated
    /// verification runs.
    private static func dataDirectory() -> URL {
        if let override = ProcessInfo.processInfo.environment["ONDA_DATA_DIR"], !override.isEmpty {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appending(path: "Library/Application Support")
        return support.appending(path: "Onda", directoryHint: .isDirectory)
    }

    private static func failToLaunch(_ error: Error) {
        // MSG-3: the copy comes from the core, which never started
        let copy = startFailureAlert()
        let alert = NSAlert()
        alert.alertStyle = .critical
        alert.messageText = copy.title
        alert.informativeText = error.localizedDescription
        alert.addButton(withTitle: copy.button)
        alert.runModal()
        NSApp.terminate(nil)
    }
}

/// Hops core events onto the main queue. The main queue is FIFO, so events
/// keep the order the core emitted them in (ARCH-5).
final class EventRelay: CoreListener, Sendable {
    private let handler: @MainActor @Sendable (CoreEvent) -> Void

    init(handler: @escaping @MainActor @Sendable (CoreEvent) -> Void) {
        self.handler = handler
    }

    func onEvent(event: CoreEvent) {
        DispatchQueue.main.async { [handler] in
            MainActor.assumeIsolated {
                handler(event)
            }
        }
    }
}

/// The app menu bar, visible while Onda is a regular app (UI-2). Edit and
/// Window menus give text fields and windows their standard shortcuts.
@MainActor
enum AppMenu {
    static func make(core: OndaCore) -> NSMenu {
        let main = NSMenu()
        main.addItem(submenu("Onda", appItems(core: core)))
        main.addItem(submenu("Edit", editItems()))
        let window = submenu("Window", windowItems())
        main.addItem(window)
        NSApp.windowsMenu = window.submenu
        return main
    }

    private static func appItems(core: OndaCore) -> [NSMenuItem] {
        // UI-4: Settings… opens the main window on General
        let settings = ActionMenuItem(title: "Settings…") { core.openSettings() }
        settings.keyEquivalent = ","
        let hideOthers = item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h")
        hideOthers.keyEquivalentModifierMask = [.command, .option]
        return [
            settings,
            .separator(),
            item("Hide Onda", #selector(NSApplication.hide(_:)), "h"),
            hideOthers,
            item("Show All", #selector(NSApplication.unhideAllApplications(_:)), ""),
            .separator(),
            item("Quit Onda", #selector(NSApplication.terminate(_:)), "q")
        ]
    }

    private static func editItems() -> [NSMenuItem] {
        let redo = item("Redo", Selector(("redo:")), "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        return [
            item("Undo", Selector(("undo:")), "z"),
            redo,
            .separator(),
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Delete", #selector(NSText.delete(_:)), ""),
            item("Select All", #selector(NSText.selectAll(_:)), "a")
        ]
    }

    private static func windowItems() -> [NSMenuItem] {
        [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:)), ""),
            item("Close", #selector(NSWindow.performClose(_:)), "w"),
            .separator(),
            item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:)), "")
        ]
    }

    private static func submenu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let parent = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        let menu = NSMenu(title: title)
        items.forEach(menu.addItem)
        parent.submenu = menu
        return parent
    }

    private static func item(_ title: String, _ action: Selector, _ key: String) -> NSMenuItem {
        NSMenuItem(title: title, action: action, keyEquivalent: key)
    }
}
