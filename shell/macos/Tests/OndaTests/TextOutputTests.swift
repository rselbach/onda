import AppKit
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

    private static func privatePasteboard() -> NSPasteboard {
        NSPasteboard(name: NSPasteboard.Name("edu.greendale.onda-tests.\(UUID().uuidString)"))
    }
}
