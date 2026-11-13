import AppKit
import Darwin
import OndaCore

/// Now-playing control through mediaremote-adapter's perl helper, as in
/// Jabber (MEDIA-1, MEDIA-5). Every call blocks; the core runs them one at a
/// time, owns the 500 ms query timeout, and decides when to pause and
/// resume. Each helper run has a hard timeout and drains its output, so a
/// stuck helper cannot hang later commands.
final class MediaControl: MediaControlPort, Sendable {
    private static let perl = URL(fileURLWithPath: "/usr/bin/perl")
    private static let libraryName = "libMediaRemoteAdapter.dylib"
    /// Where scripts/bundle-app.sh puts the helper script.
    private static let bundledScript = "MediaRemoteAdapter/run.pl"
    /// SwiftPM's resource bundle next to the dylib in a build folder.
    private static let builtScript = "MediaRemoteAdapter_MediaRemoteAdapter.bundle/Contents/Resources/run.pl"

    private let library: URL
    private let script: URL
    private let muteRecord: URL

    /// Nil when the adapter's dylib or helper script is missing; the core
    /// then never pauses media. `muteRecord` is where a mute is remembered
    /// until it is undone (`OutputMute.restore`).
    init?(muteRecord: URL) {
        guard let library = Self.locateLibrary() else {
            Log.media.error("MediaRemoteAdapter library not found; media pause unavailable")
            return nil
        }
        guard let script = Self.locateScript(near: library) else {
            Log.media.error("MediaRemoteAdapter helper script not found; media pause unavailable")
            return nil
        }
        self.library = library
        self.script = script
        self.muteRecord = muteRecord
    }

    /// The raw now-playing facts; the core decides whether they mean
    /// playing and logs failures (MEDIA-1).
    func nowPlaying() -> NowPlaying {
        let run = helper("get")
        guard run.succeeded else {
            return .failed(reason: run.failure)
        }
        guard let line = NowPlayingLine.first(in: run.output) else {
            return .noTrack
        }
        let payload: NowPlayingLine.Payload
        do {
            payload = try JSONDecoder().decode(NowPlayingLine.self, from: Data(line.utf8)).payload
        } catch {
            return .failed(reason: "could not decode the now-playing track: \(error.localizedDescription)")
        }
        // LOG-2: the player and flags only, never the track title
        Log.media.info(
            "Now playing in \(payload.bundleIdentifier ?? "unknown", privacy: .public): playing \(payload.isPlaying.map(String.init) ?? "nil", privacy: .public), rate \(payload.playbackRate.map { String($0) } ?? "nil", privacy: .public)"
        )
        return .track(isPlaying: payload.isPlaying, playbackRate: payload.playbackRate)
    }

    func pause() -> Bool {
        command("pause")
    }

    func play() -> Bool {
        command("play")
    }

    /// The system play/pause media key, down then up.
    func playPauseKey() -> Bool {
        let key = Int32(NX_KEYTYPE_PLAY)
        let posted = postMediaKey(key, down: true) && postMediaKey(key, down: false)
        if !posted {
            Log.media.warning("Could not post the play/pause key")
        }
        return posted
    }

    func muteAudibleOutput() -> UInt32? {
        OutputMute.muteIfAudible(record: muteRecord)
    }

    func unmuteOutput(device: UInt32) -> Bool {
        OutputMute.unmute(device, record: muteRecord)
    }

    private func command(_ name: String) -> Bool {
        let run = helper(name)
        if !run.succeeded {
            Log.media.warning("Media \(name, privacy: .public) failed: \(run.failure, privacy: .public)")
        }
        return run.succeeded
    }

    private func helper(_ command: String) -> HelperRun {
        HelperRun(executable: Self.perl, arguments: [script.path, library.path, command])
    }

    private func postMediaKey(_ key: Int32, down: Bool) -> Bool {
        let state = Int32(down ? NX_KEYDOWN : NX_KEYUP)
        let event = NSEvent.otherEvent(
            with: .systemDefined,
            location: .zero,
            modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: 0,
            context: nil,
            subtype: Int16(NX_SUBTYPE_AUX_CONTROL_BUTTONS),
            data1: Int((key << 16) | (state << 8)),
            data2: -1
        )
        guard let cgEvent = event?.cgEvent else { return false }
        cgEvent.post(tap: .cghidEventTap)
        return true
    }

    /// The adapter dylib, which Onda links, so dyld has it loaded wherever it
    /// lives (Contents/Frameworks in the bundle, the build folder otherwise).
    private static func locateLibrary() -> URL? {
        for index in 0 ..< _dyld_image_count() {
            guard let name = _dyld_get_image_name(index) else { continue }
            let path = String(cString: name)
            if path.hasSuffix("/\(libraryName)") {
                return URL(fileURLWithPath: path)
            }
        }
        return nil
    }

    private static func locateScript(near library: URL) -> URL? {
        let candidates = [
            Bundle.main.resourceURL?.appending(path: bundledScript),
            library.deletingLastPathComponent().appending(path: builtScript)
        ]
        return candidates.compactMap(\.self).first { FileManager.default.fileExists(atPath: $0.path) }
    }
}

/// The adapter's `get` output; only what MEDIA-1 reads. The title is never
/// decoded (LOG-2).
struct NowPlayingLine: Decodable {
    struct Payload: Decodable {
        let isPlaying: Bool?
        let playbackRate: Double?
        let bundleIdentifier: String?
    }

    let payload: Payload

    /// The first non-blank output line; nil when there is none or the
    /// helper printed `NIL` (no track).
    static func first(in output: Data) -> String? {
        let line = String(decoding: output, as: UTF8.self)
            .split(whereSeparator: \.isNewline)
            .first { !$0.allSatisfy(\.isWhitespace) }?
            .trimmingCharacters(in: .whitespaces)
        guard let line, line != "NIL" else { return nil }
        return line
    }
}

/// One helper run with MEDIA-5's hard timeout: terminate after 5 s, kill
/// 0.5 s later. Both pipes drain while it runs, so a large payload (track
/// artwork) cannot fill a pipe and block the helper.
struct HelperRun {
    static let timeout: TimeInterval = 5
    static let killGrace: TimeInterval = 0.5

    let output: Data
    let failure: String
    let succeeded: Bool

    init(
        executable: URL,
        arguments: [String],
        timeout: TimeInterval = HelperRun.timeout,
        killGrace: TimeInterval = HelperRun.killGrace
    ) {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr
        process.standardInput = FileHandle.nullDevice
        let exited = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in exited.signal() }
        do {
            try process.run()
        } catch {
            output = Data()
            failure = error.localizedDescription
            succeeded = false
            return
        }
        let outputDrain = PipeDrain(stdout.fileHandleForReading)
        let errorDrain = PipeDrain(stderr.fileHandleForReading)

        var timedOut = false
        if exited.wait(timeout: .now() + timeout) == .timedOut {
            timedOut = true
            process.terminate()
            if exited.wait(timeout: .now() + killGrace) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
                exited.wait()
            }
        }
        output = outputDrain.wait()
        let errors = String(decoding: errorDrain.wait(), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        switch (timedOut, process.terminationStatus) {
        case (true, _):
            failure = "helper timed out and was stopped"
            succeeded = false
        case (false, 0):
            failure = ""
            succeeded = true
        case let (false, status):
            failure = "helper exited with status \(status): \(errors)"
            succeeded = false
        }
    }
}

/// Reads a pipe to its end on a background queue.
private final class PipeDrain: @unchecked Sendable {
    private let done = DispatchSemaphore(value: 0)
    /// Written once before `done` signals, read only after it.
    private var data = Data()

    init(_ handle: FileHandle) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            data = handle.readDataToEndOfFile()
            done.signal()
        }
    }

    func wait() -> Data {
        done.wait()
        return data
    }
}
