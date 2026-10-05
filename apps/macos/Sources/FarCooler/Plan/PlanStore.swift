import AgentKit
import Combine
import Foundation

// The plan layer on the Mac (ov-268, phase P4 is ov-273): the plan's
// sections, a theme's page and a lane's page.
//
// The primary way of working since ov-298 (the owner, Oct 4: "Let's make that
// the primary way of working"): there's no Tasks | Plan control. A board whose
// runner keeps a plan, once it has one, lists its themes and pages beside the
// task index (`planned`); a runner without `board_plan`, or a board with no
// plan yet, draws the board as it always did.
//
// The rules are AgentKit's `PlanModel`; this reads it.

/// A page the Plan view opens in the main area, where a task opens.
enum PlanPage: Hashable {
    /// A theme, by id.
    case theme(String)
    /// A lane, by id.
    case lane(String)
    /// An orchestrator's page, by slot (ov-284).
    case page(String)
    /// The workspace's Needs You, answered in place (ov-321): where the one
    /// tree's Needs You row opens.
    case needsYou

    /// What it is, for a menu with no name to give it.
    var word: String {
        switch self {
        case .theme: "Theme"
        case .lane: "Lane"
        case .page: "Page"
        case .needsYou: "Needs You"
        }
    }

    /// Its symbol in a menu.
    var symbol: String {
        switch self {
        case .theme: "map"
        case .lane: "arrow.triangle.branch"
        case .page: "doc.text"
        case .needsYou: "flag"
        }
    }
}

/// One board's plan, as this window last read it.
///
/// One per `TaskBoardStore` (`TaskBoardStore.plan`), so it lives and dies
/// with the board's: a store held over from a dropped connection would go on
/// asking a runner nobody is answering.
@MainActor
final class PlanStore: ObservableObject {
    @Published private(set) var plan: PlanModel = .empty
    @Published private(set) var hasRead = false
    @Published private(set) var reading = false
    /// This app's sentence for a read that didn't come back, never the
    /// CLI's stderr.
    @Published private(set) var trouble: String?
    /// Each theme's and lane's record, by page, once read.
    @Published private(set) var records: [PlanPage: PlanRecord] = [:]
    /// The one tree last built for each filter, and what from (ov-321
    /// review H2): the sidebar, the jump bar and ⌘↑ share it, and it's
    /// built again only when its input changes.
    private var trees: [OneTreeFilter: (input: OneTreeInput, tree: OneTree)] = [:]

    /// The tree for `input`, from the cache when nothing it reads moved.
    func oneTree(_ input: OneTreeInput) -> OneTree {
        if let kept = trees[input.filter], kept.input == input { return kept.tree }
        let tree = OneTree.build(input)
        trees[input.filter] = (input, tree)
        return tree
    }
    /// The board's orchestrator pages, with their documents, once read
    /// (ov-284). Empty on a runner without `board_pages`.
    @Published var pages: [BoardPage] = []
    /// Whether the pages have been read at least once.
    @Published var pagesRead = false
    /// This app's sentence for a pages read that didn't come back.
    @Published var pagesTrouble: String?
    /// The slots hidden on this Mac with Hide Page: the owner's own filter,
    /// never sent to the runner. Kept per board, as `shown` is.
    @Published var hiddenPages: Set<String> {
        didSet {
            guard hiddenPages != oldValue else { return }
            defaults.set(hiddenPages.sorted(), forKey: Self.hiddenKey(host: host, workspace: workspace.id))
        }
    }

    let client: DaemonClient
    let workspace: WorkspaceSummary
    let host: String
    private let defaults: UserDefaults
    private var seenGeneration = -1
    private var readAgain = false

    init(client: DaemonClient, workspace: WorkspaceSummary, host: String, defaults: UserDefaults = .standard) {
        self.client = client
        self.workspace = workspace
        self.host = host
        self.defaults = defaults
        hiddenPages = Set(defaults.stringArray(forKey: Self.hiddenKey(host: host, workspace: workspace.id)) ?? [])
    }

    /// Whether this runner keeps a plan. A runner not read yet, or too old,
    /// offers nothing new.
    var available: Bool { client.daemonBuild?.can(.boardPlan) == true }

    /// Whether the board is drawn around its plan (ov-298): a runner that
    /// keeps one, a plan read, and something in it, a theme, a lane or a
    /// page. Until then, and on a board with nothing planned, the navigator
    /// is the task list it always was.
    var planned: Bool { available && hasRead && (!plan.isEmpty || !pages.isEmpty) }

    var repositoryID: String { workspace.repository ?? workspace.id }

    /// A number that moves whenever this plan may have: a plan event about
    /// this board, or anything that moves the board's tasks, whose statuses
    /// the plan counts.
    var generation: Int { (client.planNews[workspace.id] ?? 0) + client.boardGeneration(for: workspace) }

    /// Read the plan once, however many views ask.
    func readIfNeverRead() async {
        guard !hasRead, !reading else { return }
        await reload()
    }

    /// Re-read if the runner said something moved since the last read.
    func reloadIfMoved() async {
        guard generation != seenGeneration else { return }
        seenGeneration = generation
        await reload()
        // An open page's record moves with the plan.
        for page in records.keys { await readRecord(page) }
    }

    /// Re-read the whole plan: one call, one at a time, the last asked for
    /// read last, as the board's `reload` does.
    func reload() async {
        if reading {
            readAgain = true
            return
        }
        reading = true
        defer { reading = false }
        repeat {
            readAgain = false
            await readOnce()
        } while readAgain
    }

    private func readOnce() async {
        let (data, _) = await client.planRead(repository: repositoryID, workspace: workspace.boardWorkspace)
        guard let data, let read = try? PlanModel.decode(data) else {
            trouble = PlanWords.couldntRead
            return
        }
        trouble = nil
        if read != plan { plan = read }
        await readPages()
        // A view that went away mid-read cancels it: not read, so the next
        // view to ask reads again rather than drawing no pages until the
        // runner says something moved.
        if Task.isCancelled { return }
        hasRead = true
    }

    /// Read a theme's or lane's record, for its page's timeline and What
    /// Changed.
    func readRecord(_ page: PlanPage) async {
        let data: Data?
        switch page {
        case .page, .needsYou:
            // A page has no record: it's replaced whole.
            return
        case .theme(let id):
            guard let theme = plan.themes.first(where: { $0.id == id }) else { return }
            data = await client.planRecord(of: ["theme", "show", theme.short], repository: repositoryID, workspace: workspace.boardWorkspace).data
        case .lane(let id):
            guard let lane = plan.lanes.first(where: { $0.id == id }) else { return }
            data = await client.planRecord(of: ["lane", "show", lane.name], repository: repositoryID, workspace: workspace.boardWorkspace).data
        }
        guard let data, let record = try? PlanRecord.decode(data) else { return }
        if records[page] != record { records[page] = record }
    }

    func theme(_ id: String) -> PlanTheme? { plan.themes.first { $0.id == id } }
    func lane(_ id: String) -> PlanLane? { plan.lanes.first { $0.id == id } }

    /// What a page's jump bar and the window's title call it.
    func title(_ page: PlanPage) -> String? {
        switch page {
        case .theme(let id): theme(id)?.name
        case .lane(let id): lane(id)?.name
        case .page(let slot): self.page(slot)?.title
        case .needsYou: OneTreeWords.needsYou
        }
    }
}

extension DaemonClient {
    /// One board's plan: `farcooler plan --json`, the shape AgentKit's
    /// `PlanModel` decodes. In the background, as the board's read is.
    func planRead(repository: String, workspace: String?) async -> (data: Data?, message: String?) {
        await runRaw(
            ["plan", "--repo", repository] + (workspace.map { ["--workspace", $0] } ?? []) + ["--json"],
            background: true)
    }

    /// A theme's or lane's page: `plan theme show` or `plan lane show`, with
    /// its record.
    func planRecord(of verb: [String], repository: String, workspace: String?) async -> (data: Data?, message: String?) {
        await runRaw(
            ["plan"] + verb + ["--repo", repository] + (workspace.map { ["--workspace", $0] } ?? []) + ["--json"],
            background: true)
    }
}
