import Foundation
import Testing

@testable import AgentKit

// ov-373: the rules the phone's conversation view decides by. The views
// themselves are held by the iOS UI tests (`NativeAgentViewTests`).

@Suite struct AgentConversationTests {
    private func build(_ capabilities: [String]) -> DaemonBuild {
        DaemonBuild(version: "t", matches: true, platform: "t", capabilities: Set(capabilities))
    }

    @Test func servedOnlyWithRowsAndCompose() {
        #expect(AgentConversation.served(by: build(["agent_rows", "agent_compose"])))
        #expect(!AgentConversation.served(by: build(["agent_rows"])), "rows from before compose: no view whose sends fail")
        #expect(!AgentConversation.served(by: build(["agent_compose"])), "the projector off: no rows")
        #expect(!AgentConversation.served(by: build([])), "a runner from before capabilities")
        #expect(!AgentConversation.served(by: nil), "not read yet")
    }

    @Test func claudeOrCodexInATerminal() {
        let old = build(["agent_rows", "agent_compose"])
        let codex = build(["agent_rows", "agent_compose", "codex_view"])
        #expect(AgentConversation.isAgentInATerminal(paneMode: "terminal", preset: "claude", build: old))
        #expect(AgentConversation.isAgentInATerminal(paneMode: nil, preset: "claude --resume x", build: nil))
        #expect(AgentConversation.isAgentInATerminal(paneMode: "terminal", preset: "codex", build: codex), "ov-416")
        #expect(!AgentConversation.isAgentInATerminal(paneMode: "terminal", preset: "codex", build: old), "a runner from before codex's view")
        #expect(!AgentConversation.isAgentInATerminal(paneMode: "changes", preset: "codex", build: codex))
        #expect(!AgentConversation.isAgentInATerminal(paneMode: "terminal", preset: "shell", build: codex))
        #expect(AgentConversation.agentName(preset: "codex:gpt-5") == "Codex")
        #expect(AgentConversation.agentName(preset: "claude") == "Claude")
        #expect(AgentConversation.pressesKeys(preset: "claude"))
        #expect(!AgentConversation.pressesKeys(preset: "codex"), "the runner presses no keys in codex")
    }

    /// A codex pane's words name codex, its own refusals included (ov-416).
    @Test func codexsRefusalsNameCodex() {
        let said = { (what: String) -> String in
            guard case .said(let words) = AgentConversation.issue(for: .refused(what: what), agent: "Codex") else { return "" }
            return words
        }
        for what in ["busy", "left_at_shell", "unconfirmed", "not_running", "unconfirmable", "picker", "too_tall", "command"] {
            #expect(said(what).contains("Codex"), "\(what): \(said(what))")
            #expect(!said(what).contains("Claude"), "\(what)")
        }
        #expect(said("picker").contains("@"))
        #expect(AgentConversation.issue(for: .refused(what: "picker")) != .said("The message wasn’t sent."))
        #expect(AgentConversation.handoff("Codex") == "Codex is showing something only the terminal can.")
        #expect(AgentConversation.askTitle(
            AgentRow.Ask(kind: "Permission", text: "ls", tool: "Bash", askedMs: 1, answered: false), agent: "Codex") == "Codex is asking for permission")
    }

    /// R-27: a phone opens the conversation until the pane is switched, and
    /// remembers each pane's last view.
    @Test func eachPaneRemembersItsViewAndStartsOnTheConversation() throws {
        let defaults = try #require(ScratchDefaults.suite("conversation-\(UUID())"))
        #expect(AgentConversation.showsConversation("a", defaults: defaults))
        AgentConversation.remember(conversation: false, for: "a", defaults: defaults)
        #expect(!AgentConversation.showsConversation("a", defaults: defaults))
        #expect(AgentConversation.showsConversation("b", defaults: defaults), "one pane's choice is its own")
        AgentConversation.remember(conversation: true, for: "a", defaults: defaults)
        #expect(AgentConversation.showsConversation("a", defaults: defaults))
    }

    @Test func aDraftIsOneLineAndCommandsGoToTheTerminal() {
        #expect(AgentConversation.flattened("one\ntwo\r\nthree\rfour") == "one two three four")
        #expect(AgentConversation.isCommand("/compact"))
        #expect(AgentConversation.isCommand("!ls"))
        #expect(!AgentConversation.isCommand("Tidy the parser"))
        #expect(!AgentConversation.isCommand(""))
    }

    /// R-28 and the dialog: a draft in the terminal's box refuses with Show
    /// Terminal, and a dialog hands off. A send that may have arrived never
    /// says it wasn't sent.
    @Test func refusalsMapAsTheMacsDo() {
        #expect(AgentConversation.issue(for: .refused(what: "dialog")) == .handoff)
        #expect(AgentConversation.issue(for: .refused(what: "prompt")) == .handoff)
        #expect(AgentConversation.issue(for: .refused(what: "draft")) == .draftInTerminal)
        #expect(AgentConversation.issue(for: .refused(what: "too_long")) == .said(AgentConversation.tooLong))
        #expect(AgentConversation.issue(for: .refused(what: nil)) == .said("The message wasn’t sent."))
        #expect(AgentConversation.issue(for: .timedOut) == .said(AgentConversation.mayHaveBeenSent))
        #expect(AgentConversation.issue(for: .lost(notSent: false)) == .said(AgentConversation.mayHaveBeenSent))
        #expect(AgentConversation.issue(for: .lost(notSent: true)) == .said("The runner isn’t connected, so the message wasn’t sent."))
        #expect(
            AgentConversation.issue(for: .refused(what: nil, word: "scope-denied"))
                == .said("This device can’t send messages to this runner."))
        #expect(AgentConversation.issue(for: .refused(what: "busy", word: "resource-conflict")) != .said("The message wasn’t sent."))
        // A person's send holds three seconds after a key (ov-407); the copy names that.
        #expect(
            AgentConversation.issue(for: .refused(what: "typing", word: "resource-conflict"))
                == .said("Someone typed in the terminal in the last 3 seconds, so the message wasn’t sent. Try again once they stop."))
    }

    @Test func onlyARunningPaneIsTalkedTo() {
        #expect(AgentConversation.isRunning(state: "running"))
        #expect(AgentConversation.isRunning(state: "starting"))
        #expect(!AgentConversation.isRunning(state: "exited"))
        #expect(!AgentConversation.isRunning(state: "stopped"))
    }

    @Test func aQueuedEchoSettlesOnceTheTranscriptShowsIt() {
        let rows = [
            AgentRow(id: "q", ord: 1, rev: 1, kind: .queued(.init(text: "first", state: "Waiting", atMs: nil))),
            AgentRow(id: "t", ord: 2, rev: 1, kind: .turn(.init(
                prompt: "second", origin: "Queued", startedMs: nil, endedMs: nil, durationMs: nil, outcome: nil,
                backgroundRunning: 0, activity: nil))),
        ]
        #expect(AgentConversation.unsettled(["first", "second", "third"], newest: rows) == ["third"])
    }

    @Test func noticeTurnsAndWords() {
        let turn = { (origin: String) in
            AgentRow.Turn(
                prompt: "Agent \"Count\" finished", origin: origin, startedMs: nil, endedMs: nil, durationMs: 65_000,
                outcome: .finished, backgroundRunning: 0, activity: nil)
        }
        #expect(AgentConversation.isNotice(turn("Notification")))
        #expect(AgentConversation.isNotice(turn("System")))
        #expect(!AgentConversation.isNotice(turn("Typed")))
        #expect(!AgentConversation.isNotice(turn("Queued")))
        #expect(AgentConversation.noticeText(turn("Notification")) == "Agent Count finished")
        #expect(AgentConversation.outcome(turn("Typed")) == "Took 1:05")
        #expect(AgentConversation.short(ms: 7_384_000) == "2:03:04")
        #expect(AgentConversation.agentType("general-purpose") == "General purpose")
        #expect(AgentConversation.toolsLabel(14) == "14 tools" && AgentConversation.toolsLabel(1) == "1 tool")
        #expect(AgentConversation.toolsLabel(0) == nil, "an unknown count is hidden, not '0 tools'")
        #expect(AgentConversation.queuedLabel("Waiting") == "Queued")
    }

    /// The field takes a draft set from outside (a send clearing it, a
    /// suggestion taken), but never while an input method is composing.
    @Test func theFieldTakesTheDraftOnlyOutsideAComposition() {
        #expect(AgentConversation.fieldTakesDraft("", showing: "Sent words", composing: false))
        #expect(AgentConversation.fieldTakesDraft("Try this", showing: "", composing: false))
        #expect(!AgentConversation.fieldTakesDraft("same", showing: "same", composing: false))
        #expect(!AgentConversation.fieldTakesDraft("に", showing: "にほ", composing: true))
        #expect(!AgentConversation.fieldTakesDraft("", showing: "か", composing: true))
    }

    /// ov-492: SwiftUI delivered the draft one keystroke behind the field, and
    /// the field took it, losing the typed character ("Anthing").
    @Test func aDraftLaggingTheTypingIsNeverWrittenBack() {
        var echo = AgentConversation.DraftEcho()
        for typed in ["A", "An", "Ant", "Anth", "Anthi"] { echo.typed(typed) }
        // The field holds "Anthi"; the draft is two keystrokes old.
        #expect(!AgentConversation.fieldTakesDraft("Anth", showing: "Anthi", composing: false, echo: echo))
        #expect(!AgentConversation.fieldTakesDraft("A", showing: "Anthi", composing: false, echo: echo))
        // A send clearing the box is from outside, and starts fresh.
        #expect(AgentConversation.fieldTakesDraft("", showing: "Anthi", composing: false, echo: echo))
        echo.typed("")
        echo.typed("Hi")
        #expect(AgentConversation.fieldTakesDraft("", showing: "Hi", composing: false, echo: echo))
        echo.took("")
        #expect(AgentConversation.fieldTakesDraft("Anth", showing: "x", composing: false, echo: echo))
    }
}
