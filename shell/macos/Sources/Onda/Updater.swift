import Foundation
import OndaCore
import Sparkle

/// The updater: Sparkle 2 with EdDSA-signed appcasts (UPD-1..UPD-4). The
/// feed URL and public key live in Info.plist. Sparkle runs on the main
/// thread; the core reads a snapshot that Sparkle's state changes keep
/// current, and hears about each change through `onChange`.
final class Updater: UpdaterPort, @unchecked Sendable {
    private struct Snapshot: Equatable {
        var canCheck = false
        var automatic = false
        var lastCheck: Date?
    }

    private let lock = NSLock()
    /// Guarded by `lock`.
    private var snapshot = Snapshot()
    /// Guarded by `lock`.
    private var changed: (@Sendable () -> Void)?
    /// Main thread only. Nil for a dev build (UPD-4).
    private var driver: SparkleDriver?

    /// Starts Sparkle unless this is a dev build without a bundle
    /// identifier and build version (UPD-4). `isolated` (an `ONDA_DATA_DIR`
    /// verification run) blocks background checks so a test run never goes
    /// to the network on its own; a user-initiated check still runs.
    @MainActor
    init(isolated: Bool) {
        let bundle = Bundle.main
        guard bundle.bundleIdentifier != nil, bundle.object(forInfoDictionaryKey: "CFBundleVersion") != nil else {
            Log.updates.info("No bundle identifier or build version; updater disabled")
            return
        }
        driver = SparkleDriver(isolated: isolated) { [weak self] in
            self?.publish($0)
        }
    }

    /// Called after every updater state change; set once the core runs.
    var onChange: (@Sendable () -> Void)? {
        get { lock.withLock { changed } }
        set { lock.withLock { changed = newValue } }
    }

    func canCheck() -> Bool {
        lock.withLock { snapshot.canCheck }
    }

    func checkNow() {
        DispatchQueue.main.async { [self] in
            MainActor.assumeIsolated {
                driver?.checkNow()
            }
        }
    }

    func automaticChecks() -> Bool {
        lock.withLock { snapshot.automatic }
    }

    func setAutomaticChecks(enabled: Bool) {
        // the core re-reads right away; Sparkle catches up on the main queue
        lock.withLock { snapshot.automatic = enabled }
        DispatchQueue.main.async { [self] in
            MainActor.assumeIsolated {
                driver?.setAutomaticChecks(enabled)
            }
        }
    }

    func lastCheck() -> Date? {
        lock.withLock { snapshot.lastCheck }
    }

    private func publish(_ state: SparkleDriver.State) {
        let next = Snapshot(canCheck: state.canCheck, automatic: state.automatic, lastCheck: state.lastCheck)
        let notify: (@Sendable () -> Void)? = lock.withLock {
            guard snapshot != next else { return nil }
            snapshot = next
            return changed
        }
        notify?()
    }
}

/// Owns the Sparkle controller on the main thread and reports its state.
@MainActor
private final class SparkleDriver: NSObject, SPUUpdaterDelegate {
    struct State {
        let canCheck: Bool
        let automatic: Bool
        let lastCheck: Date?
    }

    private let isolated: Bool
    private let publish: @MainActor (State) -> Void
    private var controller: SPUStandardUpdaterController?
    private var observations: [NSKeyValueObservation] = []
    /// UPD-2: one background check per run, decided at launch.
    private var launchCheckPending: Bool

    init(isolated: Bool, publish: @escaping @MainActor (State) -> Void) {
        self.isolated = isolated
        self.publish = publish
        launchCheckPending = !isolated
        super.init()
        let controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
        self.controller = controller
        let updater = controller.updater
        launchCheckPending = launchCheckPending && updater.automaticallyChecksForUpdates
        observations = [
            updater.observe(\.canCheckForUpdates, options: [.initial, .new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.refresh() }
            },
            updater.observe(\.automaticallyChecksForUpdates, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.refresh() }
            }
        ]
        Log.updates.info(
            "Updater started: automatic checks \(updater.automaticallyChecksForUpdates, privacy: .public), isolated \(isolated, privacy: .public)"
        )
    }

    func checkNow() {
        controller?.checkForUpdates(nil)
    }

    func setAutomaticChecks(_ enabled: Bool) {
        controller?.updater.automaticallyChecksForUpdates = enabled
        refresh()
    }

    private func refresh() {
        guard let updater = controller?.updater else { return }
        publish(State(
            canCheck: updater.canCheckForUpdates,
            automatic: updater.automaticallyChecksForUpdates,
            lastCheck: updater.lastUpdateCheckDate
        ))
        if launchCheckPending, updater.canCheckForUpdates {
            launchCheckPending = false
            Log.updates.info("Checking for updates in the background")
            updater.checkForUpdatesInBackground()
        }
    }

    // MARK: SPUUpdaterDelegate

    func updater(_: SPUUpdater, mayPerform updateCheck: SPUUpdateCheck) throws {
        guard isolated, updateCheck != .updates else { return }
        Log.updates.info("Skipping a background update check in an isolated run")
        throw NSError(domain: "com.rselbach.onda", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "Background update checks are off for isolated runs"
        ])
    }

    func updaterShouldPromptForPermissionToCheck(forUpdates _: SPUUpdater) -> Bool {
        !isolated
    }

    func updater(_: SPUUpdater, didFinishUpdateCycleFor _: SPUUpdateCheck, error: (any Error)?) {
        if let error {
            Log.updates.error("Update check failed: \(error.localizedDescription, privacy: .public)")
        }
        refresh()
    }
}
