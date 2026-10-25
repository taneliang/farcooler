import Foundation

/// Whom this phone talks to in each workspace of a runner (ov-526): the
/// orchestrator, or, for a member who doesn't lead the workspace, their own
/// rep. The runner refuses anyone else a composer on the orchestrator
/// (`not_lead`), so the screens ask here before they draw one.
///
/// It reads `member.list` once the runner says it has people
/// (`representatives`), and again when the fleet's panes change, which is how
/// a rep started or stopped elsewhere gets here. The rules are
/// `RunnerPeople.talk(in:)`, in AgentKit, where they are tested.
@MainActor
final class TalkSeats: ObservableObject {
    @Published private(set) var people: RunnerPeople?
    @Published private(set) var failed = false
    private var panes: Set<String>?
    private var reading = false
    private var lastRead: Date?
    /// How long a list is trusted before it's read again, so a member added
    /// or removed with no pane changing still gets here.
    private static let staleAfter: TimeInterval = 30

    /// This runner's key, for what's remembered about who this phone is.
    private let runner: () -> String

    init(runner: @escaping () -> String = { "" }) { self.runner = runner }

    private var youKey: String { "talk.you.\(runner())" }

    /// Who this phone was the last time the runner said: "" for the owner,
    /// a member's handle, nil if never told. What keeps an owner's phone on
    /// its orchestrator while the list is read, and when it can't be.
    private var knownYou: String? { UserDefaults.standard.string(forKey: youKey) }

    /// Who this phone is on the runner (`RunnerPeople.you`): "" for the
    /// owner, a member's handle, nil if never told. What hides the owner's
    /// marks from a member (ov-707).
    var viewer: String? { people?.you ?? knownYou }

    #if DEBUG
    /// Who this phone is, for a harness that has no `member.list` to say.
    func standIn(you: String) {
        UserDefaults.standard.set(you, forKey: youKey)
    }
    #endif

    /// Whom `summary`'s Talk is (`TalkSeat.resolve`).
    func seat(for summary: WorkspaceSummary, build: DaemonBuild?) -> TalkSeat {
        TalkSeat.resolve(
            workspace: summary, people: people, readFailed: failed,
            hasPeople: build?.can(.representatives) == true, knownYou: knownYou)
    }

    /// Read the people, when the runner has any and the fleet's panes are
    /// not the ones last read for. `force` reads regardless, after a start or
    /// a Try Again.
    func load(_ connection: Connection, panes now: Set<String>? = nil, force: Bool = false) async {
        guard connection.daemon?.can(.representatives) == true, !reading else { return }
        let moved = now != nil && now != panes
        let stale = lastRead.map { Date().timeIntervalSince($0) > Self.staleAfter } ?? true
        guard force || people == nil || moved || stale else { return }
        reading = true
        defer { reading = false }
        if let now { panes = now }
        // The screens watch the connection, not this: tell them first.
        if let data = try? await connection.rpc("member.list"), let read = RunnerPeople(json: data) {
            connection.objectWillChange.send()
            people = read
            failed = false
            lastRead = Date()
            UserDefaults.standard.set(read.you, forKey: youKey)
        } else if people == nil {
            connection.objectWillChange.send()
            failed = true
        }
    }

    /// Open `handle`'s rep in `workspace`: nil when it started (or turned out
    /// to be running already), else what to say. The phone names its own
    /// handle; the runner refuses any other.
    func start(_ connection: Connection, workspace: String, handle: String) async -> String? {
        var said: String?
        do {
            _ = try await connection.rpc("rep.start", ["workspace": workspace, "handle": handle])
        } catch {
            // A rep that is already running, started from another device a
            // moment ago, is what was asked for: the runner says
            // `rep_running`, and the screen says nothing.
            if let core = error as? ClientCore.CoreError, case let .rejected(_, _, what) = core, what == "rep_running" {
                said = nil
            } else {
                said = RunnerPeople.repNotStarted
            }
        }
        await connection.refresh()
        await load(connection, force: true)
        // A rep that was already running (started from another device, or
        // before this phone looked) is what was asked for, whatever the
        // runner said about it.
        if said != nil, let rep = existing(in: workspace, handle: handle),
            connection.fleet.worktrees.lazy.flatMap(\.terminals).contains(where: {
                $0.id == rep.terminal && [.running, .starting].contains(StateKind.parse($0.state))
            })
        {
            said = nil
        }
        return said
    }

    private func existing(in workspace: String, handle: String) -> RunnerPeople.Rep? {
        people?.members.first { $0.handle == handle }?.reps.first { $0.workspace == workspace }
    }
}
