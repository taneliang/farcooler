import AgentKit
import AppKit
import SwiftUI

struct ContentView: View {
    @StateObject private var store = FleetStore()
    /// This window's identity in `Notifier`'s per-window record of what is on
    /// screen, so a second window adds to it rather than overwriting it.
    @State private var windowID = UUID()
    @ObservedObject private var preferences = Preferences.shared
    @ObservedObject private var themes = Themes.shared
    @Environment(\.openSettings) private var openSettings
    @Environment(\.colorScheme) private var colorScheme
    @State private var selection: Selection?
    @State private var expanded: Set<String> = []
    /// Which projects have their hidden worktrees showing. Collapsed is the
    /// point of hiding, so absence means collapsed.
    @State private var hiddenExpanded: Set<String> = []
    /// Which repositories have their Unclaimed group open, by
    /// `SidebarEntry.group`. Collapsed by default: absence means collapsed.
    @State private var unclaimedExpanded: Set<String> = []

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
    private struct NewWorktreeIntent: Identifiable {
        /// The project whose header was clicked, by display name — which is
        /// what the sidebar groups by and what the sheet matches on. Empty
        /// when the control named no project at all.
        var project: String = ""
        /// The runner that project is on. Only meaningful beside `project`:
        /// two runners can share a display name, and only one of them is the
        /// project the header actually named. Empty means "none was named",
        /// which is the sidebar's own `+` and the empty-state buttons — the
        /// one case where letting the sheet choose a default is legitimate
        /// rather than the picker again in disguise.
        var host: String = ""
        var id: String { "\(host)\u{1}\(project)" }
    }

    @State private var newWorktreeIntent: NewWorktreeIntent?
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
    @State private var boardStores: [String: TaskBoardStore] = [:]
    @State private var showAddRepository = false
    @State private var showAdd = false
    @State private var showShortcuts = false
    @State private var showAbout = false
    @State private var query = ""
    @State private var showQuickCreate = false
    /// The ⌘N panel's start in flight, kept here so it survives the panel
    /// closing and reopening.
    @StateObject private var taskSubmission = TaskSubmission()
    @AppStorage("tasks.lastProject") private var lastProject = ""
    /// The workspace selection that was on screen when the app last closed,
    /// as `SelectionMemory.encode` writes it. Recorded as it changes rather
    /// than on quit: an app that is force quit, crashes, or is killed by a
    /// rebuild never gets a last word.
    ///
    /// Reopening somewhere else is a small thing that costs a real one: the
    /// workspace you were in is the reason you came back. It replaces
    /// `fleet.lastTerminal`, which `SelectionMemory.migrate` maps once.
    @AppStorage(SelectionMemory.key) private var lastSelection = ""
    /// Whether the launch rule has chosen where the window opens. See
    /// `settleLaunch`.
    @State private var launched = false
    @FocusState private var searchFocused: Bool
    @State private var removeWorktree: Worktree?
    @State private var removeRepository: RepositoryToRemove?
    @State private var showResumeBranch = false
    @State private var showPalette = false
    /// Quick-create's draft, reachable from here so that what was typed into
    /// the palette arrives in the panel that acts on it. See `perform`.
    @AppStorage("tasks.draft") private var taskDraft = ""
    /// One divider resize at a time. See `resizeDivider`.
    @State private var resizingDivider = false
    /// Set when `setPaneMode` comes back `confirmationRequired` — a turn is in
    /// flight and switching would cancel it. Drives a sheet the same way
    /// `removeWorktree` does, rather than a banner: this refusal has an
    /// answer ("cancel it anyway?") a banner cannot offer.
    @State private var pendingPaneModeSwitch: PaneModeConfirmation?
    /// What an editor said when it would not start.
    ///
    /// Its own state rather than routed through `errorBanner`, which is
    /// rendered only by `fleetPlaceholder`'s error branch and by the general
    /// banner below — the opposite of when this control exists. See
    /// `EditorErrorBanner`.
    @State private var editorError: String?
    /// What a refused or failed action said, shown by the banner over the
    /// detail pane.
    ///
    /// Its own state now rather than one client's `lastError`: there is no
    /// longer one client whose `lastError` could stand for "the last thing
    /// that went wrong" — an action against one runner must not be reported
    /// through, or cleared by, a banner bound to a different one. Set by
    /// `act(on:default:_:)`, which is also the one place that clears it: on
    /// refusal, and by copying back whatever the client itself set on
    /// failure.
    @State private var errorBanner: String?
    /// Workspaces whose orchestrator this app has asked to start and the
    /// runner hasn't answered, by `orchestratorKey`.
    @State private var startingOrchestrators = OrchestratorStarts()
    /// A Replace Orchestrator waiting on its confirmation.
    @State private var orchestratorReplacement: OrchestratorReplacement?
    /// Use as Orchestrator on a workspace that has one, until confirmed.
    @State private var adoptionPending: OrchestratorAdoptionPending?

    /// The pane last clicked or focused, which the keyboard acts on while
    /// it's on screen. See `WorkspaceScreen.keyPane`: with a task open, the
    /// conversation and the task's agent are both on screen, and ⌃B has to
    /// act on the one you were last in.
    @State private var keyPane: PaneRef?
    /// The agent each task's column shows, by task id, when several are on
    /// it and one was picked.
    @State private var chosenAgents: [String: String] = [:]
    /// The Needs You item ⌃⌘N last opened, by its key: where the next
    /// press goes on from, while the window is still showing it.
    @State private var lastAttention: String?
    /// Which column a workspace's one-column form shows, by `host|workspace`.
    @State private var workspacePicks: [String: WorkspacePick] = [:]
    /// Focus (⌃⌘↩): a task or worktree opened, alone, without the
    /// orchestrator's rail or the task's own text.
    @State private var focusColumn = false
    /// The orchestrator popped open from its rail, over the task or
    /// worktree opened. Closed on going anywhere else.
    @State private var orchestratorPeek = false
    /// The detail's width, as the workspace view last measured it: which of
    /// a workspace's columns are on screen. Nil until one has been drawn.
    @State private var detailWidth: CGFloat?
    /// The task a worktree was opened from with Open Worktree: where Back
    /// goes. See `WorkspaceNavigation`.
    @State private var trail: Selection?
    /// The worktree Open Worktree opened from `trail`.
    @State private var trailWorktree: String?
    /// The task whose changes the keyboard is in: the Diff menu's
    /// shortcuts are for the diff you clicked into.
    @State private var changesFocus: String?
    /// Whether the one-time workspaces tip is up. See `WorkspacesTip`.
    @State private var showWorkspacesTip = WorkspacesTip.shouldShow()
    /// The key monitor that turns Esc into Back. See `EscapeBack`.
    @State private var escapeMonitor: Any?
    /// ⌥⌘2 gave the board the keyboard: no terminal takes typed keys until
    /// a pane is clicked or chosen again.
    @State private var keyboardOnBoard = false
    /// This window, for the Esc monitor, which hears every window's keys.
    @State private var windowBox = WindowBox()
    /// New Workspace…, with the name typed into the palette, while its
    /// sheet is up.
    @State private var newWorkspaceName: NewWorkspaceName?
    struct NewWorkspaceName: Identifiable {
        let name: String
        var id: String { name }
    }
    /// Which workspaces are open in the sidebar, their worktrees listed, as
    /// `SidebarEntry.openKey`s, one a line. Empty by default (spec §9).
    @AppStorage("sidebar.openWorktrees") private var openWorktrees = ""
    /// When each workspace's orchestrator start began, by `host|workspace`:
    /// this app's, or one first seen starting. See `ConversationColumn.slowStart`.
    @State private var orchestratorStartedAt: [String: Date] = [:]

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
        NavigationSplitView {
            sidebar
        } detail: {
            // Top-aligned over the detail pane specifically, not the sidebar —
            // `fleetPlaceholder` already owns the sidebar's pre-load state, and
            // this banner only appears once something has actually been acted
            // on, so the two never draw at once.
            detail
                // Attached here rather than beside each of the four
                // `navigationTitle` calls, which sit in three different views
                // that would each need the failure channel threaded down to
                // them. This is the one place that decides which worktree the
                // window is showing, which is exactly what the control acts on.
                .openInEditorToolbar(worktree: detailWorktree) { editorError = $0 }
                .toolbar {
                    // Not offered on a runner that has already said it cannot
                    // read changes at all. Its daemon predates the whole
                    // feature, so the split would open a pane running a
                    // subcommand that runner has never heard of — a dead pane
                    // where a diff was asked for, with nothing saying why.
                    // For a worktree opened whole, with a terminal or not,
                    // or the main checkout beside the Orchestrator column
                    // (ov-78); not for a task, whose column shows its
                    // changes already, without a pane (spec R3).
                    if let ws = WorkspaceScreen.changesTarget(
                        selection, in: store.fleet, repositories: repositoryIDs(selection?.host ?? "")),
                        store.client(for: ws)?.changesSupported != false
                    {
                        ToolbarItem(placement: .primaryAction) {
                            Button {
                                toggleChangesPane(in: ws)
                            } label: {
                                Label("Changes", systemImage: "plusminus")
                            }
                            // Lit while one is open, the way a toggle in a
                            // toolbar says which state you are in. This is a
                            // `Button` rather than a `Toggle` because the two
                            // directions are not symmetrical: opening splits a
                            // pane, closing kills one, and a `Toggle`'s binding
                            // would have to pretend they were one value.
                            .symbolVariant(changesPane(in: ws) == nil ? .none : .fill)
                            .help(
                                changesPane(in: ws) == nil
                                    ? "Show what this worktree changed, in a pane"
                                    : "Close the changes pane")
                        }
                    }
                }
                .overlay(alignment: .top) {
                    ErrorBanner(message: errorBanner) { errorBanner = nil }
                }
                // A message arriving on a keystroke, so the same snappy preset
                // `PrefixHintOverlay` uses for its chip.
                .animation(.snappy(duration: 0.22), value: errorBanner)
        }
        .task {
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
                store.clients[""]?.lastError = problem
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
                    !showPalette, !showQuickCreate,
                    EscapeBack.goesBack(
                        responder: window.firstResponder, selection: selection,
                        focusColumn: focusColumn || orchestratorPeek)
                else { return event }
                goBack()
                return nil
            }
        }
        // Recorded as it changes rather than on quit: an app that is force
        // quit, crashes, or is killed by a rebuild never gets a last word, and
        // this is exactly the state worth surviving all three.
        // Tells the menu bar the main window is key, and whether an overlay
        // is open over it. See `MainWindowFocus`.
        .focusedSceneValue(
            \.mainWindow, MainWindowFocus(overlayOpen: showQuickCreate || showPalette))
        .onCommand { command in run(command) }
        .onTileCommand { command in Task { await tile(command) } }
        .onSelectIndex { index in selectTerminal(at: index) }
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
        // A column that comes on screen, or goes, as the detail is resized
        // or a pick changes: what's seen and watched follows it.
        .onChange(of: detailWidth) { _, _ in markVisibleSeen() }
        .onChange(of: workspacePicks) { _, _ in markVisibleSeen() }
        .onChange(of: focusColumn) { _, _ in markVisibleSeen() }
        .onChange(of: orchestratorPeek) { _, _ in markVisibleSeen() }
        .onChange(of: store.needsYouSettled) { _, _ in settleLaunch() }
        .onChange(of: selection) { _, now in
            if let saved = SelectionMemory.encode(now) { lastSelection = saved }
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
            // Cleared on every navigation, so a refusal or failure left behind
            // on one pane does not go on describing a pane the user is no
            // longer looking at. Selection is the one thing every navigation
            // path — sidebar click, ⌘P, ⌘], ⌃B o, closing a terminal — funnels
            // through, which makes it the narrowest point that sees every one
            // of them.
            errorBanner = nil
            // Focus, the orchestrator popped open, and a board holding the
            // keyboard are about the view that was on screen, not the next
            // one. Focus and the popped-open orchestrator stay while the
            // same thing is open, whichever of its panes is selected.
            if !WorkspaceSelection.samePlace(old, new) {
                focusColumn = false
                orchestratorPeek = false
            }
            keyboardOnBoard = false
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
            if let arrived,
                let worktree = worktree(host: arrived.host, id: arrived.worktree),
                let c = store.client(for: worktree),
                let holder = c.group(holding: arrived.terminal, in: arrived.worktree),
                let pane = holder.pane(arrived.terminal),
                !holder.isActive || !pane.focused
            {
                Task {
                    await act(on: worktree) { client in
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
                    await act(on: worktree) { client in
                        await client.selectLayout(shown.id, in: worktree)
                    }
                }
            }

            if let arrived {
                // Stamped here rather than in the palette, so every way of
                // arriving counts: a sidebar click, ⌘], ⌃B o, a jump from the
                // palette itself. A switcher that only learned from its own
                // choices would order by where you had used IT, not by where you
                // have been.
                VisitLog.shared.visited(arrived.terminal)
            }

            // Opening a terminal is what ends `done`. Being LISTED is still not
            // being read — the sidebar shows every terminal on the runner and
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
        .sheet(isPresented: $showAbout) { AboutSheet() }
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
                EditorErrorBanner(message: editorError) { self.editorError = nil }
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
                        // host would leave that host's sidebar rows stale.
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
                // about which project this is. An empty host is the sidebar's
                // own `+` and the empty-state buttons, none of which named a
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
                // path in `act(on:default:)` keeps its own prefix: nothing
                // there draws a sentence above it.
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
                // one place that says what "go to it" means: expand the
                // worktree in the sidebar, then select its terminal.
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
                let result = await act(
                    on: ws, default: .failed("This runner can’t be reached right now.")
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
        .sheet(item: $pendingPaneModeSwitch) { pending in
            PaneModeConfirmSheet(message: pending.message) {
                await act(
                    on: pending.worktree, default: .failed("This runner can’t be reached right now.")
                ) { c in
                    await c.setPaneMode(pending.terminal, mode: pending.mode, force: true)
                }
            }
        }
    }

    // MARK: - Sidebar

    /// The sidebar's rows: worktrees matching the search, under their
    /// repository and workspace, on every runner. See
    /// `ContentView.sidebarRows(fleet:query:silentHosts:)`.
    ///
    /// Hidden worktrees are separated rather than filtered out: they still
    /// belong to the project, and a collapsed section at the bottom is how you
    /// get back to one.
    ///
    /// Each row has a stable `id`, not its position: `ForEach` used to key
    /// this list by `.offset`, so inserting a group renumbered every header
    /// after it and any transient `@State` (row hovering) followed the index
    /// instead of following the row it belonged to.
    private var sidebarEntries: [SidebarEntry] {
        Self.sidebarRows(fleet: store.fleet, query: query, silentHosts: silentHosts, open: isOpen)
    }

    /// Whether a workspace's row is open: as it was left
    /// (`sidebar.openWorktrees`), or because the selection is inside it.
    private func isOpen(_ key: String) -> Bool {
        if openWorktreesSet.contains(key) { return true }
        guard case .workspace(let host, let id, .worktree?)? = selection else { return false }
        return key == SidebarEntry.openKey(host: host, workspace: id)
    }

    private var openWorktreesSet: Set<String> {
        Set(openWorktrees.split(separator: "\n").map(String.init))
    }

    private func toggleWorktrees(_ key: String) {
        var set = openWorktreesSet
        if set.contains(key) { set.remove(key) } else { set.insert(key) }
        openWorktrees = set.sorted().joined(separator: "\n")
    }

    /// New Worktree… from a workspace row's menu: in the workspace first,
    /// so the sheet claims what it makes for it
    /// (`claim(newWorktreeIn:…)`).
    private func newWorktreeAction(for entry: SidebarEntry) -> () -> Void {
        let host = entry.host
        let project = entry.project
        let workspace = entry.workspace?.id
        return {
            if let workspace { selection = .workspace(host: host, workspace: workspace, focus: nil) }
            newWorktree(host: host, project: project)
        }
    }

    /// Whether a workspace's row is lit for what's open in it: with nothing
    /// or a task open, yes; with a worktree open, that worktree's row under
    /// it is.
    nonisolated static func highlightsWorkspace(_ focus: Focus?) -> Bool {
        if case .worktree? = focus { return false }
        return true
    }

    /// Show Board: the workspace, with its board on screen, in the
    /// one-column form too.
    private func showBoard(host: String, workspace: String) {
        workspacePicks["\(host)|\(workspace)"] = .board
        selection = .workspace(host: host, workspace: workspace, focus: nil)
    }

    /// Repositories New Workspace… can make one in: on runners with
    /// workspaces.
    private var workspaceRepositories: [(host: String, repository: Repository)] {
        store.repositories.filter { store.fleet.runnerWorkspaces[$0.host] != nil }
    }

    /// Every workspace in the sidebar, for the palette.
    private var paletteWorkspaces: [PaletteWorkspace] {
        Self.sidebarRows(fleet: store.fleet).compactMap { row in
            guard case .workspace(let name) = row.kind, let workspace = row.workspace else { return nil }
            return PaletteWorkspace(
                host: row.host, id: workspace.id, name: name, repository: row.project,
                hasOrchestrator: row.orchestrator != nil)
        }
    }

    /// Every task on a board this window has read, for the palette.
    private var paletteTasks: [PaletteTask] {
        boardStores.values.flatMap { board -> [PaletteTask] in
            let host = board.client.target
            let name = board.title
            return board.board.columns.flatMap(\.rows).map {
                PaletteTask(
                    host: host, workspace: board.workspace.id, workspaceName: name, id: $0.id, key: $0.key,
                    title: $0.title, status: $0.status)
            }
        }
    }

    /// A project header's repository, on its way to `RemoveRepositorySheet`.
    /// `.sheet(item:)` needs `Identifiable`; a bare tuple is not one.
    private struct RepositoryToRemove: Identifiable {
        let host: String
        let repository: Repository
        var id: String { "\(host)\u{1}\(repository.id)" }
    }

    /// Whether to name runners at all.
    private var showHosts: Bool { store.hosts.count > 1 }

    /// Runners with nothing to show yet still get a header.
    ///
    /// A runner that has never connected has no rows, and without this it
    /// would simply be missing — leaving you to wonder where it went rather
    /// than seeing that it needs attention.
    ///
    /// The local runner is excluded: it already has a dedicated placeholder
    /// (`fleetPlaceholder`, below) that reads its loading/error state in more
    /// detail than a bare header could say. Only a REMOTE runner that has
    /// contributed nothing gets one of these instead.
    private var silentHosts: [String] {
        let present = Set(store.fleet.worktrees.map { $0.host ?? "" })
        return store.hosts.filter { !present.contains($0) && !$0.isEmpty }
    }

    /// One runner's daemon, as something the sidebar can offer to replace —
    /// or nil, which is the ordinary case and the quiet one.
    ///
    /// This is the only place that pairs a host with the client that can act
    /// on it, so the views downstream never learn whether a runner is updated
    /// over ssh or out of this app's own bundle. `DaemonClient.updateDaemon()`
    /// knows, and it is the only thing that needs to.
    ///
    /// Gated on `offersUpdate` rather than on "not current": a runner whose
    /// version could not be read is not a runner to offer an update for, and a
    /// runner nobody can reach is not one either. See `DaemonSkew`.
    private func daemonUpdate(for host: String) -> DaemonUpdateTarget? {
        guard let client = store.clients[host], client.daemonSkew.offersUpdate else { return nil }
        return DaemonUpdateTarget(host: host, skew: client.daemonSkew) {
            await client.updateDaemon()
        }
    }

    /// The repository a project header stands for: by its uuid, which a
    /// worktree row carries as `repository_id`, and by its display name only
    /// from a CLI too old to send that — two repositories can share a name.
    private func repository(host: String, id: String?, project: String) -> Repository? {
        let here = store.repositories.filter { $0.host == host }.map(\.repository)
        if let id { return here.first { $0.id == id } }
        return here.first { $0.displayName == project }
    }

    /// Start a worktree in a named project.
    ///
    /// The sheet already has a repository picker; this just answers it in
    /// advance, because someone clicking `+` on a project header has already
    /// said which one.
    private func newWorktree(host: String, project: String) {
        newWorktreeIntent = NewWorktreeIntent(project: project, host: host)
    }

    /// A terminal in the repository's own checkout.
    ///
    /// The main checkout is always present in the fleet — the daemon adopts it
    /// the moment a repository is registered — so this finds it rather than
    /// asking the CLI to produce or locate it.
    private func newMainTerminal(host: String, repositoryID: String?, project: String) async {
        guard
            let worktree = Self.mainCheckout(
                host: host, repositoryID: repositoryID, project: project, in: store.fleet.worktrees)
        else { return }
        await act(on: worktree) { client in
            await client.createTerminal(worktree: worktree.short, preset: "shell", title: "")
            await client.refresh()
        }
    }

    /// A repository's own checkout on `host`: by the repository's uuid, which
    /// a header carries as `repositoryID`, and by its display name only from
    /// a CLI too old to send one — two repositories on one runner can share a
    /// name, and the first of them is not the one whose header was clicked.
    static func mainCheckout(
        host: String, repositoryID: String?, project: String, in worktrees: [Worktree]
    ) -> Worktree? {
        worktrees.first {
            guard $0.isMainCheckout, ($0.host ?? "") == host else { return false }
            if let repositoryID { return $0.repositoryID == repositoryID }
            return $0.repository == project
        }
    }

    /// A plain, non-optional entry point into `newMainTerminal(host:repositoryID:project:)`.
    ///
    /// `ProjectHeader.onNewTerminal` is optional — nil for a silent host's
    /// placeholder — and a ternary handing back `Task { await ... }` directly
    /// for the non-nil branch leaves the compiler unable to settle on which
    /// `Task.init` overload the closure means, reported as an unhelpful
    /// "ambiguous use of 'init(name:priority:operation:)'" with no line
    /// pointing at the ternary itself. A named function sidesteps it.
    private func startMainTerminal(host: String, repositoryID: String?, project: String) {
        Task { await newMainTerminal(host: host, repositoryID: repositoryID, project: project) }
    }

    /// A drop that landed: what it means, then tell the runner.
    ///
    /// Here rather than in the row because only this level can see a whole
    /// project group. See `dropMeaning` for which drops mean what; one it
    /// has no meaning for never got this far, because the row asked it
    /// while hovering (`WorktreeDrag.accepts`), and is ignored here too.
    private func landed(_ done: WorktreeDrag.Completion) {
        guard
            let (dragged, meaning) = Self.dropMeaning(
                of: done.dragged, onto: done.target, in: store.fleet, assigns: Self.assigns(store))
        else { return }
        switch meaning {
        case .reorder:
            if case .worktree(let target, let edge) = done.target { reorder(done.dragged, to: target, edge) }
        case .assign(let workspace):
            move(dragged, to: workspace) {
                // Dropped between two of that workspace's rows: there, too.
                // A failure now is said in our words: the move itself held.
                if case .worktree(let target, let edge) = done.target {
                    reorder(
                        done.dragged, to: target, edge,
                        failure: DaemonClient.movedButNotPlaced(dragged, to: workspace))
                }
            }
        }
    }

    /// Move a worktree to another workspace (`farcooler worktree assign`):
    /// a drop on a workspace row, or Move to Workspace ▸.
    private func move(_ worktree: Worktree, to workspace: WorkspaceSummary, then: @escaping () -> Void = {}) {
        Task {
            // Refused first, as every write here is; see `act(on:_:)`.
            if let why = store.refusal(for: worktree) {
                errorBanner = "Cannot do that: \(why)"
                return
            }
            guard let client = store.client(for: worktree) else { return }
            // Nothing moves on screen until the runner says it has: a
            // refused move leaves the row where it was, with the sentence.
            if let refused = await client.assignWorktree(worktree, to: workspace) {
                errorBanner = refused
                return
            }
            then()
        }
    }

    /// Put `dragged` on `edge` of `target` and tell the runner.
    ///
    /// The runner is sent the group's WHOLE order, not "move this one" — see
    /// `WorktreeReorder` in the proto for why an index alone would be
    /// meaningless against a list this has already filtered. The order is the
    /// repository's, which each workspace's rows keep, so a drop within a
    /// workspace lands where it was dropped — and so does one that moved the
    /// worktree to that workspace first. Every card in a group is on one
    /// runner and in one project, which is what makes a single call to a
    /// single client the whole of it.
    ///
    /// `failure`, when given, is the banner for a reorder the runner
    /// refuses, in place of the CLI's own words.
    private func reorder(
        _ dragged: String, to target: String, _ edge: WorktreeOrder.Edge, failure: String? = nil
    ) {
        guard
            let group = sidebarEntries.first(where: { g in
                g.kind == .repository && g.worktrees.contains { $0.id == dragged }
            })
        else { return }
        let shown = group.worktrees
        guard shown.contains(where: { $0.id == target }) else { return }
        let ids = shown.map(\.id)
        let next = WorktreeOrder.moved(ids, dragging: dragged, to: target, edge)
        // A drop that changes nothing costs no round trip. It is not free: a
        // reorder makes every other connected client re-read the fleet.
        guard next != ids, let anchor = shown.first else { return }
        let order = next.compactMap { id in shown.first { $0.id == id }?.short }
        guard let failure else {
            Task { await act(on: anchor) { client in await client.reorderWorktrees(order) } }
            return
        }
        Task {
            if let why = store.refusal(for: anchor) {
                errorBanner = "Cannot do that: \(why)"
                return
            }
            guard let client = store.client(for: anchor) else { return }
            if !(await client.reorderWorktrees(order)) { errorBanner = failure }
        }
    }

    /// What dropping a worktree's row means.
    enum DropMeaning: Equatable {
        /// Put it between two rows of its own workspace, or of Unclaimed.
        case reorder
        /// Give it to this workspace (`farcooler worktree assign`).
        case assign(WorkspaceSummary)
    }

    /// What dropping `dragged` on `target` does, or nil for nothing — and a
    /// drop that would do nothing is refused while hovering, so it draws no
    /// insertion line.
    ///
    /// - On a row in its own workspace, or Unclaimed onto Unclaimed: a
    ///   reorder, as before.
    /// - On a row in another workspace of its repository, or on that
    ///   workspace's header: it moves there, the drag's version of `farcooler
    ///   worktree assign`. Only on a runner with `workstreams` (`assigns`),
    ///   which is the one that has the command.
    /// - Never into Unclaimed: nothing un-assigns a worktree, and the drop
    ///   has no command to send.
    /// - Never the main checkout: it is the repository's own directory, where
    ///   every workspace's orchestrator runs, not one workspace's worktree.
    /// - Never across repositories or runners — Unclaimed onto Unclaimed
    ///   included, which `sameSidebarPlace` alone would call one place.
    /// - Never a hidden row or onto one, nor a row onto itself. Neither can
    ///   happen from the sidebar today — hidden rows are no drag source or
    ///   target, and `WorktreeDrag.allows` refuses a row's own — so these
    ///   hold the rule to what it says rather than guard a live path.
    ///
    /// Asked twice, from one place: by the rows while a card hovers
    /// (`WorktreeDrag.accepts`) and by `landed` before anything is written,
    /// both through `dropMeaning(of:onto:in:assigns:)`.
    static func dropMeaning(
        _ dragged: Worktree, onto target: WorktreeDrag.Target, in fleet: Fleet, assigns: Bool
    ) -> DropMeaning? {
        let host = dragged.host ?? ""
        let listed = fleet.runnerWorkspaces[host] ?? []
        func workspace(_ id: String?) -> WorkspaceSummary? {
            guard let id, let repository = dragged.repositoryID else { return nil }
            return listed.first { $0.id == id && $0.repository == repository }
        }
        func assign(_ to: WorkspaceSummary?) -> DropMeaning? {
            guard assigns, !dragged.isMainCheckout, let to, dragged.workspace != to.id else { return nil }
            return .assign(to)
        }
        switch target {
        case .workspace(let id):
            return assign(workspace(id))
        case .worktree(let id, _):
            guard
                let onto = fleet.worktrees.first(where: { ($0.host ?? "") == host && $0.id == id }),
                onto.id != dragged.id, !onto.isHidden, !dragged.isHidden,
                (onto.repositoryID ?? onto.repository) == (dragged.repositoryID ?? dragged.repository)
            else { return nil }
            if sameSidebarPlace(dragged, onto, in: fleet) { return .reorder }
            return assign(workspace(onto.workspace))
        }
    }

    /// `dropMeaning` for a dragged worktree's id, with the worktree it is:
    /// the one seam the hover and the drop both go through, so the two can't
    /// come to disagree. `assigns` says whether a worktree's runner has
    /// `workstreams`; see `assigns(_:)`. Nil for an id the fleet doesn't
    /// have.
    static func dropMeaning(
        of dragged: String, onto target: WorktreeDrag.Target, in fleet: Fleet,
        assigns: (Worktree) -> Bool
    ) -> (Worktree, DropMeaning)? {
        guard let moving = fleet.worktrees.first(where: { $0.id == dragged }),
            let meaning = dropMeaning(moving, onto: target, in: fleet, assigns: assigns(moving))
        else { return nil }
        return (moving, meaning)
    }

    /// Whether a worktree's runner can take `worktree assign`, as `store`
    /// knows it at the moment of asking.
    static func assigns(_ store: FleetStore) -> (Worktree) -> Bool {
        { store.client(for: $0)?.daemonBuild?.can("workstreams") ?? false }
    }

    /// Whether two worktrees are drawn under the same workspace, or both in
    /// Unclaimed: the rows a drag may reorder between.
    static func sameSidebarPlace(_ a: Worktree, _ b: Worktree, in fleet: Fleet) -> Bool {
        let listed = Set((fleet.runnerWorkspaces[a.host ?? ""] ?? []).map(\.id))
        func place(_ w: Worktree) -> String? { w.workspace.flatMap { listed.contains($0) ? $0 : nil } }
        return place(a) == place(b)
    }

    /// A workspace header's menu: its board, its orchestrator, its charter.
    private func workspaceActions(_ entry: SidebarEntry) -> WorkspaceHeaderActions? {
        guard let workspace = entry.workspace, !workspace.isImplicit else { return nil }
        let host = entry.host
        return WorkspaceHeaderActions(
            // The board's own gate: a runner without `tasks` has none.
            hasBoard: store.clients[host]?.daemonBuild.map { $0.can("tasks") } ?? true,
            hasOrchestrator: entry.orchestrator != nil,
            charter: CharterAccess.of(workspace, host: host),
            onShowBoard: { showBoard(host: host, workspace: workspace.id) },
            onStart: { harness, replace in
                switch OrchestratorRequest(harness: harness, replace: replace) {
                case .confirmReplace(let harness):
                    orchestratorReplacement = OrchestratorReplacement(
                        host: host, workspace: workspace, harness: harness)
                case .start(let harness):
                    startOrchestrator(workspace, host: host, harness: harness, replace: false)
                }
            },
            onShowCharter: { url in
                if !NSWorkspace.shared.open(url) {
                    errorBanner = "Couldn’t open \(workspace.name)’s charter. It may have been moved or deleted."
                }
            })
    }

    /// Start `workspace`'s orchestrator, or replace the one running. The row
    /// says "Starting Orchestrator…" until the runner answers; a refusal is
    /// the banner, in `orchestratorRefusal`'s words.
    private func startOrchestrator(
        _ workspace: WorkspaceSummary, host: String, harness: OrchestratorHarness, replace: Bool
    ) {
        if let why = store.refusal(for: host) {
            errorBanner = "Cannot do that: \(why)"
            return
        }
        guard let client = store.clients[host] else { return }
        guard startingOrchestrators.begin(workspace, host: host) else { return }
        orchestratorStartedAt["\(host)|\(workspace.id)"] = Date()
        Task {
            let refused = await client.startOrchestrator(workspace, harness: harness, replace: replace)
            startingOrchestrators.end(workspace, host: host)
            if let refused { errorBanner = refused }
        }
    }

    /// Every pane on one runner, for the board to find the ones working a
    /// task — or none while that runner is refused. See `BoardAgents.on`.
    private func boardAgents(host: String, client: DaemonClient) -> BoardAgents {
        BoardAgents.on(
            store.fleet.worktrees.filter { ($0.host ?? "") == host },
            state: client.state, build: client.daemonBuild)
    }

    /// Why a selected board can't be drawn, said only as far as this app
    /// knows it: "gone" only from a runner that is answering and has listed
    /// its projects without it.
    private func missingBoardSentence(host: String) -> String {
        guard let client = store.clients[host] else {
            return "The runner this board was on isn’t in Far Cooler anymore."
        }
        guard client.state == .connected, client.repositoriesListed else {
            return "Far Cooler is still loading this runner’s repositories."
        }
        return "This board isn’t on its runner anymore. Choose another board in the sidebar."
    }

    /// Go to a pane a card offered, as the fleet has it now. See
    /// `BoardPane.landing`: its worktree when the pane has gone, and the
    /// board with a sentence when the worktree has too.
    private func go(to pane: BoardPane) {
        guard let landed = BoardPane.landing(for: pane, in: store.fleet) else {
            errorBanner = "That agent has closed, and its worktree is gone."
            return
        }
        expanded.insert(pane.worktree.id)
        selection = landed
    }

    /// One row of the sidebar, drawn from its entry.
    ///
    /// Lifted out of `sidebar` when projects became collapsible: the rows had to
    /// go behind an `if`, and wrapping fifty lines of view builder in one would
    /// have re-indented the whole block to say one thing. A builder method is
    /// what this file already does for the detail side — see `tiled(_:group:)`.
    private func sidebarRow(_ entry: SidebarEntry) -> some View {
        // One gutter in per level, and only two levels: see
        // `SidebarEntry.depth`.
        sidebarRowContent(entry)
            .padding(.leading, SidebarGrid.indent(entry.depth))
    }

    @ViewBuilder
    private func sidebarRowContent(_ entry: SidebarEntry) -> some View {
        let key = entry.collapseKey
        let usable = store.refusal(for: entry.host) == nil
        switch entry.kind {
        case .repository:
            projectHeader(entry)
        // Everything under the header is what a collapsed project hides.
        case _ where preferences.isProjectCollapsed(key):
            EmptyView()
        case .workspace(let name):
            if let workspace = entry.workspace {
                WorkspaceRow(
                    name: name, workspace: workspace.id, taskPrefix: workspace.taskPrefix,
                    seat: entry.orchestrator, implicit: workspace.isImplicit,
                    count: WorkspaceCounts.count(for: workspace, host: entry.host, in: store.needsYou),
                    unread: ConversationColumn.unread(entry.orchestrator),
                    isSelected: selection?.host == entry.host && selection?.workspace == workspace.id
                        && Self.highlightsWorkspace(selection?.focus),
                    onSelect: { selection = .workspace(host: entry.host, workspace: workspace.id, focus: nil) },
                    actions: usable ? workspaceActions(entry) : nil,
                    isOpen: isOpen(SidebarEntry.openKey(host: entry.host, workspace: workspace.id)),
                    onToggle: { toggleWorktrees(SidebarEntry.openKey(host: entry.host, workspace: workspace.id)) },
                    onNewWorktree: usable ? newWorktreeAction(for: entry) : nil)
            }
        case .worktree:
            if let worktree = entry.worktree { worktreeRow(worktree, usable: usable) }
        case .noWorktrees:
            NoWorktreesRow()
        case .unclaimed:
            UnclaimedWorktrees(
                worktrees: entry.worktrees,
                // Open while the selection is inside it, too: a worktree
                // chosen from the palette or the attention cycle has to be
                // somewhere you can see.
                isExpanded: unclaimedExpanded.contains(entry.group)
                    || entry.worktrees.contains { $0.id == currentWorktree?.id },
                onToggle: {
                    if unclaimedExpanded.contains(entry.group) {
                        unclaimedExpanded.remove(entry.group)
                    } else {
                        unclaimedExpanded.insert(entry.group)
                    }
                },
                // One step in from the group's header, as a workspace's
                // worktrees are from its row.
                row: { worktree in
                    worktreeRow(worktree, usable: usable).padding(.leading, SidebarGrid.gutter)
                })
        case .hidden:
            HiddenWorktrees(
                project: key,
                worktrees: entry.worktrees,
                isExpanded: hiddenExpanded.contains(key),
                onToggle: {
                    if hiddenExpanded.contains(key) {
                        hiddenExpanded.remove(key)
                    } else {
                        hiddenExpanded.insert(key)
                    }
                },
                onUnhide: { ws in
                    Task { await act(on: ws) { c in await c.unhideWorktree(ws.short) } }
                }
            )
        }
    }

    /// A repository's header — or a silent runner's, which names the runner.
    private func projectHeader(_ group: SidebarEntry) -> some View {
        let key = group.collapseKey
        // A silent host's placeholder has no project of its own to name or add
        // into — the header names the runner instead, and there is nothing yet
        // to route a `+` to.
        let isSilentHost = group.project.isEmpty
        return ProjectHeader(
            name: isSilentHost ? (group.host.isEmpty ? "This Mac" : group.host) : group.project,
            count: group.worktrees.count,
            onNewWorktree: isSilentHost
                ? nil : { newWorktree(host: group.host, project: group.project) },
            onNewTerminal: isSilentHost
                ? nil
                : {
                    startMainTerminal(
                        host: group.host, repositoryID: group.repositoryID, project: group.project)
                },
            onRemove: isSilentHost
                ? nil
                : {
                    guard
                        let repo = repository(
                            host: group.host, id: group.repositoryID, project: group.project)
                    else { return }
                    removeRepository = RepositoryToRemove(host: group.host, repository: repo)
                },
            host: group.host,
            hostState: store.state(of: group.host),
            daemonUpdate: daemonUpdate(for: group.host),
            showHost: isSilentHost ? false : showHosts,
            onReconnect: { store.reconnect(group.host) },
            isCollapsed: preferences.isProjectCollapsed(key),
            // A silent host's header has no worktrees under it, so there is
            // nothing for a chevron to do.
            onToggleCollapse: isSilentHost ? nil : { preferences.toggleProject(key) }
        )
    }

    private var sidebar: some View {
        VStack(spacing: 0) {
            sidebarHeader
            searchField

            // Gated on the rows, not the raw merged worktree count: a
            // runner that has never connected contributes no worktrees but
            // still gets a `silentHosts` header in them (unless it's the
            // local runner, which has its own placeholder below). Gating on
            // `worktrees.isEmpty` instead used to short-circuit straight to
            // the LOCAL runner's empty state whenever the merged fleet had
            // no rows — even with a remote runner configured and its header
            // sitting right below — making that remote runner vanish from
            // the sidebar entirely rather than showing as unreachable.
            // `query.isEmpty` keeps this from swallowing a plain "no search
            // results" into the same screen.
            let entries = sidebarEntries
            if entries.isEmpty && query.isEmpty {
                fleetPlaceholder
                Spacer(minLength: 0)
            } else if entries.isEmpty {
                VStack(spacing: 6) {
                    Text("Nothing matches").font(.callout.weight(.medium))
                    Text("\u{201c}\(query)\u{201d}")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 32)
                Spacer(minLength: 0)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        NeedsYouRow(
                            count: store.needsYou.count, isSelected: selection == .needsYou,
                            onSelect: { selection = .needsYou })
                            .padding(.bottom, 6)
                        ForEach(entries) { entry in sidebarRow(entry) }
                    }
                    .padding(.bottom, 10)
                }
            }

            statusBar
        }
        .background(WorkspaceStyle.sidebar)
        // Once, on the first launch after workspaces became places.
        .overlay(alignment: .bottom) {
            if showWorkspacesTip {
                WorkspacesTipView {
                    WorkspacesTip.dismiss()
                    showWorkspacesTip = false
                }
                .padding(.bottom, 28)
                .transition(.opacity)
            }
        }
        // A finished drag, published by the row that took the drop. The row
        // says only that a card landed on another card's top or bottom edge;
        // what that MEANS needs the whole project group, which is here.
        .onReceive(WorktreeDrag.shared.$completion.compactMap { $0 }) { landed($0) }
        // The rule a row asks while a card hovers over it. Reads the store
        // it was handed, which is a reference, so it answers from the fleet
        // as it is at the moment of asking, not as it was here.
        .onAppear {
            let store = store
            WorktreeDrag.shared.accepts = { dragged, target in
                Self.dropMeaning(of: dragged, onto: target, in: store.fleet, assigns: Self.assigns(store)) != nil
            }
            WorktreeDrag.shared.endStaleDragsOnMouseDown()
        }
        // Declared in exactly one place. A second declaration on the
        // `NavigationSplitView`'s sidebar closure made which width the column
        // settled on nondeterministic, and the window drifted with it.
        .navigationSplitViewColumnWidth(min: 220, ideal: 248, max: 360)
    }

    /// Search, because worktrees are unbounded.
    ///
    /// It matches terminals too, so typing an agent's name finds the worktree
    /// containing it — which is how you reach an agent on another runner
    /// without going looking for the runner.
    private var searchField: some View {
        SidebarRow {
            HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            TextField("Find a workspace, task or agent", text: $query)
                .textFieldStyle(.plain)
                .font(.system(size: 12.5))
                .focused($searchFocused)
                // Esc clears the search, and on an empty one leaves the
                // field (checklist F3). The window's Esc monitor passes Esc
                // to a text field, so this is the one place it's heard.
                .onExitCommand {
                    let next = SearchEscape.after(query: query)
                    query = next.query
                    if !next.keepsFocus { searchFocused = false }
                }
            if !query.isEmpty {
                Button { query = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
            }
        }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: 7).fill(Color.primary.opacity(0.055)))
            // The field is hand-rolled — a `.plain` `TextField` in a styled box
            // — so it gets no focus ring from AppKit, and `searchFocused` was
            // bound and then read by nobody. ⌘F moved the keyboard into this
            // field and changed not one pixel, which is indistinguishable from
            // a shortcut that did nothing.
            .overlay(
                RoundedRectangle(cornerRadius: 7)
                    .strokeBorder(
                        Color.accentColor.opacity(searchFocused ? 0.8 : 0),
                        lineWidth: 2)
            )
            .animation(Motion.snap, value: searchFocused)

        }
        .padding(.bottom, 6)
    }

    private var sidebarHeader: some View {
        SidebarRow {
            HStack(spacing: 8) {
            // Just the word, now. It used to be the one runner being driven,
            // because the sidebar was that runner's worktrees and the pane
            // could not otherwise say whose. Now it is every runner's at
            // once, and each row already names its own runner below, so a
            // header naming one would be naming the wrong thing, or picking
            // a favorite among rows that are not ranked.
            Text("Fleet")
                .font(.system(size: 15, weight: .semibold))
            if store.clients.values.contains(where: \.busy) { ProgressView().controlSize(.mini) }

            Spacer()

            // A menu rather than a button, because "add a repository" has to be
            // reachable at all times. It used to live only in the empty state,
            // so once you had one worktree there was no way to add a second
            // repository without dropping to the terminal.
            SidebarMenuButton(
                systemImage: "plus",
                help: "Add a worktree, a repository, or a runner",
                items: [
                    SidebarMenuItem(title: "New Worktree…") {
                        newWorktreeIntent = NewWorktreeIntent()
                    },
                    SidebarMenuItem(title: "Add Repository…") { showAddRepository = true },
                    // Here as well as in the picker, because this is the menu
                    // people open looking for "add a thing" — and a runner is
                    // a thing you add.
                    // Straight to the thing, rather than to the tab that
                    // contains a field that does it. This opened Settings on
                    // the Runners tab and left you to find the text field at
                    // the bottom of a list of existing runners — and it offered
                    // only the typing road, when the shorter one is to scan a
                    // code from a device that already knows the address, the
                    // user, the port and the host key.
                    SidebarMenuItem(title: "Add…") { showAdd = true },
                ])
            }
        }
        .padding(.top, 12)
        .padding(.bottom, 8)
    }

    /// Shown only once a fleet has actually been read.
    ///
    /// Telling someone they have no worktrees when the truth is that we could
    /// not read them is worse than saying nothing, because it sends them to
    /// create one they already have.
    ///
    /// Reads the LOCAL runner's own load state, not a merged one across every
    /// configured runner: this Mac is always present (`FleetStore.hosts` puts
    /// it first), and it is the one runner whose failure to answer is worth a
    /// dedicated screen here rather than a row in the sidebar saying so — which
    /// is what an unreachable REMOTE runner gets instead, so its own trouble
    /// does not blank out a sidebar the local runner is perfectly able to show.
    @ViewBuilder
    private var fleetPlaceholder: some View {
        let local = store.clients[""]
        if local?.hasLoaded == true {
            emptyFleet
        } else if let error = local?.lastError {
            VStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle")
                    .font(.system(size: 26))
                    .foregroundStyle(.orange)
                Text("Could not read the fleet").font(.callout.weight(.medium))
                // The heading, then a sentence, then the box — the shape
                // `ChangesPane` settled on for this exact string, and the
                // wording it uses, because it is the same event: the command
                // that reads something came back non-zero.
                //
                // `lastError` is `farcooler`'s stderr. It used to be this
                // caption, centered under a heading the app wrote, in the
                // app's own face — so ssh's words read as Far Cooler's
                // account of the runner. Nothing is dropped moving it: when
                // this Mac's own daemon will not answer, those words are the
                // only diagnosis anyone has. No cause is named above them
                // either, because from here it is unknowable and a guess
                // sends somebody to fix the wrong thing — see
                // `Enrollment.note(about:outcome:)`.
                Text("The command that reads it didn’t finish.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                DetailBox(text: error)
                    .frame(maxWidth: 420)
                Button("Try again") {
                    Task { await local?.refresh() }
                }
                .padding(.top, 4)
            }
            .padding(.horizontal, 26)
            .padding(.vertical, 34)
        } else {
            ProgressView()
                .controlSize(.small)
                .padding(.vertical, 40)
        }
    }

    private var emptyFleet: some View {
        VStack(spacing: 10) {
            Image(systemName: "rectangle.stack")
                .font(.system(size: 26))
                .foregroundStyle(.tertiary)
            Text("No worktrees").font(.callout.weight(.medium))
            Text("A worktree is a directory and branch of its own.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            if store.repositories.isEmpty {
                Text("Add a repository to get started.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .multilineTextAlignment(.center)
                Button("Add Repository…") { showAddRepository = true }.padding(.top, 4)
            } else {
                Button("New Worktree…") {
                    newWorktreeIntent = NewWorktreeIntent()
                }.padding(.top, 4)
            }
        }
        .padding(.horizontal, 26)
        .padding(.vertical, 34)
    }

    /// Whether the daemon starts at login.
    ///
    /// Lives in Settings now. It is a preference you set once, not a status you
    /// watch, and a permanent control in the status bar made a piece of
    /// configuration look like live information.
    @ViewBuilder

    private var statusBar: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 7) {
                // Red, not amber, when the runtime is down. Amber means one
                // thing in this app and in both phones — an agent is waiting
                // on you — and nobody is waiting here: tmux is the runtime
                // every pane on every runner lives inside, and a fleet that
                // cannot reach it cannot show you a single one of them. That
                // is a failure rather than a request. iOS settled this in
                // `7e4a4f7` and Android followed in `8481657`, which left this
                // dot as the last place the three apps disagreed about it.
                //
                // Neutral, not green, when it is healthy — the other half of
                // the same line, and `8741757` recorded it here rather than
                // changing it inside an amber sweep. Three arguments, and they
                // agree. Green in this palette is `Status.done` and nothing
                // else: `StatusGlyph` says so outright, `150eb0f` took it back
                // off `WorktreeDot`, and a permanent green at the foot of the
                // window is the one place a person looks for finished agents.
                // `HostDot` states the principle a few hundred lines away —
                // "a dot that is always there is a dot nobody reads, and the
                // whole point is that you notice it only when something is
                // wrong" — and draws `EmptyView()` for `.connected` on that
                // reasoning; this dot cannot vanish, because it is the mark the
                // "tmux unavailable" sentence needs, so neutral is the same
                // argument at the only volume available. And both phones
                // already moved: iOS in `8481657`, Android's `Dns` tint to
                // `onSurfaceVariant`, so the Mac was again the last surface out
                // of step on a line whose other half it had just fixed.
                //
                // The audit that prompted this had it backwards, which is worth
                // recording because it is the second time: item 5 reads "iOS
                // draws a green dot for a healthy connection where the other
                // two draw nothing — Keep the Mac's." The Mac drew green too,
                // and had drawn it longer. Same shape as `150eb0f`, where the
                // Mac was credited with avoiding the exact bug it had.
                //
                // "N live" beside it already says the fleet is alive, and the
                // per-runner dots to the right still name anyone who isn't.
                //
                // Red only for `runtimeDown`. With no runner connected the bar
                // says so in neutral words rather than "tmux unavailable" in
                // red: a runner between reconnection attempts is not known to
                // have lost anything. See `FleetStore.Reading`.
                Circle()
                    .fill(store.reading.isTrouble ? Color.red : Color.secondary)
                    .frame(width: 7, height: 7)
                Text(store.reading.sentence)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                // The dot above is an OR across every runner, deliberately —
                // ANDing would turn the bar red every time one laptop was
                // merely asleep. But on a fleet of more than one, an OR alone
                // means it takes just ONE healthy runner to keep that dot
                // quiet while a second sits there unreachable or without
                // tmux at all, invisibly. These name that runner instead of
                // letting the merged dot speak for it; a click retries it at
                // once, same as a header's own dot.
                //
                // This read "orange" and "green" for those two states, which
                // it did until `8741757` and the line above respectively.
                // `FleetStore.unhealthyHosts` records both moves and why
                // neither touches the argument.
                //
                // Built by hand here rather than calling `HostDot`, and
                // deliberately so, not as an oversight: `HostDot` draws
                // `EmptyView()` for `.connected`, which is right for a
                // header naming only whether the RUNNER answers, but wrong
                // here — a runner can be fully reachable and still be the
                // reason this row reads red (reachable, no tmux), and
                // that case has to draw a dot. See `troubleColor(for:)`
                // below for the resulting, deliberately different, palette.
                if showHosts && !store.unhealthyHosts.isEmpty {
                    ForEach(store.unhealthyHosts, id: \.self) { host in
                        Button {
                            store.reconnect(host)
                        } label: {
                            Circle()
                                .fill(troubleColor(for: host))
                                .frame(width: 6, height: 6)
                        }
                        .buttonStyle(.plain)
                        .help(
                            "\(host.isEmpty ? "this Mac" : host): \(troubleReason(for: host)) — click to retry"
                        )
                    }
                }

                // A runner running a daemon that is not this app's build.
                //
                // Beside the trouble dots and not among them: those say a
                // runner cannot do its job, this says it is doing its job as a
                // different program from the one this app was built against —
                // and unlike them, a click here must not act, it must ask. See
                // `FleetStore.staleHosts` for why the two lists stay separate,
                // and `DaemonUpdateBar` for why this one is not hidden on a
                // fleet of one the way `showHosts` hides the rest.
                if !store.staleHosts.isEmpty {
                    DaemonUpdateBar(targets: store.staleHosts.compactMap(daemonUpdate(for:)))
                }

                Spacer()

                Button {
                    Task {
                        for client in store.clients.values {
                            await client.refresh()
                            // Roots and layouts too, not only repositories —
                            // a reconnection now re-seeds all three on its
                            // own (see `DaemonClient.onReconnect`), and this
                            // button asking for less than that would be a
                            // step backwards from what happens automatically.
                            await client.refreshRepositories()
                            await client.refreshRoots()
                            await client.refreshLayouts()
                        }
                    }
                } label: {
                    Image(systemName: "arrow.clockwise").font(.system(size: 11))
                }
                .buttonStyle(.borderless)
                .help("Refresh")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
        }
    }

    /// A trouble dot's color, for a runner `store.unhealthyHosts` has
    /// already decided is not fully well.
    ///
    /// Not `HostDot`'s palette reused outright: `HostDot` renders nothing at
    /// all for `.connected`, which is right for a header naming only whether
    /// the RUNNER is reachable — but a runner can be perfectly reachable
    /// and still be the reason the fleet's tmux status reads red, and that
    /// case has to draw something here. ("Orange" here until `8741757`, which
    /// is the commit that made it red.)
    private func troubleColor(for host: String) -> Color {
        switch store.state(of: host) {
        case .unreachable: return .red
        // Reachable, and its tmux is not answering — the one case `HostDot`
        // has nothing to say about, and the reason this function exists. The
        // same event as the bar's own dot going red a few lines up, so it
        // wears the same color: a runner whose every pane is unreadable has
        // failed at the only job it has here.
        case .connected: return .red
        case .notInstalled: return .secondary
        // Neutral rather than amber. Amber would say an agent is waiting on
        // you; what is waiting is a socket. `Status.tint(_:)` paints `working`
        // and `starting` in `GlancePalette.ink2` for exactly that reason, and a
        // connection coming back up is the fleet-level version of the same
        // sentence. `.connecting` cannot actually reach this — the list this
        // colors leaves out a runner nothing is yet known about, see
        // `FleetStore.unhealthyHosts` — and is named beside `.reconnecting`
        // because they are one situation and the switch is exhaustive.
        case .connecting, .reconnecting: return .secondary
        }
    }

    private func troubleReason(for host: String) -> String {
        switch store.state(of: host) {
        case .unreachable(let why): return why
        case .notInstalled: return "Far Cooler is not installed here"
        case .reconnecting: return "reconnecting"
        case .connecting: return "connecting"
        case .connected: return "tmux is not available here"
        }
    }

    // MARK: - Detail

    /// One worktree's sidebar row.
    ///
    /// Extracted from the sidebar builder rather than written inline. With
    /// eighteen arguments, most of them closures, this call sits right at the
    /// type checker's budget — adding one more parameter to it pushed the whole
    /// enclosing expression past "unable to type-check in reasonable time",
    /// which is a compile error with no line number worth reading.
    private func worktreeRow(_ ws: Worktree, usable: Bool) -> some View {
        let client = store.client(for: ws)
        // The runner's worktree, not the row's `ws`: the row's has the
        // orchestrators drawn in rows of their own taken out.
        let listed = worktree(host: ws.host ?? "", id: ws.id) ?? ws
        let tiled = Set(Self.shownLayout(client?.layouts[ws.id], of: listed)?.terminals ?? [])
        return WorktreeSection(
            worktree: ws,
            isExpanded: expanded.contains(ws.id),
            selected: Self.selected(in: ws, by: selection),
            onSelect: { terminal in open(ws, terminal: terminal) },
            onToggle: { toggle(ws.id) },
            onNewTerminal: { newTerminal(in: ws) },
            onHide: {
                Task { await act(on: ws) { c in await c.hideWorktree(ws.short) } }
            },
            onUnhide: {
                Task { await act(on: ws) { c in await c.unhideWorktree(ws.short) } }
            },
            onRemove: { removeWorktree = ws },
            onTerminalAction: { term, action in
                Task { await run(action, on: term, in: ws) }
            },
            // Never an orchestrator's window: nothing is put beside it.
            layouts: Self.ownLayouts(client?.layouts[ws.id] ?? [], of: listed),
            onMoveToLayout: { term, group in
                moveToLayout(term, in: ws, group: group)
            },
            onDropTogether: { dragged, onto in
                placePane(dragged, onto: onto.id, side: .right, in: ws)
            },
            tiled: tiled,
            onEditorError: { editorError = $0 },
            usable: usable,
            reorderable: WorktreeDrag.offersDrag(usable: usable, runner: client?.daemonBuild),
            moveTargets: Self.moveTargets(for: listed, in: store.fleet, assigns: Self.assigns(store)(listed)),
            onMove: { target in move(listed, to: target) },
            roleOffer: roleOffer(in: listed),
            onShowChanges: showChangesAction(for: listed, usable: usable),
            changes: changesStatus(ws),
            countsWidth: countsWidth
        )
    }

    /// `OrchestratorAdoption.offer` in `worktree`, as the fleet has it when
    /// the menu opens. Hoisted for `worktreeRow`'s type checker, as
    /// `changesStatus` is.
    private func roleOffer(in worktree: Worktree) -> (Terminal) -> OrchestratorAdoption.Offer? {
        let store = store
        return { OrchestratorAdoption.offer(for: $0, in: worktree, host: worktree.host ?? "", fleet: store.fleet) }
    }

    /// The diff column's width, for every row in the sidebar at once.
    ///
    /// Measured across the whole fleet rather than per row, because a column
    /// each row sizes for itself is not a column — that was the alignment bug.
    /// Computed here rather than inside the row for the same reason it is
    /// measured at all: every row has to agree, and only this level can see
    /// them all.
    private var countsWidth: CGFloat {
        SidebarMetrics.countsWidth(
            store.clients.values.flatMap { Array($0.changesInbox.values) })
    }

    /// Diff status for one worktree, or nil when the fleet inbox has not been
    /// read yet. Hoisted out of the sidebar builder: inline, the chained
    /// optional subscript pushed that expression past the type checker's budget.
    private func changesStatus(_ ws: Worktree) -> InboxRow? {
        guard let client = store.client(for: ws) else { return nil }
        return client.changesInbox[ws.short]
    }

    /// This worktree's changes pane, if it has one open in the layout the
    /// detail draws for it, drilled into or loose.
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

    /// The layout the detail draws for `ws` opened whole: drilled into in a
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
            Task { await act(on: ws) { c in await c.stop(terminal: open.short) } }
            return
        }
        showChanges(in: ws)
    }

    /// Show Changes: the toolbar's, a worktree row's menus' and its card's
    /// (ov-78). Reachable whether or not the worktree has a terminal open.
    ///
    /// With the worktree's layout drilled into, a split of its
    /// focused pane, exactly as `⌃B %` and a drop on an edge are: the daemon
    /// has one verb for "a new pane, here, running this", and a changes pane
    /// is that verb with a different preset. With no layout there (no
    /// terminal, or the main checkout named from the Orchestrator column),
    /// its changes pane, one it has or a new one in a window of its own,
    /// opened, drilled into. Never a split of the orchestrator's
    /// window.
    private func showChanges(in ws: Worktree) {
        if let open = changesPane(in: ws) {
            focus(PaneRef(host: ws.host ?? "", worktree: ws.id, terminal: open.id))
            return
        }
        if let layout = worktreeColumn(of: ws) {
            Task {
                let groups = await act(on: ws, default: []) { c in
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
                    on: listed, default: nil as Terminal?,
                    { c in await c.createTerminal(in: listed, preset: "changes", title: "Changes") })
            }
            guard let pane else { return }
            expanded.insert(listed.id)
            if stays, case .workspace(host, let id, _)? = selection {
                selection = .workspace(host: host, workspace: id, focus: .worktree(listed.id, terminal: pane.id))
            } else {
                selection = Self.opening(listed, terminal: pane.id, in: store.fleet)
            }
            keyPane = PaneRef(host: host, worktree: listed.id, terminal: pane.id)
        }
    }

    /// Show Changes on `ws`'s row, or nil where it can't be: a runner that
    /// can't be acted on, or that has said it can't read changes. Hoisted
    /// for `worktreeRow`'s type checker.
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
    private func boardStore(for workspace: WorkspaceSummary, client: DaemonClient, host: String)
        -> TaskBoardStore
    {
        let key = "\(host)/\(workspace.id)"
        if let existing = boardStores[key], Self.keeps(existing, for: workspace, client: client) {
            if existing.onChoose == nil {
                existing.onChoose = { row in openTask(row.id, host: host, workspace: workspace.id) }
            }
            return existing
        }
        let made = TaskBoardStore(client: client, workspace: workspace)
        made.onChoose = { row in openTask(row.id, host: host, workspace: workspace.id) }
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
    /// The workspace of whatever the sidebar is showing — Main's for a
    /// worktree in Unclaimed — and the only repository's Main when nothing is
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
    private func shownLayouts(for selection: Selection?) -> [ShownLayout] {
        let all = WorkspaceScreen.shown(
            selection, in: store.fleet,
            layouts: { host, worktree in store.clients[host]?.layouts[worktree] },
            repositories: { host in store.clients[host]?.repositories.map(\.id) ?? [] },
            chosen: { chosenAgents[$0] })
        guard case .workspace(let host, let id, let focus)? = selection else { return all }
        let summary = WorkspaceScreen.workspace(
            id, host: host, in: store.fleet, repositories: store.clients[host]?.repositories.map(\.id) ?? [])
        let arrangement = detailWidth.map { width in
            WorkspaceColumns.layout(
                width: width, drilled: focus != nil, cell: TerminalMetrics.cell(preferences.terminalFont()).width,
                hasConversation: summary.map { !$0.isImplicit } ?? false, focused: focusColumn,
                peek: orchestratorPeek)
        }
        return WorkspaceScreen.visible(all, arrangement: arrangement, pick: workspacePicks["\(host)|\(id)"] ?? .orchestrator)
    }

    /// What the detail draws now.
    private var shown: [ShownLayout] { shownLayouts(for: selection) }

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

        case .looseWorktree(let host, let id, _):
            if let front = shown.last {
                tiled(front)
            } else if let ws = worktree(host: host, id: id) {
                worktreeDetail(ws)
            } else {
                placeholder
            }

        case .workspace(let host, let id, let focus):
            workspaceDetail(host: host, id: id, focus: focus)

        case nil:
            placeholder
        }
    }

    /// A workspace: its conversation and its board, or, drilled in, the
    /// task or worktree opened, under the breadcrumb back.
    private func workspaceDetail(host: String, id: String, focus: Focus?) -> some View {
        let summary = WorkspaceScreen.workspace(
            id, host: host, in: store.fleet, repositories: store.clients[host]?.repositories.map(\.id) ?? [])
        let layouts = shown
        let key = "\(host)|\(id)"
        return WorkspaceView(
            drilled: focus != nil,
            hasConversation: summary.map { !$0.isImplicit } ?? false,
            cell: TerminalMetrics.cell(preferences.terminalFont()).width,
            focused: focusColumn,
            peek: orchestratorPeek,
            pick: Binding(
                get: { workspacePicks[key] ?? .orchestrator }, set: { workspacePicks[key] = $0 }),
            conversation: {
                conversationColumn(host: host, workspace: summary, shown: layouts.first { $0.column == .conversation })
            },
            rail: { conversationRail(host: host, workspace: summary) },
            board: { boardColumn(host: host, id: id) },
            opened: {
                VStack(spacing: 0) {
                    DrillBreadcrumb(
                        crumbs: crumbs(host: host, workspace: summary),
                        onGo: { target in
                            if target == trail { trail = nil }
                            selection = target
                        },
                        onBack: { goBack(unfocusFirst: false) })
                    Divider()
                    openedView(host: host, focus: focus, shown: layouts.last { $0.column != .conversation })
                }
            }
        )
        .animation(.snappy(duration: 0.2), value: orchestratorPeek)
        .onPreferenceChange(WorkspaceWidthPreference.self) { width in
            MainActor.assumeIsolated {
                if let width, width != detailWidth { detailWidth = width }
            }
        }
        .modifier(WindowTitle(title: workspaceTitle(host: host, workspace: summary, focus: focus).title,
                              subtitle: workspaceTitle(host: host, workspace: summary, focus: focus).subtitle))
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
        if case .task(let id)? = focus, let row = taskRow(host: host, workspace: workspace, id: id) {
            let under = workspace?.isImplicit == true ? [repository] : [name, repository]
            return ("\(row.key) \(row.title)", under.filter { !$0.isEmpty }.joined(separator: " · "))
        }
        return (name, [repository, host].filter { !$0.isEmpty }.joined(separator: " · "))
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
    private func conversationColumn(host: String, workspace: WorkspaceSummary?, shown: ShownLayout?) -> some View {
        if let workspace {
            let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet)
            let key = "\(host)|\(workspace.id)"
            let canAct = store.refusal(for: host) == nil
            VStack(spacing: 0) {
                ConversationHeader(
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
                    })
                Divider()
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
                TimelineView(.periodic(from: .now, by: 5)) { context in
                    let state = ConversationColumn.state(
                        seat: seat, isStarting: startingOrchestrators.isStarting(workspace, host: host),
                        startedAt: orchestratorStartedAt[key], now: context.date)
                    conversationBody(
                        state: state, offers: ConversationColumn.offers(state, canAct: canAct),
                        shown: shown, seat: seat, workspace: workspace, host: host)
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
                Label("This workspace isn’t here", systemImage: "square.stack.3d.up")
            } description: {
                Text(missingBoardSentence(host: host))
            }
        }
    }

    @ViewBuilder
    private func conversationBody(
        state: ConversationColumn.State, offers: [ConversationColumn.Offer], shown: ShownLayout?,
        seat: BoardPane?, workspace: WorkspaceSummary, host: String
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
                tiled(shown, titled: false)
            } else if let seat {
                bareTerminal(seat)
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

    /// The conversation shrunk to a rail beside a task or a worktree: its
    /// status and its dot, and a click that pops it open over what's opened,
    /// or closes it again.
    private func conversationRail(host: String, workspace: WorkspaceSummary?) -> some View {
        let seat = workspace.flatMap { WorkspaceScreen.orchestrator(of: $0, host: host, in: store.fleet) }
        return Button {
            if orchestratorPeek { orchestratorPeek = false } else { focusWorkspaceColumn(.focusConversation) }
        } label: {
            VStack {
                if let seat {
                    StatusGlyph(status: seat.terminal.status)
                } else {
                    Image(systemName: "circle.dashed").foregroundStyle(.tertiary)
                }
                // Its needs-you dot: something in this workspace is waiting,
                // or the orchestrator finished a turn nobody has seen.
                if let workspace,
                    store.needsYou.count(in: workspace.id) > 0 || ConversationColumn.unread(seat)
                {
                    Circle().fill(GlancePalette.amber(colorScheme)).frame(width: 6, height: 6)
                }
                Spacer()
                Image(systemName: orchestratorPeek ? "chevron.left" : "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.bottom, 12)
            }
            .padding(.top, 12)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(orchestratorPeek ? Color.primary.opacity(0.06) : WorkspaceStyle.canvas)
        .help(orchestratorPeek ? "Hide the orchestrator" : "Show the orchestrator (⌥⌘1)")
        .accessibilityLabel(orchestratorPeek ? "Hide the Orchestrator" : "Show the Orchestrator")
    }

    /// The breadcrumb over a task or a worktree opened: Workspace › Task, or
    /// Workspace › Task › Worktree, or Workspace › Worktree.
    private func crumbs(host: String, workspace: WorkspaceSummary?) -> [WorkspaceNavigation.Crumb] {
        let repository = workspace.flatMap { w in
            store.clients[host]?.repositories.first { $0.id == (w.repository ?? w.id) }?.displayName
        } ?? ""
        return WorkspaceNavigation.crumbs(
            selection, trail: trail,
            workspace: workspace.map { $0.isImplicit ? repository : $0.name } ?? "Workspace",
            task: { id in
                taskRow(host: host, workspace: workspace, id: id).map { "\($0.key) \($0.title)" } ?? "Task"
            },
            worktree: { id in worktree(host: host, id: id)?.task ?? "Worktree" })
    }

    /// What's drilled into: a task, or a worktree opened whole.
    @ViewBuilder
    private func openedView(host: String, focus: Focus?, shown: ShownLayout?) -> some View {
        switch focus {
        case .task(let id)?:
            taskView(host: host, id: id, shown: shown)
        case .worktree(let wt, _)?:
            if let shown {
                tiled(shown, titled: false)
            } else if let ws = worktree(host: host, id: wt) {
                worktreeDetail(ws)
            } else {
                ContentUnavailableView("This worktree isn’t here anymore", systemImage: "folder")
            }
        case nil:
            EmptyView()
        }
    }

    /// Back (⌃⌘←, Esc, the breadcrumb's chevron): up one level, along the
    /// breadcrumb to a task a worktree was opened from, else to the
    /// workspace.
    ///
    /// Esc and ⌃⌘← close the popped-open orchestrator, then leave Focus,
    /// first; the breadcrumb's chevron (`unfocusFirst` false) goes up as it
    /// does anywhere else.
    private func goBack(unfocusFirst: Bool = true) {
        if orchestratorPeek {
            orchestratorPeek = false
            if unfocusFirst { return }
        }
        if focusColumn {
            focusColumn = false
            if unfocusFirst { return }
        }
        guard let back = WorkspaceNavigation.back(from: selection, trail: trail) else { return }
        if back == trail { trail = nil }
        selection = back
    }

    /// A task, drilled into (spec §4.4): its text first, whole, and its
    /// agent and changes beneath.
    @ViewBuilder
    private func taskView(host: String, id: String, shown: ShownLayout?) -> some View {
        let summary = selection?.workspace.flatMap {
            WorkspaceScreen.workspace($0, host: host, in: store.fleet, repositories: store.clients[host]?.repositories.map(\.id) ?? [])
        }
        if let client = store.clients[host], let summary {
            let board = boardStore(for: summary, client: client, host: host)
            if let row = board.board.columns.flatMap(\.rows).first(where: { $0.id == id }) {
                taskView(row: row, board: board, client: client, host: host, shown: shown)
            } else if !board.hasRead {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .task(id: ObjectIdentifier(board)) { await board.readIfNeverRead() }
            } else {
                ContentUnavailableView {
                    Label("This task isn’t on the board anymore", systemImage: "checklist")
                } actions: {
                    Button("Back to the Board") { goBack(unfocusFirst: false) }
                }
            }
        } else {
            ContentUnavailableView("This task isn’t here", systemImage: "checklist")
        }
    }

    private func taskView(
        row: TaskRow, board: TaskBoardStore, client: DaemonClient, host: String, shown: ShownLayout?
    ) -> some View {
        let agents = WorkspaceScreen.agents(of: row.id, host: host, in: store.fleet)
        let chosen = WorkspaceScreen.agent(of: row.id, host: host, in: store.fleet, chosen: chosenAgents[row.id])
        let worktreeID = TaskColumnModel.worktree(of: row, agent: chosen)
        let lane = worktreeID.flatMap { worktree(host: host, id: $0) }
        let agent = TaskColumnModel.agent(hasAgent: chosen != nil, worktree: lane?.id)
        let showsChanges = lane != nil && client.changesSupported != false
        let work = TaskColumnModel.work(agent, showsChanges: showsChanges)
        let openWorktree = {
            guard let lane, let current = selection else { return }
            let opened = WorkspaceNavigation.openWorktree(lane.id, from: current)
            trail = opened.trail
            trailWorktree = lane.id
            selection = opened.next
        }
        return VStack(spacing: 0) {
            TaskViewHeader(row: row, store: board)
            Divider()
            TaskViewSplit(work: work, focused: focusColumn) {
                ScrollView {
                    TaskColumnCard(row: row, store: board)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                }
                .background(WorkspaceStyle.document)
            } workArea: {
                VStack(spacing: 0) {
                    TaskWorkHeader(
                        agent: agent, worktree: lane.map { WorkspaceScreen.ownTerminals(of: $0, fleet: store.fleet) },
                        agents: agents, chosen: chosen,
                        onChooseAgent: { pane in chosenAgents[row.id] = pane.terminal.id },
                        onOpenWorktree: openWorktree)
                    if work == .full {
                        Divider()
                        TaskColumnSplit(
                            status: row.status, showsChanges: showsChanges, showsAgent: chosen != nil,
                            agent: {
                                if let shown {
                                    tiled(shown, titled: false)
                                } else if let chosen {
                                    // Working the task, and in no layout read yet.
                                    bareTerminal(chosen)
                                }
                            },
                            changes: {
                                if let lane {
                                    TaskColumnChanges(
                                        changes: changesStore(for: lane, client: client),
                                        isFocused: changesFocus == row.id,
                                        agents: lane.reviewAgentTargets(), onFocus: { changesFocus = row.id })
                                }
                            })
                    }
                }
            }
        }
        // The card's record and question, read for the task on screen.
        .task(id: row.id) { await board.open(row) }
    }

    /// Move to Its Own Window: `terminal`, sharing the orchestrator's
    /// window, gets a window of its own (`layout break`, tmux's
    /// `break-pane -d`), so the orchestrator keeps its window and focus.
    /// Only on this click: nothing rearranges a runner's windows unasked.
    @discardableResult
    private func moveOutOfOrchestratorWindow(_ terminal: Terminal, in worktree: Worktree) async -> Bool {
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
            onHide: { Task { await act(on: ws) { c in await c.hideWorktree(ws.short) } } },
            onUnhide: { Task { await act(on: ws) { c in await c.unhideWorktree(ws.short) } } },
            onRemove: { removeWorktree = ws },
            onOpenTerminal: { t in open(ws, terminal: t.id) },
            onShowChanges: showChangesAction(for: ws, usable: store.refusal(for: host) == nil)
        )
    }

    /// Open `worktree`, or `terminal` in it, as its sidebar row and its card
    /// do. See `navigate(to:key:)`.
    private func open(_ worktree: Worktree, terminal: String?) {
        navigate(to: Self.opening(worktree, terminal: terminal, in: store.fleet))
    }

    /// Go to `next`, an explicit open: a sidebar row, a card, Needs You, the
    /// palette, or a pane gone to from the keyboard. A terminal it names that
    /// shares a seated orchestrator's window is moved to a window of its own
    /// first (`moveOutOfOrchestratorWindow`: the sharer, never the
    /// orchestrator), because the Orchestrator column draws that window and
    /// the checkout can't draw a pane apart from it (ov-78). Opening it is
    /// the ask. If the move fails the selection stays where it was, with the
    /// banner saying why, rather than landing on a pane that isn't there.
    private func navigate(to next: Selection, key: PaneRef? = nil) {
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
    private func bareTerminal(_ pane: BoardPane) -> some View {
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
                // Not through `act(on:_:)`: this is geometry, not a click.
                await store.client(for: ws)?.resize(terminal: term.short, columns: cols, rows: rows)
            },
            onSearchFiles: { query in
                await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? []
            },
            onAction: { action in Task { await run(action, on: term, in: ws) } },
            hasKeyboard: WorkspaceScreen.bareTakesKeyboard(shown)
        )
    }

    /// A workspace's board, or a sentence saying where it went.
    @ViewBuilder
    private func boardColumn(host: String, id: String) -> some View {
        if let client = store.clients[host], client.daemonBuild.map({ !$0.can("tasks") }) == true {
            // A runner too old for boards: said, rather than a board that
            // can't be read.
            ContentUnavailableView {
                Label("No board on this runner", systemImage: "checklist")
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
                waiting: client.boardWaiting(
                    columnCount: boardStore(for: workspace, client: client, host: host).board.waitingOnYou,
                    decisions: WorkspaceCounts.decisions(for: workspace, host: host, in: store.needsYou)),
                onGoTo: { pane in go(to: pane) }
            )
        } else {
            // Said, rather than the generic "Select a worktree": this
            // was a board, and the reader should know where it went.
            ContentUnavailableView {
                Label("This board isn’t here", systemImage: "checklist")
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
    /// behaved differently depending on which sidebar row you had clicked last.
    @ViewBuilder
    private func tiled(_ shown: ShownLayout, titled: Bool = true) -> some View {
        let ws = shown.worktree
        if let client = store.client(for: ws) {
            tiled(shown, client: client, frame: Self.frame(of: shown, in: store.fleet), titled: titled)
        } else {
            placeholder
        }
    }

    private func tiled(
        _ shown: ShownLayout, client: DaemonClient, frame: (title: String, subtitle: String), titled: Bool
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
                Task { await act(on: ws) { c in await c.focusPane(pane.short, in: ws) } }
            },
            onSelectGroup: { chosen in
                Task {
                    // Land in the layout you just chose, not on whatever pane the
                    // previous one had focused.
                    let groups = await act(on: ws, default: []) { c in
                        await c.selectLayout(chosen.id, in: ws)
                    }
                    reveal(groups, in: ws)
                }
            },
            onDropOnPane: { dragged, target, side in
                placePane(dragged, onto: target, side: side, in: ws)
            },
            onViewport: { layout, columns, rows in
                // Not routed through `act(on:_:)` — see `onGeometry`'s
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
            title: frame.title,
            subtitle: frame.subtitle,
            setsTitle: titled,
            hasKeyboard: WorkspaceScreen.hasKeyboard(shown, key: selectedPane, onBoard: keyboardOnBoard)
        )
    }

    private var placeholder: some View {
        ContentUnavailableView {
            Label("Select a workspace", systemImage: "square.stack.3d.up")
        } description: {
            Text("A workspace shows its orchestrator beside its board.")
        } actions: {
            if !store.repositories.isEmpty {
                Button("New Worktree…") {
                    newWorktreeIntent = NewWorktreeIntent()
                }
            }
        }
    }

    // MARK: - Routing

    /// A repository to default the project picker to, when nothing was
    /// chosen yet — the empty state's "New Worktree…" button, the sidebar's
    /// own `+`, and the palette's "New Worktree…" all reach this with no project
    /// and therefore no host in hand at all, which is the one case where a
    /// default runner is legitimate rather than the picker again in
    /// disguise. This Mac's own repositories come first: it is the runner
    /// guaranteed to be present, the one everything else is optional next to.
    /// Falls back to any repository so the picker still has something to
    /// preselect the very first time, before this Mac has one of its own.
    private var defaultProjectID: String? {
        store.repositories.first { $0.host.isEmpty }?.repository.id
            ?? store.repositories.first?.repository.id
    }

    /// Route a mutation to the runner a worktree is on, refusing it first.
    ///
    /// Checked here rather than at each call site — see `FleetStore.refusal(for:)`
    /// for why. On refusal, `fallback` is handed back and nothing is called; on
    /// success, whatever the client itself left in `lastError` is surfaced too,
    /// which is how a command that reached its runner and failed there still
    /// reaches the banner.
    @discardableResult
    private func act<T>(
        on ws: Worktree, default fallback: T, _ body: (DaemonClient) async -> T
    ) async -> T {
        if let why = store.refusal(for: ws) {
            errorBanner = "Cannot do that: \(why)"
            return fallback
        }
        guard let client = store.client(for: ws) else { return fallback }
        let result = await body(client)
        if let failure = client.lastError { errorBanner = failure }
        return result
    }

    private func act(on ws: Worktree, _ body: (DaemonClient) async -> Void) async {
        await act(on: ws, default: ()) { client in await body(client) }
    }

    /// A terminal's rendered screen, for the palette's preview tiles.
    ///
    /// `short` is what `ScreenPreviews` keys everything by, and short ids can
    /// collide across runners — the reason `Selection` carries a host at all.
    /// This is the one place left that has to work backwards from a bare short
    /// id with no host of its own to check against, because that is the whole
    /// interface `ScreenPreviews` and `CommandPalette` were built around. Local
    /// runner first, then the rest in the same order the sidebar lists them:
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

    private func run(_ action: TerminalAction, on term: Terminal, in worktree: Worktree) async {
        switch action {
        case .restart: await act(on: worktree) { c in await c.restart(terminal: term.short) }
        case .dismissLost: await act(on: worktree) { c in await c.dismissLost(term) }
        case .stop: await act(on: worktree) { c in await c.stop(terminal: term.short) }
        case .useAsOrchestrator: useAsOrchestrator(BoardPane(terminal: term, worktree: worktree))
        case .stopBeingOrchestrator: await stepDown(BoardPane(terminal: term, worktree: worktree))
        }
    }

    /// Use as Orchestrator: make `pane`, already running, its workspace's
    /// orchestrator. Asks first when that would replace one, naming it.
    private func useAsOrchestrator(_ pane: BoardPane) {
        let host = pane.worktree.host ?? ""
        if let why = store.refusal(for: host) {
            errorBanner = "Cannot do that: \(why)"
            return
        }
        guard let id = pane.terminal.workspace,
            let workspace = store.fleet.runnerWorkspaces[host]?.first(where: { $0.id == id })
        else {
            errorBanner = OrchestratorAdoption.refusal(
                "code: invalid-argument\nwhat: workspace", terminal: pane.terminal.label, workspace: "")
            return
        }
        if let old = OrchestratorAdoption.replacing(pane, in: workspace, host: host, fleet: store.fleet) {
            adoptionPending = OrchestratorAdoptionPending(host: host, workspace: workspace, pane: pane, old: old)
        } else {
            Task { await adopt(pane, in: workspace, host: host, replacing: nil) }
        }
    }

    /// Set the roles: `old` steps down first, since the runner allows one
    /// live orchestrator a workspace, then `pane` takes the seat, and every
    /// other pane in its window is moved to a window of its own. If the
    /// runner refuses `pane`, `old` is put back, so a refusal never leaves
    /// the workspace with none. The column follows on the refresh.
    private func adopt(_ pane: BoardPane, in workspace: WorkspaceSummary, host: String, replacing old: BoardPane?) async {
        guard let client = store.clients[host] else { return }
        if let old {
            let (refused, message) = await client.setRole(old.terminal, to: OrchestratorAdoption.steppedDown(old.terminal))
            if refused {
                errorBanner = OrchestratorAdoption.refusal(message, terminal: old.terminal.label, workspace: workspace.name)
                return
            }
        }
        let (refused, message) = await client.setRole(pane.terminal, to: "orchestrator")
        guard refused else {
            // Adopted: what shares its window moves out, the orchestrator it
            // replaced included, so the column draws it alone (ov-78). The
            // adopting is the ask.
            for other in WorkspaceScreen.movedOnAdopting(
                pane, replacing: old?.terminal.id, layouts: client.layouts[pane.worktree.id])
            {
                await moveOutOfOrchestratorWindow(other, in: pane.worktree)
            }
            return
        }
        errorBanner = OrchestratorAdoption.refusal(message, terminal: pane.terminal.label, workspace: workspace.name)
        if let old { _ = await client.setRole(old.terminal, to: "orchestrator") }
    }

    /// Stop Being Orchestrator: `pane` goes back to what it would have been
    /// made as (`OrchestratorAdoption.steppedDown`), and keeps running.
    private func stepDown(_ pane: BoardPane) async {
        guard let client = store.client(for: pane.worktree) else { return }
        let workspace = pane.terminal.workspace.flatMap { id in
            store.fleet.runnerWorkspaces[pane.worktree.host ?? ""]?.first { $0.id == id }
        }
        let (refused, message) = await client.setRole(pane.terminal, to: OrchestratorAdoption.steppedDown(pane.terminal))
        if refused {
            errorBanner = OrchestratorAdoption.refusal(
                message, terminal: pane.terminal.label, workspace: workspace?.name ?? "The workspace")
        }
    }

    private func worktree(host: String, id: String) -> Worktree? {
        store.fleet.worktrees.first { ($0.host ?? "") == host && $0.id == id }
    }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }

    /// Where the window opens: Needs You when anything is waiting, else the
    /// last workspace selection (spec §4.6, ruling 4). See
    /// `SelectionMemory.launch`.
    ///
    /// Asked on every fleet and Needs You change until it has an answer, since
    /// each runner comes up on its own schedule, and never again after: a
    /// window that has opened somewhere, or where somebody already clicked,
    /// isn't moved by a count that rises later.
    private func settleLaunch() {
        SelectionMemory.migrate(
            .standard, fleet: store.fleet, ready: { host in store.clients[host]?.hasLoaded ?? true })
        guard !launched else { return }
        guard selection == nil else {
            launched = true
            return
        }
        guard
            let decided = SelectionMemory.launch(
                needsYou: store.needsYou.count, settled: store.needsYouSettled,
                last: SelectionMemory.decode(lastSelection), in: store.fleet)
        else { return }
        launched = true
        selection = decided
    }

    // MARK: - Commands

    /// The terminals of the view on screen, in the order they're drawn:
    /// what ⌘] and ⌘[, ⌥⌘↓ and ⌥⌘↑ and ⌘1… step through (spec §4.9). The
    /// sidebar no longer lists terminals, so stepping through all of them
    /// would walk a list nobody can see.
    private var allTerminals: [PaneRef] { Self.stepOrder(shown) }

    /// `allTerminals` for what's shown: column by column, each layout's
    /// panes in tmux's order.
    static func stepOrder(_ shown: [ShownLayout]) -> [PaneRef] {
        shown.flatMap { layout in
            layout.group.terminals.map { PaneRef(host: layout.host, worktree: layout.worktree.id, terminal: $0) }
        }
    }

    /// The pane the keyboard acts on, with its worktree and terminal. See
    /// `WorkspaceScreen.keyPane`.
    private var selectedPane: PaneRef? { WorkspaceScreen.keyPane(keyPane, in: shown, selection: selection) }

    private var selectedTerminal: (worktree: Worktree, terminal: Terminal)? {
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
    private func onScreen(in ws: Worktree) -> (group: PaneGroup, groups: [PaneGroup])? {
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
    /// Not routed through `act(on:_:)`: this is a best-effort background
    /// bookkeeping call, not a user-initiated action, and a runner gone quiet
    /// for a moment must not put "Cannot do that" on screen just because an
    /// agent on it happened to finish.
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
    private var tileTarget: Worktree? { currentWorktree }

    /// Carry out a `⌃B`-prefixed command.
    ///
    /// Every one of them is a single `layout` call whose reply is the worktree's
    /// whole layout, so there is nothing to reconcile here: tmux decides what a
    /// zoom or a split means, and it decides the same way for this app, for the
    /// CLI and for an agent driving the CLI.
    ///
    /// Each names the layout on screen (`shown`), never leaving the runner to
    /// pick one. The runner's pick for the main checkout is never an
    /// orchestrator's window, so the orchestrator's row has to name its own,
    /// or its ⌃B z would zoom the checkout's.
    ///
    /// This file no longer contributes geometry. Directional focus used to be
    /// worked out here from a recomputed arrangement; it is now read off the
    /// rectangles tmux reported, which is the only copy.
    private func tile(_ command: TileCommand) async {
        guard let worktree = tileTarget else { return }
        let screen = onScreen(in: worktree)
        let group = screen?.group
        let shown = group?.id
        /// The pane a keystroke acts on: the selected one, else whatever tmux says
        /// is focused.
        let here: PaneRect? = {
            if let id = selectedPane?.terminal, let pane = group?.pane(id) { return pane }
            return group?.panes.first(where: \.focused)
        }()
        // The orchestrator is one pane (ov-78): with the keyboard in its
        // column, what would add a pane there opens a shell in the main
        // checkout instead, which the workspace drills into.
        if WorkspaceScreen.opensShellInstead(command, key: selectedPane, in: self.shown) {
            await openShell(besideOrchestratorIn: worktree)
            return
        }

        switch command {
        case .zoom:
            await act(on: worktree) { c in await c.zoomPane(nil, in: worktree, layout: shown) }

        case .focusNext:
            await act(on: worktree) { c in
                await c.focusPane(step: "--next", in: worktree, layout: shown)
            }
        case .focusPrevious:
            await act(on: worktree) { c in
                await c.focusPane(step: "--prev", in: worktree, layout: shown)
            }

        case .focus(let direction):
            guard let group, let from = here,
                let next = group.neighbour(of: from.id, direction)
            else { return }
            await act(on: worktree) { c in await c.focusPane(next.short, in: worktree) }

        case .focusIndex(let n):
            // Counted in the layout on screen. See `pane(numbered:in:)`.
            guard let pane = Self.pane(numbered: n, in: group) else { return }
            await act(on: worktree) { c in await c.focusPane(pane.short, in: worktree) }

        case .cycle:
            await act(on: worktree) { c in await c.cycleLayout(worktree, layout: shown) }

        case .preset(let preset):
            await act(on: worktree) { c in await c.applyPreset(preset, in: worktree, layout: shown) }

        case .evenPanes:
            // Which even arrangement, read off the panes rather than asked for.
            //
            // tmux has two — columns and rows — and picking the wrong one does
            // not "even out" a layout, it turns it inside out: a stack of three
            // becomes a row of three. So this counts how the window is already
            // split, the same way `TileView.Viewport` does, and hands back the
            // even version of the shape that is on screen.
            let columns = Set(group?.panes.map(\.left) ?? []).count
            let rows = Set(group?.panes.map(\.top) ?? []).count
            let preset: TilePreset = columns >= rows ? .evenHorizontal : .evenVertical
            await act(on: worktree) { c in await c.applyPreset(preset, in: worktree, layout: shown) }

        case .splitRight, .splitDown:
            // One call. It used to be create-then-join-then-apply-a-preset, three
            // round trips whose only way of saying WHERE the new pane went was to
            // re-arrange every pane in the layout — so splitting the third pane of
            // four rebuilt the other three as well. `layout split` splits the pane
            // you name, on the side you name, and leaves the rest alone.
            let side: TileDirection = command == .splitRight ? .right : .bottom
            let groups = await act(on: worktree, default: []) { c in
                await c.split(worktree, beside: here?.short, side: side, layout: shown)
            }
            // Land in the pane that was just made, which is the one tmux focuses.
            reveal(groups, in: worktree)

        case .breakPane:
            guard let here else { return }
            // Never the orchestrator: what shares its window moves out
            // instead, as Move to Its Own Window does.
            if let seat = WorkspaceScreen.seated(in: worktree, fleet: store.fleet).map(\.pane)
                .first(where: { $0.terminal.id == here.id })
            {
                let sharers = WorkspaceScreen.sharers(of: seat, layouts: store.client(for: worktree)?.layouts[worktree.id])
                if sharers.isEmpty { errorBanner = "The orchestrator already has a window of its own." }
                for sharer in sharers { await moveOutOfOrchestratorWindow(sharer, in: worktree) }
                return
            }
            let groups = await act(on: worktree, default: []) { c in
                await c.breakPane(here.short, in: worktree)
            }
            reveal(groups, in: worktree, preferring: here.id)

        case .closePane:
            // tmux's `x`, and it means the same thing: the pane's process ends.
            // Routed through the existing close so there is one implementation of
            // what closing a terminal does.
            run(.closeTerminal)

        case .newGroup:
            await openTerminalInNewLayout(worktree)

        case .nextGroup:
            // Landing in the new layout is the point of switching to it. Without
            // this the selection stayed on a pane from the OLD layout, which the
            // detail view then showed on its own — so ⌃B n looked like it opened a
            // random terminal and came back.
            //
            // Through the layouts the bar offers. See `layout(stepping:from:in:)`.
            guard let next = Self.layout(stepping: 1, from: group?.id, in: screen?.groups ?? [])
            else { return }
            let groups = await act(on: worktree, default: []) { c in
                await c.selectLayout(next.id, in: worktree)
            }
            reveal(groups, in: worktree)
        case .previousGroup:
            guard let previous = Self.layout(stepping: -1, from: group?.id, in: screen?.groups ?? [])
            else { return }
            let groups = await act(on: worktree, default: []) { c in
                await c.selectLayout(previous.id, in: worktree)
            }
            reveal(groups, in: worktree)

        case .toggleAgentPane:
            // Falls back to the plain selection when there is no tmux group
            // yet — the few seconds `detail`'s own comment describes, before
            // the first `layout show` has come back, where `here` is nil but
            // a terminal is still very much selected.
            let target =
                here.flatMap { rect in worktree.terminals.first { $0.id == rect.id } }
                ?? selectedTerminal?.terminal
                // Selecting a LAYOUT TAB is not selecting a terminal, and a
                // cached layout does not always mark a pane focused — so both
                // of the above are nil for the commonest way of getting here,
                // and the command silently did nothing at all. A layout with
                // one pane has no ambiguity about which pane is meant.
                ?? group?.panes.first.flatMap { pane in
                    worktree.terminals.first { $0.id == pane.id }
                }
            guard let target else {
                // Never silent. A keystroke that does nothing and says nothing
                // is indistinguishable from a broken feature.
                errorBanner = "No pane to switch — select a terminal first."
                return
            }
            // Said here rather than left to the daemon's refusal, because this
            // is where the agent's NAME is known. The on-pane button is hidden
            // for a pane that cannot switch; the keystroke was not, so ⌃B a on
            // a Codex pane did nothing and explained nothing.
            guard target.canSwitchPaneMode || target.isAgentPane else {
                // Two different refusals, not one. A plain shell was never an
                // agent and telling it to add a config.toml entry is bad
                // advice; an agent Far Cooler recognizes but cannot host
                // needs exactly that entry. Mirrors the daemon's own two
                // refusal strings in `Service::set_pane_mode`.
                if target.hasDetectedAgent {
                    let agent = target.agentLabel
                    errorBanner =
                        "\(agent) has no chat adapter, so it stays a terminal. Add one in "
                        + "~/.config/farcooler/config.toml, then restart the daemon "
                        + "(farcooler daemon ensure) — it only reads the file at startup."
                } else {
                    errorBanner = "Nothing here to chat with — this pane isn’t running an agent."
                }
                return
            }
            await togglePaneMode(target, in: worktree)

        case .help:
            showShortcuts = true
        }
    }

    /// Ask the daemon to flip a pane between its terminal and its agent chat.
    ///
    /// One call, and the daemon is the one deciding whether that is even
    /// possible — a client guessing "this preset can't be an agent" would be
    /// exactly the kind of state the design says clients never derive.
    private func togglePaneMode(_ terminal: Terminal, in worktree: Worktree) async {
        let target = terminal.isAgentPane ? "terminal" : "agent"
        let result = await act(on: worktree, default: DaemonClient.PaneModeResult.ok) { c in
            await c.setPaneMode(terminal.short, mode: target)
        }
        switch result {
        case .ok, .failed:
            // A failure already reached `errorBanner` via `act`, so there is
            // nothing further to do from here.
            break
        case let .confirmationRequired(message):
            pendingPaneModeSwitch = PaneModeConfirmation(
                worktree: worktree, terminal: terminal.short, mode: target, message: message)
        }
    }

    /// Send a terminal to another layout, or to one of its own.
    ///
    /// `nil` is `break-pane`: a layout with just this in it. Naming a layout moves
    /// the pane against that layout's focused pane, because "which layout" is only
    /// half an instruction — tmux has to be told which pane and which edge, and the
    /// menu has no way to ask. Dragging is how you say the other half, and the drop
    /// indicator is why that is easier than answering a dialog about it.
    private func moveToLayout(_ terminal: Terminal, in worktree: Worktree, group: PaneGroup?) {
        Task {
            let groups: [PaneGroup]
            if let group, let onto = group.panes.first(where: \.focused) ?? group.panes.first {
                groups = await act(on: worktree, default: []) { c in
                    await c.movePane(terminal.short, onto: onto.short, side: .right, in: worktree)
                }
            } else {
                groups = await act(on: worktree, default: []) { c in
                    await c.breakPane(terminal.short, in: worktree)
                }
            }
            reveal(groups, in: worktree, preferring: terminal.id)
        }
    }

    /// Drop a terminal on an edge of a pane: it splits that pane on that edge.
    ///
    /// One write for every drag in the app — a pane onto a pane, a sidebar row
    /// onto a pane, a sidebar row onto another row — because they all say the same
    /// thing: this terminal, against that one, on this side. It was three
    /// operations while a layout was an ordered list, and the list could only
    /// express "before" and "after", which is why dropping on the left half of a
    /// pane and on its right half used to do the same thing.
    ///
    /// Move a divider, in cells. Answers whether the request was taken.
    ///
    /// Serialized rather than queued. A drag produces one of these per cell
    /// crossed and each is a round trip, so a fast drag would stack up dozens of
    /// requests that land after the pointer has stopped and walk the divider past
    /// where it was let go.
    ///
    /// Refusing has to be VISIBLE to the caller, which is what the return value is
    /// for. The handle counts cells the layout has actually moved by, so a refusal
    /// leaves them owed and the next mouse event asks for them again. Without
    /// that the handle counted a dropped request as done and threw its cells away
    /// — losing most of them over a fast drag, so the divider followed the pointer
    /// at a fraction of its speed.
    @discardableResult
    private func resizeDivider(
        _ terminal: String, side: TileDirection, cells: Int, in worktree: Worktree
    ) -> Bool {
        guard cells != 0, !resizingDivider else { return false }
        guard let pane = store.client(for: worktree)?.group(holding: terminal, in: worktree.id)?
            .pane(terminal)
        else {
            return false
        }
        resizingDivider = true
        Task {
            await act(on: worktree) { c in
                await c.resizePane(pane.short, side: side, cells: cells, in: worktree)
            }
            resizingDivider = false
        }
        return true
    }

    /// Works across layouts too: the pane leaves whichever one it was in.
    private func placePane(
        _ dragged: String, onto target: String, side: TileDirection, in worktree: Worktree
    ) {
        let shorts = [dragged, target].compactMap { id in
            worktree.terminals.first { $0.id == id }?.short
        }
        guard shorts.count == 2 else { return }
        // The orchestrator is one pane (ov-78): nothing joins its window,
        // and it never leaves it.
        let window = store.client(for: worktree)?.group(holding: target, in: worktree.id)?.terminals ?? [target]
        if WorkspaceScreen.joinsOrchestrator(dragged, window: window, in: worktree, fleet: store.fleet) {
            errorBanner = "The orchestrator keeps a window of its own, so nothing can be put beside it."
            return
        }
        Task {
            let groups = await act(on: worktree, default: []) { c in
                await c.movePane(shorts[0], onto: shorts[1], side: side, in: worktree)
            }
            reveal(groups, in: worktree, preferring: dragged)
        }
    }

    /// Select the active layout's focused pane after a layout command.
    ///
    /// Every command that changes which group is on screen goes through this, so
    /// "the thing I am looking at" and "the thing the layout says is focused" cannot
    /// disagree. `preferring` is for the cases where the command was about a
    /// specific terminal and that terminal should win.
    private func reveal(
        _ groups: [PaneGroup], in worktree: Worktree, preferring: String? = nil
    ) {
        let host = worktree.host ?? ""
        guard let active = groups.first(where: { $0.isActive }) ?? groups.first else {
            // No layouts left, which now means no terminals left. Fall back to
            // the worktree rather than to a pane that no longer exists.
            if Self.shows(worktree, selection) { selection = Self.opening(worktree, terminal: nil, in: store.fleet) }
            return
        }
        let target = preferring.flatMap { active.terminals.contains($0) ? $0 : nil }
            ?? active.focused
            ?? active.terminals.first
        guard let target else {
            if Self.shows(worktree, selection) { selection = Self.opening(worktree, terminal: nil, in: store.fleet) }
            return
        }
        focus(PaneRef(host: host, worktree: worktree.id, terminal: target))
    }

    /// Whether `selection` opens `worktree` whole: the one case where a
    /// layout command's answer can move the selection within it.
    nonisolated static func shows(_ worktree: Worktree, _ selection: Selection?) -> Bool {
        selected(in: worktree, by: selection) != nil
    }

    /// Put the keyboard in `pane`, which is on screen or about to be.
    ///
    /// Within the view on screen: a worktree opened whole selects the pane
    /// in it, so its sidebar row lights and it's what the window reopens on;
    /// in the conversation or a task's column, the selection stays and only
    /// the key pane moves. A pane on no column of this view goes to where it
    /// lives, as `land(on:)` does.
    private func focus(_ pane: PaneRef) {
        changesFocus = nil
        keyboardOnBoard = false
        // A click into what's opened puts the popped-open orchestrator away.
        if orchestratorPeek, !shown.contains(where: { $0.column == .conversation && $0.contains(pane) }) {
            orchestratorPeek = false
        }
        guard let next = WorkspaceScreen.focusing(pane, selection: selection, shown: shown, fleet: store.fleet)
        else {
            land(on: pane)
            return
        }
        if next != selection { selection = next }
        keyPane = pane
    }

    /// ⌥⌘1, ⌥⌘2, ⌥⌘3: the conversation, the board, or the task or
    /// worktree opened, brought on screen and given the keyboard.
    ///
    /// Drilled in, ⌥⌘1 pops the orchestrator open over what's opened, and
    /// ⌥⌘2 goes back up to the board: neither leaves a task you're reading
    /// for the conversation (ov-79).
    private func focusWorkspaceColumn(_ command: AppCommand) {
        guard case .workspace(let host, let id, let focus)? = selection else { return }
        let key = "\(host)|\(id)"
        switch command {
        case .focusConversation:
            workspacePicks[key] = .orchestrator
            focusColumn = false
            if focus != nil { orchestratorPeek = true }
            if let pane = shownLayouts(for: selection).first(where: { $0.column == .conversation })
                .flatMap(WorkspaceScreen.columnPane)
            {
                step(to: pane)
            }
        case .focusBoard:
            workspacePicks[key] = .board
            focusColumn = false
            // Beside the others, the board has no text to type into: the
            // terminals let go of the keyboard. After going up, which clears
            // it as every navigation does.
            if focus != nil {
                trail = nil
                selection = selection?.closed
                DispatchQueue.main.async {
                    keyboardOnBoard = true
                    windowBox.window?.makeFirstResponder(nil)
                }
            } else {
                keyboardOnBoard = true
                windowBox.window?.makeFirstResponder(nil)
            }
        case .focusTask:
            orchestratorPeek = false
            if let pane = shownLayouts(for: selection).last(where: { $0.column != .conversation })
                .flatMap(WorkspaceScreen.columnPane)
            {
                step(to: pane)
            }
        default:
            break
        }
    }

    /// Open a task: its workspace, drilled into it.
    private func openTask(_ id: String, host: String, workspace: String) {
        trail = nil
        selection = .workspace(host: host, workspace: workspace, focus: .task(id))
    }

    /// Open a Needs You item where spec §2.5 says it lands.
    private func open(_ item: NeedsYouItem) {
        lastAttention = item.key
        guard let landed = NeedsYouNavigation.landing(for: item, in: store.fleet) else {
            errorBanner = "That’s no longer on its runner."
            return
        }
        navigate(to: landed)
    }

    /// Go to `pane` wherever it lives: its workspace, its task, or its
    /// worktree (`WorkspaceSelection.landing`).
    private func land(on pane: PaneRef) {
        expanded.insert(pane.worktree)
        guard let landed = WorkspaceSelection.landing(on: pane, in: store.fleet) else { return }
        navigate(to: landed, key: pane)
    }

    /// Follow the layout's focus when something else moved it.
    ///
    /// The CLI and an agent can both focus a pane, and when they do the app has
    /// to be looking at it — otherwise `farcooler layout focus` from a script
    /// draws a border around a pane whose keystrokes still go somewhere else.
    ///
    /// Within one layout only, and now that is the whole of what it does. The
    /// last condition — the selected terminal being in the layout that moved —
    /// is what keeps this from flip-flopping: the app assumes a focus locally
    /// (`DaemonClient.assumeFocus`) and the runner's confirmation of the
    /// PREVIOUS focus can still be in flight, so a version that followed across
    /// layouts would follow that stale answer straight back. The cost is that
    /// `layout focus` aimed at another layout no longer brings it on screen —
    /// the app draws the layout holding the SELECTED terminal now, so it stays
    /// where you left it. Selection and screen agree, which they did not before;
    /// the runner and the app disagree about which layout is at the front, which
    /// nothing on screen claims either way.
    private func followLayoutFocus() {
        guard let pane = selectedPane,
            let worktree = worktree(host: pane.host, id: pane.worktree),
            let group = store.client(for: worktree)?.activeGroup(pane.worktree),
            let focused = group.focused,
            focused != pane.terminal,
            group.terminals.contains(pane.terminal)
        else { return }
        focus(PaneRef(host: pane.host, worktree: pane.worktree, terminal: focused))
    }

    private func run(_ command: AppCommand) {
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
                await act(on: worktree) { c in await c.stop(terminal: terminal.short) }
                await act(on: worktree) { c in await c.removeTerminal(terminal.short) }
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
                // Never a guess between several. The rows are in the sidebar
                // for exactly this case.
                errorBanner = "Select a workspace first."
            }
        case .back: goBack()
        case .focusColumn:
            if selection?.focus != nil {
                focusColumn.toggle()
                if focusColumn { orchestratorPeek = false }
            }
        case .focusConversation, .focusBoard, .focusTask:
            focusWorkspaceColumn(command)
        case .openInEditor: openInPreferredEditor()
        case .reload: Task { for client in store.clients.values { await client.refresh() } }
        case .showShortcuts: showShortcuts = true
        case .about: showAbout = true
        case .search: searchFocused = true

        // Toggles rather than opens. ⌘P on an open palette is what a hand
        // reaches for when it changed its mind, and every switcher on this
        // machine closes that way.
        case .commandPalette: showPalette.toggle()

        case .toggleSidebar:
            // See `Sidebar.toggle`: AppKit's own action, with the collapse
            // behavior set first so the detail pane absorbs the space instead of
            // the window growing.
            Sidebar.toggle()

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
    /// panel and clicking it in the sidebar cannot drift apart.
    private func perform(_ action: PaletteAction) {
        showPalette = false
        switch action {
        case .openTerminal(let worktree, let terminal):
            let host = store.fleet.worktrees.first { $0.id == worktree }?.host ?? ""
            land(on: PaneRef(host: host, worktree: worktree, terminal: terminal))

        case .openWorktree(let id):
            expanded.insert(id)
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
    private func openInPreferredEditor() {
        guard let worktree = detailWorktree else {
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

    /// Start a task and go to it as soon as it exists.
    ///
    /// Selects the worktree and its agent's terminal the moment
    /// `DaemonClient.startTask` has made them, which is before the agent has
    /// booted: the description travels as the agent's launch argument, so
    /// nothing is left to wait for. The point is to be looking at the thing
    /// you asked for while it starts. (This comment said so from the start,
    /// while `startTask` in fact waited up to a minute for the agent to look
    /// idle before returning. It is true now.)
    ///
    /// `host` is handed in rather than re-derived from `project` here — see
    /// `QuickCreate.chosen`, which resolves both together from the same
    /// picker selection. A lookup repeated at this end, from `project`
    /// alone, is exactly the shape that goes silently wrong: `project` can
    /// name a repository that has since been removed, or one on a runner
    /// whose `repositories` has not been re-read since a reconnect, and a
    /// lookup that finds nothing has to be answered with a refusal, not a
    /// fallback to this Mac.
    private func startTask(_ request: TaskRequest) async -> TaskSubmission.Outcome {
        let host = request.host
        if let client = store.clients[host], client.state == .notInstalled {
            return .failed("Far Cooler isn’t installed on this runner, so the agent wasn’t started.")
        }
        guard store.refusal(for: host) == nil, let client = store.clients[host] else {
            return .failed(
                "Can’t reach this runner right now, so the agent wasn’t started. Try again once it’s back.")
        }
        let outcome = await client.startTask(
            project: request.project,
            description: request.description,
            name: request.name,
            agent: request.preset.isEmpty ? Preferences.shared.defaultAgent : request.preset,
            reusing: request.worktree,
            // Claimed for the workspace the window is in, when the new
            // worktree is going into that workspace's repository, and
            // otherwise for that repository's Main.
            workspace: Self.claim(
                newWorktreeIn: request.project, on: host, from: selection, in: store.fleet),
            // After the start has returned and the panel has let go of the
            // draft, so it's said over the pane instead.
            undelivered: { sentence in errorBanner = sentence })
        switch outcome {
        case .started(let worktree, let terminal, let name):
            // By the ids the create calls returned, not by a later look at
            // the fleet — which, this soon, may not have the terminal yet.
            // A new worktree with no task yet is opened whole, under the
            // workspace it was claimed for.
            expanded.insert(worktree)
            selection = arrival(host: host, worktree: worktree, terminal: terminal)
            keyPane = PaneRef(host: host, worktree: worktree, terminal: terminal)
            return .started(name: name)
        case .failed(let sentence, let made):
            if let made { reveal(made.id) }
            // After the selection change, which clears the banner. The panel
            // shows it when it is still open; this is for when it is not.
            if !showQuickCreate { errorBanner = sentence }
            // The worktree it made, so starting again goes on in it.
            return .failed(
                sentence,
                left: made.map {
                    TaskSubmission.Left(
                        host: host, project: request.project, worktree: $0.id, name: $0.name)
                })
        }
    }

    /// Pick up an existing branch and open an agent in it.
    ///
    /// Same landing as starting a task, because it is the same act from the
    /// user's side: there is now a worktree with an agent in it and you want to
    /// be looking at it. The only difference is where the code came from.
    ///
    /// Routed the same way `startTask` is: `host` comes from the resume
    /// sheet's own picker selection rather than a `project`-keyed lookup
    /// repeated here — see `startTask`'s own doc comment for why that lookup
    /// belongs where the selection was made, not downstream of it.
    private func resume(branch: String, host: String, project: String, agent: String) {
        Task {
            if let why = store.refusal(for: host) {
                errorBanner = "Can’t do that because \(why)."
                return
            }
            guard let client = store.clients[host] else { return }
            let created = await client.adoptBranch(
                project: project, branch: branch, agent: agent)
            reveal(created)
        }
    }

    /// Select a freshly created worktree, preferring its terminal.
    private func reveal(_ worktree: String?) {
        guard let worktree else { return }
        expanded.insert(worktree)
        let found = store.fleet.worktrees.first { $0.id == worktree }
        let host = found?.host ?? ""
        selection = arrival(host: host, worktree: worktree, terminal: found?.terminals.first?.id)
    }

    /// Where a worktree this window just made lands, with `terminal` in it:
    /// opened whole under the workspace the window is in when the fleet
    /// hasn't listed it yet, which, this soon after making it, is usual.
    private func arrival(host: String, worktree: String, terminal: String?) -> Selection {
        if let found = self.worktree(host: host, id: worktree) {
            return Self.opening(found, terminal: terminal, in: store.fleet)
        }
        if case .workspace(host, let current, _)? = selection {
            return .workspace(host: host, workspace: current, focus: .worktree(worktree, terminal: terminal))
        }
        return .looseWorktree(host: host, worktree: worktree, terminal: terminal)
    }

    /// Create a terminal and go straight to it.
    ///
    /// Selecting it afterwards matters: you made a terminal because you want to
    /// type in it, and leaving the selection where it was means a second click
    /// to get to the thing you just asked for.
    private func newTerminal(in worktree: Worktree) {
        Task { await openTerminalInNewLayout(worktree) }
    }

    /// A new terminal, in a layout of its own.
    ///
    /// Every way of making a terminal goes through this — the sidebar button, ⌘T,
    /// the palette's action, ⌃B c.
    ///
    /// One call now, where it used to be two. A terminal IS a tmux window and a
    /// window IS a layout, so creating one already produces the layout; the
    /// separate "make a group, then put it in the group" step was describing a
    /// distinction that no longer exists.
    ///
    /// tmux's `c` opens a window with a shell in it. So does this.
    @discardableResult
    private func openTerminalInNewLayout(_ worktree: Worktree) async -> Terminal? {
        expanded.insert(worktree.id)
        guard
            let created = await act(
                on: worktree, default: nil as Terminal?,
                { c in
                    await c.createTerminal(
                        in: worktree,
                        preset: "shell",
                        title: "Terminal \(worktree.terminals.count + 1)")
                })
        else { return nil }
        focus(PaneRef(host: worktree.host ?? "", worktree: worktree.id, terminal: created.id))
        return created
    }

    /// ⌃B %, ⌃B " or ⌃B c with the keyboard in the Orchestrator column: a
    /// shell in the main checkout, in a window of its own, drilled into in
    /// the workspace on screen. Never a split of the
    /// orchestrator's window, which would make a checkout terminal only the
    /// column could draw (ov-78).
    private func openShell(besideOrchestratorIn checkout: Worktree) async {
        guard case .workspace(let host, let id, _)? = selection else { return }
        expanded.insert(checkout.id)
        guard
            let created = await act(
                on: checkout, default: nil as Terminal?,
                { c in
                    await c.createTerminal(
                        in: checkout, preset: "shell", title: "Terminal \(checkout.terminals.count + 1)")
                })
        else { return }
        selection = .workspace(host: host, workspace: id, focus: .worktree(checkout.id, terminal: created.id))
        keyPane = PaneRef(host: host, worktree: checkout.id, terminal: created.id)
    }

    /// The worktree the detail pane is actually showing.
    ///
    /// Not `currentWorktree` by name, though the two now compute the same
    /// value: `currentWorktree`'s own fallback to the first worktree in the
    /// fleet is gone, removed for the same reason this property never had
    /// one — with nothing selected, the placeholder is on screen, and
    /// offering to open a worktree the window is not showing would be the
    /// control lying about what it points at. Kept as a separate property so
    /// each name still reads as what it answers: this one, what the detail
    /// pane draws; `currentWorktree`, what a keystroke acts on.
    private var detailWorktree: Worktree? {
        currentWorktree
    }

    /// The worktree the selection is in — nil when nothing is selected.
    ///
    /// Used to be "or the first one": with nothing selected, that first
    /// worktree could belong to ANY runner in the fleet, chosen by nothing
    /// more meaningful than merge order. `tileTarget` and ⌘T both read this
    /// to decide what a keystroke acts on, and a fallback here meant a ⌃B
    /// command issued while the detail pane was blank still landed — split,
    /// break, preset, cycle, zoom, focus, a new terminal — on whichever
    /// runner happened to own that first row. Wrong-runner routing is the
    /// one failure this feature must never produce, so with no selection
    /// there is now no target, and `tileTarget`'s and `.newTerminal`'s own
    /// `guard`/`if let` already do nothing rather than guess.
    private var currentWorktree: Worktree? {
        // The key pane's, which is on screen by construction. A board alone
        // is a workspace's, not a worktree's: a keystroke that acts on "the
        // current worktree" has nothing to act on there.
        if let pane = selectedPane { return worktree(host: pane.host, id: pane.worktree) }
        if let (host, id) = Self.openedWhole(selection) { return worktree(host: host, id: id) }
        return nil
    }

    private func step(by offset: Int) {
        let ordered = allTerminals
        guard !ordered.isEmpty else { return }
        let current = ordered.firstIndex { $0 == selectedPane } ?? 0
        // Wraps, because a list you can walk off the end of makes you look.
        let next = (current + offset + ordered.count) % ordered.count
        step(to: ordered[next])
    }

    /// Move the selection off a terminal, or a worktree, that has gone.
    ///
    /// Prefers to stay where the user was looking: another terminal in the same
    /// worktree, whatever wants attention first, then anything running. Only
    /// falls back to the worktree itself when the worktree is empty, and to a
    /// sibling worktree on the same runner when the worktree itself is gone.
    ///
    /// The one rule for every way a terminal or a worktree can disappear,
    /// closing one here with ⌘W included. See `healed(_:in:was:)`, which is the
    /// rule itself. `previous` is the fleet before the change, which is the only
    /// place a removed worktree's repository can still be read.
    private func healSelection(previous: [Worktree] = []) {
        let next = Self.healed(
            selection, in: store.fleet.worktrees, was: previous, workspaces: store.fleet.runnerWorkspaces)
        if next != selection { selection = next }
    }

    /// Where a selection goes when what it points at is gone, or the same
    /// selection when it is not.
    ///
    /// Static and free of the view so it can be tested; `healSelection` holds
    /// only the assignment.
    ///
    /// - **Never leaves the worktree while the worktree is there.** A closed
    ///   terminal's neighbor is in the worktree you were working in, not
    ///   wherever the fleet happens to list a running terminal first. The
    ///   phones keep the same promise their own way, clamping to the
    ///   neighboring tab (`ShellFleet.reseat`).
    /// - **Never leaves the runner.** When the worktree itself is gone, a
    ///   terminal selection and a worktree selection both land on a sibling
    ///   worktree on the same runner (see `sibling(of:host:in:was:)`), or on
    ///   nothing. It used to take the first worktree in the merged fleet,
    ///   which is often another runner's, and it left a selected worktree's
    ///   id in place after the worktree was gone.
    nonisolated static func healed(
        _ selection: Selection?, in worktrees: [Worktree], was previous: [Worktree] = [],
        workspaces: [String: [WorkspaceSummary]] = [:]
    ) -> Selection? {
        let host: String
        let worktreeID: String
        let terminalID: String?
        switch selection {
        case nil: return nil
        // Needs You is every runner's, and has nothing to heal.
        case .needsYou: return selection
        // A workspace with nothing opened has nothing to heal either: a
        // runner that loses it draws the column's sentence until you choose.
        // A task is its own view's to say it's gone.
        case .workspace(_, _, nil), .workspace(_, _, .task): return selection
        case .workspace(let h, _, .worktree(let w, let t)): (host, worktreeID, terminalID) = (h, w, t)
        case .looseWorktree(let h, let w, let t): (host, worktreeID, terminalID) = (h, w, t)
        }
        /// The same selection, opening `worktree` with `terminal` in it.
        func with(_ worktree: String, terminal: String?) -> Selection {
            switch selection {
            case .workspace(let h, let id, _)?: return .workspace(host: h, workspace: id, focus: .worktree(worktree, terminal: terminal))
            default: return .looseWorktree(host: host, worktree: worktree, terminal: terminal)
            }
        }
        guard
            let worktree = worktrees.first(where: {
                ($0.host ?? "") == host && $0.id == worktreeID
            })
        else {
            // Gone: a workspace's column closes, back to the workspace; a
            // loose worktree lands on a sibling on its runner.
            if case .workspace(let h, let id, _)? = selection { return .workspace(host: h, workspace: id, focus: nil) }
            return sibling(of: worktreeID, host: host, in: worktrees, was: previous, workspaces: workspaces)
        }
        guard let terminalID, !worktree.terminals.contains(where: { $0.id == terminalID })
        else {
            return selection
        }

        let candidates = worktree.terminals
        let next = candidates.first(where: { $0.status.wantsAttention })
            ?? candidates.first(where: { StateKind.parse($0.state) == .running })
            ?? candidates.first
        return with(worktreeID, terminal: next?.id)
    }

    /// The worktree to land on when `worktreeID` is gone: one on the same
    /// runner, in the same repository if `previous` still says which that was,
    /// and one the sidebar actually draws before a hidden one. Nil when the
    /// runner has none left, because landing on another runner is the app
    /// moving you somewhere you never asked to go.
    nonisolated static func sibling(
        of worktreeID: String, host: String, in worktrees: [Worktree],
        was previous: [Worktree], workspaces: [String: [WorkspaceSummary]] = [:]
    ) -> Selection? {
        let repository = previous.first {
            ($0.host ?? "") == host && $0.id == worktreeID
        }?.repository
        let sameRunner = worktrees.filter { ($0.host ?? "") == host && $0.id != worktreeID }
        let shown = sameRunner.filter { !$0.isHidden }
        let next =
            shown.first(where: { repository != nil && $0.repository == repository })
            ?? shown.first
            ?? sameRunner.first
        // Opened as its row would open it: under the workspace that owns it,
        // and loose only when none does.
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: worktrees, branchPrefix: nil)
        fleet.runnerWorkspaces = workspaces
        return next.map { opening($0, terminal: nil, in: fleet) }
    }

    private func selectTerminal(at index: Int) {
        let ordered = allTerminals
        guard index >= 0, index < ordered.count else { return }
        step(to: ordered[index])
    }

    /// Put the keyboard in a pane on screen, and tmux's focus with it.
    private func step(to pane: PaneRef) {
        focus(pane)
        guard let worktree = worktree(host: pane.host, id: pane.worktree),
            let rect = store.client(for: worktree)?.group(holding: pane.terminal, in: pane.worktree)?.pane(pane.terminal)
        else { return }
        Task { await act(on: worktree) { c in await c.focusPane(rect.short, in: worktree) } }
    }

}

/// What Esc does in the sidebar's search field.
enum SearchEscape {
    /// With text, clears it and keeps the field; empty, leaves the field.
    static func after(query: String) -> (query: String, keepsFocus: Bool) {
        query.isEmpty ? ("", false) : ("", true)
    }
}

enum TerminalAction { case restart, dismissLost, stop, useAsOrchestrator, stopBeingOrchestrator }

/// Errors the app writes but nothing else shows.
///
/// Backed by `ContentView`'s own `errorBanner` now rather than one client's
/// `lastError` — see that property's doc comment for why a single client's
/// field stopped being able to answer "what should this banner say" once
/// there was more than one client to have said it.
private struct ErrorBanner: View {
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
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.primary.opacity(0.08)))
            .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .transition(.opacity.combined(with: .move(edge: .top)))
        }
    }
}
