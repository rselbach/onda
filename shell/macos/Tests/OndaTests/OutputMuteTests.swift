import Foundation
@testable import Onda
import Testing

@Suite(.serialized)
struct OutputMuteTests {
    @Test
    func `a recorded device that is not connected stays recorded`() throws {
        defer { OutputMute.stopWatcherForTesting() }
        let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let record = folder.appending(path: OutputMute.recordName)
        try "GreendaleStudyRoomF\nGreendaleLibrary".write(to: record, atomically: true, encoding: .utf8)
        OutputMute.restore(record: record)
        #expect(try String(contentsOf: record, encoding: .utf8) == "GreendaleStudyRoomF\nGreendaleLibrary")
    }

    @Test
    func `unknown mute token fails without removing recorded devices`() throws {
        defer { OutputMute.stopWatcherForTesting() }
        let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let record = folder.appending(path: OutputMute.recordName)
        try "GreendaleStudyRoomF".write(to: record, atomically: true, encoding: .utf8)
        #expect(!OutputMute.unmute(999, record: record))
        #expect(try String(contentsOf: record, encoding: .utf8) == "GreendaleStudyRoomF")
    }

    @Test
    func `restore preserves active session mute uid without arming watcher`() throws {
        defer { OutputMute.stopWatcherForTesting() }
        let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let record = folder.appending(path: OutputMute.recordName)
        let token = OutputMute.addActiveTokenForTesting(uid: "GreendaleActiveAudio")
        defer { OutputMute.takeTokenForTesting(token) }
        try "GreendaleActiveAudio".write(to: record, atomically: true, encoding: .utf8)
        OutputMute.restore(record: record)
        #expect(try String(contentsOf: record, encoding: .utf8) == "GreendaleActiveAudio")
    }
}
