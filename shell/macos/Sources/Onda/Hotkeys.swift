@preconcurrency import ApplicationServices
import Carbon
import CoreGraphics
import OndaCore

/// The dictation shortcut and the session-only Escape (HOT-13, HOT-14,
/// HOT-34). Combos register through Carbon; lone modifiers through a session
/// event tap. Every raw event goes straight into the core's ordered queue
/// (HOT-20a); the core runs the gesture, the re-enable bound, the retry
/// episode, and HOT-18's pending key-up (ARCH-3).
@MainActor
final class GlobalHotkeys {
    private static let signature: OSType = 0x4F4E_4441 // "ONDA"
    /// The status for a tap that cannot be created or was given up on.
    private static let tapFailure = OSStatus(eventNotHandledErr)

    private let core: OndaCore
    private var handler: EventHandlerRef?
    private var dictation: EventHotKeyRef?
    private var cancels: [EventHotKeyRef] = []
    private var tap: CFMachPort?
    private var tapSource: CFRunLoopSource?

    init(core: OndaCore) {
        self.core = core
    }

    /// Installs the Carbon event handler. The core registers the shortcut
    /// itself through `HotkeyBridge`.
    func start() {
        let installed = installHandler()
        guard installed == noErr else {
            Log.hotkey.error("Could not install the hotkey handler: \(installed, privacy: .public)")
            core.hotkeyRegistrationFailed(status: installed)
            return
        }
    }

    func stop() {
        unregister()
        unregisterCancel()
        if let handler {
            RemoveEventHandler(handler)
        }
        handler = nil
    }

    func register(_ shortcut: HotkeyShortcut) -> RegistrationResult {
        unregister()
        let status = shortcut.loneModifier ? openTap() : registerCombo(shortcut)
        guard status == noErr else {
            Log.hotkey.error("Could not register \(shortcut.display, privacy: .public): \(status, privacy: .public)")
            return .failed(status: status)
        }
        Log.hotkey.info("Registered global hotkey \(shortcut.display, privacy: .public)")
        return .registered
    }

    func unregister() {
        if let dictation {
            UnregisterEventHotKey(dictation)
        }
        dictation = nil
        closeTap()
    }

    func reenableStream() {
        guard let tap else { return }
        CGEvent.tapEnable(tap: tap, enable: true)
    }

    func registerCancel(_ modifiers: [HotkeyModifiers]) {
        guard cancels.isEmpty else { return }
        let id = EventHotKeyID(signature: Self.signature, id: cancelHotkeyId())
        for set in modifiers {
            var ref: EventHotKeyRef?
            let status = RegisterEventHotKey(
                UInt32(kVK_Escape), Self.carbonModifiers(set), id, GetApplicationEventTarget(), 0, &ref
            )
            guard status == noErr, let ref else {
                // HOT-34: logged only; the overlay's cancel button still works
                Log.hotkey.error("Could not register Escape: \(status, privacy: .public)")
                continue
            }
            cancels.append(ref)
        }
        let count = cancels.count
        Log.hotkey.info("Registered Escape for the session (\(count, privacy: .public) variants)")
    }

    func unregisterCancel() {
        for ref in cancels {
            UnregisterEventHotKey(ref)
        }
        cancels = []
    }

    private func registerCombo(_ shortcut: HotkeyShortcut) -> OSStatus {
        guard let keyCode = KeyCodes.virtualKey(forCode: shortcut.key) else {
            Log.hotkey.error("No macOS key for \(shortcut.key, privacy: .public)")
            return OSStatus(paramErr)
        }
        let modifiers = HotkeyModifiers(
            control: shortcut.control,
            alt: shortcut.alt,
            shift: shortcut.shift,
            meta: shortcut.meta
        )
        let id = EventHotKeyID(signature: Self.signature, id: dictationHotkeyId())
        let status = RegisterEventHotKey(
            keyCode, Self.carbonModifiers(modifiers), id, GetApplicationEventTarget(), 0, &dictation
        )
        if status != noErr {
            dictation = nil
        }
        return status
    }

    /// HOT-14: a `.defaultTap` needs only Accessibility (`.listenOnly`
    /// would also ask for Input Monitoring), so check Accessibility first.
    /// A default tap can swallow input, so the callback returns every
    /// event unchanged (HOT-15).
    private func openTap() -> OSStatus {
        guard AXIsProcessTrusted() else {
            Log.hotkey.error("Lone-modifier shortcuts need Accessibility, which is not granted")
            return Self.tapFailure
        }
        let mask = CGEventMask((1 << CGEventType.flagsChanged.rawValue) | (1 << CGEventType.keyDown.rawValue))
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: mask,
            callback: Self.tapCallback,
            userInfo: context
        ) else {
            Log.hotkey.error("Could not create the event tap")
            return Self.tapFailure
        }
        guard let source = CFMachPortCreateRunLoopSource(nil, tap, 0) else {
            CGEvent.tapEnable(tap: tap, enable: false)
            CFMachPortInvalidate(tap)
            Log.hotkey.error("Could not create the event tap's run-loop source")
            return Self.tapFailure
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        self.tap = tap
        tapSource = source
        return noErr
    }

    /// The tap's callback: reads only plain values and returns the same
    /// event unchanged, never dropping, altering, or replacing it (HOT-15).
    /// `context` is the unretained `GlobalHotkeys`.
    nonisolated static let tapCallback: CGEventTapCallBack = { _, type, event, context in
        let keyCode = event.getIntegerValueField(.keyboardEventKeycode)
        let flags = event.flags.rawValue
        if let context {
            // the tap runs on the main run loop, where `GlobalHotkeys` lives
            nonisolated(unsafe) let context = context
            MainActor.assumeIsolated {
                let hotkeys = Unmanaged<GlobalHotkeys>.fromOpaque(context).takeUnretainedValue()
                hotkeys.observe(type: type, keyCode: keyCode, flags: flags)
            }
        }
        return Unmanaged.passUnretained(event)
    }

    private func closeTap() {
        guard let tap else { return }
        CGEvent.tapEnable(tap: tap, enable: false)
        // invalidating stops callbacks still queued for this tap
        CFMachPortInvalidate(tap)
        if let tapSource {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), tapSource, .commonModes)
        }
        self.tap = nil
        tapSource = nil
    }

    /// Forwards one tap event as raw facts; direction comes from the
    /// event's own flags (HOT-16).
    private func observe(type: CGEventType, keyCode: Int64, flags: UInt64) {
        switch type {
        case .tapDisabledByTimeout, .tapDisabledByUserInput:
            core.onHotkeyEvent(event: .streamDisabled(status: Self.tapFailure))
        case .flagsChanged:
            guard let change = KeyCodes.flagsChange(virtualKey: keyCode, flags: CGEventFlags(rawValue: flags)) else {
                return
            }
            core.onHotkeyEvent(event: change)
        case .keyDown:
            let key = KeyCodes.code(forVirtualKey: keyCode) ?? KeyCodes.unidentified
            core.onHotkeyEvent(event: .keyDown(key: key))
        default:
            break
        }
    }

    private static func carbonModifiers(_ modifiers: HotkeyModifiers) -> UInt32 {
        var carbon: UInt32 = 0
        if modifiers.control {
            carbon |= UInt32(controlKey)
        }
        if modifiers.alt {
            carbon |= UInt32(optionKey)
        }
        if modifiers.shift {
            carbon |= UInt32(shiftKey)
        }
        if modifiers.meta {
            carbon |= UInt32(cmdKey)
        }
        return carbon
    }

    private func installHandler() -> OSStatus {
        var types = [
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased))
        ]
        let context = Unmanaged.passUnretained(self).toOpaque()
        return InstallEventHandler(GetApplicationEventTarget(), { _, event, context -> OSStatus in
            guard let event, let context else { return OSStatus(eventNotHandledErr) }
            var id = EventHotKeyID()
            let read = GetEventParameter(
                event,
                EventParamName(kEventParamDirectObject),
                EventParamType(typeEventHotKeyID),
                nil,
                MemoryLayout<EventHotKeyID>.size,
                nil,
                &id
            )
            guard read == noErr else { return OSStatus(eventNotHandledErr) }
            let pressed = GetEventKind(event) == UInt32(kEventHotKeyPressed)
            let hotkeyID = id.id
            MainActor.assumeIsolated {
                let hotkeys = Unmanaged<GlobalHotkeys>.fromOpaque(context).takeUnretainedValue()
                hotkeys.deliver(id: hotkeyID, pressed: pressed)
            }
            return noErr
        }, types.count, &types, context, &handler)
    }

    private func deliver(id: UInt32, pressed: Bool) {
        guard let input = routeGlobalHotkey(id: id, pressed: pressed) else { return }
        core.onHotkeyEvent(event: input)
    }
}

/// The core's hotkey requests, hopped to the main thread in call order.
/// `register` waits for its result; the core never calls from the main
/// thread.
final class HotkeyBridge: HotkeyPort, @unchecked Sendable {
    /// Set once on the main thread before the core can call.
    nonisolated(unsafe) weak var hotkeys: GlobalHotkeys?

    func registerShortcut(shortcut: HotkeyShortcut) -> RegistrationResult {
        let register = { @MainActor [self] () -> RegistrationResult in
            hotkeys?.register(shortcut) ?? .failed(status: OSStatus(paramErr))
        }
        if Thread.isMainThread {
            return MainActor.assumeIsolated { register() }
        }
        return DispatchQueue.main.sync {
            MainActor.assumeIsolated { register() }
        }
    }

    func unregisterShortcut() {
        onMain { $0.unregister() }
    }

    func reenableStream() {
        onMain { $0.reenableStream() }
    }

    func registerCancel(modifiers: [HotkeyModifiers]) throws {
        onMain { $0.registerCancel(modifiers) }
    }

    func unregisterCancel() {
        onMain { $0.unregisterCancel() }
    }

    /// HOT-35: System Settings > Keyboard > "Press 🌐 key to". Synchronizes
    /// the domain first, as Jabber does, so the re-read on activation sees a
    /// change made in System Settings meanwhile.
    func globeKeyUsage() -> Int64? {
        let domain = "com.apple.HIToolbox" as CFString
        if !CFPreferencesAppSynchronize(domain) {
            Log.hotkey.error("Could not synchronize the com.apple.HIToolbox preferences")
        }
        return (CFPreferencesCopyAppValue("AppleFnUsageType" as CFString, domain) as? NSNumber)?.int64Value
    }

    /// HOT-35: the pane with "Press 🌐 key to".
    func openKeyboardSettings() {
        SystemSettings.open("x-apple.systempreferences:com.apple.Keyboard-Settings.extension")
    }

    private func onMain(_ body: @escaping @MainActor (GlobalHotkeys) -> Void) {
        DispatchQueue.main.async { [self] in
            MainActor.assumeIsolated {
                if let hotkeys {
                    body(hotkeys)
                }
            }
        }
    }
}

/// W3C `KeyboardEvent.code` names to macOS virtual key codes (HOT-1: each
/// shell maps the portable codes to its own).
enum KeyCodes {
    /// W3C's name for a key without a code.
    static let unidentified = "Unidentified"

    static func virtualKey(forCode code: String) -> UInt32? {
        table[code].map(UInt32.init)
    }

    static func code(forVirtualKey key: Int64) -> String? {
        let key = Int(key)
        return modifierKeys[key] ?? codes[key]
    }

    /// A flags-changed event as the facts the core's direction detection
    /// needs (HOT-16): the changed key, its family flag, and its per-side
    /// device bit (IOKit's `NX_DEVICE*KEYMASK`; Fn has no sides). None for
    /// keys without a code.
    static func flagsChange(virtualKey: Int64, flags: CGEventFlags) -> HotkeyInput? {
        guard let code = code(forVirtualKey: virtualKey) else { return nil }
        guard let bits = flagBits[Int(virtualKey)] else {
            return .modifierFlags(key: code, familyFlagSet: false, sideFlagSet: false)
        }
        let family = flags.contains(bits.family)
        let side = bits.side == 0 ? family : flags.rawValue & bits.side != 0
        return .modifierFlags(key: code, familyFlagSet: family, sideFlagSet: side)
    }

    private static let modifierKeys: [Int: String] = [
        kVK_Control: "ControlLeft", kVK_RightControl: "ControlRight",
        kVK_Option: "AltLeft", kVK_RightOption: "AltRight",
        kVK_Shift: "ShiftLeft", kVK_RightShift: "ShiftRight",
        kVK_Command: "MetaLeft", kVK_RightCommand: "MetaRight",
        kVK_Function: "Fn"
    ]

    private static let flagBits: [Int: (family: CGEventFlags, side: UInt64)] = [
        kVK_Control: (.maskControl, 0x0001), kVK_RightControl: (.maskControl, 0x2000),
        kVK_Shift: (.maskShift, 0x0002), kVK_RightShift: (.maskShift, 0x0004),
        kVK_Command: (.maskCommand, 0x0008), kVK_RightCommand: (.maskCommand, 0x0010),
        kVK_Option: (.maskAlternate, 0x0020), kVK_RightOption: (.maskAlternate, 0x0040),
        kVK_Function: (.maskSecondaryFn, 0), kVK_CapsLock: (.maskAlphaShift, 0)
    ]

    /// `table` reversed; where two codes share a key, the first by name.
    private static let codes: [Int: String] = Dictionary(
        table.map { ($0.value, $0.key) },
        uniquingKeysWith: { min($0, $1) }
    )

    private static let table: [String: Int] = [
        "KeyA": kVK_ANSI_A, "KeyB": kVK_ANSI_B, "KeyC": kVK_ANSI_C, "KeyD": kVK_ANSI_D,
        "KeyE": kVK_ANSI_E, "KeyF": kVK_ANSI_F, "KeyG": kVK_ANSI_G, "KeyH": kVK_ANSI_H,
        "KeyI": kVK_ANSI_I, "KeyJ": kVK_ANSI_J, "KeyK": kVK_ANSI_K, "KeyL": kVK_ANSI_L,
        "KeyM": kVK_ANSI_M, "KeyN": kVK_ANSI_N, "KeyO": kVK_ANSI_O, "KeyP": kVK_ANSI_P,
        "KeyQ": kVK_ANSI_Q, "KeyR": kVK_ANSI_R, "KeyS": kVK_ANSI_S, "KeyT": kVK_ANSI_T,
        "KeyU": kVK_ANSI_U, "KeyV": kVK_ANSI_V, "KeyW": kVK_ANSI_W, "KeyX": kVK_ANSI_X,
        "KeyY": kVK_ANSI_Y, "KeyZ": kVK_ANSI_Z,
        "Digit0": kVK_ANSI_0, "Digit1": kVK_ANSI_1, "Digit2": kVK_ANSI_2, "Digit3": kVK_ANSI_3,
        "Digit4": kVK_ANSI_4, "Digit5": kVK_ANSI_5, "Digit6": kVK_ANSI_6, "Digit7": kVK_ANSI_7,
        "Digit8": kVK_ANSI_8, "Digit9": kVK_ANSI_9,
        "Minus": kVK_ANSI_Minus, "Equal": kVK_ANSI_Equal,
        "BracketLeft": kVK_ANSI_LeftBracket, "BracketRight": kVK_ANSI_RightBracket,
        "Backslash": kVK_ANSI_Backslash, "Semicolon": kVK_ANSI_Semicolon, "Quote": kVK_ANSI_Quote,
        "Comma": kVK_ANSI_Comma, "Period": kVK_ANSI_Period, "Slash": kVK_ANSI_Slash,
        "Backquote": kVK_ANSI_Grave, "IntlBackslash": kVK_ISO_Section,
        "Space": kVK_Space, "Enter": kVK_Return, "Tab": kVK_Tab, "Backspace": kVK_Delete,
        "Delete": kVK_ForwardDelete, "Escape": kVK_Escape, "CapsLock": kVK_CapsLock,
        "Home": kVK_Home, "End": kVK_End, "PageUp": kVK_PageUp, "PageDown": kVK_PageDown,
        "Insert": kVK_Help, "Help": kVK_Help,
        "ArrowLeft": kVK_LeftArrow, "ArrowRight": kVK_RightArrow,
        "ArrowDown": kVK_DownArrow, "ArrowUp": kVK_UpArrow,
        "F1": kVK_F1, "F2": kVK_F2, "F3": kVK_F3, "F4": kVK_F4, "F5": kVK_F5, "F6": kVK_F6,
        "F7": kVK_F7, "F8": kVK_F8, "F9": kVK_F9, "F10": kVK_F10, "F11": kVK_F11, "F12": kVK_F12,
        "F13": kVK_F13, "F14": kVK_F14, "F15": kVK_F15, "F16": kVK_F16, "F17": kVK_F17,
        "F18": kVK_F18, "F19": kVK_F19, "F20": kVK_F20,
        "Numpad0": kVK_ANSI_Keypad0, "Numpad1": kVK_ANSI_Keypad1, "Numpad2": kVK_ANSI_Keypad2,
        "Numpad3": kVK_ANSI_Keypad3, "Numpad4": kVK_ANSI_Keypad4, "Numpad5": kVK_ANSI_Keypad5,
        "Numpad6": kVK_ANSI_Keypad6, "Numpad7": kVK_ANSI_Keypad7, "Numpad8": kVK_ANSI_Keypad8,
        "Numpad9": kVK_ANSI_Keypad9, "NumpadDecimal": kVK_ANSI_KeypadDecimal,
        "NumpadMultiply": kVK_ANSI_KeypadMultiply, "NumpadAdd": kVK_ANSI_KeypadPlus,
        "NumpadSubtract": kVK_ANSI_KeypadMinus, "NumpadDivide": kVK_ANSI_KeypadDivide,
        "NumpadEnter": kVK_ANSI_KeypadEnter, "NumpadEqual": kVK_ANSI_KeypadEquals,
        "NumLock": kVK_ANSI_KeypadClear
    ]
}
