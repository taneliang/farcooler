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

    func start(binary: String, terminal: String, environment: [String: String]) {
        stop()

        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = ["terminal", "input", terminal]
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
        // A closed pipe throws rather than crashing the app.
        try? stdin.write(contentsOf: data)
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
