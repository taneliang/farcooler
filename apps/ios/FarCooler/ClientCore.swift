import Foundation
import FarCoolerClient

/// Swift's view of the Rust client core.
///
/// Everything about talking to a host — SSH, the protocol, the shapes of the
/// answers — happens on the other side of this file. What is left here is
/// turning a polling C API into something Swift concurrency can await.
///
/// The core is asynchronous underneath and synchronous at its boundary, which
/// is deliberate: bridging two async runtimes through callbacks means one of
/// them is always wrong about which thread it is on. So calls hand back a
/// ticket, a timer drains finished results, and this actor matches them to the
/// continuations waiting for them.
actor ClientCore {
    private var handle: UnsafeMutableRawPointer?
    private var waiting: [UInt64: CheckedContinuation<Data, Error>] = [:]
    private var pump: Task<Void, Never>?
    private var streams: [String: Stream] = [:]

    /// What a live terminal stream reports back. Not a continuation, because a
    /// stream is not an answer to anything: it produces bytes until the pane
    /// ends or someone stops watching.
    private struct Stream {
        let onChunk: @Sendable ([UInt8]) -> Void
        /// Nil means the pane ended; a string is the reason it could not be
        /// read. Either way this stream is over and has been forgotten here.
        let onEnd: @Sendable (String?) -> Void
    }

    enum CoreError: LocalizedError {
        case notStarted
        case rejected(String)
        /// The link is gone, as opposed to the request being refused.
        ///
        /// Answered by the core rather than worked out from the message here:
        /// Rust still has the error's type at the moment it is produced, and
        /// `Connection.Failure` matching substrings is a compromise the
        /// connect path makes because a connect failure genuinely arrives as
        /// prose. A call on a live session need not make it.
        case disconnected(String)
        case malformed

        var errorDescription: String? {
            switch self {
            case .notStarted: return "The client core could not be started."
            case .rejected(let message): return message
            case .disconnected(let message): return message
            case .malformed: return "The client core returned something unreadable."
            }
        }
    }

    init() {
        handle = farcooler_client_new()
    }

    deinit {
        pump?.cancel()
        if let handle { farcooler_client_free(handle) }
    }

    /// Connect to a host. `config` is the JSON the core documents.
    func connect(config: [String: Any]) async throws -> Data {
        try await submit { handle in
            withJSON(config) { farcooler_client_connect(handle, $0) }
        }
    }

    /// Invoke a method.
    func call(_ method: String, _ args: [String: Any] = [:]) async throws -> Data {
        try await submit { handle in
            withJSON(args) { json in
                method.withCString { farcooler_client_call(handle, $0, json) }
            }
        }
    }

    var isConnected: Bool {
        guard let handle else { return false }
        return farcooler_client_connected(handle)
    }

    /// Watch a terminal's output as it happens.
    ///
    /// The core opens a second ssh channel carrying nothing but this pane's
    /// bytes — the same bytes tmux writes, in the order it writes them — so
    /// what arrives here is what a terminal emulator eats, not a snapshot of
    /// the screen after it settled. Cursor motion, redraws and animation exist
    /// on the wire again, and the delay is one network round trip rather than
    /// however long until the next poll.
    ///
    /// Returns false when there is no ssh session to open a channel on, which
    /// is a real answer and not an error: the caller falls back to polling.
    func startStream(
        _ terminal: String,
        onChunk: @escaping @Sendable ([UInt8]) -> Void,
        onEnd: @escaping @Sendable (String?) -> Void
    ) -> Bool {
        guard let handle else { return false }
        startPumping()
        // Registered before starting, not after: the first chunk can be
        // delivered by a pump tick that lands between the two, and a stream
        // nothing is listening for yet would drop the replay of the screen —
        // the one chunk whose loss is visible, because it is the whole screen.
        streams[terminal] = Stream(onChunk: onChunk, onEnd: onEnd)
        let started = terminal.withCString { farcooler_client_stream_start(handle, $0) }
        if !started { streams[terminal] = nil }
        return started
    }

    /// Stop watching. Safe when nothing is running.
    func stopStream(_ terminal: String) {
        streams[terminal] = nil
        guard let handle else { return }
        terminal.withCString { farcooler_client_stream_stop(handle, $0) }
    }

    private func submit(
        _ start: (UnsafeMutableRawPointer) -> UInt64
    ) async throws -> Data {
        guard let handle else { throw CoreError.notStarted }
        startPumping()

        let ticket = start(handle)
        guard ticket != 0 else { throw CoreError.malformed }

        return try await withCheckedThrowingContinuation { continuation in
            waiting[ticket] = continuation
        }
    }

    /// Drain finished results into the continuations waiting for them.
    ///
    /// 20 ms is well under a frame and far above the cost of one atomic read of
    /// an empty queue, which is what this almost always is.
    private func startPumping() {
        guard pump == nil else { return }
        pump = Task { [weak self] in
            while !Task.isCancelled {
                await self?.drain()
                guard let interval = await self?.pumpInterval else { return }
                try? await Task.sleep(for: interval)
            }
        }
    }

    /// How often to look, which is not the same question while a stream is
    /// open.
    ///
    /// 20 ms is nothing next to a round trip to a host, so it is the right
    /// price for noticing that a request finished. It is not nothing next to a
    /// keystroke echoing back in 6 ms, though — a fixed 20 ms tick would be
    /// most of the latency of the fast path it is sitting in front of, and
    /// would make the stream feel like the polling it replaced. So the pump
    /// runs hot exactly while something is streaming, and settles back the
    /// moment nothing is.
    private var pumpInterval: Duration {
        streams.isEmpty ? .milliseconds(20) : .milliseconds(4)
    }

    private func drain() {
        guard let handle else { return }
        while let raw = farcooler_client_poll(handle) {
            let json = String(cString: raw)
            guard
                let data = json.data(using: .utf8),
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { continue }

            // A stream line carries no ticket, because nothing asked for it.
            if let terminal = object["stream"] as? String {
                deliver(terminal, object)
                continue
            }

            guard
                let ticket = object["ticket"] as? UInt64,
                let continuation = waiting.removeValue(forKey: ticket)
            else { continue }

            if object["ok"] as? Bool == true {
                let result = object["result"] ?? [:]
                if let payload = try? JSONSerialization.data(withJSONObject: result) {
                    continuation.resume(returning: payload)
                } else {
                    continuation.resume(throwing: CoreError.malformed)
                }
            } else {
                let message = object["error"] as? String ?? "the host refused the request"
                if object["disconnected"] as? Bool == true {
                    continuation.resume(throwing: CoreError.disconnected(message))
                } else {
                    continuation.resume(throwing: CoreError.rejected(message))
                }
            }
        }
    }

    /// Hand one stream line to whoever is watching that terminal.
    ///
    /// Called from inside `drain`, so chunks reach the handler in exactly the
    /// order the host produced them — which is the one property a byte stream
    /// cannot do without. What the handler does with that ordering afterwards
    /// is its own problem; see `TerminalSession.Inbox`.
    private func deliver(_ terminal: String, _ line: [String: Any]) {
        guard let stream = streams[terminal] else { return }
        if let chunk = line["chunk"] as? String {
            if let bytes = Data(base64Encoded: chunk) { stream.onChunk([UInt8](bytes)) }
            return
        }
        // Anything that is not a chunk ends the stream: the pane finished, or
        // it could not be opened at all.
        streams[terminal] = nil
        stream.onEnd(line["error"] as? String)
    }
}

/// Serialize a dictionary and hand it to C as a NUL-terminated string.
///
/// Scoped rather than returned: the C side copies what it needs during the
/// call, so the buffer must outlive the call and nothing more.
private func withJSON<T>(_ object: [String: Any], _ body: (UnsafePointer<CChar>) -> T) -> T {
    let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
    let text = String(data: data, encoding: .utf8) ?? "{}"
    return text.withCString(body)
}
