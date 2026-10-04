import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's field (ov-214, slice 4; ov-264): a search bar. Typing
/// searches straight away, with no `/`; nothing is ever sent from it; the
/// keyboard through the results; and input methods.
@MainActor
struct TitleConsoleTests {
    // MARK: - Modes

    @Test("At rest until opened; ⌘K shows the activity, ⌘P the recent terminals, and typing searches in both")
    func modes() {
        var c = TitleConsole()
        #expect(c.mode == .rest)
        c.open(recents: false)
        #expect(c.mode == .activity && c.text.isEmpty)
        c.edit("ov-1")
        #expect(c.mode == .find && c.query == "ov-1", "typing searches, no slash needed")
        c.edit("/ov-1")
        #expect(c.query == "/ov-1", "a slash is only a character now")
        c.close()
        #expect(c.mode == .rest && c.text.isEmpty, "a search isn't kept")

        var p = TitleConsole()
        p.open(recents: true)
        #expect(p.mode == .find && p.query.isEmpty, "⌘P lists the recent terminals")
        p.toggle(recents: true)
        #expect(p.mode == .rest, "⌘P again closes it")
        p.toggle(recents: false)
        #expect(p.mode == .activity)
        p.toggle(recents: true)
        #expect(p.mode == .find, "⌘P on ⌘K's field switches to the recent terminals")
        p.toggle(recents: false)
        p.toggle(recents: false)
        #expect(p.mode == .rest, "⌘K again closes it")
    }

    @Test("Moving to another workspace closes the field")
    func anotherWorkspaceCloses() {
        var c = TitleConsole()
        c.enter(workspace: "mac|billing")
        c.open(recents: false)
        c.edit("bil")
        c.enter(workspace: "mac|billing")
        #expect(c.isOpen, "the same workspace again changes nothing")
        c.enter(workspace: "mac|shop")
        #expect(c.mode == .rest && c.text.isEmpty && c.workspace == "mac|shop")
    }

    @Test("The field asks for nothing to be sent: its placeholder and every Return find")
    func searchOnly() {
        #expect(TitleConsole.placeholder == "Find a workspace, task, terminal, or file")
        var c = TitleConsole()
        c.open(recents: false)
        c.edit("ship the invoice fix")
        #expect(c.submit(results: 0) == nil, "with no results, Return does nothing")
        #expect(c.submit(results: 2) == 0, "with results, Return opens the first")
    }

    @Test("A message starting with a dash goes after -- to an agent's composer")
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
        c.open(recents: true)
        c.edit("w")
        c.move(1, count: 3)
        c.move(1, count: 3)
        #expect(c.highlight(opening: 0) == 2)
        c.move(1, count: 3)
        #expect(c.highlight(opening: 0) == 0, "wraps")
        c.move(-1, count: 3)
        #expect(c.highlight(opening: 0) == 2)
        #expect(c.submit(results: 3) == 2)
        c.edit("wo")
        #expect(c.highlight(opening: 0) == 0)
        #expect(c.submit(results: 0) == nil, "nothing to open")
    }

    @Test("⌘P then Return goes to the other terminal: the highlight starts past the one you're in")
    func openingSkipsTheCurrentTerminal() {
        let recent = [
            PaletteEntry(id: "a", action: .openTerminal(worktree: "w", terminal: "here"), title: "here", detail: "", kind: "Terminal"),
            PaletteEntry(id: "b", action: .openTerminal(worktree: "w", terminal: "there"), title: "there", detail: "", kind: "Terminal"),
        ]
        let actions = TitleConsoleActions(find: { _ in recent }, current: "here")
        var c = TitleConsole()
        c.open(recents: true)
        #expect(actions.opening(c, recent) == 1)
        #expect(c.submit(results: 2, opening: actions.opening(c, recent)) == 1)
        c.move(1, count: 2, opening: 1)
        #expect(c.highlight(opening: 1) == 0, "↓ from the opening wraps to the first")
        c.edit("the")
        #expect(actions.opening(c, recent) == 0, "typed, the best match leads")
    }

    @Test("Typing in ⌘K's field, with no slash, runs the highlighted result and closes the field")
    func findRuns() {
        final class Ran { var actions: [PaletteAction] = [] }
        let ran = Ran()
        let model = TitleConsoleModel()
        let actions = TitleConsoleActions(
            find: { query in query == "w" ? Self.entries(2) : [] },
            run: { ran.actions.append($0) })
        model.console.open(recents: false)
        model.console.edit("w")
        model.console.move(1, count: actions.entries(model.console).count)
        actions.submit(model)
        #expect(ran.actions == [.openWorktree("w1")])
        #expect(model.console.mode == .rest)
    }

    // MARK: - Losing the keyboard

    @Test("Losing the keyboard closes the field, unless it went to a click in the field's own panel")
    func losingTheKeyboard() {
        let model = TitleConsoleModel()
        model.console.open(recents: true)
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
        model.console.open(recents: false)
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
