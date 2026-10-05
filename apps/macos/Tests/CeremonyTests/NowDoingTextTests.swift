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

    @Test("The activity is part of the orchestrator's one label, before its caret, with no dash")
    func labelComposition() {
        #expect(TitleStatus.orchestratorWords(.working, doing: "Reviewing the gesture fix") == "Orchestrator · Working: Reviewing the gesture fix")
        #expect(TitleStatus.orchestratorWords(.idle, doing: nil) == "Orchestrator · Idle")
        #expect(TitleStatus.orchestratorWords(OrchestratorRow.State.none, doing: "anything") == "No Orchestrator")
        #expect(!TitleStatus.orchestratorWords(.working, doing: "x").contains("—"))
    }

    @Test("A long line is cut at its tail to the form's width, whatever else is showing; the state is never cut", arguments: [TitleStatus.Form.medium, .wide])
    func fits(form: TitleStatus.Form) {
        let long = "Reading ov-192’s jump bar diff, then the integration report for integ-4 and its captures"
        let row = TaskRow(id: "t", key: "ov-1", title: "T", status: .inProgress, statusSince: Date(timeIntervalSince1970: 0))
        for model in [
            TitleStatus.Model(orchestrator: .working, status: nil, nowDoing: long, needYou: 0, running: [], inReview: []),
            TitleStatus.Model(orchestrator: .working, status: nil, nowDoing: long, needYou: 142, running: [row], inReview: [row], queued: 3),
        ] {
            let words = TitleStatus.fittedWords(model, form: form)
            if TitleStatus.textWidth("Orchestrator · Working: Reading") < form.width - TitleStatus.reserved(model, form: form) {
                #expect(words.hasPrefix("Orchestrator · Working: Reading") && words.hasSuffix("…"), "\(words)")
            } else {
                // Crowded by every count: the state, which is the status, and what fits after it (the width check below).
                #expect(words.hasPrefix("Orchestrator · Working"), "\(form): \(words)")
            }
            let room = form.width - TitleStatus.reserved(model, form: form)
            #expect(TitleStatus.textWidth(words) <= max(room, TitleStatus.textWidth("Orchestrator · Working")), "\(form): \(words)")
        }
        // Short enough, whole; no line, the state alone.
        let short = TitleStatus.Model(orchestrator: .working, status: nil, nowDoing: "Reviewing", needYou: 1, running: [], inReview: [])
        #expect(TitleStatus.fittedWords(short, form: .wide) == "Orchestrator · Working: Reviewing")
        let none = TitleStatus.Model(orchestrator: .idle, status: nil, nowDoing: nil, needYou: 0, running: [], inReview: [])
        #expect(TitleStatus.fittedWords(none, form: .medium) == "Orchestrator · Idle")
        #expect(TitleStatus.fittedWords(short, form: .short) == "Working")
    }

    @Test("At the medium form the room goes to the line, not the Activity button; at wide both show")
    func activityButtonRoom() {
        let doing = TitleStatus.Model(orchestrator: .working, status: nil, nowDoing: "x", needYou: 0, running: [], inReview: [])
        var quiet = doing
        quiet.nowDoing = nil
        #expect(!TitleStatus.showsActivityButton(doing, form: .medium))
        #expect(TitleStatus.showsActivityButton(quiet, form: .medium))
        #expect(TitleStatus.showsActivityButton(doing, form: .wide))
        #expect(!TitleStatus.showsActivityButton(doing, form: .short))
    }
}
