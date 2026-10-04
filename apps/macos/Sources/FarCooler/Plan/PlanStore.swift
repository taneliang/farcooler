import AgentKit
import Combine
import Foundation

// The plan layer on the Mac (ov-268, phase P4 is ov-273): the Plan view, a
// theme's page and a lane's page, behind a Tasks | Plan control on the board.
//
// EXPERIMENTAL and opt-in. Tasks is the default, and the control only shows
// on a runner that advertises `board_plan`; until someone picks Plan, the
// board draws exactly what it drew before. Removing the layer means deleting
// this folder, the `plan` case of `WorkspaceSelection.Focus` and the call
// sites that name `PlanStore` (see the design's section 8).
//
// The rules are AgentKit's `PlanModel`; this reads it and remembers the
// choice.

/// A page the Plan view opens in the main area, where a task opens.
enum PlanPage: Hashable {
    /// A theme, by id.
    case theme(String)
    /// A lane, by id.
    case lane(String)

    /// What it is, for a menu with no name to give it.
    var word: String {
        switch self {
        case .theme: "Theme"
        case .lane: "Lane"
        }
    }

    /// Its symbol in a menu.
    var symbol: String {
        switch self {
        case .theme: "map"
        case .lane: "arrow.triangle.branch"
        }
    }
}

/// One board's plan, as this window last read it, and whether the board
/// shows it.
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
    /// Whether the board shows the plan rather than its tasks. Kept per
    /// board on this Mac, as the Unread strip's collapsed state is.
    @Published var shown: Bool {
        didSet {
            guard shown != oldValue else { return }
            defaults.set(shown, forKey: Self.shownKey(host: host, workspace: workspace.id))
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
        shown = defaults.bool(forKey: Self.shownKey(host: host, workspace: workspace.id))
    }

    static func shownKey(host: String, workspace: String) -> String { "board.plan.shown.\(host).\(workspace)" }

    /// Whether this runner keeps a plan: only then is there a control. A
    /// runner not read yet, or too old, offers nothing new.
    var available: Bool { client.daemonBuild?.can(.boardPlan) == true }

    /// What the board draws: the plan, only where the runner keeps one and
    /// someone chose it.
    var showing: Bool { available && shown }

    private var repositoryID: String { workspace.repository ?? workspace.id }

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
        hasRead = true
    }

    /// Read a theme's or lane's record, for its page's timeline and What
    /// Changed.
    func readRecord(_ page: PlanPage) async {
        let data: Data?
        switch page {
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
