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

    /// Stop `control`, and remove the scratch directory `path` is in.
    private static func done(_ control: AppControl, _ path: String) {
        control.stop(removing: path)
        try? FileManager.default.removeItem(atPath: (path as NSString).deletingLastPathComponent)
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
        // The app hangs up on another user without reading, so a write can
        // meet a closed socket, and left at its default that raises SIGPIPE,
        // which kills the whole test process. Set before connecting: on a
        // connection the app already closed, setsockopt fails with EINVAL.
        AppControl.ignoreSIGPIPE(on: fd)
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { return Data() }
        // Long: the answer is made on the main actor, which the rest of the
        // suite can hold for many seconds (it took 18 in one full run).
        var wait = timeval(tv_sec: 120, tv_usec: 0)
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

    /// Write until it fails, and say how. A peer that has hung up refuses the
    /// write at once, unless a program being started still holds a copy of
    /// its end: `Process` forks, and the child's copy lives until its `exec`
    /// closes it (ov-608), a few milliseconds in which the first write goes
    /// through. So the claim is that EPIPE comes, not that it comes first.
    static func writeUntilItFails(_ fd: Int32, patient: Bool = true) -> Int32? {
        let data = Data("hello\n".utf8)
        let deadline = Date().addingTimeInterval(30)
        while true {
            if let code = AppControl.writeAll(fd, data) { return code }
            if !patient || Date() > deadline { return nil }
            usleep(1000)
        }
    }

    /// The closed-peer half of the EPIPE test, once.
    static func hungUpPeerFails(patient: Bool = true) -> Bool {
        var pair: [Int32] = [0, 0]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { return false }
        defer { close(pair[0]) }
        AppControl.ignoreSIGPIPE(on: pair[0])
        close(pair[1])
        return writeUntilItFails(pair[0], patient: patient) == EPIPE
    }

    /// Threads that start and wait for programs, as the app does all day.
    final class Spawners: @unchecked Sendable {
        private let lock = NSLock()
        private var stopped = false
        private var running = 0

        init(_ count: Int) {
            for _ in 0..<count {
                running += 1
                Thread.detachNewThread { [self] in
                    while !isStopped {
                        let process = Process()
                        process.executableURL = URL(fileURLWithPath: "/usr/bin/true")
                        process.standardOutput = FileHandle.nullDevice
                        process.standardError = Pipe()
                        guard (try? process.run()) != nil else { break }
                        process.waitUntilExit()
                    }
                    lock.withLock { running -= 1 }
                }
            }
        }

        private var isStopped: Bool { lock.withLock { stopped } }

        /// Stop them, and wait until they have.
        func stop() {
            lock.withLock { stopped = true }
            while lock.withLock({ running }) > 0 { usleep(1000) }
        }
    }

    /// A write to a peer that has hung up is an error to check, not a death.
    @Test func aWriteToAClosedPeerFailsWithEPIPE() {
        var pair: [Int32] = [0, 0]
        #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0)
        defer { close(pair[0]) }
        #expect(AppControl.ignoreSIGPIPE(on: pair[0]))
        close(pair[1])
        #expect(Self.writeUntilItFails(pair[0]) == EPIPE)
        // Mid-write: a peer that hangs up while a long answer is still going.
        var big: [Int32] = [0, 0]
        #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &big) == 0)
        defer { close(big[0]) }
        #expect(AppControl.ignoreSIGPIPE(on: big[0]))
        let peer = big[1]
        let closer = Thread {
            var byte: UInt8 = 0
            _ = read(peer, &byte, 1)
            close(peer)
        }
        closer.start()
        #expect(AppControl.writeAll(big[0], Data(repeating: 0x61, count: 8 << 20)) == EPIPE)
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
        defer { Self.done(control, path) }

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
        defer { Self.done(control, path) }

        #expect(Self.ask(path, #"{"op":"about"}"#).isEmpty)
    }

    @Test func aSocketLeftByACopyThatQuitIsTakenOver() throws {
        let path = Self.scratch()
        FileManager.default.createFile(atPath: path, contents: Data())
        let control = AppControl()
        try control.start(at: path, handler: Self.echo)
        defer { Self.done(control, path) }
        #expect(Self.ask(path, #"{"op":"about"}"#).count == 1)
    }

    @Test func aSocketARunningCopyAnswersOnIsLeftToIt() throws {
        let path = Self.scratch()
        let first = AppControl()
        try first.start(at: path, handler: Self.echo)
        defer { Self.done(first, path) }

        #expect(throws: AppControl.Failure.taken) { try AppControl().start(at: path, handler: Self.echo) }
        #expect(Self.ask(path, #"{"op":"about"}"#).count == 1)
    }

    /// Review 1004q F9: the running copy holds the socket by its lock, not
    /// only by answering. A second copy that found the file gone, or dead
    /// for a moment, used to bind over the first.
    @Test func aRunningCopysLockKeepsASecondCopyOut() throws {
        let path = Self.scratch()
        let first = AppControl()
        try first.start(at: path, handler: Self.echo)
        defer { Self.done(first, path) }
        unlink(path)

        #expect(throws: AppControl.Failure.taken) { try AppControl().start(at: path, handler: Self.echo) }
    }

    /// `{"op":"about","a":{"a":…1…}}`, nested `depth` deep.
    private static func nested(_ depth: Int) -> String {
        #"{"op":"about","a":"# + String(repeating: #"{"a":"#, count: depth - 1) + "1"
            + String(repeating: "}", count: depth)
    }

    /// The review's crash (ov-302 F1): dictionaries 480 deep, 2.9 KB,
    /// overflowed the parsing thread's stack and took the app down with
    /// SIGBUS. Refused while it's arriving now, before anything parses it.
    @Test func anOversizeRequestIsHungUpOnAndTheAppKeepsAnswering() throws {
        let path = Self.scratch()
        let control = AppControl()
        try control.start(at: path, handler: Self.echo)
        defer { Self.done(control, path) }

        let line = Self.nested(480)
        #expect(line.utf8.count > 2_800)
        #expect(Self.ask(path, line).isEmpty)
        #expect(Self.ask(path, #"{"op":"about"}"#).count == 1)
    }

    /// Nesting short enough to fit the size cap is refused too: a request
    /// is one flat object.
    @Test func aNestedRequestIsHungUpOnUnparsed() throws {
        let path = Self.scratch()
        let control = AppControl()
        try control.start(at: path, handler: Self.echo)
        defer { Self.done(control, path) }

        let line = Self.nested(40)
        #expect(line.utf8.count < 512)
        #expect(Self.ask(path, line).isEmpty)
        #expect(Self.ask(path, #"{"op":"about","relaunch":false}"#).count == 1)
    }

    /// ov-608: Train 172's gate saw the test above fail once, and the cause
    /// was not a descriptor a child kept for good (`Process` hands a child
    /// nothing it isn't told to), but the fork-to-exec gap, which no
    /// close-on-exec flag shortens. Many rounds beside programs being
    /// started make that gap likely instead of rare.
    ///
    /// Off in the per-push suite: it is slow, and its process churn can set
    /// off that same race in tests running beside it. By hand:
    /// `FARCOOLER_SPAWN_STRESS=1 swift test --filter AppControlTests`.
    @Test(.enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_SPAWN_STRESS"] != nil))
    func aHungUpPeerFailsEvenWhileProgramsAreBeingStarted() {
        let spawners = Spawners(4)
        defer { spawners.stop() }
        var failures = 0
        usleep(50_000)
        for _ in 0..<2000 where !Self.hungUpPeerFails() { failures += 1 }
        #expect(failures == 0)
    }

    /// Which of `fds` a program started with plain `posix_spawn` can use.
    /// Unlike `Process`, that starts a child with every descriptor that isn't
    /// close-on-exec: the test of what the flag does.
    private static func inherited(_ fds: [Int32]) -> [Int32] {
        var pipes: [Int32] = [0, 0]
        guard pipe(&pipes) == 0 else { return fds }
        for end in pipes { AppControl.closeOnExec(end) }
        var actions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&actions)
        defer { posix_spawn_file_actions_destroy(&actions) }
        posix_spawn_file_actions_adddup2(&actions, pipes[1], 1)
        // Perl, because a shell keeps descriptors of its own above 9.
        let script = "use POSIX; for (@ARGV) { print \"$_ \" if defined POSIX::dup($_) }"
        let arguments = ["perl", "-e", script] + fds.map { "\($0)" }
        var cStrings: [UnsafeMutablePointer<CChar>?] = arguments.map { strdup($0) } + [nil]
        defer { cStrings.forEach { free($0) } }
        var child: pid_t = 0
        let started = posix_spawn(&child, "/usr/bin/perl", &actions, nil, cStrings, environ)
        close(pipes[1])
        defer { close(pipes[0]) }
        guard started == 0 else { return fds }
        var output = Data()
        var buffer = [UInt8](repeating: 0, count: 256)
        while true {
            let n = read(pipes[0], &buffer, buffer.count)
            if n <= 0 { break }
            output.append(contentsOf: buffer[0..<n])
        }
        var status: Int32 = 0
        waitpid(child, &status, 0)
        return String(decoding: output, as: UTF8.self).split(separator: " ").compactMap { Int32($0) }
    }

    /// ov-608: the listener and every accepted connection are close-on-exec,
    /// so nothing the app starts, however it starts it, holds the socket.
    @Test func noProgramTheAppStartsInheritsItsSockets() throws {
        let path = Self.scratch()
        let control = AppControl()
        try control.start(at: path) { _, reply in
            let listener = control.listenerDescriptor
            let found = Self.inherited([listener, reply.fd])
            reply.send(["event": "inherited", "listener": found.contains(listener), "connection": found.contains(reply.fd)])
            reply.close()
        }
        defer { Self.done(control, path) }

        let lines = Self.ask(path, #"{"op":"about"}"#)
        #expect(lines.count == 1)
        #expect(lines.first?["listener"] as? Bool == false)
        #expect(lines.first?["connection"] as? Bool == false)
    }

    /// The check above can fail: a socket that isn't close-on-exec is found.
    @Test func theInheritanceCheckSeesASocketThatIsNotCloseOnExec() {
        var pair: [Int32] = [0, 0]
        #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0)
        defer { close(pair[0]); close(pair[1]) }
        #expect(Self.inherited([pair[0]]) == [pair[0]])
        AppControl.closeOnExec(pair[0])
        #expect(Self.inherited([pair[0]]).isEmpty)
    }

    @Test func depthIsCountedOutsideStringsOnly() {
        #expect(AppControl.depth(of: Data(#"{"op":"about"}"#.utf8)) == 1)
        #expect(AppControl.depth(of: Data(#"{"op":"{{[[\"{"}"#.utf8)) == 1)
        #expect(AppControl.depth(of: Data(#"{"a":[{"b":1}]}"#.utf8)) == 3)
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
        defer { Self.done(control, path) }
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
