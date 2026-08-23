import AgentKit
import AppKit
import SwiftUI

struct ContentView: View {
    @StateObject private var store = FleetStore()
    @ObservedObject private var preferences = Preferences.shared
    @ObservedObject private var themes = Themes.shared
    @Environment(\.openSettings) private var openSettings
    @State private var selection: Selection?
    @State private var expanded: Set<String> = []
    /// Which projects have their hidden worktrees showing. Collapsed is the
    /// point of hiding, so absence means collapsed.
    @State private var hiddenExpanded: Set<String> = []

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
    private struct NewWorkspaceIntent: Identifiable {
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

    @State private var newWorkspace: NewWorkspaceIntent?
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
    @State private var showAddRepository = false
    @State private var showAdd = false
    @State private var showShortcuts = false
    @State private var showAbout = false
    @State private var query = ""
    @State private var showQuickCreate = false
    @AppStorage("tasks.lastProject") private var lastProject = ""
    /// The terminal that was open when the app last closed, as
    /// `host/workspace/terminal`.
    ///
    /// Reopening somewhere else is a small thing that costs a real one: the
    /// pane you were reading is the reason you came back, and finding it again
    /// means walking a sidebar you had already navigated once. The host is part
    /// of the key now, same as `Selection`: a workspace id alone does not say
    /// which runner to look for it on, and it does not need to — full ids are
    /// unique per runner already — but resolving it needs the client, and the
    /// client is looked up by host.
    @AppStorage("fleet.lastTerminal") private var lastTerminal = ""
    @FocusState private var searchFocused: Bool
    @State private var removeWorkspace: Workspace?
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
    /// `removeWorkspace` does, rather than a banner: this refusal has an
    /// answer ("cancel it anyway?") a banner cannot offer.
    @State private var pendingPaneModeSwitch: PaneModeConfirmation?
    /// What an editor said when it would not start.
    ///
    /// Its own state rather than routed through `errorBanner`, which is
    /// rendered only by `fleetPlaceholder`'s error branch and by the general
    /// banner below — the opposite of when this control exists. See
    /// `EditorErrorBanner`.
    @State private var editorError: String?
    /// Terminals with a `terminal seen` call already in flight. See
    /// `markVisibleSeen`.
    @State private var markingSeen: Set<String> = []
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

    /// What the detail pane is showing.
    ///
    /// Carries the host alongside the id. A workspace's own id is a full
    /// per-daemon UUID and is not expected to collide across runners, but
    /// resolving a selection means finding both the workspace AND the client
    /// that owns it, and `FleetStore.client(for:)` — deliberately, see its own
    /// doc comment — never routes by id alone. Matching on host as well here
    /// keeps that same rule rather than leaning on id uniqueness as the only
    /// thing standing between a click and the right runner.
    enum Selection: Hashable {
        case workspace(host: String, id: String)
        case terminal(host: String, workspace: String, terminal: String)
    }

    /// What confirming a pane-mode switch would do, and to which pane.
    struct PaneModeConfirmation: Identifiable {
        let id = UUID()
        /// Which runner `terminal` is on — routing is by workspace, not by
        /// the client that was current when the confirmation was raised,
        /// because by the time someone answers the sheet that may no longer
        /// be the same runner.
        let workspace: Workspace
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
                .openInEditorToolbar(workspace: detailWorkspace) { editorError = $0 }
                .toolbar {
                    // Not offered on a runner that has already said it cannot
                    // read changes at all. Its daemon predates the whole
                    // feature, so the split would open a pane running a
                    // subcommand that runner has never heard of — a dead pane
                    // where a diff was asked for, with nothing saying why.
                    if let ws = detailWorkspace,
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
                                    ? "Show what this workspace changed, in a pane"
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
            selectFirstRunningTerminal()
        }
        .onDisappear {
            for client in store.clients.values { client.stopEvents() }
        }
        // Recorded as it changes rather than on quit: an app that is force
        // quit, crashes, or is killed by a rebuild never gets a last word, and
        // this is exactly the state worth surviving all three.
        .onChange(of: selection) { _, now in
            if case let .terminal(host, workspace, terminal) = now {
                lastTerminal = "\(host)/\(workspace)/\(terminal)"
            }
        }
        .onCommand { command in run(command) }
        .onTileCommand { command in Task { await tile(command) } }
        .onSelectIndex { index in selectTerminal(at: index) }
        .onChange(of: store.layouts) { _, _ in followLayoutFocus() }
        .onChange(of: store.fleet) { _, _ in
            // One rule for every way a terminal can disappear: exiting on its
            // own, being closed here, being closed from a phone, or its
            // workspace being hidden. Hooking each path separately meant the
            // common one — you press Ctrl-D in the terminal you are looking at —
            // left the selection pointing at something that no longer existed.
            healSelection()
            // An agent finishing under your nose is a fleet change and nothing
            // else — no click, no selection change — so this is the only hook
            // that can catch the case where you were already watching it.
            markVisibleSeen()
            // A runner's first read can land well after this view's own
            // `.task` already ran and found nothing to select — `FleetStore`
            // brings every client up in the background, on its own schedule.
            // Guarded on `selection == nil` inside, so this is a no-op once
            // something is already selected.
            selectFirstRunningTerminal()
        }
        // Coming back to the app is reading whatever it comes back to. The
        // notification did its job while you were away; leaving the row lit
        // afterwards makes you dismiss the same news twice.
        .onReceive(
            NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            markVisibleSeen()
        }
        .onChange(of: selection) { _, new in
            // Cleared on every navigation, so a refusal or failure left behind
            // on one pane does not go on describing a pane the user is no
            // longer looking at. Selection is the one thing every navigation
            // path — sidebar click, ⌘P, ⌘], ⌃B o, closing a terminal — funnels
            // through, which makes it the narrowest point that sees every one
            // of them.
            errorBanner = nil

            // Selecting a pane focuses it, which is also what switches to its
            // layout. Done here rather than in `detail`, because it is a write and
            // a view must not perform one while it is being evaluated.
            //
            // One call, not two. It used to activate the group and stop there,
            // which put the layout on screen and left its REMEMBERED focus in
            // charge — so clicking Terminal 6 landed you on whichever pane of that
            // layout you had used last. `⌘P` had the same fault for the same
            // reason: both set a selection and let the layout overrule it.
            if case .terminal(let host, let wsID, let termID) = new,
                let workspace = workspace(host: host, id: wsID),
                let c = store.client(for: workspace),
                let holder = c.group(holding: termID, in: wsID),
                let pane = holder.pane(termID),
                !holder.isActive || !pane.focused
            {
                Task {
                    await act(on: workspace) { client in
                        await client.focusPane(pane.short, in: workspace)
                    }
                }
            }

            if case .terminal(_, _, let id) = new {
                // Stamped here rather than in the palette, so every way of
                // arriving counts: a sidebar click, ⌘], ⌃B o, a jump from the
                // palette itself. A switcher that only learned from its own
                // choices would order by where you had used IT, not by where you
                // have been.
                VisitLog.shared.visited(id)
            }

            // Opening a terminal is what ends `done`. Being LISTED is still not
            // being read — the sidebar shows every terminal on the runner and
            // clearing a notification nobody read is worse than not sending one
            // — but being on screen is, which is more than the pane you clicked.
            markVisibleSeen()
        }
        .sheet(isPresented: $showShortcuts) { ShortcutsSheet() }
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
                    onSubmit: { description, host, project, preset in
                        lastProject = project
                        startTask(description: description, host: host, project: project, agent: preset)
                    },
                    onResume: {
                        showQuickCreate = false
                        showResumeBranch = true
                    },
                    onClose: { showQuickCreate = false },
                    branchPrefix: { host in store.clients[host]?.fleet.branchPrefix ?? "" }
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
                        workspaces: store.fleet.workspaces,
                        current: selection,
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
        .sheet(item: $newWorkspace) { intent in
            NewWorkspaceSheet(
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
                let created = await client.createWorkspace(
                    repo: repo, task: task, branch: branch, base: base)
                if let failure = created.failure { return failure }
                // Land in the terminal it came up with, exactly as starting a
                // task does. Creating a worktree is not a filing act — you make
                // one because you are about to work in it — and `reveal` is the
                // one place that says what "go to it" means: expand the
                // workspace in the sidebar, then select its terminal.
                reveal(created.workspace)
                return nil
            }
        }
        .sheet(item: $removeWorkspace) { ws in
            RemoveWorkspaceSheet(
                workspace: ws,
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
                if case .ok = result, case .terminal(let h, let w, _) = selection,
                    h == (ws.host ?? ""), w == ws.id
                {
                    selection = nil
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
        .sheet(item: $pendingPaneModeSwitch) { pending in
            PaneModeConfirmSheet(message: pending.message) {
                await act(
                    on: pending.workspace, default: .failed("This runner can’t be reached right now.")
                ) { c in
                    await c.setPaneMode(pending.terminal, mode: pending.mode, force: true)
                }
            }
        }
    }

    // MARK: - Sidebar

    /// Worktrees matching the search, grouped by runner and project.
    ///
    /// The key carries the host because two runners can have a project of the
    /// same name, and they are not the same project. The host is only DISPLAYED
    /// when there is more than one runner — on a fleet of one, saying which
    /// runner is noise.
    ///
    /// Hidden worktrees are separated rather than filtered out: they still
    /// belong to the project, and a collapsed section at the bottom is how you
    /// get back to one.
    ///
    /// A struct with a stable `id`, not a bare tuple keyed by array position:
    /// `ForEach` used to key this list by `.offset`, so inserting a group
    /// renumbered every header after it and any transient `@State` (row
    /// hovering) followed the index instead of following the row it belonged
    /// to. `groupKey(host:project:)` was already computed for `hiddenExpanded`
    /// three lines below — `id` is the same value, just attached to the
    /// element itself so `ForEach` can use it too.
    private struct ProjectGroup: Identifiable {
        let host: String
        let project: String
        let shown: [Workspace]
        let hidden: [Workspace]
        var id: String { "\(host)\u{1}\(project)" }
    }

    /// A project header's repository, on its way to `RemoveRepositorySheet`.
    /// `.sheet(item:)` needs `Identifiable`; a bare tuple is not one.
    private struct RepositoryToRemove: Identifiable {
        let host: String
        let repository: Repository
        var id: String { "\(host)\u{1}\(repository.id)" }
    }

    private var groups: [ProjectGroup] {
        let visible = store.fleet.workspaces.filter { $0.matches(query) }
        var order: [String] = []
        var byKey: [String: [Workspace]] = [:]

        for workspace in visible {
            let host = workspace.host ?? ""
            let project = (workspace.repository ?? "").isEmpty
                ? "Ungrouped" : workspace.repository!
            let key = "\(host)\u{1}\(project)"
            if byKey[key] == nil { order.append(key) }
            byKey[key, default: []].append(workspace)
        }

        var result: [ProjectGroup] = order.map { key in
            let parts = key.split(separator: "\u{1}", maxSplits: 1, omittingEmptySubsequences: false)
            let host = String(parts[0])
            let project = String(parts.count > 1 ? parts[1] : "")
            let all = byKey[key] ?? []
            // Main checkout first, then the daemon's order. A stable partition,
            // not a sort: sorted(by:) is not guaranteed stable and rows would
            // reshuffle on every fleet event.
            let shown = all.filter { !$0.isHidden && $0.isMainCheckout }
                + all.filter { !$0.isHidden && !$0.isMainCheckout }
            return ProjectGroup(host: host, project: project, shown: shown, hidden: all.filter(\.isHidden))
        }

        // A runner that has never connected has no rows of its own, and
        // without this it would simply be missing — leaving you to wonder
        // where it went rather than seeing that it needs attention. Skipped
        // while searching: a runner with nothing on it can never match a
        // query, and a header appearing only here would look like a hit.
        if query.isEmpty {
            for host in silentHosts {
                result.append(ProjectGroup(host: host, project: "", shown: [], hidden: []))
            }
        }

        return result
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
        let present = Set(store.fleet.workspaces.map { $0.host ?? "" })
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

    /// A group's identity for `hiddenExpanded`'s key.
    ///
    /// Two runners can have a project of the same name, and a lone project
    /// name is no longer unique on its own now that the group carries the host
    /// separately — so `hiddenExpanded`, keyed by this rather than by
    /// `project` alone, cannot conflate "hidden expanded on this runner" with
    /// "hidden expanded on that one." Same value as `ProjectGroup.id` above.
    private func groupKey(host: String, project: String) -> String {
        "\(host)\u{1}\(project)"
    }

    /// The repository a project header's name and host actually stand for.
    ///
    /// `ProjectGroup.project` is a display name, not an id — see
    /// `groups`'s own use of `workspace.repository` — so removing it needs
    /// this lookup rather than something already in hand.
    private func repository(host: String, project: String) -> Repository? {
        store.repositories.first { $0.host == host && $0.repository.displayName == project }?
            .repository
    }

    /// Start a worktree in a named project.
    ///
    /// The sheet already has a repository picker; this just answers it in
    /// advance, because someone clicking `+` on a project header has already
    /// said which one.
    private func newWorktree(host: String, project: String) {
        newWorkspace = NewWorkspaceIntent(project: project, host: host)
    }

    /// A terminal in the repository's own checkout.
    ///
    /// The main checkout is always present in the fleet — the daemon adopts it
    /// the moment a repository is registered — so this finds it rather than
    /// asking the CLI to produce or locate it.
    private func newMainTerminal(host: String, project: String) async {
        guard
            let workspace = store.fleet.workspaces.first(where: {
                $0.isMainCheckout && $0.repository == project && ($0.host ?? "") == host
            })
        else { return }
        await act(on: workspace) { client in
            await client.createTerminal(workspace: workspace.short, preset: "shell", title: "")
            await client.refresh()
        }
    }

    /// A plain, non-optional entry point into `newMainTerminal(host:project:)`.
    ///
    /// `ProjectHeader.onNewTerminal` is optional — nil for a silent host's
    /// placeholder — and a ternary handing back `Task { await ... }` directly
    /// for the non-nil branch leaves the compiler unable to settle on which
    /// `Task.init` overload the closure means, reported as an unhelpful
    /// "ambiguous use of 'init(name:priority:operation:)'" with no line
    /// pointing at the ternary itself. A named function sidesteps it.
    private func startMainTerminal(host: String, project: String) {
        Task { await newMainTerminal(host: host, project: project) }
    }

    /// One project's worktrees, plus its hidden section.
    ///
    /// Lifted out of `sidebar` when projects became collapsible: the rows had to
    /// go behind an `if`, and wrapping fifty lines of view builder in one would
    /// have re-indented the whole block to say one thing. A builder method is
    /// what this file already does for the detail side — see `tiled(_:group:)`.
    @ViewBuilder
    private func projectRows(_ group: ProjectGroup, key: String, usable: Bool) -> some View {
        ForEach(group.shown) { ws in
            workspaceRow(ws, usable: usable)
        }
        if !group.hidden.isEmpty {
            HiddenWorktrees(
                project: key,
                worktrees: group.hidden,
                isExpanded: hiddenExpanded.contains(key),
                onToggle: {
                    if hiddenExpanded.contains(key) {
                        hiddenExpanded.remove(key)
                    } else {
                        hiddenExpanded.insert(key)
                    }
                },
                onUnhide: { ws in
                    Task { await act(on: ws) { c in await c.unhideWorkspace(ws.short) } }
                }
            )
        }
    }

    private var sidebar: some View {
        VStack(spacing: 0) {
            sidebarHeader
            searchField

            // Gated on `groups`, not the raw merged workspace count: a
            // runner that has never connected contributes no workspaces but
            // still gets a `silentHosts` header in `groups` (unless it's the
            // local runner, which has its own placeholder below). Gating on
            // `workspaces.isEmpty` instead used to short-circuit straight to
            // the LOCAL runner's empty state whenever the merged fleet had
            // no rows — even with a remote runner configured and its header
            // sitting in `groups` right below — making that remote runner
            // vanish from the sidebar entirely rather than showing as
            // unreachable. `query.isEmpty` keeps this from swallowing a
            // plain "no search results" into the same screen.
            if groups.isEmpty && query.isEmpty {
                fleetPlaceholder
                Spacer(minLength: 0)
            } else if groups.isEmpty {
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
                        ForEach(groups) { group in
                            let key = groupKey(host: group.host, project: group.project)
                            // A silent host's placeholder has no project of its
                            // own to name or add into — the header names the
                            // runner instead, and there is nothing yet to
                            // route a `+` to.
                            let isSilentHost = group.project.isEmpty
                            let usable = store.refusal(for: group.host) == nil
                            ProjectHeader(
                                name: isSilentHost
                                    ? (group.host.isEmpty ? "This Mac" : group.host) : group.project,
                                count: group.shown.count,
                                onNewWorktree: isSilentHost
                                    ? nil : { newWorktree(host: group.host, project: group.project) },
                                onNewTerminal: isSilentHost
                                    ? nil
                                    : { startMainTerminal(host: group.host, project: group.project) },
                                onRemove: isSilentHost
                                    ? nil
                                    : {
                                        guard
                                            let repo = repository(
                                                host: group.host, project: group.project)
                                        else { return }
                                        removeRepository = RepositoryToRemove(
                                            host: group.host, repository: repo)
                                    },
                                host: group.host,
                                hostState: store.state(of: group.host),
                                daemonUpdate: daemonUpdate(for: group.host),
                                showHost: isSilentHost ? false : showHosts,
                                onReconnect: { store.reconnect(group.host) },
                                isCollapsed: preferences.isProjectCollapsed(key),
                                // A silent host's header has no worktrees under
                                // it, so there is nothing for a chevron to do.
                                onToggleCollapse: isSilentHost
                                    ? nil : { preferences.toggleProject(key) }
                            )
                            // Everything under the header, which is everything a
                            // collapsed project hides.
                            if !preferences.isProjectCollapsed(key) {
                                projectRows(group, key: key, usable: usable)
                            }
                        }
                    }
                    .padding(.bottom, 10)
                }
            }

            statusBar
        }
        .background(WorkspaceStyle.sidebar)
        // Declared in exactly one place. A second declaration on the
        // `NavigationSplitView`'s sidebar closure made which width the column
        // settled on nondeterministic, and the window drifted with it.
        .navigationSplitViewColumnWidth(min: 268, ideal: 320, max: 440)
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
            TextField("Search workspaces and agents", text: $query)
                .textFieldStyle(.plain)
                .font(.system(size: 12.5))
                .focused($searchFocused)
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

    /// Everything waiting on you, across every project and every runner.
    ///
    /// The one number the app exists to produce, and it deliberately spans
    /// hosts: an agent blocked on a runner in another room is exactly as
    /// urgent as one on this desk.
    private var attentionWaiting: [Status] {
        store.fleet.workspaces.flatMap(\.terminals).map(\.status).filter(\.wantsAttention)
    }

    private var sidebarHeader: some View {
        SidebarRow {
            HStack(spacing: 8) {
            // Just the word, now. It used to be the one runner being driven,
            // because the sidebar was that runner's workspaces and the pane
            // could not otherwise say whose. Now it is every runner's at
            // once, and each row already names its own runner below, so a
            // header naming one would be naming the wrong thing, or picking
            // a favorite among rows that are not ranked.
            Text("Fleet")
                .font(.system(size: 15, weight: .semibold))
            if store.clients.values.contains(where: \.busy) { ProgressView().controlSize(.mini) }

            Spacer()

            // With the actions, not beside the title. It is a button — it jumps
            // to the next agent waiting — and sitting it against the runner
            // name read as part of the name, which is the one thing it is not.
            if !attentionWaiting.isEmpty {
                // A dot and a number, not a filled capsule. A solid block of
                // color in the header shouts louder than the row it points at,
                // which leaves it pointing at itself.
                Button {
                    AppCommand.nextAttention.post()
                } label: {
                    AttentionBadge(waiting: attentionWaiting)
                }
                .buttonStyle(.plain)
                .help("\(attentionWaiting.count) waiting on you — click to jump there")
            }
            // A menu rather than a button, because "add a repository" has to be
            // reachable at all times. It used to live only in the empty state,
            // so once you had one workspace there was no way to add a second
            // repository without dropping to the terminal.
            SidebarMenuButton(
                systemImage: "plus",
                help: "Add a workspace, a repository, or a runner",
                items: [
                    SidebarMenuItem(title: "New Workspace…") {
                        newWorkspace = NewWorkspaceIntent()
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
    /// Telling someone they have no workspaces when the truth is that we could
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
            Text("No workspaces").font(.callout.weight(.medium))
            Text("A workspace is one worktree and branch for one task.")
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
                Button("New Workspace") {
                    newWorkspace = NewWorkspaceIntent()
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
                Circle()
                    .fill(store.fleet.runtimeHealthy ? Color.green : Color.orange)
                    .frame(width: 7, height: 7)
                Text(
                    store.fleet.runtimeHealthy
                        ? "\(store.fleet.livePanes) live"
                        : "tmux unavailable"
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                // The dot above is an OR across every runner, deliberately —
                // ANDing would paint the bar orange every time one laptop was
                // merely asleep. But on a fleet of more than one, an OR alone
                // means it takes just ONE healthy runner to turn that dot
                // green while a second sits there unreachable or without
                // tmux at all, invisibly. These name that runner instead of
                // letting the merged dot speak for it; a click retries it at
                // once, same as a header's own dot.
                //
                // Built by hand here rather than calling `HostDot`, and
                // deliberately so, not as an oversight: `HostDot` draws
                // `EmptyView()` for `.connected`, which is right for a
                // header naming only whether the RUNNER answers, but wrong
                // here — a runner can be fully reachable and still be the
                // reason this row reads orange (reachable, no tmux), and
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
    /// and still be the reason the fleet's tmux status reads orange, and that
    /// case has to draw something here.
    private func troubleColor(for host: String) -> Color {
        switch store.state(of: host) {
        case .unreachable: return .red
        case .notInstalled: return .secondary
        case .connecting, .connected, .reconnecting: return .orange
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
    private func workspaceRow(_ ws: Workspace, usable: Bool) -> some View {
        let client = store.client(for: ws)
        let tiled = Set(client?.activeGroup(ws.id)?.terminals ?? [])
        return WorkspaceSection(
            workspace: ws,
            isExpanded: expanded.contains(ws.id),
            selection: $selection,
            onToggle: { toggle(ws.id) },
            onNewTerminal: { newTerminal(in: ws) },
            onHide: {
                Task { await act(on: ws) { c in await c.hideWorkspace(ws.short) } }
            },
            onUnhide: {
                Task { await act(on: ws) { c in await c.unhideWorkspace(ws.short) } }
            },
            onRemove: { removeWorkspace = ws },
            onTerminalAction: { term, action in
                Task { await run(action, on: term, in: ws) }
            },
            layouts: client?.layouts[ws.id] ?? [],
            onMoveToLayout: { term, group in
                moveToLayout(term, in: ws, group: group)
            },
            onDropTogether: { dragged, onto in
                placePane(dragged, onto: onto.id, side: .right, in: ws)
            },
            tiled: tiled,
            onEditorError: { editorError = $0 },
            usable: usable,
            changes: changesStatus(ws),
            countsWidth: countsWidth
        )
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
    private func changesStatus(_ ws: Workspace) -> InboxRow? {
        guard let client = store.client(for: ws) else { return nil }
        return client.changesInbox[ws.short]
    }

    /// This worktree's changes pane, if it has one open.
    ///
    /// Asked of the ACTIVE layout rather than of the workspace's terminals,
    /// because that is the question the toolbar button is answering: whether
    /// the arrangement you are looking at is showing the diff. A changes pane
    /// in a layout two tabs over is not on screen, and offering to close it
    /// from here would close something the window is not showing.
    private func changesPane(in ws: Workspace) -> Terminal? {
        guard let group = store.client(for: ws)?.activeGroup(ws.id) else { return nil }
        let inGroup = Set(group.panes.map(\.id))
        return ws.terminals.first { inGroup.contains($0.id) && $0.isChangesPane }
    }

    /// Open this worktree's diff beside what is focused, or close the one that
    /// is already open.
    ///
    /// A split of the focused pane, exactly as `⌃B %` and a drop on an edge
    /// are: the daemon has one verb for "a new pane, here, running this", and a
    /// changes pane is that verb with a different preset. Nothing new had to be
    /// taught about layouts to put a diff into one.
    private func toggleChangesPane(in ws: Workspace) {
        if let open = changesPane(in: ws) {
            // Killing the pane is the whole of it. The record goes with it, but
            // the daemon does that — closing a diff from tmux's own `⌃B x` has
            // to leave as little behind as closing it from here, so the reaping
            // lives on the host where both can reach it, not in this button.
            Task { await act(on: ws) { c in await c.stop(terminal: open.short) } }
            return
        }
        Task {
            let groups = await act(on: ws, default: []) { c in
                // `beside: nil` means the focused pane, which is the daemon's
                // own default and the same anchor `⌃B %` uses.
                await c.split(ws, beside: nil, side: .right, preset: "changes")
            }
            reveal(groups, in: ws)
        }
    }

    /// One changes store per worktree.
    ///
    /// Cached on the client too, not just the worktree. `FleetStore` drops a
    /// `DaemonClient` when its runner leaves and builds a fresh one when it
    /// comes back, and a store held over from the old one would go on talking to
    /// a connection nobody is answering.
    private func changesStore(for ws: Workspace, client: DaemonClient) -> ChangesStore {
        if let existing = changesStores[ws.id], existing.client === client { return existing }
        let made = ChangesStore(client: client, workspace: ws)
        // Assigned outside the view update, because creating it IS a state
        // change and SwiftUI is reading that state right now. An earlier version
        // wrote it from a Task, which rebuilt the store on every render and threw
        // away each load before it could finish — the panel sat permanently empty.
        DispatchQueue.main.async { changesStores[ws.id] = made }
        return made
    }

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .terminal(let host, let wsID, let termID):
            // Every terminal is in a layout now — it IS a tmux window, and a
            // window IS a layout — so the second branch is reached only in the
            // seconds before the first `layout show` comes back, and after that
            // never. It is kept because "we have not read the layouts yet" and
            // "this terminal is in no layout" look identical from here, and
            // showing the terminal is the right answer to both.
            //
            // The condition deliberately asks whether the terminal is in ANY
            // layout rather than in the ACTIVE one. It used to ask the latter, so
            // selecting a pane belonging to a different layout showed it on its
            // own and the view bounced between arrangement and single terminal as
            // you clicked down the sidebar.
            if let ws = workspace(host: host, id: wsID),
                let c = store.client(for: ws), c.group(holding: termID, in: wsID) != nil,
                let group = c.activeGroup(wsID)
            {
                tiled(ws, client: c, group: group)
            } else if let ws = workspace(host: host, id: wsID),
                let term = ws.terminals.first(where: { $0.id == termID })
            {
                TerminalPane(
                    terminal: term,
                    workspace: ws,
                    binary: store.client(for: ws)?.cliPath,
                    environment: store.client(for: ws)?.cliEnvironment ?? [:],
                    hostArguments: store.client(for: ws)?.cliHostArguments ?? [],
                    linkGeneration: store.client(for: ws)?.linkGeneration ?? 0,
                    refusal: { store.refusal(for: ws) },
                    onGeometry: { cols, rows in
                        // Not routed through `act(on:_:)`, deliberately: this
                        // fires from window and pane geometry, not a click —
                        // the same reasoning `markVisibleSeen()` documents
                        // above. Selecting a row on a sleeping machine is
                        // exactly what the design wants readable, and a
                        // window resize while it's selected must not put
                        // "Cannot do that" over a pane nobody touched.
                        await store.client(for: ws)?.resize(
                            terminal: term.short, columns: cols, rows: rows)
                    },
                    onSearchFiles: { query in
                        await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? []
                    },
                    onAction: { action in Task { await run(action, on: term, in: ws) } }
                )
            } else {
                placeholder
            }

        case .workspace(let host, let wsID):
            // A worktree with a layout shows the layout. The card list was the
            // right answer when a workspace had no arrangement of its own; once
            // it does, showing a summary of the panes instead of the panes is a
            // click in the way.
            if let ws = workspace(host: host, id: wsID),
                let c = store.client(for: ws),
                let group = c.activeGroup(wsID),
                !group.terminals.isEmpty
            {
                tiled(ws, client: c, group: group)
            } else if let ws = workspace(host: host, id: wsID) {
                WorkspaceDetail(
                    workspace: ws,
                    onNewTerminal: { newTerminal(in: ws) },
                    onHide: { Task { await act(on: ws) { c in await c.hideWorkspace(ws.short) } } },
                    onUnhide: { Task { await act(on: ws) { c in await c.unhideWorkspace(ws.short) } } },
                    onRemove: { removeWorkspace = ws },
                    onOpenTerminal: { t in
                        selection = .terminal(host: host, workspace: ws.id, terminal: t.id)
                    }
                )
            } else {
                placeholder
            }

        case nil:
            placeholder
        }
    }

    /// The layout, wired up.
    ///
    /// One builder for both selections. Selecting a pane and selecting the
    /// worktree it is in put the same view on screen with the same six callbacks,
    /// and while they were written out twice they drifted: the drag handler was
    /// fixed in one copy and not the other, so dropping a pane behaved differently
    /// depending on which sidebar row you had clicked last.
    private func tiled(_ ws: Workspace, client: DaemonClient, group: PaneGroup) -> some View {
        TileView(
            groups: client.layouts[ws.id] ?? [group],
            workspace: ws,
            changes: changesStore(for: ws, client: client),
            binary: store.client(for: ws)?.cliPath,
            environment: store.client(for: ws)?.cliEnvironment ?? [:],
            hostArguments: store.client(for: ws)?.cliHostArguments ?? [],
            linkGeneration: store.client(for: ws)?.linkGeneration ?? 0,
            refusal: { store.refusal(for: ws) },
            onFocus: { id in
                selection = .terminal(host: ws.host ?? "", workspace: ws.id, terminal: id)
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
            onViewport: { columns, rows in
                // Not routed through `act(on:_:)` — see `onGeometry`'s
                // comment above: this fires from pane geometry, not a click.
                await store.client(for: ws)?.viewport(columns: columns, rows: rows, in: ws)
            },
            onResizeDivider: { terminal, side, cells in
                resizeDivider(terminal, side: side, cells: cells, in: ws)
            },
            onSearchFiles: { query in await store.client(for: ws)?.searchFiles(in: ws, query: query) ?? [] },
            onSwitchPaneMode: { terminal in Task { await togglePaneMode(terminal, in: ws) } }
        )
    }

    private var placeholder: some View {
        ContentUnavailableView {
            Label("Select a workspace", systemImage: "rectangle.split.3x1")
        } description: {
            Text("Each workspace is one worktree and branch for one task.")
        } actions: {
            if !store.repositories.isEmpty {
                Button("New Workspace") {
                    newWorkspace = NewWorkspaceIntent()
                }
            }
        }
    }

    // MARK: - Routing

    /// A repository to default the project picker to, when nothing was
    /// chosen yet — the empty state's "New Workspace" button, the sidebar's
    /// own `+`, and the palette's "New Task" all reach this with no project
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

    /// Route a mutation to the runner a workspace is on, refusing it first.
    ///
    /// Checked here rather than at each call site — see `FleetStore.refusal(for:)`
    /// for why. On refusal, `fallback` is handed back and nothing is called; on
    /// success, whatever the client itself left in `lastError` is surfaced too,
    /// which is how a command that reached its runner and failed there still
    /// reaches the banner.
    @discardableResult
    private func act<T>(
        on ws: Workspace, default fallback: T, _ body: (DaemonClient) async -> T
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

    private func act(on ws: Workspace, _ body: (DaemonClient) async -> Void) async {
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
            if client.fleet.workspaces.contains(where: { $0.terminals.contains { $0.short == short } }) {
                return await client.screen(terminal: short)
            }
        }
        return ""
    }

    // MARK: - Behavior

    private func run(_ action: TerminalAction, on term: Terminal, in workspace: Workspace) async {
        switch action {
        case .restart: await act(on: workspace) { c in await c.restart(terminal: term.short) }
        case .dismissLost: await act(on: workspace) { c in await c.dismissLost(term) }
        case .stop: await act(on: workspace) { c in await c.stop(terminal: term.short) }
        }
    }

    private func workspace(host: String, id: String) -> Workspace? {
        store.fleet.workspaces.first { ($0.host ?? "") == host && $0.id == id }
    }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }

    /// Land on something usable rather than an empty pane.
    /// Where to land on launch.
    ///
    /// Whatever wants you first, wherever it is — including on another runner.
    /// Falling back to "the first running terminal" would open a fleet on
    /// something arbitrary while an agent two projects down waits for an answer.
    private func selectFirstRunningTerminal() {
        guard selection == nil else { return }

        // Where you left off, if it is still there. A terminal that has since
        // exited falls through to the rules below rather than selecting
        // nothing — the saved id is a preference, not a promise.
        //
        // Split with empty subsequences kept: the host component is empty for
        // this Mac, so the saved string starts with "/" and the default
        // omitting behavior would swallow that leading empty piece and shift
        // everything over by one.
        let saved = lastTerminal
            .split(separator: "/", maxSplits: 2, omittingEmptySubsequences: false)
            .map(String.init)
        if saved.count == 3,
            let ws = store.fleet.workspaces.first(where: {
                ($0.host ?? "") == saved[0] && $0.id == saved[1]
            }),
            ws.terminals.contains(where: { $0.id == saved[2] })
        {
            expanded.insert(ws.id)
            selection = .terminal(host: saved[0], workspace: ws.id, terminal: saved[2])
            return
        }

        for ws in store.fleet.workspaces {
            if let t = ws.terminals.first(where: { $0.status.wantsAttention }) {
                expanded.insert(ws.id)
                selection = .terminal(host: ws.host ?? "", workspace: ws.id, terminal: t.id)
                return
            }
        }
        for ws in store.fleet.workspaces {
            if let t = ws.terminals.first(where: { StateKind.parse($0.state) == .running }) {
                expanded.insert(ws.id)
                selection = .terminal(host: ws.host ?? "", workspace: ws.id, terminal: t.id)
                return
            }
        }
        if let ws = store.fleet.workspaces.first {
            selection = .workspace(host: ws.host ?? "", id: ws.id)
        }
    }

    // MARK: - Commands

    /// Every terminal in the fleet, in the order the sidebar shows them.
    ///
    /// One definition, so ⌘1 and a click select the same thing and ⌘] walks the
    /// list a user can actually see.
    private var allTerminals: [Terminal] {
        store.fleet.workspaces.flatMap(\.terminals)
    }

    private var selectedTerminal: (workspace: Workspace, terminal: Terminal)? {
        guard case .terminal(let host, let workspaceID, let terminalID) = selection,
            let workspace = store.fleet.workspaces.first(where: {
                ($0.host ?? "") == host && $0.id == workspaceID
            }),
            let terminal = workspace.terminals.first(where: { $0.id == terminalID })
        else { return nil }
        return (workspace, terminal)
    }

    // MARK: - Attention

    /// Which terminals the detail pane is actually putting in front of you.
    ///
    /// Not just the selected one. Selecting a pane shows the whole layout it
    /// belongs to, and a terminal tiled beside the one with focus is as much on
    /// screen as the one with focus — reading it took no extra click, so it
    /// cannot go on asking for one.
    ///
    /// The branches mirror `detail` exactly, including its quirk that selecting
    /// a pane in a non-active layout shows the ACTIVE one. Anything else would
    /// have this marking terminals read that are not on screen, which is the one
    /// mistake worse than the bug it fixes.
    private var visibleTerminals: [Terminal] {
        switch selection {
        case .terminal(let host, let workspaceID, let terminalID):
            guard let ws = workspace(host: host, id: workspaceID) else { return [] }
            if let c = store.client(for: ws), c.group(holding: terminalID, in: workspaceID) != nil,
                let group = c.activeGroup(workspaceID)
            {
                return ws.terminals.filter { group.terminals.contains($0.id) }
            }
            return ws.terminals.filter { $0.id == terminalID }

        case .workspace(let host, let workspaceID):
            guard let ws = workspace(host: host, id: workspaceID),
                let group = store.client(for: ws)?.activeGroup(workspaceID),
                !group.terminals.isEmpty
            else { return [] }
            return ws.terminals.filter { group.terminals.contains($0.id) }

        case nil:
            return []
        }
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
    /// others must not quietly mark it read.
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
        guard NSApp.isActive else { return }
        let client = detailWorkspace.flatMap { store.client(for: $0) }
        // Full ids, not `short`: resolving an abbreviation costs the CLI a
        // fleet listing, and this runs on a clock. See the `Watching` command
        // in `crates/cli/src/main.rs`.
        client?.reportWatching(visibleTerminals.map(\.id))
        // And every OTHER runner is told it is showing nothing. A window puts
        // exactly one runner's workspace in the detail pane, so switching from a
        // pane on one runner to a pane on another would otherwise leave the
        // first still believing its pane is being watched — silent for as long
        // as the claim takes to age out, on the runner you just walked away
        // from. Every runner, and outside the guard below, because selecting
        // NOTHING is a way of walking away too: `visibleTerminals` is empty
        // then, and so is the claim every runner should be holding.
        for other in store.clients.values where other !== client {
            other.reportWatching([])
        }
        guard let client else { return }
        for terminal in visibleTerminals where terminal.agent == .done {
            // The daemon is idempotent, but this is not free: each call is a
            // CLI subprocess, and a fleet event arrives for every terminal on
            // the runner. Without this, ten busy panes would mean ten
            // redundant processes for every one that finished.
            guard !markingSeen.contains(terminal.id) else { continue }
            markingSeen.insert(terminal.id)
            Task {
                await client.markSeen(terminal.short)
                markingSeen.remove(terminal.id)
            }
        }
    }

    // MARK: - Tiling

    /// The workspace a tiling keystroke acts on.
    private var tileTarget: Workspace? { currentWorkspace }

    /// Carry out a `⌃B`-prefixed command.
    ///
    /// Every one of them is a single `layout` call whose reply is the workspace's
    /// whole layout, so there is nothing to reconcile here: tmux decides what a
    /// zoom or a split means, and it decides the same way for this app, for the
    /// CLI and for an agent driving the CLI.
    ///
    /// This file no longer contributes geometry. Directional focus used to be
    /// worked out here from a recomputed arrangement; it is now read off the
    /// rectangles tmux reported, which is the only copy.
    private func tile(_ command: TileCommand) async {
        guard let workspace = tileTarget else { return }
        let group = store.client(for: workspace)?.activeGroup(workspace.id)
        /// The pane a keystroke acts on: the selected one, else whatever tmux says
        /// is focused.
        let here: PaneRect? = {
            if case .terminal(_, _, let id) = selection, let pane = group?.pane(id) { return pane }
            return group?.panes.first(where: \.focused)
        }()

        switch command {
        case .zoom:
            await act(on: workspace) { c in await c.zoomPane(nil, in: workspace) }

        case .focusNext:
            await act(on: workspace) { c in await c.focusPane(step: "--next", in: workspace) }
        case .focusPrevious:
            await act(on: workspace) { c in await c.focusPane(step: "--prev", in: workspace) }

        case .focus(let direction):
            guard let group, let from = here,
                let next = group.neighbour(of: from.id, direction)
            else { return }
            await act(on: workspace) { c in await c.focusPane(next.short, in: workspace) }

        case .focusIndex(let n):
            await act(on: workspace) { c in await c.focusPane(number: n, in: workspace) }

        case .cycle:
            await act(on: workspace) { c in await c.cycleLayout(workspace) }

        case .preset(let preset):
            await act(on: workspace) { c in await c.applyPreset(preset, in: workspace) }

        case .evenPanes:
            // Which even arrangement, read off the panes rather than asked for.
            //
            // tmux has two — columns and rows — and picking the wrong one does
            // not "even out" a layout, it turns it inside out: a stack of three
            // becomes a row of three. So this counts how the window is already
            // split, the same way `TileView.Viewport` does, and hands back the
            // even version of the shape that is on screen.
            let group = store.client(for: workspace)?.activeGroup(workspace.id)
            let columns = Set(group?.panes.map(\.left) ?? []).count
            let rows = Set(group?.panes.map(\.top) ?? []).count
            let preset: TilePreset = columns >= rows ? .evenHorizontal : .evenVertical
            await act(on: workspace) { c in await c.applyPreset(preset, in: workspace) }

        case .splitRight, .splitDown:
            // One call. It used to be create-then-join-then-apply-a-preset, three
            // round trips whose only way of saying WHERE the new pane went was to
            // re-arrange every pane in the layout — so splitting the third pane of
            // four rebuilt the other three as well. `layout split` splits the pane
            // you name, on the side you name, and leaves the rest alone.
            let side: TileDirection = command == .splitRight ? .right : .bottom
            let groups = await act(on: workspace, default: []) { c in
                await c.split(workspace, beside: here?.short, side: side)
            }
            // Land in the pane that was just made, which is the one tmux focuses.
            reveal(groups, in: workspace)

        case .breakPane:
            guard let here else { return }
            let groups = await act(on: workspace, default: []) { c in
                await c.breakPane(here.short, in: workspace)
            }
            reveal(groups, in: workspace, preferring: here.id)

        case .closePane:
            // tmux's `x`, and it means the same thing: the pane's process ends.
            // Routed through the existing close so there is one implementation of
            // what closing a terminal does.
            run(.closeTerminal)

        case .newGroup:
            await openTerminalInNewLayout(workspace)

        case .nextGroup:
            // Landing in the new layout is the point of switching to it. Without
            // this the selection stayed on a pane from the OLD layout, which the
            // detail view then showed on its own — so ⌃B n looked like it opened a
            // random terminal and came back.
            let groups = await act(on: workspace, default: []) { c in
                await c.selectLayout("--next", in: workspace)
            }
            reveal(groups, in: workspace)
        case .previousGroup:
            let groups = await act(on: workspace, default: []) { c in
                await c.selectLayout("--prev", in: workspace)
            }
            reveal(groups, in: workspace)

        case .toggleAgentPane:
            // Falls back to the plain selection when there is no tmux group
            // yet — the few seconds `detail`'s own comment describes, before
            // the first `layout show` has come back, where `here` is nil but
            // a terminal is still very much selected.
            let target =
                here.flatMap { rect in workspace.terminals.first { $0.id == rect.id } }
                ?? selectedTerminal?.terminal
                // Selecting a LAYOUT TAB is not selecting a terminal, and a
                // cached layout does not always mark a pane focused — so both
                // of the above are nil for the commonest way of getting here,
                // and the command silently did nothing at all. A layout with
                // one pane has no ambiguity about which pane is meant.
                ?? group?.panes.first.flatMap { pane in
                    workspace.terminals.first { $0.id == pane.id }
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
            await togglePaneMode(target, in: workspace)

        case .help:
            showShortcuts = true
        }
    }

    /// Ask the daemon to flip a pane between its terminal and its agent chat.
    ///
    /// One call, and the daemon is the one deciding whether that is even
    /// possible — a client guessing "this preset can't be an agent" would be
    /// exactly the kind of state the design says clients never derive.
    private func togglePaneMode(_ terminal: Terminal, in workspace: Workspace) async {
        let target = terminal.isAgentPane ? "terminal" : "agent"
        let result = await act(on: workspace, default: DaemonClient.PaneModeResult.ok) { c in
            await c.setPaneMode(terminal.short, mode: target)
        }
        switch result {
        case .ok, .failed:
            // A failure already reached `errorBanner` via `act`, so there is
            // nothing further to do from here.
            break
        case let .confirmationRequired(message):
            pendingPaneModeSwitch = PaneModeConfirmation(
                workspace: workspace, terminal: terminal.short, mode: target, message: message)
        }
    }

    /// Send a terminal to another layout, or to one of its own.
    ///
    /// `nil` is `break-pane`: a layout with just this in it. Naming a layout moves
    /// the pane against that layout's focused pane, because "which layout" is only
    /// half an instruction — tmux has to be told which pane and which edge, and the
    /// menu has no way to ask. Dragging is how you say the other half, and the drop
    /// indicator is why that is easier than answering a dialog about it.
    private func moveToLayout(_ terminal: Terminal, in workspace: Workspace, group: PaneGroup?) {
        Task {
            let groups: [PaneGroup]
            if let group, let onto = group.panes.first(where: \.focused) ?? group.panes.first {
                groups = await act(on: workspace, default: []) { c in
                    await c.movePane(terminal.short, onto: onto.short, side: .right, in: workspace)
                }
            } else {
                groups = await act(on: workspace, default: []) { c in
                    await c.breakPane(terminal.short, in: workspace)
                }
            }
            reveal(groups, in: workspace, preferring: terminal.id)
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
        _ terminal: String, side: TileDirection, cells: Int, in workspace: Workspace
    ) -> Bool {
        guard cells != 0, !resizingDivider else { return false }
        guard let pane = store.client(for: workspace)?.group(holding: terminal, in: workspace.id)?
            .pane(terminal)
        else {
            return false
        }
        resizingDivider = true
        Task {
            await act(on: workspace) { c in
                await c.resizePane(pane.short, side: side, cells: cells, in: workspace)
            }
            resizingDivider = false
        }
        return true
    }

    /// Works across layouts too: the pane leaves whichever one it was in.
    private func placePane(
        _ dragged: String, onto target: String, side: TileDirection, in workspace: Workspace
    ) {
        let shorts = [dragged, target].compactMap { id in
            workspace.terminals.first { $0.id == id }?.short
        }
        guard shorts.count == 2 else { return }
        Task {
            let groups = await act(on: workspace, default: []) { c in
                await c.movePane(shorts[0], onto: shorts[1], side: side, in: workspace)
            }
            reveal(groups, in: workspace, preferring: dragged)
        }
    }

    /// Select the active layout's focused pane after a layout command.
    ///
    /// Every command that changes which group is on screen goes through this, so
    /// "the thing I am looking at" and "the thing the layout says is focused" cannot
    /// disagree. `preferring` is for the cases where the command was about a
    /// specific terminal and that terminal should win.
    private func reveal(
        _ groups: [PaneGroup], in workspace: Workspace, preferring: String? = nil
    ) {
        let host = workspace.host ?? ""
        guard let active = groups.first(where: { $0.isActive }) ?? groups.first else {
            // No layouts left, which now means no terminals left. Fall back to
            // the worktree rather than to a pane that no longer exists.
            selection = .workspace(host: host, id: workspace.id)
            return
        }
        let target = preferring.flatMap { active.terminals.contains($0) ? $0 : nil }
            ?? active.focused
            ?? active.terminals.first
        guard let target else {
            selection = .workspace(host: host, id: workspace.id)
            return
        }
        selection = .terminal(host: host, workspace: workspace.id, terminal: target)
    }

    /// Follow the layout's focus when something else moved it.
    ///
    /// The CLI and an agent can both focus a pane, and when they do the app has
    /// to be looking at it — otherwise `farcooler layout focus` from a script
    /// draws a border around a pane whose keystrokes still go somewhere else.
    private func followLayoutFocus() {
        guard case .terminal(let host, let wsID, let termID) = selection,
            let workspace = workspace(host: host, id: wsID),
            let group = store.client(for: workspace)?.activeGroup(wsID),
            let focused = group.focused,
            focused != termID,
            group.terminals.contains(termID)
        else { return }
        selection = .terminal(host: host, workspace: wsID, terminal: focused)
    }

    private func run(_ command: AppCommand) {
        switch command {
        case .newTerminal:
            // Creates immediately. There is no agent to choose: a terminal is a
            // shell, and whatever you run in it — `claude`, `codex`, a build —
            // is detected from the process, not declared in advance. Asking
            // first was a dialog whose answer was already knowable.
            if let workspace = currentWorkspace { newTerminal(in: workspace) }

        case .closeTerminal:
            guard let (workspace, terminal) = selectedTerminal else { return }
            Task {
                // Stop, then remove the record. Closing a terminal should leave
                // nothing behind — that is what closing means everywhere else.
                await act(on: workspace) { c in await c.stop(terminal: terminal.short) }
                await act(on: workspace) { c in await c.removeTerminal(terminal.short) }
                selectNeighbour(of: terminal)
            }

        case .nextTerminal: step(by: 1)
        case .previousTerminal: step(by: -1)

        case .nextAttention:
            // Straight to whatever is waiting on you. On a fleet of twenty this
            // is the difference between the app being useful and being a list.
            let ordered = allTerminals
            let start = ordered.firstIndex { $0.id == selectedTerminal?.terminal.id } ?? -1
            let rotated = ordered[(start + 1)...] + ordered[...max(start, 0)]
            if let next = rotated.first(where: { $0.agent.wantsAttention }) {
                select(next)
            }

        case .newWorkspace:
            // Only reachable with a project registered; the panel has nothing
            // to create into otherwise.
            if store.repositories.isEmpty {
                showAddRepository = true
            } else {
                if lastProject.isEmpty, let id = defaultProjectID { lastProject = id }
                showQuickCreate = true
            }
        case .addRepository: showAddRepository = true
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
        case .openTerminal(let workspace, let terminal):
            expanded.insert(workspace)
            let host = store.fleet.workspaces.first { $0.id == workspace }?.host ?? ""
            selection = .terminal(host: host, workspace: workspace, terminal: terminal)

        case .openWorkspace(let workspace):
            expanded.insert(workspace)
            let host = store.fleet.workspaces.first { $0.id == workspace }?.host ?? ""
            selection = .workspace(host: host, id: workspace)

        case .newTerminal(let id):
            guard let workspace = store.fleet.workspaces.first(where: { $0.id == id }) else {
                return
            }
            newTerminal(in: workspace)

        case .newTask(let described):
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

        case .togglePaneMode(let workspace, let terminal):
            guard
                let ws = store.fleet.workspaces.first(where: { $0.id == workspace }),
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
        guard let workspace = detailWorkspace else {
            errorBanner = "Open a workspace first — there is nothing to hand to an editor."
            return
        }
        let editors = Editors.shared
        editors.refresh()
        let runner = workspace.host ?? ""
        guard let editor = editors.preferred(host: runner) else {
            // Not an error. Nothing is wrong with an app that has never been
            // told which editor you use — so this opens the place you say so,
            // exactly as clicking the control with no editor configured does.
            EditorSettingsLink.open(openSettings)
            return
        }
        Task {
            if let problem = await editors.open(workspace, with: editor) { editorError = problem }
        }
    }

    /// Start a task and go to it as soon as it exists.
    ///
    /// Deliberately does not wait for the agent to boot before selecting. The
    /// point is to be looking at the thing you asked for while it starts, not
    /// to stare at the old screen for ten seconds first.
    ///
    /// `host` is handed in rather than re-derived from `project` here — see
    /// `QuickCreate.chosen`, which resolves both together from the same
    /// picker selection. A lookup repeated at this end, from `project`
    /// alone, is exactly the shape that goes silently wrong: `project` can
    /// name a repository that has since been removed, or one on a runner
    /// whose `repositories` has not been re-read since a reconnect, and a
    /// lookup that finds nothing has to be answered with a refusal, not a
    /// fallback to this Mac.
    private func startTask(description: String, host: String, project: String, agent: String) {
        Task {
            if let why = store.refusal(for: host) {
                errorBanner = "Cannot do that: \(why)"
                return
            }
            guard let client = store.clients[host] else { return }
            let created = await client.startTask(
                project: project,
                description: description,
                agent: agent.isEmpty ? Preferences.shared.defaultAgent : agent)
            reveal(created)
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
                errorBanner = "Cannot do that: \(why)"
                return
            }
            guard let client = store.clients[host] else { return }
            let created = await client.adoptBranch(
                project: project, branch: branch, agent: agent)
            reveal(created)
        }
    }

    /// Select a freshly created workspace, preferring its terminal.
    private func reveal(_ workspace: String?) {
        guard let workspace else { return }
        expanded.insert(workspace)
        let found = store.fleet.workspaces.first { $0.id == workspace }
        let host = found?.host ?? ""
        if let terminal = found?.terminals.first {
            selection = .terminal(host: host, workspace: workspace, terminal: terminal.id)
        } else {
            selection = .workspace(host: host, id: workspace)
        }
    }

    /// Create a terminal and go straight to it.
    ///
    /// Selecting it afterwards matters: you made a terminal because you want to
    /// type in it, and leaving the selection where it was means a second click
    /// to get to the thing you just asked for.
    private func newTerminal(in workspace: Workspace) {
        Task { await openTerminalInNewLayout(workspace) }
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
    private func openTerminalInNewLayout(_ workspace: Workspace) async -> Terminal? {
        expanded.insert(workspace.id)
        guard
            let created = await act(
                on: workspace, default: nil as Terminal?,
                { c in
                    await c.createTerminal(
                        in: workspace,
                        preset: "shell",
                        title: "Terminal \(workspace.terminals.count + 1)")
                })
        else { return nil }
        selection = .terminal(host: workspace.host ?? "", workspace: workspace.id, terminal: created.id)
        return created
    }

    /// The workspace the detail pane is actually showing.
    ///
    /// Not `currentWorkspace` by name, though the two now compute the same
    /// value: `currentWorkspace`'s own fallback to the first workspace in the
    /// fleet is gone, removed for the same reason this property never had
    /// one — with nothing selected, the placeholder is on screen, and
    /// offering to open a worktree the window is not showing would be the
    /// control lying about what it points at. Kept as a separate property so
    /// each name still reads as what it answers: this one, what the detail
    /// pane draws; `currentWorkspace`, what a keystroke acts on.
    private var detailWorkspace: Workspace? {
        switch selection {
        case .workspace(let host, let id): return workspace(host: host, id: id)
        case .terminal(let host, let id, _): return workspace(host: host, id: id)
        case nil: return nil
        }
    }

    /// The workspace the selection is in — nil when nothing is selected.
    ///
    /// Used to be "or the first one": with nothing selected, that first
    /// workspace could belong to ANY runner in the fleet, chosen by nothing
    /// more meaningful than merge order. `tileTarget` and ⌘T both read this
    /// to decide what a keystroke acts on, and a fallback here meant a ⌃B
    /// command issued while the detail pane was blank still landed — split,
    /// break, preset, cycle, zoom, focus, a new terminal — on whichever
    /// runner happened to own that first row. Wrong-runner routing is the
    /// one failure this feature must never produce, so with no selection
    /// there is now no target, and `tileTarget`'s and `.newTerminal`'s own
    /// `guard`/`if let` already do nothing rather than guess.
    private var currentWorkspace: Workspace? {
        switch selection {
        case .workspace(let host, let id): return workspace(host: host, id: id)
        case .terminal(let host, let id, _): return workspace(host: host, id: id)
        case nil: return nil
        }
    }

    private func step(by offset: Int) {
        let ordered = allTerminals
        guard !ordered.isEmpty else { return }
        let current = ordered.firstIndex { $0.id == selectedTerminal?.terminal.id } ?? 0
        // Wraps, because a list you can walk off the end of makes you look.
        let next = (current + offset + ordered.count) % ordered.count
        select(ordered[next])
    }

    /// Move the selection off a terminal that has gone.
    ///
    /// Prefers to stay where the user was looking: another terminal in the same
    /// workspace, whatever wants attention first, then anything running. Only
    /// falls back to the workspace itself when the workspace is empty.
    private func healSelection() {
        guard case .terminal(let host, let workspaceID, let terminalID) = selection else { return }
        guard
            let workspace = store.fleet.workspaces.first(where: {
                ($0.host ?? "") == host && $0.id == workspaceID
            })
        else {
            // The whole workspace went. Land on whatever is left rather than
            // on nothing.
            selection = store.fleet.workspaces.first.map { .workspace(host: $0.host ?? "", id: $0.id) }
            return
        }
        guard !workspace.terminals.contains(where: { $0.id == terminalID }) else { return }

        let candidates = workspace.terminals
        let next = candidates.first(where: { $0.status.wantsAttention })
            ?? candidates.first(where: { StateKind.parse($0.state) == .running })
            ?? candidates.first
        selection = next.map { .terminal(host: host, workspace: workspaceID, terminal: $0.id) }
            ?? .workspace(host: host, id: workspaceID)
    }

    private func selectTerminal(at index: Int) {
        let ordered = allTerminals
        guard index >= 0, index < ordered.count else { return }
        select(ordered[index])
    }

    private func select(_ terminal: Terminal) {
        guard
            let workspace = store.fleet.workspaces
                .first(where: { $0.terminals.contains(where: { $0.id == terminal.id }) })
        else { return }
        expanded.insert(workspace.id)
        selection = .terminal(host: workspace.host ?? "", workspace: workspace.id, terminal: terminal.id)
    }

    /// After closing one, land on the next terminal rather than on nothing.
    private func selectNeighbour(of terminal: Terminal) {
        let ordered = allTerminals
        guard let index = ordered.firstIndex(where: { $0.id == terminal.id }) else { return }
        let remaining = ordered.enumerated().filter { $0.offset != index }.map(\.element)
        if let next = remaining.first(where: { StateKind.parse($0.state) == .running })
            ?? remaining.first
        {
            select(next)
        } else {
            selection = currentWorkspace.map { .workspace(host: $0.host ?? "", id: $0.id) }
        }
    }
}

enum TerminalAction { case restart, dismissLost, stop }

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
