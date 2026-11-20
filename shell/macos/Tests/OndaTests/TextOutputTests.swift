import AppKit
import ApplicationServices
import Carbon
import CoreGraphics
@testable import Onda
import OndaCore
import Testing

struct UnicodeEventTests {
    static let texts = [
        "Troy Barnes studies at Greendale Community College.",
        String(repeating: "👍", count: 40),
        "Señor Chang 🐒 teaches Spanish at Greendale " + String(repeating: "é👩‍👩‍👧", count: 12),
        ""
    ]

    @Test(arguments: texts)
    func `OUT-3: chunks never split a surrogate pair and keep every unit`(text: String) {
        let chunks = unicodeChunks(text: text)
        for chunk in chunks {
            let first = chunk.first.map(UTF16.isTrailSurrogate) ?? false
            let last = chunk.last.map(UTF16.isLeadSurrogate) ?? false
            #expect(!first)
            #expect(!last)
        }
        #expect(chunks.flatMap(\.self) == Array(text.utf16))
    }

    @Test(arguments: texts)
    func `OUT-3: one key-down and key-up per chunk, each carrying the chunk`(text: String) throws {
        let chunks = unicodeChunks(text: text)
        let events = try #require(TextOutput.unicodeEvents(chunks))
        #expect(events.count == chunks.count)
        for (event, chunk) in zip(events, chunks) {
            #expect(event.down.type == .keyDown)
            #expect(event.up.type == .keyUp)
            #expect(Self.units(of: event.down) == chunk)
            #expect(Self.units(of: event.up) == chunk)
            #expect(event.down.flags.isEmpty)
            #expect(event.up.flags.isEmpty)
        }
    }

    @Test(arguments: texts)
    func `OUT-3: posts every event in order, down then up`(text: String) {
        let chunks = unicodeChunks(text: text)
        var posted: [(type: CGEventType, units: [UInt16])] = []

        let result = TextOutput.postUnicode(chunks, delivery: Delivery()) {
            posted.append(($0.type, Self.units(of: $0)))
        }

        #expect(result == .posted)
        #expect(posted.map(\.type) == Array(repeating: [CGEventType.keyDown, .keyUp], count: chunks.count).flatMap(\.self))
        #expect(posted.filter { $0.type == .keyDown }.flatMap(\.units) == Array(text.utf16))
    }

    private static func units(of event: CGEvent) -> [UInt16] {
        var buffer = [UInt16](repeating: 0, count: 64)
        var length = 0
        event.keyboardGetUnicodeString(maxStringLength: buffer.count, actualStringLength: &length, unicodeString: &buffer)
        return Array(buffer.prefix(length))
    }
}

struct FieldKindTests {
    @Test
    func `secure fields provide no context even in a terminal`() {
        for (role, subrole) in [(kAXTextFieldRole, kAXSecureTextFieldSubrole), (kAXSecureTextFieldSubrole, nil)] {
            #expect(TextOutput.fieldKind(role: role, subrole: subrole) { "com.apple.Terminal" } == nil)
        }
    }

    @Test(
        arguments: [
            "com.apple.Terminal",
            "com.googlecode.iterm2",
            "com.mitchellh.ghostty",
            "net.kovidgoyal.kitty",
            "org.alacritty",
            "com.github.wez.wezterm",
            "dev.warp.Warp-Stable"
        ]
    )
    func `a terminal screen keeps its case, but its text fields use prose`(bundleID: String) {
        #expect(TextOutput.fieldKind(role: kAXTextAreaRole, subrole: nil) { bundleID } == .terminal)
        #expect(TextOutput.fieldKind(role: kAXTextFieldRole, subrole: nil) { bundleID } == .prose)
    }

    @Test
    func `an ordinary editor uses prose`() {
        #expect(TextOutput.fieldKind(role: kAXTextAreaRole, subrole: nil) { "com.apple.TextEdit" } == .prose)
    }
}

struct DeliveryCancellationTests {
    @Test
    func `DICT-19: a delivery cancelled before its commit types and copies nothing`() {
        // the test process as the target, so a regression types only here
        let target = TargetApp(processId: UInt32(getpid()), name: "Greendale", appId: nil)
        let before = NSPasteboard.general.changeCount
        let delivery = Delivery()
        delivery.cancel()

        let result = TextOutput().insert(text: "Troy Barnes", target: target, delivery: delivery)

        #expect(result == .cancelled)
        #expect(!delivery.commit())
        #expect(NSPasteboard.general.changeCount == before)
    }
}

struct UnicodePostCommitTests {
    struct Case: Sendable, CustomTestStringConvertible {
        let name: String
        let eventsFail: Bool
        let cancelled: Bool
        let want: TextOutput.Posted
        var testDescription: String {
            name
        }
    }

    @Test(
        arguments: [
            Case(name: "posts after the commit", eventsFail: false, cancelled: false, want: .posted),
            Case(name: "a cancel first posts nothing", eventsFail: false, cancelled: true, want: .cancelled),
            Case(
                name: "failed events leave the delivery cancellable",
                eventsFail: true, cancelled: false, want: .unavailable
            )
        ]
    )
    func `DICT-19: commits only once every event exists, right before posting`(tc: Case) {
        let chunks = unicodeChunks(text: "Troy Barnes")
        let delivery = Delivery()
        if tc.cancelled {
            delivery.cancel()
        }
        var posted = 0

        let result = TextOutput.postUnicode(
            chunks,
            delivery: delivery,
            events: { tc.eventsFail ? nil : TextOutput.unicodeEvents($0) },
            { _ in posted += 1 }
        )

        #expect(result == tc.want)
        #expect(posted == (tc.want == .posted ? chunks.count * 2 : 0))
        // a later cancel still works unless output started
        delivery.cancel()
        #expect(delivery.commit() == (tc.want == .posted))
    }
}

struct PasteboardSnapshotTests {
    private static let custom = NSPasteboard.PasteboardType("edu.greendale.study-group")

    struct Case: Sendable, CustomTestStringConvertible {
        let name: String
        let laterWrite: String?
        let want: PasteboardSnapshot.RestoreOutcome
        let wantString: String
        let wantCustom: Bool
        var testDescription: String {
            name
        }
    }

    @Test(
        arguments: [
            Case(
                name: "unchanged: every item and type comes back",
                laterWrite: nil, want: .restored, wantString: "Troy Barnes", wantCustom: true
            ),
            Case(
                name: "changed: the later write stays",
                laterWrite: "Abed Nadir", want: .changed, wantString: "Abed Nadir", wantCustom: false
            )
        ]
    )
    func `OUT-3/OUT-5: restores only if nothing wrote the clipboard after the paste`(tc: Case) {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        let item = NSPasteboardItem()
        item.setString("Troy Barnes", forType: .string)
        item.setData(Data([0x47, 0x43, 0x43]), forType: Self.custom)
        pasteboard.clearContents()
        #expect(pasteboard.writeObjects([item]))

        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        pasteboard.clearContents()
        #expect(pasteboard.setString("transcript", forType: .string))
        let written = pasteboard.changeCount
        if let laterWrite = tc.laterWrite {
            pasteboard.clearContents()
            #expect(pasteboard.setString(laterWrite, forType: .string))
        }

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: written) == tc.want)
        #expect(pasteboard.string(forType: .string) == tc.wantString)
        #expect((pasteboard.data(forType: Self.custom) == Data([0x47, 0x43, 0x43])) == tc.wantCustom)
    }

    @Test
    func `OUT-5: an empty clipboard restores to empty`() {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        pasteboard.clearContents()
        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        #expect(pasteboard.setString("transcript", forType: .string))

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: pasteboard.changeCount) == .restored)
        #expect(pasteboard.pasteboardItems?.isEmpty ?? true)
    }

    @Test
    func `preserves concealed, transient, and auto-generated marker UTIs`() {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        let markers = [
            NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"),
            NSPasteboard.PasteboardType("org.nspasteboard.TransientType"),
            NSPasteboard.PasteboardType("org.nspasteboard.AutoGeneratedType")
        ]
        let item = NSPasteboardItem()
        item.setString("secret-token", forType: .string)
        for marker in markers {
            item.setData(Data(), forType: marker)
        }
        pasteboard.clearContents()
        #expect(pasteboard.writeObjects([item]))

        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        #expect(snapshot.restorable)
        pasteboard.clearContents()
        #expect(pasteboard.setString("transcript", forType: .string))

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: pasteboard.changeCount) == .restored)
        let restored = pasteboard.pasteboardItems?.first
        let restoredTypes = Set(restored?.types ?? [])
        for marker in markers {
            #expect(restoredTypes.contains(marker))
        }
        #expect(pasteboard.string(forType: .string) == "secret-token")
    }

    @Test
    func `oversized clipboard snapshot refuses restore without overwriting clipboard`() {
        #expect(PasteboardSnapshot.maxSnapshotBytes == 32 * 1024 * 1024)
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        let item = NSPasteboardItem()
        item.setData(Data(repeating: 0x41, count: 1024), forType: Self.custom)
        pasteboard.clearContents()
        #expect(pasteboard.writeObjects([item]))

        let snapshot = PasteboardSnapshot.capture(from: pasteboard, maxBytes: 512)
        #expect(!snapshot.restorable)
        pasteboard.clearContents()
        #expect(pasteboard.setString("Greendale transcript", forType: .string))

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: pasteboard.changeCount) == .failed)
        #expect(pasteboard.string(forType: .string) == "Greendale transcript")
    }

    @Test
    func `paste writes transient marker while explicit copy does not`() {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }

        let transientCount = TextOutput.writeTranscript("Annie Edison", to: pasteboard, transient: true)
        #expect(transientCount == pasteboard.changeCount)
        let transientItem = pasteboard.pasteboardItems?.first
        #expect(transientItem?.types.contains(TextOutput.transientType) == true)
        #expect(pasteboard.string(forType: .string) == "Annie Edison")

        let explicitCount = TextOutput.writeTranscript("Britta Perry", to: pasteboard, transient: false)
        #expect(explicitCount == pasteboard.changeCount)
        let explicitItem = pasteboard.pasteboardItems?.first
        #expect(explicitItem?.types.contains(TextOutput.transientType) == false)
        #expect(pasteboard.string(forType: .string) == "Britta Perry")
    }

    @Test
    func `preserves UTI preference order on restore`() throws {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        let orderedTypes = [
            NSPasteboard.PasteboardType("edu.greendale.first"),
            NSPasteboard.PasteboardType("edu.greendale.second"),
            NSPasteboard.PasteboardType("edu.greendale.third"),
            NSPasteboard.PasteboardType("edu.greendale.fourth"),
            .string
        ]
        let item = NSPasteboardItem()
        for (index, type) in orderedTypes.enumerated() {
            #expect(item.setData(Data("item-\(index)".utf8), forType: type))
        }
        pasteboard.clearContents()
        #expect(pasteboard.writeObjects([item]))
        let originalOrder = try #require(pasteboard.pasteboardItems?.first?.types)
        #expect(originalOrder == orderedTypes)

        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        pasteboard.clearContents()
        #expect(pasteboard.setString("transcript", forType: .string))

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: pasteboard.changeCount) == .restored)
        let restoredTypes = try #require(pasteboard.pasteboardItems?.first?.types)
        #expect(restoredTypes == originalOrder)
    }

    @Test
    func `omits unmaterialized promised types instead of restoring zero-byte payloads`() throws {
        final class PartialProvider: NSObject, NSPasteboardItemDataProvider {
            func pasteboard(
                _ pasteboard: NSPasteboard?,
                item: NSPasteboardItem,
                provideDataForType type: NSPasteboard.PasteboardType
            ) {
                if type == .string {
                    item.setString("Jeff Winger", forType: .string)
                }
                // Deliberately leave .png unfulfilled so AppKit returns nil for it.
            }
        }

        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        let provider = PartialProvider()
        let item = NSPasteboardItem()
        #expect(item.setDataProvider(provider, forTypes: [.png, .string]))
        pasteboard.clearContents()
        #expect(pasteboard.writeObjects([item]))
        #expect(pasteboard.pasteboardItems?.first?.types == [.png, .string])

        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        pasteboard.clearContents()
        #expect(pasteboard.setString("transcript", forType: .string))

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: pasteboard.changeCount) == .restored)
        let restored = try #require(pasteboard.pasteboardItems?.first)
        #expect(restored.types == [.string])
        #expect(pasteboard.data(forType: .png) == nil)
        #expect(pasteboard.string(forType: .string) == "Jeff Winger")
    }

    @Test
    func `keeps non-nil empty data for regular types`() throws {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        let item = NSPasteboardItem()
        #expect(item.setData(Data(), forType: Self.custom))
        #expect(item.setString("Shirley Bennett", forType: .string))
        pasteboard.clearContents()
        #expect(pasteboard.writeObjects([item]))

        let snapshot = PasteboardSnapshot.capture(from: pasteboard)
        pasteboard.clearContents()
        #expect(pasteboard.setString("transcript", forType: .string))

        #expect(snapshot.restore(to: pasteboard, ifChangeCountIs: pasteboard.changeCount) == .restored)
        let restored = try #require(pasteboard.pasteboardItems?.first)
        #expect(restored.types == [Self.custom, .string])
        #expect(pasteboard.data(forType: Self.custom) == Data())
        #expect(pasteboard.string(forType: .string) == "Shirley Bennett")
    }

    @Test
    func `pasteWithRestore skips paste and preserves clipboard when clipboard changes before paste`() {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        pasteboard.clearContents()
        #expect(pasteboard.setString("Original Greendale note", forType: .string))

        var pasted = false
        let result = TextOutput.pasteWithRestore(
            "Dictated transcript",
            on: pasteboard,
            lock: NSLock(),
            pasteDelay: 0,
            restoreDelay: 0,
            afterWrite: {
                pasteboard.clearContents()
                _ = pasteboard.setString("User copied something else", forType: .string)
            },
            preparePaste: { { pasted = true } }
        )

        #expect(result == .pasteInterrupted)
        #expect(!pasted)
        #expect(pasteboard.string(forType: .string) == "User copied something else")
    }

    @Test
    func `pasteWithRestore posts paste and restores clipboard when unchanged`() {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        pasteboard.clearContents()
        #expect(pasteboard.setString("Original Greendale note", forType: .string))

        var pastedText: String?
        let result = TextOutput.pasteWithRestore(
            "Dictated transcript",
            on: pasteboard,
            lock: NSLock(),
            pasteDelay: 0,
            restoreDelay: 0,
            preparePaste: {
                { pastedText = pasteboard.string(forType: .string) }
            }
        )

        #expect(result == .inserted)
        #expect(pastedText == "Dictated transcript")
        #expect(pasteboard.string(forType: .string) == "Original Greendale note")
    }

    @Test
    func `pasteWithRestore leaves newer clipboard alone when changed after paste`() {
        let pasteboard = Self.privatePasteboard()
        defer { pasteboard.releaseGlobally() }
        pasteboard.clearContents()
        #expect(pasteboard.setString("Original Greendale note", forType: .string))

        var pasted = false
        let result = TextOutput.pasteWithRestore(
            "Dictated transcript",
            on: pasteboard,
            lock: NSLock(),
            pasteDelay: 0,
            restoreDelay: 0,
            afterPaste: {
                pasteboard.clearContents()
                _ = pasteboard.setString("Copied right after paste", forType: .string)
            },
            preparePaste: { { pasted = true } }
        )

        #expect(result == .inserted)
        #expect(pasted)
        #expect(pasteboard.string(forType: .string) == "Copied right after paste")
    }

    @Test
    func `virtualKeyForV resolves a valid macOS keycode`() {
        #expect(TextOutput.virtualKeyForV() < 128)
    }

    @Test
    func `resolveVirtualKeyOnQueue returns ANSI V on timeout without racing late writer`() {
        let gate = DispatchSemaphore(value: 0)
        let finished = DispatchSemaphore(value: 0)

        let timedOut = TextOutput.resolveVirtualKeyOnQueue(
            timeout: .milliseconds(5),
            schedule: { work in
                DispatchQueue.global(qos: .userInitiated).async {
                    gate.wait()
                    work()
                    finished.signal()
                }
            },
            lookup: { 0x2F }
        )

        #expect(timedOut == CGKeyCode(kVK_ANSI_V))
        gate.signal()
        #expect(finished.wait(timeout: .now() + .seconds(2)) == .success)

        let immediate = TextOutput.resolveVirtualKeyOnQueue(
            timeout: .seconds(1),
            schedule: { work in
                DispatchQueue.global(qos: .userInitiated).async(execute: work)
            },
            lookup: { 0x2F }
        )
        #expect(immediate == 0x2F)
    }

    @Test
    @MainActor
    func `virtualKeyForV uses Command modifier table for Dvorak-QWERTY Cmd layout`() throws {
        let layouts: [(id: String, want: CGKeyCode)] = [
            ("com.apple.keylayout.DVORAK-QWERTYCMD", CGKeyCode(kVK_ANSI_V)),
            ("com.apple.keylayout.Dvorak", CGKeyCode(kVK_ANSI_Period)),
            ("com.apple.keylayout.US", CGKeyCode(kVK_ANSI_V))
        ]
        for layout in layouts {
            let filter = [kTISPropertyInputSourceID as String: layout.id] as CFDictionary
            guard let list = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource],
                  let source = list.first
            else {
                Issue.record("Expected system keyboard layout \(layout.id) to be available")
                continue
            }
            let code = try #require(TextOutput.virtualKeyForV(in: source))
            #expect(code == layout.want, "Layout \(layout.id) resolved \(code) instead of \(layout.want)")
        }
    }

    private static func privatePasteboard() -> NSPasteboard {
        NSPasteboard(name: NSPasteboard.Name("edu.greendale.onda-tests.\(UUID().uuidString)"))
    }
}

struct TargetLeftTests {
    @Test
    func `OUT-3: a target that quit has left, an unknown one has not`() throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/true")
        try process.run()
        process.waitUntilExit()
        let gone = TargetApp(processId: UInt32(process.processIdentifier), name: "Troy", appId: nil)
        #expect(TextOutput.targetLeft(gone))
        #expect(!TextOutput.targetLeft(nil))
        #expect(!TextOutput.targetLeft(TargetApp(processId: 0, name: nil, appId: nil)))
    }
}
