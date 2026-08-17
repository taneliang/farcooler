import Foundation

/// A persistent input channel to one terminal.
///
/// Keystrokes go down an already-open pipe rather than spawning a process each
/// time. A per-keystroke process pays for a SQLite open and a full tmux
/// inventory before a single byte moves, which is most of the latency a typist
/// feels. Writes are also inherently ordered here, so this replaces the serial
/// queue the previous path needed.
final class TerminalInput {
    private var process: Process?
    private var stdin: FileHandle?
    private let lock = NSLock()

    func start(
        binary: String, terminal: String, environment: [String: String], host: [String] = []
    ) {
        stop()

        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = host + ["terminal", "input", terminal]
        p.environment = environment

        let inPipe = Pipe()
        p.standardInput = inPipe
        p.standardOutput = Pipe()
        p.standardError = Pipe()

        do {
            try p.run()
            process = p
            stdin = inPipe.fileHandleForWriting
        } catch {
            process = nil
            stdin = nil
        }
    }

    /// Send exact bytes. One hex run per line, in the order submitted.
    func send(_ bytes: [UInt8]) {
        guard !bytes.isEmpty else { return }
        let hex = bytes.map { String(format: "%02x", $0) }.joined() + "\n"
        guard let data = hex.data(using: .ascii) else { return }

        // Serialize writes so two keystrokes cannot interleave mid-line.
        lock.lock()
        defer { lock.unlock() }
        guard let stdin else { return }
        // A closed pipe throws rather than crashing the app — but ONLY because
        // `Entry.ignoreSIGPIPE` runs first. Left at its default, writing here
        // kills the whole app on a signal, before `write` returns anything for
        // `try?` to catch. That is not a hypothetical: sleep drops the ssh
        // connection behind a remote pane, the child exits, and the first
        // keystroke after wake lands on a pipe with no reader.
        do {
            try stdin.write(contentsOf: data)
        } catch {
            // The child is gone and nothing further can reach it. Dropped
            // rather than retried on every subsequent keystroke, and dropped
            // WITHOUT terminating the process here — `stop()` is the teardown,
            // and this is being called from the typing path.
            //
            // The pane is not stranded: `DaemonClient` bumps `linkGeneration`
            // when a runner's link is replaced, which re-attaches the surface
            // and starts a new input channel. What is lost is the keystroke
            // that discovered the corpse.
            self.stdin = nil
        }
    }

    func stop() {
        lock.lock()
        defer { lock.unlock() }
        try? stdin?.close()
        stdin = nil
        if let p = process, p.isRunning { p.terminate() }
        process = nil
    }

    deinit { stop() }
}
