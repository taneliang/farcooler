import AgentKit
import AppKit
import Testing

@testable import Far_Cooler

/// The title bar's field (ov-214, slice 4): its modes, that only Return
/// sends and never an empty message, `/` and ⌘P routing to find, and the
/// keyboard through the results.
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

    // MARK: - Sending

    @Test("Typing never sends, even a pasted newline; only Return does, once")
    func onlyReturnSends() {
        var c = TitleConsole()
        c.open(finding: false)
        for text in ["l", "la", "land it\n", "land it\nnow"] {
            c.edit(text)
            #expect(!c.sending, "typing “\(text)” started a send")
        }
        #expect(c.submit(results: 0, refusal: nil) == .send("land it\nnow"))
        #expect(c.sending)
        #expect(c.submit(results: 0, refusal: nil) == .none, "a second Return while sending sends nothing")
        c.sent(failed: false)
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

    @Test("A failed send keeps the message and says so; nothing raw")
    func failedSendKeepsTheMessage() {
        var c = TitleConsole()
        c.open(finding: false)
        c.edit("hello")
        _ = c.submit(results: 0, refusal: nil)
        c.sent(failed: true)
        #expect(c.mode == .ask && c.text == "hello" && c.notice == TitleConsole.sendFailed && !c.sending)
    }

    @Test("Who the field can send to: a running orchestrator that takes a prompt")
    func recipients() {
        func seat(state: String, mode: String?, chat: Bool?) -> Terminal {
            var t = Terminal(id: "o", short: "o", title: "o", preset: "claude", state: state, epoch: 0)
            t.paneMode = mode
            t.chatCapable = chat
            return t
        }
        #expect(TitleConsoleRecipient.refusal(seat: nil) == TitleConsoleRecipient.noOrchestrator)
        #expect(TitleConsoleRecipient.refusal(seat: seat(state: "running", mode: "agent", chat: true)) == nil)
        #expect(TitleConsoleRecipient.refusal(seat: seat(state: "LOST", mode: "agent", chat: true)) == TitleConsoleRecipient.notRunning)
        #expect(TitleConsoleRecipient.refusal(seat: seat(state: "running", mode: nil, chat: false)) == TitleConsoleRecipient.cantTake)
    }

    // MARK: - The route

    @Test("Return reaches the send route once, with the trimmed message; a refusal never reaches it")
    func theRoute() async {
        final class Sent { var messages: [String] = [] }
        let sent = Sent()
        let model = TitleConsoleModel()
        var actions = TitleConsoleActions(send: { text in
            sent.messages.append(text)
            return nil
        })
        model.console.open(finding: false)
        model.console.edit("   ")
        actions.submit(model)
        model.console.edit("  ship it  ")
        actions.refusal = { TitleConsoleRecipient.noOrchestrator }
        actions.submit(model)
        actions.refusal = { nil }
        actions.submit(model)
        actions.submit(model)
        for _ in 0..<10 { await Task.yield() }
        #expect(sent.messages == ["ship it"])
        #expect(model.console.mode == .rest)
    }

    // MARK: - Finding

    private static func entries(_ n: Int) -> [PaletteEntry] {
        (0..<n).map { PaletteEntry(id: "e\($0)", action: .openWorktree("w\($0)"), title: "Entry \($0)", detail: "", kind: "Worktree") }
    }

    @Test("↑ and ↓ walk the results, wrapping; typing starts over at the top; Return opens the highlighted one")
    func keyboardThroughResults() {
        var c = TitleConsole()
        c.open(finding: true)
        c.edit("/w")
        c.move(1, count: 3)
        c.move(1, count: 3)
        #expect(c.highlight == 2)
        c.move(1, count: 3)
        #expect(c.highlight == 0, "wraps")
        c.move(-1, count: 3)
        #expect(c.highlight == 2)
        #expect(c.submit(results: 3, refusal: nil) == .open(2))
        c.edit("/wo")
        #expect(c.highlight == 0)
        #expect(c.submit(results: 0, refusal: nil) == .none, "nothing to open")
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
                return nil
            })
        model.console.open(finding: true)
        model.console.edit("/w")
        model.console.move(1, count: actions.entries(model.console).count)
        actions.submit(model)
        #expect(ran.actions == [.openWorktree("w1")])
        #expect(model.console.mode == .rest)
    }

    // MARK: - Input methods

    @Test("Return mid-composition confirms the input method's text and doesn't submit")
    func returnWhileComposing() {
        let view = PaletteTextView()
        var submitted = 0
        view.onSubmit = { submitted += 1 }
        view.setMarkedText("にほ", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(view.hasMarkedText())
        view.insertNewline(nil)
        #expect(submitted == 0)
        view.unmarkText()
        view.insertNewline(nil)
        #expect(submitted == 1)
    }
}
