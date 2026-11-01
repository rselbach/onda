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
        }
    }

    @Test(arguments: texts)
    func `OUT-3: posts every event in order, down then up`(text: String) {
        let chunks = unicodeChunks(text: text)
        var posted: [(type: CGEventType, units: [UInt16])] = []

        let ok = TextOutput.postUnicode(chunks) { posted.append(($0.type, Self.units(of: $0))) }

        #expect(ok)
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
