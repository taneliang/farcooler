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
            store: DefaultsBoardReads(), host: hostId?.uuidString ?? "runner", workspace: workspace
        ) { [weak self] raise in await self?.sendReads(raise, workspace: workspace) }
        keeper.onChange = { [weak self] reads in self?.boardReads[workspace] = reads }
        readsKeepers[workspace] = keeper
        boardReads[workspace] = keeper.reads
        return keeper
    }

    /// A board read's `reads`, adopted by the board's keeper. Only a real
    /// workspace's board carries any: an implicit one is the whole repository's.
    func adoptReads(from board: Data, workspace: String) {
        readsKeeper(for: workspace).adopt(board: board)
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
        readsKeepers[workspace]?.runnerKeepsReads ?? false
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
        guard knownBuild?.can(.boardReads) == true else { return nil }
        return try? await rpc("workspace.mark_read", raise.rpcArguments(workspace: workspace))
    }
}
