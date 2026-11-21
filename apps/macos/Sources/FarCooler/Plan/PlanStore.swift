import AgentKit
import Combine
import Foundation

// The plan layer on the Mac (ov-268, phase P4 is ov-273): the plan's
// sections, a theme's page and a ticket's page.
//
// The primary way of working since ov-298 (the owner, Oct 4: "Let's make that
// the primary way of working"): there's no Tasks | Plan control. A board whose
// runner keeps a plan, once it has one, lists its themes beside the
// task index (`planned`); a runner without `board_plan`, or a board with no
// plan yet, draws the board as it always did.
//
// The rules are AgentKit's `PlanModel`; this reads it.

/// A page the Plan view opens in the main area, where a task opens.
enum PlanPage: Hashable {
    /// A theme, by id.
    case theme(String)
    /// A ticket, by task id.
    case ticket(String)
    /// The workspace's Needs You, answered in place (ov-321): where the one
    /// tree's Needs You row opens.
    case needsYou
    /// A pipeline stage's tickets nobody is on (ov-874), by `WirePipeStage`
    /// raw value: where the column's "N more" line opens.
    case stage(String)
    case engine(EngineFocus?)

    /// What it is, for a menu with no name to give it.
    var word: String {
        switch self {
        case .theme: "Theme"
        case .ticket: "Ticket"
        case .needsYou: "Needs You"
        case .stage: "Stage"
        case .engine: EngineCopy.engine.text
        }
    }

    /// Its symbol in a menu.
    var symbol: String {
        switch self {
        case .theme: "map"
        case .ticket: "arrow.triangle.branch"
        case .needsYou: "flag"
        case .stage: "list.bullet"
        case .engine: "gearshape.2"
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
    /// Whether the last read got no answer, as it doesn't while the runner
    /// restarts or the link is down (ov-631). Not a broken plan: the last
    /// plan stays on screen, and the read goes round again by itself.
    @Published private(set) var reconnecting = false
    /// Each theme's and ticket's record, by page, once read.
    @Published private(set) var records: [PlanPage: PlanRecord] = [:]
    /// The one tree last built for each filter, and what from (ov-321
    /// review H2): the sidebar, the jump bar and ⌘↑ share it, and it's
    /// built again only when its input changes.
    private var trees: [OneTreeFilter: (input: OneTreeInput, tree: OneTree)] = [:]
    /// The same for the sidebar's two lists (ov-503).
    private var outlines: [OneTreeFilter: (input: OneTreeInput, outline: SidebarOutline)] = [:]

    /// Show `ids` kept now (ov-333), before the runner's answer: Keep is the
    /// owner's own mark and should land like read state does. The read that
    /// follows makes it the runner's word again, or puts it back if the
    /// runner refused.
    func showKept(_ ids: Set<String>) {
        var next = plan
        for index in next.rulings.indices where ids.contains(next.rulings[index].id) && next.rulings[index].isStanding {
            next.rulings[index].state = .confirmed
            next.rulings[index].settledBy = "user"
            next.rulings[index].settledAt = next.nowMs
        }
        if next != plan { plan = next }
    }

    /// The tree for `input`, from the cache when nothing it reads moved.
    func oneTree(_ input: OneTreeInput) -> OneTree {
        if let kept = trees[input.filter], kept.input == input { return kept.tree }
        let tree = OneTree.build(input)
        trees[input.filter] = (input, tree)
        return tree
    }

    /// The sidebar's Themes and Tickets lists for `input`, from the cache when
    /// nothing they read moved.
    func sidebarOutline(_ input: OneTreeInput) -> SidebarOutline {
        if let kept = outlines[input.filter], kept.input == input { return kept.outline }
        let outline = SidebarOutline.build(input)
        outlines[input.filter] = (input, outline)
        return outline
    }
    let client: DaemonClient
    let workspace: WorkspaceSummary
    let host: String
    private var seenGeneration = -1
    private var readAgain = false
    private var retry: Task<Void, Never>?
    /// How long to wait before asking again after no answer; doubles to
    /// `retryCeiling`. A test shortens it.
    var retryDelay: Duration = .seconds(2)
    var retryCeiling: Duration = .seconds(30)
    private var retries = 0
    /// How long no answer is "the runner is reconnecting" before it is "couldn't
    /// read this plan", with Try Again (ov-631): a runner that restarts is back
    /// within a minute; one that isn't has something else wrong.
    var giveUpAfter: Duration = .seconds(120)
    private var noAnswerSince: ContinuousClock.Instant?
    /// Whether the daemon link is down, which is the only no-answer that says
    /// "reconnecting": a missing CLI, a command that exited non-zero and a launch
    /// failure are not the runner coming back. A test sets it.
    lazy var linkIsDown: () -> Bool = { [weak client] in
        switch client?.state {
        case .connecting?, .reconnecting?: true
        case .unreachable?: client?.isCLIMissing != true
        default: false
        }
    }
    /// Whether the runner has no Far Cooler to ask. A test sets it.
    lazy var cliIsMissing: () -> Bool = { [weak client] in client?.isCLIMissing ?? false }
    /// A plan already read, and now the runner can't be asked: the quiet line
    /// over it, with Try Again (ov-662).
    @Published private(set) var staleLine: String?
    /// How many views show this plan. When the last goes away the retry stops;
    /// the next to appear starts it again if the plan is still unread.
    private var viewers = 0
    private var detached = false
    /// Where a plan that didn't decode says which field failed (a path, never
    /// a value). A test listens.
    var onDecodeFailure: (String) -> Void = PlanDecodeLog.log

    init(client: DaemonClient, workspace: WorkspaceSummary, host: String) {
        self.client = client
        self.workspace = workspace
        self.host = host
    }

    /// Whether this runner keeps a plan. A runner not read yet, or too old,
    /// offers nothing new.
    var available: Bool { client.daemonBuild?.can(.boardPlan) == true }

    /// Whether the board is drawn around its plan (ov-298): a runner that
    /// keeps one, a plan read, and something in it, a theme, a ticket or a
    /// page. Until then, and on a board with nothing planned, the navigator
    /// is the task list it always was.
    var planned: Bool { available && hasRead && !plan.isEmpty }

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
        // The link coming back is news: a fresh wait.
        noAnswerSince = nil
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
        guard let data else {
            // No answer. Keep the plan in hand and ask again for a while. It
            // is "reconnecting" only while the daemon link is down and the wait
            // is short; any other failure, and a wait that has gone on, is a
            // plan that couldn't be read, with Try Again.
            // A missing CLI is not coming back by waiting: say so at once.
            if cliIsMissing() {
                reconnecting = false
                trouble = PlanWords.notInstalled
                staleLine = hasRead ? PlanWords.staleNotInstalled : nil
                askAgainSoon()
                return
            }
            let started = noAnswerSince ?? .now
            noAnswerSince = started
            if started.duration(to: .now) >= giveUpAfter {
                reconnecting = false
                trouble = PlanWords.couldntRead
                staleLine = hasRead ? PlanWords.staleUnreachable : nil
                retry?.cancel()
                return
            }
            if linkIsDown() {
                reconnecting = true
                trouble = nil
                staleLine = nil
            } else {
                reconnecting = false
                trouble = PlanWords.couldntRead
                staleLine = hasRead ? PlanWords.staleUnreachable : nil
            }
            askAgainSoon()
            return
        }
        retry?.cancel()
        retries = 0
        noAnswerSince = nil
        reconnecting = false
        staleLine = nil
        let read: PlanModel
        do {
            read = try PlanModel.decode(data)
        } catch {
            onDecodeFailure(PlanDecodeLog.fieldPath(error))
            trouble = PlanWords.couldntRead
            return
        }
        trouble = nil
        if read != plan { plan = read }
        if Task.isCancelled { return }
        hasRead = true
    }

    /// Ask again after a pause, longer each time, until a read gets an answer.
    /// The link coming back moves `generation` too; this is for the read that
    /// lost the race with a daemon still starting, which nothing else moves.
    private func askAgainSoon() {
        retry?.cancel()
        guard !detached else { return }
        let delay = min(retryDelay * (1 << min(retries, 4)), retryCeiling)
        retries += 1
        retry = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            self?.retry = nil
            await self?.reload()
        }
    }

    /// A view that shows this plan appeared. If the plan was last read with no
    /// answer and nothing is asking again, ask now.
    func attach() {
        viewers += 1
        detached = false
        if (reconnecting || trouble != nil) && retry == nil && !reading { Task { await reload() } }
    }

    /// A view that showed this plan went away. With none left, stop asking
    /// again: nobody is looking at the answer.
    func detach() {
        viewers = max(0, viewers - 1)
        guard viewers == 0 else { return }
        detached = true
        retry?.cancel()
        retry = nil
    }

    /// Try Again: a fresh wait, not what was left of the last one.
    func tryAgain() async {
        noAnswerSince = nil
        retries = 0
        await reload()
    }

    /// Read a theme's or ticket's record, for its page's timeline and What
    /// Changed.
    func readRecord(_ page: PlanPage) async {
        let data: Data?
        switch page {
        case .needsYou, .stage, .engine:
            return
        case .theme(let id):
            guard let theme = plan.themes.first(where: { $0.id == id }) else { return }
            data = await client.planRecord(of: ["theme", "show", theme.short], repository: repositoryID, workspace: workspace.boardWorkspace).data
        case .ticket(let id):
            guard let line = plan.lines.first(where: { $0.id == id }) else { return }
            data = await client.planRecord(of: ["ticket", "show", line.key], repository: repositoryID, workspace: workspace.boardWorkspace).data
        }
        guard let data, let record = try? PlanRecord.decode(data) else { return }
        if records[page] != record { records[page] = record }
    }

    func theme(_ id: String) -> PlanTheme? { plan.themes.first { $0.id == id } }
    func ticket(_ id: String) -> PlanTicketLine? { plan.lines.first { $0.id == id } }

    /// What a page's jump bar and the window's title call it.
    func title(_ page: PlanPage) -> String? {
        switch page {
        case .theme(let id): theme(id)?.name
        case .ticket(let id): ticket(id)?.heading
        case .needsYou: OneTreeWords.needsYou
        case .stage(let raw): WirePipeStage(rawValue: raw)?.title
        case .engine: page.word
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

    /// A theme's or ticket's page: `plan theme show` or `plan ticket show`, with
    /// its record.
    func planRecord(of verb: [String], repository: String, workspace: String?) async -> (data: Data?, message: String?) {
        await runRaw(
            ["plan"] + verb + ["--repo", repository] + (workspace.map { ["--workspace", $0] } ?? []) + ["--json"],
            background: true)
    }
}
