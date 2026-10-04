import Foundation

/// One board's read state on a phone (ov-113): what Unread and Done read, kept
/// on the runner when the runner keeps it, and on this phone when it can't.
///
/// The rules, which the Mac's `TaskBoardStore` keeps as well:
/// - Every value only rises: a state heard late, or lower, changes nothing.
/// - A mark made while the runner keeps state is owed to it, and kept in the
///   store (not only here) until the runner answers, so a relaunch still
///   owes it. Any answer settles it, whether the runner applied it or skipped
///   a ticket that was deleted or moved. A board read never waits for it.
/// - What this phone kept before this launch goes up once per runner: its
///   marks, and never a floor. A phone's floor is a first look nobody saw as
///   Unread, and sent, it would mark everything older than a day as read on
///   every device.
/// - A phone's floor counts as its own only if somebody set it on this build
///   (a Mark All as Read that this phone kept, `floorWasSet`). One saved before
///   that, by an older build or a first look, is made up: never sent, and the
///   runner's state replaces it.
/// - A grant without Control can't write the runner's state (`mayWrite`): its
///   marks are kept here, merged by max with the runner's, and never sent.
/// - A runner that sends no state is an older build: marks go to the store,
///   times from this phone's clock, as they always did.
@MainActor
public final class BoardReadsKeeper {
    /// What's read, as this phone shows it now.
    public private(set) var reads: BoardReads {
        didSet { if reads != oldValue { onChange?(reads) } }
    }
    /// Called with the new state whenever it changes, however it did.
    public var onChange: (@MainActor (BoardReads) -> Void)?
    /// Whether the runner keeps this board's state: it sent it with the board.
    public private(set) var runnerKeepsReads = false

    private var runnerReads: BoardReads?
    /// Whether this phone's floor was somebody's doing (a Mark All as Read, or
    /// state from before floors were noted) and not the first look `load` made up.
    private var keptARealFloor: Bool {
        (beforeLaunch?.floor ?? .distantPast) > .distantPast && store.floorWasSet(host: host, workspace: workspace)
    }

    /// Whether marking something read here reaches every device: the runner
    /// keeps this board's state and this phone may write it.
    public var readsAreShared: Bool { runnerKeepsReads && mayWrite() }
    private let mayWrite: () -> Bool
    private var pending: ReadsRaise
    private let beforeLaunch: BoardReads?
    private let store: BoardReadStore
    private let host: String
    private let workspace: String
    private let send: (ReadsRaise) async -> Data?
    private var chain: Task<Void, Never>?

    /// `send` tells the runner a raise and returns its answer, nil when it
    /// didn't give one.
    public init(
        store: BoardReadStore, host: String, workspace: String, now: Date = Date(),
        mayWrite: @escaping () -> Bool = { true },
        send: @escaping (ReadsRaise) async -> Data?
    ) {
        self.mayWrite = mayWrite
        self.store = store
        self.host = host
        self.workspace = workspace
        self.send = send
        // What was kept, before `load` writes a first look of its own.
        beforeLaunch = store.keptReads(host: host, workspace: workspace)
        let loaded = store.load(host: host, workspace: workspace, now: now)
        pending = store.loadPending(host: host, workspace: workspace)
        reads = pending.applied(to: loaded)
    }

    /// A board read's `reads`, when the runner sent any: adopted, this phone's
    /// own marks sent up once, and what's owed sent behind the read. One that
    /// sends none after it did went back to an older build: this phone keeps
    /// the state again.
    public func adopt(board data: Data) {
        if let wire = WireBoardReads.decode(board: data) {
            adopt(runner: wire.reads)
            seedFromThisPhone()
            queueFlush()
        } else if runnerKeepsReads {
            runnerKeepsReads = false
            runnerReads = nil
            store.save(reads, host: host, workspace: workspace)
        }
    }

    /// Another device read something: the runner's `reads` event.
    public func heard(_ wire: WireBoardReads) {
        adopt(runner: wire.reads)
    }

    /// `row` was opened: everything on it so far is read, its notes through
    /// the newest one read (`latest`).
    public func open(_ row: TaskRow, latest: Date? = nil, now: Date = Date()) {
        var next = reads
        if runnerKeepsReads {
            next.open(row, seenThrough: latest)
        } else {
            // On this phone's clock, and only ever raising.
            var opened = reads
            opened.open(row, latest: latest, now: now)
            next = reads.merged(with: BoardReads(floor: .distantPast, opened: opened.opened.filter { $0.key == row.id }))
        }
        guard next != reads else { return }
        reads = next
        keep(ReadsRaise(opened: next.opened[row.id].map { [row.id: $0] } ?? [:]))
    }

    /// Mark All as Read: everything on `rows` so far, and the notes read.
    /// On a runner that keeps it, that clears every device.
    public func markAllRead(rows: [TaskRow], latest: Date? = nil, now: Date = Date()) {
        if runnerKeepsReads {
            reads.markAllRead(rows: rows, seenThrough: latest)
        } else {
            // On this phone's clock, and only ever raising.
            var all = reads
            all.markAllRead(rows: rows, now: now)
            reads = reads.merged(with: BoardReads(floor: all.floor))
        }
        if !readsAreShared { store.markFloorSet(host: host, workspace: workspace) }
        keep(ReadsRaise(floor: reads.floor))
    }

    /// Send the runner what it's owed, and wait for it, one send after another.
    public func flush() async { await queueFlush().value }

    private func adopt(runner state: BoardReads) {
        if !runnerKeepsReads, !keptARealFloor {
            // The floor this phone made up on its first look hid nothing
            // from the runner and must not hide what the runner shows: the
            // runner's state stands in for it, with the marks this phone has.
            reads = pending.applied(to: BoardReads(floor: .distantPast, opened: reads.opened))
        }
        runnerKeepsReads = true
        let known = runnerReads.map { $0.merged(with: state) } ?? state
        runnerReads = known
        let merged = reads.merged(with: known)
        if merged != reads { reads = merged }
    }

    /// A change to `reads`: owed to a runner that keeps it, until it answers;
    /// saved here for one that can't.
    private func keep(_ change: ReadsRaise) {
        if readsAreShared {
            pending = pending.merging(change)
            store.savePending(pending, host: host, workspace: workspace)
            queueFlush()
        } else {
            store.save(reads, host: host, workspace: workspace)
        }
    }

    /// The upgrade, once per runner: the marks this phone kept before this
    /// launch are owed to the runner, which merges by max. Nothing from a phone
    /// that kept none, and never a floor.
    private func seedFromThisPhone() {
        guard mayWrite(), !store.isUploaded(host: host, workspace: workspace) else { return }
        if let kept = beforeLaunch, !kept.opened.isEmpty {
            pending = pending.merging(ReadsRaise(opened: kept.opened))
            store.savePending(pending, host: host, workspace: workspace)
        }
        store.markUploaded(host: host, workspace: workspace)
    }

    @discardableResult
    private func queueFlush() -> Task<Void, Never> {
        let previous = chain
        let task = Task { @MainActor in
            await previous?.value
            await self.sendPending()
        }
        chain = task
        return task
    }

    private func sendPending() async {
        guard readsAreShared, !pending.isEmpty else { return }
        let sent = pending
        guard let data = await send(sent), let answer = WireBoardReads.decode(state: data) else { return }
        pending = pending.without(sent)
        store.savePending(pending, host: host, workspace: workspace)
        adopt(runner: answer.reads)
    }
}

extension ReadsRaise {
    /// `workspace.mark_read`'s arguments for a board, as the FFI takes them.
    public func rpcArguments(workspace: String) -> [String: Any] {
        var args: [String: Any] = [
            "workspace": workspace,
            "opened": opened.sorted { $0.key < $1.key }.map {
                ["task_id": $0.key, "opened_ms": BoardReads.milliseconds($0.value)] as [String: Any]
            },
        ]
        if let floor { args["floor_ms"] = BoardReads.milliseconds(floor) }
        return args
    }
}
