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
    case openInEditor
    case reload
    case showShortcuts
    case about
    case search
    case commandPalette
    case toggleSidebar
    case diffNextHunk
    case diffPreviousHunk
    case diffNextFile
    case diffPreviousFile
    case diffNextCommit
    case diffPreviousCommit
    case diffFirstCommit
    case diffMarkRead

    static let notification = Notification.Name("farcooler.command")

    func post() {
        NotificationCenter.default.post(name: Self.notification, object: rawValue)
    }

    /// Jump to the nth terminal. Sent separately because the index is data, not
    /// a distinct command.
    static func selectIndex(_ index: Int) {
        NotificationCenter.default.post(
            name: Notification.Name("farcooler.selectIndex"), object: index)
    }
}

/// The menu bar.
///
/// This IS the discoverability story. A shortcut that only exists in a cheat
/// sheet has to be looked up; one in the menu bar shows its key equivalent
/// where people already look, and is findable through Help's menu search
/// without knowing it exists.
struct FarCoolerCommands: Commands {
    var body: some Commands {
        // About Far Cooler, saying which build this is.
        //
        // The standard panel shows CFBundleShortVersionString and
        // CFBundleVersion, which for a beta and the release it names are
        // identical — so the panel that exists to answer "what am I running"
        // could not. This one names the channel too, and the daemon it is
        // driving, which is the pair that has to match.
        CommandGroup(replacing: .appInfo) {
            Button("About Far Cooler") { AppCommand.about.post() }
        }

        // Greyed out rather than absent on a feedless build — `local` and any
        // hand-assembled bundle — so the item's presence never implies an
        // update channel that isn't there.
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") { Updates.shared.checkForUpdates() }
                .disabled(!Updates.shared.isEnabled)
        }

        CommandGroup(replacing: .newItem) {
            Button("New Terminal") { AppCommand.newTerminal.post() }
                .keyboardShortcut("t", modifiers: .command)
            // ⌘N, the plainest shortcut in the app, for the thing it is for.
            Button("New Task…") { AppCommand.newWorkspace.post() }
                .keyboardShortcut("n", modifiers: .command)
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
            Divider()
            // The keyboard half of the title bar's editor control, which until
            // now was the one thing in this app you could only reach with a
            // mouse. ⇧⌘E rather than a bare ⌘E: this opens another application
            // on a worktree, which is closer to ⇧⌘R's "act on the project" than
            // to anything a single-key chord does inside this window.
            //
            // Named for the act, not the editor. Which editor it opens is the
            // one the title bar button would open — see `Editors.preferred` —
            // and putting "Open in Zed" in the menu bar would make a fixed
            // string out of a choice that changes per runner.
            Button("Open in Editor") { AppCommand.openInEditor.post() }
                .keyboardShortcut("e", modifiers: [.command, .shift])
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

        // The menu is where a prefix binding becomes discoverable to someone who
        // has never used tmux, and where someone who has can confirm that the
        // key they already know is the key here. Every item names its ⌃B
        // sequence in the title, because a menu item with no key equivalent
        // teaches nothing about a prefix.
        CommandMenu("Layout") {
            // Splitting leads, because it is now the only way a layout grows and
            // the one thing every other item here presupposes.
            Button("Split Right  ⌃B %") { TileCommand.splitRight.post() }
            Button("Split Down  ⌃B \"") { TileCommand.splitDown.post() }
            Button("Move Pane Out  ⌃B !") { TileCommand.breakPane.post() }
            Divider()
            Button("Zoom Pane  ⌃B z") { TileCommand.zoom.post() }
                .keyboardShortcut("z", modifiers: [.command, .shift])
            Button("Next Arrangement  ⌃B space") { TileCommand.cycle.post() }
                .keyboardShortcut(.space, modifiers: [.command, .shift])
            // Double-clicking a divider evens out the two panes it separates.
            // This is the same idea for the whole layout, and it is here rather
            // than only on the divider because a gesture nobody has been told
            // about needs somewhere to be discovered.
            Button("Even Out Panes  ⌃B =") { TileCommand.evenPanes.post() }
            Menu("Arrangement") {
                ForEach(TilePreset.allCases) { preset in
                    Button(preset.label) { TileCommand.preset(preset).post() }
                }
            }
            Divider()
            // The prefix-less ones, and the only tiling bindings that get a real
            // key equivalent here: they are used constantly, and a menu item is
            // how someone finds out they exist.
            Button("Pane Left  ⌃H") { TileCommand.focus(.left).post() }
            Button("Pane Right  ⌃L") { TileCommand.focus(.right).post() }
            Button("Pane Above  ⌃K") { TileCommand.focus(.top).post() }
            Button("Pane Below  ⌃J") { TileCommand.focus(.bottom).post() }
            Divider()
            Button("Next Pane  ⌃B o") { TileCommand.focusNext.post() }
            Button("Previous Pane  ⌃B ;") { TileCommand.focusPrevious.post() }
            Divider()
            Button("New Layout  ⌃B c") { TileCommand.newGroup.post() }
            // A layout IS a tab here — the pill bar across the top of a worktree
            // is a tab strip, and these are the two verbs that walk it. So they
            // carry what every tabbed app on this machine binds for that:
            // ⇧⌘] and ⇧⌘[, one shift away from the ⌘]/⌘[ that walk terminals.
            //
            // ⌃⇥ and ⌃⇧⇥, the other half of the platform convention, are bound
            // in `PrefixMode.tabSwitch` instead of here. A ⌃ chord is one the
            // terminal has a claim on, so it is intercepted where every other
            // prefix-less ⌃ binding already is, rather than being taken from
            // the whole app by a menu key equivalent.
            Button("Next Layout  ⌃B n  ⌃⇥") { TileCommand.nextGroup.post() }
                .keyboardShortcut("]", modifiers: [.command, .shift])
            Button("Previous Layout  ⌃B p  ⌃⇧⇥") { TileCommand.previousGroup.post() }
                .keyboardShortcut("[", modifiers: [.command, .shift])
            Divider()
            // Not really a layout verb — nothing about the arrangement
            // changes — but it is scoped to the focused pane exactly the way
            // zoom and the splits are, and there is no chrome on the pane
            // itself left to put a button on.
            Button("Terminal ⟷ Chat  ⌃B a") { TileCommand.toggleAgentPane.post() }
        }

        // The diff pane had not one shortcut in this file, which made the only
        // keyboard-shaped thing in the app a pane you could only read with a
        // pointer. The phone needs a thumb-sized button to move through a
        // branch; a Mac needs a key, and the chevrons in the pane are the
        // fallback rather than the other way round.
        //
        // Three pairs on the same convention the rest of the app already
        // teaches: `[` and `]` walk a list, and the modifier says WHICH list.
        // ⌘ is terminals, ⇧⌘ is layouts, so ⌥⌘ is the files in a diff and ⌃⌘ is
        // the commits behind them — outermost list, outermost modifier.
        //
        // Hunks get the arrows instead, because a hunk is not a list you pick
        // from — it is the next place down the document, which is what ⌥⌘↓
        // reads as.
        //
        // These act on the FOCUSED pane, like every other pane-scoped command
        // here. Click into a diff and it answers; with a terminal focused they
        // do nothing, which is the same rule ⌘W and ⌃B z already keep.
        CommandMenu("Diff") {
            Button("Next Hunk") { AppCommand.diffNextHunk.post() }
                .keyboardShortcut(.downArrow, modifiers: [.command, .option])
            Button("Previous Hunk") { AppCommand.diffPreviousHunk.post() }
                .keyboardShortcut(.upArrow, modifiers: [.command, .option])
            Divider()
            Button("Next File") { AppCommand.diffNextFile.post() }
                .keyboardShortcut("]", modifiers: [.command, .option])
            Button("Previous File") { AppCommand.diffPreviousFile.post() }
                .keyboardShortcut("[", modifiers: [.command, .option])
            Divider()
            // The way IN to reading a branch commit by commit, which is
            // otherwise a thing you can only discover by opening the history
            // and picking the oldest row.
            Button("Read Commit by Commit") { AppCommand.diffFirstCommit.post() }
            Button("Next Commit") { AppCommand.diffNextCommit.post() }
                .keyboardShortcut("]", modifiers: [.command, .control])
            Button("Previous Commit") { AppCommand.diffPreviousCommit.post() }
                .keyboardShortcut("[", modifiers: [.command, .control])
            Divider()
            // Not a movement, which is why it is below the divider: it says
            // something about the worktree rather than about where you are in
            // it. The daemon keeps a per-worktree watermark and this is the
            // only thing on the Mac that moves it — see `ChangesStore.markRead`
            // for what it clears, most of which is on a phone.
            //
            // No key equivalent, deliberately. Every other item here is a
            // movement you undo by pressing the other one, and this is the one
            // item with a side effect the reader cannot see all of; a shortcut
            // next to ⌥⌘] would be reachable by accident.
            Button("Mark as Reviewed") { AppCommand.diffMarkRead.post() }
        }

        // Grouped only to stay inside what `CommandsBuilder` will build: it
        // takes ten statements and the Diff menu above is the eleventh. `Group`
        // is one statement holding four, and changes nothing about where these
        // land in the menu bar.
        Group {
            CommandGroup(after: .sidebar) {
                // ⌘B, because that is what it is in every editor people already have
                // open next to this one. macOS's own ⌃⌘S still works; this is the one
                // fingers reach for.
                //
                // No collision with the tiling prefix: that is ⌃B, a different
                // modifier, and ⌘ never reaches a terminal anyway.
                Button("Toggle Sidebar") { AppCommand.toggleSidebar.post() }
                    .keyboardShortcut("b", modifiers: .command)
            }

            // Nothing here prints, and the Print item SwiftUI adds for free would
            // otherwise hold ⌘P against a window whose most useful key it is.
            CommandGroup(replacing: .printItem) {}

            CommandGroup(after: .toolbar) {
                // ⌘P, the shortcut everyone arriving here already has in their
                // fingers from an editor, for the thing it means there: show me
                // everything, I will type the part I remember. It is a switcher when
                // the field is empty and a command list when it is not, which is one
                // key for the two questions this app is always being asked.
                Button("Go to Anything…") { AppCommand.commandPalette.post() }
                    .keyboardShortcut("p", modifiers: .command)
                // Search is navigation here, not a nicety: worktrees are unbounded
                // and typing is the fastest way to any of them, on any runner.
                Button("Find Workspace or Agent") { AppCommand.search.post() }
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
            NotificationCenter.default.publisher(for: Notification.Name("farcooler.selectIndex"))
        ) { note in
            guard let index = note.object as? Int else { return }
            perform(index)
        }
    }
}
