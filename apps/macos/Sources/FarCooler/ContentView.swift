import AgentKit
import AppKit
import SwiftUI

struct ContentView: View {
    /// The app's one store: every window sees the same runners (ov-133).
    @ObservedObject var store = FleetStore.shared
    /// This window's identity in `Notifier`'s per-window record of what is on
    /// screen, so a second window adds to it rather than overwriting it.
    @State var windowID = UUID()
    @ObservedObject var preferences = Preferences.shared
    @ObservedObject private var themes = Themes.shared
    /// A click on a notification, waiting to be opened (ov-106, ov-183).
    @ObservedObject var noticeOpener = DestinationOpener.shared
    @Environment(\.openSettings) var openSettings
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.windowVisible) var windowVisible
    @Environment(\.markReadConfirmation) var markReadConfirmation
    @State var selection: Selection?

    /// What a `+` meant, carried whole from the control that was clicked to
    /// the sheet it opens — or nil when no sheet is up.
    ///
    /// One value rather than the three pieces of state this was. Those had to
    /// be kept in step by hand and two of the three call sites did not: the
    /// empty-state button and the placeholder button cleared the HOST and left
    /// the repository behind, so after using a project header's `+` once, both
    /// of them silently inherited that project forever.
    ///
    /// Being `Identifiable` is the other half, and it is what fixes the
    /// reported bug. The sheet keeps its picked repository in `@State` and
    /// seeds it once, guarded on being unset — so a sheet whose state SwiftUI
    /// reused across presentations kept the first project it was ever opened
    /// on, and clicking `+` on the second repository re-opened the first.
    /// `sheet(item:)` presents a new value as a new presentation, so each `+`
    /// gets a view that has never chosen anything.
    struct NewWorktreeIntent: Identifiable {
        /// The project the control named, by display name, which is what the
        /// sheet matches on. Empty
        /// when the control named no project at all.
        var project: String = ""
        /// The runner that project is on. Only meaningful beside `project`:
        /// two runners can share a display name, and only one of them is the
        /// project the header actually named. Empty means "none was named",
        /// which is ⌘N's own and the empty-state buttons — the
        /// one case where letting the sheet choose a default is legitimate
        /// rather than the picker again in disguise.
        var host: String = ""
        var id: String { "\(host)\u{1}\(project)" }
    }

    @State var newWorktreeIntent: NewWorktreeIntent?
    /// Each worktree's change state, kept across selection changes: a review is
    /// a session, not a mode. Coming back to a worktree should still be showing
    /// the file you were reading.
    ///
    /// There is no companion `tileLayouts` any more. The app used to hold a
    /// tree of tiles per worktree, persisted per device, purely so the diff
    /// could sit beside the terminals — a second layout engine next to tmux's,
    /// which owns every rectangle in this window. The diff is a tmux pane now,
    /// so where it sits is tmux's answer like every other pane's.
    @State var changesStores: [String: ChangesStore] = [:]
    /// One board store per workspace, keyed by runner and workspace id — the
    /// repository's id for a runner without workspaces, whose one board per
    /// repository is the repository's.
    ///
    /// Keyed by both because an id is minted per daemon: two runners can
    /// hand back rows that collide on id alone. Same lifetime rule as
    /// `changesStores` — see `boardStore(for:client:host:)`.
    @State var boardStores: [String: TaskBoardStore] = [:]
    @State var showAddRepository = false
    @State var showAdd = false
    @State var showShortcuts = false
    @State var showQuickCreate = false
    /// The ⌘N panel's start in flight, kept here so it survives the panel
    /// closing and reopening.
    @StateObject private var taskSubmission = TaskSubmission()
    @AppStorage("tasks.lastProject") var lastProject = ""
    /// Where the window was when the app last closed, as a `Destination`'s
    /// encoding: the workspace and what's open in it, a task's tab and agent,
    /// the pane the keyboard was in (ov-182). Recorded as it changes rather
    /// than on quit: an app that is force quit, crashes, or is killed by a
    /// rebuild never gets a last word.
    ///
    /// Reopening somewhere else is a small thing that costs a real one: the
    /// workspace you were in is the reason you came back.
    @AppStorage(SelectionMemory.destinationKey) var lastDestination = ""
    /// What an earlier build kept, the selection alone: read when no
    /// `lastDestination` has been written yet. It replaced `fleet.lastTerminal`,
    /// which `SelectionMemory.migrate` maps once.
    @AppStorage(SelectionMemory.key) var lastSelection = ""
    /// Whether the launch has chosen where the window opens. See
    /// `settleLaunch`.
    @State var launched = false
    /// Where this window is going back to, while it waits for its runner.
    @State var restoring: DestinationOpen?
    /// Bumped by ⌘F in a workspace: its navigator's filter takes the
    /// keyboard (ov-103).
    @State var boardFilterRequest = 0
    @State var removeWorktree: Worktree?
    @State var removeRepository: RepositoryToRemove?
    @State private var showResumeBranch = false
    /// The title bar's field (ov-214, ov-264): ⌘K for the activity, ⌘P for
    /// find. It took the floating palette's place.
    @State var console = TitleConsoleModel()
    /// Quick-create's draft, reachable from here so that what was typed into
    /// the palette arrives in the panel that acts on it. See `perform`.
    @AppStorage("tasks.draft") var taskDraft = ""
    /// One divider resize at a time. See `resizeDivider`.
    @State var resizingDivider = false
    /// Set when `setPaneMode` comes back `confirmationRequired` — a turn is in
    /// flight and switching would cancel it. Drives a sheet the same way
    /// `removeWorktree` does, rather than a banner: this refusal has an
    /// answer ("cancel it anyway?") a banner cannot offer.
    @State var pendingPaneModeSwitch: PaneModeConfirmation?
    /// What an editor said when it would not start.
    ///
    /// Its own state rather than routed through `errorBanner`, which is
    /// rendered only by `fleetPlaceholder`'s error branch and by the general
    /// banner below — the opposite of when this control exists. See
    /// `ErrorBanner`, which shows it.
    @State var editorError: String?
    /// Each action's result, shown by the banners over the detail pane.
    ///
    /// Filed by `act(_:on:target:subject:default:_:)` under the action and
    /// its target, so a result stays until it is dismissed or that action
    /// is done to that target again. It used to be one client's `lastError`
    /// copied out after every action, which every command wrote and every
    /// good refresh cleared: a refused Stop vanished when the refresh after
    /// it worked, and a focus that worked showed a minute-old failure.
    @StateObject var outcomes = ActionOutcomes()
    /// A sentence the app wrote that isn't one action's result: the one
    /// notice slot, cleared on navigation. See `ActionOutcomes.notice`.
    var errorBanner: String? {
        get { outcomes.notice }
        nonmutating set { outcomes.notice = newValue }
    }
    /// Workspaces whose orchestrator this app has asked to start and the
    /// runner hasn't answered, by `orchestratorKey`.
    @State var startingOrchestrators = OrchestratorStarts()
    /// A Replace Orchestrator waiting on its confirmation.
    @State var orchestratorReplacement: OrchestratorReplacement?
    /// Use as Orchestrator on a workspace that has one, until confirmed.
    @State var adoptionPending: OrchestratorAdoptionPending?
    /// A close waiting on its answer: an agent mid-turn (`requestClose`).
    @State var closePending: CloseTerminalPending?
    /// The terminal the Rename Terminal sheet is asking a name for (ov-234).
    @State var renaming: RenamingTerminal?

    /// The pane last clicked or focused, which the keyboard acts on while
    /// it's on screen. See `WorkspaceScreen.keyPane`: with a task open, the
    /// conversation and the task's agent are both on screen, and ⌃B has to
    /// act on the one you were last in.
    @State var keyPane: PaneRef?
    /// The agent each task's column shows, by task id, when several are on
    /// it and one was picked.
    @State var chosenAgents: [String: String] = [:]
    /// The tab each task shows, Overview, Agent or Changes, as last chosen
    /// in this window (ov-98).
    @State var taskTabs = TaskTabMemory()
    /// The Needs You item ⌃⌘N last opened, by its key: where the next
    /// press goes on from, while the window is still showing it.
    @State var lastAttention: String?
    /// The navigator's width, as its trailing edge was last dropped, on
    /// this Mac (ov-92).
    @AppStorage("workspace.navigatorWidth") var navigatorWidth = Double(WorkspaceColumns.navigatorDefault)
    /// The orchestrator's chat column's width beside the canvas, in
    /// terminal columns, as its edge was last dropped, on this Mac (ov-298).
    @AppStorage("workspace.chatColumns") var chatColumns = Double(WorkspaceColumns.chatColumnsDefault)
    /// The navigator floated over the canvas by ⌘B, in a window too narrow
    /// for it beside the canvas (ov-298).
    @State var navigatorFloating = false
    /// The plan peeked over the chat, with the canvas folded away (ov-298).
    @State var planPeeking = false
    /// The navigator's pane heights a drag chose, this window's (ov-244).
    /// Kept in this window's record, not `@SceneStorage`: that comes back only
    /// when the system restores windows, which it doesn't by default (ov-248).
    @State var navigatorSplit = ""
    /// This window's record as it was taken, which `sessionState` keeps current.
    @State var kept: WindowSession?
    @Environment(\.openWindow) var openWindow
    /// Asks the navigator for the keyboard: bumped by ⌥⌘2, and by a click
    /// on a row, so ↑ and ↓ walk it from there.
    @State var boardFocusRequest = 0
    /// Focus (⌃⌘↩): a task or worktree opened, alone, without the
    /// navigator.
    @State var focusColumn = false
    /// The detail's width, as the workspace view last measured it: which of
    /// a workspace's columns are on screen. Nil until one has been drawn.
    @State var detailWidth: CGFloat?
    /// The task a worktree was opened from with Open Worktree: where Back
    /// goes. See `WorkspaceNavigation`.
    @State var trail: Selection?
    /// The worktree Open Worktree opened from `trail`.
    @State var trailWorktree: String?
    /// The jump bar's and history's window state (ov-192). See `JumpBarWindow`.
    @State var jumpBar = JumpBarWindow()
    /// The task whose changes the keyboard is in: the Diff menu's
    /// shortcuts are for the diff you clicked into.
    @State var changesFocus: String?
    /// This window's Files (ov-189).
    @StateObject var files = FilesRouting()
    /// The key monitor that turns Esc into Back. See `EscapeBack`.
    @State private var escapeMonitor: Any?
    /// The mouse's side buttons and the swipe between pages, as Back and
    /// Forward (`BackForwardGesture`), for this window only.
    @State private var navigationMonitor: Any?
    /// ⌥⌘2 gave the board the keyboard: no terminal takes typed keys until
    /// a pane is clicked or chosen again.
    @State var keyboardOnBoard = false
    /// ⌥⌘2 from a task, going up to the board: the selection's change
    /// leaves the keyboard on the board this once.
    @State var boardKeyboardPending = false
    /// This window, for the Esc monitor, which hears every window's keys.
    @State var windowBox = WindowBox()
    /// New Workspace…, with the name typed into the palette, while its
    /// sheet is up.
    @State var newWorkspaceName: NewWorkspaceName?
    struct NewWorkspaceName: Identifiable {
        let name: String
        var id: String { name }
    }
    /// Whether this window's navigator is put away (⌘B, ov-178): as the
    /// last window left it, and out the first time. See
    /// `NavigatorVisibility`.
    @State var navigatorHidden = NavigatorVisibility.hiddenAtLaunch()
    /// The window's width, as the title bar's status area measures it
    /// (ov-214): what decides its form and whether Back and Forward fit.
    @State var windowWidth: CGFloat = 0
    /// Bumped by ⌘0 (Switch Workspace…): the title bar's switcher opens.
    @State var switcherRequest = 0
    /// When each workspace's orchestrator start began, by `host|workspace`:
    /// this app's, or one first seen starting. See `ConversationColumn.slowStart`.
    @State var orchestratorStartedAt: [String: Date] = [:]

    /// What confirming a pane-mode switch would do, and to which pane.
    struct PaneModeConfirmation: Identifiable {
        let id = UUID()
        /// Which runner `terminal` is on — routing is by worktree, not by
        /// the client that was current when the confirmation was raised,
        /// because by the time someone answers the sheet that may no longer
        /// be the same runner.
        let worktree: Worktree
        let terminal: String
        let mode: String
        let message: String
    }

    var body: some View {
        // The title bar's values, worked out once a pass (ov-229): each
        // measures text or walks the fleet, and several parts read them.
        let room = titleStatusRoom
        let layout = room.layout(window: windowWidth)
        let consoleActions = titleConsoleActions
        let statusActions = titleStatusActions(consoleActions)
        let statusSource = titleStatusSource ?? Self.noWorkspaceSource
        // The detail is the window: the old Fleet sidebar is gone, and the
        // navigator inside a workspace is its one sidebar (ov-178).
        detailOpeningNotices
        // Task keys in its text open their tasks (ov-196).
        .environment(\.taskKeyLinker, taskKeyLinker)
        // The window's toolbar, inner to outer, so its items are
        // laid out from the trailing end in (ov-105): see
        // `LeadingToolbar` and `TrailingToolbar`.
        .toolbar {
            TrailingToolbar(
                troubles: runnerTroubles, stale: store.staleHosts, ahead: store.aheadHosts,
                updates: store.staleHosts.compactMap(daemonUpdate(for:)), needsYou: store.needsYou.count,
                needsYouSelected: selection == .needsYou, onNeedsYou: { selection = .needsYou },
                perform: { perform($0) })
        }
        // Open in Editor and Changes, one capsule (ov-214).
        .toolbar {
            WorktreeToolbar(
                editor: detailWorktree, onEditorError: { editorError = $0 },
                changes: changesToolbarState, onChanges: { ws in toggleChangesPane(in: ws) },
                files: filesToolbarState,
                onFiles: { ws in files.toggleInspector(for: ws) })
        }
        .titleBarStatus(statusSource, room: room, actions: statusActions, width: $windowWidth)
        // The title would repeat the switcher or the breadcrumb
        // (`TitleBar`); the window keeps it for the Window menu.
        .toolbar(removing: TitleBar.showsTitle(for: selection) ? nil : .title)
        .toolbar {
            LeadingToolbar(
                switcher: workspaceSwitcher,
                navigator: NavigatorToggle(
                    hidden: navigatorHidden, available: selection.flatMap(workspaceScene)?.board != nil,
                    toggle: { toggleNavigator() }),
                backForward: layout.backForward ? backForward : nil)
        }
        // The regular toolbar (ov-214), on whatever made the window.
        .mainWindowChrome()
        .overlay(alignment: .top) {
            // Carries the close confirmation as well: this chain is at the
            // type checker's limit, and a dialog may be presented from any
            // view in the window.
            ActionBanners(outcomes: outcomes)
                .confirmingClose($closePending) { pending in
                    Task { await close(pending.terminal, in: pending.worktree) }
                }
        }
        // A message arriving on a keystroke, so the same snappy preset
        // `PrefixHintOverlay` uses for its chip.
        .animation(.snappy(duration: 0.22), value: outcomes.shown)
        .task {
            adoptSession()
            // After `adoptSession`, which gives the window its id.
            store.open(window: windowID)
            DestinationOpener.shared.register(window: windowID)
            Notifier.shared.requestAuthorization()
            PushRegistration.shared.label = { Host.current().localizedName ?? "Mac" }
            // Beside the label and for the same reason: AgentKit files this
            // with the device so the relay can honor it while the app is
            // closed, and the key it lives under is this app's, not AgentKit's.
            PushRegistration.shared.notifyOnDone = { Preferences.shared.notifyOnDone }
            AccountSection.afterSignIn = { await PushRegistration.shared.sendIfPossible() }
            // Before the first read, not after: a stale daemon left over from an
            // earlier build would otherwise answer it, and everything from that
            // point on would be this app talking to a different program.
            //
            // `FleetStore` already does this itself for the local runner's own
            // bring-up — see `rebuild()` — so this call is almost always just
            // confirming what is already running by the time it lands. Kept
            // anyway so a daemon that failed to start is never silent even if
            // that race is ever changed.
            if let problem = await LocalDaemon.shared.ensure().problem {
                store.clients[""]?.fleetError = problem
            }
            // Every runner's own fleet, repositories, roots, layouts and
            // event stream were brought up, or brought back after the last
            // window closed, by `store.open(window:)` above.
            settleLaunch()
        }
        .onDisappear {
            // What this window showed is shown no longer; the runners are told
            // what the remaining windows still show.
            Notifier.shared.closeWindow(windowID)
            DestinationOpener.shared.unregister(window: windowID)
            closeSession()
            // The streams stop only with the last window (`FleetStore.close`);
            // each runner is told what the windows left still show.
            for client in store.clients.values { client.reportWatching([]) }
            store.close(window: windowID)
            if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
            escapeMonitor = nil
            if let navigationMonitor { NSEvent.removeMonitor(navigationMonitor) }
            navigationMonitor = nil
        }
        // Esc goes Back when nothing that needs it has the keyboard, in this
        // window, with no sheet or overlay up.
        .background(WindowReader(box: windowBox))
        .onAppear {
            if navigationMonitor == nil {
                navigationMonitor = NSEvent.addLocalMonitorForEvents(matching: [.otherMouseDown, .swipe]) { event in
                    guard let window = event.window, window === windowBox.window, window.attachedSheet == nil,
                        let direction = BackForwardGesture.direction(of: event)
                    else { return event }
                    let control = backForward
                    switch direction {
                    case .back:
                        guard control.canGoBack else { return event }
                        control.back()
                    case .forward:
                        guard control.canGoForward else { return event }
                        control.forward()
                    }
                    return nil
                }
            }
            guard escapeMonitor == nil else { return }
            escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                guard event.keyCode == 53, event.modifierFlags.intersection(.deviceIndependentFlagsMask).isEmpty,
                    let window = event.window, window === windowBox.window, window.attachedSheet == nil,
                    !console.console.isOpen, !showQuickCreate, !jumpBar.active,
                    EscapeBack.goesBack(
                        responder: window.firstResponder, selection: selection,
                        focusColumn: focusColumn,
                        closable: selection.flatMap { current in
                            WorkspaceNavigation.closing(current, board: workspaceScene(current)?.board)
                        } != nil)
                else { return event }
                goBack(toOrchestrator: true)
                return nil
            }
        }
        // Recorded as it changes rather than on quit: an app that is force
        // quit, crashes, or is killed by a rebuild never gets a last word, and
        // this is exactly the state worth surviving all three.
        // Tells the menu bar the main window is key, and whether an overlay
        // is open over it. See `MainWindowFocus`.
        .focusedSceneValue(\.mainWindow, menuFocus)
        // The key window's alone: with two windows, both heard every
        // command, and ⌘B toggled one sidebar twice (review m2).
        .onCommand { command in if isKeyWindow { run(command) } }
        .onTileCommand(in: { windowBox.window }) { command in Task { await tile(command) } }
        .onSelectIndex { index in if isKeyWindow { selectTerminal(at: index) } }
        .onGoToHistory { spot in if isKeyWindow { go(to: spot) } }
        .onSelectWorkspace { number in
            guard isKeyWindow else { return }
            if let target = WorkspaceNumbers.target(number, in: WorkspaceNumbers.groups(in: store.fleet)) {
                selection = target
            }
        }
        // The title bar's field closes as the window moves to another
        // workspace, and keeps today's spend by it (ov-214).
        .onChange(of: selection.flatMap(workspaceScene)?.key ?? "", initial: true) { _, key in
            console.console.enter(workspace: key)
        }
        .onChange(of: navigatorHidden) { _, hidden in
            UserDefaults.standard.set(NavigatorVisibility.stored(hidden), forKey: NavigatorVisibility.key)
        }
        .onChange(of: store.layouts) { _, _ in followLayoutFocus() }
        // Every client change reaches `store.fleet` — `FleetStore` remerges
        // on each one — so this hears a runner leaving, coming back as a new
        // client, and listing its projects without one.
        .onReceive(store.$fleet) { _ in pruneBoardStores() }
        .captureOpening($selection, peeking: $planPeeking)
        .onChange(of: store.fleet) { old, _ in
            // One rule for every way a terminal can disappear: exiting on its
            // own, being closed here, being closed from a phone, or its
            // worktree being hidden or removed. Hooking each path separately
            // meant the common one — you press Ctrl-D in the terminal you are
            // looking at — left the selection pointing at something that no
            // longer existed.
            healSelection(previous: old.worktrees)
            // A result about a terminal or worktree that's gone describes
            // nothing anymore, however it went.
            outcomes.prune(in: store.fleet)
            // An agent finishing under your nose is a fleet change and nothing
            // else — no click, no selection change — so this is the only hook
            // that can catch the case where you were already watching it.
            markVisibleSeen()
            // A runner's first read can land well after this view's own
            // `.task` already ran and found nothing to select — `FleetStore`
            // brings every client up in the background, on its own schedule.
            // A no-op once the window has opened somewhere.
            settleLaunch()
        }
        .onChange(of: store.needsYou) { _, _ in settleLaunch() }
        // ⌃HJKL traverse the layout the keyboard is in, and pass through to
        // a lone pane's program. See `WorkspaceScreen.tiledPanes`.
        .onChange(of: WorkspaceScreen.tiledPanes(selectedPane, in: shown), initial: true) { _, count in
            PrefixMode.shared.tiledPanes = count
        }
        // A column that comes on screen, or goes, as the detail is resized:
        // what's seen and watched follows it.
        .onChange(of: detailWidth) { _, _ in markVisibleSeen() }
        .onChange(of: focusColumn) { _, _ in markVisibleSeen() }
        // A task's agent is on screen only behind its Agent tab (ov-98).
        .onChange(of: taskTabs) { _, _ in markVisibleSeen() }
        .onChange(of: store.needsYouSettled) { _, _ in settleLaunch() }
        .onChange(of: keptPlace) { _, now in
            if let now, windowBox.window?.isKeyWindow != false { lastDestination = now }
        }
        .onReceive(windowGeometry) { keepFrame($0) }
        .onChange(of: sessionInputs) { _, _ in if let record = sessionState { WindowSessions.shared.update(record) } }
        // Where a new window starts, and the fallback: the key window's.
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { note in
            if let now = keptPlace, (note.object as? NSWindow) === windowBox.window { lastDestination = now }
        }
        // Coming back to the app is reading whatever it comes back to. The
        // notification did its job while you were away; leaving the row lit
        // afterwards makes you dismiss the same news twice.
        .onReceive(
            NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            markVisibleSeen()
        }
        // This window minimized, brought back, covered or uncovered: what's
        // in sight changed with no fleet event or selection change.
        .onReceive(
            NotificationCenter.default.publisher(for: NSWindow.didChangeOcclusionStateNotification)
                .merge(with: NotificationCenter.default.publisher(for: NSWindow.didMiniaturizeNotification))
                .merge(with: NotificationCenter.default.publisher(for: NSWindow.didDeminiaturizeNotification))
        ) { note in
            guard let window = note.object as? NSWindow, window === windowBox.window else { return }
            markVisibleSeen()
        }
        .onChange(of: selection) { old, new in
            // The notice is cleared on every navigation, so a sentence left
            // behind on one pane does not go on describing a pane the user is
            // no longer looking at. Selection is the one thing every
            // navigation path — sidebar click, ⌘P, ⌘], ⌃B o, closing a
            // terminal — funnels through, which makes it the narrowest point
            // that sees every one of them. An action's own result stays: it
            // names what it was about, and a refused Stop that went away on
            // the next click would be the bug it exists to fix.
            outcomes.clearNotice()
            jumpBar.history.record(from: old, to: new, trail: openedFrom(old))
            // Focus and a navigator holding the keyboard are about the view
            // that was on screen, not the next one. Focus stays while the
            // same thing is open, whichever of its panes is selected.
            if !WorkspaceSelection.samePlace(old, new) { focusColumn = false }
            keyboardOnBoard = WorkspaceNavigation.boardKeepsKeyboard(
                pending: boardKeyboardPending, from: old, to: new)
            boardKeyboardPending = false
            // The breadcrumb holds only while in the worktree it opened.
            if trail != nil, !WorkspaceNavigation.keeps(trail: trail, opened: trailWorktree, now: new) {
                trail = nil
                trailWorktree = nil
            }

            // Selecting a pane focuses it, which is also what switches to its
            // layout. Done here rather than in `detail`, because it is a write and
            // a view must not perform one while it is being evaluated.
            //
            // One call, not two. It used to activate the group and stop there,
            // which put the layout on screen and left its REMEMBERED focus in
            // charge — so clicking Terminal 6 landed you on whichever pane of that
            // layout you had used last. `⌘P` had the same fault for the same
            // reason: both set a selection and let the layout overrule it.
            let arrived = WorkspaceScreen.keyPane(nil, in: shownLayouts(for: new), selection: new)
            keyPane = arrived
            // Glancing from the board, the keyboard stays there: nothing on
            // the runner moves for a task passed on the way.
            if !keyboardOnBoard, let arrived,
                let worktree = worktree(host: arrived.host, id: arrived.worktree),
                let c = store.client(for: worktree),
                let holder = c.group(holding: arrived.terminal, in: arrived.worktree),
                let pane = holder.pane(arrived.terminal),
                !holder.isActive || !pane.focused
            {
                Task {
                    await act(.arrange, on: worktree) { client in
                        await client.focusPane(pane.short, in: worktree)
                    }
                }
            }

            // Selecting the main checkout's row shows its own layout, never
            // the orchestrator's window tmux calls active (see `ownLayouts`).
            // Bring that layout forward on the runner too. This app's ⌃B
            // commands name the layout on screen (see `tile`), so they don't
            // need it; this undoes an agent having focused the orchestrator,
            // so tmux's active window matches the screen.
            //
            // Only then: when tmux's active window is an orchestrator's, the
            // one case where the row's choice differs from the runner's. Any
            // other row shows the window tmux already calls active, and
            // selecting it moves nothing on the runner.
            if let (host, wsID) = Self.openedWhole(new),
                let worktree = worktree(host: host, id: wsID),
                let c = store.client(for: worktree),
                let active = c.activeGroup(wsID),
                active.terminals.contains(where: Self.orchestrators(in: worktree).contains),
                let shown = Self.shownLayout(c.layouts[wsID], of: worktree),
                shown.id != active.id
            {
                Task {
                    await act(.arrange, on: worktree) { client in
                        await client.selectLayout(shown.id, in: worktree)
                    }
                }
            }

            if let arrived {
                // Stamped here rather than in the palette, so every way of
                // arriving counts: a navigator click, ⌘], ⌃B o, a jump from the
                // palette itself. A switcher that only learned from its own
                // choices would order by where you had used IT, not by where you
                // have been.
                VisitLog.shared.visited(arrived.terminal)
            }

            // Opening a terminal is what ends `done`. Being LISTED is still not
            // being read — the palette lists every terminal on the runner and
            // clearing a notification nobody read is worse than not sending one
            // — but being on screen is, which is more than the pane you clicked.
            markVisibleSeen()
        }
        .sheet(isPresented: $showShortcuts) { ShortcutsSheet() }
        .sheet(item: $newWorkspaceName) { intent in
            NewWorkspaceSheet(
                repositories: workspaceRepositories, name: intent.name,
                takenPrefixes: { host in Set((store.fleet.runnerWorkspaces[host] ?? []).map(\.taskPrefix)) }
            ) { host, repository, name, prefix in
                if let why = store.refusal(for: host) { return why }
                guard let client = store.clients[host] else { return "That runner isn’t connected." }
                let made = await client.createWorkspace(repository: repository, name: name, prefix: prefix)
                if let workspace = made.made {
                    selection = .workspace(host: host, workspace: workspace.id, focus: nil)
                }
                return made.refusal
            }
        }
        .sheet(isPresented: $showResumeBranch) {
            ResumeBranch(
                projects: store.repositories,
                project: $lastProject,
                load: { host, project in await store.clients[host]?.branches(project: project) ?? [] },
                onAdopt: { branch, host, project, preset in
                    lastProject = project
                    resume(branch: branch.name, host: host, project: project, agent: preset)
                }
            )
        }
        // An overlay, not a sheet. A sheet dims the window and takes it over,
        // which is the wrong weight for something you use several times in a
        // row and want to see the results of behind.
        .overlay(alignment: .top) {
            if showQuickCreate {
                QuickCreate(
                    projects: store.repositories,
                    project: $lastProject,
                    onSubmit: { request in
                        lastProject = request.project
                        return await startTask(request)
                    },
                    onResume: {
                        showQuickCreate = false
                        showResumeBranch = true
                    },
                    onClose: { showQuickCreate = false },
                    branchPrefix: { host in store.clients[host]?.fleet.branchPrefix ?? "" },
                    submission: taskSubmission
                )
                .padding(.top, 14)
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.snappy(duration: 0.16), value: showQuickCreate)
        // The same weight as quick-create, and for the same reason: an editor
        // that would not start is something to read and act on, not a decision
        // to be interrupted for. Below quick-create's padding so the two do not
        // land on top of each other in the rare moment both are up.
        .overlay(alignment: .top) {
            if let editorError {
                ErrorBanner(message: editorError) { self.editorError = nil }
                    .padding(.top, 14)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.snappy(duration: 0.16), value: editorError)
        // Under the title bar's field (ov-214): the activity, the message's
        // one action, or the results; and the field itself where the status
        // area is too narrow to hold it.
        .overlay(alignment: .top) {
            if console.console.isOpen {
                TitleConsoleDropdown(
                    model: console, actions: consoleActions, status: statusActions, source: statusSource,
                    showsField: !TitleStatus.fieldInline(layout.form))
                    .padding(.top, Spacing.tight)
                    .transition(reduceMotion ? AnyTransition.opacity : AnyTransition.opacity.combined(with: .move(edge: .top)))
            }
        }
        .animation(.snappy(duration: 0.12), value: console.console.isOpen)
        .onReceive(
            NotificationCenter.default.publisher(
                for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            // The user may have approved the login item in System Settings
            // while we were in the background; nothing tells us but this.
        }
        .sheet(isPresented: $showAdd) {
            AddView()
        }
        .sheet(isPresented: $showAddRepository) {
            AddRepositorySheet(
                hosts: store.hosts,
                roots: store.roots,
                onAddRoot: { host, path in
                    if let why = store.refusal(for: host) { return why }
                    guard let client = store.clients[host] else {
                        return "that runner is not connected"
                    }
                    let failure = await client.addRoot(path)
                    await client.refreshRoots()
                    return failure
                },
                onRegister: { host, path in
                    if let why = store.refusal(for: host) { return why }
                    guard let client = store.clients[host] else {
                        return "that runner is not connected"
                    }
                    return await client.registerRepository(path)
                },
                onRegistered: { host in
                    Task {
                        // Registering adopts every worktree the repository
                        // already has, main checkout included, so there is
                        // nothing left to offer to import. Refreshed on the
                        // runner it was registered on — there is no event
                        // push for a repository or root change, so any other
                        // host would leave that host's switcher and navigator stale.
                        await store.clients[host]?.refreshRepositories()
                        await store.clients[host]?.refresh()
                    }
                }
            )
        }
        .sheet(item: $newWorktreeIntent) { intent in
            NewWorktreeSheet(
                repositories: store.repositories,
                preselected: intent.project,
                // Both halves off the one intent, so they cannot disagree
                // about which project this is. An empty host is ⌘N's and the
                // empty-state buttons, none of which named a
                // runner, so the sheet's own picker is left to choose one.
                preselectedHost: intent.host,
                branchPrefix: { host in store.clients[host]?.fleet.branchPrefix ?? "" }
            ) { host, repo, task, branch, base in
                // The reason alone, not "Cannot do that: " + it. The sheet
                // supplies its own sentence now and shows this in a
                // `DetailBox` underneath, and `HostState.refusal` is mostly
                // the stderr of whatever last failed to reach the runner — so
                // the prefix was this app's words joined to a runner's with a
                // colon, the join `e0f72df` took out of the phone. The banner
                // path in `act` says its own sentence instead
                // (`ActionCopy.refused`).
                if let why = store.refusal(for: host) { return why }
                guard let client = store.clients[host] else {
                    return "that runner is not connected"
                }
                // Claimed as ⌘N claims: for the workspace you're in when
                // it's in this repository, else the repository's Main. The
                // sheet names the repository by its short id.
                let repository = store.repositories.first {
                    $0.host == host && $0.repository.short == repo
                }?.repository.id
                let created = await client.createWorktree(
                    repo: repo, task: task, branch: branch, base: base,
                    workspace: repository.flatMap {
                        Self.claim(newWorktreeIn: $0, on: host, from: selection, in: store.fleet)
                    })
                if let failure = created.failure { return failure }
                // Land in the terminal it came up with, exactly as starting a
                // task does. Creating a worktree is not a filing act — you make
                // one because you are about to work in it — and `reveal` is the
                // one place that says what "go to it" means: select its
                // terminal.
                reveal(created.worktree)
                return nil
            }
        }
        .sheet(item: $removeWorktree) { ws in
            RemoveWorktreeSheet(
                worktree: ws,
                // `Running | Starting`, matching what the daemon actually
                // closes: a terminal mid-launch is as alive as one already
                // confirmed. A count now rather than a flag — removal closes
                // them, so the sheet reports the consequence instead of
                // refusing over it.
                runningCount: ws.terminals.filter {
                    let kind = StateKind.parse($0.state)
                    return kind == .running || kind == .starting
                }.count
            ) { typed in
                // The sheet shows this one's failures itself; `removeWorktree`
                // runs through `runRaw`, so only a refusal reaches a banner.
                let result = await act(
                    .removeWorktree, on: ws, default: .failed("This runner can’t be reached right now.")
                ) { c in
                    await c.removeWorktree(ws.short, confirm: typed)
                }
                // Healed as though the fleet had already dropped it, by the
                // same rule the fleet change runs. Whichever of the two lands
                // first, the second finds the selection already off `ws` and
                // changes nothing, so where you end up no longer depends on
                // which finished first. It used to set nil here only when the
                // selection was still one of `ws`'s terminals, so if the fleet
                // arrived first you kept wherever that had put you.
                if case .ok = result {
                    let without = store.fleet.worktrees.filter {
                        !(($0.host ?? "") == (ws.host ?? "") && $0.id == ws.id)
                    }
                    let next = Self.healed(
                        selection, in: without, was: [ws], workspaces: store.fleet.runnerWorkspaces)
                    if next != selection { selection = next }
                }
                return result
            }
        }
        .renameTerminalSheet($renaming) { target, typed in
            await act(.rename, on: target.worktree, target: target.terminal.id, subject: Self.quoted(target.terminal)) { c in
                await c.rename(terminal: target.terminal.short, to: typed)
            }
        }
        .sheet(item: $removeRepository) { target in
            if let found = store.rootAndSiblings(of: target.repository, host: target.host) {
                RemoveRepositorySheet(
                    repository: target.repository,
                    root: found.root,
                    siblings: found.siblings
                ) { confirm in
                    if let why = store.refusal(for: target.host) { return .failed(why) }
                    guard let client = store.clients[target.host] else {
                        return .failed("that runner is not connected")
                    }
                    return await client.removeRoot(found.root.id, confirm: confirm)
                }
            } else {
                // Stale data: this repository's own root is missing from
                // `store.roots`. Refreshing rather than presenting a sheet
                // with nothing true to say about what it would remove.
                ProgressView()
                    .frame(width: 200, height: 120)
                    .task {
                        await store.clients[target.host]?.refreshRoots()
                        removeRepository = nil
                    }
            }
        }
        .confirmationDialog(
            orchestratorReplacement.map { "Replace \($0.workspace.name)’s orchestrator?" } ?? "",
            isPresented: Binding(
                get: { orchestratorReplacement != nil },
                set: { if !$0 { orchestratorReplacement = nil } }),
            presenting: orchestratorReplacement
        ) { pending in
            Button("Replace Orchestrator", role: .destructive) {
                startOrchestrator(
                    pending.workspace, host: pending.host, harness: pending.harness, replace: true)
            }
            Button("Cancel", role: .cancel) {}
        } message: { pending in
            Text("The orchestrator running now closes, and a new one starts with \(pending.harness.title).")
        }
        .confirmationDialog(
            adoptionPending.map { "Replace \($0.old.terminal.label) as \($0.workspace.name)’s orchestrator?" } ?? "",
            isPresented: Binding(
                get: { adoptionPending != nil },
                set: { if !$0 { adoptionPending = nil } }),
            presenting: adoptionPending
        ) { pending in
            Button("Use \(pending.pane.terminal.label)") {
                Task { await adopt(pending.pane, in: pending.workspace, host: pending.host, replacing: pending.old) }
            }
            Button("Cancel", role: .cancel) {}
        } message: { pending in
            Text(
                "\(pending.old.terminal.label) keeps running as an ordinary terminal, and "
                    + "\(pending.pane.terminal.label) runs \(pending.workspace.name)’s board.")
        }
        // A dialog rather than a sheet: it is one yes-or-no question, and the
        // replacement orchestrator confirmations above are dialogs already.
        .confirmationDialog(
            "Stop this turn and switch modes?",
            isPresented: Binding(
                get: { pendingPaneModeSwitch != nil },
                set: { if !$0 { pendingPaneModeSwitch = nil } }),
            presenting: pendingPaneModeSwitch
        ) { pending in
            Button("Stop Turn and Switch", role: .destructive) {
                Task {
                    await act(
                        .switchMode, on: pending.worktree,
                        target: pending.worktree.terminals.first(where: { $0.short == pending.terminal })?.id
                            ?? pending.terminal,
                        subject: pending.worktree.terminals.first(where: { $0.short == pending.terminal }).map { Self.quoted($0) }
                            ?? "this pane",
                        default: .failed("This runner can’t be reached right now.")
                    ) { c in
                        await c.setPaneMode(pending.terminal, mode: pending.mode, force: true)
                    }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text("This pane has a turn in flight. Switching modes stops it.")
        }
    }
}

/// What Esc does in a search or filter field: the navigator's filter, and
/// History's search.
enum SearchEscape {
    /// With text, clears it and keeps the field; empty, leaves the field.
    static func after(query: String) -> (query: String, keepsFocus: Bool) {
        query.isEmpty ? ("", false) : ("", true)
    }
}

enum TerminalAction {
    case restart, dismissLost, stop, useAsOrchestrator, stopBeingOrchestrator
    /// Stop it and remove its record (ov-234): the Close on a task's terminals.
    case close
    /// Ask for a new name (`RenameTerminalSheet`).
    case rename
    /// Open its listening port in this Mac's browser.
    case openInBrowser
}

/// The banners over the detail pane: the latest few results, and a row for
/// the rest when there are more (`ActionOutcomes.visibleLimit`).
struct ActionBanners: View {
    @ObservedObject var outcomes: ActionOutcomes

    var body: some View {
        VStack(spacing: 0) {
            ForEach(outcomes.visible) { failure in
                ErrorBanner(message: failure.sentence) { outcomes.dismiss(failure.key) }
            }
            if outcomes.hiddenCount > 0 || outcomes.expanded {
                HStack(spacing: 12) {
                    if outcomes.hiddenCount > 0 {
                        Text(outcomes.hiddenCount == 1 ? "1 more failure" : "\(outcomes.hiddenCount) more failures")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 8)
                    Button(outcomes.expanded ? "Show Fewer" : "Show All") { outcomes.expanded.toggle() }
                    Button("Dismiss All") { outcomes.dismissAll() }
                }
                .buttonStyle(.link)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .floatingPanel()
                .padding(.horizontal, 16)
                .padding(.top, 10)
            }
        }
    }
}

/// One result the app wrote: an action's failure, or the notice.
///
/// One per entry in `ActionOutcomes.shown`, stacked, each with its own close
/// button: two actions that failed are two things to read, and closing one
/// must not take the other with it.
struct ErrorBanner: View {
    let message: String?
    let onDismiss: () -> Void

    var body: some View {
        if let message {
            HStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(message)
                    .font(.callout)
                    .textSelection(.enabled)
                Spacer(minLength: 8)
                Button {
                    onDismiss()
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .floatingPanel()
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .transition(.opacity.combined(with: .move(edge: .top)))
        }
    }
}
