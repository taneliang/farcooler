import Foundation

/// A live byte stream from one terminal.
///
/// Runs `farcooler terminal stream <id>`, which emits the retained history and
/// then pipes the pane's raw output. Bytes arrive as tmux produces them, so the
/// emulator sees cursor motion, redraws and animation, none of which a polled
/// snapshot of the settled screen can ever show.
final class TerminalStream {
    private var process: Process?
    /// Held open for as long as this stream should live. See `start`.
    private var attachment: Pipe?
    private let onBytes: @Sendable ([UInt8]) -> Void
    private let onEnd: @Sendable () -> Void

    init(
        onBytes: @escaping @Sendable ([UInt8]) -> Void,
        onEnd: @escaping @Sendable () -> Void = {}
    ) {
        self.onBytes = onBytes
        self.onEnd = onEnd
    }

    func start(
        binary: String, terminal: String, environment: [String: String], host: [String] = []
    ) {
        stop()

        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = host + ["terminal", "stream", terminal]
        p.environment = environment

        let out = Pipe()
        p.standardOutput = out
        p.standardError = Pipe()

        // An open pipe on stdin, which nothing ever writes to.
        //
        // It is there to stay open, because closed stdin is how the far end
        // learns that nobody is watching any more. Without it a child inherits
        // this application's own stdin — `/dev/null` for anything launched from
        // the Dock — which is at end of stream before it is read, and a remote
        // stream would end the instant ssh forwarded that along. With it, the
        // stream ends when this process ends, which is exactly when it should.
        let attachment = Pipe()
        p.standardInput = attachment
        self.attachment = attachment

        // Incremental reads. Never readDataToEndOfFile: this process only ends
        // when the terminal does.
        let handle = out.fileHandleForReading
        handle.readabilityHandler = { [onBytes] h in
            let data = h.availableData
            if data.isEmpty { return }
            onBytes([UInt8](data))
        }

        p.terminationHandler = { [onEnd] _ in
            handle.readabilityHandler = nil
            onEnd()
        }

        do {
            try p.run()
            process = p
        } catch {
            process = nil
        }
    }

    func stop() {
        // Closed first, so a remote stream is told to end even if the local
        // `terminate` only reaches the ssh client sitting in front of it.
        try? attachment?.fileHandleForWriting.close()
        attachment = nil
        guard let p = process else { return }
        process = nil
        if p.isRunning { p.terminate() }
    }

    deinit { stop() }
}
