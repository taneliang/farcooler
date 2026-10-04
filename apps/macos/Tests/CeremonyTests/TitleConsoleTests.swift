import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's field (ov-214, slice 4): its modes, that only Return
/// sends and never an empty message, that a message belongs to the
/// workspace it was written in, `/` and ⌘P routing to find, the keyboard
/// through the results, and input methods.
@MainActor
struct TitleConsoleTests {
    // MARK: - Modes

    @Test("At rest until opened; ⌘K asks, ⌘P finds with / typed, Esc returns to rest")
    func modes() {
        var c = TitleConsole()
        #expect(c.mode == .rest)
        c.open(finding: false)
        #expect(c.mode == .ask && c.text.isEmpty)
        c.edit("/ov-1")
        #expect(c.mode == .find && c.query == "ov-1")
        c.edit("a/b")
        #expect(c.mode == .ask, "a slash later on is part of a message")
        c.close()
        #expect(c.mode == .rest)

        var p = TitleConsole()
        p.open(finding: true)
        #expect(p.mode == .find && p.text == "/" && p.query.isEmpty)
        p.toggle(finding: true)
        #expect(p.mode == .rest, "⌘P again closes it")
        p.toggle(finding: false)
        #expect(p.mode == .ask)
        p.toggle(finding: false)
        #expect(p.mode == .rest, "⌘K again closes it")
    }

    @Test("A message being written survives ⌘P and Esc, and comes back with ⌘K; a search doesn't")
    func draftsSurvive() {
        var c = TitleConsole()
        c.open(finding: false)
        c.edit("land ov-214 after the rebase")
        c.open(finding: true)
        #expect(c.text == "/")
        c.edit("/ov-2")
        c.open(finding: false)
        #expect(c.text == "land ov-214 after the rebase")
        c.close()
        c.open(finding: false)
        #expect(c.text == "land ov-214 after the rebase", "Esc kept it")
        c.open(finding: true)
        c.edit("/ov-9")
        c.close()
        #expect(c.text == "land ov-214 after the rebase", "a search closed gives the message back")
    }

    // MARK: - Whose message

    @Test("A message stays with the workspace it was written in, and never reaches another's orchestrator")
    func messagesBelongToTheirWorkspace() {
        var c = TitleConsole()
        c.enter(workspace: "mac|billing")
        c.open(finding: false)
        c.edit("ship the invoice fix")
        c.close()
        c.enter(workspace: "mac|shop")
        #expect(c.mode == .rest && c.text.isEmpty, "the other workspace starts empty")
        c.open(finding: false)
        #expect(c.submit(results: 0, refusal: nil) == .refuse, "nothing of billing's to send here")
        c.edit("shop's own")
        c.enter(workspace: "mac|billing")
        #expect(c.text == "ship the invoice fix" && c.mode == .rest, "billing's comes back, closed")
        c.enter(workspace: "mac|shop")
        #expect(c.text == "shop's own")
    }

    @Test("The field names whose orchestrator it asks")
    func namesTheRecipient() {
        #expect(TitleConsole.askPlaceholder(recipient: "Billing") == "Ask Billing’s orchestrator, or type / to find")
        #expect(TitleConsole.sendTitle(recipient: "Billing") == "To Billing’s orchestrator")
        #expect(TitleConsole.askPlaceholder(recipient: nil) == "Ask the orchestrator, or type / to find")
    }

    // MARK: - Sending

    @Test("Typing never sends, even a pasted newline; only Return does, once")
    func onlyReturnSends() {
        var c = TitleConsole()
        c.open(finding: false)
        for text in ["l", "la", "land it\n", "land it\nnow"] {
            c.edit(text)
            #expect(!c.sending, "typing “\(text)” started a send")
        }
        #expect(c.submit(results: 0, refusal: nil) == .send("land it\nnow", id: 1))
        #expect(c.sending)
        #expect(c.submit(results: 0, refusal: nil) == .none, "a second Return while sending sends nothing")
        c.sent(id: 1, .sent)
        #expect(c.mode == .rest && c.text.isEmpty)
    }

    @Test("An empty or whitespace-only message is refused, with a sentence", arguments: ["", " ", "\n\t  "])
    func emptyIsRefused(text: String) {
        var c = TitleConsole()
        c.open(finding: false)
        c.edit(text)
        #expect(c.submit(results: 0, refusal: nil) == .refuse)
        #expect(!c.sending)
        #expect(c.notice == TitleConsole.empty)
        c.edit(text + "x")
        #expect(c.notice == nil, "typing clears it")
    }

    @Test("With nobody to send to, nothing is sent and the field says why")
    func refusedWithoutARecipient() {
        var c = TitleConsole()
        c.open(finding: false)
        c.edit("hello")
        #expect(c.submit(results: 0, refusal: TitleConsoleRecipient.noOrchestrator) == .refuse)
        #expect(c.notice == TitleConsoleRecipient.noOrchestrator && !c.sending)
    }

    @Test("Refused keeps the message and says why; left lets it go and says where")
    func outcomes() {
        var c = TitleConsole()
        c.open(finding: false)
        c.edit("hello")
        _ = c.submit(results: 0, refusal: nil)
        c.sent(id: 1, .refused("busy"))
        #expect(c.mode == .ask && c.text == "hello" && c.notice == "busy" && !c.sending)
        _ = c.submit(results: 0, refusal: nil)
        c.sent(id: 2, .left("in its box"))
        #expect(c.text.isEmpty && c.notice == "in its box" && c.mode == .ask)
        c.sent(id: 2, .sent)
        #expect(c.notice == "in its box", "an answer is heard once")
    }

    @Test("A send can be walked away from: Esc closes, a timeout says so, and a late success isn't sent twice")
    func stuckSends() {
        var c = TitleConsole()
        c.enter(workspace: "a")
        c.open(finding: false)
        c.edit("hello")
        _ = c.submit(results: 0, refusal: nil)
        c.close()
        #expect(c.mode == .rest && !c.sending, "Esc works while a send is out")
        c.sent(id: 1, .sent)
        #expect(c.text.isEmpty, "it went, so it's not kept to be sent again")

        c.open(finding: false)
        c.edit("again")
        _ = c.submit(results: 0, refusal: nil)
        c.timedOut(id: 2)
        #expect(!c.sending && c.notice == TitleConsole.timedOut && c.text == "again")
        c.sent(id: 2, .refused("busy"))
        #expect(c.text == "again" && c.notice == TitleConsole.timedOut, "a late refusal changes nothing")

        _ = c.submit(results: 0, refusal: nil)
        c.enter(workspace: "b")
        c.sent(id: 3, .sent)
        c.enter(workspace: "a")
        #expect(c.text.isEmpty, "a message that went while you were elsewhere isn't kept")
    }

    @Test("Who the field can send to, and by which route")
    func recipients() {
        func seat(state: String, mode: String?, chat: Bool?) -> Terminal {
            var t = Terminal(id: "o", short: "o", title: "o", preset: "claude", state: state, epoch: 0)
            t.paneMode = mode
            t.chatCapable = chat
            t.role = "orchestrator"
            return t
        }
        #expect(TitleConsoleRecipient.refusal(seat: nil) == TitleConsoleRecipient.noOrchestrator)
        let chat = seat(state: "running", mode: "agent", chat: true)
        #expect(TitleConsoleRecipient.refusal(seat: chat) == nil)
        #expect(TitleConsoleRecipient.route(chat) == .composer)
        // The owner's setup: claude in a terminal, chat-capable or adopted.
        for terminal in [seat(state: "running", mode: "terminal", chat: true), seat(state: "running", mode: nil, chat: false)] {
            #expect(TitleConsoleRecipient.refusal(seat: terminal) == nil)
            #expect(TitleConsoleRecipient.route(terminal) == .terminal, "never the composer's channel, which it lacks")
        }
        #expect(TitleConsoleRecipient.refusal(seat: seat(state: "LOST", mode: "agent", chat: true)) == TitleConsoleRecipient.notRunning)
        #expect(TitleConsoleRecipient.refusal(seat: seat(state: "running", mode: "changes", chat: nil)) == TitleConsoleRecipient.cantTake)
    }

    @Test("The runner's refusals become this app's sentences, never its own text")
    func tellRefusals() {
        func outcome(_ what: String?, code: String = "resource-conflict") -> TitleConsole.Outcome {
            TellRefusal.outcome(message: "error: something internal\ncode: \(code)" + (what.map { "\nwhat: \($0)" } ?? ""))
        }
        for word in ["busy", "prompt", "draft", "typing", "not_an_agent", "unfamiliar", "unproven", "too_long", "not_running"] {
            guard case .refused(let why) = outcome(word) else {
                Issue.record("\(word) let the message go")
                continue
            }
            #expect(!why.contains("internal") && !why.contains(word), "\(word): \(why)")
        }
        for word in ["paste_left", "left_at_shell"] {
            guard case .left = outcome(word) else {
                Issue.record("\(word) kept a message that's already in the pane")
                continue
            }
        }
        #expect(outcome(nil, code: "capability-unsupported") == .refused("This runner needs an update to take messages from the title bar."))
        #expect(outcome(nil) == .refused(TitleConsole.sendFailed))
        #expect(TellRefusal.outcome(message: nil) == .refused(TitleConsole.sendFailed))
    }

    // MARK: - The route

    @Test("Return reaches the send route once, with the trimmed message; a refusal never reaches it")
    func theRoute() async {
        final class Sent { var messages: [String] = [] }
        let sent = Sent()
        let model = TitleConsoleModel()
        var actions = TitleConsoleActions(send: { text in
            sent.messages.append(text)
            return .sent
        })
        model.console.open(finding: false)
        model.console.edit("   ")
        actions.submit(model)
        model.console.edit("  ship it  ")
        actions.refusal = { TitleConsoleRecipient.noOrchestrator }
        actions.submit(model)
        #expect(sent.messages.isEmpty, "a refused send types nothing")
        actions.refusal = { nil }
        actions.submit(model)
        actions.submit(model)
        for _ in 0..<10 { await Task.yield() }
        #expect(sent.messages == ["ship it"])
        #expect(model.console.mode == .rest)
    }

    @Test("A message starting with a dash goes after --, for the composer and the field alike")
    func dashesAreWords() {
        let composer = AgentAction.sendArguments(terminal: "t1", text: "-x --help", images: ["/tmp/a.png"])
        #expect(composer == ["terminal", "agent-prompt", "t1", "--image", "/tmp/a.png", "--json", "--", "-x --help"])
        #expect(AgentAction.editQueued(id: "q", text: "--help").arguments(terminal: "t1").suffix(2) == ["--", "--help"])
    }

    // MARK: - Finding

    private static func entries(_ n: Int) -> [PaletteEntry] {
        (0..<n).map { PaletteEntry(id: "e\($0)", action: .openWorktree("w\($0)"), title: "Entry \($0)", detail: "", kind: "Worktree") }
    }

    @Test("↑ and ↓ walk the results, wrapping; typing starts over; Return opens the highlighted one")
    func keyboardThroughResults() {
        var c = TitleConsole()
        c.open(finding: true)
        c.edit("/w")
        c.move(1, count: 3)
        c.move(1, count: 3)
        #expect(c.highlight(opening: 0) == 2)
        c.move(1, count: 3)
        #expect(c.highlight(opening: 0) == 0, "wraps")
        c.move(-1, count: 3)
        #expect(c.highlight(opening: 0) == 2)
        #expect(c.submit(results: 3, refusal: nil) == .open(2))
        c.edit("/wo")
        #expect(c.highlight(opening: 0) == 0)
        #expect(c.submit(results: 0, refusal: nil) == .none, "nothing to open")
    }

    @Test("⌘P then Return goes to the other terminal: the highlight starts past the one you're in")
    func openingSkipsTheCurrentTerminal() {
        let recent = [
            PaletteEntry(id: "a", action: .openTerminal(worktree: "w", terminal: "here"), title: "here", detail: "", kind: "Terminal"),
            PaletteEntry(id: "b", action: .openTerminal(worktree: "w", terminal: "there"), title: "there", detail: "", kind: "Terminal"),
        ]
        let actions = TitleConsoleActions(find: { _ in recent }, current: "here")
        var c = TitleConsole()
        c.open(finding: true)
        #expect(actions.opening(c, recent) == 1)
        #expect(c.submit(results: 2, opening: actions.opening(c, recent), refusal: nil) == .open(1))
        c.move(1, count: 2, opening: 1)
        #expect(c.highlight(opening: 1) == 0, "↓ from the opening wraps to the first")
        c.edit("/the")
        #expect(actions.opening(c, recent) == 0, "typed, the best match leads")
    }

    @Test("Find runs the highlighted result, closes the field, and never sends")
    func findRuns() {
        final class Ran { var actions: [PaletteAction] = [] }
        let ran = Ran()
        let model = TitleConsoleModel()
        let actions = TitleConsoleActions(
            find: { query in query == "w" ? Self.entries(2) : [] },
            run: { ran.actions.append($0) },
            send: { _ in
                Issue.record("find sent a message")
                return .sent
            })
        model.console.open(finding: true)
        model.console.edit("/w")
        model.console.move(1, count: actions.entries(model.console).count)
        actions.submit(model)
        #expect(ran.actions == [.openWorktree("w1")])
        #expect(model.console.mode == .rest)
    }

    // MARK: - Losing the keyboard

    @Test("Losing the keyboard closes the field, unless it went to a click in the field's own panel")
    func losingTheKeyboard() {
        let model = TitleConsoleModel()
        model.console.open(finding: true)
        model.pointerInPanel = true
        model.fieldEndedEditing()
        #expect(model.console.isOpen, "a click on a result keeps it open long enough to land")
        model.pointerInPanel = false
        model.fieldEndedEditing()
        #expect(!model.console.isOpen)
    }

    /// In a real window: the field's text view gives up the keyboard, as a
    /// click elsewhere makes it, and the field closes, or doesn't.
    @Test("In a real window, the field closes when the keyboard leaves it, and not for its own panel", arguments: [false, true])
    func losingTheKeyboardInAWindow(pointerInPanel: Bool) async throws {
        let model = TitleConsoleModel()
        model.console.open(finding: false)
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: 400, height: 80), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = NSHostingView(rootView: TitleConsoleField(model: model, actions: TitleConsoleActions()))
        window.orderFrontRegardless()
        for _ in 0..<10 {
            window.contentView?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(window.firstResponder is PaletteTextView, "the field took the keyboard as it opened")
        model.pointerInPanel = pointerInPanel
        window.makeFirstResponder(nil)
        for _ in 0..<5 { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.console.isOpen == pointerInPanel)
    }

    // MARK: - Input methods

    private static func key(_ code: UInt16, _ characters: String, in window: NSWindow) -> NSEvent {
        NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber,
            context: nil, characters: characters, charactersIgnoringModifiers: characters, isARepeat: false,
            keyCode: code)!
    }

    @Test("While an input method composes, Return, Esc, Tab and the arrows are its, through keyDown itself")
    func keysWhileComposing() {
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: 300, height: 60), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let view = PaletteTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 30))
        window.contentView = view
        window.makeFirstResponder(view)
        var heard: [String] = []
        view.onSubmit = { heard.append("submit") }
        view.onCancel = { heard.append("cancel") }
        view.onMove = { heard.append("move \($0)") }
        let keys: [(UInt16, String)] = [(36, "\r"), (53, "\u{1b}"), (48, "\t"), (126, "\u{F700}"), (125, "\u{F701}")]
        for (code, characters) in keys {
            view.setMarkedText(
                "にほ", selectedRange: NSRange(location: 2, length: 0),
                replacementRange: NSRange(location: NSNotFound, length: 0))
            #expect(view.hasMarkedText())
            view.keyDown(with: Self.key(code, characters, in: window))
            #expect(heard.isEmpty, "key \(code) mid-composition reached the field: \(heard)")
            view.unmarkText()
        }
        // And with nothing composing, the same keys are the field's.
        for (code, characters) in keys { view.keyDown(with: Self.key(code, characters, in: window)) }
        #expect(heard == ["submit", "cancel", "move next", "move up", "move down"])
    }
}
