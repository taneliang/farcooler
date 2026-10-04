import AgentKit
import Foundation

// Read state kept on the runner (ov-113): what a board does with it. The state
// itself, and the stored properties these use, are in TaskBoard.swift.

extension TaskBoardStore {
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
        keepReads()
    }

    /// Where a change to `reads` goes: to the runner, which keeps it for
    /// every device (ov-113), or for a runner that can't, this Mac's defaults.
    private func keepReads() {
        if runnerKeepsReads {
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
        keepReads()
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

    /// Send the runner what it hasn't got: at the upgrade, this Mac's floor
    /// and marks from before (once, recorded by `readStore.markUploaded`), and after
    /// that any mark a send missed. One after another, and each works out what
    /// is unsent when its turn comes, so a repeat sends nothing.
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
            await self.sendUnsentReads()
        }
        flushChain = task
        return task
    }

    func sendUnsentReads() async {
        guard runnerKeepsReads, let board = workspace.boardWorkspace else { return }
        let runner = runnerReads ?? BoardReads(floor: .distantPast)
        guard let raise = reads.raising(over: runner) else {
            // Nothing this Mac holds is news to the runner.
            readStore.markUploaded(host: hostKey, workspace: workspace.id)
            return
        }
        guard
            let data = await client.markBoardRead(repository: repositoryID, workspace: board, raise: raise),
            let answer = WireBoardReads.decode(state: data)
        else { return }  // stays unsent, and goes with the next flush
        adopt(runner: answer.reads)
        readStore.markUploaded(host: hostKey, workspace: workspace.id)
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
