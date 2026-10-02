import Testing

@testable import Far_Cooler

/// Where the popped-open board and the keyboard go for each command (ov-89
/// review): `WorkspaceNavigation.boardStep`, which `ContentView` only does.
struct BoardKeyboardTests {
    private typealias Nav = WorkspaceNavigation
    private typealias State = Nav.BoardState
    private typealias Step = Nav.BoardStep

    private static let commands: [Nav.BoardCommand] = [
        .conversation, .board, .task, .toggleFocus, .close, .dismissBoard,
        .choose(glance: true, opening: true), .choose(glance: false, opening: true),
        .choose(glance: false, opening: false),
    ]

    /// Every state the window can be in: collapsed or not, something open or
    /// not, the board popped or not (only collapsed), Focus or not (only
    /// with something open), the board holding the keyboard or not (only
    /// where it's drawn).
    private static var states: [State] {
        var all: [State] = []
        for collapsed in [false, true] {
            for opened in [false, true] {
                for popped in [false, true] where !popped || collapsed {
                    for focus in [false, true] where !focus || (opened && !popped) {
                        for onBoard in [false, true] {
                            let state = State(collapsed: collapsed, opened: opened, popped: popped, focus: focus, onBoard: onBoard)
                            if !onBoard || state.boardInSight { all.append(state) }
                        }
                    }
                }
            }
        }
        return all
    }

    /// The state after `step`, for `command` from `state`.
    private static func after(_ state: State, _ command: Nav.BoardCommand, _ step: Step) -> State {
        var opened = state.opened
        switch command {
        case .close: opened = false
        case .choose(_, let opening): opened = opening
        default: break
        }
        let onBoard: Bool =
            switch step.keyboard {
            case .board: true
            case .unchanged: state.onBoard
            default: false
            }
        return State(collapsed: state.collapsed, opened: opened, popped: step.popped, focus: step.focus, onBoard: onBoard)
    }

    /// The rule under all of it: whatever the command, from whatever state,
    /// the board ends up holding the keyboard only where it's drawn, and is
    /// popped open only where it's collapsed and not in Focus. (Fails with
    /// a click in the popped board that closes the task leaving the keyboard
    /// on the board, as `boardTakesKeyboard` did: review M2; with Focus
    /// keeping the board popped: M3.)
    @Test("The keyboard never stays on a board that isn't drawn")
    func theKeyboardNeverStaysOnAHiddenBoard() {
        for state in Self.states {
            // A glance is ↑ or ↓ in the board's list, so only from there.
            for command in Self.commands where command != .choose(glance: true, opening: true) || state.onBoard {
                let step = Nav.boardStep(command, from: state)
                let next = Self.after(state, command, step)
                #expect(!next.onBoard || next.boardInSight, "\(String(describing: command)) from \(String(describing: state))")
                #expect(!step.popped || (state.collapsed && !step.focus), "\(command) from \(state) popped a board that isn't drawn")
            }
        }
    }

    /// Commands with nothing to act on change nothing: ⌥⌘3 and Focus with
    /// nothing open, and putting away a board that isn't popped. (Fails
    /// with ⌥⌘3 clearing the popped board as it did: review M3.)
    @Test("A command with nothing to act on changes nothing")
    func noOpsAreNoOps() {
        for state in Self.states where !state.opened {
            for command in [Nav.BoardCommand.task, .toggleFocus] {
                #expect(
                    Nav.boardStep(command, from: state) == Step(popped: state.popped, focus: state.focus, keyboard: .unchanged),
                    "\(command) from \(state)")
            }
        }
        for state in Self.states where !state.popped {
            #expect(
                Nav.boardStep(.dismissBoard, from: state) == Step(popped: false, focus: state.focus, keyboard: .unchanged))
        }
    }

    /// Each command's own rule, in the states where it differs.
    @Test("Each command takes the board and the keyboard where it says")
    func eachCommand() {
        let side = State(collapsed: false, opened: true, onBoard: true)
        let strip = State(collapsed: true, opened: true)
        let popped = State(collapsed: true, opened: true, popped: true, onBoard: true)
        // A click in the popped board: put away, and the keyboard to what
        // fills the main area, the orchestrator if the click closed the task.
        #expect(Nav.boardStep(.choose(glance: false, opening: false), from: popped) == Step(popped: false, focus: false, keyboard: .main))
        #expect(Nav.boardStep(.choose(glance: false, opening: true), from: popped) == Step(popped: false, focus: false, keyboard: .opened))
        // A glance keeps it popped and the keyboard on it.
        #expect(Nav.boardStep(.choose(glance: true, opening: true), from: popped) == Step(popped: true, focus: false, keyboard: .board))
        // The sidebar keeps the keyboard for a click either way.
        #expect(Nav.boardStep(.choose(glance: false, opening: true), from: side) == Step(popped: false, focus: false, keyboard: .board))
        #expect(Nav.boardStep(.choose(glance: false, opening: false), from: side) == Step(popped: false, focus: false, keyboard: .board))
        // Closing: to the board where it's drawn, else the orchestrator.
        #expect(Nav.boardStep(.close, from: side).keyboard == .board)
        #expect(Nav.boardStep(.close, from: strip).keyboard == .main)
        #expect(Nav.boardStep(.close, from: popped) == Step(popped: true, focus: false, keyboard: .board))
        // ⌥⌘2: the sidebar; popped from the strip; put away pressed again.
        #expect(Nav.boardStep(.board, from: side) == Step(popped: false, focus: false, keyboard: .board))
        #expect(Nav.boardStep(.board, from: strip) == Step(popped: true, focus: false, keyboard: .board))
        #expect(Nav.boardStep(.board, from: popped) == Step(popped: false, focus: false, keyboard: .main))
        // ⌥⌘2 in Focus leaves it, and the board takes the keyboard.
        let focused = State(collapsed: true, opened: true, focus: true)
        #expect(Nav.boardStep(.board, from: focused) == Step(popped: true, focus: false, keyboard: .board))
        // ⌥⌘1 and ⌥⌘3 put a popped board away.
        #expect(Nav.boardStep(.conversation, from: popped) == Step(popped: false, focus: false, keyboard: .conversation))
        #expect(Nav.boardStep(.task, from: popped) == Step(popped: false, focus: false, keyboard: .opened))
        // Into Focus the keyboard goes to what's opened; out, it stays.
        #expect(Nav.boardStep(.toggleFocus, from: popped) == Step(popped: false, focus: true, keyboard: .opened))
        #expect(Nav.boardStep(.toggleFocus, from: focused) == Step(popped: false, focus: false, keyboard: .unchanged))
        // Put away from the board's keyboard, to the main area.
        #expect(Nav.boardStep(.dismissBoard, from: popped) == Step(popped: false, focus: false, keyboard: .main))
        var notOnIt = popped
        notOnIt.onBoard = false
        #expect(Nav.boardStep(.dismissBoard, from: notOnIt).keyboard == .unchanged)
    }
}
