import AppKit
@preconcurrency import ApplicationServices
import Carbon
import OndaCore

/// Delivers text into other apps (OUT-2..OUT-7). Called on core threads;
/// every call may block. Chunking comes from the core (`unicodeChunks`);
/// whether to type at all, the joiner, and the notices are core decisions.
/// An insertion commits its delivery right before the first effect another
/// app can see, so a cancel that came first outputs nothing (DICT-19).
final class TextOutput: TextOutputPort, @unchecked Sendable {
    /// OUT-2: a hung app must not stall typing.
    private static let readTimeout: Float = 0.25
    /// OUT-2: bound the accessibility insertion too (Jabber does not).
    private static let insertTimeout: Float = 1.0
    /// OUT-3 strategy 4 timings.
    private static let pasteDelay: TimeInterval = 0.05
    private static let restoreDelay: TimeInterval = 0.5
    private static let vKey: CGKeyCode = 0x09
    private static let keyLookupTimeout: DispatchTimeInterval = .milliseconds(100)
    private static let pasteboardLock = NSLock()
    static let transientType = NSPasteboard.PasteboardType("org.nspasteboard.TransientType")

    func focusedTarget() -> TargetApp? {
        guard let pid = Self.focusedPid() else { return nil }
        let app = NSRunningApplication(processIdentifier: pid)
        return TargetApp(
            processId: UInt32(pid),
            name: app?.localizedName,
            appId: app?.bundleIdentifier
        )
    }

    func fieldText(target: TargetApp?) -> FieldContents? {
        guard let element = Self.focusedElement(target: target, timeout: Self.readTimeout),
              let kind = Self.fieldKind(element),
              let value = Self.stringAttribute(element, kAXValueAttribute),
              let range = Self.selectedRange(element) else { return nil }
        return FieldContents(text: value, caretUtf16: UInt64(max(range.location, 0)), kind: kind)
    }

    func insert(text: String, target: TargetApp?, delivery: Delivery) -> InsertResult {
        let chunks = unicodeChunks(text: text)
        if let pid = Self.livePid(target) {
            switch Self.postUnicode(chunks, delivery: delivery, { $0.postToPid(pid) }) {
            case .posted: return .inserted
            case .cancelled: return .cancelled
            case .unavailable: break
            }
        }
        // a failed accessibility write has already committed: the OS gives
        // no earlier point, so a cancel can no longer stop the fallbacks
        if insertViaAccessibility(text, target: target, delivery: delivery) {
            return .inserted
        }
        // strategies 3 and 4 reach whatever has focus now, which is no
        // longer the target once it quit or the user moved on
        if Self.targetLeft(target) {
            guard delivery.commit() else { return .cancelled }
            return copy(text: text) ? .targetLeft : .copyFailed
        }
        switch Self.postUnicode(chunks, delivery: delivery, { $0.post(tap: .cghidEventTap) }) {
        case .posted: return .inserted
        case .cancelled: return .cancelled
        case .unavailable: break
        }
        guard delivery.commit() else { return .cancelled }
        return pasteWithRestore(text)
    }

    func copy(text: String) -> Bool {
        Self.pasteboardLock.withLock {
            Self.writeTranscript(text, to: .general, transient: false) != nil
        }
    }

    @discardableResult
    static func writeTranscript(_ text: String, to pasteboard: NSPasteboard, transient: Bool) -> Int? {
        let item = NSPasteboardItem()
        guard item.setString(text, forType: .string),
              !transient || item.setData(Data(), forType: transientType)
        else {
            Log.output.error("Could not copy the transcript to the clipboard")
            return nil
        }
        let changeCount = pasteboard.clearContents()
        let copied = pasteboard.writeObjects([item])
        if !copied {
            Log.output.error("Could not copy the transcript to the clipboard")
            return nil
        }
        return changeCount
    }

    // MARK: - Strategies

    /// Strategy 1 needs a live target: events posted to an exited process
    /// vanish without an error and would skip every fallback.
    private static func livePid(_ target: TargetApp?) -> pid_t? {
        guard let target, target.processId > 0 else { return nil }
        let pid = pid_t(target.processId)
        guard let app = NSRunningApplication(processIdentifier: pid), !app.isTerminated else {
            Log.output.info("Target process \(pid, privacy: .public) is gone; using a fallback")
            return nil
        }
        return pid
    }

    /// How posting Unicode key events ended.
    enum Posted: Equatable {
        case posted
        /// An event could not be created; nothing was posted or committed.
        case unavailable
        /// The delivery was cancelled first; nothing was posted.
        case cancelled
    }

    /// Strategies 1 and 3. Builds every event before committing and posting
    /// any, so a failed allocation leaves nothing typed and the delivery
    /// still cancellable before a fallback, and never half the text typed
    /// before a fallback types all of it again.
    static func postUnicode(
        _ chunks: [[UInt16]],
        delivery: Delivery,
        events makeEvents: ([[UInt16]]) -> [(down: CGEvent, up: CGEvent)]? = unicodeEvents,
        _ post: (CGEvent) -> Void
    ) -> Posted {
        guard let events = makeEvents(chunks) else {
            Log.output.error("Could not create a Unicode key event")
            return .unavailable
        }
        guard delivery.commit() else { return .cancelled }
        for event in events {
            post(event.down)
            post(event.up)
        }
        return .posted
    }

    /// A key-down and key-up per chunk carrying its UTF-16 units; nil when
    /// any event cannot be created.
    static func unicodeEvents(_ chunks: [[UInt16]]) -> [(down: CGEvent, up: CGEvent)]? {
        var events: [(down: CGEvent, up: CGEvent)] = []
        events.reserveCapacity(chunks.count)
        for chunk in chunks {
            guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true),
                  let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false) else {
                return nil
            }
            // a nil source stamps the modifiers held right now, and with
            // virtual key 0 a held Command turns the text into Command-A
            down.flags = []
            up.flags = []
            chunk.withUnsafeBufferPointer { buffer in
                guard let base = buffer.baseAddress else { return }
                down.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: base)
                up.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: base)
            }
            events.append((down, up))
        }
        return events
    }

    /// Strategy 2: replace the selection in the focused element's value and
    /// move the caret after the inserted text. False without writing when
    /// the delivery was cancelled during the reads.
    private func insertViaAccessibility(_ text: String, target: TargetApp?, delivery: Delivery) -> Bool {
        guard let element = Self.focusedElement(target: target, timeout: Self.insertTimeout),
              Self.fieldKind(element) != nil,
              let value = Self.stringAttribute(element, kAXValueAttribute),
              let selected = Self.selectedRange(element) else { return false }
        var settable = DarwinBoolean(false)
        guard AXUIElementIsAttributeSettable(element, kAXValueAttribute as CFString, &settable) == .success,
              settable.boolValue else { return false }
        let current = value as NSString
        let location = max(0, min(selected.location, current.length))
        let length = max(0, min(selected.length, current.length - location))
        let updated = NSMutableString(string: value)
        updated.replaceCharacters(in: NSRange(location: location, length: length), with: text)
        guard delivery.commit() else { return false }
        let result = AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, updated as CFString)
        guard result == .success else {
            Log.output.info("Accessibility insertion failed: \(result.rawValue, privacy: .public)")
            return false
        }
        var caret = CFRange(location: location + (text as NSString).length, length: 0)
        if let range = AXValueCreate(.cfRange, &caret) {
            let moved = AXUIElementSetAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, range)
            if moved != .success {
                Log.output.info("Could not move the caret: \(moved.rawValue, privacy: .public)")
            }
        }
        return true
    }

    /// Strategy 4: paste, then restore the previous clipboard only if
    /// nothing else changed it meanwhile.
    private func pasteWithRestore(_ text: String) -> InsertResult {
        Self.pasteWithRestore(
            text,
            on: .general,
            lock: Self.pasteboardLock,
            pasteDelay: Self.pasteDelay,
            restoreDelay: Self.restoreDelay
        ) {
            let source = CGEventSource(stateID: .hidSystemState)
            let pasteKey = Self.virtualKeyForV()
            guard let down = CGEvent(keyboardEventSource: source, virtualKey: pasteKey, keyDown: true),
                  let up = CGEvent(keyboardEventSource: source, virtualKey: pasteKey, keyDown: false) else {
                // OUT-4: the transcript stays on the clipboard
                Log.output.error("Could not create the paste key event")
                return nil
            }
            down.flags = .maskCommand
            up.flags = .maskCommand
            return {
                down.post(tap: .cghidEventTap)
                up.post(tap: .cghidEventTap)
            }
        }
    }

    static func pasteWithRestore(
        _ text: String,
        on pasteboard: NSPasteboard,
        lock: NSLock,
        pasteDelay: TimeInterval,
        restoreDelay: TimeInterval,
        afterWrite: () -> Void = {},
        afterPaste: () -> Void = {},
        preparePaste: () -> (() -> Void)?
    ) -> InsertResult {
        let prepared: (snapshot: PasteboardSnapshot, written: Int)? = lock.withLock {
            let snapshot = PasteboardSnapshot.capture(from: pasteboard)
            guard let written = writeTranscript(text, to: pasteboard, transient: true) else {
                // OUT-6: put back what was there before reporting
                if !snapshot.restore(to: pasteboard) {
                    Log.output.error("Could not restore the clipboard after a failed copy")
                }
                return nil
            }
            return (snapshot, written)
        }
        guard let (snapshot, written) = prepared else {
            return .copyFailed
        }
        afterWrite()
        if pasteDelay > 0 {
            Thread.sleep(forTimeInterval: pasteDelay)
        }
        guard let postPaste = preparePaste() else {
            return .pasteFailed
        }
        let posted = lock.withLock {
            guard pasteboard.changeCount == written else { return false }
            postPaste()
            return true
        }
        guard posted else {
            Log.output.info("Clipboard changed before the paste; skipping paste")
            return .pasteInterrupted
        }
        afterPaste()
        if restoreDelay > 0 {
            Thread.sleep(forTimeInterval: restoreDelay)
        }
        let outcome = lock.withLock {
            snapshot.restore(to: pasteboard, ifChangeCountIs: written)
        }
        switch outcome {
        case .restored:
            return .inserted
        case .changed:
            Log.output.info("Clipboard changed after the paste; not restoring")
            return .inserted
        case .failed:
            Log.output.error("Could not restore the previous clipboard")
            return .clipboardRestoreFailed
        }
    }

    /// Finds the physical virtual keycode mapped to 'v' on the active keyboard
    /// layout, falling back to ANSI V (`0x09`) if unavailable.
    static func virtualKeyForV() -> CGKeyCode {
        if Thread.isMainThread {
            return lookupVirtualKeyForV()
        }
        return resolveVirtualKeyOnQueue(
            timeout: keyLookupTimeout,
            schedule: { DispatchQueue.main.async(execute: $0) },
            lookup: lookupVirtualKeyForV
        )
    }

    static func resolveVirtualKeyOnQueue(
        timeout: DispatchTimeInterval,
        schedule: (@escaping @Sendable () -> Void) -> Void,
        lookup: @escaping @Sendable () -> CGKeyCode
    ) -> CGKeyCode {
        let answer = Answer<CGKeyCode?>(nil)
        let done = DispatchSemaphore(value: 0)
        schedule {
            answer.set(lookup())
            done.signal()
        }
        guard done.wait(timeout: .now() + timeout) == .success else {
            return vKey
        }
        return answer.get() ?? vKey
    }

    private static func lookupVirtualKeyForV() -> CGKeyCode {
        let keyboardType = UInt32(LMGetKbdType())
        if let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue(),
           let code = virtualKeyForV(in: source, keyboardType: keyboardType) {
            return code
        }
        if let source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource()?.takeRetainedValue(),
           let code = virtualKeyForV(in: source, keyboardType: keyboardType) {
            return code
        }
        return vKey
    }

    static func virtualKeyForV(in source: TISInputSource, keyboardType: UInt32 = UInt32(LMGetKbdType())) -> CGKeyCode? {
        guard let raw = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) else {
            return nil
        }
        let data = Unmanaged<CFData>.fromOpaque(raw).takeUnretainedValue()
        return virtualKeyForV(in: data, keyboardType: keyboardType)
    }

    static func virtualKeyForV(in layoutData: CFData, keyboardType: UInt32 = UInt32(LMGetKbdType())) -> CGKeyCode? {
        guard let bytes = CFDataGetBytePtr(layoutData) else { return nil }
        let layout = UnsafeRawPointer(bytes).assumingMemoryBound(to: UCKeyboardLayout.self)
        // Check the Command modifier table first so layouts like Dvorak - QWERTY ⌘
        // resolve Cmd+V to QWERTY V (0x09) rather than unmodified 'v' (0x2F, Cmd+.).
        for modifierKeyState in [UInt32(cmdKey >> 8), 0] {
            if let code = findVirtualKeyForV(
                in: layout,
                modifierKeyState: modifierKeyState,
                keyboardType: keyboardType
            ) {
                return code
            }
        }
        return nil
    }

    private static func findVirtualKeyForV(
        in layout: UnsafePointer<UCKeyboardLayout>,
        modifierKeyState: UInt32,
        keyboardType: UInt32
    ) -> CGKeyCode? {
        var uppercaseFallback: CGKeyCode?
        for code in UInt16(0) ..< UInt16(128) {
            var deadKeyState: UInt32 = 0
            var chars: [UniChar] = [0, 0, 0, 0]
            var length = 0
            let status = UCKeyTranslate(
                layout,
                code,
                UInt16(kUCKeyActionDisplay),
                modifierKeyState,
                keyboardType,
                OptionBits(kUCKeyTranslateNoDeadKeysBit),
                &deadKeyState,
                chars.count,
                &length,
                &chars
            )
            guard status == noErr, length == 1 else { continue }
            if chars[0] == 0x76 {
                return CGKeyCode(code)
            }
            if chars[0] == 0x56, uppercaseFallback == nil {
                uppercaseFallback = CGKeyCode(code)
            }
        }
        return uppercaseFallback
    }

    // MARK: - Accessibility helpers

    /// The process that owns the keyboard focus; nil without Accessibility
    /// or when the read fails.
    private static func focusedPid() -> pid_t? {
        guard AXIsProcessTrusted() else { return nil }
        // every dictation start waits on this read, and without a timeout a
        // hung frontmost app holds it for the system default of seconds
        let systemWide = AXUIElementCreateSystemWide()
        AXUIElementSetMessagingTimeout(systemWide, readTimeout)
        guard let element = elementAttribute(systemWide, kAXFocusedUIElementAttribute as CFString) else {
            return nil
        }
        var pid: pid_t = 0
        guard AXUIElementGetPid(element, &pid) == .success, pid > 0 else { return nil }
        return pid
    }

    /// The known target quit, or the focus is now in another process. An
    /// unknown target or an unreadable focus counts as still there.
    static func targetLeft(_ target: TargetApp?) -> Bool {
        guard let target, target.processId > 0 else { return false }
        let pid = pid_t(target.processId)
        guard let app = NSRunningApplication(processIdentifier: pid), !app.isTerminated else {
            Log.output.info("Target process \(pid, privacy: .public) quit; copying instead")
            return true
        }
        guard let focused = focusedPid(), focused != pid else { return false }
        Log.output.info("Focus moved from \(pid, privacy: .public) to \(focused, privacy: .public); copying instead")
        return true
    }

    private static func focusedElement(target: TargetApp?, timeout: Float) -> AXUIElement? {
        let container = target.flatMap { $0.processId > 0 ? AXUIElementCreateApplication(pid_t($0.processId)) : nil }
            ?? AXUIElementCreateSystemWide()
        AXUIElementSetMessagingTimeout(container, timeout)
        guard let element = elementAttribute(container, kAXFocusedUIElementAttribute as CFString) else {
            return nil
        }
        AXUIElementSetMessagingTimeout(element, timeout)
        return element
    }

    private static func elementAttribute(_ element: AXUIElement, _ attribute: CFString) -> AXUIElement? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success,
              let value, CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        return unsafeDowncast(value, to: AXUIElement.self)
    }

    private static func fieldKind(_ element: AXUIElement) -> FieldKind? {
        guard let role = stringAttribute(element, kAXRoleAttribute) else { return nil }
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(element, kAXSubroleAttribute as CFString, &value)
        let subrole: String?
        switch status {
        case .success:
            guard let name = value as? String else { return nil }
            subrole = name
        case .attributeUnsupported, .noValue:
            subrole = nil
        default:
            return nil
        }
        return fieldKind(role: role, subrole: subrole) {
            var pid: pid_t = 0
            guard AXUIElementGetPid(element, &pid) == .success else { return nil }
            return NSRunningApplication(processIdentifier: pid)?.bundleIdentifier
        }
    }

    static func fieldKind(role: String, subrole: String?, bundleID: () -> String?) -> FieldKind? {
        guard role != kAXSecureTextFieldSubrole, subrole != kAXSecureTextFieldSubrole else { return nil }
        // macOS has no public terminal role. The terminal screen in these apps is a text area.
        if role == kAXTextAreaRole {
            switch bundleID() {
            case "com.apple.Terminal",
                 "com.googlecode.iterm2",
                 "com.mitchellh.ghostty",
                 "net.kovidgoyal.kitty",
                 "org.alacritty",
                 "com.github.wez.wezterm",
                 "dev.warp.Warp-Stable":
                return .terminal
            default:
                break
            }
        }
        return .prose
    }

    private static func stringAttribute(_ element: AXUIElement, _ attribute: String) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else {
            return nil
        }
        return value as? String
    }

    private static func selectedRange(_ element: AXUIElement) -> CFRange? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, &value) == .success,
              let value, CFGetTypeID(value) == AXValueGetTypeID() else { return nil }
        var range = CFRange()
        guard AXValueGetValue(unsafeDowncast(value, to: AXValue.self), .cfRange, &range) else { return nil }
        return range
    }
}

/// Every item and type on the clipboard, for OUT-3 strategy 4.
struct PasteboardSnapshot {
    static let maxSnapshotBytes = 32 * 1024 * 1024

    private struct Representation {
        let type: NSPasteboard.PasteboardType
        let data: Data
    }

    private static let markerTypes: Set<NSPasteboard.PasteboardType> = [
        NSPasteboard.PasteboardType("org.nspasteboard.TransientType"),
        NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"),
        NSPasteboard.PasteboardType("org.nspasteboard.AutoGeneratedType")
    ]

    private let items: [[Representation]]
    let restorable: Bool

    static func capture(
        from pasteboard: NSPasteboard,
        maxBytes: Int = PasteboardSnapshot.maxSnapshotBytes
    ) -> PasteboardSnapshot {
        var captured: [[Representation]] = []
        var totalBytes = 0
        for item in pasteboard.pasteboardItems ?? [] {
            var representations: [Representation] = []
            representations.reserveCapacity(item.types.count)
            for type in item.types {
                if let data = item.data(forType: type) {
                    totalBytes += data.count
                    if totalBytes > maxBytes {
                        return PasteboardSnapshot(items: [], restorable: false)
                    }
                    representations.append(Representation(type: type, data: data))
                } else if markerTypes.contains(type) {
                    representations.append(Representation(type: type, data: Data()))
                }
            }
            if !representations.isEmpty {
                captured.append(representations)
            }
        }
        return PasteboardSnapshot(items: captured, restorable: true)
    }

    /// What a restore after the paste did (OUT-3 strategy 4).
    enum RestoreOutcome: Equatable {
        /// The previous clipboard is back.
        case restored
        /// Something else wrote the clipboard since the paste; left alone.
        case changed
        /// Restoring failed.
        case failed
    }

    /// Restores only if nothing wrote `pasteboard` since the transcript
    /// left it at change count `written`.
    func restore(to pasteboard: NSPasteboard, ifChangeCountIs written: Int) -> RestoreOutcome {
        guard pasteboard.changeCount == written else { return .changed }
        guard let restored = buildItems() else { return .failed }
        guard pasteboard.changeCount == written else { return .changed }
        pasteboard.clearContents()
        return (restored.isEmpty || pasteboard.writeObjects(restored)) ? .restored : .failed
    }

    /// OUT-5: builds the items before clearing, so a failure never leaves
    /// the clipboard empty.
    func restore(to pasteboard: NSPasteboard) -> Bool {
        guard let restored = buildItems() else { return false }
        pasteboard.clearContents()
        return restored.isEmpty || pasteboard.writeObjects(restored)
    }

    private func buildItems() -> [NSPasteboardItem]? {
        guard restorable else { return nil }
        var restored: [NSPasteboardItem] = []
        restored.reserveCapacity(items.count)
        for representations in items {
            let pasteboardItem = NSPasteboardItem()
            for representation in representations {
                guard pasteboardItem.setData(representation.data, forType: representation.type) else { return nil }
            }
            restored.append(pasteboardItem)
        }
        return restored
    }
}
