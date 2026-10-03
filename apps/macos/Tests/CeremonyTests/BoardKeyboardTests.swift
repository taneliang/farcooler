import Testing

@testable import Far_Cooler

/// Where Focus, the selection and the keyboard go for each command (ov-89
/// review, ov-92): `WorkspaceNavigation.boardStep`, which `ContentView` only
/// does.
struct BoardKeyboardTests {
    private typealias Nav = WorkspaceNavigation
    private typealias State = Nav.BoardState
    private typealias Step = Nav.BoardStep

    private static let commands: [Nav.BoardCommand] = [
        .conversation, .board, .task, .toggleFocus, .close, .choose(glance: true), .choose(glance: false),
    ]

    /// Every state the window can be in: something open or not, Focus or
    /// not (only with something open), a navigator or not, the navigator
    /// holding the keyboard or not (only where it's drawn).
    private static var states: [State] {
        var all: [State] = []
        for opened in [false, true] {
            for focus in [false, true] where !focus || opened {
                for hasNavigator in [false, true] {
                    for onBoard in [false, true] {
                        let state = State(opened: opened, focus: focus, onBoard: onBoard, hasNavigator: hasNavigator)
                        if !onBoard || state.boardInSight { all.append(state) }
                    }
                }
            }
        }
        return all
    }

    /// The state after `step`, for `command` from `state`.
    private static func after(_ state: State, _ command: Nav.BoardCommand, _ step: Step) -> State {
        var opened = state.opened
        if command == .close || step.selectsOrchestrator { opened = false }
        let onBoard: Bool =
            switch step.keyboard {
            case .board: true
            case .unchanged: state.onBoard
            default: false
            }
        return State(opened: opened, focus: step.focus, onBoard: onBoard, hasNavigator: state.hasNavigator)
    }

    /// The rule under all of it: whatever the command, from whatever state,
    /// the navigator ends up holding the keyboard only where it's drawn.
    /// (Fails with ⌥⌘2 leaving Focus as it was: the keyboard on a navigator
    /// put away.)
    @Test("The keyboard never stays on a navigator that isn't drawn")
    func theKeyboardNeverStaysOnAHiddenNavigator() {
        for state in Self.states {
            for command in Self.commands where command != .choose(glance: true) || state.onBoard {
                let step = Nav.boardStep(command, from: state)
                let next = Self.after(state, command, step)
                #expect(!next.onBoard || next.boardInSight, "\(String(describing: command)) from \(String(describing: state))")
            }
        }
    }

    /// Focus with nothing open has nothing to act on, and changes nothing.
    @Test("A command with nothing to act on changes nothing")
    func noOpsAreNoOps() {
        for state in Self.states where !state.opened {
            #expect(
                Nav.boardStep(.toggleFocus, from: state) == Step(focus: state.focus, keyboard: .unchanged),
                "from \(state)")
        }
        let bare = State(opened: true, hasNavigator: false)
        #expect(Nav.boardStep(.board, from: bare) == Step(focus: false, keyboard: .unchanged))
    }

    /// Each command's own rule (ov-92): ⌥⌘1 selects the orchestrator and
    /// gives it the keyboard; ⌥⌘2 the navigator, out of Focus; ⌥⌘3 the main
    /// area, whatever it shows; a row chosen and a close keep the keyboard
    /// on the navigator.
    @Test("Each command takes the selection and the keyboard where it says")
    func eachCommand() {
        let task = State(opened: true, onBoard: true)
        let focused = State(opened: true, focus: true)
        let orchestrator = State(opened: false, onBoard: true)
        #expect(Nav.boardStep(.conversation, from: task) == Step(focus: false, keyboard: .conversation, selectsOrchestrator: true))
        #expect(Nav.boardStep(.conversation, from: focused).selectsOrchestrator)
        #expect(Nav.boardStep(.board, from: focused) == Step(focus: false, keyboard: .board))
        #expect(Nav.boardStep(.task, from: orchestrator) == Step(focus: false, keyboard: .main))
        #expect(Nav.boardStep(.task, from: focused) == Step(focus: true, keyboard: .main))
        #expect(Nav.boardStep(.choose(glance: false), from: task) == Step(focus: false, keyboard: .board))
        #expect(Nav.boardStep(.choose(glance: true), from: orchestrator) == Step(focus: false, keyboard: .board))
        #expect(Nav.boardStep(.close, from: task) == Step(focus: false, keyboard: .board))
        #expect(Nav.boardStep(.close, from: State(opened: true, hasNavigator: false)).keyboard == .main)
        // Into Focus the keyboard leaves the navigator for what's opened;
        // out, it stays where it is.
        #expect(Nav.boardStep(.toggleFocus, from: task) == Step(focus: true, keyboard: .opened))
        #expect(Nav.boardStep(.toggleFocus, from: focused) == Step(focus: false, keyboard: .unchanged))
    }
}
