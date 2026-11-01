import Foundation
@testable import Onda
import Testing

struct HelperRunTests {
    private static let shell = URL(fileURLWithPath: "/bin/sh")

    @Test
    func `MEDIA-5: a helper that exits 0 succeeds with its output`() {
        let run = HelperRun(executable: Self.shell, arguments: ["-c", "echo Greendale"])
        #expect(run.succeeded)
        #expect(String(decoding: run.output, as: UTF8.self) == "Greendale\n")
    }

    @Test
    func `MEDIA-5: a failing helper reports its status and stderr`() {
        let run = HelperRun(executable: Self.shell, arguments: ["-c", "echo 'Chang was here' >&2; exit 3"])
        #expect(!run.succeeded)
        #expect(run.failure == "helper exited with status 3: Chang was here")
    }

    @Test
    func `MEDIA-5: output larger than a pipe buffer drains instead of blocking`() {
        let run = HelperRun(
            executable: Self.shell,
            arguments: ["-c", "exec head -c 1000000 /dev/zero"],
            timeout: 3
        )
        #expect(run.succeeded)
        #expect(run.output.count == 1_000_000)
    }

    struct Hang: Sendable, CustomTestStringConvertible {
        let name: String
        let script: String
        var testDescription: String {
            name
        }
    }

    @Test(
        arguments: [
            Hang(name: "terminate stops it", script: "exec sleep 10"),
            Hang(name: "kill stops one that ignores terminate", script: "trap '' TERM; exec sleep 10")
        ]
    )
    func `MEDIA-5: a stuck helper is stopped after the timeout`(hang: Hang) {
        let started = Date()
        let run = HelperRun(executable: Self.shell, arguments: ["-c", hang.script], timeout: 0.2, killGrace: 0.2)
        #expect(!run.succeeded)
        #expect(run.failure == "helper timed out and was stopped")
        #expect(Date().timeIntervalSince(started) < 3)
    }

    @Test
    func `MEDIA-5: a helper that cannot launch fails`() {
        let run = HelperRun(executable: URL(fileURLWithPath: "/nonexistent/greendale"), arguments: [])
        #expect(!run.succeeded)
        #expect(!run.failure.isEmpty)
    }
}

struct NowPlayingLineTests {
    struct Case: Sendable, CustomTestStringConvertible {
        let name: String
        let output: String
        let want: String?
        var testDescription: String {
            name
        }
    }

    @Test(
        arguments: [
            Case(name: "empty", output: "", want: nil),
            Case(name: "no track", output: "NIL\n", want: nil),
            Case(name: "no track with spaces", output: "  NIL  \n", want: nil),
            Case(name: "blank lines first", output: "\n   \n{\"payload\":{}}\nmore\n", want: "{\"payload\":{}}")
        ]
    )
    func `MEDIA-1: the first non-blank line, none for NIL`(tc: Case) {
        #expect(NowPlayingLine.first(in: Data(tc.output.utf8)) == tc.want)
    }

    @Test
    func `MEDIA-1: decodes the raw facts without the title`() throws {
        let json = #"{"payload":{"title":"Daybreak","playbackRate":1.0,"bundleIdentifier":"com.apple.Music"}}"#
        let payload = try JSONDecoder().decode(NowPlayingLine.self, from: Data(json.utf8)).payload
        #expect(payload.isPlaying == nil)
        #expect(payload.playbackRate == 1.0)
        #expect(payload.bundleIdentifier == "com.apple.Music")
    }
}
