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
    case newWorktree
    case addRepository
    case newWorkspace
    case showBoard
    case openInEditor
    case reload
    case showShortcuts
    case search
    /// Board ▸ Mark All as Read: the navigator's Unread, read (ov-104).
    case markAllRead
    case commandPalette
    /// ⌘K: the title bar's field, to see what's happening or find
    /// something (ov-214, ov-264).
    case showActivity
    case toggleSidebar
    case diffNextHunk
    case diffPreviousHunk
    case diffNextFile
    case diffPreviousFile
    case diffNextCommit
    case diffPreviousCommit
    case diffFirstCommit
    case diffMarkRead
    /// Back: where the window was before (ov-192); with nowhere, up a
    /// level, from an opened worktree to its task, from a task to the
    /// workspace (spec §4.9).
    case back
    /// Forward: where Back left.
    case forward
    /// The jump bar takes the keyboard (⌘L, ov-192).
    case jumpBar
    /// Focus: a task's or worktree's terminals alone, at full size, or the
    /// rest put back.
    case focusColumn
    case focusConversation
    case focusBoard
    case focusTask
    /// The next or previous worktree in the workspace, in the board list's
    /// order (ov-86).
    case nextWorktree
    case previousWorktree
    /// Open the title bar's workspace switcher (⌘0).
    case switchWorkspace
    /// A task's next or previous tab, Overview, Agent and Changes (ov-98).
    case nextTaskTab
    case previousTaskTab
    /// Go to a line of the file open in Files (ov-189).
    case goToLine

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

    /// Go to the workspace numbered `number` (⌘1 through ⌘9, ov-86).
    static func selectWorkspace(_ number: Int) {
        NotificationCenter.default.post(name: selectWorkspaceNotification, object: number)
    }

    static let selectWorkspaceNotification = Notification.Name("farcooler.selectWorkspace")

    /// Go to a row of Workspace ▸ History (ov-248).
    static func goToHistory(_ spot: NavigationHistory.Spot) {
        NotificationCenter.default.post(name: goToHistoryNotification, object: spot)
    }

    static let goToHistoryNotification = Notification.Name("farcooler.goToHistory")
}

/// The menu bar.
///
/// This IS the discoverability story. A shortcut that only exists in a cheat
/// sheet has to be looked up; one in the menu bar shows its key equivalent
/// where people already look, and is findable through Help's menu search
/// without knowing it exists.
struct FarCoolerCommands: Commands {
    /// Nil unless the main window is key. See `MainWindowFocus`.
    @FocusedValue(\.mainWindow) private var mainWindow
    /// Nil unless a diff is the focused pane. See `DiffMenuFocus`.
    @FocusedValue(\.diffMenu) private var diff
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        // About Far Cooler, saying which build this is.
        //
        // The standard panel shows CFBundleShortVersionString and
        // CFBundleVersion, which for a beta and the release it names are
        // identical — so the panel that exists to answer "what am I running"
        // could not. This one names the channel too, and the daemon it is
        // driving, which is the pair that has to match.
        CommandGroup(replacing: .appInfo) {
            Button("About Far Cooler") { openWindow(id: AboutView.windowID) }
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
                .disabled(!(MainWindowFocus.isKey(mainWindow) && mainWindow?.hasWorktree == true))
            // ⌘N, the plainest shortcut in the app, for the thing it is for.
            //
            // "New Worktree…" and not "New Task…": this makes a worktree and
            // starts an agent in it, and puts nothing on the board, so "task"
            // on this item would name the other thing.
            Button("New Worktree…") { AppCommand.newWorktree.post() }
                .keyboardShortcut("n", modifiers: .command)
                .disabled(!MainWindowFocus.isKey(mainWindow))
            Button("New Workspace…") { AppCommand.newWorkspace.post() }
                .keyboardShortcut("n", modifiers: [.command, .option])
                .disabled(!(MainWindowFocus.isKey(mainWindow) && mainWindow?.makesWorkspaces == true))
            Button("Add Repository…") { AppCommand.addRepository.post() }
                .keyboardShortcut("r", modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.isKey(mainWindow))
        }

        // The board, beside the things it is about.
        //
        // ⇧⌘B rather than a bare ⌘B, which is the sidebar's on every Mac (here,
        // the navigator's) and is not up for grabs. It opens the board of the
        // workspace the window is in, and of the only repository there is
        // when nothing is selected.
        CommandGroup(after: .toolbar) {
            Button("Show Board") { AppCommand.showBoard.post() }
                .keyboardShortcut("b", modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.isKey(mainWindow))
        }

        // File's close items, in place of SwiftUI's (ov-265). ⌘W closes the
        // selected terminal while the main window is key, and with Settings
        // or About key it closes that window (`CloseCommand`).
        //
        // No Restart. A terminal is its process: restarting one is closing
        // it and opening another, which is ⌘W then ⌘T. A separate verb for
        // the same two steps is a concept to learn for nothing.
        CommandGroup(replacing: .saveItem) {
            let close = CloseCommand.at(mainWindow)
            Button(close.title) { close.perform(keyWindow: NSApp.keyWindow) }
                .keyboardShortcut("w", modifiers: .command)
                .disabled(!CloseCommand.closes(NSApp.keyWindow))
            if close == .terminal {
                Button("Close Window") { CloseCommand.window.perform(keyWindow: NSApp.keyWindow) }
                    .disabled(!MainWindowFocus.isKey(mainWindow))
            }
            Button("Close All") { CloseCommand.closeAll(NSApp.windows) }
                .keyboardShortcut("w", modifiers: [.command, .option])
                .disabled(!CloseCommand.closes(NSApp.keyWindow))
        }

        CommandGroup(after: .newItem) {
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
                .disabled(!(MainWindowFocus.isKey(mainWindow) && mainWindow?.hasWorktree == true))
        }

        // A workspace's levels and panes. ⌃⌘ and ⌥⌘ because the usual chords are
        // taken: ⌘[ is Previous Terminal and ⇧⌘↩ is Zoom Pane (spec §4.9).
        //
        // ⌘1 through ⌘9 are the workspaces themselves (ov-86), in the title
        // bar switcher's order, as a browser's are its tabs: they're the
        // fastest way between them. They were the terminals on
        // screen, which moved to ⌃⌘1 through ⌃⌘9.
        CommandMenu("Workspace") {
            // ⌃⌘← and ⌃⌘→, Xcode's history pair (ov-192): ⌘[ and ⌘] are
            // the terminals'.
            Button("Back") { AppCommand.back.post() }
                .keyboardShortcut(.leftArrow, modifiers: [.command, .control])
                .disabled(!MainWindowFocus.goes(\.goesBack, mainWindow))
            Button("Forward") { AppCommand.forward.post() }
                .keyboardShortcut(.rightArrow, modifiers: [.command, .control])
                .disabled(!MainWindowFocus.goes(\.goesForward, mainWindow))
            // The same list a long press on Back or Forward opens, for the
            // keyboard and the menu bar's search (ov-248).
            Menu("History") {
                if let mainWindow, MainWindowFocus.goes(\.goesHistory, mainWindow) {
                    HistoryMenuItems(rows: mainWindow.history, go: AppCommand.goToHistory)
                }
            }
            .disabled(!MainWindowFocus.goes(\.goesHistory, mainWindow))
            // ⌘L, a browser's "focus the location bar": nothing here or in
            // the system's menus holds it, and ⌘ never reaches a terminal.
            Button("Go to Jump Bar") { AppCommand.jumpBar.post() }
                .keyboardShortcut("l", modifiers: .command)
                .disabled(!MainWindowFocus.goes(\.hasJumpBar, mainWindow))
            // A checkmark while the terminals are at full size: it's a state,
            // and the same item puts the rest back (HIG, Menus: "Consider using
            // a checkmark to show that an attribute is currently in effect").
            Toggle("Focus", isOn: Binding(get: { mainWindow?.focused == true }, set: { _ in AppCommand.focusColumn.post() }))
                .keyboardShortcut(.return, modifiers: [.command, .control])
                .disabled(!MainWindowFocus.goes(\.focuses, mainWindow))
            Divider()
            Button("Orchestrator") { AppCommand.focusConversation.post() }
                .keyboardShortcut("1", modifiers: [.command, .option])
                .disabled(!MainWindowFocus.goes(\.inWorkspace, mainWindow))
            Button("Navigator") { AppCommand.focusBoard.post() }
                .keyboardShortcut("2", modifiers: [.command, .option])
                .disabled(!MainWindowFocus.goes(\.inWorkspace, mainWindow))
            Button("Main Area") { AppCommand.focusTask.post() }
                .keyboardShortcut("3", modifiers: [.command, .option])
                .disabled(!MainWindowFocus.goes(\.inWorkspace, mainWindow))
            // A task's tabs (ov-98): ⌃⌘] and ⌃⌘[, the owner's keys, on the
            // bracket family's rule that `[` and `]` walk a list. They were
            // the diff's commits, which moved out a modifier to ⌃⌥⌘.
            Section {
                Button("Next Task Tab") { AppCommand.nextTaskTab.post() }
                    .keyboardShortcut("]", modifiers: [.command, .control])
                    .disabled(!MainWindowFocus.stepsTaskTabs(mainWindow))
                Button("Previous Task Tab") { AppCommand.previousTaskTab.post() }
                    .keyboardShortcut("[", modifiers: [.command, .control])
                    .disabled(!MainWindowFocus.stepsTaskTabs(mainWindow))
            }
            // Only while the main window is key with nothing over it: with
            // the palette open or Settings key, they moved the window behind.
            Section {
                // The worktrees in the navigator's order: each task's, then
                // the loose ones under Worktrees.
                Button("Next Worktree") { AppCommand.nextWorktree.post() }
                    .keyboardShortcut(.downArrow, modifiers: [.command, .control])
                    .disabled(!MainWindowFocus.goes(\.nextWorktree, mainWindow))
                Button("Previous Worktree") { AppCommand.previousWorktree.post() }
                    .keyboardShortcut(.upArrow, modifiers: [.command, .control])
                    .disabled(!MainWindowFocus.goes(\.previousWorktree, mainWindow))
            }
            Section {
                // ⌘0, which nothing else here or in the system's menus holds:
                // the switcher from the keyboard, past the ninth workspace.
                Button("Switch Workspace…") { AppCommand.switchWorkspace.post() }
                    .keyboardShortcut("0", modifiers: .command)
                    .disabled(!MainWindowFocus.navigates(mainWindow))
                ForEach(1...WorkspaceNumbers.count, id: \.self) { n in
                    Button("Workspace \(n)") { AppCommand.selectWorkspace(n) }
                        .keyboardShortcut(KeyEquivalent(Character("\(n)")), modifiers: .command)
                        .disabled(!MainWindowFocus.picksWorkspace(n, mainWindow))
                }
            }
        }

        // The navigator's board. Mark All as Read is the Unread section's
        // context menu and its header's button too; here so it can be found
        // (ov-104 review). ⇧⌘K, as feed readers keep ⌘K: plain ⌘K is a
        // terminal's clear, and nothing here or in the system's menus holds
        // ⇧⌘K (ov-177).
        CommandMenu("Board") {
            Button("Mark All as Read") { AppCommand.markAllRead.post() }
                .keyboardShortcut("k", modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.marksRead(mainWindow))
        }

        CommandMenu("Terminal") {
            Button("Next Terminal") { AppCommand.nextTerminal.post() }
                .keyboardShortcut("]", modifiers: .command)
                .disabled(!MainWindowFocus.goes(\.stepsTerminals, mainWindow))
            Button("Previous Terminal") { AppCommand.previousTerminal.post() }
                .keyboardShortcut("[", modifiers: .command)
                .disabled(!MainWindowFocus.goes(\.stepsTerminals, mainWindow))
            Divider()
            // The one shortcut that is not a convention from elsewhere, because
            // nothing else has this idea: go straight to whatever is waiting on
            // you. It is the reason to open the app at all.
            Button("Next Item That Needs You") { AppCommand.nextAttention.post() }
                .keyboardShortcut("n", modifiers: [.command, .control])
                .disabled(!MainWindowFocus.stepsToAttention(mainWindow))
            Divider()
            // ⌃⌘, since ⌘1 through ⌘9 went to the workspaces (ov-86).
            ForEach(1...9, id: \.self) { n in
                Button("Terminal \(n)") { AppCommand.selectIndex(n - 1) }
                    .keyboardShortcut(KeyEquivalent(Character("\(n)")), modifiers: [.command, .control])
                    .disabled(!MainWindowFocus.picksTerminal(n, mainWindow))
            }
        }

        // The menu is where a prefix binding becomes discoverable to someone who
        // has never used tmux, and where someone who has can confirm that the
        // key they already know is the key here. Titles stay plain:
        // a prefix chord cannot be a key equivalent, so the HUD and the ⌘/ sheet
        // show those keys, with the prefix as set in Settings.
        CommandMenu("Layout") {
            // Splitting leads, because it is now the only way a layout grows and
            // the one thing every other item here presupposes.
            //
            // Each dimmed when the layout on screen can't do it, or there's
            // none (ov-211).
            Button("Split Right") { TileCommand.splitRight.post() }
                .disabled(!MainWindowFocus.lays(\.splits, mainWindow))
            Button("Split Down") { TileCommand.splitDown.post() }
                .disabled(!MainWindowFocus.lays(\.splits, mainWindow))
            Button("Move Pane Out") { TileCommand.breakPane.post() }
                .disabled(!MainWindowFocus.lays(\.movesOut, mainWindow))
            Divider()
            // ⇧⌘↩ rather than ⇧⌘Z, which is Edit ▸ Redo on every Mac and was
            // bound here twice over: the menu bar showed ⇧⌘Z in two menus, and
            // which one a keypress reached depended on whether the field you
            // were typing in had anything to redo. ⇧⌘↩ is what iTerm2 uses for
            // the same idea, and nothing standard holds it.
            // `ShortcutSheetTests` now refuses a chord the system's own menus
            // already use.
            //
            // A checkmark while a pane is zoomed, since choosing it again
            // puts the layout back.
            Toggle("Zoom Pane", isOn: Binding(get: { mainWindow?.layout?.zoomed == true }, set: { _ in TileCommand.zoom.post() }))
                .keyboardShortcut(.return, modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.zoomsPane(mainWindow))
            Button("Next Arrangement") { TileCommand.cycle.post() }
                .keyboardShortcut(.space, modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.lays(\.arranges, mainWindow))
            // Double-clicking a divider evens out the two panes it separates.
            // This is the same idea for the whole layout, and it is here rather
            // than only on the divider because a gesture nobody has been told
            // about needs somewhere to be discovered.
            Button("Even Out Panes") { TileCommand.evenPanes.post() }
                .disabled(!MainWindowFocus.lays(\.arranges, mainWindow))
            // The submenu stays openable with its items dimmed, so what it
            // offers can still be read.
            Menu("Arrangement") {
                ForEach(TilePreset.allCases) { preset in
                    Button(preset.label) { TileCommand.preset(preset).post() }
                        .disabled(!MainWindowFocus.lays(\.arranges, mainWindow))
                }
            }
            Divider()
            // The prefix-less ones, and the only tiling bindings that get a real
            // key equivalent here: they are used constantly, and a menu item is
            // how someone finds out they exist.
            Button("Pane Left") { TileCommand.focus(.left).post() }
                .disabled(!MainWindowFocus.lays(\.left, mainWindow))
            Button("Pane Right") { TileCommand.focus(.right).post() }
                .disabled(!MainWindowFocus.lays(\.right, mainWindow))
            Button("Pane Above") { TileCommand.focus(.top).post() }
                .disabled(!MainWindowFocus.lays(\.above, mainWindow))
            Button("Pane Below") { TileCommand.focus(.bottom).post() }
                .disabled(!MainWindowFocus.lays(\.below, mainWindow))
            Divider()
            Button("Next Pane") { TileCommand.focusNext.post() }
                .disabled(!MainWindowFocus.lays(\.stepsPanes, mainWindow))
            Button("Previous Pane") { TileCommand.focusPrevious.post() }
                .disabled(!MainWindowFocus.lays(\.stepsPanes, mainWindow))
            Divider()
            Button("New Layout") { TileCommand.newGroup.post() }
                .disabled(!MainWindowFocus.lays(\.splits, mainWindow))
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
            Button("Next Layout") { TileCommand.nextGroup.post() }
                .keyboardShortcut("]", modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.lays(\.stepsLayouts, mainWindow))
            Button("Previous Layout") { TileCommand.previousGroup.post() }
                .keyboardShortcut("[", modifiers: [.command, .shift])
                .disabled(!MainWindowFocus.lays(\.stepsLayouts, mainWindow))
            Divider()
            // Not really a layout verb — nothing about the arrangement
            // changes — but it is scoped to the focused pane exactly the way
            // zoom and the splits are, and there is no chrome on the pane
            // itself left to put a button on.
            Button("Switch Between Terminal and Chat") {
                TileCommand.toggleAgentPane.post()
            }
            .disabled(!MainWindowFocus.lays(\.switchesMode, mainWindow))
        }

        // The diff pane had not one shortcut in this file, which made the only
        // keyboard-shaped thing in the app a pane you could only read with a
        // pointer. The phone needs a thumb-sized button to move through a
        // branch; a Mac needs a key, and the chevrons in the pane are the
        // fallback rather than the other way round.
        //
        // Three pairs on the same convention the rest of the app already
        // teaches: `[` and `]` walk a list, and the modifier says WHICH list.
        // ⌘ is terminals, ⇧⌘ is layouts, ⌃⌘ a task's tabs, so ⌥⌘ is the files
        // in a diff and ⌃⌥⌘ the commits behind them — outermost list, most
        // modifiers.
        //
        // Hunks get the arrows instead, because a hunk is not a list you pick
        // from — it is the next place down the document, which is what ⌥⌘↓
        // reads as.
        //
        // These act on the FOCUSED pane, like every other pane-scoped command
        // here. Click into a diff and it answers; with a terminal focused they
        // do nothing, and say so by being dimmed (ov-211): the focused diff
        // publishes what it can do, as `DiffMenuFocus`.
        CommandMenu("Diff") {
            Button("Next Hunk") { AppCommand.diffNextHunk.post() }
                .keyboardShortcut(.downArrow, modifiers: [.command, .option])
                .disabled(!DiffMenuFocus.allows(\.nextHunk, diff, in: mainWindow))
            Button("Previous Hunk") { AppCommand.diffPreviousHunk.post() }
                .keyboardShortcut(.upArrow, modifiers: [.command, .option])
                .disabled(!DiffMenuFocus.allows(\.previousHunk, diff, in: mainWindow))
            Divider()
            Button("Next File") { AppCommand.diffNextFile.post() }
                .keyboardShortcut("]", modifiers: [.command, .option])
                .disabled(!DiffMenuFocus.allows(\.nextFile, diff, in: mainWindow))
            Button("Previous File") { AppCommand.diffPreviousFile.post() }
                .keyboardShortcut("[", modifiers: [.command, .option])
                .disabled(!DiffMenuFocus.allows(\.previousFile, diff, in: mainWindow))
            Divider()
            // The way IN to reading a branch commit by commit, which is
            // otherwise a thing you can only discover by opening the history
            // and picking the oldest row.
            Button("Read Commit by Commit") { AppCommand.diffFirstCommit.post() }
                .disabled(!DiffMenuFocus.allows(\.readsCommits, diff, in: mainWindow))
            Button("Next Commit") { AppCommand.diffNextCommit.post() }
                .keyboardShortcut("]", modifiers: [.command, .control, .option])
                .disabled(!DiffMenuFocus.allows(\.nextCommit, diff, in: mainWindow))
            Button("Previous Commit") { AppCommand.diffPreviousCommit.post() }
                .keyboardShortcut("[", modifiers: [.command, .control, .option])
                .disabled(!DiffMenuFocus.allows(\.previousCommit, diff, in: mainWindow))
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
                .disabled(!DiffMenuFocus.allows(\.marksReviewed, diff, in: mainWindow))
        }

        // Grouped only to stay inside what `CommandsBuilder` will build: it
        // takes ten statements and the Diff menu above is the eleventh. `Group`
        // is one statement holding four, and changes nothing about where these
        // land in the menu bar.
        Group {
            CommandGroup(after: .sidebar) {
                // ⌘B, because that is what it is in every editor people already have
                // open next to this one. It shows and hides the workspace's
                // navigator, the window's one sidebar (ov-178).
                //
                // No collision with the tiling prefix: that is ⌃B, a different
                // modifier, and ⌘ never reaches a terminal anyway.
                //
                // Named for what it will do, Show Sidebar or Hide Sidebar (HIG,
                // The menu bar: "Ensure that each show/hide item title reflects
                // the current state of the corresponding view"). ov-204.
                Button(MainWindowFocus.sidebarTitle(mainWindow)) { AppCommand.toggleSidebar.post() }
                    .keyboardShortcut("b", modifiers: .command)
                    .disabled(!MainWindowFocus.togglesSidebar(mainWindow))
            }

            // Nothing here prints, and the Print item SwiftUI adds for free would
            // otherwise hold ⌘P against a window whose most useful key it is.
            CommandGroup(replacing: .printItem) {}

            CommandGroup(after: .toolbar) {
                // ⌘P, the shortcut everyone arriving here already has in their
                // fingers from an editor, for the thing it means there: show me
                // everything, I will type the part I remember. Since ov-214 it
                // opens the title bar's field on the recent terminals.
                Button("Go to Anything…") { AppCommand.commandPalette.post() }
                    .keyboardShortcut("p", modifiers: .command)
                    .disabled(!MainWindowFocus.isKey(mainWindow))
                // ⌘K, the command bar's key in most apps that have one. Free
                // here: no menu item held it and a terminal pane hands every
                // ⌘ chord to the menu (`TerminalRenderView.keyDown`), so it
                // was never Clear in a pane.
                Button("Show Activity") { AppCommand.showActivity.post() }
                    .keyboardShortcut("k", modifiers: .command)
                    .disabled(!MainWindowFocus.isKey(mainWindow))
                // ⌘R, which is Reload everywhere else. ⌘0 is Actual Size.
                Button("Reload Fleet") { AppCommand.reload.post() }
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(!MainWindowFocus.isKey(mainWindow))
            }

            // In Edit, beside the other kinds of find, and with the ellipsis a
            // command that asks for more input takes.
            //
            // Search is navigation here, not a nicety: worktrees are unbounded
            // and typing is the fastest way to any of them, on any runner.
            CommandGroup(after: .textEditing) {
                // In a workspace, ⌘F filters its navigator's tasks (ov-103),
                // and says so; elsewhere it opens Go to Anything (ov-178).
                Button(MainWindowFocus.findTitle(mainWindow)) { AppCommand.search.post() }
                    .keyboardShortcut("f", modifiers: .command)
                    .disabled(!MainWindowFocus.isKey(mainWindow))
                // ⇧⌘L for the file open in Files (ov-189): the jump bar
                // (ov-192) holds Xcode's ⌘L.
                Button("Go to Line…") { AppCommand.goToLine.post() }
                    .keyboardShortcut("l", modifiers: [.command, .shift])
                    .disabled(mainWindow?.findsInFile != true)
            }

            CommandGroup(replacing: .help) {
                Button("Keyboard Shortcuts") { AppCommand.showShortcuts.post() }
                    .keyboardShortcut("/", modifiers: .command)
                    .disabled(!MainWindowFocus.isKey(mainWindow))
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

    func onSelectWorkspace(_ perform: @escaping (Int) -> Void) -> some View {
        onReceive(NotificationCenter.default.publisher(for: AppCommand.selectWorkspaceNotification)) { note in
            guard let number = note.object as? Int else { return }
            perform(number)
        }
    }

    func onGoToHistory(_ perform: @escaping (NavigationHistory.Spot) -> Void) -> some View {
        onReceive(NotificationCenter.default.publisher(for: AppCommand.goToHistoryNotification)) { note in
            guard let spot = note.object as? NavigationHistory.Spot else { return }
            perform(spot)
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
