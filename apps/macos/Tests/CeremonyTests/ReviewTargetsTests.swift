import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Which panes a review note can be handed to.
///
/// In the target with teeth because both ways of getting this wrong are silent.
/// Too narrow and a worktree with a working agent in it offers nowhere to send
/// to, for the sole reason that somebody flipped the pane back to its tty —
/// which looks like the feature being broken. Too wide and the list offers the
/// diff pane itself, whose pane-mode switch the daemon refuses, so the note
/// goes nowhere and says nothing. Neither is visible by looking at the pane.
@MainActor
struct ReviewTargetsTests {
    private func workspace(_ terminals: String) throws -> Workspace {
        try JSONDecoder().decode(
            Workspace.self,
            from: Data(
                """
                {"id":"w1","short":"w1","task":"t","branch":"feature",
                 "worktree":"/tmp/w1","state":"ready","terminals":[\(terminals)]}
                """.utf8))
    }

    private func terminal(
        id: String, short: String, preset: String, title: String = "Terminal 1",
        paneMode: String? = nil, chatCapable: Bool? = nil
    ) -> String {
        let mode = paneMode.map { "\"\($0)\"" } ?? "null"
        let chat = chatCapable.map { $0 ? "true" : "false" } ?? "null"
        return """
            {"id":"\(id)","short":"\(short)","title":"\(title)","preset":"\(preset)",
             "state":"running","epoch":1,"paneMode":\(mode),"chatCapable":\(chat)}
            """
    }

    /// A claude the reader has flipped back to its raw terminal is still an
    /// agent holding an ACP session, and `terminal agent-prompt` reaches it.
    @Test func aChatFlippedBackToItsTerminalIsStillATarget() throws {
        let ws = try workspace(
            terminal(id: "t1", short: "%1", preset: "claude", chatCapable: true))
        let targets = ws.reviewAgentTargets()
        #expect(targets.count == 1)
        #expect(targets.first?.id == "%1")
        // Not showing a chat, so there is no composer to put anything in — but
        // it is still perfectly good to SEND to.
        #expect(targets.first?.showsChat == false)
    }

    @Test func aPlainShellIsNotATarget() throws {
        let ws = try workspace(terminal(id: "t1", short: "%1", preset: "zsh"))
        #expect(ws.reviewAgentTargets().isEmpty)
    }

    /// The one the phone's filter would get wrong here. A changes pane is a
    /// real tmux pane whose contents this app draws, and the daemon refuses to
    /// switch it — so it can never hold an agent, and offering it would be a
    /// note sent into a diff.
    @Test func theDiffPaneIsNeverATarget() throws {
        let ws = try workspace(
            terminal(
                id: "t1", short: "%1", preset: "farcooler", title: "Changes",
                paneMode: "changes", chatCapable: true))
        #expect(ws.reviewAgentTargets().isEmpty)
    }

    /// Two identical `claude`s are told apart, and a pane the agent has named
    /// keeps its name — an ordinal exists to disambiguate, and a title already
    /// has.
    @Test func identicalAgentsAreNumberedAndNamedOnesAreNot() throws {
        let ws = try workspace(
            [
                terminal(id: "t1", short: "%1", preset: "claude", paneMode: "agent"),
                terminal(id: "t2", short: "%2", preset: "claude", paneMode: "agent"),
                terminal(
                    id: "t3", short: "%3", preset: "claude", title: "Fixing the parser",
                    paneMode: "agent"),
            ].joined(separator: ","))
        let targets = ws.reviewAgentTargets()
        #expect(targets.map(\.name) == ["claude 1", "claude 2", "Fixing the parser"])
        #expect(targets.filter(\.showsChat).count == 3)
    }
}
