import SwiftUI

/// What a keyboard shortcut asks the window to do.
///
/// Menu items live in the App scene and the state they act on lives in the
/// window, so they cannot call each other directly. A notification carries the
/// intent across that gap — which also means a shortcut and a click on the same
/// control end up in exactly one place, rather than two implementations that
/// drift.
enum AppCommand: String {
    case newTerminal
    case closeTerminal
    case nextTerminal
    case previousTerminal
    case nextAttention
    case newWorkspace
    case addRepository
    case reload
    case showShortcuts
    case search

    static let notification = Notification.Name("overnight.command")

    func post() {
        NotificationCenter.default.post(name: Self.notification, object: rawValue)
    }

    /// Jump to the nth terminal. Sent separately because the index is data, not
    /// a distinct command.
    static func selectIndex(_ index: Int) {
        NotificationCenter.default.post(
            name: Notification.Name("overnight.selectIndex"), object: index)
    }
}

/// The menu bar.
///
/// This IS the discoverability story. A shortcut that only exists in a cheat
/// sheet has to be looked up; one in the menu bar shows its key equivalent
/// where people already look, and is findable through Help's menu search
/// without knowing it exists.
struct OvernightCommands: Commands {
    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Terminal") { AppCommand.newTerminal.post() }
                .keyboardShortcut("t", modifiers: .command)
            Button("New Workspace…") { AppCommand.newWorkspace.post() }
                .keyboardShortcut("n", modifiers: [.command, .shift])
            Button("Add Repository…") { AppCommand.addRepository.post() }
                .keyboardShortcut("r", modifiers: [.command, .shift])
        }

        CommandGroup(after: .newItem) {
            Divider()
            // No Restart. A terminal is its process: restarting one is closing
            // it and opening another, which is ⌘W then ⌘T. A separate verb for
            // the same two steps is a concept to learn for nothing.
            Button("Close Terminal") { AppCommand.closeTerminal.post() }
                .keyboardShortcut("w", modifiers: .command)
        }

        CommandMenu("Terminal") {
            Button("Next Terminal") { AppCommand.nextTerminal.post() }
                .keyboardShortcut("]", modifiers: .command)
            Button("Previous Terminal") { AppCommand.previousTerminal.post() }
                .keyboardShortcut("[", modifiers: .command)
            Divider()
            // The one shortcut that is not a convention from elsewhere, because
            // nothing else has this idea: go straight to whatever is waiting on
            // you. It is the reason to open the app at all.
            Button("Next Needing Attention") { AppCommand.nextAttention.post() }
                .keyboardShortcut("n", modifiers: [.command, .control])
            Divider()
            ForEach(1...9, id: \.self) { n in
                Button("Terminal \(n)") { AppCommand.selectIndex(n - 1) }
                    .keyboardShortcut(KeyEquivalent(Character("\(n)")), modifiers: .command)
            }
        }

        CommandGroup(after: .toolbar) {
            // Search is navigation here, not a nicety: worktrees are unbounded
            // and typing is the fastest way to any of them, on any machine.
            Button("Find Worktree or Agent") { AppCommand.search.post() }
                .keyboardShortcut("f", modifiers: .command)
            Button("Reload Fleet") { AppCommand.reload.post() }
                .keyboardShortcut("0", modifiers: .command)
        }

        CommandGroup(replacing: .help) {
            Button("Keyboard Shortcuts") { AppCommand.showShortcuts.post() }
                .keyboardShortcut("/", modifiers: .command)
        }
    }
}

extension View {
    /// React to a menu command.
    func onCommand(_ perform: @escaping (AppCommand) -> Void) -> some View {
        onReceive(NotificationCenter.default.publisher(for: AppCommand.notification)) { note in
            guard let raw = note.object as? String, let command = AppCommand(rawValue: raw) else {
                return
            }
            perform(command)
        }
    }

    func onSelectIndex(_ perform: @escaping (Int) -> Void) -> some View {
        onReceive(
            NotificationCenter.default.publisher(for: Notification.Name("overnight.selectIndex"))
        ) { note in
            guard let index = note.object as? Int else { return }
            perform(index)
        }
    }
}
