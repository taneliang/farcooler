import AgentKit
import Foundation

/// Whom this Mac talks to in each workspace of one runner (ov-526): the
/// orchestrator, or, for a member who doesn't lead the workspace, their own
/// rep. The runner refuses anyone else a composer on the orchestrator
/// (`not_lead`), so the window asks here before it draws one.
///
/// It reads `farcooler member list --json` once the runner says it has
/// people (`representatives`), and again when the runner's panes change,
/// which is how a rep started or stopped elsewhere gets here. The rules are
/// `TalkSeat.resolve`, in AgentKit, where they are tested. The Mac that runs
/// the runner is its owner's, and always talks to the orchestrator.
@MainActor
final class TalkRoster {
    private(set) var people: RunnerPeople?
    private(set) var failed = false
    private var panes: Set<String>?
    private var reading = false
    private var lastRead: Date?
    /// How long a list is trusted before it's read again, so a member added
    /// or removed with no pane changing still gets here.
    private static let staleAfter: TimeInterval = 30

    /// The runner, as `DaemonClient.target` names it: "" for this Mac's own,
    /// whose owner this Mac is.
    let target: String

    init(target: String) { self.target = target }

    private var youKey: String { "talk.you.\(target)" }

    /// Who this Mac was the last time the runner said: "" for the owner, a
    /// member's handle, nil if never told. Keeps an owner's Mac on the
    /// orchestrator while the list is read again, and when it can't be.
    private var knownYou: String? { UserDefaults.standard.string(forKey: youKey) }

    /// Who this Mac is on the runner (`RunnerPeople.you`): "" for the owner,
    /// as this Mac always is on its own runner, else what the runner last
    /// said, nil if never told. What hides a ruling's marks from a released
    /// Mac (ov-736).
    var viewer: String? { target.isEmpty ? "" : people?.you ?? knownYou }

    /// Whom `workspace`'s Talk is on `client`'s runner.
    func seat(for workspace: WorkspaceSummary, build: DaemonBuild?) -> TalkSeat {
        TalkSeat.resolve(
            workspace: workspace, people: people, readFailed: failed,
            hasPeople: build?.can(.representatives) == true, knownYou: knownYou, isLocalOwner: target.isEmpty)
    }

    /// Read the people, when the runner has any and its panes are not the
    /// ones last read for. `force` reads regardless, after a start or a Try
    /// Again.
    func load(_ client: DaemonClient, panes now: Set<String>? = nil, force: Bool = false) async {
        guard client.daemonBuild?.can(.representatives) == true, !reading else { return }
        let moved = now != nil && now != panes
        let stale = lastRead.map { Date().timeIntervalSince($0) > Self.staleAfter } ?? true
        guard force || people == nil || moved || stale else { return }
        reading = true
        defer { reading = false }
        if let now { panes = now }
        let (data, _) = await client.runRaw(["member", "list", "--json"], background: true)
        // The window watches the client, not this: tell it first.
        if let data, let read = RunnerPeople(json: data) {
            client.objectWillChange.send()
            people = read
            failed = false
            lastRead = Date()
            UserDefaults.standard.set(read.you, forKey: youKey)
        } else if people == nil {
            client.objectWillChange.send()
            failed = true
        }
    }

    /// Open `handle`'s rep in `workspace`: nil when it started (or turned out
    /// to be running already), else the sentence to show. The Mac names its
    /// own handle; the runner refuses any other.
    func start(_ client: DaemonClient, workspace: WorkspaceSummary, handle: String) async -> String? {
        let (data, message) = await client.runRaw(["rep", handle, "--workspace", workspace.id, "--json"])
        await client.refresh()
        await load(client, force: true)
        if data != nil { return nil }
        // A rep that is already running, or starting, is what was asked for,
        // whatever the runner said about it: it says `rep_running` when
        // another device got there first.
        if TaskFailure.what(in: message) == "rep_running" { return nil }
        if let rep = people?.members.first(where: { $0.handle == handle })?.reps.first(where: { $0.workspace == workspace.id }),
            client.fleet.worktrees.lazy.flatMap(\.terminals).contains(where: {
                $0.id == rep.terminal && [.running, .starting].contains(StateKind.parse($0.state))
            })
        {
            return nil
        }
        return RunnerPeople.repNotStarted
    }
}

extension DaemonClient {
    /// Who this Mac is on the runner, for reading a pane's Claude login as
    /// "yours" (ov-597): "" on this Mac's own runner, whose owner this Mac is,
    /// else what `member.list` said, nil until it has.
    var claudeLoginViewer: String? { target.isEmpty ? "" : talk.people?.you }
}
