import SwiftUI

// The navigation shell over a real runner: the fleet mapped onto the shell's
// vocabulary, and terminals in the slots.
//
// `ShellRootView` is generic over its pane and knows nothing about a runner —
// that is the seam this file sits on. Everything that is a fact about THIS app
// rather than about the gesture is here: which tabs a workspace has and in
// what order, what each mark means, which pane each tab draws, and the two
// pieces of state a mounted pane cannot own for itself — `isVisible` and
// `Notifier.shared.visibleTerminal`.

/// How long the daemon's answer about a terminal stays fresh.
///
/// **This is not a fifth state competing with the daemon's four.** The rule
/// `FleetView.swift:168-171` states — *"every state shown here is DERIVED by
/// the daemon at the moment of asking; the phone never computes a terminal's
/// state, because a client that re-derives can disagree with the daemon and
/// with the Mac about the same terminal"* — is about what a terminal is DOING.
/// A threshold on `activityChangedAt` (`CoreModel.swift:379`, a
/// daemon-supplied timestamp) says how long ago the runner last told us
/// anything, which is a fact about OUR knowledge. It cannot disagree with the
/// daemon, because it is not an opinion about the same question.
///
/// So the dashed ring is drawn UNDER whatever the daemon's state already says
/// and never instead of it: an agent that is blocked is `needsYou` however old
/// the news is, because a blocked agent is waiting on you either way. Stale
/// only ever displaces `working`, which is the state that means "nothing to
/// report" and is exactly the one that stops being true when nobody has
/// reported for an hour.
///
/// One named constant, in one place. If it ever has to agree with the Mac,
/// that is the moment to push it into `farcooler_core::feed` — not the moment
/// to copy it.
///
/// An hour, because the thing it has to separate is an agent that is thinking
/// from an agent whose session has quietly died. Turns run for many minutes;
/// a threshold in minutes would draw half a healthy fleet as unknown.
private let staleAfter: TimeInterval = 60 * 60

/// What one shell tab draws, and where.
///
/// A side table keyed by tab id rather than something encoded IN the id.
/// `ShellTab` lives in AgentKit, which cannot see `Terminal` — see that file's
/// header on why the shell's model is a shared package rather than shared code
/// — so the alternative is packing a workspace id and a pane id into one
/// string and parsing it back out at every use, which is a decoder nobody
/// wrote a test for standing between the fleet and the screen.
struct ShellPaneRef: Hashable {
    var workspace: String
    var pane: Pane
}

/// The fleet as the shell needs it, plus what each of its tabs is.
///
/// Built whole from a `Connection` on every poll and thrown away. Nothing here
/// is remembered: the retained set lives in `ShellPaneTrack` and is keyed by
/// tab id, so this value moving underneath it is exactly the case that was
/// designed for.
@MainActor
struct ShellFleetMap {
    var fleet: ShellFleet
    var refs: [String: ShellPaneRef]

    /// A tab's id: the workspace it is in, then the pane it is.
    ///
    /// The workspace is in it because of the Changes tab. `Pane.changes` has
    /// one id for the whole app — it is the only pane with no object on the
    /// runner behind it — and forty workspaces each with a Changes tab would
    /// otherwise be forty tabs sharing one SwiftUI identity, which resolves by
    /// drawing one of them. Terminal ids are already unique per runner and
    /// gain nothing from the prefix except being legible in a probe.
    static func tabID(workspace: String, pane: Pane) -> String {
        "\(workspace)/\(pane.id)"
    }

    /// Read a runner's fleet as the shell's.
    ///
    /// `now` is an argument rather than `Date()` so staleness is a pure
    /// function of its inputs.
    static func of(_ connection: Connection, now: Date = Date()) -> ShellFleetMap {
        var refs: [String: ShellPaneRef] = [:]
        var workspaces: [ShellWorkspace] = []

        for workspace in connection.fleet.workspaces {
            let inbox = connection.inbox[workspace.id]
            // A host-side `changes` pane is not a tab of its own: it IS the
            // Changes tab, and both resolve to the same `ChangesStore`. Two
            // chips for one diff is what `Pane.init(_:)` exists to prevent.
            let terminals = workspace.terminals.filter { !$0.isChangesPane }

            // **Changes leads, then fleet order, and never `sortRank`.**
            //
            // The tab strip this replaced made the argument and it is worse
            // here rather than better: a ribbon is a MAP of the workspace, and
            // a map whose landmarks move when an agent goes from working to
            // blocked is not a map — you would have to read it every time
            // instead of remembering it. The diff leading means the one tab
            // that is always there is always at the same end.
            var tabs: [ShellTab] = [
                ShellTab(
                    id: tabID(workspace: workspace.id, pane: .changes),
                    title: "Diff",
                    mark: diffMark(inbox))
            ]
            var order: [ShellPaneRef] = [ShellPaneRef(workspace: workspace.id, pane: .changes)]

            for terminal in terminals {
                let pane = Pane(terminal)
                tabs.append(
                    ShellTab(
                        id: tabID(workspace: workspace.id, pane: pane),
                        title: terminal.label,
                        mark: mark(of: terminal, now: now)))
                order.append(ShellPaneRef(workspace: workspace.id, pane: pane))
            }

            for (tab, ref) in zip(tabs, order) { refs[tab.id] = ref }

            workspaces.append(
                ShellWorkspace(
                    id: workspace.id,
                    name: workspace.task,
                    // Nil, always, and that is honest rather than unfinished.
                    // A `Connection` is one runner — `RootView` keys the whole
                    // tree `.id(host)` — so every workspace on this screen is
                    // on the same machine, and a name that is on every bar is
                    // a name that stops being read. The field earns its keep
                    // the day a fleet spans runners.
                    server: nil,
                    tail: tail(of: workspace),
                    resume: resume(workspace, connection: connection, tabs: order),
                    tabs: tabs))
        }

        return ShellFleetMap(fleet: ShellFleet(workspaces: workspaces), refs: refs)
    }

    /// The Diff tab's mark.
    ///
    /// **Only the Diff tab can be cyan, and that is a model fact rather than a
    /// style choice.** Unread-diff comes from `Connection.inbox`, and an
    /// `InboxRow` is a WORKSPACE's counts, not an agent's state.
    /// `NeedsYou.swift:94-97` already refuses to invent a per-agent version of
    /// it — *"inventing one from the workspace's terminals would sort a diff
    /// by how blocked some agent in the same worktree happens to be"* — and
    /// the same refusal is what stops an agent tab ever drawing a cyan ring.
    ///
    /// Both halves of the condition, and they are not the same fact.
    /// `hasDiff` is true of every worktree with work on it and stays true
    /// after you have read it; `changedSinceReviewed` is the daemon's
    /// watermark, and it is what makes this "there is something new here"
    /// rather than "there is a branch here". `PaneFocus.rule` gates on the
    /// same pair.
    ///
    /// Never stale: the diff has no activity and no timestamp, so there is no
    /// answer whose age could be shown.
    private static func diffMark(_ inbox: InboxRow?) -> ShellMark {
        guard let inbox, inbox.changedSinceReviewed, inbox.hasDiff else { return .working }
        return .unreadDiff
    }

    /// One agent's mark.
    ///
    /// In order, and the order is the whole rule. `wantsAttention` — blocked
    /// or done, the app's single definition of "interrupt someone", shared
    /// with the Mac — comes first and is never displaced by age. Staleness
    /// comes next and displaces only `working`; see `staleAfter`.
    ///
    /// A terminal the host has said nothing about at all — no `activitySince`
    /// — is NOT stale. Nil means "not told", which is a different thing from
    /// "told a long time ago" and must never be rendered as it: an older
    /// daemon sends no timestamp for anything, and reading that as silence
    /// would draw a whole healthy fleet as unknown.
    private static func mark(of terminal: Terminal, now: Date) -> ShellMark {
        if terminal.agent.wantsAttention { return .needsYou }
        if let changed = terminal.activityChangedAt,
            now.timeIntervalSince(changed) > staleAfter
        {
            return .stale
        }
        return .working
    }

    /// What this workspace's card shows: the last few things its most recently
    /// active agent said.
    ///
    /// Most recently active rather than first, because the card's whole job is
    /// "what happened here while I was away" and the first terminal in fleet
    /// order is an arbitrary answer to that. Falls back to the first pane that
    /// has anything to say, and to nothing at all — a workspace whose agents
    /// have said nothing has nothing to show, and a placeholder there would be
    /// forty lies.
    private static func tail(of workspace: Workspace) -> [String] {
        let speaking = workspace.terminals.filter { !$0.isChangesPane && !$0.recentSteps.isEmpty }
        let latest = speaking.max { a, b in
            (a.activityChangedAt ?? .distantPast) < (b.activityChangedAt ?? .distantPast)
        }
        return (latest ?? speaking.first)?.recentSteps ?? []
    }

    /// Which tab this workspace should be REOPENED on.
    ///
    /// The one memory the app already keeps — `Connection.lastFocus`, written
    /// only by a person choosing a tab — resolved against the tabs that exist
    /// right now. A second memory living in the shell would be a second thing
    /// to disagree with it.
    ///
    /// Degrades rather than guessing. A remembered agent that has since exited
    /// falls through to `PaneFocus.rule(for:inbox:)`, which is this app's
    /// existing answer to "which pane should this workspace open on" — blocked
    /// agent, then unread diff, then top-ranked pane, then Changes — and a
    /// rule that answers with a pane this workspace does not have falls
    /// through to the first tab, which is the diff and always exists.
    ///
    /// Read by the BAR swipe, the carried lift and an overview card, and
    /// deliberately not by the content swipe. See `ShellWorkspace.resume`.
    private static func resume(
        _ workspace: Workspace, connection: Connection, tabs: [ShellPaneRef]
    ) -> Int? {
        var wanted: PaneFocus = connection.lastFocus[workspace.id] ?? .none
        if case .agent(let id) = wanted,
            !workspace.terminals.contains(where: { $0.id == id })
        {
            wanted = .none
        }
        if case .none = wanted {
            wanted = PaneFocus.rule(
                for: workspace, inbox: connection.inbox[workspace.id])
        }
        let pane: Pane
        switch wanted {
        case .changes, .none:
            pane = .changes
        case .agent(let id):
            guard let terminal = workspace.terminals.first(where: { $0.id == id }) else {
                return nil
            }
            pane = Pane(terminal)
        }
        return tabs.firstIndex { $0.pane.id == pane.id }
    }
}

/// One pane of a real workspace.
///
/// The two branches are the two things a workspace is: a pane on the runner,
/// and the worktree's own diff. The second needs nothing on the runner to
/// exist — see `Pane`.
///
/// The `Terminal` is LATCHED in `init` rather than re-read on every poll, and
/// that is `WorkspaceView.visited`'s rule carried across: the value a
/// `TerminalView` is built from is its identity, and handing it a fresh
/// snapshot three times a second would make the pane's own view of itself
/// change under it. What has to be live — the pane's mode, its activity — the
/// pane reads from the connection itself.
struct ShellPaneRealView: View {
    let slot: ShellPaneSlot
    let ref: ShellPaneRef
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue
    /// What GitHub says about this worktree's branch, resolved ABOVE this view
    /// and handed down as a plain value. See `ShellScreen.readPullRequest`.
    let pullRequest: BranchPullRequest?

    @State private var terminal: Terminal?

    init(
        slot: ShellPaneSlot, ref: ShellPaneRef, connection: Connection,
        pastes: ImagePasteQueue, pullRequest: BranchPullRequest?
    ) {
        self.slot = slot
        self.ref = ref
        self.connection = connection
        self.pastes = pastes
        self.pullRequest = pullRequest
        // Latched in `init`, not in `onAppear`, and the difference is a whole
        // extra mount. An `onAppear` latch means the first frame has no
        // terminal, so the pane draws a placeholder and then STRUCTURALLY
        // changes into a `TerminalView` — which is a build, a teardown and a
        // build, at the exact moment the shell has just promised not to do
        // that. `@State`'s initial value is evaluated here and kept for the
        // life of this view's identity, which is the same latch with no frame
        // in between.
        _terminal = State(initialValue: ref.pane.terminal)
    }

    private var workspace: Workspace? {
        connection.fleet.workspaces.first { $0.id == ref.workspace }
    }

    /// This pane's terminal as the daemon describes it RIGHT NOW, where the
    /// pane has one.
    ///
    /// `terminal` above is a LATCHED snapshot and is right to be one — it is
    /// the pane's identity, and handing a fresh copy to `TerminalView` three
    /// times a second would make the pane's view of itself change under it.
    /// What the bar needs is the opposite: its pane-mode item switches the very
    /// flag the snapshot froze, so a bar reading the snapshot would go on
    /// offering the switch it had already made. `TerminalView.live` draws the
    /// same distinction, one layer down, for the same reason.
    private var live: Terminal? {
        guard let terminal else { return nil }
        return connection.terminal(terminal.id, in: ref.workspace) ?? terminal
    }

    var body: some View {
        // One `NavigationStack` PER PANE, and none anywhere else in the shell.
        //
        // The shell has no navigation of its own — Phase 3 took the app's
        // single stack out, and the overview has one of its own — so this is
        // the pane borrowing the platform's chrome for the pane's own
        // controls, inside the pane, travelling with it on the track. See
        // `ShellPaneBar.swift`, which is the whole argument, and note the two
        // things it must not disturb: the track's geometry (a stack inside a
        // pane is invisible to `ShellPaneTrack`, which sizes every pane to
        // `page` × full height and offsets it) and the overview's own stack.
        NavigationStack {
            paneContent
                // The shell's furniture at the bottom, and NOT the keyboard.
                //
                // A `NavigationStack` is a `UINavigationController`, and the
                // framework re-derives keyboard avoidance from the window on
                // the far side of it — so the pane's content is inset by the
                // keyboard in here whatever the pane as a whole says about
                // ignoring it. That is fine, and it is now the ONLY subtraction
                // of the keyboard on this path: see `TerminalView`, which used
                // to make the same room by hand and no longer does. Measured on
                // an iPhone 17 with a keyboard up: the content's bottom safe
                // area is 450 — the shell's 90 points of furniture plus the
                // keyboard's whole 360-point reach — and the grid gets the 308
                // points that are left.
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    Color.clear.frame(height: slot.chrome.bottom)
                }
                // The pane's own controls and title, wrapped AROUND the pane
                // rather than handed to `.toolbar` as a view: `.toolbar` is a
                // modifier on this stack's root, and the picker and sheets have
                // to hang off a live view hierarchy that can present.
                .modifier(paneChrome)
        }
        // The display's own safe area, handed back to the pane.
        //
        // `ShellRootView` lays the shell out full bleed and zeroes the safe
        // area to do it (`ShellRootView.body`), so a navigation stack mounted
        // in here would put its bar under the status bar. This gives the stack
        // the top inset the window would have given it, which is what makes
        // the bar sit where a navigation bar sits and its material run up
        // behind the clock. The BOTTOM half of the shell's furniture is a
        // content inset on the pane instead — see above — because there is no
        // bottom bar in this stack for it to belong to.
        .safeAreaInset(edge: .top, spacing: 0) {
            Color.clear.frame(height: slot.chrome.top)
        }
        // A terminal is dark regardless of the phone's own appearance — the
        // host doesn't know or care whether this device is in Light Mode.
        .preferredColorScheme(Themes.shared.current.colorScheme)
        // Painted past every edge, including under the home indicator and
        // behind the docked composer.
        .background(TerminalPalette.background.ignoresSafeArea())
        // Keyboard room is owned by the pane below. The shell around it stays
        // full-height so the bar and the track never move under a keyboard.
        .ignoresSafeArea(.keyboard, edges: .bottom)
    }

    /// The pane's chrome, as a modifier so `paneContent` can be the thing it
    /// wraps. Only the Diff tab gets a review store: it is the pane with no
    /// terminal behind it, and a host-side `changes` pane resolves to the same
    /// tab and the same store — see `Pane.init(_:)` — so this cannot be two
    /// menus over one review.
    private var paneChrome: ShellPaneChromeModifier {
        ShellPaneChromeModifier(
            title: slot.tab.title,
            connection: connection,
            pastes: pastes,
            workspace: workspace,
            live: live,
            changes: terminal == nil
                ? connection.changesStores.store(for: ref.workspace) : nil,
            isVisible: slot.isVisible)
    }

    @ViewBuilder
    private var paneContent: some View {
        Group {
            if let terminal {
                TerminalView(
                    terminal: terminal,
                    // Exactly one pane in the whole track has this. See
                    // `ShellPaneTrack`, and `DockedBar.swift:34-41` for what
                    // two of them costs.
                    isVisible: slot.isVisible,
                    connection: connection,
                    pastes: pastes)
            } else {
                // The store comes from `Connection`, keyed by workspace, so
                // this is the SAME review a `changes` pane in this worktree
                // would show and the same one the Mac is looking at.
                //
                // **No `Connection` argument, deliberately.**
                // `ChangesView.swift:91-97`: its body is a forty-card lazy
                // stack somebody is mid-scroll through, and observing the
                // connection there would rebuild it on every three-second
                // poll. It gets resolved values instead — the agents as plain
                // targets, the pull request read above.
                //
                // No `isVisible` either, and it needs none. What that flag
                // buys on a terminal is a stream, a poll and a tmux size
                // assertion that must stop when the pane is merely hidden;
                // `ChangesView` holds none of those.
                ChangesView(
                    store: connection.changesStores.store(for: ref.workspace),
                    workspaceName: workspace?.task ?? "Workspace",
                    agents: workspace?.reviewAgentTargets() ?? [],
                    pullRequest: pullRequest)
            }
        }
    }
}

/// The shell, standing on a runner.
///
/// What this owns that `ShellRootView` cannot:
///
/// - The mapping from a fleet to the shell's vocabulary, rebuilt every poll.
/// - `ImagePasteQueue`, owned here rather than per pane so a transfer keeps
///   running — and keeps reporting — when you swipe away from the pane that
///   started it. The same argument the pane host made for owning it above
///   its panes.
/// - The pull request on the Changes tab's header, read for the workspace at
///   rest and only while its diff is the pane on screen.
/// - **The one writer of `Notifier.shared.visibleTerminal`.** Read by
///   `Notifications.swift:162` to suppress a banner about the pane you are
///   already looking at, and by `Connection.markVisibleSeen()` to claim the
///   runner's ten-second watch. One writer, fed by one callback, fired when
///   the pane AT REST changes and at no other time — never mid-gesture, when
///   two panes are on screen and neither has arrived.
struct ShellScreen: View {
    @ObservedObject var connection: Connection
    /// The runners this device knows, for the menu in the overview's toolbar.
    ///
    /// The app's only way to reach another runner, to correct the one it is on,
    /// and to see this device's own key. It used to hang off `HostSwitcherBar`
    /// at the bottom of the workspace list; the shell has no strip to put a bar
    /// on, so the same menu is a toolbar item on the overview. See `RunnerMenu`.
    @ObservedObject var hosts: RunnerStore
    /// The terminal a tapped Live Activity card asked for, held by `FleetView`
    /// until this runner's fleet has it. See `requestedTab`.
    @Binding var pendingTerminal: String?

    @StateObject private var pastes = ImagePasteQueue()
    /// Where the shell opens. Resolved once, from the first fleet that
    /// arrives, and never again — see `seed`.
    @State private var initial: ShellPosition?
    /// What the pane at rest IS, in this app's own vocabulary.
    ///
    /// Recorded when the shell says a pane arrived rather than looked up
    /// again afterwards, and that is not thrift. Everything downstream of it —
    /// which terminal the runner should believe is being read, whether the
    /// pull request is worth a round trip, whether the last move was a choice
    /// — would otherwise have to rebuild the whole fleet mapping to ask, on
    /// every body pass, sixty times a second while a finger is down. The
    /// mapping is already in hand at the one moment the answer changes.
    ///
    /// By VALUE, so a poll that renumbers the fleet cannot make it name a
    /// different pane. `ShellPosition` is a pair of indices and is only
    /// meaningful against the fleet it was resolved with.
    @State private var restingRef: ShellPaneRef?
    /// What GitHub says about the branch of the workspace at rest.
    @State private var pullRequest: BranchPullRequest?
    /// The tab a deep link was last honored onto, until a rest accounts for it.
    ///
    /// A deep link is not a choice, and `remember(_:leaving:)` must not write
    /// one down. The pane host got this distinction for free — a card sent a
    /// `Terminal` through `select` and only a tapped chip went through
    /// `choose` — and the shell has one path in, so it has to be carried.
    ///
    /// The tab's ID and not a bare flag, and that is the difference between
    /// this working and this poisoning the next real choice. A link naming the
    /// pane already at rest moves nothing, so it produces no rest at all —
    /// `ShellRootView.honorRequest` only writes a position that differs — and a
    /// flag set for an arrival that never happens would be spent on whatever
    /// somebody chose next. An id can be compared: it is cleared by the first
    /// rest either way, and only skipped when that rest is the one the link
    /// asked for.
    @State private var linkedTab: String?

    /// Describe it, or fill in the form. Both were `WorkspaceListView`'s
    /// toolbar and are the overview's now — see `overviewActions`.
    @State private var showQuickTask = false
    @State private var showNewWorkspace = false

    @Environment(\.scenePhase) private var scenePhase

    private var map: ShellFleetMap { ShellFleetMap.of(connection) }

    var body: some View {
        Group {
            if let initial {
                shell(map, from: initial)
            } else {
                // Before the first fleet there is no position to open on, and
                // an empty shell would be a bar naming a workspace that does
                // not exist. The same gap `FleetView`'s connected branch
                // covers with a spinner.
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Themes.shared.current.backgroundColor.ignoresSafeArea())
            }
        }
        .onAppear { seed() }
        .onChange(of: connection.hasFleet) { _, _ in seed() }
        .task(id: pullRequestKey) { await readPullRequest() }
        .onChange(of: scenePhase) { _, phase in
            // Coming back to the app is reading whatever it comes back to.
            guard phase == .active else { return }
            markVisible()
        }
        .onDisappear { Notifier.shared.visibleTerminal = nil }
        // The two ways of starting work, moved here from the workspace list's
        // toolbar with their flows untouched: both were sheets there and both
        // are sheets here.
        //
        // Presented from THIS view rather than from the toolbar item that opens
        // them, and that is not tidying. The overview is mounted from the first
        // point of a lift and unmounted when the grid is neither showing nor
        // flying — so a sheet whose presenter lives inside it is a sheet whose
        // presenter can go away underneath it, and a sheet with nobody left to
        // close it is a sheet you cannot close.
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceView(
                repositories: connection.repositories, connection: connection
            ) { repository, name, branch, adopt in
                await connection.createWorkspace(
                    repository: repository, name: name, branch: branch, adopt: adopt)
            }
        }
        .sheet(isPresented: $showQuickTask) {
            TaskComposerView(connection: connection)
        }
    }

    /// What this app puts in the overview's navigation bar, opposite `Done`.
    ///
    /// All three were somewhere else, and all three had the same somewhere
    /// else: the pushed workspace list. It was a searchable screen of every
    /// workspace on the runner with a toolbar for starting work and the runner
    /// switcher along its bottom, and the overview is that screen — sorted by
    /// what needs you rather than by repository, with cards instead of rows.
    /// Two of them would be two answers to "what is on this runner".
    ///
    /// - The runner, and every way of changing it. See `RunnerMenu`.
    /// - The link, but only when there is something wrong with it. That is
    ///   the pane host's rule for the same chip, kept: a permanent
    ///   "Connected" is noise, and this is the app's only manual reconnect now
    ///   that the bar carrying it is gone from every connected screen.
    /// - A sparkle for "describe it" (`TaskComposerView`), a plain plus for
    ///   "fill in the form" (`NewWorkspaceView`) — the same two flows the Mac
    ///   keeps side by side, kept apart here by icon rather than by picking a
    ///   winner. `sparkle` and not `sparkles`, because the Mac's `QuickCreate`
    ///   marks this flow with the singular and one concept gets one glyph.
    ///
    /// Named out loud, because an `Image` alone in a `Button` is read as its SF
    /// Symbol: "sparkle" and "plus" were the whole of what VoiceOver had to
    /// tell the app's two ways of starting work apart, and neither is a word
    /// this product uses. Each is named for the sheet it opens.
    @ViewBuilder
    private var overviewActions: some View {
        RunnerMenu(hosts: hosts, connection: connection)

        if connection.phase != .connected {
            LinkStatusChip(connection: connection)
        }

        Button { showQuickTask = true } label: { Image(systemName: "sparkle") }
            .accessibilityLabel("Quick Task")

        Button { showNewWorkspace = true } label: { Image(systemName: "plus") }
            .accessibilityLabel("New Workspace")
    }

    private func shell(_ map: ShellFleetMap, from initial: ShellPosition) -> some View {
        ShellRootView(
            fleet: map.fleet,
            initial: initial,
            // A tapped Live Activity card, resolved against the fleet this
            // very body pass was built from. See `requestedTab`.
            request: Binding(
                get: { requestedTab(in: map) },
                set: { taken in
                    guard taken == nil, pendingTerminal != nil else { return }
                    // Whatever rest this produces is not a choice, so it must
                    // not be written down as one. See `linkedTab`.
                    linkedTab = requestedTab(in: map)
                    pendingTerminal = nil
                }),
            // The single writer. `ShellRootView` calls this when `position`
            // changes and on first appearance, which is exactly "a pane came
            // to rest" — a commit's silent re-seat included, because the
            // silence is about animation and not about change notification.
            onRest: { at in
                let tab = map.fleet.tab(at: at)
                let arrived = tab.flatMap { map.refs[$0.id] }
                remember(arrived, leaving: restingRef, tab: tab?.id)
                restingRef = arrived
                markVisible(arrived)
            },
            overviewActions: { overviewActions }
        ) { slot in
            if let ref = map.refs[slot.tab.id] {
                ShellPaneRealView(
                    slot: slot, ref: ref, connection: connection, pastes: pastes,
                    // Only the workspace at rest gets an answer. A neighbour's
                    // diff header can wait until you land on it; asking for
                    // three is three GitHub round trips per swipe.
                    pullRequest: ref.workspace == restingRef?.workspace ? pullRequest : nil)
            }
        }
        // Over the panes, above the key row, and gone the moment the path is
        // typed. Nothing about a transfer is ever written into the pane itself.
        .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
    }

    // MARK: - Where the shell opens

    /// Put the shell on a pane, once, from the first fleet that has one.
    ///
    /// Once and only once, for `FleetView.restorePlace`'s reason: after this
    /// the position is whatever the person holding the phone has done with it,
    /// and a second pass on a later reconnect would be the app steering them
    /// somewhere they had already left.
    ///
    /// `ShellFleet.first` rather than a rule, because the rule already ran:
    /// `ShellFleetMap.resume` resolved every workspace's remembered tab, and
    /// the first workspace's is where a launch lands.
    private func seed() {
        guard initial == nil else { return }
        let map = self.map
        guard !map.fleet.isEmpty, let at = map.fleet.first else { return }
        initial = ShellPosition(
            workspace: at.workspace, tab: map.fleet.workspaces[at.workspace].resumeTab)
    }


    // MARK: - Remembering where you were

    /// Write down the tab somebody chose, and only that.
    ///
    /// **Only when the arrival did not change workspace**, which is exactly
    /// the pane host's `choose(_:)` rule arrived at from the shell's side. In
    /// that screen the one writer was a tap on a chip; here the equivalents
    /// are a swipe along the content within a workspace and a tap on a menu
    /// row, and both of them are moves BETWEEN this workspace's tabs.
    ///
    /// Arriving in a workspace records nothing, and that matters more here
    /// than it did there. A bar swipe, a carried lift and a tapped card all
    /// land on `ShellWorkspace.resumeTab`, which for a workspace nobody has
    /// ever chosen a tab in is `PaneFocus.rule`'s answer — so recording an
    /// arrival would write the rule's own answer into the memory and the rule
    /// would never get to run again. That is the self-fulfilling memory the
    /// pane host refused to create, and the reason its `initial` pane never
    /// came through `choose`.
    ///
    /// The first rest of all is an arrival with nothing to compare against and
    /// is therefore also not a choice.
    private func remember(
        _ arrived: ShellPaneRef?, leaving previous: ShellPaneRef?, tab: String?
    ) {
        // A deep link is not a choice either, and it is the one arrival that
        // can look exactly like one: a card naming a pane in the workspace
        // already on screen lands on a different tab of the same workspace,
        // which is the shape this function is otherwise here to record. The
        // pane host never had to say so — a request went through `select` and a
        // chip through `choose`, and only the second wrote anything.
        //
        // Spent on the first rest whether or not it matched, so a link that
        // moved nothing cannot leave this standing over somebody's next choice.
        // See `linkedTab`.
        let linked = linkedTab
        linkedTab = nil
        if let tab, tab == linked { return }
        guard let arrived, arrived.workspace == previous?.workspace else { return }
        connection.rememberFocus(arrived.pane.focus, in: arrived.workspace)
    }

    // MARK: - A card tapped from outside the app

    /// Which TAB the pending deep link names, in the fleet this pass mapped.
    ///
    /// The whole of the two-phase dance, as a derivation rather than as a
    /// sequence of steps. `FleetView` holds the terminal id from the moment the
    /// URL arrives — see `FleetView.dropUnknownTerminal` — and this is nil for
    /// as long as the runner has not answered with a fleet containing it. When
    /// one does arrive, this stops being nil, and `ShellRootView` honors it on
    /// the next change or on its own appearance, whichever comes first.
    ///
    /// That "or on its own appearance" is the cold-launch half. A card tapped
    /// while the app was not running delivers its URL before there is a
    /// connection, so the shell is MOUNTED with the request already resolved
    /// and there is no change for an `onChange` to see.
    ///
    /// Against the map that was handed in rather than `self.map`, so the tab id
    /// this produces and the tab ids the shell is being drawn from are the same
    /// numbering. Rebuilding the map here would be a second read of a fleet
    /// that a poll may have replaced between the two.
    private func requestedTab(in map: ShellFleetMap) -> String? {
        guard let id = pendingTerminal else { return nil }
        for workspace in connection.fleet.workspaces {
            guard let terminal = workspace.terminals.first(where: { $0.id == id }) else {
                continue
            }
            let tab = ShellFleetMap.tabID(workspace: workspace.id, pane: Pane(terminal))
            // Only if the shell actually has it. A `changes` pane the host
            // happens to have open is folded into the Changes tab by
            // `Pane.init(_:)` and is not a tab of its own, so an id naming one
            // resolves to a tab that does exist; anything that does not is a
            // request this shell cannot honor and must not hold open.
            return map.fleet.position(ofTab: tab) == nil ? nil : tab
        }
        return nil
    }

    // MARK: - The one writer of `visibleTerminal`

    /// Which pane the runner should believe is being read.
    ///
    /// Nil on the Changes tab, and that is the honest answer rather than a
    /// gap: no pane is on screen, so no pane's notification should be
    /// suppressed and no agent's finished turn should be marked seen.
    /// `Connection.markVisibleSeen` reads exactly this and reports an empty
    /// watch list for it.
    private func markVisible(_ ref: ShellPaneRef? = nil) {
        Notifier.shared.visibleTerminal = (ref ?? restingRef)?.pane.terminal?.id
        Task { await connection.markVisibleSeen() }
    }

    // MARK: - The Changes tab's header

    /// When to read the pull request again: on arriving at the Changes tab of
    /// a workspace, and on every fleet poll while it is up.
    ///
    /// Deliberately not a timer of this screen's own — see
    /// `WorkspaceView.pullRequestReadKey`, which this is carried across from,
    /// including the branch being in the key so a worktree that changes branch
    /// under you re-reads at once.
    private var pullRequestKey: String {
        guard let ref = restingRef, case .changes = ref.pane else { return "" }
        let branch = connection.fleet.workspaces.first { $0.id == ref.workspace }?.branch ?? ""
        return "\(ref.workspace)|\(branch)|\(connection.pollGeneration)"
    }

    /// Read `stack.get` for the resting worktree's branch.
    ///
    /// Here rather than in `ChangesView` for the two reasons
    /// `WorkspaceView.readPullRequest` gives, which have not changed: this
    /// view has the repository UUID the call takes, and `ChangesView` must not
    /// be handed a `Connection` at all.
    private func readPullRequest() async {
        guard let ref = restingRef, case .changes = ref.pane,
            let workspace = connection.fleet.workspaces.first(where: { $0.id == ref.workspace })
        else { return }
        guard let repository = workspace.repository, !workspace.branch.isEmpty else {
            pullRequest = nil
            return
        }
        // A failed read leaves the last answer on screen rather than blanking
        // the row. The link dropping is not news about this pull request.
        guard let reply = await connection.stack(
            repository: repository, branch: workspace.branch)
        else { return }
        let mine = reply.links.first { $0.branch == workspace.branch }
        pullRequest = BranchPullRequest(
            pr: mine?.pr, known: reply.prAnswered, repoURL: reply.repoUrl)
    }
}
