import Foundation
@testable import Onda
import Testing

struct OutputMuteTests {
    @Test
    func `a recorded device that is not connected stays recorded`() throws {
        let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let record = folder.appending(path: OutputMute.recordName)
        try "GreendaleStudyRoomF\nGreendaleLibrary".write(to: record, atomically: true, encoding: .utf8)
        OutputMute.restore(record: record)
        #expect(try String(contentsOf: record, encoding: .utf8) == "GreendaleStudyRoomF\nGreendaleLibrary")
    }
}
