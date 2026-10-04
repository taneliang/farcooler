import AgentKit
import SwiftUI

/// The window's side of the history list (ov-248): what a long press on Back
/// or Forward, and Workspace ▸ History, list, and the move a chosen row
/// makes.
extension ContentView {
    /// The list: where the window has been and is going, named from what the
    /// window has read. A place whose runner hasn't answered keeps the
    /// kind's own word, and one known to be gone isn't listed.
    var historyRows: [PlaceRow] {
        HistoryMenu.rows(
            jumpBar.history.rows(current: selection, trail: openedFrom(selection), resolves: resolves),
            names: HistoryMenu.Names(
                workspace: { host, id in
                    board(host: host, id: id).map { w in
                        w.isImplicit
                            ? store.clients[host]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName ?? w.name
                            : w.name
                    }
                },
                task: { host, workspace, id in
                    boardStores["\(host)/\(workspace)"]?.board.columns.flatMap(\.rows).first { $0.id == id }
                        .map { "\($0.key) \($0.title)" }
                },
                worktree: { host, id in worktree(host: host, id: id)?.task }),
            showHosts: showHosts)
    }

    /// A row of the list chosen: there in one move, the stops between kept
    /// on the other side.
    func go(to spot: NavigationHistory.Spot) {
        if let stop = jumpBar.history.go(to: spot, from: selection, trail: openedFrom(selection)) { land(stop) }
    }
}
