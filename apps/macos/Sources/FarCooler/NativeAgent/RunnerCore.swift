import AgentKit
import CFarCoolerClient
import Foundation

/// This Mac's runner over the client core (ov-372): one connection to the
/// local daemon's socket, shared by every native pane, where the rest of the
/// Mac runs the CLI a process at a time.
///
/// The FFI is asynchronous underneath and polled at its boundary: a call
/// hands back a ticket and `drain` matches finished results to the
/// continuations waiting on them. An actor, so the polling, the JSON of each
/// answer and the continuations all live off the main thread; a caller gets
/// the result's bytes undecoded (`AgentRowSource`).
actor RunnerCore {
    nonisolated(unsafe) private var handle: UnsafeMutableRawPointer?
    private var waiting: [UInt64: CheckedContinuation<Data, Error>] = [:]
    private var pump: Task<Void, Never>?
    /// What the runner's hello offered, empty until connected.
    private(set) var capabilities: Set<String> = []

    /// A failed call, by what the core said about it.
    enum Failure: LocalizedError, Equatable {
        /// The runner understood and said no: its error code's word, and the
        /// conflict or argument it named, if any.
        case refused(String, word: String?, what: String?)
        /// The link is gone.
        case lost(String)
        case notConnected

        var errorDescription: String? {
            switch self {
            case .refused(let message, _, _), .lost(let message): message
            case .notConnected: "The runner isn't connected."
            }
        }

        var word: String? {
            if case .refused(_, let word, _) = self { return word }
            return nil
        }

        var what: String? {
            if case .refused(_, _, let what) = self { return what }
            return nil
        }
    }

    init() {
        handle = farcooler_client_new()
    }

    deinit {
        pump?.cancel()
        if let handle { farcooler_client_free(handle) }
    }

    /// The local daemon's socket: the app socket's directory
    /// (`AppControl.socketPath`), which is the daemon's runtime directory.
    static func localSocket(environment: [String: String] = ProcessInfo.processInfo.environment) -> String {
        (AppControl.socketPath(environment: environment) as NSString).deletingLastPathComponent + "/farcoolerd.sock"
    }

    var isConnected: Bool {
        guard let handle else { return false }
        return farcooler_client_connected(handle)
    }

    /// Connect to the daemon at `socket`; what its hello offers.
    @discardableResult
    func connect(socket: String) async throws -> Set<String> {
        let data = try await submit { handle in
            Self.json(["socket": socket]).withCString { farcooler_client_connect(handle, $0) }
        }
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        capabilities = Set(object["capabilities"] as? [String] ?? [])
        return capabilities
    }

    /// Invoke a method; its result's JSON, undecoded.
    func call(_ method: String, _ args: [String: any Sendable]) async throws -> Data {
        try await submit { handle in
            Self.json(args).withCString { json in method.withCString { farcooler_client_call(handle, $0, json) } }
        }
    }

    private func submit(_ start: (UnsafeMutableRawPointer) -> UInt64) async throws -> Data {
        guard let handle else { throw Failure.notConnected }
        let ticket = start(handle)
        guard ticket != 0 else { throw Failure.refused("The call couldn't be made.", word: nil, what: nil) }
        startPumping()
        return try await withCheckedThrowingContinuation { waiting[ticket] = $0 }
    }

    /// Drain while anything waits, and stop when nothing does: an idle
    /// native view costs no wake-ups beyond its one held follow per pane.
    private func startPumping() {
        guard pump == nil else { return }
        pump = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, await self.drain() else { break }
                try? await Task.sleep(for: .milliseconds(15))
            }
        }
    }

    /// Hand each finished result to its caller. False when nothing is left
    /// waiting, which ends the pump.
    private func drain() -> Bool {
        guard let handle else { return false }
        while let raw = farcooler_client_poll(handle) {
            let data = Data(String(cString: raw).utf8)
            guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let ticket = (object["ticket"] as? NSNumber)?.uint64Value,
                let continuation = waiting.removeValue(forKey: ticket)
            else { continue }
            if object["ok"] as? Bool == true {
                let result = object["result"] ?? [String: Any]()
                continuation.resume(
                    returning: (try? JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed])) ?? Data("{}".utf8))
            } else {
                let message = object["error"] as? String ?? "The runner refused the request."
                if object["disconnected"] as? Bool == true || object["trouble"] != nil {
                    continuation.resume(throwing: Failure.lost(message))
                } else {
                    continuation.resume(
                        throwing: Failure.refused(
                            message, word: RunnerRefusal.word(inAnswerLine: object), what: RunnerRefusal.what(inAnswerLine: object)))
                }
            }
        }
        // Fleet news isn't this connection's to read: the Mac follows the
        // fleet through the CLI. Drained so the queue never grows.
        while farcooler_client_next_event(handle) != nil {}
        if waiting.isEmpty {
            pump = nil
            return false
        }
        return true
    }

    private static func json(_ object: [String: Any]) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }
}

/// One pane's rows over a `RunnerCore`: `agent.rows` and `agent.rows_follow`,
/// their answers handed to `AgentRowLedger` undecoded.
struct CoreRowSource: AgentRowSource {
    let core: RunnerCore
    let terminal: String

    func page(before: UInt64?, limit: Int) async throws -> Data {
        var args: [String: any Sendable] = ["terminal": terminal, "limit": limit]
        if let before { args["before"] = before }
        return try await mapped { try await core.call("agent.rows", args) }
    }

    func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
        try await mapped {
            try await core.call("agent.rows_follow", ["terminal": terminal, "epoch": epoch, "afterRev": afterRev, "waitMs": waitMs])
        }
    }

    /// A runner that doesn't serve rows, or a pane that's gone, is not
    /// worth retrying.
    private func mapped(_ call: () async throws -> Data) async throws -> Data {
        do {
            return try await call()
        } catch let failure as RunnerCore.Failure where ["capability-unsupported", "not-found"].contains(failure.word ?? "") {
            throw AgentRowsUnavailable()
        }
    }
}
