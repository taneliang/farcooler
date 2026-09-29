import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The conversation column's states around an orchestrator (spec §8).
struct ConversationColumnTests {
    private static func seat(state: String, activity: String? = nil) -> BoardPane {
        var t = Terminal(id: "conductor", short: "cond", title: "orchestrator", preset: "claude", state: state, epoch: 0)
        t.role = "orchestrator"
        t.activity = activity
        let checkout = Worktree(
            id: "checkout", short: "co", task: "main", branch: "main", repository: "overnight", host: "",
            path: "/tmp/co", state: "active", terminals: [t])
        return BoardPane(terminal: t, worktree: checkout)
    }

    private static let now = Date(timeIntervalSince1970: 1_800_000_000)

    /// An empty seat offers a start with each harness, and says what an
    /// orchestrator is for. A read-only runner offers nothing.
    @Test("No orchestrator offers Start Orchestrator with each harness")
    func noOrchestratorOffersStartWithEachHarness() {
        let state = ConversationColumn.state(seat: nil, isStarting: false, startedAt: nil, now: Self.now)
        #expect(state == .none)
        #expect(ConversationColumn.offers(state) == [.start(.claude), .start(.codex), .start(.cursor)])
        #expect(ConversationColumn.offers(state, canAct: false).isEmpty)
        // While this app's start is in flight, nothing more is offered.
        let starting = ConversationColumn.state(seat: nil, isStarting: true, startedAt: Self.now, now: Self.now)
        #expect(starting == .starting(slow: false))
        #expect(ConversationColumn.offers(starting).isEmpty)
    }

    /// A lost pane, or one that exited, is its last screen dimmed with
    /// Restart, which resumes the conversation, and Replace….
    @Test("A lost orchestrator offers Restart and Replace")
    func aLostOrchestratorOffersRestartAndReplace() {
        for word in ["LOST", "exited", "error"] {
            let state = ConversationColumn.state(
                seat: Self.seat(state: word), isStarting: false, startedAt: nil, now: Self.now)
            #expect(state == .lost, "\(word)")
            #expect(ConversationColumn.offers(state) == [.restart, .replace])
        }
        let live = ConversationColumn.state(seat: Self.seat(state: "running"), isStarting: false, startedAt: nil, now: Self.now)
        #expect(live == .live)
        #expect(ConversationColumn.offers(live).isEmpty)
    }

    /// The seat can stick in `starting`. After 30 seconds the column says
    /// it's taking longer than usual and offers Replace…, and not before.
    @Test("A start unconfirmed after 30 seconds offers Replace")
    func aStartUnconfirmedAfterThirtySecondsOffersReplace() {
        let began = Self.now.addingTimeInterval(-29)
        let early = ConversationColumn.state(seat: Self.seat(state: "starting"), isStarting: false, startedAt: began, now: Self.now)
        #expect(early == .starting(slow: false))
        #expect(ConversationColumn.offers(early).isEmpty)
        let late = ConversationColumn.state(
            seat: Self.seat(state: "starting"), isStarting: false, startedAt: began, now: Self.now.addingTimeInterval(1))
        #expect(late == .starting(slow: true))
        #expect(ConversationColumn.offers(late) == [.replace])
        // This app's own start that the runner hasn't answered, too.
        #expect(
            ConversationColumn.state(seat: nil, isStarting: true, startedAt: began, now: Self.now.addingTimeInterval(5))
                == .starting(slow: true))
    }

    /// Its finished turn is a dot, not an inbox item (ruling 10), until the
    /// pane is seen, which turns `done` into `idle`.
    @Test("An orchestrator's finished turn is an unread dot until seen")
    func anOrchestratorsFinishedTurnIsAnUnreadDotUntilSeen() {
        #expect(ConversationColumn.unread(Self.seat(state: "running", activity: "done")))
        #expect(!ConversationColumn.unread(Self.seat(state: "running", activity: "idle")))
        #expect(!ConversationColumn.unread(Self.seat(state: "running", activity: "working")))
        // A block is an item of its own, not an unread turn.
        #expect(!ConversationColumn.unread(Self.seat(state: "running", activity: "blocked")))
        #expect(!ConversationColumn.unread(nil))
        // And it isn't an item: an older runner's derived list leaves it out.
        let finished = Self.seat(state: "running", activity: "done")
        #expect(NeedsYou.derived(fromTerminals: DaemonClient.olderPanes(in: [finished.worktree])).isEmpty)
    }
}
