import AgentKit
import AppKit
import SwiftUI

struct ContentView: View {
    @StateObject var store = FleetStore()
    /// This window's identity in `Notifier`'s per-window record of what is on
    /// screen, so a second window adds to it rather than overwriting it.
    @State var windowID = UUID()
    @ObservedObject private var preferences = Preferences.shared
    @ObservedObject private var themes = Themes.shared
    /// A click on a notification, waiting to be opened (ov-106, ov-183).
    @ObservedObject var noticeOpener = DestinationOpener.shared
    @Environment(\.openSettings) var openSettings
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.windowVisible) private var windowVisible
    @Environment(\.markReadConfirmation) private var markReadConfirmation
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
    @State private var changesStores: [String: ChangesStore] = [:]
    /// One board store per workspace, keyed by runner and workspace id — the
    /// repository's id for a runner without workspaces, whose one board per
    /// repository is the repository's.
    ///
    /// Keyed by both because an id is minted per daemon: two runners can
    /// hand back rows that collide on id alone. Same lifetime rule as
    /// `changesStores` — see `boardStore(for:client:host:)`.
    @State var boardStores: [String: TaskBoardStore] = [:]
    @State var showAddRepository = false
    @State private var showAdd = false
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
    @AppStorage(SelectionMemory.destinationKey) private var lastDestination = ""
    /// What an earlier build kept, the selection alone: read when no
    /// `lastDestination` has been written yet. It replaced `fleet.lastTerminal`,
    /// which `SelectionMemory.migrate` maps once.
    @AppStorage(SelectionMemory.key) private var lastSelection = ""
    /// Whether the launch has chosen where the window opens. See
    /// `settleLaunch`.
    @State private var launched = false
    /// Where this window is going back to, while it waits for its runner.
    @State var restoring: DestinationOpen?
    /// Bumped by ⌘F in a workspace: its navigator's filter takes the
    /// keyboard (ov-103).
    @State private var boardFilterRequest = 0
    @State private var removeWorktree: Worktree?
    @State private var removeRepository: RepositoryToRemove?
    @State private var showResumeBranch = false
    @State var showPalette = false
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
    @State private var orchestratorReplacement: OrchestratorReplacement?
    /// Use as Orchestrator on a workspace that has one, until confirmed.
    @State var adoptionPending: OrchestratorAdoptionPending?

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
    @AppStorage("workspace.navigatorWidth") private var navigatorWidth = Double(WorkspaceColumns.navigatorDefault)
    /// Asks the navigator for the keyboard: bumped by ⌥⌘2, and by a click
    /// on a row, so ↑ and ↓ walk it from there.
    @State var boardFocusRequest = 0
    /// Focus (⌃⌘↩): a task or worktree opened, alone, without the
    /// navigator.
    @State var focusColumn = false
    /// The detail's width, as the workspace view last measured it: which of
    /// a workspace's columns are on screen. Nil until one has been drawn.
    @State private var detailWidth: CGFloat?
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
    /// The key monitor that turns Esc into Back. See `EscapeBack`.
    @State private var escapeMonitor: Any?
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
    /// Bumped by ⌘0 (Switch Workspace…): the title bar's switcher opens.
    @State private var switcherRequest = 0
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
                changes: changesToolbarState, onChanges: { ws in toggleChangesPane(in: ws) })
        }
        .titleBarStatus(titleStatusSource, room: titleStatusRoom, actions: titleStatusActions)
        // The title would repeat the switcher or the breadcrumb
        // (`TitleBar`); the window keeps it for the Window menu.
        .toolbar(removing: TitleBar.showsTitle(for: selection) ? nil : .title)
        .toolbar {
            LeadingToolbar(
                switcher: workspaceSwitcher,
                navigator: NavigatorToggle(
                    hidden: navigatorHidden, available: selection.flatMap(workspaceScene)?.board != nil,
                    toggle: { toggleNavigator() }))
        }
        // The compact toolbar (ov-214), on whatever made the window.
        .mainWindowChrome()
        .overlay(alignment: .top) {
            ActionBanners(outcomes: outcomes)
        }
        // A message arriving on a keystroke, so the same snappy preset
        // `PrefixHintOverlay` uses for its chip.
        .animation(.snappy(duration: 0.22), value: outcomes.shown)
        .task {
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
            // Every runner's own fleet, repositories, roots, and layouts are
            // already being brought up by `FleetStore` — see `rebuild()`.
            // Its event stream is a separate question: `rebuild()` starts
            // one only the first time a client is added, and `.onDisappear`
            // below stops every one of them on the way out. `resume()` is
            // this `.task`'s half of that pair — without it, a window that
            // closes and reopens (⌘W, then the Dock) comes back with every
            // client's `state` still reading whatever it was, but nothing
            // actually listening.
            store.resume()
            settleLaunch()
        }
        .onDisappear {
            // What this window showed is shown no longer; the runners are told
            // what the remaining windows still show.
            Notifier.shared.closeWindow(windowID)
            DestinationOpener.shared.unregister(window: windowID)
            for client in store.clients.values {
                client.stopEvents()
                client.reportWatching([])
            }
            if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
            escapeMonitor = nil
        }
        // Esc goes Back when nothing that needs it has the keyboard, in this
        // window, with no sheet or overlay up.
        .background(WindowReader(box: windowBox))
        .onAppear {
            guard escapeMonitor == nil else { return }
            escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                guard event.keyCode == 53, event.modifierFlags.intersection(.deviceIndependentFlagsMask).isEmpty,
                    let window = event.window, window === windowBox.window, window.attachedSheet == nil,
                    !showPalette, !showQuickCreate, !jumpBar.active,
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
        .onTileCommand { command in Task { await tile(command) } }
        .onSelectIndex { index in if isKeyWindow { selectTerminal(at: index) } }
        .onSelectWorkspace { number in
            guard isKeyWindow else { return }
            if let target = WorkspaceNumbers.target(number, in: WorkspaceNumbers.groups(in: store.fleet)) {
                selection = target
            }
        }
        .onChange(of: navigatorHidden) { _, hidden in
            UserDefaults.standard.set(NavigatorVisibility.stored(hidden), forKey: NavigatorVisibility.key)
        }
        .onChange(of: store.layouts) { _, _ in followLayoutFocus() }
        // Every client change reaches `store.fleet` — `FleetStore` remerges
        // on each one — so this hears a runner leaving, coming back as a new
        // client, and listing its projects without one.
        .onReceive(store.$fleet) { _ in pruneBoardStores() }
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
            if let now { lastDestination = now }
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
        // Centered and over everything, unlike quick-create. This one is not
        // something you work alongside — it is a switcher, it is on screen for
        // about a second, and while it is there every keystroke belongs to it.
        // The scrim is what makes that true for the mouse as well: a panel this
        // large with a live terminal showing round the edges invites a click
        // that lands somewhere surprising.
        .overlay {
            if showPalette {
                ZStack {
                    // 0.25, not the 0.12 this shipped with. A scrim's job is
                    // stated one comment up — read as modal — and 0.12 black
                    // over a terminal on a dark theme is under the threshold
                    // where anything looks different, so the panel floated over
                    // a window that still looked live and clickable.
                    Color.black.opacity(0.25)
                        .ignoresSafeArea()
                        .onTapGesture { showPalette = false }
                    CommandPalette(
                        worktrees: store.fleet.worktrees,
                        workspaces: paletteWorkspaces,
                        tasks: paletteTasks,
                        offersNewWorkspace: !workspaceRepositories.isEmpty,
                        current: selectedPane,
                        screen: { short in await screen(forTerminalShort: short) },
                        onRun: { perform($0) },
                        onClose: { showPalette = false }
                    )
                }
                .transition(.opacity)
            }
        }
        .animation(.snappy(duration: 0.14), value: showPalette)
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

    // MARK: - Detail

    /// `OrchestratorAdoption.offer` in `worktree`, as the fleet has it when
    /// the menu opens.
    private func roleOffer(in worktree: Worktree) -> (Terminal) -> OrchestratorAdoption.Offer? {
        let store = store
        return { OrchestratorAdoption.offer(for: $0, in: worktree, host: worktree.host ?? "", fleet: store.fleet) }
    }

    /// This worktree's changes pane, if it has one open in the layout the
    /// detail draws for it, in a workspace or loose.
    ///
    /// Asked of that layout rather than of the worktree's terminals,
    /// because that is the question the toolbar button is answering: whether
    /// the arrangement you are looking at is showing the diff. A changes pane
    /// in a layout two tabs over is not on screen, and offering to close it
    /// from here would close something the window is not showing. Never the
    /// Orchestrator column's: nothing is split into the orchestrator's
    /// window (ov-78).
    private func changesPane(in ws: Worktree) -> Terminal? {
        guard let group = worktreeColumn(of: ws)?.group else { return nil }
        let inGroup = Set(group.panes.map(\.id))
        return ws.terminals.first { inGroup.contains($0.id) && $0.isChangesPane }
    }

    /// The layout the detail draws for `ws` opened whole: beside a board in a
    /// workspace, or on its own. Nil when it draws none, which is the
    /// case for a worktree with no terminal.
    private func worktreeColumn(of ws: Worktree) -> ShownLayout? {
        shown.first { $0.column == .worktree && $0.host == (ws.host ?? "") && $0.worktree.id == ws.id }
    }

    /// Open this worktree's diff, or close the one the toolbar says is open.
    private func toggleChangesPane(in ws: Worktree) {
        if let open = changesPane(in: ws) {
            // Killing the pane is the whole of it. The record goes with it, but
            // the daemon does that — closing a diff from tmux's own `⌃B x` has
            // to leave as little behind as closing it from here, so the reaping
            // lives on the host where both can reach it, not in this button.
            Task {
                await act(.close, on: ws, target: open.id, subject: Self.quoted(open)) { c in
                    await c.stop(terminal: open.short)
                }
            }
            return
        }
        showChanges(in: ws)
    }

    /// Show Changes: the toolbar's, a worktree row's menus' and its card's
    /// (ov-78). Reachable whether or not the worktree has a terminal open.
    ///
    /// With the worktree's layout open, a split of its
    /// focused pane, exactly as `⌃B %` and a drop on an edge are: the daemon
    /// has one verb for "a new pane, here, running this", and a changes pane
    /// is that verb with a different preset. With no layout there (no
    /// terminal, or the main checkout named from the Orchestrator column),
    /// its changes pane, one it has or a new one in a window of its own,
    /// opened beside the board. Never a split of the orchestrator's
    /// window.
    private func showChanges(in ws: Worktree) {
        if let open = changesPane(in: ws) {
            focus(PaneRef(host: ws.host ?? "", worktree: ws.id, terminal: open.id))
            return
        }
        if let layout = worktreeColumn(of: ws) {
            Task {
                let groups = await act(.openChanges, on: ws, default: []) { c in
                    // `beside: nil` means the focused pane of the layout
                    // named, which is the daemon's own default and the same
                    // anchor `⌃B %` uses.
                    await c.split(ws, beside: nil, side: .right, preset: "changes", layout: layout.group.id)
                }
                reveal(groups, in: ws)
            }
            return
        }
        // Where it opens: in the workspace on screen when the toolbar named
        // this worktree, as its row does otherwise.
        let stays = WorkspaceScreen.changesTarget(
            selection, in: store.fleet, repositories: repositoryIDs(ws.host ?? ""))?.id == ws.id
        let listed = worktree(host: ws.host ?? "", id: ws.id) ?? ws
        Task {
            let host = listed.host ?? ""
            let layouts = store.client(for: listed)?.layouts[listed.id]
            // One it has already, unless it's in an orchestrator's window.
            let existing = listed.terminals.first {
                $0.isChangesPane
                    && WorkspaceScreen.seat(sharedBy: $0.id, in: listed, fleet: store.fleet, layouts: layouts) == nil
            }
            var pane = existing
            if pane == nil {
                pane = await act(
                    .openChanges, on: listed, default: nil as Terminal?,
                    { c in await c.createTerminal(in: listed, preset: "changes", title: "Changes") })
            }
            guard let pane else { return }
            if stays, case .workspace(host, let id, _)? = selection {
                selection = .workspace(host: host, workspace: id, focus: .worktree(listed.id, terminal: pane.id))
            } else {
                selection = Self.opening(listed, terminal: pane.id, in: store.fleet)
            }
            keyPane = PaneRef(host: host, worktree: listed.id, terminal: pane.id)
        }
    }

    /// Show Changes on `ws`'s row, or nil where it can't be: a runner that
    /// can't be acted on, or that has said it can't read changes.
    private func showChangesAction(for ws: Worktree, usable: Bool) -> (() -> Void)? {
        guard usable, store.client(for: ws)?.changesSupported != false else { return nil }
        return { showChanges(in: ws) }
    }

    /// The repositories `host` lists, by id: what an implicit workspace is.
    private func repositoryIDs(_ host: String) -> [String] {
        store.clients[host]?.repositories.map(\.id) ?? []
    }

    /// One board store per workspace.
    ///
    /// Cached on the client for the reason `changesStore(for:client:)` gives:
    /// `FleetStore` drops a `DaemonClient` when its runner leaves and builds a
    /// fresh one when it comes back, and a store held over from the old one
    /// would go on talking to a connection nobody is answering. A workspace
    /// renamed since is a new store too, so the board's title follows it.
    func boardStore(for workspace: WorkspaceSummary, client: DaemonClient, host: String)
        -> TaskBoardStore
    {
        let key = "\(host)/\(workspace.id)"
        if let existing = boardStores[key], Self.keeps(existing, for: workspace, client: client) {
            if existing.onChoose == nil {
                existing.onChoose = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: false) }
                existing.onGlance = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: true) }
            }
            return existing
        }
        let made = TaskBoardStore(client: client, workspace: workspace)
        made.onChoose = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: false) }
        made.onGlance = { row in chooseTask(row.id, host: host, workspace: workspace.id, glance: true) }
        // Outside the view update, because creating it IS a state change and
        // SwiftUI is reading that state right now. See `changesStore`.
        DispatchQueue.main.async { boardStores[key] = made }
        return made
    }

    /// Whether `existing` still serves `workspace`'s board on `client`.
    ///
    /// Compared by what the board shows — its name — and not the whole
    /// summary: an orchestrator starting or stopping changes the summary, and
    /// a new store would close the card that is open and read the board again.
    static func keeps(
        _ existing: TaskBoardStore, for workspace: WorkspaceSummary, client: DaemonClient
    ) -> Bool {
        existing.client === client && existing.workspace.id == workspace.id
            && existing.workspace.name == workspace.name
            && existing.workspace.isImplicit == workspace.isImplicit
    }

    /// The board stores still worth holding, given the runners there are now.
    /// See `TaskBoardStore.isHeld(by:)`.
    ///
    /// Before this, `boardStores` only ever grew: every repository a runner
    /// had ever listed, and every client a runner had ever had, stayed held
    /// for the life of the window. A dropped store's open card is closed on
    /// the way out, so a view still holding it for a moment has nothing to
    /// put back on screen.
    static func heldBoardStores(
        _ stores: [String: TaskBoardStore], clients: [String: DaemonClient]
    ) -> [String: TaskBoardStore] {
        let held = stores.filter { $0.value.isHeld(by: clients) }
        for (key, dropped) in stores where held[key] == nil { dropped.opened = nil }
        return held
    }

    private func pruneBoardStores() {
        let held = Self.heldBoardStores(boardStores, clients: store.clients)
        if held.count != boardStores.count { boardStores = held }
    }

    /// Whose board ⇧⌘B selects.
    ///
    /// The workspace of whatever the window is showing — Main's for an
    /// unclaimed worktree — and the only repository's Main when nothing is
    /// selected. Never a guess between several: opening the wrong board looks
    /// exactly like a workspace with somebody else's work on it. See
    /// `ContentView.boardWorkspace(for:in:)`.
    private var boardTarget: (host: String, workspace: WorkspaceSummary)? {
        if let selection, let host = selection.host {
            if let found = Self.boardWorkspace(for: selection, in: store.fleet) {
                return (host, found)
            }
            // A CLI too old to send `repository_id`, on a runner without
            // workspaces: the repository is found by the name it does send.
            if let ws = currentWorktree, let client = store.client(for: ws),
                let name = ws.repository,
                let match = client.repositories.first(where: { $0.displayName == name })
            {
                return (host, .implicit(repository: match.id))
            }
            return nil
        }
        let all = store.repositories
        guard all.count == 1, let only = all.first else { return nil }
        let main = store.fleet.runnerWorkspaces[only.host]?.first {
            $0.isMain && $0.repository == only.repository.id
        }
        return (only.host, main ?? .implicit(repository: only.repository.id))
    }

    /// The board a `.board` selection names, as its runner lists it now: a
    /// workspace it lists, or on a runner without workspaces the repository
    /// whose id it is. Nil once it is gone.
    private func board(host: String, id: String) -> WorkspaceSummary? {
        guard let client = store.clients[host] else { return nil }
        if let listed = client.fleet.workspaces {
            return listed.first { $0.id == id }
        }
        guard client.repositories.contains(where: { $0.id == id }) else { return nil }
        return .implicit(repository: id)
    }

    /// One changes store per worktree.
    ///
    /// Cached on the client too, not just the worktree. `FleetStore` drops a
    /// `DaemonClient` when its runner leaves and builds a fresh one when it
    /// comes back, and a store held over from the old one would go on talking to
    /// a connection nobody is answering.
    private func changesStore(for ws: Worktree, client: DaemonClient) -> ChangesStore {
        if let existing = changesStores[ws.id], existing.client === client { return existing }
        let made = ChangesStore(client: client, worktree: ws)
        // Assigned outside the view update, because creating it IS a state
        // change and SwiftUI is reading that state right now. An earlier version
        // wrote it from a Task, which rebuilt the store on every render and threw
        // away each load before it could finish — the panel sat permanently empty.
        DispatchQueue.main.async { changesStores[ws.id] = made }
        return made
    }

    /// Every layout the detail draws for `selection`: `WorkspaceScreen`'s,
    /// and of a workspace's, only the columns its width draws.
    func shownLayouts(for selection: Selection?) -> [ShownLayout] {
        let all = drawableLayouts(for: selection)
        guard let selection, let scene = workspaceScene(selection) else { return all }
        let arrangement = detailWidth.map { _ in
            WorkspaceColumns.layout(
                opened: scene.opened != nil, hasConversation: scene.hasConversation, hasBoard: scene.board != nil,
                focused: focusColumn)
        }
        return WorkspaceScreen.visible(all, arrangement: arrangement, taskTab: taskTab(for: selection))
    }

    /// Whether an agent is working `task`: what opens it on its Agent tab.
    private func agentWorking(_ task: String, host: String) -> Bool {
        WorkspaceScreen.agent(of: task, host: host, in: store.fleet, chosen: chosenAgents[task]) != nil
    }

    /// The tab the task `selection` names shows (ov-98), or Agent for
    /// anything else, which has no tabs.
    func taskTab(for selection: Selection?) -> TaskTab {
        guard case .workspace(let host, _, .task(let id)?)? = selection else { return .agent }
        return taskTabs.tab(for: id, agentWorking: agentWorking(id, host: host))
    }

    /// Whether `selection` has a task open, with its tabs.
    nonisolated static func taskOpen(_ selection: Selection?) -> Bool {
        if case .workspace(_, _, .task?)? = selection { return true }
        return false
    }

    /// Show `tab` for `task`, chosen; to the Agent tab, the keyboard goes
    /// with it, into the terminal. Leaving Changes, its diff gives up the
    /// Diff menu's keys.
    func choose(_ tab: TaskTab, for task: String) {
        taskTabs.choose(tab, for: task)
        if tab != .changes, changesFocus == task { changesFocus = nil }
        if tab == .agent { DispatchQueue.main.async { keyOpened() } }
    }

    /// ⌃⌘] and ⌃⌘[: the task open steps to its next or previous tab.
    private func stepTaskTab(by offset: Int) {
        guard case .workspace(let host, _, .task(let id)?)? = selection else { return }
        var stepped = taskTabs
        stepped.step(id, by: offset, agentWorking: agentWorking(id, host: host))
        choose(stepped.tab(for: id, agentWorking: agentWorking(id, host: host)), for: id)
    }

    /// What the detail draws now.
    var shown: [ShownLayout] { shownLayouts(for: selection) }

    /// Every layout `selection` could draw, on screen or not: the
    /// conversation's included while it's kept hidden behind a task, so it's
    /// one terminal view whether it's selected or not.
    private func drawableLayouts(for selection: Selection?) -> [ShownLayout] {
        func shown(_ selection: Selection?) -> [ShownLayout] {
            WorkspaceScreen.shown(
                selection, in: store.fleet,
                layouts: { host, worktree in store.clients[host]?.layouts[worktree] },
                repositories: { host in store.clients[host]?.repositories.map(\.id) ?? [] },
                chosen: { chosenAgents[$0] })
        }
        // A loose worktree beside a workspace's board keeps that
        // workspace's orchestrator mounted, ahead of the worktree.
        if case .looseWorktree(let host, _, _)? = selection, let scene = workspaceScene(selection!),
            scene.hasConversation, let board = scene.board
        {
            let conversation = shown(.workspace(host: host, workspace: board, focus: nil)).filter { $0.column == .conversation }
            return conversation + shown(selection)
        }
        return shown(selection)
    }

    /// `detail`, and a click on a task notice: its task, opened as the
    /// navigator opens one, once its runner is connected (ov-106). Its own
    /// property because `body`'s chain is already at the type checker's
    /// limit: one more modifier there and it gives up.
    private var detailOpeningNotices: some View {
        detail
            .task(id: noticeOpener.pending?.id) { await openNoticedTask() }
            .task(id: restoring?.id) { await restoreWhereYouWere() }
    }

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .needsYou:
            NeedsYouView(
                items: store.needsYou,
                olderRunners: store.hosts.filter { store.clients[$0]?.needsYouFromOlderRunner == true },
                canAct: { item in
                    store.refusal(for: item.runner) == nil
                        && TaskBoardWrites.offered(by: store.clients[item.runner]?.daemonBuild)
                },
                onOpen: { open($0) },
                onAnswerAsk: { item, option in
                    guard let client = store.clients[item.runner], let terminal = item.terminal,
                        let ask = item.askID
                    else { return .failed }
                    return await client.answerAsk(terminal: terminal.id, request: ask, option: option)
                },
                onDecide: { item, body in
                    guard let client = store.clients[item.runner], let task = item.task,
                        let repository = item.repositoryID
                    else { return false }
                    return await client.answerDecision(key: task.key, body: body, repository: repository) == nil
                })

        // One branch for both, so a loose worktree and the workspace whose
        // board is beside it are one view: going from one to the other keeps
        // the board, its scroll and its visit, and moves on the spring.
        case .workspace, .looseWorktree:
            if let scene = selection.flatMap(workspaceScene) {
                workspaceDetail(scene)
                    .id(scene.key)
            } else {
                placeholder
            }

        case nil:
            placeholder
        }
    }

    /// What a workspace's detail draws for a selection: whose board, whether
    /// it has a conversation, and what's open beside the board.
    struct WorkspaceScene {
        var host: String
        /// The board's workspace: the selection's own, or for a loose
        /// worktree the one `boardWorkspace(for:in:)` finds, if any.
        var board: String?
        var summary: WorkspaceSummary?
        var hasConversation: Bool
        /// What's open beside the board, as a place: the pane a worktree
        /// names left out, so choosing another pane in it is the same one.
        var opened: Selection?
        /// The view's identity: another workspace is drawn afresh, not moved
        /// to.
        var key: String { "\(host)|\(board ?? "")" }
    }

    func workspaceScene(_ selection: Selection) -> WorkspaceScene? {
        Self.workspaceScene(
            selection, in: store.fleet, repositories: store.clients[selection.host ?? ""]?.repositories.map(\.id) ?? [])
    }

    /// The scene `selection` draws, from `fleet`: a workspace's own board
    /// and orchestrator; for a loose worktree, its repository's board
    /// (`boardWorkspace(for:in:)`) and that board's orchestrator, kept
    /// mounted, so neither the navigator nor the orchestrator comes and goes
    /// between the two.
    nonisolated static func workspaceScene(
        _ selection: Selection, in fleet: Fleet, repositories: [String]
    ) -> WorkspaceScene? {
        switch selection {
        case .workspace(let host, let id, let focus):
            let summary = WorkspaceScreen.workspace(id, host: host, in: fleet, repositories: repositories)
            return WorkspaceScene(
                host: host, board: id, summary: summary, hasConversation: summary.map { !$0.isImplicit } ?? false,
                opened: focus == nil ? nil : WorkspaceSelection.place(selection))
        case .looseWorktree(let host, _, _):
            let found = boardWorkspace(for: selection, in: fleet)
            return WorkspaceScene(
                host: host, board: found?.id, summary: found, hasConversation: found.map { !$0.isImplicit } ?? false,
                opened: WorkspaceSelection.place(selection))
        case .needsYou:
            return nil
        }
    }

    // MARK: - The title bar, the toolbar and the worktree menus (ov-86)

    /// Whether this is the window menu commands are for: the key one, or
    /// the only one before its window is known.
    private var isKeyWindow: Bool { windowBox.window?.isKeyWindow ?? true }

    /// The title bar's workspace switcher, naming where the window is.
    private var workspaceSwitcher: WorkspaceSwitcherButton {
        let scene = selection.flatMap(workspaceScene)
        let repository = scene?.summary.flatMap { w in
            store.clients[scene?.host ?? ""]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        let title: String = {
            if selection == .needsYou { return "Needs You" }
            guard let summary = scene?.summary else { return "Workspaces" }
            return summary.isImplicit ? "Main" : summary.name
        }()
        let entries = WorkspaceSwitcherMenu.entries(
            groups: WorkspaceNumbers.groups(in: store.fleet),
            current: scene.flatMap { s in s.board.map { (s.host, $0) } },
            waiting: { place in WorkspaceCounts.count(for: place.workspace, host: place.host, in: store.needsYou) },
            showsHosts: showHosts, needsYou: store.needsYou.count, offersNewWorkspace: !workspaceRepositories.isEmpty,
            status: store.reading.sentence, statusTrouble: store.reading.isTrouble, troubled: store.unhealthyHosts)
        return WorkspaceSwitcherButton(
            title: title, repository: scene?.summary == nil ? "" : repository, entries: entries,
            openRequest: switcherRequest, perform: { perform($0) })
    }

    /// Show Changes in the toolbar, where offered (`WorktreeToolbar`). Not
    /// on a runner that has already said it can't read changes at all: its
    /// daemon predates the feature, and the split would open a dead pane
    /// where a diff was asked for. For a worktree opened whole, or the main
    /// checkout beside the orchestrator (ov-78); not for a task, whose
    /// column shows its changes already (spec R3).
    private var changesToolbarState: WorktreeToolbar.Changes? {
        guard
            let ws = WorkspaceScreen.changesTarget(
                selection, in: store.fleet, repositories: repositoryIDs(selection?.host ?? "")),
            store.client(for: ws)?.changesSupported != false
        else { return nil }
        return WorktreeToolbar.Changes(worktree: ws, open: changesPane(in: ws) != nil)
    }

    /// The title bar's status area for the workspace on screen (ov-214):
    /// its orchestrator as the navigator's row says it, and its board. Nil
    /// with no workspace on screen.
    private var titleStatusSource: TitleStatusSource? {
        guard let scene = selection.flatMap(workspaceScene), let summary = scene.summary,
            let client = store.clients[scene.host]
        else { return nil }
        let host = scene.host
        let orchestrator = scene.hasConversation ? navigatorOrchestrator(host: host, workspace: summary) : nil
        let decisions = WorkspaceCounts.decisions(for: summary, host: host, in: store.needsYou)
        let read = scene.board == nil ? nil : boardStore(for: summary, client: client, host: host)
        let tasks = Set(read?.board.rows.map(\.id) ?? [])
        // The workspace's panes: its own, and any working one of its tasks.
        let panes = store.fleet.worktrees.filter { ($0.host ?? "") == host }.flatMap { worktree in
            worktree.terminals
                .filter { $0.workspace == summary.id || $0.taskId.map(tasks.contains) == true }
                .map { BoardPane(terminal: $0, worktree: worktree) }
        }
        return TitleStatusSource(
            orchestrator: orchestrator?.state, status: orchestrator?.status, nowDoing: orchestrator?.nowDoing,
            board: read,
            waiting: { column in client.boardWaiting(columnCount: column, decisions: decisions) },
            seat: WorkspaceScreen.orchestrator(of: summary, host: host, in: store.fleet)?.terminal, panes: panes)
    }

    /// What the status area sizes itself around (`TitleStatusRoom`).
    private var titleStatusRoom: TitleStatusRoom {
        let switcher = workspaceSwitcher
        return TitleStatusRoom(
            switcherTitle: switcher.title, switcherRepository: switcher.repository,
            editor: detailWorktree != nil, changes: changesToolbarState != nil,
            trouble: RunnerStatusItem.label(troubles: runnerTroubles, stale: store.staleHosts, ahead: store.aheadHosts),
            needsYou: store.needsYou.count)
    }

    /// What the status area's parts do, by the routes the window already
    /// has: the orchestrator's row, ⌃⌘N within this workspace, a task's row.
    private var titleStatusActions: TitleStatusActions {
        guard let scene = selection.flatMap(workspaceScene), let summary = scene.summary, let board = scene.board
        else { return TitleStatusActions() }
        let host = scene.host
        return TitleStatusActions(
            goToOrchestrator: { selectOrchestrator(keyboard: .conversation) },
            orchestratorMenu: scene.hasConversation ? orchestratorMenu(host: host, workspace: summary) : nil,
            nextNeedingYou: {
                let here = store.needsYou.filter { WorkspaceCounts.count(for: summary, host: host, in: [$0]) > 0 }
                if let next = NeedsYouNavigation.step(
                    lastOpened: lastAttention, items: here, fleet: store.fleet, showing: selection)
                {
                    open(next.item)
                }
            },
            openTask: { row in chooseTask(row.id, host: host, workspace: board, glance: false) },
            openLine: { line in openActivityLine(line, host: host, workspace: summary) },
            readSpend: {
                guard let client = store.clients[host] else { return .couldntRead }
                let read = await client.spendToday(repository: summary.repository ?? summary.id)
                return ActivitySpend.read(data: read.data, message: read.message)
            })
    }

    /// A row of the activity panel or the failed menu, opened: its pane, or
    /// its task on this workspace's board.
    private func openActivityLine(_ line: ActivityLine, host: String, workspace: WorkspaceSummary) {
        switch line.kind {
        case .agent(let terminal, let worktreeID):
            if let ws = worktree(host: host, id: worktreeID), let term = ws.terminals.first(where: { $0.id == terminal }) {
                go(to: BoardPane(terminal: term, worktree: ws))
            }
        case .subagent(let key), .queued(let key):
            guard let client = store.clients[host] else { return }
            let board = boardStore(for: workspace, client: client, host: host).board
            if let row = board.rows.first(where: { $0.key == key }) {
                chooseTask(row.id, host: host, workspace: workspace.id, glance: false)
            }
        }
    }

    /// The orchestrator's menu, for the status area: what its column's
    /// header offered before the header went (ov-214).
    private func orchestratorMenu(host: String, workspace: WorkspaceSummary) -> OrchestratorMenu {
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
        let canAct = store.refusal(for: host) == nil
        let column = ConversationColumn.state(
            seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
            startedAt: orchestratorStartedAt["\(host)|\(workspace.id)"], now: Date())
        return OrchestratorMenu(
            seat: seat, charter: CharterAccess.of(workspace, host: host), canAct: canAct,
            onReplace: { harness in
                orchestratorReplacement = OrchestratorReplacement(host: host, workspace: workspace, harness: harness)
            },
            onShowCharter: { url in
                if !NSWorkspace.shared.open(url) {
                    errorBanner = "Couldn’t open \(workspace.name)’s charter. It may have been moved or deleted."
                }
            },
            onTogglePaneMode: {
                if let seat { Task { await togglePaneMode(seat.terminal, in: seat.worktree) } }
            },
            onRestart: {
                if let seat { Task { await run(.restart, on: seat.terminal, in: seat.worktree) } }
            },
            onStepDown: {
                if let seat { Task { await stepDown(seat) } }
            },
            wakeOnAnswer: workspace.wakeOnAnswer,
            onSetWakeOnAnswer: { on in
                guard let client = store.clients[host] else { return }
                Task {
                    if let refused = await client.setWakeOnAnswer(workspace, on: on) { errorBanner = refused }
                }
            },
            starts: ConversationColumn.offers(column, canAct: canAct).compactMap { offer in
                if case .start(let harness) = offer { return harness }
                return nil
            },
            onStart: { harness in startOrchestrator(workspace, host: host, harness: harness, replace: false) })
    }

    /// What a switcher item does, by the routes the rest of the window uses.
    func perform(_ command: SwitcherCommand) {
        switch command {
        case .go(let target): selection = target
        case .needsYou: selection = .needsYou
        case .newWorkspace: newWorkspaceName = NewWorkspaceName(name: "")
        case .newWorktree: run(.newWorktree)
        case .addRepository: showAddRepository = true
        case .addRunner: showAdd = true
        case .find: showPalette = true
        case .runners:
            preferences.settingsTab = "machines"
            openSettings()
        case .reconnect(let host): store.reconnect(host)
        case .newCheckoutTerminal(let host, let id, let name):
            startMainTerminal(host: host, repositoryID: id, project: name)
        case .removeRepository(let host, let id, let name):
            guard let repo = repository(host: host, id: id, project: name) else { return }
            removeRepository = RepositoryToRemove(host: host, repository: repo)
        }
    }

    /// A worktree's menu (`WorktreeMenu.items`), on navigator rows, task rows
    /// and the breadcrumb.
    private func worktreeMenu(for ws: Worktree) -> [WorktreeMenu.Item] {
        let host = ws.host ?? ""
        let listed = worktree(host: host, id: ws.id) ?? ws
        let usable = store.refusal(for: host) == nil
        let offer = roleOffer(in: listed)
        return WorktreeMenu.items(
            for: listed, usable: usable, showsChanges: showChangesAction(for: listed, usable: usable) != nil,
            moveTargets: Self.moveTargets(for: listed, in: store.fleet, assigns: Self.assigns(store)(listed)),
            adoptable: listed.terminals.filter { offer($0) == .use })
    }

    func perform(_ item: WorktreeMenu.Item, on ws: Worktree) {
        let host = ws.host ?? ""
        let listed = worktree(host: host, id: ws.id) ?? ws
        switch item {
        case .open:
            trail = nil
            open(listed, terminal: nil)
        case .openInEditor: openInPreferredEditor(listed)
        case .showChanges: showChangesAction(for: listed, usable: store.refusal(for: host) == nil)?()
        case .newTerminal: newTerminal(in: listed)
        case .move(let id, _):
            if let target = store.fleet.runnerWorkspaces[host]?.first(where: { $0.id == id }) { move(listed, to: target) }
        case .useAsOrchestrator(let terminal, _):
            if let term = listed.terminals.first(where: { $0.id == terminal }) {
                Task { await run(.useAsOrchestrator, on: term, in: listed) }
            }
        case .hide: Task { await act(.hide, on: listed) { c in await c.hideWorktree(listed.short) } }
        case .unhide: Task { await act(.unhide, on: listed) { c in await c.unhideWorktree(listed.short) } }
        case .remove, .dismiss: removeWorktree = listed
        }
    }

    /// The runners `FleetStore.unhealthyHosts` names, with what's wrong
    /// with each, for the toolbar's runner item.
    private var runnerTroubles: [RunnerStatusItem.Trouble] {
        store.unhealthyHosts.compactMap { host in
            RunnerStatusItem.problem(store.state(of: host)).map { RunnerStatusItem.Trouble(host: host, problem: $0) }
        }
    }

    /// What a line of the runner item's menu does. The update opens its
    /// card from the item itself (`RunnerStatusMenu`).
    func perform(_ entry: RunnerStatusItem.Entry) {
        switch entry {
        case .reconnect(let host): store.reconnect(host)
        case .reconnectAll: for trouble in runnerTroubles { store.reconnect(trouble.host) }
        case .runners:
            preferences.settingsTab = "machines"
            openSettings()
        case .note, .update, .separator: break
        }
    }

    /// The workspace's worktrees in the board list's order, for the scene
    /// the selection draws: what ⌃⌘↑ and ⌃⌘↓ walk and the breadcrumb's
    /// menu lists.
    func worktreeEntries(_ scene: WorkspaceScene) -> [WorkspaceWorktrees.Entry] {
        guard let summary = scene.summary, let client = store.clients[scene.host] else { return [] }
        let board = boardStore(for: summary, client: client, host: scene.host).board
        return WorkspaceWorktrees.entries(in: summary, host: scene.host, board: board, fleet: store.fleet)
    }

    /// ⌃⌘↓ and ⌃⌘↑: the next or previous worktree in the workspace on
    /// screen, in the board list's order.
    private func stepWorktree(by offset: Int) {
        guard let scene = selection.flatMap(workspaceScene), let board = scene.board else { return }
        if let next = WorkspaceWorktrees.step(
            from: selection, by: offset, in: worktreeEntries(scene), host: scene.host, workspace: board,
            fleet: store.fleet)
        {
            trail = nil
            navigate(to: next)
        }
    }

    /// The board list's worktrees: each task's name, and the loose ones
    /// under Worktrees.
    private func boardWorktrees(
        host: String, workspace: WorkspaceSummary, client: DaemonClient, board: TaskBoardModel
    ) -> BoardWorktrees {
        let loose = WorkspaceWorktrees.loose(in: workspace, host: host, board: board, fleet: store.fleet)
        let usable = store.refusal(for: host) == nil
        let repository = client.repositories.first { $0.id == (workspace.repository ?? workspace.id) }
        let terminals = projectTerminals(host: host, workspace: workspace, usable: usable)
        return BoardWorktrees(
            byTask: WorkspaceWorktrees.taskWorktrees(on: board, host: host, in: store.fleet),
            shown: loose.shown, hidden: loose.hidden,
            // A project terminal open lights its own row, not the checkout's.
            selected: Self.openedWhole(selection).map(\.worktree)
                ?? (terminals.selected == nil ? WorkspaceScreen.namedTerminal(selection).map(\.worktree) : nil),
            onOpen: { worktree in glance(at: worktree) },
            onNew: usable ? repository.map { repo in { newWorktree(host: host, project: repo.displayName) } } : nil,
            onUnhide: usable ? { ws in Task { await act(.unhide, on: ws) { c in await c.unhideWorktree(ws.short) } } } : nil,
            menu: { worktreeMenu(for: $0) },
            perform: { item, ws in perform(item, on: ws) },
            terminals: terminals,
            unclaimed: BoardWorktrees.unclaimed(loose.shown, in: store.fleet))
    }

    /// The navigator's Terminals section for `workspace` (ov-178): its
    /// repository's own terminals, in the main checkout every one of its
    /// workspaces shares, each opened in this workspace, with the keyboard.
    private func projectTerminals(host: String, workspace: WorkspaceSummary, usable: Bool) -> ProjectTerminals {
        guard let checkout = ProjectTerminals.checkout(for: workspace, host: host, in: store.fleet) else { return .none }
        let terminals = ProjectTerminals.terminals(in: checkout, fleet: store.fleet)
        let named = WorkspaceScreen.namedTerminal(selection)
        return ProjectTerminals(
            checkout: checkout, terminals: terminals,
            selected: named.flatMap { open in
                open.host == host && open.worktree == checkout.id && terminals.contains { $0.id == open.terminal }
                    ? open.terminal : nil
            },
            onOpen: { terminal in
                trail = nil
                focusColumn = false
                keyboardOnBoard = false
                navigate(
                    to: .workspace(host: host, workspace: workspace.id, focus: .worktree(checkout.id, terminal: terminal.id)),
                    key: PaneRef(host: host, worktree: checkout.id, terminal: terminal.id))
            },
            onAction: { action, terminal in Task { await run(action, on: terminal, in: checkout) } },
            onNew: usable ? { Task { await openShell(besideOrchestratorIn: checkout, workspace: workspace.id) } } : nil)
    }

    /// ↑ or ↓ in the navigator onto `item`: it's selected, and the
    /// navigator keeps the keyboard, to go on (ov-92).
    func step(to item: NavigatorItem, host: String, workspace: WorkspaceSummary) {
        switch item {
        case .orchestrator:
            selectOrchestrator(keyboard: .board)
        case .task(let id):
            chooseTask(id, host: host, workspace: workspace.id, glance: true)
        case .unread(let line):
            chooseTask(BoardSummaryStrip.task(ofLine: line), host: host, workspace: workspace.id, glance: true)
        case .worktree(let id):
            if let found = worktree(host: host, id: id) { glance(at: found) }
        }
    }

    /// A loose worktree in the navigator chosen, by a click or ↑ or ↓: it
    /// opens in the main area, and the navigator keeps the keyboard, as a
    /// task's row does.
    private func glance(at worktree: Worktree) {
        trail = nil
        let step = WorkspaceNavigation.boardStep(.choose(glance: true), from: boardState)
        if step.keyboard == .board { boardKeyboardPending = true }
        focusColumn = step.focus
        open(worktree, terminal: nil)
        key(step.keyboard)
    }

    /// The breadcrumb's worktree segment for `place`: standing for a
    /// worktree opened whole, or after a task's crumb, that task's own
    /// (`WorkspaceWorktrees.segment`).
    private func worktreeCrumb(for place: Selection, scene: WorkspaceScene) -> WorktreeCrumb? {
        guard let board = scene.board else { return nil }
        let entries = worktreeEntries(scene)
        let current = WorkspaceSelection.samePlace(place, selection) ? selection : place
        let rows = scene.summary.flatMap { summary in
            store.clients[scene.host].map { boardStore(for: summary, client: $0, host: scene.host).board.rows }
        } ?? []
        guard let current,
            let segment = WorkspaceWorktrees.segment(
                for: current, trail: current == selection ? trail : nil, entries: entries,
                taskWorktrees: { id in
                    rows.first { $0.id == id }.map { WorkspaceWorktrees.worktrees(of: $0, host: scene.host, in: store.fleet) }
                        ?? []
                },
                name: { worktree(host: $0, id: $1)?.task }, host: scene.host, workspace: board, fleet: store.fleet)
        else { return nil }
        // The worktree it stands for, for its own menu's items.
        let named: Worktree? = {
            switch place {
            case .workspace(let host, _, .worktree(let id, _)?), .looseWorktree(let host, let id, _):
                return worktree(host: host, id: id)
            default:
                return nil
            }
        }()
        let help =
            segment.isWorkspace
            ? WorktreeCrumb.workspaceHelp
            : segment.opens != nil ? "Open \(segment.title)" : "Go to one of this task’s worktrees"
        return WorktreeCrumb(
            title: segment.title, isHere: segment.isHere, tasks: segment.tasks, loose: segment.loose,
            worktree: named?.task, actions: named.map { worktreeMenu(for: $0).filter { $0 != .open } } ?? [],
            perform: { item in if let named { perform(item, on: named) } }, opens: segment.opens, help: help,
            children: named.map { JumpMenus.terminals(of: $0, selection: selection, fleet: store.fleet) } ?? [])
    }

    /// A workspace: the navigator, and what's selected in it in the main
    /// area, the orchestrator or a task or worktree under its jump bar
    /// (ov-92). A loose worktree is drawn here too, beside its repository's
    /// navigator.
    private func workspaceDetail(_ scene: WorkspaceScene) -> some View {
        let host = scene.host
        let summary = scene.summary
        let layouts = shown
        let title = sceneTitle(scene, front: layouts.last)
        return WorkspaceView(
            opened: scene.opened,
            hasConversation: scene.hasConversation,
            // Put away with ⌘B, it's drawn as a scene without one.
            hasBoard: scene.board != nil && !navigatorHidden,
            cell: TerminalMetrics.cell(preferences.terminalFont()).width,
            focused: focusColumn,
            navigatorWidth: $navigatorWidth,
            conversation: {
                // Mounted and kept while something else is selected, drawing
                // the same layout; it just doesn't have the keyboard, or
                // count as seen or watched (`shown`).
                let drawn = KeptOrchestrator.conversation(visible: layouts, drawable: drawableLayouts(for: selection))
                conversationColumn(host: host, workspace: summary, shown: drawn.layout, onScreen: drawn.onScreen)
            },
            navigator: {
                if let board = scene.board {
                    boardColumn(
                        host: host, id: board,
                        orchestrator: scene.hasConversation ? navigatorOrchestrator(host: host, workspace: summary) : nil)
                }
            },
            breadcrumb: { place in
                let worktrees = worktreeCrumb(for: place, scene: scene)
                let crumbs = WorkspaceWorktrees.crumbs(
                    crumbs(for: place, host: host, workspace: summary), isHere: worktrees?.isHere == true)
                DrillBreadcrumb(
                    crumbs: crumbs,
                    worktrees: worktrees,
                    onGo: { target in
                        if target == trail { trail = nil }
                        selection = target
                    },
                    onClose: WorkspaceNavigation.closing(place, board: scene.board) == nil ? nil : { closeOpened() },
                    // A task's own worktree keeps the task as the way back, as its Open Worktree does.
                    onOpen: { item in land(NavigationHistory.Stop(item.target, trail: item.trail)) },
                    menus: jumpMenus(crumbs, place: place, host: host, summary: summary),
                    onJump: { jump($0) }, focusRequest: jumpBar.request, onLeave: { keyOpened() },
                    onActive: { jumpBar.active = $0 })
            },
            detail: { place, settled in openedView(place, layouts: layouts, settled: settled) }
        )
        .onPreferenceChange(WorkspaceWidthPreference.self) { width in
            MainActor.assumeIsolated {
                if let width, width != detailWidth { detailWidth = width }
            }
        }
        // No subtitle: "Billing · shop" is the switcher's, beside it (ov-86).
        .modifier(WindowTitle(title: title.title, subtitle: ""))
    }

    /// The window's title for `scene`: a workspace's, or a loose worktree's
    /// own, as its layout in front says.
    private func sceneTitle(_ scene: WorkspaceScene, front: ShownLayout?) -> (title: String, subtitle: String) {
        if case .looseWorktree(let host, let id, _)? = scene.opened {
            if let front { return Self.frame(of: front, in: store.fleet) }
            let ws = worktree(host: host, id: id)
            return (ws?.windowTitle ?? "Worktree", ws?.windowSubtitle ?? "")
        }
        return workspaceTitle(host: scene.host, workspace: scene.summary, focus: selection?.focus)
    }

    /// The window's title in a workspace (spec §4.9): the workspace, with
    /// "repository · runner" beneath it; a task's key and title with a task
    /// open, and "workspace · repository" beneath.
    private func workspaceTitle(host: String, workspace: WorkspaceSummary?, focus: Focus?)
        -> (title: String, subtitle: String)
    {
        let repository = workspace.flatMap { w in
            store.clients[host]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        let name = workspace.map { $0.isImplicit ? repository : $0.name } ?? "Workspace"
        var leaf: String?
        switch focus {
        case .task(let id)?:
            leaf = taskRow(host: host, workspace: workspace, id: id).map { "\($0.key) \($0.title)" }
        case .worktree(let id, _)?:
            leaf = worktree(host: host, id: id)?.windowTitle
        case .history(let status)?:
            leaf = BoardHistory.title(status)
        case nil:
            break
        }
        return Self.workspaceTitle(
            workspace: name, repository: repository, host: host, leaf: leaf,
            implicit: workspace?.isImplicit == true)
    }

    /// The window's title is the leaf of the breadcrumb: the task or the
    /// worktree opened, with the workspace and repository beneath it
    /// (ov-81 P10). The breadcrumb over the content owns the way back, so the
    /// title no longer repeats the path, and a worktree opened says so here
    /// as a task does instead of leaving the workspace's name up top. In the
    /// workspace itself, the workspace, with "repository · runner" beneath.
    nonisolated static func workspaceTitle(
        workspace: String, repository: String, host: String, leaf: String?, implicit: Bool
    ) -> (title: String, subtitle: String) {
        if let leaf {
            let under = implicit ? [repository] : [workspace, repository]
            return (leaf, under.filter { !$0.isEmpty }.joined(separator: " · "))
        }
        return (workspace, [repository, host].filter { !$0.isEmpty }.joined(separator: " · "))
    }

    /// A task on a workspace's board, as the board last read it.
    private func taskRow(host: String, workspace: WorkspaceSummary?, id: String) -> TaskRow? {
        guard let workspace, let client = store.clients[host] else { return nil }
        return boardStore(for: workspace, client: client, host: host).board.columns
            .flatMap(\.rows).first { $0.id == id }
    }

    /// The conversation column: the orchestrator, drawn as selecting its
    /// row drew it, under its header, or the state around one (spec §8).
    /// See `ConversationColumn`.
    @ViewBuilder
    private func conversationColumn(
        host: String, workspace: WorkspaceSummary?, shown: ShownLayout?, onScreen: Bool = true
    ) -> some View {
        if let workspace {
            let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
            let key = "\(host)|\(workspace.id)"
            let canAct = store.refusal(for: host) == nil
            // No header row (ov-214): the orchestrator's state, its harness
            // and its menu are the title bar's status area's.
            VStack(spacing: 0) {
                if let seat {
                    let sharers = WorkspaceScreen.sharers(
                        of: seat, layouts: store.client(for: seat.worktree)?.layouts[seat.worktree.id])
                    ForEach(sharers) { terminal in
                        SharedWindowNotice(title: terminal.label, canAct: canAct) {
                            Task { await moveOutOfOrchestratorWindow(terminal, in: seat.worktree) }
                        }
                        Divider()
                    }
                }
                // Ticks only while a start is being timed, in a window
                // somebody can see: the time matters to nothing else (ov-229).
                TimelineView(
                    WhileSchedule(every: 5, running: orchestratorStartedAt[key] != nil && windowVisible)
                ) { context in
                    let state = ConversationColumn.state(
                        seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
                        startedAt: orchestratorStartedAt[key], now: context.date)
                    conversationBody(
                        state: state, offers: ConversationColumn.offers(state, canAct: canAct),
                        shown: shown, seat: seat, workspace: workspace, host: host, onScreen: onScreen)
                }
            }
            // A start first seen here is timed from here; a live one clears it.
            .task(id: seat.map { "\($0.terminal.id)|\($0.terminal.state)" } ?? "") {
                let starting = seat.map { StateKind.parse($0.terminal.state) == .starting } ?? false
                if starting, orchestratorStartedAt[key] == nil { orchestratorStartedAt[key] = Date() }
                if let seat, StateKind.parse(seat.terminal.state) == .running { orchestratorStartedAt[key] = nil }
            }
        } else {
            ContentUnavailableView {
                Label("Workspace Not Found", systemImage: "square.stack.3d.up")
            } description: {
                Text(missingBoardSentence(host: host))
            }
        }
    }

    @ViewBuilder
    private func conversationBody(
        state: ConversationColumn.State, offers: [ConversationColumn.Offer], shown: ShownLayout?,
        seat: BoardPane?, workspace: WorkspaceSummary, host: String, onScreen: Bool
    ) -> some View {
        let placeholder = ConversationPlaceholder(
            state: state, offers: offers,
            onStart: { harness in startOrchestrator(workspace, host: host, harness: harness, replace: false) },
            onRestart: { if let seat { Task { await run(.restart, on: seat.terminal, in: seat.worktree) } } },
            onReplace: {
                let harness = seat.flatMap { OrchestratorHarness(rawValue: Terminal.name(of: $0.terminal.preset)) } ?? .claude
                orchestratorReplacement = OrchestratorReplacement(host: host, workspace: workspace, harness: harness)
            },
            candidates: OrchestratorAdoption.candidates(for: workspace, host: host, in: store.fleet),
            onUse: { useAsOrchestrator($0) })
        switch state {
        case .live:
            if let shown {
                tiled(shown, titled: false, keyboard: onScreen)
            } else if let seat {
                bareTerminal(seat, keyboard: onScreen)
            }
        case .lost:
            // Its last screen, dimmed, under what can be done about it.
            ZStack {
                if let shown { tiled(shown, titled: false).opacity(0.35).allowsHitTesting(false) }
                placeholder.background(.regularMaterial.opacity(shown == nil ? 0 : 1))
            }
        case .none, .starting:
            placeholder
        }
    }

    /// The orchestrator's row in the navigator (ov-92): its state, what
    /// it's doing now, and, with none, the ways to start one.
    private func navigatorOrchestrator(host: String, workspace: WorkspaceSummary?) -> NavigatorOrchestrator? {
        guard let workspace else { return nil }
        let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
        let canAct = store.refusal(for: host) == nil
        let column = ConversationColumn.state(
            seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
            startedAt: orchestratorStartedAt["\(host)|\(workspace.id)"], now: Date())
        // Asked for here and not yet seated: starting, as its column says.
        var state = OrchestratorRow.state(seat: seat)
        if case .starting = column { state = .starting }
        return NavigatorOrchestrator(
            state: state,
            agent: OrchestratorMenu.agentName(seat),
            status: seat?.terminal.status,
            nowDoing: OrchestratorRow.nowDoing(seat?.terminal, state: state),
            offers: ConversationColumn.offers(column, canAct: canAct),
            candidates: OrchestratorAdoption.candidates(for: workspace, host: host, in: store.fleet),
            onSelect: { selectOrchestrator(keyboard: .board) },
            onStart: { harness in startOrchestrator(workspace, host: host, harness: harness, replace: false) },
            onUse: { useAsOrchestrator($0) })
    }

    /// The breadcrumb over what's opened: Workspace › Task, Workspace › Task
    /// › Worktree, or Workspace › Worktree; for a loose worktree, its board's
    /// workspace › the worktree. `place` is what's drawn, which may be the
    /// one leaving rather than the window's.
    private func crumbs(for place: Selection, host: String, workspace: WorkspaceSummary?) -> [WorkspaceNavigation.Crumb] {
        let repository = workspace.flatMap { w in
            store.clients[host]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        let name = workspace.map { $0.isImplicit ? repository : $0.name } ?? "Workspace"
        if case .looseWorktree(_, let id, _) = place {
            let here = WorkspaceNavigation.Crumb(title: worktree(host: host, id: id)?.task ?? "Worktree", target: nil)
            guard let workspace else { return [here] }
            return [.init(title: name, target: .workspace(host: host, workspace: workspace.id, focus: nil)), here]
        }
        // The window's own, with its trail, while it's the one drawn.
        let current = WorkspaceSelection.samePlace(place, selection) ? selection : place
        return WorkspaceNavigation.crumbs(
            current, trail: current == selection ? trail : nil,
            workspace: name,
            task: { id in
                taskRow(host: host, workspace: workspace, id: id).map { "\($0.key) \($0.title)" } ?? "Task"
            },
            worktree: { id in worktree(host: host, id: id)?.task ?? "Worktree" })
    }

    /// What's opened beside the board: a task, or a worktree opened whole.
    ///
    /// `place` is the window's own while it's open, drawn with the layouts on
    /// screen and the keyboard; or the one leaving, sliding out or fading
    /// under the next, drawn with its layout and no keyboard, so its
    /// terminal view stays what it was until it's gone.
    @ViewBuilder
    private func openedView(_ place: Selection, layouts: [ShownLayout], settled: Bool) -> some View {
        let current = WorkspaceSelection.samePlace(place, selection)
        let shown =
            current
            ? layouts.last { $0.column != .conversation }
            : drawableLayouts(for: place).last { $0.column != .conversation }
        switch place {
        case .workspace(let host, _, .task(let id)?):
            // Its agent's layout whichever tab is in front, so the
            // terminal is one view across them; whether it's on screen is
            // the tab's to say (`WorkspaceScreen.visible`).
            taskView(
                host: host, id: id, place: place,
                shown: drawableLayouts(for: place).last { $0.column != .conversation }, keyboard: current,
                settled: settled)
        case .workspace(let host, let id, .history(let status)?):
            if let client = store.clients[host], let workspace = board(host: host, id: id) {
                BoardHistoryView(
                    store: boardStore(for: workspace, client: client, host: host), status: status,
                    onOpen: { row in chooseTask(row.id, host: host, workspace: id, glance: false) })
            } else {
                ContentUnavailableView("Board Not Found", systemImage: "checklist")
            }
        case .workspace(let host, _, .worktree(let wt, _)?), .looseWorktree(let host, let wt, _):
            if !settled {
                // Passed on the way: its terminals wait until it settles.
                Color.clear
            } else if let paneless = WorkspaceScreen.paneless(current ? selection : place, in: store.fleet, shown: shown) {
                // A lost terminal clicked on its card: its own page, with
                // Restart and Dismiss, not the card again (ov-191). Asked of
                // the selection, not `place`, which leaves the pane out
                // (`WorkspaceSelection.place`): asked of that, this never
                // fired, and the click still did nothing.
                bareTerminal(paneless, keyboard: current)
            } else if let shown {
                tiled(shown, titled: false, keyboard: current)
            } else if let ws = worktree(host: host, id: wt) {
                worktreeDetail(ws)
            } else {
                ContentUnavailableView("Worktree Not Found", systemImage: "folder")
            }
        default:
            EmptyView()
        }
    }

    /// Back: up one level, Focus first. ⌃⌘← goes along the breadcrumb to a
    /// task a worktree was opened from; Esc (`toOrchestrator`) goes up to
    /// the orchestrator (ov-92).
    func goBack(toOrchestrator: Bool = false) {
        let step = WorkspaceNavigation.backStep(
            focus: focusColumn, oneAtATime: true, toOrchestrator: toOrchestrator, from: selection, trail: trail)
        if step.leavesFocus { focusColumn = false }
        if let back = step.goesTo {
            guard back.focus != nil else {
                closeOpened()
                return
            }
            if back == trail { trail = nil }
            // The keyboard follows to the level it lands on, by the
            // selection's own rule (`WorkspaceScreen.keyPane`), or, with no
            // terminal there, to the view itself.
            selection = back
            if WorkspaceScreen.keyPane(nil, in: shownLayouts(for: back), selection: back) == nil {
                windowBox.window?.makeFirstResponder(nil)
            }
        } else if step.leavesFocus {
            keyOpened()
        } else if case .looseWorktree? = selection {
            closeOpened()
        }
    }

    /// Close what's opened: the jump bar's close button, a click on the
    /// selected task, or Back from a task. The orchestrator is selected
    /// again, and the navigator keeps the keyboard, so ↑ and ↓ go on from
    /// it.
    private func closeOpened() {
        let board = selection.flatMap(workspaceScene)?.board
        guard let current = selection, let closed = WorkspaceNavigation.closing(current, board: board) else { return }
        trail = nil
        let step = WorkspaceNavigation.boardStep(.close, from: boardState)
        if step.keyboard == .board { boardKeyboardPending = true }
        focusColumn = step.focus
        selection = closed
        key(step.keyboard)
    }

    /// The orchestrator selected (⌥⌘1, its row, ↑ or ↓ onto it): whatever
    /// was open goes, and `keyboard` says where the keyboard goes, into the
    /// orchestrator or staying on the navigator.
    func selectOrchestrator(keyboard: WorkspaceNavigation.KeyTarget) {
        guard let current = selection, let scene = workspaceScene(current) else { return }
        trail = nil
        focusColumn = false
        if let board = scene.board, current.focus != nil || scene.opened != nil {
            if keyboard == .board { boardKeyboardPending = true }
            selection = .workspace(host: scene.host, workspace: board, focus: nil)
        }
        key(keyboard)
    }

    /// The keyboard to what the main area shows: what's opened, else the
    /// orchestrator.
    func keyMain() {
        let layouts = shownLayouts(for: selection)
        if let pane = (layouts.last(where: { $0.column != .conversation }) ?? layouts.first)
            .flatMap(WorkspaceScreen.columnPane)
        {
            step(to: pane)
        } else {
            keyboardOnBoard = false
            keyPane = nil
            windowBox.window?.makeFirstResponder(nil)
        }
    }

    /// The keyboard to the task's or worktree's terminal, if it has one on
    /// screen, else to the view itself, so Esc and the arrows reach it.
    func keyOpened() {
        if let pane = shownLayouts(for: selection).last(where: { $0.column != .conversation })
            .flatMap(WorkspaceScreen.columnPane)
        {
            step(to: pane)
        } else if case .workspace(_, _, nil)? = selection {
            // Nothing opened: the orchestrator takes it.
            keyMain()
        } else {
            keyPane = nil
            windowBox.window?.makeFirstResponder(nil)
        }
    }

    /// A task, beside the navigator (spec §4.4, ov-98): its header, and
    /// under it its three tabs, Overview, Agent and Changes.
    @ViewBuilder
    private func taskView(
        host: String, id: String, place: Selection, shown: ShownLayout?, keyboard: Bool, settled: Bool
    ) -> some View {
        let summary = place.workspace.flatMap {
            WorkspaceScreen.workspace($0, host: host, in: store.fleet, repositories: store.clients[host]?.repositories.map(\.id) ?? [])
        }
        if let client = store.clients[host], let summary {
            let board = boardStore(for: summary, client: client, host: host)
            if let row = board.board.columns.flatMap(\.rows).first(where: { $0.id == id }) {
                taskView(
                    row: row, board: board, client: client, host: host, shown: shown, keyboard: keyboard,
                    settled: settled)
            } else if !board.hasRead {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .task(id: ObjectIdentifier(board)) { await board.readIfNeverRead() }
            } else {
                ContentUnavailableView {
                    Label("Task Not on Board", systemImage: "checklist")
                } actions: {
                    Button("Close") { closeOpened() }
                }
            }
        } else {
            ContentUnavailableView("Task Not Found", systemImage: "checklist")
        }
    }

    /// Ask the Orchestrator for `workspace`'s tasks: on while it has an
    /// orchestrator running, which it reads from the fleet as it is now. It
    /// leaves the draft in that composer, or pastes it into a terminal
    /// orchestrator (`AskOrchestrator.deliver`), and goes there; it starts
    /// nothing.
    private func askOrchestrator(host: String, workspace: WorkspaceSummary) -> AskOrchestrator.Action {
        // As the runner lists it now: the board's own copy is kept across an
        // orchestrator starting or stopping.
        let live = WorkspaceScreen.workspace(
            workspace.id, host: host, in: store.fleet,
            repositories: store.clients[host]?.repositories.map(\.id) ?? []) ?? workspace
        let seat = WorkspaceScreen.orchestrator(of: live, host: host, in: store.fleet)
        return AskOrchestrator.Action(
            available: seat != nil,
            perform: { row in
                guard let seat else { return }
                selection = .workspace(host: host, workspace: workspace.id, focus: nil)
                let client = store.client(for: seat.worktree)
                Task { @MainActor in
                    let delivery = await AskOrchestrator.deliver(
                        row, to: seat,
                        paste: { text in await client?.draftPrompt(terminal: seat.terminal.short, text: text) ?? false },
                        copy: client?.copyToClipboard ?? AskOrchestrator.copyToPasteboard)
                    // A terminal pane has no composer to fill: the person
                    // pastes, so the pane takes the keyboard.
                    if delivery != .composer {
                        focus(PaneRef(host: host, worktree: seat.worktree.id, terminal: seat.terminal.id))
                    }
                    if delivery == .copied { errorBanner = AskOrchestrator.copiedNotice(for: row) }
                }
            })
    }

    private func taskView(
        row: TaskRow, board: TaskBoardStore, client: DaemonClient, host: String, shown: ShownLayout?,
        keyboard: Bool = true, settled: Bool = true
    ) -> some View {
        let onRunner = boardAgents(host: host, client: client)
        let agents = WorkspaceScreen.agents(of: row.id, host: host, in: store.fleet)
        let chosen = WorkspaceScreen.agent(of: row.id, host: host, in: store.fleet, chosen: chosenAgents[row.id])
        let worktreeID = TaskColumnModel.worktree(of: row, agent: chosen)
        let lane = worktreeID.flatMap { worktree(host: host, id: $0) }
        let agent = TaskColumnModel.agent(
            hasAgent: chosen != nil, worktree: lane?.id,
            stopped: TaskColumnModel.hadAgent(row.id, host: host, in: store.fleet))
        let showsChanges = lane != nil && client.changesSupported != false
        let tab = taskTabs.tab(for: row.id, agentWorking: chosen != nil)
        let openWorktree = {
            guard let lane, let current = selection else { return }
            let opened = WorkspaceNavigation.openWorktree(lane.id, from: current)
            trail = opened.trail
            trailWorktree = lane.id
            selection = opened.next
        }
        let ask = askOrchestrator(host: host, workspace: board.workspace)
        let start = TaskStartPanel(
            sentence: TaskColumnModel.sentence(agent) ?? "", row: row, ask: ask)
        return VStack(spacing: 0) {
            TaskViewHeader(row: row, ask: ask, agent: chosen)
            TaskTabBar(
                tab: tab, onChoose: { choose($0, for: row.id) },
                worktree: lane.map { WorkspaceScreen.ownTerminals(of: $0, fleet: store.fleet) },
                agents: agents, chosen: chosen,
                onChooseAgent: { pane in chosenAgents[row.id] = pane.terminal.id },
                onOpenWorktree: openWorktree)
            TaskTabs(tab: tab) {
                ScrollView {
                    TaskColumnCard(
                        row: row, store: board, orchestrator: onRunner.orchestrator(for: row),
                        speaksOfAgents: onRunner.runnerRecordsTasks, onGoTo: { go(to: $0) }
                    )
                    .padding(TaskTypography.inset)
                }
                // The record runs past the window on a long task: bars that
                // stay, and a soft edge that says there is more below.
                .scrollIndicators(.visible)
                .scrollEdgeEffectStyle(.soft, for: .bottom)
                .background(WorkspaceStyle.document)
            } agent: {
                // A task passed on the way, glancing: no terminal mounted
                // until it settles. Mounted once, then kept behind the other
                // tabs (`TaskTabs`); on screen and given the keyboard only
                // in front.
                switch TaskColumnModel.agentView(hasAgent: chosen != nil, settled: settled, hasLayout: shown != nil) {
                case .start: start
                case .waiting: Color.clear
                case .tiled: if let shown { tiled(shown, titled: false, keyboard: keyboard && tab == .agent) }
                // Working the task, and in no layout read yet.
                case .bare: if let chosen { bareTerminal(chosen, keyboard: keyboard && tab == .agent) }
                }
            } changes: {
                if let lane, showsChanges {
                    if settled {
                        TaskColumnChanges(
                            changes: changesStore(for: lane, client: client),
                            isFocused: TaskColumnModel.changesFocused(focus: changesFocus, task: row.id, tab: tab),
                            agents: lane.reviewAgentTargets(), onFocus: { changesFocus = row.id })
                    }
                } else if lane != nil {
                    TaskStartPanel(sentence: "Update Far Cooler on this runner to see changes here.")
                } else {
                    start
                }
            }
        }
        // The card's record and question, read for the task on screen once
        // it has settled: one read for a held arrow's whole walk.
        .task(id: settled ? row.id : nil) {
            if settled { await board.open(row) }
        }
    }

    /// Move to Its Own Window: `terminal`, sharing the orchestrator's
    /// window, gets a window of its own (`layout break`, tmux's
    /// `break-pane -d`), so the orchestrator keeps its window and focus.
    /// Only on this click: nothing rearranges a runner's windows unasked.
    @discardableResult
    func moveOutOfOrchestratorWindow(_ terminal: Terminal, in worktree: Worktree) async -> Bool {
        guard let client = store.client(for: worktree) else { return false }
        if !(await client.moveToOwnWindow(terminal, in: worktree)) {
            errorBanner = "Couldn’t move \(terminal.label) to its own window. Check that the runner is reachable, then try again."
            return false
        }
        return true
    }

    /// A worktree with no layout yet: its card of terminals.
    ///
    /// Without the orchestrators seated in it, which are their workspaces'
    /// conversation columns', and with where to find them instead.
    private func worktreeDetail(_ ws: Worktree) -> some View {
        let host = ws.host ?? ""
        return WorktreeDetail(
            worktree: WorkspaceScreen.ownTerminals(of: ws, fleet: store.fleet),
            hosted: WorkspaceScreen.seated(in: ws, fleet: store.fleet).map { seat in
                WorktreeDetail.Hosted(name: seat.workspace.name) {
                    selection = .workspace(host: host, workspace: seat.workspace.id, focus: nil)
                }
            },
            onNewTerminal: { newTerminal(in: ws) },
            onHide: { Task { await act(.hide, on: ws) { c in await c.hideWorktree(ws.short) } } },
            onUnhide: { Task { await act(.unhide, on: ws) { c in await c.unhideWorktree(ws.short) } } },
            onRemove: { removeWorktree = ws },
            onOpenTerminal: { t in open(ws, terminal: t.id) },
            onTerminalAction: { action, t in Task { await run(action, on: t, in: ws) } },
            onShowChanges: showChangesAction(for: ws, usable: store.refusal(for: host) == nil)
        )
    }

    /// Open `worktree`, or `terminal` in it, as its navigator row and its card
    /// do. See `navigate(to:key:)`.
    func open(_ worktree: Worktree, terminal: String?) {
        navigate(to: Self.opening(worktree, terminal: terminal, in: store.fleet))
    }

    /// Go to `next`, an explicit open: a navigator row, a card, Needs You, the
    /// palette, or a pane gone to from the keyboard. A terminal it names that
    /// shares a seated orchestrator's window is moved to a window of its own
    /// first (`moveOutOfOrchestratorWindow`: the sharer, never the
    /// orchestrator), because the Orchestrator column draws that window and
    /// the checkout can't draw a pane apart from it (ov-78). Opening it is
    /// the ask. If the move fails the selection stays where it was, with the
    /// banner saying why, rather than landing on a pane that isn't there.
    func navigate(to next: Selection, key: PaneRef? = nil) {
        func land() {
            selection = next
            if let key { keyPane = key }
        }
        guard let named = WorkspaceScreen.namedTerminal(next),
            let listed = worktree(host: named.host, id: named.worktree),
            let sharer = listed.terminals.first(where: { $0.id == named.terminal }),
            WorkspaceScreen.seat(
                sharedBy: sharer.id, in: listed, fleet: store.fleet,
                layouts: store.client(for: listed)?.layouts[listed.id]) != nil
        else {
            land()
            return
        }
        Task {
            if await moveOutOfOrchestratorWindow(sharer, in: listed) { land() }
        }
    }

    /// A pane drawn on its own, for the moment before its layout is read, or
    /// a pane in no layout at all: "we haven't read the layouts yet" and
    /// "it's in none" look the same from here, and showing the terminal is
    /// the right answer to both.
    private func bareTerminal(_ pane: BoardPane, keyboard: Bool = true) -> some View {
        let ws = pane.worktree
        let term = pane.terminal
        let client = store.client(for: ws)
        return TerminalPane(
            terminal: term,
            worktree: ws,
            binary: client?.cliPath,
            environment: client?.cliEnvironment ?? [:],
            hostArguments: client?.cliHostArguments ?? [],
            linkGeneration: client?.linkGeneration ?? 0,
            refusal: { store.refusal(for: ws) },
            onGeometry: { cols, rows in
                // Not through `act`: this is geometry, not a click.
                await store.client(for: ws)?.resize(terminal: term.short, columns: cols, rows: rows)
            },
            onSearchFiles: { query in
                await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? []
            },
            onAction: { action in Task { await run(action, on: term, in: ws) } },
            hasKeyboard: keyboard && WorkspaceScreen.bareTakesKeyboard(shown)
        )
    }

    /// A workspace's navigator, or a sentence saying where its board went.
    @ViewBuilder
    private func boardColumn(host: String, id: String, orchestrator: NavigatorOrchestrator?) -> some View {
        if let client = store.clients[host], client.daemonBuild.map({ !$0.can("tasks") }) == true {
            // A runner too old for boards: said, rather than a board that
            // can't be read.
            ContentUnavailableView {
                Label("No Board on This Runner", systemImage: "checklist")
            } description: {
                Text("This runner’s Far Cooler is too old for boards. Update it there to see this workspace’s tasks.")
            }
        } else if let client = store.clients[host], let workspace = board(host: host, id: id),
            let repository = client.repositories.first(where: {
                $0.id == (workspace.repository ?? workspace.id)
            })
        {
            TaskBoardView(
                store: boardStore(for: workspace, client: client, host: host),
                client: client,
                agents: boardAgents(host: host, client: client),
                onGoTo: { pane in go(to: pane) },
                selected: WorkspaceNavigation.selectedTask(selection, trail: trail, board: id),
                focusRequest: boardFocusRequest,
                onKeyboard: { keyboardOnBoard = true },
                onEnter: { focusWorkspaceColumn(.focusTask) },
                hasKeyboard: keyboardOnBoard,
                worktrees: { board in boardWorktrees(host: host, workspace: workspace, client: client, board: board) },
                orchestrator: orchestrator,
                current: Navigator.current(selection, trail: trail, board: id),
                onStep: { item in step(to: item, host: host, workspace: workspace) },
                onHistory: { status in openHistory(status, host: host, workspace: workspace.id) },
                filterRequest: boardFilterRequest,
                ask: askOrchestrator(host: host, workspace: workspace)
            )
        } else {
            // Said, rather than the generic "Select a worktree": this
            // was a board, and the reader should know where it went.
            ContentUnavailableView {
                Label("Board Not Found", systemImage: "checklist")
            } description: {
                Text(missingBoardSentence(host: host))
            }
        }
    }

    /// One shown layout, wired up.
    ///
    /// One builder for every column. Selecting a pane and selecting the
    /// worktree it is in put the same view on screen with the same six
    /// callbacks, and while they were written out twice they drifted: the drag
    /// handler was fixed in one copy and not the other, so dropping a pane
    /// behaved differently depending on which row you had clicked last.
    @ViewBuilder
    private func tiled(_ shown: ShownLayout, titled: Bool = true, keyboard: Bool = true) -> some View {
        let ws = shown.worktree
        if let client = store.client(for: ws) {
            tiled(
                shown, client: client, frame: Self.frame(of: shown, in: store.fleet), titled: titled,
                keyboard: keyboard)
        } else {
            placeholder
        }
    }

    private func tiled(
        _ shown: ShownLayout, client: DaemonClient, frame: (title: String, subtitle: String), titled: Bool,
        keyboard: Bool = true
    ) -> some View {
        let ws = shown.worktree
        return TileView(
            groups: shown.groups,
            showing: shown.group.id,
            worktree: ws,
            changes: changesStore(for: ws, client: client),
            binary: client.cliPath,
            environment: client.cliEnvironment,
            hostArguments: client.cliHostArguments,
            linkGeneration: client.linkGeneration,
            refusal: { store.refusal(for: ws) },
            onFocus: { id in
                focus(PaneRef(host: ws.host ?? "", worktree: ws.id, terminal: id))
                guard let pane = store.client(for: ws)?.group(holding: id, in: ws.id)?.pane(id)
                else { return }
                Task { await act(.arrange, on: ws) { c in await c.focusPane(pane.short, in: ws) } }
            },
            onSelectGroup: { chosen in
                Task {
                    // Land in the layout you just chose, not on whatever pane the
                    // previous one had focused.
                    let groups = await act(.arrange, on: ws, default: []) { c in
                        await c.selectLayout(chosen.id, in: ws)
                    }
                    reveal(groups, in: ws)
                }
            },
            onDropOnPane: { dragged, target, side in
                placePane(dragged, onto: target, side: side, in: ws)
            },
            onViewport: { layout, columns, rows in
                // Not routed through `act` — see `onGeometry`'s
                // comment above: this fires from pane geometry, not a click.
                //
                // The layout drawn, by name: tmux's active window can be an
                // orchestrator's while the checkout's own is on screen.
                await store.client(for: ws)?.viewport(
                    columns: columns, rows: rows, in: ws, layout: layout)
            },
            onResizeDivider: { terminal, side, cells in
                resizeDivider(terminal, side: side, cells: cells, in: ws)
            },
            onSearchFiles: { query in await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? [] },
            onSwitchPaneMode: { terminal in Task { await togglePaneMode(terminal, in: ws) } },
            onTerminalAction: { action, terminal in Task { await run(action, on: terminal, in: ws) } },
            title: frame.title,
            subtitle: frame.subtitle,
            setsTitle: titled,
            hasKeyboard: KeptOrchestrator.takesKeyboard(
                shown, onScreen: keyboard, key: selectedPane, onBoard: keyboardOnBoard)
        )
    }

    /// The detail with no workspace to show: the fleet's own state first,
    /// before it has anything in it (`FleetPlaceholder`), then "choose one".
    private var placeholder: some View {
        let local = store.clients[""]
        return FleetPlaceholder(
            phase: FleetPlaceholder.phase(
                hasWorktrees: !store.fleet.worktrees.isEmpty, localLoaded: local?.hasLoaded == true,
                localError: local?.fleetError, hasRepositories: !store.repositories.isEmpty),
            onOpenMain: FleetPlaceholder.mainToOpen(in: store.repositories, fleet: store.fleet).map { target in
                { selection = .workspace(host: target.host, workspace: target.workspace.id, focus: nil) }
            },
            onNewWorkspace: workspaceRepositories.isEmpty ? nil : { newWorkspaceName = NewWorkspaceName(name: "") },
            onAddRepository: { showAddRepository = true },
            onNewWorktree: { newWorktreeIntent = NewWorktreeIntent() },
            onTryAgain: { Task { await local?.refresh() } })
    }

    // MARK: - Routing

    /// A repository to default the project picker to, when nothing was
    /// chosen yet — the empty state's "New Worktree…" button, ⌘N, and the
    /// palette's "New Worktree…" all reach this with no project
    /// and therefore no host in hand at all, which is the one case where a
    /// default runner is legitimate rather than the picker again in
    /// disguise. This Mac's own repositories come first: it is the runner
    /// guaranteed to be present, the one everything else is optional next to.
    /// Falls back to any repository so the picker still has something to
    /// preselect the very first time, before this Mac has one of its own.
    var defaultProjectID: String? {
        store.repositories.first { $0.host.isEmpty }?.repository.id
            ?? store.repositories.first?.repository.id
    }

    /// A terminal's rendered screen, for the palette's preview tiles.
    ///
    /// `short` is what `ScreenPreviews` keys everything by, and short ids can
    /// collide across runners — the reason `Selection` carries a host at all.
    /// This is the one place left that has to work backwards from a bare short
    /// id with no host of its own to check against, because that is the whole
    /// interface `ScreenPreviews` and `CommandPalette` were built around. Local
    /// runner first, then the rest in the fleet's order:
    /// with one runner, or with short ids that do not collide, this finds the
    /// right terminal every time; a genuine collision costs a preview tile
    /// showing the wrong screen, never an action landing on the wrong runner.
    private func screen(forTerminalShort short: String) async -> String {
        for host in store.hosts {
            guard let client = store.clients[host] else { continue }
            if client.fleet.worktrees.contains(where: { $0.terminals.contains { $0.short == short } }) {
                return await client.screen(terminal: short)
            }
        }
        return ""
    }

    // MARK: - Behavior

    func worktree(host: String, id: String) -> Worktree? {
        store.fleet.worktrees.first { ($0.host ?? "") == host && $0.id == id }
    }

    /// Where the window opens (spec §4.6, ov-182): where it was when the app
    /// last closed, whatever is waiting on Needs You; with nowhere kept, Needs
    /// You while anything is waiting, else the first workspace. See
    /// `SelectionMemory.launch`.
    ///
    /// A place kept is held for its runner, which comes up on its own
    /// schedule, and opened when it has (`restoreWhereYouWere`), or at the
    /// nearest level of it that's still there when it's gone. Asked on every
    /// fleet and Needs You change until it has an answer, and never again
    /// after: a window that has opened somewhere, or where somebody already
    /// clicked, isn't moved by a count that rises later.
    private func settleLaunch() {
        SelectionMemory.migrate(
            .standard, fleet: store.fleet, ready: { host in store.clients[host]?.hasLoaded ?? true })
        guard !launched else { return }
        guard selection == nil else {
            launched = true
            return
        }
        if let kept = SelectionMemory.kept(destination: lastDestination, legacy: lastSelection) {
            launched = true
            restoring = DestinationOpen(destination: kept, arrival: .restore, since: Date())
            return
        }
        guard
            let decided = SelectionMemory.launch(
                needsYou: store.needsYou.count, settled: store.needsYouSettled, in: store.fleet)
        else { return }
        launched = true
        selection = decided
    }

    // MARK: - Commands

    /// The terminals of the view on screen, in the order they're drawn:
    /// what ⌘] and ⌘[, ⌥⌘↓ and ⌥⌘↑ and ⌃⌘1… step through (spec §4.9).
    /// Nothing lists every terminal, so stepping through all of them would
    /// walk a list nobody can see.
    var allTerminals: [PaneRef] { Self.stepOrder(shown) }

    /// `allTerminals` for what's shown: column by column, each layout's
    /// panes in tmux's order.
    static func stepOrder(_ shown: [ShownLayout]) -> [PaneRef] {
        shown.flatMap { layout in
            layout.group.terminals.map { PaneRef(host: layout.host, worktree: layout.worktree.id, terminal: $0) }
        }
    }

    /// The pane the keyboard acts on, with its worktree and terminal. See
    /// `WorkspaceScreen.keyPane`.
    var selectedPane: PaneRef? { WorkspaceScreen.keyPane(keyPane, in: shown, selection: selection) }

    var selectedTerminal: (worktree: Worktree, terminal: Terminal)? {
        guard let pane = selectedPane, let worktree = worktree(host: pane.host, id: pane.worktree),
            let terminal = worktree.terminals.first(where: { $0.id == pane.terminal })
        else { return nil }
        return (worktree, terminal)
    }

    /// The layout the detail draws for `ws`, and the layouts its bar offers
    /// beside it. Nil when it draws none.
    ///
    /// What the keyboard's layout commands act on, so ⌃B and a digit counts
    /// the panes on screen and ⌃B n steps through the layouts in the bar —
    /// not through the window tmux calls active, which in the main checkout
    /// can be an orchestrator's. With two columns showing one worktree's
    /// layouts, the one holding the key pane.
    func onScreen(in ws: Worktree) -> (group: PaneGroup, groups: [PaneGroup])? {
        let mine = shown.filter { $0.host == (ws.host ?? "") && $0.worktree.id == ws.id }
        let pick = selectedPane.flatMap { key in mine.first { $0.contains(key) } } ?? mine.last
        return pick.map { ($0.group, $0.groups) }
    }

    // MARK: - Attention

    /// Which terminals the detail pane is actually putting in front of you.
    ///
    /// Not just the selected one. Selecting a pane shows the whole layout it
    /// belongs to, and a terminal tiled beside the one with focus is as much on
    /// screen as the one with focus — reading it took no extra click, so it
    /// cannot go on asking for one.
    ///
    /// Read from `shown`, which is every layout `detail` draws, column by
    /// column. Anything else would have this marking
    /// terminals read that are not on screen, which is the one mistake worse
    /// than the bug it fixes. It used to ask for the layout tmux calls active,
    /// from when `detail` drew that one; since the orchestrators run in the
    /// main checkout's session, that can be Billing's orchestrator while the
    /// checkout's own shells are on screen.
    private var visibleTerminals: [Terminal] {
        shown.flatMap { layout in layout.worktree.terminals.filter { layout.group.terminals.contains($0.id) } }
    }

    /// End `done` for everything on screen, if anyone is there to see it.
    ///
    /// `done` is finished-and-UNSEEN, so it has to end when you see it — and you
    /// see a pane by having it in front of you, not only by clicking on it.
    /// Marking on selection alone missed the commonest case there is: you sit
    /// watching an agent work, it finishes, and because you never had to click
    /// anything the row goes on flagging itself indefinitely. Nothing short of
    /// clicking away and back could clear it.
    ///
    /// Gated on the app being active, which is the whole distinction the feature
    /// rests on. An agent finishing while you are in another app is precisely
    /// what the notification exists for, and a window sitting behind three
    /// others must not quietly mark it read. And, for the marking itself, on
    /// somebody being at the Mac at all (`Presence`): a frontmost window on a
    /// locked or sleeping Mac, or one left for the kitchen, gets fleet events
    /// too, and nobody sees what they bring.
    ///
    /// Only `done`. `blocked` is the agent waiting on an ANSWER, and looking at
    /// a question does not answer it — the daemon agrees, so sending anything
    /// else would only be a subprocess spent to be told no.
    ///
    /// Not routed through `act`: this is a best-effort background
    /// bookkeeping call, not a user-initiated action, and a runner gone quiet
    /// for a moment must not put a banner on screen just because an agent on
    /// it happened to finish.
    ///
    /// It also tells each runner what this window is SHOWING, which is the same
    /// judgement one beat earlier — see `DaemonClient.reportWatching`. Marking
    /// seen ends a `done` that already happened; the claim of attention stops
    /// the notification about it being raised in the first place, and the
    /// difference between the two is the buzz on a wrist about a reply already
    /// on screen. Reported from here rather than from hooks of its own because
    /// there is one question underneath both — "is a person looking at this
    /// pane right now" — and every path that answers it already funnels
    /// through this: a fleet event, coming back to the app, and a selection
    /// change. Anywhere those are wrong about what is on screen, `seen` has
    /// been wrong in the same way for as long as it has existed, and one
    /// answer is the point.
    private func markVisibleSeen() {
        guard NSApp.isActive else {
            Notifier.shared.setWatching([], window: windowID)
            return
        }
        // Minimized, or wholly behind other windows: nothing here is on
        // screen, whatever is selected, so it silences no banner and its
        // runners are told so. Asked again when that changes (`body`).
        guard WindowSight.inSight(windowBox.window) else {
            WindowSight.leave(window: windowID, clients: Array(store.clients.values))
            return
        }
        // What `willPresent` asks: the panes on screen, the same set the
        // runners are told below.
        Notifier.shared.setWatching(visibleTerminals.map(\.id), window: windowID)
        let client = selection?.host.flatMap { store.clients[$0] }
        // Full ids, not `short`: resolving an abbreviation costs the CLI a
        // fleet listing, and this runs on a clock. See the `Watching` command
        // in `crates/cli/src/main.rs`.
        client?.reportWatching(visibleTerminals.map(\.id))
        // And every OTHER runner is told it is showing nothing. A window puts
        // exactly one runner's worktree in the detail pane, so switching from a
        // pane on one runner to a pane on another would otherwise leave the
        // first still believing its pane is being watched — silent for as long
        // as the claim takes to age out, on the runner you just walked away
        // from. Every runner, and outside the guard below, because selecting
        // NOTHING is a way of walking away too: `visibleTerminals` is empty
        // then, and so is the claim every runner should be holding.
        for other in store.clients.values where other !== client {
            other.reportWatching([])
        }
        // Only for somebody there, by the same `Presence` the claim above
        // asks — see `DaemonClient.markSeen(onScreen:)`.
        client?.markSeen(onScreen: visibleTerminals)
    }

    // MARK: - Tiling

    /// The worktree a tiling keystroke acts on.
    var tileTarget: Worktree? { currentWorktree }

    /// What the menu bar can act on in this window: each item dimmed when
    /// what it reads here has nothing to act on (ov-211).
    var menuFocus: MainWindowFocus {
        let scene = selection.flatMap(workspaceScene)
        let terminals = allTerminals
        var focus = MainWindowFocus(
            overlayOpen: showQuickCreate || showPalette, taskOpen: Self.taskOpen(selection),
            hasNavigator: scene?.board != nil)
        focus.sidebarShown = !navigatorHidden
        focus.hasWorktree = currentWorktree != nil
        focus.terminals = terminals.count
        focus.stepsTerminals = terminals.count > 1 || (terminals.count == 1 && terminals.first != selectedPane)
        focus.hasAttention =
            NeedsYouNavigation.step(lastOpened: lastAttention, items: store.needsYou, fleet: store.fleet, showing: selection) != nil
        focus.goesBack = jumpBar.history.canGoBack || Self.goesBack(focus: focusColumn, from: selection, trail: trail, board: scene?.board)
        focus.goesForward = jumpBar.history.canGoForward
        focus.hasJumpBar = scene?.opened != nil || selection?.focus != nil
        focus.focuses = scene?.opened != nil || selection?.focus != nil
        focus.focused = focusColumn
        focus.inWorkspace = scene != nil
        if let scene, let board = scene.board {
            let entries = worktreeEntries(scene)
            let step = { (by: Int) in
                WorkspaceWorktrees.step(
                    from: selection, by: by, in: entries, host: scene.host, workspace: board, fleet: store.fleet) != nil
            }
            focus.nextWorktree = step(1)
            focus.previousWorktree = step(-1)
        }
        focus.workspaces = WorkspaceNumbers.groups(in: store.fleet).flatMap(\.places).filter { $0.number != nil }.count
        focus.makesWorkspaces = !workspaceRepositories.isEmpty
        focus.layout = tileTarget.map { worktree in
            let screen = onScreen(in: worktree)
            let here = selectedPane.flatMap { screen?.group.pane($0.terminal) } ?? screen?.group.panes.first(where: \.focused)
            // The pane Switch Between Terminal and Chat would switch, found
            // as `tile(_:)` finds it.
            let target = here.flatMap { rect in worktree.terminals.first { $0.id == rect.id } }
                ?? selectedTerminal?.terminal
                ?? screen?.group.panes.first.flatMap { pane in worktree.terminals.first { $0.id == pane.id } }
            return LayoutMenuFocus.make(
                group: screen?.group, here: here, layouts: screen?.groups ?? [],
                switchesMode: target.map { $0.canSwitchPaneMode || $0.isAgentPane } ?? false)
        }
        return focus
    }

    /// Whether Back (⌃⌘←) does anything, by `goBack()`'s own steps: leave
    /// Focus, go up a level, or close a loose worktree to its board.
    nonisolated static func goesBack(focus: Bool, from selection: Selection?, trail: Selection?, board: String?) -> Bool {
        let step = WorkspaceNavigation.backStep(focus: focus, oneAtATime: true, from: selection, trail: trail)
        if step.leavesFocus || step.goesTo != nil { return true }
        guard case .looseWorktree? = selection, let selection else { return false }
        return WorkspaceNavigation.closing(selection, board: board) != nil
    }

    func run(_ command: AppCommand) {
        switch command {
        case .newTerminal:
            // Creates immediately. There is no agent to choose: a terminal is a
            // shell, and whatever you run in it — `claude`, `codex`, a build —
            // is detected from the process, not declared in advance. Asking
            // first was a dialog whose answer was already knowable.
            if let worktree = currentWorktree { newTerminal(in: worktree) }

        case .closeTerminal:
            guard let (worktree, terminal) = selectedTerminal else { return }
            Task {
                // Stop, then remove the record. Closing a terminal should leave
                // nothing behind — that is what closing means everywhere else.
                // One action: a Close whose stop was refused fails its remove
                // too, and that is one thing that didn't happen, not two.
                await act(.close, on: worktree, target: terminal.id, subject: Self.quoted(terminal)) { c in
                    await c.stop(terminal: terminal.short)
                    await c.removeTerminal(terminal.short)
                }
                // The runner publishes no layout when a pane closes, so the
                // pane left behind kept the closed one's half of the grid
                // until something else read the layout: a click (checklist
                // O1). Read it now; the view re-sends its viewport when the
                // arrangement changes.
                await store.client(for: worktree)?.refreshLayout(worktree)
                // Nothing to select here. Where the selection goes when a
                // terminal disappears is `healSelection`'s one rule, run from
                // `.onChange(of: store.fleet)` once the removal reaches the
                // merged fleet — and it is that one rule on purpose.
                //
                // This used to call `selectNeighbour(of:)`, which walked the
                // WHOLE fleet and took the first running terminal anywhere,
                // on any runner. It also ran before the removal had reached
                // `store.fleet` — `FleetStore.remerge` runs in a task of its
                // own after the client changes — so it found the closed
                // terminal still listed, moved the selection off it, and left
                // `healSelection` nothing to heal. ⌘W in one worktree landed
                // you in another, often on another runner.
            }

        case .nextTerminal: step(by: 1)
        case .previousTerminal: step(by: -1)

        case .nextAttention:
            // Straight to whatever is waiting on you, in rank order across
            // every runner. On a fleet of twenty this is the difference
            // between the app being useful and being a list. Walked from the
            // item last opened while the window still shows it, else from the
            // top.
            if let next = NeedsYouNavigation.step(
                lastOpened: lastAttention, items: store.needsYou, fleet: store.fleet, showing: selection)
            {
                open(next.item)
            }

        case .newWorktree:
            // Only reachable with a project registered; the panel has nothing
            // to create into otherwise.
            if store.repositories.isEmpty {
                showAddRepository = true
            } else {
                if lastProject.isEmpty, let id = defaultProjectID { lastProject = id }
                showQuickCreate = true
            }
        case .addRepository: showAddRepository = true
        case .newWorkspace:
            // Only where a runner has workspaces, as the palette's item is.
            if !workspaceRepositories.isEmpty { newWorkspaceName = NewWorkspaceName(name: "") }
        case .showBoard:
            // Only reachable with a project registered; a board needs a
            // repository to be scoped to, the same way New Worktree needs one
            // to create into.
            if store.repositories.isEmpty {
                showAddRepository = true
            } else if case .workspace(let host, let id, nil) = selection {
                // Already there — unless "there" has gone, which is said
                // rather than answered with nothing.
                if board(host: host, id: id) == nil {
                    errorBanner = missingBoardSentence(host: host)
                }
            } else if let target = boardTarget {
                selection = .workspace(host: target.host, workspace: target.workspace.id, focus: nil)
            } else {
                // Never a guess between several. The switcher lists them for
                // exactly this case.
                errorBanner = "Select a workspace first."
            }
        case .back, .forward: step(back: command == .back)
        case .jumpBar: jumpBar.request += 1
        case .focusColumn:
            if selection.flatMap(workspaceScene)?.opened != nil {
                apply(WorkspaceNavigation.boardStep(.toggleFocus, from: boardState))
            } else if selection?.focus != nil {
                focusColumn.toggle()
            }
        case .focusConversation, .focusBoard, .focusTask:
            focusWorkspaceColumn(command)
        case .switchWorkspace: switcherRequest += 1
        case .nextTaskTab: stepTaskTab(by: 1)
        case .previousTaskTab: stepTaskTab(by: -1)
        case .nextWorktree: stepWorktree(by: 1)
        case .previousWorktree: stepWorktree(by: -1)
        case .openInEditor: openInPreferredEditor()
        // Everything the old sidebar's Refresh button read, on every
        // runner: the fleet, and its repositories, roots and layouts, as a
        // reconnection re-reads them (`DaemonClient.onReconnect`).
        case .reload:
            Task {
                for client in store.clients.values {
                    await client.refresh()
                    await client.refreshRepositories()
                    await client.refreshRoots()
                    await client.refreshLayouts()
                }
            }
        case .showShortcuts: showShortcuts = true
        // In a workspace, ⌘F filters its navigator's tasks (ov-103): the
        // find a person in a list of tasks reaches for, bringing the
        // navigator back if it was put away. Anywhere else, the palette,
        // which finds the same workspaces, tasks and agents the old
        // sidebar's search did (ov-178).
        case .search:
            // A loose worktree draws its board's navigator too.
            if selection.flatMap(workspaceScene)?.board != nil {
                navigatorHidden = false
                boardFilterRequest += 1
            } else {
                showPalette = true
            }

        case .markAllRead:
            if let scene = selection.flatMap(workspaceScene), let board = scene.board {
                boardStores["\(scene.host)/\(board)"]?.askToMarkAllRead(markReadConfirmation)
            }

        // Toggles rather than opens. ⌘P on an open palette is what a hand
        // reaches for when it changed its mind, and every switcher on this
        // machine closes that way.
        case .commandPalette: showPalette.toggle()

        // The window's one sidebar is the navigator (ov-178).
        case .toggleSidebar: toggleNavigator()

        // Moving through a diff belongs to the pane showing one, and only to
        // the FOCUSED one — see `ChangesPane.isFocused`. Listed rather than
        // caught by a `default` so the next command added to `AppCommand`
        // still fails to compile until somebody decides where it goes.
        case .diffNextHunk, .diffPreviousHunk,
            .diffNextFile, .diffPreviousFile,
            .diffNextCommit, .diffPreviousCommit, .diffFirstCommit,
            // Not a movement, but pane-scoped for the same reason: the
            // watermark it moves belongs to the worktree whose diff you were
            // reading, and a window can hold a diff and three terminals.
            .diffMarkRead:
            break
        }
    }

    /// Carry out whatever was chosen in the palette.
    ///
    /// Every case here routes into a method that already existed, and that is
    /// the whole design of `PaletteAction`: the palette knows what you picked
    /// and nothing about what picking it means, so opening a terminal from the
    /// panel and clicking it in the window cannot drift apart.
    func perform(_ action: PaletteAction) {
        showPalette = false
        switch action {
        case .openTerminal(let worktree, let terminal):
            let host = store.fleet.worktrees.first { $0.id == worktree }?.host ?? ""
            land(on: PaneRef(host: host, worktree: worktree, terminal: terminal))

        case .openWorktree(let id):
            guard let worktree = store.fleet.worktrees.first(where: { $0.id == id }) else { return }
            selection = Self.opening(worktree, terminal: nil, in: store.fleet)

        case .newTerminal(let id):
            guard let worktree = store.fleet.worktrees.first(where: { $0.id == id }) else {
                return
            }
            newTerminal(in: worktree)

        case .openWorkspace(let host, let id):
            selection = .workspace(host: host, workspace: id, focus: nil)

        case .openTask(let host, let workspace, let id):
            openTask(id, host: host, workspace: workspace)

        case .newWorkspace(let name):
            newWorkspaceName = NewWorkspaceName(name: name)

        case .newWorktree(let described):
            if store.repositories.isEmpty {
                showAddRepository = true
                return
            }
            if lastProject.isEmpty, let id = defaultProjectID { lastProject = id }
            // What was typed into the palette carries over as the description,
            // because in this panel it nearly always was one. It never
            // overwrites a draft already in progress — that draft is often the
            // thing someone opened the palette to go and look something up for.
            if !described.isEmpty, taskDraft.isEmpty { taskDraft = described }
            showQuickCreate = true

        case .togglePaneMode(let worktree, let terminal):
            guard
                let ws = store.fleet.worktrees.first(where: { $0.id == worktree }),
                let target = ws.terminals.first(where: { $0.id == terminal })
            else { return }
            Task { await togglePaneMode(target, in: ws) }
        }
    }

    /// Hand the worktree on screen to an editor, from the keyboard.
    ///
    /// The same act as clicking the title bar control, routed through the same
    /// two rules so a click and ⇧⌘E cannot come to different answers: the
    /// editor is whatever `Editors.preferred` says for THIS worktree's runner,
    /// and using it does not change the preference — only picking one out of
    /// the menu does. See `OpenInEditorButton`'s primary action, which this
    /// mirrors deliberately rather than reimplements.
    ///
    /// `refresh()` first, because the menu bar has no `onAppear` to hang it on.
    /// The control gets its probe when it draws; a shortcut can be the first
    /// thing pressed after launch, and without this it would report "no editors
    /// found" on a Mac with four of them installed.
    func openInPreferredEditor(_ chosen: Worktree? = nil) {
        guard let worktree = chosen ?? detailWorktree else {
            errorBanner = "Open a worktree first — there is nothing to hand to an editor."
            return
        }
        let editors = Editors.shared
        editors.refresh()
        let runner = worktree.host ?? ""
        guard let editor = editors.preferred(host: runner) else {
            // Not an error. Nothing is wrong with an app that has never been
            // told which editor you use — so this opens the place you say so,
            // exactly as clicking the control with no editor configured does.
            EditorSettingsLink.open(openSettings)
            return
        }
        Task {
            if let problem = await editors.open(worktree, with: editor) { editorError = problem }
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

enum TerminalAction { case restart, dismissLost, stop, useAsOrchestrator, stopBeingOrchestrator }

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
