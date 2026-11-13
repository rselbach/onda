import CoreAudio
import Darwin

/// The default audio output's mute switch, for audio that pause cannot
/// reach. macOS sends now-playing commands to its current player, even while
/// another app is the one playing, and mediaremoted redirects a command
/// aimed at any other app unless the sender holds an Apple-only entitlement.
enum OutputMute {
    private static let system = AudioObjectID(kAudioObjectSystemObject)

    /// Mutes the default output device when another process is playing
    /// through it and the device is not muted already. Returns the muted
    /// device for `unmute`. Nothing is muted while another process records
    /// and plays at once, such as a call, since muting would cut it off.
    static func muteIfAudible() -> AudioObjectID? {
        guard let device = value(kAudioHardwarePropertyDefaultOutputDevice, of: system, AudioObjectID(0)) else {
            Log.media.warning("No default audio output to mute")
            return nil
        }
        guard let player = playingProcess(on: device) else { return nil }
        // a device the user muted stays theirs to unmute
        guard value(kAudioDevicePropertyMute, kAudioDevicePropertyScopeOutput, of: device, UInt32(0)) == 0 else {
            return nil
        }
        guard set(muted: true, on: device) else {
            Log.media.warning("Could not mute audio output \(device, privacy: .public)")
            return nil
        }
        Log.media.info("Muted audio output \(device, privacy: .public): \(player, privacy: .public) is playing")
        return device
    }

    static func unmute(_ device: AudioObjectID) -> Bool {
        let unmuted = set(muted: false, on: device)
        if !unmuted {
            Log.media.warning("Could not unmute audio output \(device, privacy: .public)")
        }
        return unmuted
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
        return objects
    }

    private static func set(muted: Bool, on device: AudioObjectID) -> Bool {
        var address = address(kAudioDevicePropertyMute, kAudioDevicePropertyScopeOutput)
        var value: UInt32 = muted ? 1 : 0
        return AudioObjectSetPropertyData(device, &address, 0, nil, UInt32(MemoryLayout<UInt32>.size), &value) == noErr
    }

    /// A fixed-size property; nil when the object does not have it.
    private static func value<T>(
        _ selector: AudioObjectPropertySelector,
        _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        of object: AudioObjectID,
        _ initial: T
    ) -> T? {
        var address = address(selector, scope)
        var value = initial
        var size = UInt32(MemoryLayout<T>.size)
        let status = withUnsafeMutablePointer(to: &value) {
            AudioObjectGetPropertyData(object, &address, 0, nil, &size, $0)
        }
        return status == noErr ? value : nil
    }

    private static func address(
        _ selector: AudioObjectPropertySelector,
        _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }
}
