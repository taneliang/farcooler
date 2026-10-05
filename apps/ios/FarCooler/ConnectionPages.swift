import Foundation

// Orchestrator pages on a phone (ov-269 design 6.5, ov-285): each board's
// pages, as this connection last read them with `page.list`, which the client
// core answers in the shape `farcooler page list --json` prints
// (`crates/client/src/page_json.rs`). The blocks are AgentKit's, the same
// `PageView` the Mac draws.
//
// EXPERIMENTAL, behind `board_pages`, and only ever in the Plan view: nothing
// here is asked of a runner that doesn't advertise it, or before someone picks
// Plan on a board. Removing pages means deleting this file,
// `PlanOrchestratorPages.swift`, the `page` case of `PhonePlanPage` and the
// call sites that name `pages`.

/// What this connection holds of a board's pages.
enum PageListState: Equatable {
    case loaded([BoardPage])
    /// The read was refused, failed or never came back, and no list was in
    /// hand: `PageWords.couldntRead`, with Try Again.
    case unavailable

    var pages: [BoardPage] {
        if case .loaded(let pages) = self { return pages }
        return []
    }
}

/// Each board's pages, by workspace id. A class of its own, held by one
/// property on `Connection`, so a page that moves redraws only the screens
/// that show pages.
@MainActor
final class PageReads: ObservableObject {
    @Published private(set) var lists: [String: PageListState] = [:]

    func state(_ workspace: String) -> PageListState? { lists[workspace] }

    /// A board's pages, or none before a read or after one that failed.
    func pages(_ workspace: String) -> [BoardPage] { lists[workspace]?.pages ?? [] }

    fileprivate func set(_ state: PageListState, for workspace: String) {
        if lists[workspace] != state { lists[workspace] = state }
    }

    /// Forget a board's pages: its runner stopped keeping them.
    fileprivate func clear(_ workspace: String) {
        if lists[workspace] != nil { lists[workspace] = nil }
    }
}

extension Connection {
    /// Whether this runner keeps pages. A runner not read yet, or too old,
    /// offers none, and nothing is drawn for them.
    var keepsPages: Bool { knownBuild?.can(.boardPages) == true }

    /// Read `summary`'s board's pages with their documents: one call, at most
    /// 12 × 32 KiB. A read that fails over a list in hand keeps the list, as
    /// the plan's does; one that fails with nothing in hand says so.
    func readPages(_ summary: WorkspaceSummary) async {
        guard keepsPages, let board = summary.boardWorkspace else {
            pages.clear(summary.id)
            return
        }
        let data = await Self.firstAnswer(within: PlanReadState.timeout) {
            try await self.rpc("page.list", ["workspace": board])
        }
        if let data, let list = try? BoardPageList.decode(data) {
            pages.set(.loaded(list.pages), for: summary.id)
        } else if pages.state(summary.id) == nil || pages.state(summary.id) == .unavailable {
            pages.set(.unavailable, for: summary.id)
        }
    }

    /// The client core's `pages` notice: a board's page was written or
    /// removed. Boards whose pages were read read them again; the rest wait
    /// until someone picks Plan.
    func hearPages(_ notice: [String: Any]) async {
        guard let workspace = notice["workspace"] as? String else { return }
        for board in boardList
        where pages.state(board.id) != nil
            && (board.boardWorkspace?.caseInsensitiveCompare(workspace) == .orderedSame
                || board.id.caseInsensitiveCompare(workspace) == .orderedSame)
        {
            await readPages(board)
        }
    }

    /// What a page's references are drawn from on this phone: the board's
    /// cards, its plan when the runner keeps one, its pages, and the fleet's
    /// worktrees and their terminals by name. Nothing new is asked of the
    /// runner.
    func pageWorld(_ summary: WorkspaceSummary, now: Date = Date()) -> PageWorld {
        var names: [String: String] = [:]
        var terminals: Set<String> = []
        for worktree in fleet.worktrees {
            let path = worktree.worktree.map { ($0 as NSString).lastPathComponent } ?? ""
            for name in [worktree.task, worktree.branch, path] where !name.isEmpty && names[name] == nil {
                names[name] = worktree.id
            }
            for terminal in worktree.terminals {
                terminals.insert(PageWorld.terminalKey(worktree: worktree.id, name: terminal.title))
            }
        }
        return PageWorld(
            tasks: boards[summary.id]?.rows ?? [], plan: plans.state(summary.id)?.plan,
            pages: pages.pages(summary.id), worktrees: names, terminals: terminals,
            nowMs: Int64(now.timeIntervalSince1970 * 1000))
    }

    /// Where a reference on a page goes on a phone. A question still waiting
    /// opens Needs You, where it's answered (design 3.4, Q3); one answered
    /// opens its task, which `PageWorld.resolve` already chose. A web link
    /// never reaches here: `PageView` re-checks it and hands it to the
    /// system.
    func route(_ destination: PageDestination, from place: PhoneWorkspace) -> PageRouting? {
        switch destination {
        case .task(let id): return .push(.task(place, task: id))
        case .ask: return .needsYou
        case .lane(let id): return .push(.plan(place, page: .lane(id)))
        case .theme(let id): return .push(.plan(place, page: .theme(id)))
        case .page(let slot): return .push(.plan(place, page: .page(slot)))
        case .worktree(let id):
            return .push(.worktree(runner: place.runner, worktree: id, landing: .resume))
        case .terminal(let worktree, let name):
            let terminal = fleet.worktrees.first { $0.id == worktree }?.terminals.first { $0.title == name }
            return .push(
                .worktree(runner: place.runner, worktree: worktree, landing: terminal.map { .terminal($0.id) } ?? .resume))
        case .url: return nil
        }
    }

    /// `fetch`'s answer, or nil when it fails or doesn't come back within
    /// `timeout`. Not a task group, for the reason `PlanReadState.read`
    /// gives: a call into the client core can't be cancelled.
    private static func firstAnswer(
        within timeout: Duration, _ fetch: @escaping @Sendable () async throws -> Data
    ) async -> Data? {
        await withCheckedContinuation { (continuation: CheckedContinuation<Data?, Never>) in
            let once = OnceAnswer(continuation)
            let work = Task { once.resume(try? await fetch()) }
            Task {
                try? await Task.sleep(for: timeout)
                once.resume(nil)
                work.cancel()
            }
        }
    }
}

/// What a page's reference does on a phone.
enum PageRouting: Equatable {
    /// Push this screen over the one showing.
    case push(PhoneRoute)
    /// Back to Needs You, the root, where a question is answered.
    case needsYou
}

/// A continuation resumed by whichever caller gets there first.
private final class OnceAnswer: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Data?, Never>?

    init(_ continuation: CheckedContinuation<Data?, Never>) { self.continuation = continuation }

    func resume(_ value: Data?) {
        lock.lock()
        let waiting = continuation
        continuation = nil
        lock.unlock()
        waiting?.resume(returning: value)
    }
}
