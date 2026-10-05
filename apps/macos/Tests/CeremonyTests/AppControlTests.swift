import Foundation
import Sparkle
import Testing

@testable import Far_Cooler

/// `farcooler app`'s way into the app (ov-302): where the socket is, that
/// only this user gets an answer, and what each request answers in a build
/// without updates, as `swift test` runs.
@Suite(.serialized)
struct AppControlTests {
    /// A scratch socket path under /tmp, short enough for `sun_path`.
    private static func scratch() -> String {
        let dir = "/tmp/fc-ac-\(UUID().uuidString.prefix(8))"
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        return dir + "/app.sock"
    }

    /// Connect to `path`, send `line`, and read until the app hangs up.
    private static func ask(_ path: String, _ line: String) -> [[String: Any]] {
        lines(heard(path, line))
    }

    private static func lines(_ data: Data) -> [[String: Any]] {
        String(decoding: data, as: UTF8.self).split(separator: "\n").compactMap {
            try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]
        }
    }

    /// Everything the app wrote back before it hung up.
    private static func heard(_ path: String, _ line: String) -> Data {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: Array(path.utf8) + [0]) }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        defer { close(fd) }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { return Data() }
        var wait = timeval(tv_sec: 5, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &wait, socklen_t(MemoryLayout<timeval>.size))
        _ = (line + "\n").withCString { write(fd, $0, strlen($0)) }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(fd, &buffer, buffer.count)
            if n <= 0 { break }
            data.append(contentsOf: buffer[0..<n])
        }
        return data
    }

    private static let echo: AppControl.Handler = { request, reply in
        reply.send(["event": "echo", "op": request["op"] as? String ?? ""])
        reply.close()
    }

    @Test func theSocketSitsBesideTheChannelsDaemon() {
        #expect(
            AppControl.socketPath(environment: [:], channel: "canary", home: "/Users/me")
                == "/Users/me/Library/Application Support/com.farcooler.FarCoolerCanary/app.sock")
        #expect(
            AppControl.socketPath(environment: [:], channel: "stable", home: "/Users/me")
                == "/Users/me/Library/Application Support/com.farcooler.FarCooler/app.sock")
        #expect(
            AppControl.socketPath(environment: [:], channel: "local", home: "/Users/me")
                == "/Users/me/Library/Application Support/com.farcooler.FarCoolerLocal/app.sock")
        #expect(
            AppControl.socketPath(environment: ["FARCOOLER_HOME": "/tmp/fc-t/x"], channel: "canary", home: "/Users/me")
                == "/tmp/fc-t/x/app.sock")
    }

    @Test func thisUserIsAnsweredOnASocketOnlyItCanOpen() throws {
        let path = Self.scratch()
        let control = AppControl()
        try control.start(at: path, handler: Self.echo)
        defer { control.stop(removing: path) }

        let lines = Self.ask(path, #"{"op":"about"}"#)
        #expect(lines.count == 1)
        #expect(lines.first?["op"] as? String == "about")
        let mode = try FileManager.default.attributesOfItem(atPath: path)[.posixPermissions] as? Int
        #expect(mode == 0o600)
    }

    @Test func anotherUserIsHungUpOnUnanswered() throws {
        let path = Self.scratch()
        let control = AppControl()
        control.ownUser = getuid() + 1
        try control.start(at: path, handler: Self.echo)
        defer { control.stop(removing: path) }

        #expect(Self.ask(path, #"{"op":"about"}"#).isEmpty)
    }

    @Test func aSocketLeftByACopyThatQuitIsTakenOver() throws {
        let path = Self.scratch()
        FileManager.default.createFile(atPath: path, contents: Data())
        let control = AppControl()
        try control.start(at: path, handler: Self.echo)
        defer { control.stop(removing: path) }
        #expect(Self.ask(path, #"{"op":"about"}"#).count == 1)
    }

    @Test func aSocketARunningCopyAnswersOnIsLeftToIt() throws {
        let path = Self.scratch()
        let first = AppControl()
        try first.start(at: path, handler: Self.echo)
        defer { first.stop(removing: path) }

        #expect(throws: AppControl.Failure.taken) { try AppControl().start(at: path, handler: Self.echo) }
        #expect(Self.ask(path, #"{"op":"about"}"#).count == 1)
    }

    /// The requests, through `AppControlRequests.handle`, in the test
    /// bundle: it has no feed, so it's a build that doesn't update.
    @MainActor
    private static func handled(_ op: String) async -> [[String: Any]] {
        let path = scratch()
        let control = AppControl()
        try? control.start(at: path) { request, reply in
            let op = request["op"] as? String ?? ""
            Task { @MainActor in AppControlRequests.handle(op: op, relaunch: true, reply: reply) }
        }
        defer { control.stop(removing: path) }
        return lines(await Task.detached { heard(path, #"{"op":"\#(op)"}"#) }.value)
    }

    @MainActor @Test func aboutAnswersWithThisAppsFacts() async {
        let lines = await Self.handled("about")
        let app = lines.first?["app"] as? [String: Any]
        #expect(lines.first?["event"] as? String == "about")
        #expect(app?["pid"] as? Int == Int(getpid()))
        #expect(app?["channel"] as? String == "local")
    }

    @MainActor @Test func aBuildWithoutAFeedRefusesToUpdateAndSaysWhyLatestIsUnknown() async {
        let update = await Self.handled("update")
        #expect(update.count == 1)
        #expect(update.first?["event"] as? String == "refused")
        #expect(update.first?["code"] as? String == "updates-off")

        let version = await Self.handled("version")
        #expect(version.first?["event"] as? String == "version")
        #expect(version.first?["unknown"] as? String == "updates-off")
        #expect(version.first?["app"] != nil)
    }
}
