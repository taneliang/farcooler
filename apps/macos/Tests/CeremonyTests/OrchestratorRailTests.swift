import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The rail beside a task says it's the orchestrator, and what it's doing
/// (ov-84).
struct OrchestratorRailTests {
    private static func seat(state: String = "running", activity: String? = nil, preset: String = "claude") -> BoardPane {
        var t = Terminal(id: "conductor", short: "cond", title: "orchestrator", preset: preset, state: state, epoch: 0)
        t.role = "orchestrator"
        t.activity = activity
        let checkout = Worktree(
            id: "checkout", short: "co", task: "main", branch: "main", repository: "overnight", host: "",
            path: "/tmp/co", state: "active", terminals: [t])
        return BoardPane(terminal: t, worktree: checkout)
    }

    /// The same sources as the column's header: the seat's status and its
    /// unread turn, and the workspace's needs-you count, which leads.
    @Test("The rail's state comes from the seat and what waits on you")
    func theRailsStateComesFromTheSeat() {
        typealias Rail = OrchestratorRail
        #expect(Rail.state(seat: nil, waiting: 0) == .none)
        #expect(Rail.state(seat: nil, waiting: 3) == .none)
        #expect(Rail.state(seat: Self.seat(activity: "working"), waiting: 0) == .working)
        #expect(Rail.state(seat: Self.seat(activity: "idle"), waiting: 0) == .idle)
        #expect(Rail.state(seat: Self.seat(), waiting: 0) == .idle)
        #expect(Rail.state(seat: Self.seat(activity: "blocked"), waiting: 0) == .needsYou)
        #expect(Rail.state(seat: Self.seat(activity: "working"), waiting: 2) == .needsYou)
        #expect(Rail.state(seat: Self.seat(activity: "done"), waiting: 0) == .unread)
        #expect(Rail.state(seat: Self.seat(state: "starting"), waiting: 0) == .starting)
        #expect(Rail.state(seat: Self.seat(state: "lost"), waiting: 0) == .stopped)
        #expect(Rail.state(seat: Self.seat(state: "exited"), waiting: 0) == .stopped)
    }

    @Test("VoiceOver hears Orchestrator and its state, and Show or Hide")
    func voiceOverHearsOrchestratorAndItsState() {
        typealias Rail = OrchestratorRail
        #expect(Rail.accessibilityLabel(.working) == "Orchestrator, working")
        #expect(Rail.accessibilityLabel(.needsYou) == "Orchestrator, needs you")
        #expect(Rail.accessibilityLabel(.idle) == "Orchestrator, idle")
        #expect(Rail.accessibilityLabel(.none) == "Orchestrator, not running")
        #expect(Rail.action(open: false) == "Show")
        #expect(Rail.action(open: true) == "Hide")
        #expect(Rail.help(open: false) == "Show Orchestrator (⌥⌘1)")
        #expect(Rail.help(open: true) == "Hide Orchestrator (⌥⌘1)")
    }

    /// "Orchestrator · claude" where it fits, else "Orchestrator"; no name
    /// with no agent running, as the header says (checklist O7).
    @Test("The label names the agent when there's room")
    func theLabelNamesTheAgentWhenTheresRoom() {
        #expect(OrchestratorRail.titles(agent: "claude") == ["Orchestrator · claude", "Orchestrator"])
        #expect(OrchestratorRail.titles(agent: nil) == ["Orchestrator"])
        #expect(OrchestratorRail.titles(agent: ConversationHeader.agentName(Self.seat(state: "lost", preset: ""))) == ["Orchestrator"])
        #expect(OrchestratorRail.titles(agent: ConversationHeader.agentName(Self.seat())).first == "Orchestrator · claude")
    }
}
