import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The orchestrator's now-doing line in the title bar (ov-329): its terminal
/// furniture stripped, and its place in the label.
@MainActor
struct NowDoingTextTests {
    @Test("A status line as an agent's screen gives it loses its hints, spinners and boxes",
          arguments: [
            ("✻ Reviewing the gesture fix… (esc to interrupt)", "Reviewing the gesture fix…"),
            ("· Thinking… (12s · ↓ 1.2k tokens · esc to interrupt)", "Thinking… (12s · ↓ 1.2k tokens)"),
            ("User test issues ⌘K", "User test issues"),
            ("Reading the board ⌃C", "Reading the board"),
            ("Reading the board (⌃C)", "Reading the board"),
            ("Compiling ctrl+c", "Compiling"),
            ("╭─ Writing the plan ─╮", "Writing the plan"),
            ("⠋ Compiling AgentKit", "Compiling AgentKit"),
            ("2/5 · Writing tests · 1 agent", "2/5 · Writing tests · 1 agent"),
          ])
    func strips(raw: String, clean: String) {
        #expect(NowDoingText.clean(raw) == clean)
    }

    @Test("Real content survives: chords and phrases in a sentence, tokens, a slash command",
          arguments: [
            "Bind ⌥⌘← to Collapse All?",
            "Making escape to dismiss work in the popover",
            "Fix the escape key handling",
            "Esc to cancel the import?",
            "Fixed the colors (visual tokens lint)",
            "Reading 12k (12k tokens)",
            "(12k tokens)",
            "/review the PR",
            "-1 on the ⌘K idea",
          ])
    func keeps(raw: String) {
        #expect(NowDoingText.clean(raw) == raw)
    }

    @Test("With nothing meaningful left, there is no activity", arguments: ["⌘K", "esc to interrupt", "✻", "╭────╮", "  ⏺ ", "(esc to interrupt)", "— ⌘K", "(⌃C)"])
    func nothingLeft(raw: String) {
        #expect(NowDoingText.clean(raw) == nil)
        #expect(NowDoingText.clean(nil) == nil)
    }

    @Test("Only the status line is cleaned: the question it's blocked on and what it said are kept as written")
    func throughNowDoing() {
        var terminal = Terminal(id: "o", short: "o", title: "o", preset: "claude", state: "running", epoch: 0)
        terminal.line = "✻ Reviewing the gesture fix… (esc to interrupt)"
        #expect(OrchestratorRow.nowDoing(terminal, state: .working) == "Reviewing the gesture fix…")
        terminal.line = "⌘K"
        #expect(OrchestratorRow.nowDoing(terminal, state: .working) == nil)
        // Each of these would lose its tail if it were cleaned as a status line.
        terminal.blockedQuestion = "Should the palette open on ⌘P or ⌘K"
        #expect(OrchestratorRow.nowDoing(terminal, state: .needsYou) == "Should the palette open on ⌘P or ⌘K")
        terminal.said = "Pressing esc to interrupt stops the turn"
        terminal.line = nil
        #expect(OrchestratorRow.nowDoing(terminal, state: .idle) == "Pressing esc to interrupt stops the turn")
    }

    @Test("A long line is cut at its tail to the room it has; a short one is whole")
    func cutAtTheTail() {
        let long = "Reading ov-192’s jump bar diff, then the integration report for integ-4 and its captures"
        let room: CGFloat = 130
        let cut = TitleStatus.fit(long, room: room)
        #expect(cut.hasSuffix("…") && long.hasPrefix(String(cut.dropLast())), "\(cut)")
        #expect(TitleStatus.textWidth(cut) <= room, "\(cut)")
        #expect(TitleStatus.fit("Delegating ov-468", room: 200) == "Delegating ov-468")
    }

    @Test("The tooltip over a cut line has all of it")
    func tooltip() {
        let long = "Reading ov-192’s jump bar diff, then the integration report for integ-4 and its captures"
        let cut = TitleStatus.Model(orchestrator: .working, status: nil, nowDoing: long, needYou: 0, running: [], inReview: [])
        #expect(TitleStatus.statusHelp(cut) == "Working: \(long)")
    }
}
