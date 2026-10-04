import Foundation

/// The one way this app runs a child process and waits for its answer.
///
/// There used to be four hand-written copies (`CLI.run`, `DaemonClient.runRaw`,
/// `AgentStream.runCLI`, `RunnerFacts.run`). Three of them read stdout to EOF
/// and only then stderr, so a child that wrote more than a pipe's 64 KB of
/// stderr before closing stdout blocked on its write while we blocked on its
/// stdout: neither side could move, and the continuation never resumed. None
/// had a deadline, and none could be cancelled.
///
/// This one drains both pipes at once, takes an optional deadline, and
/// terminates the child when the awaiting task is cancelled.
enum ProcessRunner {
    /// What a run produced. Never throws: a child that could not start is one
    /// more way of not answering, and every caller already has a sentence for
    /// that.
    struct Result: Sendable {
        /// Why the child never started (the system's words), else nil.
        var launchFailure: String?
        var status: Int32 = -1
        var stdout = Data()
        var stderr = Data()
        /// The deadline passed and the child was terminated.
        var timedOut = false
        /// The awaiting task was cancelled and the child was terminated.
        var cancelled = false

        var succeeded: Bool {
            launchFailure == nil && !timedOut && !cancelled && status == 0
        }
    }

    /// How long to wait for a pipe to reach EOF after its child has exited.
    ///
    /// A grandchild that inherited the pipe (an ssh control master, a
    /// daemonizing helper) keeps it open long after the child we asked about is
    /// gone. What it wrote so far is the answer; waiting for its EOF is not.
    static let drainGrace: TimeInterval = 1

    /// Run `executable`, feed it `stdin`, and wait for it.
    /// - Parameters:
    ///   - stdin: written on its own thread and then closed, so a child that
    ///     fills its output pipe while we are still writing cannot wedge both.
    ///   - deadline: seconds before the child is terminated, or nil for none.
    ///   - discardStderr: for tools whose stderr is an ordinary answer.
    static func run(
        _ executable: String, _ arguments: [String],
        environment: [String: String]? = nil, stdin: Data? = nil,
        deadline: TimeInterval? = nil, discardStderr: Bool = false
    ) async -> Result {
        let handle = Handle()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                DispatchQueue.global(qos: .userInitiated).async {
                    continuation.resume(
                        returning: Self.runBlocking(
                            handle, executable, arguments, environment, stdin, deadline,
                            discardStderr))
                }
            }
        } onCancel: {
            handle.cancel()
        }
    }

    // MARK: - Plumbing

    private static func runBlocking(
        _ handle: Handle, _ executable: String, _ arguments: [String],
        _ environment: [String: String]?, _ stdin: Data?, _ deadline: TimeInterval?,
        _ discardStderr: Bool
    ) -> Result {
        var result = Result()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        if let environment { process.environment = environment }
        let out = Pipe()
        let err = Pipe()
        process.standardOutput = out
        process.standardError = discardStderr ? FileHandle.nullDevice : err
        let input = stdin.map { _ in Pipe() }
        if let input { process.standardInput = input }

        // Started under the handle's lock, so a cancel that raced ahead of the
        // launch is seen here and a cancel that follows finds a live process.
        let started: String? = handle.lock.withLock {
            if handle.cancelled { return nil }
            do { try process.run() } catch { return error.localizedDescription }
            handle.process = process
            return nil
        }
        if let started {
            result.launchFailure = started
            return result
        }
        if handle.lock.withLock({ handle.process == nil }) {
            result.cancelled = true
            return result
        }

        let stdoutBuffer = Buffer()
        let stderrBuffer = Buffer()
        let drained = DispatchGroup()
        drain(out.fileHandleForReading, into: stdoutBuffer, group: drained)
        if discardStderr {
            try? err.fileHandleForWriting.close()
            try? err.fileHandleForReading.close()
        } else {
            drain(err.fileHandleForReading, into: stderrBuffer, group: drained)
        }
        if let input, let stdin {
            // The throwing spellings: `write(_:)` raises an Objective-C
            // exception Swift cannot catch when the child is already gone.
            DispatchQueue.global(qos: .userInitiated).async {
                try? input.fileHandleForWriting.write(contentsOf: stdin)
                try? input.fileHandleForWriting.close()
            }
        }

        var watchdog: DispatchWorkItem?
        if let deadline {
            let item = DispatchWorkItem { handle.expire() }
            watchdog = item
            DispatchQueue.global(qos: .userInitiated)
                .asyncAfter(deadline: .now() + deadline, execute: item)
        }

        process.waitUntilExit()
        handle.finish()
        watchdog?.cancel()

        // Our copy of the write end of each pipe was closed by `run()`; the
        // readers see EOF when the child and anything it spawned let go. A
        // holdout gets `drainGrace`, and its readers are left to die with it.
        if drained.wait(timeout: .now() + drainGrace) == .timedOut {
            try? out.fileHandleForReading.close()
            if !discardStderr { try? err.fileHandleForReading.close() }
        }

        result.status = process.terminationStatus
        result.stdout = stdoutBuffer.snapshot()
        result.stderr = stderrBuffer.snapshot()
        let flags = handle.flags
        result.timedOut = flags.timedOut
        result.cancelled = flags.cancelled
        return result
    }

    private static func drain(_ reader: FileHandle, into buffer: Buffer, group: DispatchGroup) {
        group.enter()
        DispatchQueue.global(qos: .userInitiated).async {
            while let chunk = try? reader.read(upToCount: 65536), !chunk.isEmpty {
                buffer.append(chunk)
            }
            group.leave()
        }
    }

    /// Bytes arriving on one thread and read on another.
    private final class Buffer: @unchecked Sendable {
        private let lock = NSLock()
        private var data = Data()
        func append(_ chunk: Data) { lock.withLock { data.append(chunk) } }
        func snapshot() -> Data { lock.withLock { data } }
    }

    /// A `Process` and the questions a watchdog or a cancel may ask about it.
    ///
    /// The lock keeps `terminate()` from running against a process already
    /// reaped: Foundation raises an Objective-C exception for that, which Swift
    /// cannot catch.
    private final class Handle: @unchecked Sendable {
        let lock = NSLock()
        var process: Process?
        var cancelled = false
        private var timedOut = false
        private var reaped = false

        var flags: (timedOut: Bool, cancelled: Bool) {
            lock.withLock { (timedOut, cancelled) }
        }

        func cancel() {
            lock.withLock {
                cancelled = true
                terminateLocked()
            }
        }

        func expire() {
            lock.withLock {
                guard !reaped else { return }
                timedOut = true
                terminateLocked()
            }
        }

        func finish() { lock.withLock { reaped = true } }

        private func terminateLocked() {
            guard !reaped, let process, process.isRunning else { return }
            process.terminate()
        }
    }
}
