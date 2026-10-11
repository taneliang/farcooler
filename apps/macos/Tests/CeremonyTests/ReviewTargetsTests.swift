import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Which panes a review note can be handed to: claude or codex in a terminal
/// pane, whose box `terminal compose` types into.
///
/// In the target with teeth because both ways of getting this wrong are silent.
/// Too narrow and a worktree with a working agent in it offers nowhere to send
/// to, which looks like the feature being broken. Too wide and the list offers
/// a shell or the diff pane itself, which nothing composes into, so the note
/// goes nowhere and says nothing. Neither is visible by looking at the pane.
@MainActor
struct ReviewTargetsTests {
    private func worktree(_ terminals: String) throws -> Worktree {
        try JSONDecoder().decode(
            Worktree.self,
            from: Data(
                """
                {"id":"w1","short":"w1","task":"t","branch":"feature",
                 "worktree":"/tmp/w1","state":"ready","terminals":[\(terminals)]}
                """.utf8))
    }

    private func terminal(
        id: String, short: String, preset: String, title: String = "Terminal 1", paneMode: String? = nil
    ) -> String {
        let mode = paneMode.map { "\"\($0)\"" } ?? "null"
        return """
            {"id":"\(id)","short":"\(short)","title":"\(title)","preset":"\(preset)",
             "state":"running","epoch":1,"paneMode":\(mode)}
            """
    }

    /// Claude and codex in their terminals, whichever view is showing.
    @Test func claudeAndCodexInATerminalAreTargets() throws {
        let ws = try worktree(
            [
                terminal(id: "t1", short: "%1", preset: "claude", paneMode: "terminal"),
                terminal(id: "t2", short: "%2", preset: "codex:gpt-5", title: "Codex work"),
            ].joined(separator: ","))
        #expect(ws.reviewAgentTargets().map(\.id) == ["%1", "%2"])
    }

    @Test func aPlainShellIsNotATarget() throws {
        let ws = try worktree(terminal(id: "t1", short: "%1", preset: "zsh"))
        #expect(ws.reviewAgentTargets().isEmpty)
    }

    /// A changes pane is a real tmux pane whose contents this app draws, so it
    /// can never hold an agent, and offering it would be a note sent into a
    /// diff.
    @Test func theDiffPaneIsNeverATarget() throws {
        let ws = try worktree(
            terminal(id: "t1", short: "%1", preset: "claude", title: "Changes", paneMode: "changes"))
        #expect(ws.reviewAgentTargets().isEmpty)
    }

    /// Two identical `claude`s are told apart, and a pane the agent has named
    /// keeps its name — an ordinal exists to disambiguate, and a title already
    /// has.
    @Test func identicalAgentsAreNumberedAndNamedOnesAreNot() throws {
        let ws = try worktree(
            [
                terminal(id: "t1", short: "%1", preset: "claude"),
                terminal(id: "t2", short: "%2", preset: "claude"),
                terminal(id: "t3", short: "%3", preset: "claude", title: "Fixing the parser"),
            ].joined(separator: ","))
        #expect(ws.reviewAgentTargets().map(\.name) == ["claude 1", "claude 2", "Fixing the parser"])
    }
}
