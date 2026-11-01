import AppKit
import OndaCore
import SwiftUI

/// The shortcut recorder's local key monitor, one for the whole app
/// (HOT-8..HOT-11). The core runs the recorder and says when to listen
/// (`HotkeyState.listening`); this installs the monitor exactly then,
/// forwards every key event to the core, and swallows what the core says
/// to swallow. Local monitors see events for every Onda window, so the main
/// window's Hotkey page and onboarding's "Another shortcut" share it.
@MainActor
final class RecorderKeyMonitor {
    private let core: OndaCore
    private var monitor: Any?

    init(core: OndaCore) {
        self.core = core
    }

    /// Follows the core's `listening` flag.
    func render(_ hotkey: HotkeyState) {
        switch (hotkey.listening, monitor != nil) {
        case (true, false):
            install()
        case (false, true):
            remove()
        default:
            break
        }
    }

    private func install() {
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [weak self] event in
            let type = event.type
            let keyCode = Int64(event.keyCode)
            let flags = event.modifierFlags
            let swallow = MainActor.assumeIsolated {
                self?.forward(type: type, keyCode: keyCode, flags: flags) ?? false
            }
            return swallow ? nil : event
        }
        guard monitor == nil else { return }
        Log.hotkey.error("Could not start listening for the shortcut recorder")
        core.hotkeyCaptureFailed()
    }

    private func remove() {
        guard let monitor else { return }
        NSEvent.removeMonitor(monitor)
        self.monitor = nil
    }

    private func forward(type: NSEvent.EventType, keyCode: Int64, flags: NSEvent.ModifierFlags) -> Bool {
        let key = KeyCodes.code(forVirtualKey: keyCode) ?? KeyCodes.unidentified
        let modifiers = HotkeyModifiers(
            control: flags.contains(.control),
            alt: flags.contains(.option),
            shift: flags.contains(.shift),
            meta: flags.contains(.command)
        )
        switch type {
        case .flagsChanged:
            return core.recorderFlagsChanged(key: key, held: modifiers, fnKey: flags.contains(.function))
        case .keyDown:
            return core.recorderKeyDown(key: key, modifiers: modifiers)
        default:
            return false
        }
    }
}

/// The recorder's button, Cancel, help text, and message (HOT-10, HOT-11),
/// all from `HotkeyChanged`; the Hotkey page and onboarding both show it.
struct ShortcutRecorderControls: View {
    let hotkey: HotkeyState
    let core: OndaCore

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Button(hotkey.recordButtonTitle) {
                    core.toggleHotkeyCapture()
                }
                if hotkey.recording {
                    Button(hotkey.cancelButtonTitle) {
                        core.endHotkeyCapture()
                    }
                    .buttonStyle(.borderless)
                }
            }
            MainWindowCaption(text: hotkey.helpText)
            if let message = hotkey.message {
                MainWindowCaption(text: message, warning: true)
            }
        }
    }
}
