import AppKit
@preconcurrency import ApplicationServices
import AVFoundation
import OndaCore
import ServiceManagement
@preconcurrency import UserNotifications

/// Privacy permissions (PERM-1..PERM-4). Every read is fresh; the core
/// decides what a status means.
final class Permissions: PermissionsPort, Sendable {
    func microphone() -> MicrophoneStatus {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: .granted
        case .notDetermined: .notDetermined
        case .denied: .denied
        case .restricted: .restricted
        @unknown default: .denied
        }
    }

    func requestMicrophone() -> Bool {
        let answer = Answer(false)
        let answered = DispatchSemaphore(value: 0)
        AVCaptureDevice.requestAccess(for: .audio) { granted in
            answer.set(granted)
            answered.signal()
        }
        answered.wait()
        return answer.get()
    }

    func accessibilityTrusted() -> Bool {
        AXIsProcessTrusted()
    }

    /// PERM-3: the specific privacy pane, else the general one.
    func openSettings(pane: PrivacyPane) {
        let anchor = switch pane {
        case .microphone: "Privacy_Microphone"
        case .accessibility: "Privacy_Accessibility"
        }
        let specific = "x-apple.systempreferences:com.apple.preference.security?\(anchor)"
        let general = "x-apple.systempreferences:com.apple.preference.security?Privacy"
        if !SystemSettings.open(specific) {
            SystemSettings.open(general)
        }
    }
}

/// Opens System Settings deep links.
enum SystemSettings {
    @discardableResult
    static func open(_ link: String) -> Bool {
        guard let url = URL(string: link) else { return false }
        return onMainSync { NSWorkspace.shared.open(url) }
    }

    static func onMainSync<T: Sendable>(_ work: @MainActor @Sendable () -> T) -> T {
        if Thread.isMainThread {
            return MainActor.assumeIsolated { work() }
        }
        return DispatchQueue.main.sync {
            MainActor.assumeIsolated { work() }
        }
    }
}

/// The start and stop cues (SND-1..SND-3). A missing file is logged and
/// silent, never a crash.
final class Sounds: SoundPort, @unchecked Sendable {
    private let queue = DispatchQueue(label: "com.rselbach.onda.sounds")
    /// Only touched on `queue`.
    private var players: [SoundCue: AVAudioPlayer] = [:]

    func play(cue: SoundCue) {
        queue.async { [self] in
            guard let audioPlayer = player(for: cue) else { return }
            audioPlayer.currentTime = 0
            if !audioPlayer.play() {
                players.removeValue(forKey: cue)
                guard let fresh = player(for: cue) else { return }
                fresh.currentTime = 0
                _ = fresh.play()
            }
        }
    }

    private func player(for cue: SoundCue) -> AVAudioPlayer? {
        if let player = players[cue] {
            return player
        }
        let name = switch cue {
        case .start: "dictation_start"
        case .stop: "dictation_stop"
        }
        guard let url = Bundle.main.url(forResource: name, withExtension: "m4a") else {
            Log.system.error("Missing sound \(name, privacy: .public)")
            return nil
        }
        do {
            let player = try AVAudioPlayer(contentsOf: url)
            player.prepareToPlay()
            players[cue] = player
            return player
        } catch {
            Log.system.error("Could not load sound \(name, privacy: .public): \(error.localizedDescription, privacy: .public)")
            return nil
        }
    }
}

/// Idle-sleep prevention while a session runs (POW-1).
final class Power: PowerPort, @unchecked Sendable {
    private let lock = NSLock()
    /// Guarded by `lock`.
    private var activity: (any NSObjectProtocol)?

    func beginPreventIdleSleep(reason: String) {
        let token = ProcessInfo.processInfo.beginActivity(options: .idleSystemSleepDisabled, reason: reason)
        let previous = lock.withLock {
            let previous = activity
            activity = token
            return previous
        }
        if let previous {
            ProcessInfo.processInfo.endActivity(previous)
        }
    }

    func endPreventIdleSleep() {
        let token = lock.withLock {
            let token = activity
            activity = nil
            return token
        }
        if let token {
            ProcessInfo.processInfo.endActivity(token)
        }
    }
}

/// System notifications (MSG-1, MSG-2). Without an app bundle there is no
/// notification center, so authorization reads as unsupported and the core
/// falls back to the overlay notice.
final class Notifier: NSObject, NotifierPort, UNUserNotificationCenterDelegate, Sendable {
    private static let waitTimeout: DispatchTimeInterval = .seconds(5)
    private let center: UNUserNotificationCenter?

    override init() {
        let isBundled = Bundle.main.bundleURL.pathExtension == "app" && Bundle.main.bundleIdentifier != nil
        center = isBundled ? UNUserNotificationCenter.current() : nil
        super.init()
        center?.delegate = self
    }

    func authorization() -> NotificationStatus {
        guard let center else { return .unsupported }
        let status = Answer(UNAuthorizationStatus.notDetermined)
        let read = DispatchSemaphore(value: 0)
        center.getNotificationSettings { settings in
            status.set(settings.authorizationStatus)
            read.signal()
        }
        if read.wait(timeout: .now() + Self.waitTimeout) == .timedOut {
            Log.system.warning("Timed out reading notification authorization")
            return .notDetermined
        }
        return switch status.get() {
        case .authorized: .authorized
        case .provisional: .provisional
        case .ephemeral: .ephemeral
        case .denied: .denied
        case .notDetermined: .notDetermined
        @unknown default: .denied
        }
    }

    func post(title: String, body: String) throws {
        guard let center else {
            throw ShellError.Failed(message: "no notification center outside an app bundle")
        }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        // a fresh identifier per post: notifications stack, none replaces
        // another (MSG-2)
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        let failure = Answer<String?>(nil)
        let posted = DispatchSemaphore(value: 0)
        center.add(request) { error in
            failure.set(error?.localizedDescription)
            posted.signal()
        }
        if posted.wait(timeout: .now() + Self.waitTimeout) == .timedOut {
            throw ShellError.Failed(message: "Timed out posting notification")
        }
        if let message = failure.get() {
            throw ShellError.Failed(message: message)
        }
    }

    /// MSG-2: alert and sound, no badge. The core decides when (ONB-12).
    func requestAuthorization() {
        center?.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if let error {
                Log.system.error("Notification authorization failed: \(error.localizedDescription, privacy: .public)")
            } else {
                Log.system.info("Notification authorization granted: \(granted, privacy: .public)")
            }
        }
    }

    /// MSG-2: banners with sound even while Onda is frontmost.
    func userNotificationCenter(
        _: UNUserNotificationCenter,
        willPresent _: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}

/// File manager services.
final class Files: FilesPort, Sendable {
    func excludeFromBackup(path: String) throws {
        var url = URL(fileURLWithPath: path, isDirectory: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        do {
            try url.setResourceValues(values)
        } catch {
            throw ShellError.Failed(message: error.localizedDescription)
        }
    }

    func revealFile(path: String) throws {
        guard FileManager.default.fileExists(atPath: path) else {
            throw ShellError.Failed(message: "File not found: \(path)")
        }
        let url = URL(fileURLWithPath: path)
        DispatchQueue.main.async {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    }

    func openFile(path: String) throws {
        let url = URL(fileURLWithPath: path)
        guard SystemSettings.onMainSync({ NSWorkspace.shared.open(url) }) else {
            throw ShellError.Failed(message: "Could not open \(path)")
        }
    }
}

/// The main-app login item (LOGIN-1..LOGIN-3). An unbundled build cannot
/// register, so its status reads as unavailable.
final class LoginItem: LoginItemPort, Sendable {
    func status() -> LoginItemRegistryStatus? {
        guard Bundle.main.bundleURL.pathExtension == "app" else { return nil }
        return switch SMAppService.mainApp.status {
        case .enabled: .enabled
        case .notRegistered: .notRegistered
        case .requiresApproval: .requiresApproval
        case .notFound: .notFound
        @unknown default: .notFound
        }
    }

    func setEnabled(enabled: Bool) throws {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            throw ShellError.Failed(message: error.localizedDescription)
        }
    }

    func openSettings() {
        DispatchQueue.main.async {
            SMAppService.openSystemSettingsLoginItems()
        }
    }
}

/// A value handed back from a completion handler on another thread.
final class Answer<Value: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    /// Guarded by `lock`.
    private var value: Value

    init(_ value: Value) {
        self.value = value
    }

    func set(_ newValue: Value) {
        lock.withLock { value = newValue }
    }

    func get() -> Value {
        lock.withLock { value }
    }
}
