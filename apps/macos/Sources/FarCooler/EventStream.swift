import Foundation

/// Changes pushed from the daemon, one JSON object per line.
///
/// The app used to poll every couple of seconds, which is the wrong shape for
/// this product twice over: too slow to notice an agent asking a question, and
/// pure waste on a fleet where nothing is happening. The daemon now derives
/// activity once for everyone and sends only what changed.
///
/// A line reader over a subprocess rather than a socket client, for the same
/// reason everything else here is: the CLI is the app's transport, and one
/// transport with one set of bugs beats two.
/// One pushed change.
///
/// Decoded here rather than handed across as a dictionary: `[String: Any]` is
/// not `Sendable`, and passing one from the reader's queue to the main actor is
/// a data race the compiler is right to refuse. A typed value is also the thing
/// the UI actually wants.
struct TerminalEvent: Sendable, Decodable {
    var id: String
    var short: String
    var workspace: String
    var title: String
    var preset: String
    var state: String
    var activity: String?
}

/// A workspace's tiling, pushed whole.
///
/// Whole rather than as a diff, and that is the daemon's decision showing
/// through: layout is changed by this app, by the CLI, and by agents driving the
/// CLI at the same time, so a client that missed one event converges on the next
/// instead of applying a delta to a state it may not hold.
/// The pushed form is the read form.
///
/// There used to be a second shape here — member ids instead of member objects —
/// with a conversion between them, and the conversion was where a group arrived
/// with no idea which pane was focused. The daemon now pushes exactly what
/// `layout show` returns, so this decodes the same type and there is nothing to
/// convert or to get wrong.
struct LayoutEvent: Sendable, Decodable {
    var workspace: String
    var groups: [PaneGroup]
}

/// Which resource a line is about.
private struct EventKind: Decodable {
    var kind: String
}

final class EventStream {
    private var process: Process?
    private let onEvent: @Sendable (TerminalEvent) -> Void
    private let onLayout: @Sendable (LayoutEvent) -> Void
    /// The set of workspaces changed — a worktree appeared, vanished, or moved
    /// between shown and hidden. Carries nothing; the reconciler that emits it
    /// can both create and delete rows in one pass, and a client re-reads the
    /// fleet rather than applying this as a delta.
    private let onFleet: @Sendable () -> Void
    private let onEnd: @Sendable () -> Void

    init(
        onEvent: @escaping @Sendable (TerminalEvent) -> Void,
        onLayout: @escaping @Sendable (LayoutEvent) -> Void = { _ in },
        onFleet: @escaping @Sendable () -> Void = {},
        onEnd: @escaping @Sendable () -> Void = {}
    ) {
        self.onEvent = onEvent
        self.onLayout = onLayout
        self.onFleet = onFleet
        self.onEnd = onEnd
    }

    func start(binary: String, environment: [String: String], host: [String] = []) {
        stop()

        let p = Process()
        p.executableURL = URL(fileURLWithPath: binary)
        p.arguments = host + ["events"]
        p.environment = environment

        let out = Pipe()
        p.standardOutput = out
        p.standardError = Pipe()

        // Lines can arrive split across reads, so a partial one is held until
        // its newline shows up. Parsing half an object would be worse than
        // waiting for the rest of it.
        //
        // The buffer is a reference type because the read handler is called on
        // a queue Foundation owns; a captured `var` would be shared mutable
        // state across threads, which the compiler is right to refuse.
        let buffer = LineBuffer()
        let handle = out.fileHandleForReading
        handle.readabilityHandler = { [onEvent, onLayout, onFleet] h in
            let chunk = h.availableData
            if chunk.isEmpty { return }
            let decoder = JSONDecoder()
            for line in buffer.take(chunk) {
                // Dispatched on `kind` rather than by trying each shape in turn.
                // Guessing worked while there was one shape; with two, a layout
                // line that happened to decode as a terminal would have been
                // applied as one.
                guard let kind = try? decoder.decode(EventKind.self, from: line) else { continue }
                switch kind.kind {
                case "terminal":
                    if let event = try? decoder.decode(TerminalEvent.self, from: line) {
                        onEvent(event)
                    }
                case "layout":
                    if let event = try? decoder.decode(LayoutEvent.self, from: line) {
                        onLayout(event)
                    }
                case "fleet":
                    onFleet()
                // Resources this app does not track yet are skipped, not an error.
                default: continue
                }
            }
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
            onEnd()
        }
    }

    func stop() {
        guard let p = process else { return }
        process = nil
        if p.isRunning { p.terminate() }
    }

    deinit { stop() }
}


/// Accumulates bytes and hands back whole lines.
///
/// Foundation calls the read handler on its own queue, so this is locked rather
/// than assumed single-threaded — the assumption would hold right up until it
/// did not, and the failure would be a corrupted line rather than a crash.
private final class LineBuffer: @unchecked Sendable {
    private var data = Data()
    private let lock = NSLock()

    func take(_ chunk: Data) -> [Data] {
        lock.lock()
        defer { lock.unlock() }
        data.append(chunk)

        var lines: [Data] = []
        while let newline = data.firstIndex(of: 0x0A) {
            let line = data[data.startIndex..<newline]
            data = data[data.index(after: newline)...]
            if !line.isEmpty { lines.append(Data(line)) }
        }
        return lines
    }
}
