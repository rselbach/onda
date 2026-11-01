import AppKit
import OndaCore

/// The shell's half of the updater (UPD-4, UPD-6, UPD-7). The core checks
/// the signed feed, downloads, verifies, and swaps the bundle; the shell
/// describes the running bundle and does what needs AppKit.
final class Updater: UpdaterPort, Sendable {
    /// Sparkle's saved "check automatically" choice; the core adopts it
    /// until it saves its own.
    private static let sparkleAutomaticChecksKey = "SUEnableAutomaticChecks"

    /// The running bundle, or nil for a dev build without a bundle
    /// identifier and build version, which never updates (UPD-4).
    /// `isolated` (an `ONDA_DATA_DIR` verification run) blocks background
    /// checks, keeps the run's data folder across a relaunch, and leaves
    /// the user's Sparkle preference unread.
    static func host(isolated: Bool) -> UpdateHost? {
        let bundle = Bundle.main
        guard bundle.bundleIdentifier != nil,
              let build = bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String,
              bundle.bundleURL.pathExtension == "app"
        else {
            Log.updates.info("No app bundle, bundle identifier, or build version; updater disabled")
            return nil
        }
        let os = ProcessInfo.processInfo.operatingSystemVersion
        let legacy = isolated
            ? nil
            : UserDefaults.standard.object(forKey: sparkleAutomaticChecksKey) as? Bool
        Log.updates.info("Updater enabled for build \(build, privacy: .public), isolated \(isolated, privacy: .public)")
        return UpdateHost(
            bundlePath: bundle.bundleURL.path,
            build: build,
            osVersion: "\(os.majorVersion).\(os.minorVersion).\(os.patchVersion)",
            isolated: isolated,
            legacyAutomaticChecks: legacy
        )
    }

    func openUrl(url: String) {
        DispatchQueue.main.async {
            guard let url = URL(string: url) else { return }
            NSWorkspace.shared.open(url)
        }
    }

    func terminate() {
        DispatchQueue.main.async {
            Log.updates.info("Quitting to finish the update")
            NSApp.terminate(nil)
        }
    }
}
