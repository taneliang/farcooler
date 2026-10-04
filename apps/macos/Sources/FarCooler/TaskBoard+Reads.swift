import AgentKit
import Foundation

// Read state kept on the runner (ov-113): what a board does with it. The state
// itself, and the stored properties these use, are in TaskBoard.swift.

extension TaskBoardStore {
    /// What a board starts with: what this Mac kept (`before`, nil when it
    /// kept nothing and `load` made up a first look), with what was marked
    /// and not yet told to the runner counted as read.
    static func startingReads(_ store: BoardReadStore, host: String, workspace: String)
        -> (reads: BoardReads, pending: ReadsRaise, before: BoardReads?)
    {
        let kept = store.hasState(host: host, workspace: workspace)
        let loaded = store.load(host: host, workspace: workspace, now: Date())
        let pending = store.loadPending(host: host, workspace: workspace)
        return (pending.applied(to: loaded), pending, kept ? loaded : nil)
    }

    /// A board read's `reads`, if the runner sent any: adopted, this Mac's
    /// own sent up once, and what is owed sent behind the read (never waited
    /// for, so a slow send can't hold the board). If it sent none for a board
    /// it used to, the runner went back to an older build: this Mac keeps it.
    func adoptReads(from board: Data) {
        if let wire = WireBoardReads.decode(board: board) {
            adopt(runner: wire.reads)
            seedFromThisMac()
            queueFlush()
        } else if workspace.boardWorkspace != nil, runnerKeepsReads {
            runnerKeepsReads = false
            runnerReads = nil
            readStore.save(reads, host: hostKey, workspace: workspace.id)
        }
    }

    /// `row` was opened: everything on it so far is read, its notes up to
    /// the newest one read (`latest`).
    ///
    /// On a runner that keeps read state, through what the runner has told
    /// this Mac and no clock of this Mac's own, and sent to it; otherwise
    /// on this Mac, as it always was.
    func markRead(_ row: TaskRow, latest: Date? = nil, now: Date = Date()) {
        var next = reads
        if runnerKeepsReads {
            next.open(row, seenThrough: latest)
        } else {
            next.open(row, latest: latest, now: now)
        }
        guard next != reads else { return }
        reads = next
        keepReads(ReadsRaise(opened: next.opened[row.id].map { [row.id: $0] } ?? [:]))
    }

    /// Where a change to `reads` goes: for a runner that keeps it, owed to
    /// the runner until it answers (`pendingReads`, kept in defaults so a
    /// relaunch still owes it); for one that can't, this Mac's defaults.
    private func keepReads(_ change: ReadsRaise) {
        if runnerKeepsReads {
            pendingReads = pendingReads.merging(change)
            readStore.savePending(pendingReads, host: hostKey, workspace: workspace.id)
            queueFlush()
        } else {
            readStore.save(reads, host: hostKey, workspace: workspace.id)
        }
    }

    /// Mark All as Read: everything on the board so far, once the person
    /// said yes (`MarkReadConfirmation`, ov-210).
    func markAllRead(_ granted: MarkReadGrant, now: Date = Date()) {
        if runnerKeepsReads {
            // Through the runner's own last words on the rows and the notes
            // read for Unread. It clears every device, not only this one.
            reads.markAllRead(
                rows: board.rows, seenThrough: summaryNotes.values.flatMap { $0 }.map(\.at).max())
        } else {
            reads.markAllRead(rows: board.rows, now: now)
        }
        keepReads(ReadsRaise(floor: reads.floor))
    }

    /// What the runner said about this board's read state: merged into
    /// `reads`, which can only rise.
    func adopt(runner state: BoardReads) {
        runnerKeepsReads = true
        let known = runnerReads.map { $0.merged(with: state) } ?? state
        runnerReads = known
        let merged = reads.merged(with: known)
        if merged != reads { reads = merged }
    }

    /// The upgrade, once per board: what this Mac kept before this launch
    /// (its floor and marks) is owed to the runner, which merges by max. A
    /// Mac that kept nothing sends nothing, and never a first look it made
    /// up, which nobody saw as Unread.
    func seedFromThisMac() {
        guard !readStore.isUploaded(host: hostKey, workspace: workspace.id) else { return }
        if let kept = readsBeforeLaunch {
            pendingReads = pendingReads.merging(ReadsRaise(floor: kept.floor, opened: kept.opened))
            readStore.savePending(pendingReads, host: hostKey, workspace: workspace.id)
        }
        readStore.markUploaded(host: hostKey, workspace: workspace.id)
    }

    /// Send the runner what it is owed, and wait for it. One after another.
    func flushReads() async {
        await queueFlush().value
    }

    /// `flushReads`, without waiting for it: put in line at once, so the
    /// next one finds this one to wait for.
    @discardableResult
    func queueFlush() -> Task<Void, Never> {
        let previous = flushChain
        let task = Task { @MainActor in
            await previous?.value
            await self.sendPendingReads()
        }
        flushChain = task
        return task
    }

    /// Tell the runner `pendingReads`. Once it has answered, those are done,
    /// whether it applied them or skipped a ticket that was deleted or moved:
    /// resending a skipped one would never end. No answer keeps them owed.
    func sendPendingReads() async {
        guard runnerKeepsReads, let board = workspace.boardWorkspace, !pendingReads.isEmpty else { return }
        let sent = pendingReads
        guard
            let data = await client.markBoardRead(repository: repositoryID, workspace: board, raise: sent),
            let answer = WireBoardReads.decode(state: data)
        else { return }
        pendingReads = pendingReads.without(sent)
        readStore.savePending(pendingReads, host: hostKey, workspace: workspace.id)
        adopt(runner: answer.reads)
    }

    /// The runner pushed this board's state: another device read something.
    func heard(_ all: [String: WireBoardReads]) {
        guard !workspace.isImplicit, let state = all[workspace.id.lowercased()] else { return }
        adopt(runner: state.reads)
    }
}

extension DaemonClient {
    /// A device marked something read: keep what the runner said, merged with
    /// what it said before, since each state only rises.
    func readsChanged(_ reads: WireBoardReads) {
        let id = reads.workspaceID.lowercased()
        let before = heardReads[id]?.reads
        let merged = before.map { $0.merged(with: reads.reads) } ?? reads.reads
        // A new value each time, so a board hears every one.
        heardReads[id] = WireBoardReads(
            workspaceID: id, floorMs: BoardReads.milliseconds(merged.floor),
            opened: merged.opened.map { .init(taskID: $0.key, openedMs: BoardReads.milliseconds($0.value)) })
    }

    /// Raise what is read on a workspace's board, on the runner
    /// (`board mark-read`): the runner's state after the merge, or nil when
    /// it didn't answer, which leaves the mark to be sent again.
    func markBoardRead(repository: String, workspace: String, raise: ReadsRaise) async -> Data? {
        await runRaw(
            ["board", "mark-read", "--repo", repository, "--workspace", workspace] + raise.arguments
                + ["--json"],
            background: true
        ).data
    }
}
