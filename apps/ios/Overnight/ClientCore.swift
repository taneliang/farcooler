import Foundation
import OvernightClient

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

    enum CoreError: LocalizedError {
        case notStarted
        case rejected(String)
        case malformed

        var errorDescription: String? {
            switch self {
            case .notStarted: return "The client core could not be started."
            case .rejected(let message): return message
            case .malformed: return "The client core returned something unreadable."
            }
        }
    }

    init() {
        handle = overnight_client_new()
    }

    deinit {
        pump?.cancel()
        if let handle { overnight_client_free(handle) }
    }

    /// Connect to a host. `config` is the JSON the core documents.
    func connect(config: [String: Any]) async throws -> Data {
        try await submit { handle in
            withJSON(config) { overnight_client_connect(handle, $0) }
        }
    }

    /// Invoke a method.
    func call(_ method: String, _ args: [String: Any] = [:]) async throws -> Data {
        try await submit { handle in
            withJSON(args) { json in
                method.withCString { overnight_client_call(handle, $0, json) }
            }
        }
    }

    var isConnected: Bool {
        guard let handle else { return false }
        return overnight_client_connected(handle)
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
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
    }

    private func drain() {
        guard let handle else { return }
        while let raw = overnight_client_poll(handle) {
            let json = String(cString: raw)
            guard
                let data = json.data(using: .utf8),
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
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
                continuation.resume(throwing: CoreError.rejected(message))
            }
        }
    }
}

/// Serialise a dictionary and hand it to C as a NUL-terminated string.
///
/// Scoped rather than returned: the C side copies what it needs during the
/// call, so the buffer must outlive the call and nothing more.
private func withJSON<T>(_ object: [String: Any], _ body: (UnsafePointer<CChar>) -> T) -> T {
    let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
    let text = String(data: data, encoding: .utf8) ?? "{}"
    return text.withCString(body)
}
