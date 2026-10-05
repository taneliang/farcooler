import AgentKit
import Foundation

/// The command line's way into this running app (ov-302): `farcooler app
/// version` and `farcooler app update`, from `crates/cli/src/app_update.rs`.
///
/// A Unix socket, `app.sock`, beside this channel's daemon socket. Reachable
/// only from this Mac, and only by this user: the file sits in a 0700
/// directory with mode 0600, and each connection's peer is asked its user ID
/// (`getpeereid`) and dropped unless it is ours. Nothing listens on a network,
/// so neither a remote device nor `--runner` over ssh can reach it.
///
/// One request per connection, JSON on one line; answers are JSON lines, and
/// the app hangs up after the last. That conversation is described in the
/// CLI's module, which is its other half.
final class AppControl: @unchecked Sendable {
    /// One open connection, for writing answers back on.
    final class Reply: @unchecked Sendable {
        private let fd: Int32
        private let queue: DispatchQueue
        private var closed = false

        init(fd: Int32, queue: DispatchQueue) {
            self.fd = fd
            self.queue = queue
        }

        /// Write `object` as one line. Silently dropped once the CLI hung up:
        /// it stopped listening, and that isn't this app's failure.
        func send(_ object: [String: Any]) {
            guard var data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else { return }
            data.append(0x0A)
            let line = data
            queue.async { [self] in
                guard !closed else { return }
                line.withUnsafeBytes { raw in
                    var offset = 0
                    while offset < raw.count {
                        let wrote = Darwin.write(fd, raw.baseAddress! + offset, raw.count - offset)
                        if wrote <= 0 { return }
                        offset += wrote
                    }
                }
            }
        }

        /// Hang up, after whatever was sent before.
        func close() {
            queue.async { [self] in
                guard !closed else { return }
                closed = true
                Darwin.close(fd)
            }
        }
    }

    /// What the CLI asked for, and where to answer.
    typealias Handler = @Sendable (_ request: [String: Any], _ reply: Reply) -> Void

    private let queue = DispatchQueue(label: "com.farcooler.app-control")
    private var listener: Int32 = -1
    private var source: DispatchSourceRead?
    /// Every connection is asked who it is. Replaced in tests, which cannot
    /// be another user.
    var ownUser: uid_t = getuid()

    /// The socket's path: `FARCOOLER_HOME` when it is set, as the CLI and the
    /// daemon read it, else this channel's runtime directory. The mapping is
    /// `crates/daemon/src/paths.rs`'s, which the directories crate turns into
    /// `~/Library/Application Support/com.farcooler.<name>` on a Mac.
    static func socketPath(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        channel: String = AppVersion.channel,
        home: String = NSHomeDirectory()
    ) -> String {
        if let over = environment["FARCOOLER_HOME"] { return over + "/app.sock" }
        let name =
            switch channel {
            case "stable": "FarCooler"
            case "preview": "FarCoolerPreview"
            case "canary": "FarCoolerCanary"
            default: "FarCoolerLocal"
            }
        return "\(home)/Library/Application Support/com.farcooler.\(name)/app.sock"
    }

    /// Why listening failed, for the log. The app works without the socket;
    /// only `farcooler app` goes without.
    enum Failure: Error, Equatable {
        case pathTooLong
        /// Another copy of this app answers there already.
        case taken
        case system(String)
    }

    /// Start listening at `path`, taking over a socket nobody answers.
    func start(at path: String, handler: @escaping Handler) throws(Failure) {
        let directory = (path as NSString).deletingLastPathComponent
        if !FileManager.default.fileExists(atPath: directory) {
            try? FileManager.default.createDirectory(
                atPath: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard path.utf8.count < capacity else { throw .pathTooLong }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: Array(path.utf8) + [0])
        }

        // A file left by a copy that quit is taken over; one a running copy
        // answers on is left to it.
        if FileManager.default.fileExists(atPath: path) {
            if Self.answers(path) { throw .taken }
            unlink(path)
        }

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw .system("socket") }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else {
            Darwin.close(fd)
            throw .system("bind")
        }
        chmod(path, 0o600)
        guard listen(fd, 8) == 0 else {
            Darwin.close(fd)
            throw .system("listen")
        }
        listener = fd

        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.accept(handler: handler) }
        source.setCancelHandler { Darwin.close(fd) }
        source.resume()
        self.source = source
    }

    /// Stop listening, and remove the socket file if it is still ours.
    func stop(removing path: String) {
        source?.cancel()
        source = nil
        unlink(path)
    }

    private func accept(handler: @escaping Handler) {
        let fd = Darwin.accept(listener, nil, nil)
        guard fd >= 0 else { return }
        // A CLI that hangs up mid-answer must not take the app down with
        // SIGPIPE.
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        var uid: uid_t = 0
        var gid: gid_t = 0
        guard getpeereid(fd, &uid, &gid) == 0, Self.admits(peer: uid, own: ownUser) else {
            Darwin.close(fd)
            return
        }
        // A request that never finishes arriving is dropped after five seconds.
        var wait = timeval(tv_sec: 5, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &wait, socklen_t(MemoryLayout<timeval>.size))
        let reply = Reply(fd: fd, queue: DispatchQueue(label: "com.farcooler.app-control.reply"))
        DispatchQueue.global().async {
            guard let line = Self.readLine(fd),
                let request = try? JSONSerialization.jsonObject(with: line) as? [String: Any]
            else {
                reply.close()
                return
            }
            handler(request, reply)
        }
    }

    /// Only this user. Root is not this user either.
    static func admits(peer: uid_t, own: uid_t) -> Bool { peer == own }

    /// One request line, at most 4 KB.
    private static func readLine(_ fd: Int32) -> Data? {
        var line = Data()
        var byte: UInt8 = 0
        while line.count < 4096 {
            guard read(fd, &byte, 1) == 1 else { return nil }
            if byte == 0x0A { return line }
            line.append(byte)
        }
        return nil
    }

    /// Whether something answers at `path`.
    static func answers(_ path: String) -> Bool {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        guard path.utf8.count < MemoryLayout.size(ofValue: address.sun_path) else { return false }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: Array(path.utf8) + [0]) }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        defer { Darwin.close(fd) }
        return withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) == 0
            }
        }
    }
}

/// What the CLI's requests do, in this app.
enum AppControlRequests {
    /// The app's own facts, as `about` answers them.
    @MainActor
    static var about: [String: Any] {
        [
            "version": AppVersion.marketing, "build": AppVersion.build, "channel": AppVersion.channel,
            "display": AppVersion.display, "pid": Int(getpid()), "path": Bundle.main.bundlePath,
        ]
    }

    /// Start answering the CLI. Called once the app has finished launching.
    @MainActor
    static func listen() {
        let path = AppControl.socketPath()
        do {
            try AppControl.shared.start(at: path) { request, reply in
                let op = request["op"] as? String ?? ""
                let relaunch = request["relaunch"] as? Bool ?? true
                Task { @MainActor in handle(op: op, relaunch: relaunch, reply: reply) }
            }
        } catch {
            NSLog("Far Cooler: farcooler app can't reach this app: \(error) at \(path)")
        }
    }

    @MainActor
    static func handle(op: String, relaunch: Bool, reply: AppControl.Reply) {
        switch op {
        case "about":
            reply.send(["event": "about", "app": about])
            reply.close()
        case "version":
            Updates.shared.latest { latest, unknown in
                var answer: [String: Any] = ["event": "version", "app": about]
                if let latest { answer["latest"] = latest }
                if let unknown { answer["unknown"] = unknown }
                reply.send(answer)
                reply.close()
            }
        case "update":
            Updates.shared.run(
                UpdateErrand(relaunch: relaunch, from: about) { event in reply.send(event) } done: { reply.close() })
        default:
            reply.send(["event": "refused", "code": "unknown-request"])
            reply.close()
        }
    }
}

extension AppControl {
    static let shared = AppControl()
}
