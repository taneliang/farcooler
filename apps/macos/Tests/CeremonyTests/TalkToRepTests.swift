import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A member's Talk on the Mac opens their own rep (ov-526): the orchestrator
/// for the owner and the lead, a rep for any other member, started first when
/// it isn't running, and never a composer the runner would refuse `not_lead`.
@MainActor
@Suite(.serialized)
struct TalkToRepTests {
    /// What this Mac remembers of who it is must not leak between tests.
    init() { UserDefaults.standard.removeObject(forKey: "talk.you.box") }

    static let board = WorkspaceSummary(
        id: "w-billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, lead: "alice")

    static let members =
        #"{"members":[{"handle":"alice","name":"Alice Chen","devices":[],"reps":0,"rep_panes":[]},"#
        + #"{"handle":"bob","name":"Bob","devices":["mac-b"],"reps":1,"rep_panes":[{"terminal":"t1","workspace":"w-billing"}]}],"#
        + #""claude_account":"team@example.com","you":"bob"}"#

    static func build(_ capabilities: Set<String>) -> DaemonBuild {
        DaemonBuild(version: "0.1.0", matches: true, platform: "macos", capabilities: capabilities)
    }

    /// A client whose CLI answers `member list` with `members` and records
    /// what else it was asked.
    static func client(members: String? = members, started: Started = .ok(#"{"terminal":"t1","desk":"bil-1","claude_account":"team@example.com"}"#), log: Log = Log()) -> DaemonClient {
        let client = DaemonClient(target: "box")
        client.daemonBuild = build(["representatives", "workstreams"])
        client.commandRunnerForTesting = { args in
            log.asked.append(args)
            if args == ["member", "list", "--json"] { return (members.map { Data($0.utf8) }, members == nil ? "unavailable" : nil) }
            if args.first == "rep" {
                switch started {
                case .ok(let out): return (Data(out.utf8), nil)
                case .refused(let why): return (nil, why)
                }
            }
            return (Data(#"{"worktrees":[]}"#.utf8), nil)
        }
        return client
    }

    enum Started { case ok(String), refused(String) }

    final class Log { var asked: [[String]] = [] }

    @Test("A member who doesn't lead the workspace gets their rep, once the people are read; until then nothing to type into")
    func memberGetsTheirRep() async {
        let client = Self.client()
        #expect(client.talk.seat(for: Self.board, build: client.daemonBuild) == .reading)
        await client.talk.load(client)
        #expect(
            client.talk.seat(for: Self.board, build: client.daemonBuild)
                == .rep(handle: "bob", name: "Bob", existing: .init(terminal: "t1", workspace: "w-billing")))
        // The lead of another workspace is still a member here; the lead of this one keeps the orchestrator.
        let led = WorkspaceSummary(id: "w-billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, lead: "bob")
        #expect(client.talk.seat(for: led, build: client.daemonBuild) == .orchestrator)
    }

    @Test("A runner without people is never asked, and its orchestrator is the seat")
    func runnerWithoutPeople() async {
        let log = Log()
        let client = Self.client(log: log)
        client.daemonBuild = Self.build(["workstreams"])
        await client.talk.load(client)
        #expect(log.asked.isEmpty)
        #expect(client.talk.seat(for: Self.board, build: client.daemonBuild) == .orchestrator)
    }

    @Test("A read that fails says it can't tell, rather than offering the orchestrator")
    func failedRead() async {
        let client = Self.client(members: nil)
        await client.talk.load(client)
        #expect(client.talk.seat(for: Self.board, build: client.daemonBuild) == .unknown)
    }

    @Test("Starting a rep names the workspace by id and the member's own handle, and reads the people again")
    func start() async {
        let log = Log()
        let client = Self.client(log: log)
        await client.talk.load(client)
        let said = await client.talk.start(client, workspace: Self.board, handle: "bob")
        #expect(said == nil)
        #expect(log.asked.contains(["rep", "bob", "--workspace", "w-billing", "--json"]), "\(log.asked)")
        #expect(log.asked.filter { $0 == ["member", "list", "--json"] }.count == 2, "the people were not read again")
    }

    @Test("A refused start says so in the app's words, not the runner's")
    func refusedStart() async {
        let client = Self.client(started: .refused("code: scope-denied\nthe runner said something raw"))
        await client.talk.load(client)
        let said = await client.talk.start(client, workspace: Self.board, handle: "bob")
        #expect(said == RunnerPeople.repNotStarted, "\(said ?? "nil")")
    }

    @Test("A start that races another device's, which the runner answers rep_running, says nothing")
    func racedStart() async {
        let client = Self.client(started: .refused("code: resource-conflict\nwhat: rep_running"))
        await client.talk.load(client)
        #expect(await client.talk.start(client, workspace: Self.board, handle: "bob") == nil)
    }

    @Test("A refusal that is not rep_running is still said, even for a rep that is not running")
    func otherRefusal() async {
        let client = Self.client(started: .refused("code: resource-conflict\nwhat: something_else"))
        await client.talk.load(client)
        #expect(await client.talk.start(client, workspace: Self.board, handle: "bob") == RunnerPeople.repNotStarted)
    }

    @Test("The owner never waits on the people: this Mac's own runner is the orchestrator's at once, and a remote one once it has said so")
    func ownerDoesNotWait() async {
        UserDefaults.standard.removeObject(forKey: "talk.you.box")
        defer { UserDefaults.standard.removeObject(forKey: "talk.you.box") }
        let local = DaemonClient(target: "")
        local.daemonBuild = Self.build(["representatives", "workstreams"])
        #expect(local.talk.seat(for: Self.board, build: local.daemonBuild) == .orchestrator)

        // A remote runner this Mac has never been told about is shut out until it is.
        let owners = Self.client(members: #"{"members":[],"claude_account":"a@b.c","you":""}"#)
        #expect(owners.talk.seat(for: Self.board, build: owners.daemonBuild) == .reading)
        await owners.talk.load(owners)
        #expect(owners.talk.seat(for: Self.board, build: owners.daemonBuild) == .orchestrator)
        // Told once, a fresh client with a failing read keeps the orchestrator.
        let later = Self.client(members: nil)
        #expect(later.talk.seat(for: Self.board, build: later.daemonBuild) == .orchestrator)
        await later.talk.load(later)
        #expect(later.talk.seat(for: Self.board, build: later.daemonBuild) == .orchestrator)
    }

    @Test("The column: a rep's states from its pane and a start in flight; Talk starts it only when it isn't running")
    func columnStates() {
        let seat = TalkSeat.rep(handle: "bob", name: "Bob", existing: .init(terminal: "t1", workspace: "w-billing"))
        func pane(_ state: String) -> Terminal {
            Terminal(id: "t1", short: "d1", title: "claude", preset: "claude", state: state, epoch: 1, paneMode: "terminal", role: "agent")
        }
        #expect(RepColumn.state(seat: .reading, pane: nil, isStarting: false) == .reading)
        #expect(RepColumn.state(seat: .unknown, pane: nil, isStarting: false) == .unknown)
        #expect(RepColumn.state(seat: .neither, pane: nil, isStarting: false) == .neither)
        #expect(RepColumn.state(seat: seat, pane: nil, isStarting: false) == .none)
        #expect(RepColumn.state(seat: seat, pane: nil, isStarting: true) == .starting)
        #expect(RepColumn.state(seat: seat, pane: pane("starting"), isStarting: false) == .starting)
        #expect(RepColumn.state(seat: seat, pane: pane("exited"), isStarting: false) == .stopped)
        #expect(RepColumn.state(seat: seat, pane: pane("running"), isStarting: false) == .live)
        #expect(RepColumn.startsOnTalk(.none, canAct: true))
        #expect(RepColumn.startsOnTalk(.stopped, canAct: true))
        for state in [RepColumn.State.starting, .live, .reading, .unknown, .neither] {
            #expect(!RepColumn.startsOnTalk(state, canAct: true), "\(state) starts a rep")
        }
        #expect(!RepColumn.startsOnTalk(.none, canAct: false), "a read-only Mac starts a rep")
    }
}
