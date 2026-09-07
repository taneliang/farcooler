import Foundation
import Testing

@testable import AgentKit

/// Which panes need a terminal session behind them, and which have never drawn
/// one.
///
/// The cost of getting this wrong is invisible on screen and expensive
/// everywhere else: `TerminalView` hung its visibility work off one `.task` on
/// the whole `VStack`, outside the branch that decides what a pane draws, so
/// every agent pane opened a full ssh session and a two-second geometry poll
/// for a VT grid it does not render. A default `sshd` allows ten concurrent
/// sessions; a fleet of agents is how you spend them.
struct PaneSessionTests {
    private func terminal(mode: String?) throws -> Terminal {
        let pane = mode.map { "\"paneMode\":\"\($0)\"," } ?? ""
        let json = """
            {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude",
             "state":"running",\(pane)"epoch":1}
            """
        return try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    }

    /// A tty. This is the one, and it is the only one.
    @Test func aTerminalPaneNeedsASession() throws {
        #expect(try terminal(mode: "terminal").needsTerminalSession)
    }

    /// **The defect.** A chat reads its own subscribe stream from seq 0 and
    /// draws no grid at all; the session it was opening carried bytes nothing
    /// ever looked at.
    @Test func anAgentPaneNeedsNoSession() throws {
        let pane = try terminal(mode: "agent")
        #expect(pane.isAgentPane)
        #expect(!pane.needsTerminalSession)
    }

    /// And a diff reads a `ChangesStore`. Same argument, other branch.
    @Test func aChangesPaneNeedsNoSession() throws {
        let pane = try terminal(mode: "changes")
        #expect(pane.isChangesPane)
        #expect(!pane.needsTerminalSession)
    }

    /// A daemon too old to send the mode falls through to the VT grid — that is
    /// what `isAgentPane` and `isChangesPane` both being false means — so it
    /// needs the session. Answering "no" here would be a pane that draws a grid
    /// with nothing behind it.
    @Test func aPaneWithNoModeAtAllStillDrawsAGrid() throws {
        let pane = try terminal(mode: nil)
        #expect(pane.paneMode == nil)
        #expect(pane.needsTerminalSession)
    }

    /// A mode this app has never heard of draws the grid too, for the same
    /// reason, and must not be quietly left without a session.
    @Test func anUnknownModeIsTreatedAsAGrid() throws {
        #expect(try terminal(mode: "something-newer").needsTerminalSession)
    }
}
