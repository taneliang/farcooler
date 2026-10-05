import AgentKit
import Foundation

// Task keys in the window's text as links (ov-196): which keys a runner's text
// may link, from the boards this window has read. The rule for which words are
// keys is AgentKit's `TaskKeyLinks`; this is only where the Mac's boards are.

extension TaskKeyIndex {
    /// `host`'s keys: its workspaces' prefixes and the tasks on the boards
    /// this window has read for it, the palette's tasks (`paletteTasks`).
    @MainActor
    static func mac(host: String, workspaces: [WorkspaceSummary], stores: some Sequence<TaskBoardStore>)
        -> TaskKeyIndex
    {
        var boards: [String: TaskBoardModel] = [:]
        for store in stores where store.client.target == host {
            boards[store.workspace.id] = store.board
        }
        return TaskKeyIndex(runner: host, workspaces: workspaces, boards: boards)
    }
}

extension TaskKeyLinker {
    /// The window's linker for `host`, each link opening its task through
    /// `openTask` (`ContentView.openTask(_:host:workspace:)`, the palette's
    /// way). Here rather than inline in `ContentView` so a test holds which
    /// of the target's ids goes where.
    ///
    /// Each key's card (ov-299) comes from `cards`, which builds them once
    /// per change in the boards, plans and records this window has read.
    @MainActor
    static func mac(
        host: String, workspaces: [WorkspaceSummary], stores: some Sequence<TaskBoardStore>,
        cards: TaskKeyCardCache? = nil,
        openTask: @escaping @MainActor (_ task: String, _ host: String, _ workspace: String) -> Void
    ) -> TaskKeyLinker {
        let stores = stores.filter { $0.client.target == host }
        return TaskKeyLinker(
            index: .mac(host: host, workspaces: workspaces, stores: stores),
            cards: cards.map { TaskKeyCards.mac(host: host, stores: stores, cache: $0) } ?? .empty
        ) {
            openTask($0.task, $0.runner, $0.workspace)
        }
    }
}

extension TaskKeyCards {
    /// `host`'s cards, from the boards this window has read for it, each
    /// board's plan once read, and the records of the cards opened.
    @MainActor
    static func mac(host: String, stores: some Sequence<TaskBoardStore>, cache: TaskKeyCardCache) -> TaskKeyCards {
        var boards: [String: TaskBoardModel] = [:]
        var plans: [String: PlanModel] = [:]
        var notes: [String: [TaskNoteRow]] = [:]
        for store in stores where store.client.target == host {
            boards[store.workspace.id] = store.board
            if store.plan.hasRead { plans[store.workspace.id] = store.plan.plan }
            notes.merge(store.readNotes) { first, _ in first }
        }
        return cache.cards(runner: host, boards: boards, plans: plans, notes: notes)
    }
}
