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
    /// The runner this pane is on.
    ///
    /// Carried rather than assumed, and that is the whole of what the port
    /// changes here. A `ShellFleetMap` used to be one runner's fleet by
    /// construction — `RootView` keyed the tree `.id(host)`, so the screen was
    /// destroyed and rebuilt on every change of runner — and every ref in it
    /// named a workspace on the same machine, so the runner went without
    /// saying. The merged map holds several, and a ref that did not say which
    /// would send an RPC down whichever connection the screen happened to be
    /// holding. See `ShellIdentity`.
    var runner: UUID
    /// The DAEMON's own workspace id — the full UUID it minted, which is what
    /// goes on the wire in `changes.*` and `workspace.*` calls. Not the shell's
    /// composite, and not the eight-character `short` field, which is a display
    /// form nothing here uses as an identity.
    var workspace: String
    var pane: Pane
}

/// The fleet as the shell needs it, plus what each of its tabs is.
///
/// Built whole from the store on every poll and thrown away. Nothing here is
/// remembered: the retained set lives in `ShellPaneTrack` and is keyed by tab
/// id, so this value moving underneath it is exactly the case that was designed
/// for.
///
/// **It is the whole fleet and not one runner's**, which is the port's central
/// change. `of(_ store:)` walks `FleetStore.entries` — every workspace on every
/// connected runner, in the order the runner list is in — and every id it mints
/// carries the runner, so a tab resolves back to a connection rather than to a
/// search. See `ShellIdentity`, which is where that composition, its test and
/// the reason live.
@MainActor
struct ShellFleetMap {
    var fleet: ShellFleet
    var refs: [String: ShellPaneRef]

    /// Which runner each shell workspace is on, and what it said, keyed by the
    /// composite id `ShellWorkspace.id` now carries.
    ///
    /// The side table that makes a merged fleet act-on-able. A screen holding a
    /// `ShellWorkspace` has a name and a ribbon and nothing to send an RPC
    /// down; this is where it gets the connection, the runner and the daemon's
    /// own workspace id back. Keyed rather than searched, because the overview
    /// asks per card and the shell asks per rest.
    var entries: [String: FleetEntry] = [:]

    /// A tab's id: the runner, the workspace it is in, then the pane it is.
    ///
    /// Composed by `ShellIdentity` rather than here, so the composition sits
    /// somewhere a test can read it back: that file is in AgentKit, which
    /// `swift test` runs, and this target's tests are compiled by CI and never
    /// executed.
    static func tabID(runner: UUID, workspace: String, pane: Pane) -> String {
        ShellIdentity.tab(
            runner: runner.uuidString, workspace: workspace, pane: pane.id)
    }

    /// Read the whole fleet — every connected runner's — as the shell's.
    ///
    /// Order is the store's, which is the order the runner list is in, so what
    /// is on screen does not reshuffle when a laptop wakes up. A runner that is
    /// not answering contributes its last good rows rather than a gap; what
    /// explains those rows is `RunnerStatusRow`, not their absence.
    ///
    /// `now` is an argument rather than `Date()` so staleness is a pure
    /// function of its inputs.
    static func of(_ store: FleetStore, now: Date = Date()) -> ShellFleetMap {
        of(store.entries, now: now)
    }

    /// The merge itself, over the entries rather than the store that published
    /// them, so the one caller that has a runner instead of a fleet can reach
    /// it too.
    static func of(_ entries: [FleetEntry], now: Date = Date()) -> ShellFleetMap {
        var map = ShellFleetMap(fleet: ShellFleet(workspaces: []), refs: [:])
        var workspaces: [ShellWorkspace] = []
        // More than one runner in the merge is what makes a card name its
        // machine. See `server` below.
        let servers = Set(entries.map(\.host.id)).count
        for entry in entries {
            let built = one(entry, naming: servers > 1, now: now)
            workspaces.append(built.workspace)
            for (id, ref) in built.refs { map.refs[id] = ref }
            map.entries[built.workspace.id] = entry
        }
        map.fleet = ShellFleet(workspaces: workspaces)
        return map
    }

    /// One runner's fleet, on its own, for the one caller that is about a
    /// runner rather than about the fleet: `Connection.recordDirectory`, which
    /// writes down what THIS runner has so a grid can draw it later.
    ///
    /// Shares `one(_:naming:now:)` with the merge above rather than mapping a
    /// fleet a second way, which is the rule `recordDirectory` already states:
    /// tab order, what a mark means and how a tail is chosen are decided once,
    /// in the file that draws them, or a cached card and a live one disagree
    /// about the same worktree.
    static func of(
        _ connection: Connection, host: Runner, now: Date = Date()
    ) -> ShellFleetMap {
        of(
            connection.fleet.workspaces.map {
                FleetEntry(
                    host: host, connection: connection, workspace: $0,
                    counts: connection.inbox[$0.id])
            },
            now: now)
    }

    /// One entry, as a workspace and the refs of its tabs.
    private static func one(
        _ entry: FleetEntry, naming server: Bool, now: Date
    ) -> (workspace: ShellWorkspace, refs: [String: ShellPaneRef]) {
        var refs: [String: ShellPaneRef] = [:]
        let connection = entry.connection
        let runner = entry.host.id
        let workspace = entry.workspace
        let inbox = entry.counts
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
                id: tabID(runner: runner, workspace: workspace.id, pane: .changes),
                title: "Diff",
                mark: diffMark(inbox))
        ]
        var order: [ShellPaneRef] = [
            ShellPaneRef(runner: runner, workspace: workspace.id, pane: .changes)
        ]

        for terminal in terminals {
            let pane = Pane(terminal)
            tabs.append(
                ShellTab(
                    id: tabID(runner: runner, workspace: workspace.id, pane: pane),
                    title: terminal.label,
                    mark: mark(of: terminal, now: now),
                    // The sort's own question, kept separate from the
                    // drawing's. See `ShellTab.wantsAttention`.
                    wantsAttention: terminal.agent.wantsAttention))
            order.append(ShellPaneRef(runner: runner, workspace: workspace.id, pane: pane))
        }

        for (tab, ref) in zip(tabs, order) { refs[tab.id] = ref }

        return (
            ShellWorkspace(
                // The COMPOSITE, not the daemon's own id. This string is a
                // SwiftUI identity — the overview gives each card `.id(_:)`
                // and an accessibility identifier off it, and
                // `ShellPaneTrack` remembers which workspace a retained pane
                // belongs to by it — and it is what a screen resolves a
                // runner from. The daemon's own id is on the `FleetEntry` in
                // `entries`, which is where the wire gets it back. See
                // `ShellIdentity`.
                id: ShellIdentity.workspace(
                    runner: runner.uuidString, workspace: workspace.id),
                name: workspace.task,
                // The runner's name, but only where the fleet on screen has
                // more than one runner in it.
                //
                // It was unconditionally nil, and the reason it gave was
                // "a `Connection` IS one runner, so the overview names that
                // machine once, on the section header over these cards,
                // rather than forty times underneath them". The first half
                // stopped being true; the second half is still right when
                // it applies, which is why this is a condition rather than
                // a name on every card. One runner: the header says it
                // once, exactly as before. Several: a card that did not say
                // where its worktree is would leave the one question a
                // merged grid raises unanswered.
                server: server ? entry.host.label : nil,
                tail: tail(of: workspace),
                resume: resume(workspace, connection: connection, tabs: order),
                // The daemon's own view preference, carried rather than
                // re-derived. iOS had no consumer for it at all, so a
                // worktree somebody put away on the Mac came back as an
                // ordinary card on the phone. See `ShellFleet.hiddenOrder`.
                isHidden: workspace.isHidden,
                // The one workspace the overview card's menu must not
                // offer to remove. Carried rather than looked up again
                // from the connection at menu-build time, so the card and
                // the daemon are reading one fact.
                isPrimaryCheckout: workspace.isPrimaryCheckout,
                tabs: tabs),
            refs
        )
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
    ///
    /// **The core is `nil` on both arms, and that is the whole of what a Diff
    /// tab is.** `GlanceMark.core` documents nil as a surface DECLINING to
    /// state the agent axis, which is a different thing from stating that an
    /// agent is at a prompt — and a diff is the one tab in this app with no
    /// terminal behind it and no agent that could be producing anything. It
    /// therefore has nothing to say on that axis and says nothing.
    ///
    /// This matters twice over. It is what keeps a Diff tab from drawing the
    /// filled mark that now means "an agent is producing" — under the old
    /// `ShellMark` the quiet arm here was `.working`, the same case a running
    /// agent used, so every workspace's Diff tab would have started drawing a
    /// filled dot the moment fill began to mean something. And it is what lets
    /// `RunnerDirectory.word(for:)` tell an unread diff from a finished turn,
    /// since both are `.toReview` and only one of them is an agent.
    private static func diffMark(_ inbox: InboxRow?) -> GlanceMark {
        guard let inbox, inbox.changedSinceReviewed, inbox.hasDiff else {
            return GlanceMark(attention: .quiet, core: nil)
        }
        return GlanceMark(attention: .toReview, core: nil)
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
    ///
    /// **This is the same table `GlanceMark.init(agent:)` reads**, and it is
    /// here rather than beside that one because it takes an `AgentActivity`:
    /// `GlanceMark.swift` is compiled a file at a time by the watch's
    /// complication and `CoreModel.swift` is not in that list, so a mapping
    /// living there would break a build this app does not run.
    ///
    /// **`working` is no longer the catch-all it was.** The old `ShellMark`
    /// had one case for a producing agent, an idle one, an agent the daemon
    /// had said nothing about, and a Diff tab — and drawing them alike is the
    /// defect this replaced: the bar could not say an agent was running even
    /// in principle. The core axis carries that distinction now, and `idle`,
    /// `none` and `unknown` land at a prompt rather than borrowing a claim
    /// that some agent somewhere is producing.
    private static func mark(of terminal: Terminal, now: Date) -> GlanceMark {
        // Latched, both of them, and never dashed. `GlanceMark.Link` states
        // the rule — "blocked and to-review hold at any age; working and idle
        // go dashed" — and `FleetSnapshot.Confidence.isLatched` is the same
        // sentence about the same two statuses. An agent that stopped for you
        // an hour ago is still stopped for you.
        switch terminal.agent {
        case .blocked: return GlanceMark(attention: .needsYou, core: .atAPrompt)
        case .done: return GlanceMark(attention: .toReview, core: .atAPrompt)
        default: break
        }
        let stale =
            if let changed = terminal.activityChangedAt {
                now.timeIntervalSince(changed) > staleAfter
            } else {
                false
            }
        return GlanceMark(
            attention: .quiet,
            core: terminal.agent == .working ? .producing : .atAPrompt,
            link: stale ? .broken : .live)
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
    /// A terminal this pane's bar just made. Carried straight through to
    /// `ShellPaneChromeModifier` — a pane has no idea where it sits on the
    /// track, so the arrival is `ShellScreen`'s to ask for.
    let onCreated: (String) -> Void

    @State private var terminal: Terminal?

    /// How far the keyboard reaches up this pane, the key row included.
    ///
    /// Observed rather than derived, because the number this pane has to
    /// cancel is applied by the framework on the far side of a
    /// `NavigationStack` where nothing in SwiftUI's own vocabulary can see it.
    /// The same object `AgentView` uses, and the same reading: one
    /// notification whose reported frame already includes the accessory. See
    /// `KeyboardInset` and `paneBottom`.
    @StateObject private var keyboard = KeyboardInset()

    /// The display's own bottom inset, as the shell measured it. See
    /// `EnvironmentValues.shellDisplayBottom`.
    @Environment(\.shellDisplayBottom) private var displayBottom

    init(
        slot: ShellPaneSlot, ref: ShellPaneRef, connection: Connection,
        pastes: ImagePasteQueue, pullRequest: BranchPullRequest?,
        onCreated: @escaping (String) -> Void
    ) {
        self.slot = slot
        self.ref = ref
        self.connection = connection
        self.pastes = pastes
        self.pullRequest = pullRequest
        self.onCreated = onCreated
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
                // The shell's furniture at the bottom, and only the part of it
                // this stack has not already reserved. See `paneBottom`.
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    Color.clear.frame(height: paneBottom)
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

    /// The room to reserve under this pane's content, which is NOT
    /// `chrome.bottom` and was.
    ///
    /// A `NavigationStack` is a `UINavigationController`, and the framework
    /// re-derives the bottom safe area from the window on the far side of it —
    /// the home indicator always, and the keyboard whenever one is up —
    /// whatever the pane as a whole says about ignoring either. So this inset
    /// is not the furniture, it is the furniture MINUS what the stack has
    /// already reserved, and reserving the flat number instead reserved two
    /// different invisible things:
    ///
    /// - **The home indicator, twice.** `chrome.bottom` is 90 on an iPhone 17
    ///   — 34 of home indicator plus the bar's 44 and its 12 of breathing room
    ///   — and the stack had already inset by the same 34. Measured on the
    ///   simulator with the key row down: the grid ran to y=750 where the
    ///   bar's top edge is at 784. With this, 784.
    /// - **The bar, while the keyboard is over it.** The shell's bar does not
    ///   move for a keyboard, on purpose (`ShellRootView.body`,
    ///   `ShellPaneTrack.swift:53-61`), so everything the keyboard reaches
    ///   over is room nobody can see. Two states, both measured:
    ///   - Only the terminal's key row up, over a hardware keyboard. The
    ///     stack reserved 52 of its own, the flat number added 90 on top, and
    ///     the grid ran to y=732 — 52 points of nothing under the last line.
    ///     With this, 784, the bar's edge.
    ///   - A software keyboard up. The stack reserves its whole 360-point
    ///     reach, the flat number added 90 to that, and the grid stopped
    ///     short of the MIDDLE OF THE DISPLAY: the test could not find a pane
    ///     under y=437 at all, because 874 − 116 of bar and status bar − 450
    ///     leaves 308 and the grid ended at 424, with the key row at 514. A
    ///     90-point band of nothing between the last line and the keys, on a
    ///     phone, while you are typing. With this, 514 — the keyboard's own
    ///     top edge, and no band.
    ///
    /// All three are from a booted iPhone 17 and `TerminalScrollTests` asserts
    /// them. Between them they pin the identity this expression rests on —
    /// the stack's own inset is `max(displayBottom, keyboard)` — at 0, at 52
    /// and at 360, which is either side of `chrome.bottom` and therefore both
    /// branches of the outer `max`. That clamp is the point: a NEGATIVE
    /// `safeAreaInset` height is the second subtraction that once produced a
    /// 0-point-tall grid and a pane that drew nothing.
    ///
    /// **Which of the two keyboard states a run gets is not this code's
    /// choice.** XCUITest attaches a hardware keyboard to the simulator and
    /// does not always detach it, so the key row is sometimes docked on the
    /// home indicator and sometimes riding a full keyboard. That is why the
    /// test asserts on "the nearest thing over the pane" rather than on a
    /// number: it is the same sentence in both, and the flat expression fails
    /// it in both.
    ///
    /// `max` and not a subtraction of both, because the stack's own inset is
    /// itself a `max` of the two things it re-derives: with a keyboard up the
    /// home indicator is already inside the keyboard's reach and is not
    /// subtracted a second time. So the total the content ends up with is
    /// `max(chrome.bottom, keyboard)` — the furniture when nothing is over it,
    /// the keyboard when the keyboard is taller — which is what "the bar never
    /// moves and never hides behind the keyboard" means as a number. Clamped
    /// at zero: a keyboard taller than the furniture wants no help from here.
    private var paneBottom: CGFloat {
        max(0, slot.chrome.bottom - max(displayBottom, keyboard.height))
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
            isVisible: slot.isVisible,
            onCreated: onCreated)
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
    /// Every runner this app is talking to, and the merged fleet across them.
    ///
    /// It was one `Connection`, and the whole screen read that one object: the
    /// fleet to map, the inbox to mark a diff with, the object to send an RPC
    /// down. A merged fleet has no such object — a pane on `gpu-box-2` and a
    /// pane on the laptop are two sessions — so every call site that used to
    /// reach for `connection` now resolves one from the pane it is about. See
    /// `connection(_:)`, which is the one place that resolution happens.
    @ObservedObject var fleet: FleetStore
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
    /// The other runners' worktrees. See `readElsewhere`.
    @State private var elsewhere: [ShellServerGroup] = []
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

    /// A terminal a pane's bar just made, until the shell has landed on it.
    ///
    /// The same two-phase dance as `pendingTerminal` and resolved through the
    /// same `requestedTab`: the id is known the moment the runner answers, and
    /// the tab it becomes does not exist until a poll has brought the fleet
    /// back. Held HERE rather than in the pane that asked, because a pane is
    /// destroyed and rebuilt by nothing more than a change of runner, and this
    /// has to outlive the refresh it is waiting for.
    ///
    /// Its own property and not a second writer of `pendingTerminal`, and that
    /// is the whole reason it exists: a deep link is not a choice and must not
    /// be written down as one (see `linkedTab`), while a terminal somebody just
    /// MADE is the plainest choice in the app. Sharing the binding would have
    /// the workspace forget, on the next visit, the very tab it was just told
    /// to open.
    @State private var createdTerminal: String?

    /// Describe it, or fill in the form. Both were `WorkspaceListView`'s
    /// toolbar and are the overview's now — see `overviewActions`.
    @State private var showQuickTask = false
    @State private var showNewWorkspace = false

    /// Where a removal started from an overview card's menu has got to.
    ///
    /// Held HERE and not in the grid, for the reason the two sheets below are
    /// presented here: the overview is mounted from the first point of a lift
    /// and unmounted again when the grid is neither showing nor flying, so a
    /// confirmation whose presenter lives inside it is a confirmation that can
    /// lose the view it is attached to. A removal outlives the card that asked
    /// for it — that is most of the point of asking.
    @State private var removing: RemoveWorktreeRequest?
    /// The runner a status row asked to correct, and whether this device's own
    /// key is on screen. Both held HERE rather than in the row for the reason
    /// the sheets above are: the overview is unmounted when the grid is neither
    /// showing nor flying, and a presenter that can go away is a sheet nobody
    /// can close.
    @State private var editingRunner: Runner?
    @State private var authorizingDevice = false

    @Environment(\.scenePhase) private var scenePhase

    /// The whole fleet, in the shell's vocabulary, rebuilt every poll.
    private var map: ShellFleetMap { ShellFleetMap.of(fleet) }

    // MARK: - Which runner a thing is on

    /// The connection a pane is on, or nil for a runner this app has stopped
    /// talking to.
    ///
    /// **The one place the resolution happens.** Nil is an ordinary answer and
    /// not an error: the battery gate can retire a runner while a pane of its
    /// is still mounted, and a screen that forced an answer here would be a
    /// screen sending one runner's RPC down another runner's session — the
    /// exact mistake `ShellPaneRef` gained a runner to prevent.
    private func connection(_ ref: ShellPaneRef?) -> Connection? {
        ref.flatMap { fleet.connection(for: $0.runner) }
    }

    /// The connection the pane AT REST is on.
    ///
    /// What the screen's own furniture reads — the toolbar's link chip, the two
    /// sheets that start work, the pull request on the Changes tab. Each of
    /// those is about the runner somebody is looking at, which is what "at
    /// rest" means, and none of them is about a pane.
    private var resting: Connection? { connection(restingRef) }

    /// The runner an action starts work ON.
    ///
    /// The one at rest — the runner whose worktree is on screen, which is the
    /// honest default and the answer the overview gives by putting these
    /// actions over its own cards. Falling back to the selection and then to
    /// the first runner that answered, for the one screen where there is no
    /// pane to rest on: a runner with no worktrees has nothing to look at and
    /// "New Workspace" is the only move on it. Without a fallback both sheets
    /// presented an empty body there — a sheet you can open and cannot use.
    ///
    /// `RunnerStore.selected` before list order, because it is persisted for
    /// exactly this question: `onSelectedRunner` uses it to decide where a
    /// launch LANDS, and the two must not answer differently.
    private var acting: Connection? {
        if let resting { return resting }
        if let selected = hosts.selected?.id,
            let picked = fleet.runners.first(where: { $0.host.id == selected })
        {
            return picked.connection
        }
        return fleet.runners.first { $0.connection.hasFleet }?.connection
    }

    var body: some View {
        Group {
            switch opening {
            case .pane(let initial):
                shell(map, from: initial)
            case .waiting:
                // Before the first fleet there is no position to open on, and
                // an empty shell would be a bar naming a workspace that does
                // not exist. The same gap `FleetView`'s connected branch
                // covers with a spinner — and a spinner is honest here only
                // because `opening` has already ruled out the case where
                // nothing is coming. See `ShellBringUp`.
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Themes.shared.current.backgroundColor.ignoresSafeArea())
            case .noWorkspaces:
                bringUp
            }
        }
        .onAppear {
            seed()
            elsewhere = readElsewhere()
        }
        // A fleet with a PANE in it is a fleet to open on, and that is not the
        // same question as whether a runner has answered.
        //
        // It used to be `fleet.hasFleet`, which is set by the first successful
        // refresh regardless of what came back and is never cleared. So a
        // runner with no worktrees flipped it, `seed` ran against an empty
        // merge and found nothing, and there was no second edge to fire on —
        // creating the first worktree changed nothing this screen was
        // watching, and the spinner stayed up for the life of the process.
        // `seed` runs once and only once on its own guard, so watching the
        // stronger signal costs nothing beyond the guard it already has.
        .onChange(of: openable) { _, _ in seed() }
        // The runner list changing changes which runners are somewhere else.
        // It used to be read once per mount and that was correct then: the
        // screen was destroyed and rebuilt on every change of runner, and
        // nothing in the process could write another runner's entry. Both
        // halves have stopped being true — the screen survives a crossing now,
        // and every live connection writes its own directory — so this is read
        // again whenever the set of live runners moves. See `readElsewhere`.
        .onChange(of: liveRunners) { _, _ in elsewhere = readElsewhere() }
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
        // On the runner at REST, which is the one being looked at.
        //
        // Both sheets used to have no such question to answer: there was one
        // connection and a new worktree could only go on it. With a merged
        // fleet "start some work" has to name a machine, and the honest default
        // is the one whose worktree is on screen — the same answer the overview
        // gives by putting these actions in its own toolbar, over its own
        // cards. Absent until the shell has come to rest at least once, which
        // is before the first frame anybody can tap.
        .sheet(isPresented: $showNewWorkspace) {
            if let connection = acting {
                NewWorkspaceView(
                    repositories: connection.repositories, connection: connection
                ) { repository, name, branch, adopt in
                    await connection.createWorkspace(
                        repository: repository, name: name, branch: branch, adopt: adopt)
                }
            }
        }
        .sheet(isPresented: $showQuickTask) {
            if let connection = acting {
                TaskComposerView(connection: connection)
            }
        }
        .sheet(item: $editingRunner) { runner in
            HostEditorView(
                existing: runner,
                onSave: { hosts.update($0) },
                onRemove: { hosts.remove($0) })
        }
        // This device's own key, and the one line to paste on the machine.
        //
        // A sheet and not a push, unlike the pre-fleet screen's. That screen
        // declares a `NavigationStack`; the shell deliberately declares none,
        // and the overview's own stack is inside a view that is unmounted the
        // moment the grid stops showing — so a push into it is a push whose
        // stack can vanish under it.
        .sheet(isPresented: $authorizingDevice) {
            NavigationStack { AuthorizeView(runners: hosts) }
        }
        // The ceremony a card's menu starts, run from the screen rather than
        // from the card. Shared with the pane's own bar — see
        // `RemoveWorktreeFlow`.
        //
        // On the connection the REQUEST carries, not on whatever is at rest: a
        // long press on a card in another runner's section is a removal on that
        // runner, and resolving it from the screen's own position would run the
        // ceremony against the wrong machine. `RemoveWorktreeRequest` carries
        // the connection for exactly this.
        .removeWorktreeFlow($removing)
    }

    /// The runner's workspace a card names, or nil when the fleet has moved on
    /// since the grid was built.
    ///
    /// Looked up by id rather than by index, and that is the whole reason the
    /// menu's closures carry a `ShellWorkspace` instead of a position: a poll
    /// between the long press and the tap can have taken a worktree away, and
    /// an index into a fleet that has changed length names a DIFFERENT
    /// workspace rather than none. `FleetView`'s removed-workspace rule, kept.
    private func workspace(_ shell: ShellWorkspace) -> (Workspace, Connection)? {
        // Through the map's own entry, which carries the runner this card is
        // on. Looking the id up in "the" fleet is what a single-connection
        // screen could do and a merged one cannot: `ShellWorkspace.id` is a
        // composite now, and searching every runner for the daemon's own id
        // would answer with the first match rather than with the right one.
        guard let entry = map.entries[shell.id],
            let connection = fleet.connection(for: entry.host.id),
            let live = connection.fleet.workspaces.first(where: { $0.id == entry.workspace.id })
        else { return nil }
        return (live, connection)
    }

    /// Put a worktree away, or take it back out.
    ///
    /// Fire-and-refresh, which is what `Connection.hideWorkspace` is: hiding
    /// is a view preference the runner stores, it cannot fail in a way this
    /// app could usefully say a sentence about, and the answer arrives as the
    /// card moving into — or out of — the grid's Hidden section. Nothing about
    /// where you ARE changes, because `isHidden` changes where a workspace is
    /// DRAWN and nothing else; see `ShellWorkspace.isHidden`.
    private func toggleHidden(_ shell: ShellWorkspace) {
        guard let (workspace, connection) = workspace(shell) else { return }
        Task {
            if workspace.isHidden {
                await connection.unhideWorkspace(workspace)
            } else {
                await connection.hideWorkspace(workspace)
            }
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
        // Both are about the runner AT REST — the one whose worktree is on
        // screen — and both are absent before the shell has come to rest, which
        // is before there is anything to look at. The chip in particular has to
        // be one runner's: it says "reconnecting" and offers a reconnect, and a
        // fleet-wide version of that sentence would be a chip that is amber
        // whenever any laptop anywhere is asleep. What says the same thing per
        // runner, next to the runner, is `RunnerStatusRow`.
        if let resting {
            RunnerMenu(hosts: hosts, connection: resting)

            if resting.phase != .connected {
                LinkStatusChip(connection: resting)
            }
        } else {
            RunnerMenu(hosts: hosts, connection: nil)
        }

        Button { showQuickTask = true } label: { Image(systemName: "sparkle") }
            .accessibilityLabel("Quick Task")

        Button { showNewWorkspace = true } label: { Image(systemName: "plus") }
            .accessibilityLabel("New Workspace")
    }

    /// The runners this app is CURRENTLY talking to.
    ///
    /// The trigger for re-reading the cache, and a value rather than a
    /// derivation at the point of use so `onChange` has something to compare.
    private var liveRunners: Set<String> {
        // Off the STORE's own key and not `Connection.hostId`, which is nil
        // until `start(host:)` has run. Reading it back off the connection made
        // a runner look absent for the whole of its bring-up — long enough for
        // `readElsewhere` to draw its cached worktrees beside its live ones,
        // and long enough for `takeCrossing` to throw away a crossing note
        // naming the very runner it was landing on. The same second-answer
        // mistake `FleetStore.publish` had.
        Set(fleet.runners.map(\.host.id.uuidString))
    }

    /// The worktrees on runners this app is NOT talking to.
    ///
    /// Read on the runners changing rather than on every body pass — see the
    /// `onChange` in `body`. It used to be read once per mount, and the reason
    /// given was that this app talks to one runner at a time so nothing in the
    /// process could change another runner's entry. That is exactly what
    /// stopped being true: every live connection writes its own directory now,
    /// and the screen is no longer destroyed when the runner changes. What has
    /// not changed is that a JSON decode on a `body` running three times a
    /// second would be waste — the set of live runners moves when somebody adds
    /// or removes one, which is not a per-poll event.
    private func readElsewhere() -> [ShellServerGroup] {
        let live = liveRunners
        let known = Set(hosts.hosts.map(\.id.uuidString))
        return RunnerDirectoryStore.read()
            // The live runner is excluded by ID, not by label: two entries can
            // name one box under different users, and a grid that showed the
            // runner you are ON as a cached section would draw every workspace
            // twice — once live, and once as it was thirty seconds ago.
            //
            // A runner somebody has since removed is excluded too. A card for
            // one would be a card whose tap can do nothing.
            .filter { !live.contains($0.runner) && known.contains($0.runner) }
            .map { $0.group() }
    }

    /// What this screen is: a pane, a wait, or a runner with nothing on it.
    ///
    /// The decision is `ShellBringUp.opening`, in AgentKit, and it is there
    /// because the branch it replaces was wrong for as long as it existed and
    /// looked exactly like a slow network. A connected runner with zero
    /// worktrees is an ordinary daemon state — a machine set up this morning
    /// and not used yet — and it used to be a full-bleed `ProgressView` with no
    /// navigation bar, no host switcher and no end: no way to reach settings,
    /// add a second runner, read this device's key, or make the first worktree.
    private var opening: ShellOpening {
        ShellBringUp.opening(
            seated: initial, workspaces: openableCount,
            reports: fleet.runners.map { report($0) })
    }

    /// How many worktrees the merge has to open on, counted without building
    /// the merge.
    ///
    /// `ShellFleetMap.of` walks every terminal of every workspace of every
    /// runner and this is read on every body pass, so it asks the connections
    /// directly. The two agree by construction: `of` appends one workspace per
    /// entry and gives each of them a Diff tab, so a workspace in the fleet is
    /// always a position in the shell.
    private var openableCount: Int {
        fleet.runners.reduce(0) { $0 + $1.connection.fleet.workspaces.count }
    }

    /// The same question as a flag, for the one place that only needs the edge.
    private var openable: Bool { openableCount > 0 }

    /// What one runner has said, in the bring-up rule's vocabulary.
    ///
    /// Wiring, and the only place this screen turns a `Phase` into one — the
    /// same seam `FleetView.standing` is for `StopWaiting`.
    ///
    /// `hasFleet` FIRST, because a connection is `.connected` a whole SSH round
    /// trip before its first `fleet` call returns and the answer is what this
    /// rule is about. After that the phase decides only whether anything more
    /// is coming: a runner holding a fingerprint question is waiting on a
    /// person, and a person cannot answer it from behind a spinner.
    private func report(_ runner: (host: Runner, connection: Connection))
        -> ShellBringUp.Report
    {
        if runner.connection.hasFleet { return .answered }
        switch runner.connection.phase {
        case .connecting, .reconnecting, .connected: return .pending
        case .needsApproval, .failed: return .stalled
        }
    }

    /// A runner that answered and has nothing on it.
    ///
    /// The rows first, then the sentence, then the ways out — which is the
    /// shape `FleetView.waitingForAnyone` already has, for the same reason: a
    /// screen you cannot leave is the defect, not the empty fleet. The copy is
    /// the overview's own, out of `ShellEmptyCopy`, so the two cannot drift.
    private var bringUp: some View {
        NavigationStack {
            VStack(spacing: 0) {
                ForEach(fleet.runners, id: \.host.id) { runner in
                    RunnerStatusRow(
                        connection: runner.connection,
                        host: runner.host,
                        showsLabel: true,
                        onRetry: { fleet.retry(runner.host.id) },
                        onReconnectNow: { runner.connection.reconnectNow() },
                        onTrust: { hosts.trust(runner.host, fingerprint: $0) },
                        onReviewKey: { hosts.forgetKey(runner.host) },
                        onNotNow: { runner.connection.declineHostKey(runner.host) },
                        onEdit: { editingRunner = runner.host },
                        onAuthorize: { authorizingDevice = true })
                }

                ContentUnavailableView {
                    Label(ShellEmptyCopy.title, systemImage: ShellEmptyCopy.symbol)
                } description: {
                    Text(ShellEmptyCopy.description(matching: ""))
                } actions: {
                    // The next move, and the only one this screen has: there is
                    // no card to open and no pane to swipe to. Offered against
                    // the runner the two sheets already resolve to — see
                    // `acting`, which is what lets them work before the shell
                    // has come to rest on anything.
                    Button("New Workspace") { showNewWorkspace = true }
                        .disabled(acting == nil)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Themes.shared.current.backgroundColor.ignoresSafeArea())
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HostSwitcherBar(hosts: hosts, connection: acting)
            }
            // "Runners", because that is what this screen is a list of, and
            // the same title `FleetView.escapable` gives the screen before it.
            // The device-key sheet is presented on the `Group` above and
            // therefore already reaches this branch; a second presenter here
            // would be two of them for one flow.
            .navigationTitle("Runners")
            .navigationBarTitleDisplayMode(.inline)
        }
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
                    guard taken == nil else { return }
                    // Only the LINK arms `linkedTab`. Whatever rest a deep link
                    // produces is not a choice and must not be written down as
                    // one; a terminal this app was just asked to make is one,
                    // and the workspace should remember it. See `linkedTab` and
                    // `createdTerminal`.
                    if pendingTerminal != nil {
                        linkedTab = requestedTab(in: map)
                        pendingTerminal = nil
                    }
                    createdTerminal = nil
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
            // The header over the live cards names the runner, but only when
            // there is one runner for it to name. With several merged into one
            // grid there is no single answer, and each card carries its own —
            // see `ShellFleetMap.one`, which is where that condition is
            // decided. A header saying one machine's name over another
            // machine's worktrees is the one thing worse than no header.
            liveServer: fleet.runners.count == 1 ? fleet.runners[0].host.label : nil,
            elsewhere: elsewhere,
            // A row for every runner that is not simply answering, over the
            // grid it is about. This is the whole of what replaced four
            // full-screen phases: a laptop asleep in another room is a line of
            // text above the cards rather than a screen in front of them, and a
            // runner that has never been approved shows its fingerprint here
            // instead of showing it to nobody because some other runner
            // answered. See `RunnerStatusRow`.
            runners: {
                ForEach(fleet.runners, id: \.host.id) { runner in
                    RunnerStatusRow(
                        connection: runner.connection,
                        host: runner.host,
                        onRetry: { fleet.retry(runner.host.id) },
                        onReconnectNow: { runner.connection.reconnectNow() },
                        onTrust: { hosts.trust(runner.host, fingerprint: $0) },
                        onReviewKey: { hosts.forgetKey(runner.host) },
                        onNotNow: { runner.connection.declineHostKey(runner.host) },
                        onEdit: { editingRunner = runner.host },
                        // The overview has a `NavigationStack` of its own, so
                        // this one CAN push — unlike the row over the shell's
                        // pre-fleet screen, which hands the same move back as a
                        // flag. `RunnerStatusRow` takes a callback rather than
                        // a link precisely so both placements are possible.
                        onAuthorize: { authorizingDevice = true })
                }
            },
            // **No alert.** A cached card is only ever drawn for a runner this
            // app is not connected to, which with "Connect every runner at
            // once" on is no runner at all: every worktree in the grid is live
            // and reaching one is a swipe. What remains is the gate turned OFF,
            // where a tap is a request to talk to that runner instead — the
            // thing the setting says the app does. `shellCrossingAlert` used to
            // stand here and was telling the truth while `RootView` keyed the
            // tree `.id(host)`: the tap destroyed the screen, the track and
            // every mounted pane. It does not any more, and an alert warning
            // about a teardown that no longer happens is worse than no alert.
            onCross: { group, workspace in
                select(runner: group.id, landingOn: workspace.id)
            },
            onToggleHidden: toggleHidden,
            onRemoveWorktree: { shell in
                guard let (workspace, connection) = workspace(shell) else { return }
                removing = .confirming(workspace, on: connection)
            },
            overviewActions: { overviewActions }
        ) { slot in
            // **The pane resolves its own runner.** A slot names a tab, the map
            // says which pane that is and which runner it is on, and this is
            // where the connection to talk to it over comes from. A pane whose
            // runner has been retired draws nothing rather than borrowing
            // another runner's session.
            if let ref = map.refs[slot.tab.id], let connection = connection(ref) {
                ShellPaneRealView(
                    slot: slot, ref: ref, connection: connection, pastes: pastes,
                    // Only the workspace at rest gets an answer. A neighbour's
                    // diff header can wait until you land on it; asking for
                    // three is three GitHub round trips per swipe.
                    // The WHOLE ref, runner included. What "the same worktree"
                    // means across a merged fleet is a runner and an id, and a
                    // comparison on the id alone would rest on cross-daemon
                    // uniqueness that nothing enforces.
                    pullRequest: ref.workspace == restingRef?.workspace
                        && ref.runner == restingRef?.runner ? pullRequest : nil,
                    onCreated: { createdTerminal = $0 })
            }
        }
        // Over the panes, above the key row, and gone the moment the path is
        // typed. Nothing about a transfer is ever written into the pane itself.
        .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
    }

    /// Talk to this runner instead, and land on the worktree that was tapped.
    ///
    /// Only reachable with "Connect every runner at once" turned off, because
    /// that is the only setting under which a runner's worktrees are in the
    /// grid as a memory rather than as panes. It is not a teardown of this
    /// screen any more — `RootView` stopped keying the tree on the selected
    /// runner — but it IS a teardown of the other runner's connection, which is
    /// what one-runner-at-a-time means.
    ///
    /// `UserDefaults` and not `@State` for the note, and it still has to be:
    /// the new runner's fleet does not exist yet, so the workspace it names can
    /// only be resolved on the far side of a connect. `seed` reads it back.
    private func select(runner: String, landingOn workspace: String) {
        guard let picked = hosts.hosts.first(where: { $0.id.uuidString == runner })
        else { return }
        UserDefaults.standard.set("\(runner)/\(workspace)", forKey: Self.crossingKey)
        hosts.selected = picked
    }

    /// Which worktree a crossing was aimed at, spelled `runner/workspace`.
    static let crossingKey = "shell.crossingTo"


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
        // A crossing names a worktree, so a crossing lands on it. Spent
        // whether or not it resolved: a note left standing would steer the
        // next launch of a runner somebody reached the ordinary way, which is
        // the self-fulfilling memory `remember(_:leaving:tab:)` refuses for
        // the same reason one paragraph down.
        let workspace = takeCrossing().flatMap { wanted in
            // Both halves. A crossing note names a runner AND a worktree —
            // that is what `crossingKey` writes — and honoring only the second
            // over a merged fleet would land on whichever runner's copy the
            // merge put first.
            map.fleet.workspaces.indices.first {
                let entry = map.entries[map.fleet.workspaces[$0].id]
                return entry?.workspace.id == wanted.workspace
                    && entry?.host.id.uuidString == wanted.runner
            }
        } ?? onSelectedRunner(in: map) ?? at.workspace
        initial = ShellPosition(
            workspace: workspace, tab: map.fleet.workspaces[workspace].resumeTab)
    }

    /// The first workspace on the runner somebody last picked.
    ///
    /// **The selection still decides where a launch LANDS, even though it no
    /// longer decides what is connected.** `RunnerStore.selected` is persisted
    /// for exactly this — its own comment says landing on whichever runner
    /// happened to be first in the list "would mean the app forgets where you
    /// were every time you close it" — and that argument survived the port
    /// intact. What changed is only that the other runners are on screen too
    /// rather than absent.
    ///
    /// Nil when the selection names a runner with nothing in the merge, which
    /// is a runner still connecting or one that is down. `ShellFleet.first` is
    /// the fallback then, because a shell has to open on something and the
    /// alternative is a spinner over a fleet that is already in hand.
    private func onSelectedRunner(in map: ShellFleetMap) -> Int? {
        guard let selected = hosts.selected?.id else { return nil }
        return map.fleet.workspaces.indices.first {
            map.entries[map.fleet.workspaces[$0].id]?.host.id == selected
        }
    }

    /// The worktree a crossing was aimed at, if it was aimed at THIS runner.
    ///
    /// Checked against the runner as well as read, because the note outlives
    /// the tap: an app killed between the alert and the connection would come
    /// back with a note about a runner somebody may no longer be on.
    private func takeCrossing() -> (runner: String, workspace: String)? {
        guard let note = UserDefaults.standard.string(forKey: Self.crossingKey) else { return nil }
        UserDefaults.standard.removeObject(forKey: Self.crossingKey)
        let parts = note.split(separator: "/", maxSplits: 1)
        // Checked against the runners that are LIVE rather than against the one
        // this screen is on, because there is no longer one it is on. An app
        // killed between the tap and the connection comes back with a note
        // about a runner it may no longer be talking to, and a note that names
        // nothing here is spent rather than honored.
        guard parts.count == 2, liveRunners.contains(String(parts[0])) else { return nil }
        return (String(parts[0]), String(parts[1]))
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
        guard let arrived, arrived.workspace == previous?.workspace,
            arrived.runner == previous?.runner
        else { return }
        // On the runner the pane is on. The memory is `Connection.lastFocus`,
        // which is per runner because a workspace id means nothing off the
        // machine that minted it.
        connection(arrived)?.rememberFocus(arrived.pane.focus, in: arrived.workspace)
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
    ///
    /// Two askers now, resolved the one way. `createdTerminal` is the same
    /// shape of request from inside the app — an id that is real before the tab
    /// is — and giving it a second resolver would be a second place for "the
    /// shell does not actually have that tab" to be got wrong.
    private func requestedTab(in map: ShellFleetMap) -> String? {
        guard let id = pendingTerminal ?? createdTerminal else { return nil }
        // Across every runner, because a card carries a terminal id and no
        // host: the URL is `…://terminal/<id>` and always has been, so the only
        // way to answer "which runner is that on" is to look. One connection
        // made the question invisible rather than answering it — a card about
        // another runner simply resolved to nothing and the tap opened the app
        // on whatever it would have opened on anyway. Now it lands.
        for entry in map.entries.values {
            guard
                let terminal = entry.workspace.terminals.first(where: { $0.id == id })
            else { continue }
            let tab = ShellFleetMap.tabID(
                runner: entry.host.id, workspace: entry.workspace.id, pane: Pane(terminal))
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
        let at = ref ?? restingRef
        Notifier.shared.visibleTerminal = at?.pane.terminal?.id
        // Claimed on the runner the pane is on, and only there. Every other
        // runner clears its own watch on its own next poll — `Connection.refresh`
        // has always ended with this call — so fanning out here would be N round
        // trips to say the same thing the polls are already saying.
        Task { await connection(at)?.markVisibleSeen() }
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
        guard let ref = restingRef, case .changes = ref.pane,
            let connection = connection(ref)
        else { return "" }
        let branch = connection.fleet.workspaces.first { $0.id == ref.workspace }?.branch ?? ""
        // The runner is in the key for the reason it is in every other id here:
        // what the key has to change on is "a different worktree", and across a
        // merged fleet a worktree is a runner and an id rather than an id.
        return "\(ref.runner)|\(ref.workspace)|\(branch)|\(connection.pollGeneration)"
    }

    /// Read `stack.get` for the resting worktree's branch.
    ///
    /// Here rather than in `ChangesView` for the two reasons
    /// `WorkspaceView.readPullRequest` gives, which have not changed: this
    /// view has the repository UUID the call takes, and `ChangesView` must not
    /// be handed a `Connection` at all.
    private func readPullRequest() async {
        guard let ref = restingRef, case .changes = ref.pane,
            let connection = connection(ref),
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
