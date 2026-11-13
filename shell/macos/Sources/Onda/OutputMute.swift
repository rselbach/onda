import CoreAudio
import Darwin

/// The default audio output's mute switch, for audio that pause cannot
/// reach. macOS sends now-playing commands to its current player, even while
/// another app is the one playing, and mediaremoted redirects a command
/// aimed at any other app unless the sender holds an Apple-only entitlement.
enum OutputMute {
    private static let system = AudioObjectID(kAudioObjectSystemObject)

    /// Mutes the default output device when another process is playing
    /// through an output and the device is not muted already. Returns the
    /// muted device for `unmute`.
    static func muteIfAudible() -> AudioObjectID? {
        guard let player = playingProcess() else { return nil }
        guard let device = value(kAudioHardwarePropertyDefaultOutputDevice, of: system, AudioObjectID(0)) else {
            Log.media.warning("No default audio output to mute")
            return nil
        }
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

    /// The bundle identifier of a process other than Onda that is running
    /// audio output.
    private static func playingProcess() -> String? {
        var address = address(kAudioHardwarePropertyProcessObjectList)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr else { return nil }
        var processes = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(system, &address, 0, nil, &size, &processes) == noErr else { return nil }
        let playing = processes.first { process in
            value(kAudioProcessPropertyIsRunningOutput, of: process, UInt32(0)) == 1
                && value(kAudioProcessPropertyPID, of: process, pid_t(0)) != getpid()
        }
        guard let playing else { return nil }
        let bundle = value(kAudioProcessPropertyBundleID, of: playing, nil as Unmanaged<CFString>?)
        return bundle??.takeRetainedValue() as String? ?? "unknown"
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
