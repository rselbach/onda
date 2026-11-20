import CoreAudio
import Darwin

/// The default audio output's mute switch, for audio that pause cannot
/// reach. macOS sends now-playing commands to its current player, even while
/// another app is the one playing, and mediaremoted redirects a command
/// aimed at any other app unless the sender holds an Apple-only entitlement.
enum OutputMute {
    private static let system = AudioObjectID(kAudioObjectSystemObject)
    private static let tokens = MutedTokens()
    private static let watcher = DeviceReconnectWatcher()

    /// File in the data folder that lists, one per line, the UIDs of devices
    /// Onda muted, so a run that crashed or quit mid-session is undone at
    /// next launch.
    static let recordName = "muted-output"

    /// Unmutes the devices an earlier run muted and never unmuted. A device
    /// stays listed until it is seen unmuted, so one that is unplugged now
    /// is unmuted when it reconnects or at a later launch.
    static func restore(record: URL) {
        let pending = recorded(record).filter { uid in !restored(uid) }
        do {
            try remember(pending, in: record)
        } catch {
            Log.media.warning("Could not update the muted-output record: \(error.localizedDescription, privacy: .public)")
        }
        if pending.isEmpty {
            watcher.stop()
        } else {
            watcher.watch(record: record)
        }
    }

    /// whether the device `uid` is known unmuted now
    private static func restored(_ uid: String) -> Bool {
        guard let device = device(uid: uid),
              let muted = isMuted(on: device)
        else {
            Log.media.info("Audio output \(uid, privacy: .public) left muted by an earlier run is not available")
            return false
        }
        guard muted else { return true }
        guard set(muted: false, on: device) else {
            Log.media.warning("Could not unmute audio output \(uid, privacy: .public) left muted by an earlier run")
            return false
        }
        Log.media.info("Unmuted audio output \(uid, privacy: .public) that an earlier run left muted")
        return true
    }

    /// Mutes the default output device when another process is playing
    /// through it and the device is not muted already. Returns an opaque mute
    /// token for `unmute`. Nothing is muted while another process records
    /// and plays at once, such as a call, since muting would cut it off.
    static func muteIfAudible(record: URL) -> UInt32? {
        guard let device = value(kAudioHardwarePropertyDefaultOutputDevice, of: system, AudioObjectID(0)) else {
            Log.media.warning("No default audio output to mute")
            return nil
        }
        guard let player = playingProcess(on: device) else { return nil }
        // a device the user muted stays theirs to unmute
        guard isMuted(on: device) == false else {
            return nil
        }
        // remembered first, so a crash right after the mute is undone too;
        // a mute that could not be undone later is not made at all
        guard let uid = uid(of: device) else {
            Log.media.warning("Audio output \(device, privacy: .public) has no UID; not muting")
            return nil
        }
        let before = recorded(record)
        do {
            try remember(before.contains(uid) ? before : before + [uid], in: record)
        } catch {
            Log.media.warning("Could not remember the muted output, so not muting: \(error.localizedDescription, privacy: .public)")
            return nil
        }
        guard set(muted: true, on: device) else {
            Log.media.warning("Could not mute audio output \(device, privacy: .public)")
            do {
                try remember(before, in: record)
            } catch {
                Log.media.warning("Could not update the muted-output record: \(error.localizedDescription, privacy: .public)")
            }
            return nil
        }
        let token = tokens.insert(uid: uid)
        Log.media.info("Muted audio output \(device, privacy: .public): \(player, privacy: .public) is playing")
        return token
    }

    static func unmute(_ token: UInt32, record: URL) -> Bool {
        guard let uid = tokens.take(token) else {
            Log.media.warning("Unknown mute token \(token, privacy: .public)")
            return false
        }
        guard let device = device(uid: uid) else {
            Log.media.warning("Audio output \(uid, privacy: .public) is no longer available to unmute")
            watcher.watch(record: record)
            return false
        }
        let unmuted = set(muted: false, on: device)
        guard unmuted else {
            // the record stays, so reconnect or the next launch tries again
            Log.media.warning("Could not unmute audio output \(device, privacy: .public)")
            watcher.watch(record: record)
            return false
        }
        let remaining = recorded(record).filter { $0 != uid }
        do {
            try remember(remaining, in: record)
        } catch {
            Log.media.warning("Could not update the muted-output record: \(error.localizedDescription, privacy: .public)")
        }
        if remaining.isEmpty {
            watcher.stop()
        }
        return true
    }

    private static func recorded(_ record: URL) -> [String] {
        guard let text = try? String(contentsOf: record, encoding: .utf8) else { return [] }
        return text.split(separator: "\n").map(String.init)
    }

    /// writes `uids`, or deletes the record when there are none
    private static func remember(_ uids: [String], in record: URL) throws {
        guard uids.isEmpty else {
            try uids.joined(separator: "\n").write(to: record, atomically: true, encoding: .utf8)
            return
        }
        do {
            try FileManager.default.removeItem(at: record)
        } catch let error as CocoaError where error.code == .fileNoSuchFile {}
    }

    private static func uid(of device: AudioObjectID) -> String? {
        let uid = value(kAudioDevicePropertyDeviceUID, of: device, nil as Unmanaged<CFString>?)
        return uid??.takeRetainedValue() as String?
    }

    private static func device(uid: String) -> AudioObjectID? {
        var address = address(kAudioHardwarePropertyTranslateUIDToDevice)
        var qualifier = uid as CFString
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        let status = withUnsafeMutablePointer(to: &qualifier) { qualifier in
            AudioObjectGetPropertyData(system, &address, UInt32(MemoryLayout<CFString>.size), qualifier, &size, &device)
        }
        return status == noErr && device != kAudioObjectUnknown ? device : nil
    }

    /// The bundle identifier of a process other than Onda that is playing
    /// through `device`, unless another process is in a call.
    private static func playingProcess(on device: AudioObjectID) -> String? {
        let others = objects(kAudioHardwarePropertyProcessObjectList, of: system).filter { process in
            value(kAudioProcessPropertyPID, of: process, pid_t(0)) != getpid()
                && value(kAudioProcessPropertyIsRunningOutput, of: process, UInt32(0)) == 1
        }
        if let caller = others.first(where: { value(kAudioProcessPropertyIsRunningInput, of: $0, UInt32(0)) == 1 }) {
            Log.media.info("Not muting: \(bundleID(of: caller), privacy: .public) is recording and playing")
            return nil
        }
        let playing = others.first { process in
            objects(kAudioProcessPropertyDevices, kAudioObjectPropertyScopeOutput, of: process).contains(device)
        }
        guard let playing else { return nil }
        return bundleID(of: playing)
    }

    private static func bundleID(of process: AudioObjectID) -> String {
        let bundle = value(kAudioProcessPropertyBundleID, of: process, nil as Unmanaged<CFString>?)
        return bundle??.takeRetainedValue() as String? ?? "unknown"
    }

    /// A list-of-objects property; empty when the object does not have it.
    private static func objects(
        _ selector: AudioObjectPropertySelector,
        _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        of object: AudioObjectID
    ) -> [AudioObjectID] {
        var address = address(selector, scope)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(object, &address, 0, nil, &size) == noErr else { return [] }
        var objects = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &objects) == noErr else { return [] }
        return Array(objects.prefix(Int(size) / MemoryLayout<AudioObjectID>.size))
    }

    private static func isMuted(on device: AudioObjectID) -> Bool? {
        if hasSettableMute(on: device, element: kAudioObjectPropertyElementMain),
           let main = value(
               kAudioDevicePropertyMute,
               kAudioDevicePropertyScopeOutput,
               element: kAudioObjectPropertyElementMain,
               of: device,
               UInt32(0)
           ) {
            return main == 1
        }
        let channels = [AudioObjectPropertyElement(1), AudioObjectPropertyElement(2)].compactMap { element in
            value(kAudioDevicePropertyMute, kAudioDevicePropertyScopeOutput, element: element, of: device, UInt32(0))
        }
        guard !channels.isEmpty else { return nil }
        return channels.contains(1)
    }

    private static func set(muted: Bool, on device: AudioObjectID) -> Bool {
        if hasSettableMute(on: device, element: kAudioObjectPropertyElementMain),
           set(muted: muted, on: device, element: kAudioObjectPropertyElementMain) {
            return true
        }
        let channels = [AudioObjectPropertyElement(1), AudioObjectPropertyElement(2)].filter {
            hasSettableMute(on: device, element: $0)
        }
        guard !channels.isEmpty else { return false }
        var changed: [AudioObjectPropertyElement] = []
        for channel in channels {
            guard set(muted: muted, on: device, element: channel) else {
                if muted {
                    for previous in changed {
                        _ = set(muted: false, on: device, element: previous)
                    }
                }
                return false
            }
            changed.append(channel)
        }
        return true
    }

    private static func hasSettableMute(on device: AudioObjectID, element: AudioObjectPropertyElement) -> Bool {
        var address = address(kAudioDevicePropertyMute, kAudioDevicePropertyScopeOutput, element: element)
        guard AudioObjectHasProperty(device, &address) else { return false }
        var settable = DarwinBoolean(false)
        return AudioObjectIsPropertySettable(device, &address, &settable) == noErr && settable.boolValue
    }

    private static func set(muted: Bool, on device: AudioObjectID, element: AudioObjectPropertyElement) -> Bool {
        var address = address(kAudioDevicePropertyMute, kAudioDevicePropertyScopeOutput, element: element)
        var value: UInt32 = muted ? 1 : 0
        return AudioObjectSetPropertyData(device, &address, 0, nil, UInt32(MemoryLayout<UInt32>.size), &value) == noErr
    }

    /// A fixed-size property; nil when the object does not have it.
    private static func value<T>(
        _ selector: AudioObjectPropertySelector,
        _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        element: AudioObjectPropertyElement = kAudioObjectPropertyElementMain,
        of object: AudioObjectID,
        _ initial: T
    ) -> T? {
        var address = address(selector, scope, element: element)
        var value = initial
        var size = UInt32(MemoryLayout<T>.size)
        let status = withUnsafeMutablePointer(to: &value) {
            AudioObjectGetPropertyData(object, &address, 0, nil, &size, $0)
        }
        return status == noErr ? value : nil
    }

    private static func address(
        _ selector: AudioObjectPropertySelector,
        _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        element: AudioObjectPropertyElement = kAudioObjectPropertyElementMain
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: element)
    }
}

private final class MutedTokens: @unchecked Sendable {
    private let lock = NSLock()
    private var nextToken: UInt32 = 1
    private var uids: [UInt32: String] = [:]

    func insert(uid: String) -> UInt32 {
        lock.withLock {
            var token = nextToken
            while token == 0 || uids[token] != nil {
                token = token &+ 1
            }
            nextToken = token &+ 1
            if nextToken == 0 {
                nextToken = 1
            }
            uids[token] = uid
            return token
        }
    }

    func take(_ token: UInt32) -> String? {
        lock.withLock {
            uids.removeValue(forKey: token)
        }
    }
}

/// Watches CoreAudio device-list changes while muted-output has pending UIDs so
/// reconnecting a disconnected output device unmutes it without restarting Onda.
private final class DeviceReconnectWatcher: @unchecked Sendable {
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "com.rselbach.onda.output-mute-reconnect")
    private var listener: AudioObjectPropertyListenerBlock?
    private var record: URL?

    func watch(record: URL) {
        lock.withLock {
            self.record = record
            guard listener == nil else { return }
            var address = AudioObjectPropertyAddress(
                mSelector: kAudioHardwarePropertyDevices,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: kAudioObjectPropertyElementMain
            )
            let block: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
                guard let self, let url = self.lock.withLock({ self.record }) else { return }
                OutputMute.restore(record: url)
            }
            let status = AudioObjectAddPropertyListenerBlock(
                AudioObjectID(kAudioObjectSystemObject),
                &address,
                queue,
                block
            )
            if status == noErr {
                listener = block
            } else {
                Log.media.warning("Could not install output reconnect listener: \(status, privacy: .public)")
            }
        }
    }

    func stop() {
        lock.withLock {
            record = nil
            guard let block = listener else { return }
            listener = nil
            var address = AudioObjectPropertyAddress(
                mSelector: kAudioHardwarePropertyDevices,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: kAudioObjectPropertyElementMain
            )
            AudioObjectRemovePropertyListenerBlock(
                AudioObjectID(kAudioObjectSystemObject),
                &address,
                queue,
                block
            )
        }
    }
}
