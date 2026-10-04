import AgentKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// One status, one presentation (ov-137): every Mac surface reads a
/// terminal's status through `Status.label`, `Status.word` and
/// `Status.tone`, which agree with the phones' `GlanceState` (ov-125).
struct StatusPresentationTests {
    static func seat(activity: String, turnFailed: Bool = false) -> BoardPane {
        var t = Terminal(id: "agent", short: "a", title: "claude", preset: "claude", state: "running", epoch: 0)
        t.activity = activity
        t.turnFailed = turnFailed
        let worktree = Worktree(
            id: "wt", short: "w", task: "wt", branch: "wt", repository: "shop", host: "", path: "/tmp/w",
            state: "active", terminals: [t])
        return BoardPane(terminal: t, worktree: worktree)
    }

    /// Every status, exhaustively: its word is its label, and where the
    /// phones have the state, the same word in the same tone.
    @Test(arguments: Status.allCases)
    func everyStatusReadsAsThePhonesDo(_ status: Status) {
        if let word = status.word { #expect(word == status.label.lowercased()) }
        if let glance = status.glanceState {
            #expect(status.tone == glance.tone, "\(status)")
            if let title = glance.title { #expect(status.label == title, "\(status)") }
        }
        // Color only for the states that want a person.
        #expect((status.tone != .quiet) == (status.wantsAttention && status != .done), "\(status)")
    }

    @Test func aFailedTurnReadsAsFailedEverywhere() {
        let seat = Self.seat(activity: "done", turnFailed: true)
        #expect(seat.terminal.status == .failedTurn)
        // The orchestrator's row and the title bar.
        #expect(OrchestratorRow.state(seat: seat) == .failed)
        #expect(OrchestratorRow.word(OrchestratorRow.state(seat: seat)) == "Failed")
        // A task's header.
        let line = TaskColumnModel.agentLine(seat)
        #expect(line?.text.hasSuffix("failed") == true, "\(line?.text ?? "")")
        #expect(line?.status?.tone == .failed)
        // A task row's agent word, and its ink.
        let agent = TaskRowMeta.agent(live: [seat], presence: .agents(1))
        #expect(agent?.word.hasSuffix("failed") == true)
        // The board's worktree dot.
        #expect(Status.failedTurn.tone.color(.light) == Tint.failure)
    }

    @Test func needsYouIsAmberEverywhereAndDoneIsDone() {
        let blocked = Self.seat(activity: "blocked")
        #expect(TaskColumnModel.agentLine(blocked)?.status?.tone == .needsYou)
        #expect(TaskRowMetaView.color(.attention, scheme: .dark) == Tint.attention(.dark))
        #expect(OrchestratorRow.State.needsYou.status?.tone.color(.dark) == Tint.attention(.dark))
        #expect(TaskRowMeta.word(.done) == "done")
        #expect(Status.done.tone.color(.light) == .secondary, "a finished turn's word stays neutral")
    }
}
