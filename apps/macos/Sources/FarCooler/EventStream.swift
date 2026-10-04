import AgentKit
import Foundation
import os

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
    var worktree: String
    var title: String
    var preset: String
    var state: String
    var activity: String?
    // Pushed for the same reason `preset` is: without it, a shell pane the
    // user typed `codex` into relabeled itself live from this very event,
    // but `canSwitchPaneMode` stayed false until something else forced a
    // full re-read — so `⌃B a` refused an agent this branch shipped an
    // adapter for. See `DaemonClient.apply(_:)`.
    var chatCapable: Bool?
    // Pushed for the same reason, one field later: without these, the moment
    // a live event moves a terminal to `exited` is exactly the moment its
    // `Status` cannot yet tell a shell you closed from a build that broke —
    // both read as a clean exit until a full refresh happens to backfill
    // them. See `DaemonClient.apply(_:)`.
    var exitCode: Int?
    var exitSignal: Int?
    // The daemon has sent this on every terminal event all along; it was
    // simply never decoded here, which is the same bug in a third place —
    // `statusDuration` for a live-pushed Working or Blocked row was reading
    // whatever it last got from a full refresh, not what just changed.
    var activitySince: Double?
    // Pushed for the identical reason `exitCode` is: the moment a live event
    // moves a row to Blocked is exactly the moment it needs a turn clock and
    // a question to show, not after whatever next forces a full refresh. See
    // `DaemonClient.apply(_:)`.
    var turnStartedAt: Double?
    var blockedQuestion: String?
    // Pushed for the same reason, and this is the field with the shortest
    // useful life of any of them: a line is news for as long as the agent is
    // on it. Decoded here as well as on the full read because a field carried
    // on one path and not the other is this app's oldest bug — see
    // `DaemonClient.apply(_:)`.
    var feed: [String]?
    // Where the agent is, in one line. The single most valuable string on the
    // row and the one that moves most often — a task completing moves `3/7` to
    // `4/7` while nothing else about the pane changes at all, so a row that
    // waited for a full refresh would be a progress indicator that only ever
    // updated by accident.
    var line: String?
    // The agents it spawned, named. Pushed for the identical reason: a
    // subagent that appeared on a row only at the next full refresh would be
    // finished before anyone saw it start.
    var subagents: [String]?
    // Whether the turn that just finished DIED.
    //
    // The daemon has pushed this on every terminal event since it existed and
    // this struct simply never decoded it — the same bug as `activitySince`
    // above, in a fourth place, and with the most expensive symptom of any of
    // them: a row whose agent had just died read `Done` with a clean tick
    // until something else happened in that pane, which for a dead agent is
    // indefinitely.
    var turnFailed: Bool?
    // The task this pane's own notifications fold into, as the runner decided
    // it when it built THIS event (ov-112). Applied from the push because the
    // answer belongs to the moment of the change: a Done task or a moved lane
    // since the last full read would otherwise fold a banner into a thread
    // that is not coming. See `DaemonClient.apply(_:)`.
    var noticeTaskId: String?
    // Pushed on every terminal event and applied by none until now (ov-156's
    // review): `EventResyncTests` lists every `Terminal` property against this
    // struct, so a field the wire carries and this app drops fails by name.
    var said: String?
    var paneMode: String?
    var rank: UInt32?
    var taskId: String?
    var ports: [Int]?
    var agentSessionId: String?
    var agentMode: String?
    var availableAgentModes: [String]?
    var agentFailure: String?
    var workspace: String?
    var role: String?
}

/// A worktree's tiling, pushed whole.
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
    var worktree: String
    var groups: [PaneGroup]
}

/// A board moved.
///
/// The board and not the task, because `task list` answers a whole board
/// in one call and a board is the thing on screen: a client told only which
/// task moved would still have to read the board to know where the row goes
/// now. Carries no delta for the same reason every other event here does not —
/// three editors move this state at once (this app, the CLI, and an agent
/// driving the CLI), and a client that applied deltas would have to be right
/// about all three.
///
/// `actor` is who caused it: `user`, `manager`, or `agent:<uuid>`, the word
/// the daemon's `Actor` prints and the same one stored on every note. It is
/// here so a board can tell its own write coming back from somebody else's.
/// This app deliberately re-reads either way — see `TaskBoardStore.reload` —
/// but the field has to reach Swift for that to be a decision rather than an
/// omission.
///
/// `workspace` is the board the task is on now and `fromWorkspace` the one it
/// left, set only on a move; both are null from a runner without workspaces,
/// whose one board per repository is the repository's. Which boards that
/// re-reads is AgentKit's rule, `BoardNotice.touches`, so this app and the
/// phones can't disagree about it — see `notice`.
struct TaskEvent: Sendable, Decodable {
    var repository: String
    var workspace: String?
    var fromWorkspace: String?
    var actor: String?

    init(
        repository: String, workspace: String? = nil, fromWorkspace: String? = nil,
        actor: String?
    ) {
        self.repository = repository
        self.workspace = workspace
        self.fromWorkspace = fromWorkspace
        self.actor = actor
    }

    enum CodingKeys: String, CodingKey {
        case repository, workspace, actor
        case fromWorkspace = "from_workspace"
    }

    /// This line as AgentKit's notice.
    ///
    /// Built field by field rather than by handing the line to
    /// `BoardNotice(notice:)`, which reads the client core's line: that one
    /// says `"event": "task"`, and the CLI's `events` says `"kind": "task"`,
    /// so every CLI line would read as not board news and be dropped. An
    /// empty string is absent here too, as it is there.
    var notice: BoardNotice {
        func word(_ value: String?) -> String? { value.flatMap { $0.isEmpty ? nil : $0 } }
        return BoardNotice(
            repository: repository, workspace: word(workspace),
            fromWorkspace: word(fromWorkspace), actor: word(actor))
    }
}

/// A task notice the runner composed (ov-94): `farcooler events`' `notice`
/// line. The runner decided it, worded it and named it, so the Mac posts it
/// as it arrives, under `noticeId`, and a newer notice about the same task
/// replaces the older one here exactly as the relay's push does on a phone.
/// See `Notifier.post(notice:from:)`.
struct NoticeEvent: Sendable, Decodable, Equatable {
    /// `t:<runner id>:<task key>`: the notification's identifier and thread.
    var noticeId: String
    /// `decision`, `review`, `blocked`, `done` or `new`.
    var event: String
    /// `time-sensitive`, `active` or `passive`.
    var level: String
    var title: String
    var body: String
    /// The task's key.
    var task: String
    /// The runner's `Host.runner_id`.
    var runner: String?
    /// A decision's answer options, for its buttons.
    var options: [String]
    /// The task's repository: a key is only unique within one (ov-106).
    var repository: String?

    init(
        noticeId: String, event: String, level: String, title: String, body: String, task: String,
        runner: String?, options: [String], repository: String? = nil
    ) {
        self.noticeId = noticeId
        self.event = event
        self.level = level
        self.title = title
        self.body = body
        self.task = task
        self.runner = runner
        self.options = options
        self.repository = repository
    }

    enum CodingKeys: String, CodingKey {
        case event, level, title, body, task, runner, options, repository
        case noticeId = "notice_id"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        noticeId = try c.decode(String.self, forKey: .noticeId)
        event = try c.decode(String.self, forKey: .event)
        level = try c.decodeIfPresent(String.self, forKey: .level) ?? "active"
        title = try c.decode(String.self, forKey: .title)
        body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
        task = try c.decode(String.self, forKey: .task)
        runner = try c.decodeIfPresent(String.self, forKey: .runner)
        options = try c.decodeIfPresent([String].self, forKey: .options) ?? []
        repository = try c.decodeIfPresent(String.self, forKey: .repository).flatMap { $0.isEmpty ? nil : $0 }
    }
}

/// Which resource a line is about.
private struct EventKind: Decodable {
    var kind: String
}

final class EventStream {
    private var process: Process?
    /// The pipe end `start()` attached `readabilityHandler` to, so `stop()`
    /// can clear it immediately rather than waiting for `terminationHandler`
    /// to get around to it.
    ///
    /// `stop()` used to only `terminate()` the process; Foundation keeps
    /// delivering whatever is already buffered in the pipe to the handler
    /// regardless of that, right up until the termination handler itself
    /// clears it — which runs asynchronously, after `stop()` has already
    /// returned. A line that arrived in that window was decoded and handed
    /// to `onEvent`/`onLayout`/`onFleet` for a stream its own caller just
    /// told to stop.
    private var outputHandle: FileHandle?
    private let onEvent: @Sendable (TerminalEvent) -> Void
    private let onLayout: @Sendable (LayoutEvent) -> Void
    /// The set of worktrees changed — a worktree appeared, vanished, or moved
    /// between shown and hidden. Carries nothing; the reconciler that emits it
    /// can both create and delete rows in one pass, and a client re-reads the
    /// fleet rather than applying this as a delta.
    private let onFleet: @Sendable () -> Void
    /// One worktree's diff moved: a commit landed, an agent wrote a file, a
    /// branch was checked out.
    ///
    /// Carries nothing either, for a different reason. The daemon deliberately
    /// sends the worktree and a version and never the change set itself — most
    /// clients are not showing a diff, and a lockfile regeneration would fan
    /// thousands of file records out to every connected device. So there is
    /// nothing here to apply as a delta, and the one thing worth doing with it is
    /// re-reading the counts for the whole runner in one call. Which worktree it
    /// was does not narrow that: `changes inbox` answers every row at once and is
    /// the only call the sidebar makes.
    private let onChangeSet: @Sendable () -> Void
    /// A repository's board moved. See `TaskEvent`.
    private let onTask: @Sendable (TaskEvent) -> Void
    /// The runner dropped events it owed this stream (`events_missed`): it
    /// fell more than a backlog behind, and what was lost is gone. Carries
    /// nothing, because the only answer is to re-read everything the lines
    /// above would have said — the phones' `resync`.
    private let onMissed: @Sendable () -> Void
    /// Something a person has to act on moved (`needs_you_changed`): re-read
    /// `needs-you`. Carries nothing, as `fleet` doesn't: the list is small and
    /// read whole.
    private let onNeedsYou: @Sendable () -> Void
    /// A task notice to post (`notice`, ov-94).
    private let onNotice: @Sendable (NoticeEvent) -> Void
    /// A board's read state moved on another device (`reads`, ov-113).
    private let onReads: @Sendable (WireBoardReads) -> Void
    private let onEnd: @Sendable () -> Void

    init(
        onEvent: @escaping @Sendable (TerminalEvent) -> Void,
        onLayout: @escaping @Sendable (LayoutEvent) -> Void = { _ in },
        onFleet: @escaping @Sendable () -> Void = {},
        onChangeSet: @escaping @Sendable () -> Void = {},
        onTask: @escaping @Sendable (TaskEvent) -> Void = { _ in },
        onMissed: @escaping @Sendable () -> Void = {},
        onNeedsYou: @escaping @Sendable () -> Void = {},
        onNotice: @escaping @Sendable (NoticeEvent) -> Void = { _ in },
        onReads: @escaping @Sendable (WireBoardReads) -> Void = { _ in },
        onEnd: @escaping @Sendable () -> Void = {}
    ) {
        self.onEvent = onEvent
        self.onLayout = onLayout
        self.onFleet = onFleet
        self.onChangeSet = onChangeSet
        self.onTask = onTask
        self.onMissed = onMissed
        self.onNeedsYou = onNeedsYou
        self.onNotice = onNotice
        self.onReads = onReads
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
        // Nothing reads it, and a child that fills a pipe nobody drains stops.
        p.standardError = FileHandle.nullDevice

        // Lines can arrive split across reads, so a partial one is held until
        // its newline shows up. Parsing half an object would be worse than
        // waiting for the rest of it.
        //
        // The buffer is a reference type because the read handler is called on
        // a queue Foundation owns; a captured `var` would be shared mutable
        // state across threads, which the compiler is right to refuse.
        let buffer = LineBuffer()
        let limiter = DecodeMissLimiter()
        let handle = out.fileHandleForReading
        outputHandle = handle
        handle.readabilityHandler = {
            [onEvent, onLayout, onFleet, onChangeSet, onTask, onMissed, onNeedsYou, onNotice, onReads] h in
            let chunk = h.availableData
            if chunk.isEmpty { return }
            let decoder = JSONDecoder()
            for line in buffer.take(chunk) {
                Self.dispatch(
                    line, decoder: decoder, onEvent: onEvent, onLayout: onLayout,
                    onFleet: onFleet, onChangeSet: onChangeSet, onTask: onTask,
                    onMissed: onMissed, onNeedsYou: onNeedsYou, onNotice: onNotice, onReads: onReads,
                    onUndecodable: { if limiter.allow() { onMissed() } })
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
            outputHandle = nil
            onEnd()
        }
    }

    /// One line of `farcooler events`, handed to whichever callback it is
    /// for.
    ///
    /// Static, and apart from the pipe, so the suite can feed it the lines
    /// the CLI really prints: a line this decodes into nothing is a change
    /// the window never hears about, with no error anywhere to say so.
    static func dispatch(
        _ line: Data, decoder: JSONDecoder,
        onEvent: (TerminalEvent) -> Void = { _ in },
        onLayout: (LayoutEvent) -> Void = { _ in },
        onFleet: () -> Void = {},
        onChangeSet: () -> Void = {},
        onTask: (TaskEvent) -> Void = { _ in },
        onMissed: () -> Void = {},
        onNeedsYou: () -> Void = {},
        onNotice: (NoticeEvent) -> Void = { _ in },
        onReads: (WireBoardReads) -> Void = { _ in },
        // Where a line that will not decode goes, if not to `onMissed`: the stream
        // rate-limits it (`DecodeMissLimiter`), since a full read never fixes a
        // decode failure.
        onUndecodable: (() -> Void)? = nil
    ) {
        // Dispatched on `kind` rather than by trying each shape in turn.
        // Guessing worked while there was one shape; with two, a layout
        // line that happened to decode as a terminal would have been
        // applied as one.
        func miss(_ what: String) {
            if let onUndecodable { missed(what, line, onUndecodable) } else { missed(what, line, onMissed) }
        }
        guard let kind = try? decoder.decode(EventKind.self, from: line) else {
            miss("a line with no kind")
            return
        }
        switch kind.kind {
        case "terminal":
            if let event = try? decoder.decode(TerminalEvent.self, from: line) {
                onEvent(event)
            } else {
                miss("a terminal event")
            }
        case "layout":
            if let event = try? decoder.decode(LayoutEvent.self, from: line) {
                onLayout(event)
            } else {
                miss("a layout event")
            }
        case "fleet":
            onFleet()
        case "change_set":
            onChangeSet()
        case "task":
            if let event = try? decoder.decode(TaskEvent.self, from: line) {
                onTask(event)
            } else {
                miss("a task event")
            }
        // Not a resource: news that some of the lines above never came. It
        // used to fall into `default` below, which is how a Mac that fell
        // behind went on showing boards nobody would ever tell it had moved.
        case "events_missed":
            onMissed()
        // The CLI's name for `needs_you_changed`.
        case "needs_you":
            onNeedsYou()
        // A task notice the runner composed (ov-94).
        case "notice":
            if let notice = try? decoder.decode(NoticeEvent.self, from: line) {
                onNotice(notice)
            } else {
                miss("a notice")
            }
        // A board's read state, whole, from another device (ov-113).
        case "reads":
            if let reads = try? decoder.decode(WireBoardReads.self, from: line) {
                onReads(reads)
            } else {
                miss("a board's reads")
            }
        // Resources this app does not track yet are skipped, not an error.
        default: return
        }
    }

    private static let log = Logger(subsystem: "com.farcooler.FarCooler", category: "events")

    /// A line of a kind this app tracks that did not decode.
    ///
    /// Dropping it left the window on a stale picture while it still said
    /// Connected: a field the daemon added or retyped would silence every
    /// event of that kind, with no error anywhere. It counts as an event that
    /// never arrived, so the answer is the same one `events_missed` gets: read
    /// everything again.
    private static func missed(_ what: String, _ line: Data, _ onMissed: () -> Void) {
        // Names the kind and the size, never the line: it can carry an agent's
        // words.
        log.error("Couldn't decode \(what, privacy: .public) of \(line.count, privacy: .public) bytes; re-reading")
        onMissed()
    }

    func stop() {
        // Cleared here, synchronously, rather than left for
        // `terminationHandler` to get to — see `outputHandle`'s own doc
        // comment for why waiting is the bug.
        outputHandle?.readabilityHandler = nil
        outputHandle = nil
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
final class LineBuffer: @unchecked Sendable {
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


/// How often undecodable lines may ask for a full read (ov-156).
///
/// A runner whose terminal events this build can't decode (a retyped field)
/// sends several a second during a turn, and a full read doesn't fix them: the
/// first one is worth a read, in case the line was a one-off, and the rest are
/// the same news. So the first is let through at once, and then each wait
/// doubles from `first` to `ceiling`, forgotten after `quiet` without a miss.
/// A real `events_missed` line is not subject to this.
final class DecodeMissLimiter: @unchecked Sendable {
    private let lock = NSLock()
    private let first: TimeInterval, ceiling: TimeInterval, quiet: TimeInterval
    private var lastAllowed: Date?
    private var lastMiss: Date?
    private var wait: TimeInterval

    init(first: TimeInterval = 30, ceiling: TimeInterval = 600, quiet: TimeInterval = 1800) {
        self.first = first
        self.ceiling = ceiling
        self.quiet = quiet
        wait = first
    }

    /// Whether this miss, at `now`, may start a read.
    func allow(at now: Date = Date()) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        defer { lastMiss = now }
        if let lastMiss, now.timeIntervalSince(lastMiss) > quiet { lastAllowed = nil; wait = first }
        if let lastAllowed, now.timeIntervalSince(lastAllowed) < wait { return false }
        if lastAllowed != nil { wait = min(wait * 2, ceiling) }
        lastAllowed = now
        return true
    }
}
