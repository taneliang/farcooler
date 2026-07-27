import Foundation

/// A live byte stream from one terminal.
///
/// Runs `overnight terminal stream <id>`, which emits the retained history and
/// then pipes the pane's raw output. Bytes arrive as tmux produces them, so the
/// emulator sees cursor motion, redraws and animation, none of which a polled
/// snapshot of the settled screen can ever show.
final class TerminalStream {
    private var process: Process?
    private let onBytes: @Sendable ([UInt8]) -> Void
    private let onEnd: @Sendable () -> Void

    init(
        onBytes: @escaping @Sendable ([UInt8]) -> Void,
        onEnd: @escaping @Sendable () -> Void = {}
    ) {
        self.onBytes = onBytes
        self.onEnd = onEnd
    }

    func start(binary: String, terminal: String, environment: [String: String]) {
        stop()

        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = ["terminal", "stream", terminal]
        p.environment = environment

        let out = Pipe()
        p.standardOutput = out
        p.standardError = Pipe()

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
        guard let p = process else { return }
        process = nil
        if p.isRunning { p.terminate() }
    }

    deinit { stop() }
}
