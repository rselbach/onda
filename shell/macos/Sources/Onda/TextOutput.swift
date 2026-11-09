import AppKit
@preconcurrency import ApplicationServices
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

    func focusedTarget() -> TargetApp? {
        guard AXIsProcessTrusted(), let element = Self.elementAttribute(
            AXUIElementCreateSystemWide(),
            kAXFocusedUIElementAttribute as CFString
        ) else { return nil }
        var pid: pid_t = 0
        guard AXUIElementGetPid(element, &pid) == .success, pid > 0 else { return nil }
        let app = NSRunningApplication(processIdentifier: pid)
        return TargetApp(
            processId: UInt32(pid),
            name: app?.localizedName,
            appId: app?.bundleIdentifier
        )
    }

    func fieldText(target: TargetApp?) -> FieldContents? {
        guard let element = Self.focusedElement(target: target, timeout: Self.readTimeout),
              let value = Self.stringValue(element),
              let range = Self.selectedRange(element) else { return nil }
        return FieldContents(text: value, caretUtf16: UInt64(max(range.location, 0)))
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
        switch Self.postUnicode(chunks, delivery: delivery, { $0.post(tap: .cghidEventTap) }) {
        case .posted: return .inserted
        case .cancelled: return .cancelled
        case .unavailable: break
        }
        guard delivery.commit() else { return .cancelled }
        return pasteWithRestore(text)
    }

    func copy(text: String) -> Bool {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        let copied = pasteboard.setString(text, forType: .string)
        if !copied {
            Log.output.error("Could not copy the transcript to the clipboard")
        }
        return copied
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
              let value = Self.stringValue(element),
              let selected = Self.selectedRange(element) else { return false }
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
        let pasteboard = NSPasteboard.general
        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        guard copy(text: text) else {
            // OUT-6: put back what was there before reporting
            if !snapshot.restore(to: pasteboard) {
                Log.output.error("Could not restore the clipboard after a failed copy")
            }
            return .copyFailed
        }
        let written = pasteboard.changeCount
        Thread.sleep(forTimeInterval: Self.pasteDelay)
        let source = CGEventSource(stateID: .hidSystemState)
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: Self.vKey, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: Self.vKey, keyDown: false) else {
            // OUT-4: the transcript stays on the clipboard
            Log.output.error("Could not create the paste key event")
            return .pasteFailed
        }
        down.flags = .maskCommand
        up.flags = .maskCommand
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: Self.restoreDelay)
        switch snapshot.restore(to: pasteboard, ifChangeCountIs: written) {
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

    // MARK: - Accessibility helpers

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

    private static func stringValue(_ element: AXUIElement) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value) == .success else {
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
    private let items: [[NSPasteboard.PasteboardType: Data]]

    static func capture(from pasteboard: NSPasteboard) -> PasteboardSnapshot {
        let items = (pasteboard.pasteboardItems ?? []).map { item in
            var types: [NSPasteboard.PasteboardType: Data] = [:]
            for type in item.types {
                if let data = item.data(forType: type) {
                    types[type] = data
                }
            }
            return types
        }
        return PasteboardSnapshot(items: items)
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
        return restore(to: pasteboard) ? .restored : .failed
    }

    /// OUT-5: builds the items before clearing, so a failure never leaves
    /// the clipboard empty.
    func restore(to pasteboard: NSPasteboard) -> Bool {
        var restored: [NSPasteboardItem] = []
        restored.reserveCapacity(items.count)
        for item in items {
            let pasteboardItem = NSPasteboardItem()
            for (type, data) in item {
                guard pasteboardItem.setData(data, forType: type) else { return false }
            }
            restored.append(pasteboardItem)
        }
        pasteboard.clearContents()
        return restored.isEmpty || pasteboard.writeObjects(restored)
    }
}
