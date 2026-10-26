import Foundation

// Read state on the runner (ov-113), as a connection keeps it: one
// `BoardReadsKeeper` per board, built when the board is first read, and the
// state it holds published for the screens (`boardReads`). The rules are the
// keeper's, in AgentKit, where `swift test` reads them back.

extension Connection {
    /// The keeper for `workspace`'s board, built on first use from what this
    /// phone kept for it in its defaults, under this runner's id.
    func readsKeeper(for workspace: String) -> BoardReadsKeeper {
        if let keeper = readsKeepers[workspace] { return keeper }
        let keeper = BoardReadsKeeper(
            store: DefaultsBoardReads(), host: hostId?.uuidString ?? "runner", workspace: workspace,
            mayWrite: { [weak self] in self?.mayWriteReads ?? false }
        ) { [weak self] raise in await self?.sendReads(raise, workspace: workspace) }
        keeper.onChange = { [weak self] reads in self?.boardReads[workspace] = reads }
        readsKeepers[workspace] = keeper
        boardReads[workspace] = keeper.reads
        return keeper
    }

    /// A board's JSON, decoded off the main thread (ov-791). A long-lived
    /// runner's board is about 1 MB (798 tasks with their intents, 25 Oct),
    /// read again on every task notice, which a busy runner sends every few
    /// seconds; decoded on the main thread, each read was a dropped frame
    /// in whatever the person was scrolling. Its `reads` too, which are read
    /// out of the same JSON.
    nonisolated static func decodeBoard(_ data: Data) async -> (board: TaskBoardModel, reads: WireBoardReads?)? {
        await Task.detached(priority: .userInitiated) {
            guard let board = try? TaskBoardModel.decode(data) else { return nil }
            return (board, WireBoardReads.decode(board: data))
        }.value
    }

    /// A board read's `reads`, adopted by the board's keeper. Only a real
    /// workspace's board carries any: an implicit one is the whole repository's.
    func adoptReads(_ reads: WireBoardReads?, workspace: String) {
        readsKeeper(for: workspace).adopt(boardReads: reads)
    }

    /// `row` was opened on `place`'s board: read, through the newest of its
    /// notes read (`latest`).
    func markRead(_ row: TaskRow, latest: Date?, workspace: String) {
        readsKeeper(for: workspace).open(row, latest: latest)
    }

    /// Mark All as Read, once the person said yes.
    func markAllRead(rows: [TaskRow], latest: Date?, workspace: String) {
        readsKeeper(for: workspace).markAllRead(rows: rows, latest: latest)
    }

    /// Whether marking something read here reaches every device: the runner
    /// keeps this board's state.
    func readsAreShared(workspace: String) -> Bool {
        readsKeepers[workspace]?.readsAreShared ?? false
    }

    /// Whether this phone's grant lets it write the runner's read state:
    /// `workspace.mark_read` needs Control, and a Read grant is refused.
    var mayWriteReads: Bool {
        guard let build = knownBuild else { return false }
        return build.can(.boardReads) && build.grantedScope != "read"
    }

    /// Another device read something: the runner's `reads` event, for a board
    /// this phone has read. One it hasn't gets the state with its first read.
    func hearReads(_ notice: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: notice),
            let wire = WireBoardReads.decode(state: data)
        else { return }
        let id = wire.workspaceID.lowercased()
        readsKeepers.first { $0.key.lowercased() == id }?.value.heard(wire)
    }

    /// Tell the runner what's owed (`workspace.mark_read`): its answer, or nil
    /// when it gave none, which leaves the marks owed. Never sent to a runner
    /// without `board_reads`.
    private func sendReads(_ raise: ReadsRaise, workspace: String) async -> Data? {
        guard mayWriteReads else { return nil }
        return try? await rpc("workspace.mark_read", raise.rpcArguments(workspace: workspace))
    }
}
