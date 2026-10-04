import Foundation

/// One long-lived `farcooler terminal agent-subscribe --follow`, read a line
/// at a time (ov-229).
///
/// A chat view used to run `agent-subscribe` every 200 ms: five processes a
/// second per chat on screen, and five ssh sessions a second on a remote
/// runner, nearly all of them answering "nothing new", and each one decoded
/// on the main actor. The follow mode keeps one process and one link open,
/// and prints only batches that hold something. A quiet chat costs this app
/// nothing. See `crates/cli/src/agent_follow.rs`.
final class AgentFollow: @unchecked Sendable {
    private var process: Process?
    private var output: FileHandle?
    private let lock = NSLock()
    private var printed = false
    private var stderr = Data()

    /// Start the process. `onLine` gets each JSON line; `onEnd` gets whether
    /// any line ever arrived and what the process said on stderr, once, when
    /// it exits — never after `stop()`.
    func start(
        binary: String, arguments: [String], environment: [String: String],
        onLine: @escaping @Sendable (Data) -> Void,
        onEnd: @escaping @Sendable (_ printed: Bool, _ stderr: String) -> Void
    ) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = arguments
        p.environment = environment
        let out = Pipe()
        let err = Pipe()
        p.standardOutput = out
        p.standardError = err

        let buffer = LineBuffer()
        let handle = out.fileHandleForReading
        handle.readabilityHandler = { [weak self] h in
            let chunk = h.availableData
            guard !chunk.isEmpty else { return }
            let lines = buffer.take(chunk)
            if !lines.isEmpty { self?.markPrinted() }
            for line in lines { onLine(line) }
        }
        err.fileHandleForReading.readabilityHandler = { [weak self] h in
            let chunk = h.availableData
            guard !chunk.isEmpty else { return }
            self?.appendError(chunk)
        }
        p.terminationHandler = { [weak self] _ in
            handle.readabilityHandler = nil
            err.fileHandleForReading.readabilityHandler = nil
            guard let self, let (printed, said) = self.finish() else { return }
            onEnd(printed, said)
        }

        lock.lock()
        process = p
        output = handle
        lock.unlock()
        do {
            try p.run()
        } catch {
            _ = finish()
            onEnd(false, error.localizedDescription)
        }
    }

    /// Stop reading and end the process. `onEnd` is not called for this.
    func stop() {
        lock.lock()
        let p = process
        process = nil
        output?.readabilityHandler = nil
        output = nil
        lock.unlock()
        if let p, p.isRunning { p.terminate() }
    }

    deinit { stop() }

    private func markPrinted() {
        lock.lock()
        printed = true
        lock.unlock()
    }

    private func appendError(_ chunk: Data) {
        lock.lock()
        stderr.append(chunk)
        lock.unlock()
    }

    /// What `onEnd` reports, or nil if this was stopped on purpose.
    private func finish() -> (Bool, String)? {
        lock.lock()
        defer { lock.unlock() }
        guard process != nil else { return nil }
        process = nil
        output = nil
        return (printed, String(decoding: stderr, as: UTF8.self))
    }
}
