import SwiftUI

// The navigation shell over a real runner: the fleet mapped onto the shell's
// vocabulary, and terminals in the slots.
//
// `ShellRootView` is generic over its pane and knows nothing about a runner —
// that is the seam this file sits on. Everything that is a fact about THIS app
// rather than about the gesture is here: which tabs a worktree has and in
// what order, what each mark means, which pane each tab draws, and the two
// pieces of state a mounted pane cannot own for itself — `isVisible` and
// `Notifier.shared.visibleTerminal`.

/// One worktree, as the phone's stack pushes it (ov-55, spec §6.1): the
/// shell over that worktree's panes and nothing else. `WorktreeScreen` and
/// the agent layout harness mount it.
struct ShellScope: Equatable {
    var runner: UUID
    /// The daemon's own worktree id.
    var worktree: String
    /// Which pane it opens on.
    var landing: WorktreeLanding
}

/// What one shell tab draws, and where.
///
/// A side table keyed by tab id rather than something encoded IN the id.
/// `ShellTab` lives in AgentKit, which cannot see `Terminal` — see that file's
/// header on why the shell's model is a shared package rather than shared code
/// — so the alternative is packing a worktree id and a pane id into one
/// string and parsing it back out at every use, which is a decoder nobody
/// wrote a test for standing between the fleet and the screen.
struct ShellPaneRef: Hashable {
    /// The runner this pane is on.
    ///
    /// Carried rather than assumed, and that is the whole of what the port
    /// changes here. A `ShellFleetMap` used to be one runner's fleet by
    /// construction — `RootView` keyed the tree `.id(host)`, so the screen was
    /// destroyed and rebuilt on every change of runner — and every ref in it
    /// named a worktree on the same machine, so the runner went without
    /// saying. The merged map holds several, and a ref that did not say which
    /// would send an RPC down whichever connection the screen happened to be
    /// holding. See `ShellIdentity`.
    var runner: UUID
    /// The DAEMON's own worktree id — the full UUID it minted, which is what
    /// goes on the wire in `changes.*` and `worktree.*` calls. Not the shell's
    /// composite, and not the eight-character `short` field, which is a display
    /// form nothing here uses as an identity.
    var worktree: String
    var pane: Pane
}

/// A terminal a swipe asked to close, the runner to close it on, and the words
/// to say first.
///
/// The connection travels WITH the request, exactly as `RemoveWorktreeRequest`
/// carries its own: a merged fleet has several runners in one column's reach,
/// and resolving the runner again when the button is tapped would resolve it
/// against whatever the shell has moved to since. The question travels too,
/// because it is timed — it names how long the agent has been going, and
/// rebuilding it when the dialog draws would re-read the clock and could find
/// the terminal already gone.
struct CloseTerminalRequest: Identifiable {
    let terminal: Terminal
    let connection: Connection
    let question: ShellClose.Question
    var id: String { terminal.id }
}

/// The fleet as the shell needs it, plus what each of its tabs is.
///
/// Built whole from the store on every poll and thrown away. Nothing here is
/// remembered: the retained set lives in `ShellPaneTrack` and is keyed by tab
/// id, so this value moving underneath it is exactly the case that was designed
/// for.
///
/// **It is the whole fleet and not one runner's**, which is the port's central
/// change. `of(_ store:)` walks `FleetStore.entries` — every worktree on every
/// connected runner, in the order the runner list is in — and every id it mints
/// carries the runner, so a tab resolves back to a connection rather than to a
/// search. See `ShellIdentity`, which is where that composition, its test and
/// the reason live.
@MainActor
struct ShellFleetMap {
    var fleet: ShellFleet
    var refs: [String: ShellPaneRef]
    /// Which runner each shell worktree is on, and what it said, keyed by the
    /// composite id `ShellWorktree.id` now carries.
    ///
    /// The side table that makes a merged fleet act-on-able. A screen holding a
    /// `ShellWorktree` has a name and a ribbon and nothing to send an RPC
    /// down; this is where it gets the connection, the runner and the daemon's
    /// own worktree id back. Keyed rather than searched, because the shell asks
    /// per rest.
    var entries: [String: FleetEntry] = [:]

    /// Every terminal's tab id, by terminal id, across every runner in the
    /// map — what `ShellFleet.landing` reads.
    ///
    /// Across every runner because a card carries a terminal id and no host:
    /// the URL is `…://terminal/<id>` and always has been, so the only way to
    /// answer "which runner is that on" is to look. A `changes` pane the host
    /// happens to have open is folded into the Changes tab by `Pane.init(_:)`
    /// and maps to that tab, which does exist.
    var tabOfTerminal: [String: String] {
        var out: [String: String] = [:]
        for entry in entries.values {
            for terminal in entry.worktree.terminals {
                out[terminal.id] = Self.tabID(
                    runner: entry.host.id, worktree: entry.worktree.id, pane: Pane(terminal))
            }
        }
        return out
    }

    /// A tab's id: the runner, the worktree it is in, then the pane it is.
    ///
    /// Composed by `ShellIdentity` rather than here, so the composition sits
    /// somewhere a test can read it back: that file is in AgentKit, which
    /// `swift test` runs, and this target's tests are compiled by CI and never
    /// executed.
    static func tabID(runner: UUID, worktree: String, pane: Pane) -> String {
        ShellIdentity.tab(
            runner: runner.uuidString, worktree: worktree, pane: pane.id)
    }

    /// Read the fleet the shell draws — the entries the store holds — as the
    /// shell's.
    ///
    /// Order is the store's.
    static func of(_ store: FleetStore) -> ShellFleetMap {
        of(store.entries)
    }

    /// The map itself, over the entries rather than the store that published
    /// them.
    ///
    /// The shell is mounted over one worktree (`ShellScope`), so `entries` is
    /// that worktree's entry and nothing else. What a runner's workspaces
    /// contribute is the title of each orchestrator's tab (`Fleet.orchestratorTitles`): in Main's checkout, Billing's manager is not one
    /// more terminal of Main's.
    static func of(_ entries: [FleetEntry]) -> ShellFleetMap {
        var map = ShellFleetMap(fleet: ShellFleet(worktrees: []), refs: [:])
        var worktrees: [ShellWorktree] = []
        // More than one runner in the map is what would make a worktree name
        // its machine. See `server` below.
        let servers = Set(entries.map(\.host.id)).count
        for run in byRunner(entries) {
            let connection = run[0].connection
            let orchestrators = connection.fleet.orchestratorTitles() ?? [:]
            for entry in run {
                let built = one(entry, naming: servers > 1, orchestrators: orchestrators)
                worktrees.append(built.worktree)
                for (id, ref) in built.refs { map.refs[id] = ref }
                map.entries[built.worktree.id] = entry
            }
        }
        map.fleet = ShellFleet(worktrees: worktrees)
        return map
    }

    /// `entries` in runs of one runner each, in the order the runners first
    /// appear. The store appends a runner at a time, so this is a split and
    /// never a reshuffle.
    private static func byRunner(_ entries: [FleetEntry]) -> [[FleetEntry]] {
        var order: [UUID] = []
        var runs: [UUID: [FleetEntry]] = [:]
        for entry in entries {
            if runs[entry.host.id] == nil { order.append(entry.host.id) }
            runs[entry.host.id, default: []].append(entry)
        }
        return order.compactMap { runs[$0] }
    }

    /// One entry, as a worktree and the refs of its tabs.
    ///
    /// `orchestrators` are a workspace's orchestrator terminals, by id, with
    /// the title their tab takes (`orchestratorTitles`). They are tabs here —
    /// this worktree is where their panes are — marked as orchestrators.
    private static func one(
        _ entry: FleetEntry, naming server: Bool, orchestrators: [String: String] = [:]
    ) -> (worktree: ShellWorktree, refs: [String: ShellPaneRef]) {
        var refs: [String: ShellPaneRef] = [:]
        let connection = entry.connection
        let runner = entry.host.id
        let worktree = entry.worktree
        let inbox = entry.counts
        // A host-side `changes` pane is not a tab of its own: it IS the
        // Changes tab, and both resolve to the same `ChangesStore`. Two
        // chips for one diff is what `Pane.init(_:)` exists to prevent.
        let terminals = worktree.terminals.filter { !$0.isChangesPane }

        // **Changes leads, then fleet order, and never `sortRank`.**
        //
        // The tab strip this replaced made the argument and it is worse
        // here rather than better: a ribbon is a MAP of the worktree, and
        // a map whose landmarks move when an agent goes from working to
        // blocked is not a map — you would have to read it every time
        // instead of remembering it. The diff leading means the one tab
        // that is always there is always at the same end.
        var tabs: [ShellTab] = [
            ShellTab(
                id: tabID(runner: runner, worktree: worktree.id, pane: .changes),
                title: "Changes",
                mark: diffMark(inbox))
        ]
        var order: [ShellPaneRef] = [
            ShellPaneRef(runner: runner, worktree: worktree.id, pane: .changes)
        ]

        for terminal in terminals {
            let pane = Pane(terminal)
            tabs.append(
                ShellTab(
                    id: tabID(runner: runner, worktree: worktree.id, pane: pane),
                    // An orchestrator's by its workspace, so the bar says
                    // whose it is: in Main's checkout, Billing's manager is
                    // not one more terminal of Main's.
                    title: orchestrators[terminal.id] ?? HarnessName.display(terminal.label),
                    mark: mark(of: terminal),
                    // The rank's own question, kept separate from the
                    // drawing's. See `ShellTab.wantsAttention`.
                    wantsAttention: terminal.agent.wantsAttention,
                    // Every terminal tab, and only a terminal tab. The Diff
                    // above defaults to false and must: it is synthesized here
                    // rather than being a pane, and it is what closing the last
                    // terminal in a worktree lands on.
                    closable: true,
                    isOrchestrator: orchestrators[terminal.id] != nil))
            order.append(ShellPaneRef(runner: runner, worktree: worktree.id, pane: pane))
        }

        for (tab, ref) in zip(tabs, order) { refs[tab.id] = ref }

        return (
            ShellWorktree(
                // The COMPOSITE, not the daemon's own id. This string is a
                // SwiftUI identity — `ShellPaneTrack` remembers which worktree
                // a retained pane belongs to by it — and it is what a screen
                // resolves a runner from. The daemon's own id is on the
                // `FleetEntry` in `entries`, which is where the wire gets it
                // back. See `ShellIdentity`.
                id: ShellIdentity.worktree(
                    runner: runner.uuidString, worktree: worktree.id),
                name: worktree.task,
                // The runner's name, but only where the fleet on screen has
                // more than one runner in it: a worktree that did not say
                // where it is would leave the one question a merged fleet
                // raises unanswered.
                server: server ? entry.host.label : nil,
                resume: resume(worktree, connection: connection, tabs: order),
                tabs: tabs)
                // "Can't say" for every claim about now while the runner isn't
                // answering: its fleet is the one read before the link went,
                // kept so the screen doesn't move. Connected, and read on this
                // link: see `Connection.isAnswering`.
                .said(answering: connection.isAnswering),
            refs
        )
    }

    /// The Diff tab's mark.
    ///
    /// **Only the Diff tab can be cyan, and that is a model fact rather than a
    /// style choice.** Unread-diff comes from `Connection.inbox`, and an
    /// `InboxRow` is a WORKTREE's counts, not an agent's state.
    /// `NeedsYou.swift:94-97` already refuses to invent a per-agent version of
    /// it — *"inventing one from the worktree's terminals would sort a diff
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
    /// agent used, so every worktree's Diff tab would have started drawing a
    /// filled dot the moment fill began to mean something. And it is what tells
    /// an unread diff from a finished turn, since both are `.toReview` and only
    /// one of them is an agent.
    private static func diffMark(_ inbox: InboxRow?) -> GlanceMark {
        guard let inbox, inbox.changedSinceReviewed, inbox.hasDiff else {
            return GlanceMark(attention: .quiet, core: nil)
        }
        return GlanceMark(attention: .toReview, core: nil)
    }

    /// One agent's mark: `GlanceMark(terminal:)`, the rule AgentKit and
    /// Android share through `test/fixtures/glance-in-app.json`.
    ///
    /// Never dashed for age. This used to dash a quiet ring once
    /// `activityChangedAt` was an hour old, but that is when the state began,
    /// not when the runner was last heard from, so every agent working or idle
    /// for over an hour read "Can't say" on a live link. Whether the runner is
    /// answering is the whole of it, and the caller applies that with
    /// `said(answering:)`.
    static func mark(of terminal: Terminal) -> GlanceMark {
        GlanceMark(terminal: terminal)
    }

    /// Which tab this worktree should be REOPENED on.
    ///
    /// The one memory the app already keeps — `Connection.lastFocus`, written
    /// only by a person choosing a tab — resolved against the tabs that exist
    /// right now. A second memory living in the shell would be a second thing
    /// to disagree with it.
    ///
    /// Degrades rather than guessing. A remembered agent that has since exited
    /// falls through to `PaneFocus.rule(for:inbox:)`, which is this app's
    /// existing answer to "which pane should this worktree open on" — blocked
    /// agent, then unread diff, then top-ranked pane, then Changes — and a
    /// rule that answers with a pane this worktree does not have falls
    /// through to the first tab, which is the diff and always exists.
    ///
    /// Read by the bar and by arrivals that name a worktree, and deliberately
    /// not by the content swipe. See `ShellWorktree.resume`.
    private static func resume(
        _ worktree: Worktree, connection: Connection, tabs: [ShellPaneRef]
    ) -> Int? {
        var wanted: PaneFocus = connection.lastFocus[worktree.id] ?? .none
        if case .agent(let id) = wanted,
            !worktree.terminals.contains(where: { $0.id == id })
        {
            wanted = .none
        }
        if case .none = wanted {
            wanted = PaneFocus.rule(
                for: worktree, inbox: connection.inbox[worktree.id])
        }
        let pane: Pane
        switch wanted {
        case .changes, .none:
            pane = .changes
        case .agent(let id):
            guard let terminal = worktree.terminals.first(where: { $0.id == id }) else {
                return nil
            }
            pane = Pane(terminal)
        }
        return tabs.firstIndex { $0.pane.id == pane.id }
    }
}

/// One pane of a real worktree.
///
/// The two branches are the two things a worktree is: a pane on the runner,
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

    private var worktree: Worktree? {
        connection.fleet.worktrees.first { $0.id == ref.worktree }
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
        return connection.terminal(terminal.id, in: ref.worktree) ?? terminal
    }

    var body: some View {
        // One `NavigationStack` PER PANE, and none anywhere else in the shell.
        //
        // The shell has no navigation of its own — Phase 3 took the app's
        // single stack out — so this is
        // the pane borrowing the platform's chrome for the pane's own
        // controls, inside the pane, travelling with it on the track. See
        // `ShellPaneBar.swift`, which is the whole argument, and note the two
        // things it must not disturb: the track's geometry (a stack inside a
        // pane is invisible to `ShellPaneTrack`, which sizes every pane to
        // `page` × full height and offsets it).
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
            runner: ref.runner,
            connection: connection,
            pastes: pastes,
            worktree: worktree,
            live: live,
            changes: terminal == nil
                ? connection.changesStores.store(for: ref.worktree) : nil,
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
                // The store comes from `Connection`, keyed by worktree, so
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
                    store: connection.changesStores.store(for: ref.worktree),
                    worktreeName: worktree?.task ?? "Worktree",
                    agents: worktree?.reviewAgentTargets() ?? [],
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
/// - The pull request on the Changes tab's header, read for the worktree at
///   rest and only while its diff is the pane on screen.
/// - **The shell's claim on `Notifier.shared.visibleTerminal`.** Read by
///   `Notifications.swift` to suppress a banner about the pane you are
///   already looking at, and by `Connection.markVisibleSeen()` to claim the
///   runner's ten-second watch. Claimed through `Notifier.claim`, fed by one
///   callback, fired when the pane AT REST changes and at no other time —
///   never mid-gesture, when two panes are on screen and neither has
///   arrived. `Notifier` is the one writer; a pane's own mount and the
///   workspace's orchestrator claim through it too.
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
    /// The runners this device knows. Read for the selection only: `follow`
    /// records which runner somebody last moved onto.
    @ObservedObject var hosts: RunnerStore
    /// The terminal a tapped Live Activity card asked for, held by `FleetView`
    /// until this runner's fleet has it. See `requestedTab`.
    @Binding var pendingTerminal: String?
    /// The one worktree this shell is about.
    let scope: ShellScope

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
    /// What GitHub says about the branch of the worktree at rest.
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
    /// the worktree forget, on the next visit, the very tab it was just told
    /// to open.
    @State private var createdTerminal: String?

    /// The terminal a swipe asked to close, waiting on an answer.
    ///
    /// Held here rather than in `ShellBar`, and for a reason of its own: the
    /// column that raised it FURLS. Landing on a row clears `columnPinned`, so a
    /// dialog presented from inside `ShellBar` would be a dialog whose presenter
    /// is a surface that has since closed. What is being confirmed is a runner
    /// and a terminal, and neither of those is the menu.
    @State private var closing: CloseTerminalRequest?

    @Environment(\.scenePhase) private var scenePhase

    /// The whole fleet, in the shell's vocabulary, rebuilt every poll.
    private var map: ShellFleetMap { ShellFleetMap.of(scoped) }

    /// The store's entries this shell draws: the one worktree it's scoped to.
    private var scoped: [FleetEntry] {
        fleet.entries.filter {
            $0.host.id == scope.runner && $0.worktree.id == scope.worktree
        }
    }

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

    var body: some View {
        Group {
            switch opening {
            case .pane(let initial):
                if openableCount == 0 {
                    worktreeGone
                } else {
                    shell(map, from: initial)
                }
            case .waiting:
                // Before the first fleet there is no position to open on, and
                // an empty shell would be a bar naming a worktree that does
                // not exist. The same gap `FleetView`'s connected branch
                // covers with a spinner — and a spinner is honest here only
                // because `opening` has already ruled out the case where
                // nothing is coming. See `ShellBringUp`.
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Themes.shared.current.backgroundColor.ignoresSafeArea())
            case .noWorktrees:
                worktreeGone
            }
        }
        .onAppear { seed() }
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
        .task(id: pullRequestKey) { await readPullRequest() }
        .onChange(of: scenePhase) { _, phase in
            // Coming back to the app is reading whatever it comes back to.
            guard phase == .active else { return }
            markVisible()
        }
        // Only this shell's own claim: a claim made since, by a screen
        // this one was covering, isn't this one's to give back.
        .onDisappear {
            if let id = restingRef?.pane.terminal?.id { Notifier.shared.release(id) }
        }
        #if DEBUG
        .overlay(alignment: .topLeading) { watchProbe }
        #endif
        // The one thing a swipe on a column row can raise, and only for a pane
        // with something in it to lose.
        //
        // A `confirmationDialog` and not an `alert`, which is the same choice
        // `RemoveWorktreeFlow` makes for the same reason: this is a destructive
        // action confirmed from a gesture, and the platform's action sheet is
        // where a destructive confirmation belongs — it puts the red verb under
        // the thumb that swiped and Cancel below it.
        //
        // `presenting:` rather than reading `closing` back inside the button,
        // for the reason the removal flow writes down: a dialog hands its
        // buttons the value it was BUILT with, and read at tap time the request
        // has already been cleared by the same tap's dismissal — so the button
        // would quietly do nothing.
        .confirmationDialog(
            closing?.question.title ?? "",
            isPresented: Binding(
                get: { closing != nil }, set: { if !$0 { closing = nil } }),
            titleVisibility: .visible,
            presenting: closing
        ) { request in
            Button(ShellClose.confirm, role: .destructive) {
                closing = nil
                Task { await request.connection.close(terminal: request.terminal) }
            }
            Button("Cancel", role: .cancel) { closing = nil }
        } message: { request in
            Text(request.question.message)
        }
    }

    /// A column row, swiped and closed.
    ///
    /// **Asks only where there is something to interrupt.** `ShellClose`
    /// answers nil for a pane whose process has already gone, and nil means
    /// close it now — a sheet in front of every close would be a tax charged on
    /// the harmless case to protect the rare one, which is the trade the ruling
    /// refused.
    ///
    /// **The LIVE terminal, not the one the tab was built from.** A
    /// `ShellPaneRef` carries a `Pane`, which holds the `Terminal` the daemon
    /// described when this tab was made — a snapshot, and the two things this
    /// function needs from it are exactly the two that change: whether it is
    /// still running, and how long it has been going. Confirming against a
    /// snapshot would name an agent that finished four polls ago.
    ///
    /// **And it chooses no next tab.** See `ShellRootView.onCloseTab`: the
    /// vanish rule already exists, runs on every poll, and is the one this
    /// shell is anchored to.
    private func close(tab: ShellTab, in map: ShellFleetMap) {
        guard let ref = map.refs[tab.id],
            let connection = connection(ref),
            let terminal = live(ref, on: connection)
        else { return }
        guard let question = ShellClose.question(about: terminal, at: Date()) else {
            Task { await connection.close(terminal: terminal) }
            return
        }
        closing = CloseTerminalRequest(
            terminal: terminal, connection: connection, question: question)
    }

    /// The runner's current word on the pane a ref names, or nil once it is
    /// gone.
    ///
    /// Nil is ordinary rather than exceptional: a fleet shrinks under a finger,
    /// and a swipe landing on a tab a poll has already taken away is a swipe
    /// with nothing to do. It is also the guard that keeps the Changes tab out
    /// — a `changes` pane has no terminal id to match — though `ShellTab.closable`
    /// has already refused that row a Close button.
    private func live(_ ref: ShellPaneRef, on connection: Connection) -> Terminal? {
        guard let wanted = ref.pane.terminal else { return nil }
        return connection.fleet.worktrees
            .first { $0.id == ref.worktree }?
            .terminals.first { $0.id == wanted.id }
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
            seated: initial, worktrees: openableCount,
            reports: fleet.runners.map { report($0) })
    }

    /// Whether the scoped worktree is there to open on: 1, or 0 once its
    /// runner no longer lists it. Counted straight off the connection rather
    /// than by building the map, which walks every terminal of the worktree
    /// and is read on every body pass.
    ///
    /// The two agree by construction: `ShellFleetMap.of` appends one worktree
    /// per entry and gives it a Changes tab, so a worktree in the fleet is
    /// always a position in the shell.
    private var openableCount: Int {
        fleet.connection(for: scope.runner)?.fleet.worktrees
            .contains { $0.id == scope.worktree } == true ? 1 : 0
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

    /// A scoped shell whose worktree has gone: removed, or its runner no
    /// longer lists it. Back is the way out.
    private var worktreeGone: some View {
        NavigationStack {
            ContentUnavailableView {
                Label("Worktree Gone", systemImage: "folder.badge.questionmark")
            } description: {
                Text("This worktree isn’t on its runner anymore.")
            }
            .modifier(PhoneBackItem())
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
                    // and the worktree should remember it. See `linkedTab` and
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
            onRest: { at, arrival in
                let tab = map.fleet.tab(at: at)
                let arrived = tab.flatMap { map.refs[$0.id] }
                // Read before `remember` spends it: a rest the link asked for
                // is a move as far as the shell can tell, and only this screen
                // took the request.
                let linked = tab != nil && tab?.id == linkedTab
                remember(arrived, leaving: restingRef, tab: tab?.id)
                restingRef = arrived
                markVisible(arrived)
                follow(arrived, as: linked ? .linked : arrival)
            },
            // The worktree is ignored: a tab id already names its runner and
            // its worktree — that is what `ShellIdentity.tab` composes — and
            // `map.refs` is the lookup that gets both back. Taking the
            // worktree instead would be a second route to the same runner
            // that could disagree with the first.
            onCloseTab: { _, tab in close(tab: tab, in: map) }
        ) { slot in
            // **The pane resolves its own runner.** A slot names a tab, the map
            // says which pane that is and which runner it is on, and this is
            // where the connection to talk to it over comes from. A pane whose
            // runner has been retired draws nothing rather than borrowing
            // another runner's session.
            if let ref = map.refs[slot.tab.id], let connection = connection(ref) {
                ShellPaneRealView(
                    slot: slot, ref: ref, connection: connection, pastes: pastes,
                    // Only the worktree at rest gets an answer. A neighbor's
                    // diff header can wait until you land on it; asking for
                    // three is three GitHub round trips per swipe.
                    // The WHOLE ref, runner included. What "the same worktree"
                    // means across a merged fleet is a runner and an id, and a
                    // comparison on the id alone would rest on cross-daemon
                    // uniqueness that nothing enforces.
                    pullRequest: ref.worktree == restingRef?.worktree
                        && ref.runner == restingRef?.runner ? pullRequest : nil,
                    onCreated: { createdTerminal = $0 })
            }
        }
        // Over the panes, above the key row, and gone the moment the path is
        // typed. Nothing about a transfer is ever written into the pane itself.
        .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
    }

    /// The selection follows where you MOVE, with every runner connected.
    ///
    /// `RunnerStore.selected` still decides where a launch LANDS — see
    /// `onSelectedRunner` — and the runner menu was the only way to set it.
    /// With the menu gone, a launch would land forever on whatever it last
    /// said. So a swipe or a tap onto another runner's worktree selects that
    /// runner, which is "open where I was" said about the runner.
    ///
    /// Which rests count is `ShellSelection.follows`, in AgentKit, and the
    /// answer is only a move: never the landing at launch (a race between
    /// runners, not a choice — following it made one slow launch permanent),
    /// never a re-seat, never a deep link, and never with one runner at a
    /// time.
    private func follow(_ arrived: ShellPaneRef?, as arrival: ShellArrival) {
        guard let arrived,
            ShellSelection.follows(
                arrival, everyRunnerAtOnce: FleetSettings.allRunnersAtOnce,
                arrived: arrived.runner.uuidString,
                selected: hosts.selected?.id.uuidString),
            let host = hosts.hosts.first(where: { $0.id == arrived.runner })
        else { return }
        hosts.selected = host
    }

    // MARK: - Where the shell opens

    /// Put the shell on a pane, once, from the first fleet that has the
    /// worktree.
    ///
    /// Once and only once, for `FleetView.restorePlace`'s reason: after this
    /// the position is whatever the person holding the phone has done with it,
    /// and a second pass on a later reconnect would be the app steering them
    /// somewhere they had already left.
    ///
    /// One worktree, on the pane it was opened for: `ShellScope.landing`
    /// picks the tab, and `ShellFleetMap.resume` has already resolved the
    /// worktree's remembered one for `.resume`.
    private func seed() {
        guard initial == nil else { return }
        let map = self.map
        guard !map.fleet.isEmpty else { return }
        let worktree = map.fleet.worktrees[0]
        var tab = worktree.resumeTab
        switch scope.landing {
        case .resume: break
        case .changes: tab = 0
        case .terminal(let id):
            if let wanted = map.tabOfTerminal[id],
                let index = worktree.tabs.firstIndex(where: { $0.id == wanted })
            {
                tab = index
            }
        }
        initial = ShellPosition(worktree: 0, tab: tab)
    }

    // MARK: - Remembering where you were

    /// Write down the tab somebody chose, and only that.
    ///
    /// **Only when the arrival did not change worktree**, which is exactly
    /// the pane host's `choose(_:)` rule arrived at from the shell's side. In
    /// that screen the one writer was a tap on a chip; here the equivalents
    /// are a swipe along the content within a worktree and a tap on a menu
    /// row, and both of them are moves BETWEEN this worktree's tabs.
    ///
    /// Arriving in a worktree records nothing, and that matters more here
    /// than it did there. A bar swipe, a carried lift and a tapped card all
    /// land on `ShellWorktree.resumeTab`, which for a worktree nobody has
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
        // can look exactly like one: a card naming a pane in the worktree
        // already on screen lands on a different tab of the same worktree,
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
        guard let arrived, arrived.worktree == previous?.worktree,
            arrived.runner == previous?.runner
        else { return }
        // On the runner the pane is on. The memory is `Connection.lastFocus`,
        // which is per runner because a worktree id means nothing off the
        // machine that minted it.
        connection(arrived)?.rememberFocus(arrived.pane.focus, in: arrived.worktree)
    }

    // MARK: - A card tapped from outside the app

    /// Which TAB the pending deep link names, in the fleet this pass mapped.
    ///
    /// The whole of the two-phase dance, as a derivation rather than as a
    /// sequence of steps. A caller holds the terminal id from the moment the
    /// URL arrives (`PhoneRoot` holds it as a destination now, and passes none
    /// here) — and this is nil for
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
    /// Two askers, resolved the one way. `createdTerminal` is the same shape of
    /// request from inside the app — an id that is real before the tab is — and
    /// giving it a second resolver would be a second place for "the shell does
    /// not actually have that tab" to be got wrong.
    private func requestedTab(in map: ShellFleetMap) -> String? {
        guard let id = pendingTerminal ?? createdTerminal else { return nil }
        return tab(forTerminal: id, in: map)
    }

    /// The shell's tab for a terminal, on whichever runner it is, or nil when
    /// the shell has no such tab.
    ///
    /// The decision is `ShellFleet.landing`, in AgentKit where `swift test`
    /// reaches it; `ShellFleetMap.tabOfTerminal` is the composition it reads.
    private func tab(forTerminal id: String, in map: ShellFleetMap) -> String? {
        map.fleet.landing(forTerminal: id, tabOfTerminal: map.tabOfTerminal)
    }

    // MARK: - The shell's claim on `visibleTerminal`

    #if DEBUG
    /// The one way a UI test can ask which pane the runner is told is being
    /// read: `watch=<terminal id>`, or `watch=` for none.
    ///
    /// `Notifier.visibleTerminal` itself, which is what
    /// `Connection.markVisibleSeen` claims and marks `terminal.seen` from,
    /// and not this screen's idea of it: `markVisible` is not its only writer
    /// (a pane's own mount task is another), and a probe of one writer passed
    /// while the other went on claiming a pane under the grid. `Notifier` is
    /// not observable, so it is sampled four times a second. A one-point
    /// element for the reason `ShellRootView.probe` is one.
    ///
    /// Debug builds only: the sampling redraws four times a second for as
    /// long as the shell is up, for a probe only a UI test reads.
    private var watchProbe: some View {
        TimelineView(.periodic(from: .now, by: 0.25)) { _ in
            Rectangle()
                .fill(Color.white.opacity(0.001))
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("shell-watch")
                .accessibilityValue("watch=\(Notifier.shared.visibleTerminal ?? "")")
        }
    }
    #endif

    /// Which pane the runner should believe is being read.
    ///
    /// Nil on the Changes tab, and that is the honest answer rather than a
    /// gap: no pane is on screen, so no pane's notification should be
    /// suppressed and no agent's finished turn should be marked seen.
    /// `Connection.markVisibleSeen` reads exactly this and reports an empty
    /// watch list for it.
    ///
    private func markVisible(_ ref: ShellPaneRef? = nil) {
        let resting = ref ?? restingRef
        Notifier.shared.claim(resting?.pane.terminal?.id)
        // Claimed on the runner the pane is on, and only there. Every other
        // runner clears its own watch on its own next poll — `Connection.refresh`
        // has always ended with this call — so fanning out here would be N round
        // trips to say the same thing the polls are already saying. The resting
        // pane's runner even with the grid up, so its claim is given back now
        // rather than on that runner's next poll.
        Task { await connection(resting)?.markVisibleSeen() }
    }

    // MARK: - The Changes tab's header

    /// When to read the pull request again: on arriving at the Changes tab of
    /// a worktree, and on every fleet poll while it is up.
    ///
    /// Deliberately not a timer of this screen's own — see
    /// `WorkspaceView.pullRequestReadKey`, which this is carried across from,
    /// including the branch being in the key so a worktree that changes branch
    /// under you re-reads at once.
    private var pullRequestKey: String {
        guard let ref = restingRef, case .changes = ref.pane,
            let connection = connection(ref)
        else { return "" }
        let branch = connection.fleet.worktrees.first { $0.id == ref.worktree }?.branch ?? ""
        // The runner is in the key for the reason it is in every other id here:
        // what the key has to change on is "a different worktree", and across a
        // merged fleet a worktree is a runner and an id rather than an id.
        return "\(ref.runner)|\(ref.worktree)|\(branch)|\(connection.pollGeneration)"
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
            let worktree = connection.fleet.worktrees.first(where: { $0.id == ref.worktree })
        else { return }
        guard let repository = worktree.repository, !worktree.branch.isEmpty else {
            pullRequest = nil
            return
        }
        // A failed read leaves the last answer on screen rather than blanking
        // the row. The link dropping is not news about this pull request.
        guard let reply = await connection.stack(
            repository: repository, branch: worktree.branch)
        else { return }
        let mine = reply.links.first { $0.branch == worktree.branch }
        pullRequest = BranchPullRequest(
            pr: mine?.pr, known: reply.prAnswered, repoURL: reply.repoUrl)
    }
}

extension Connection {
    /// The panes working `row` on this runner, in fleet order, each named
    /// the way a menu item needs: the pane, then the worktree it is in.
    func boardAgents(for row: TaskRow) -> [BoardAgent] {
        let found = fleet.worktrees.flatMap { worktree in
            let ordinals = worktree.ordinals()
            return row.livePanes(in: worktree.terminals).map { terminal in
                (
                    terminal: terminal,
                    title: "\(HarnessName.display(terminal.displayName(ordinal: ordinals[terminal.id]))) in \(worktree.task)"
                )
            }
        }
        let titles = TaskAgentLink.menuTitles(found.map(\.title), shorts: found.map(\.terminal.short))
        return zip(found, titles).map { pane, title in
            BoardAgent(
                id: pane.terminal.id, title: title,
                mark: ShellFleetMap.mark(of: pane.terminal).said(answering: isAnswering))
        }
    }

    /// The pane the subagents on `row` live in: the orchestrator's, which is
    /// where a Subagent control goes (ov-213). Nil when the runner didn't say
    /// which pane, or it has closed since, so the control is not a button
    /// that leads nowhere.
    func orchestratorAgent(for row: TaskRow) -> BoardAgent? {
        guard let id = row.orchestratorTerminalID else { return nil }
        for worktree in fleet.worktrees {
            guard let terminal = worktree.terminals.first(where: { $0.id == id }),
                TaskAgentLink.liveStates.contains(terminal.boardState)
            else { continue }
            return BoardAgent(
                id: id, title: "Orchestrator in \(worktree.task)",
                mark: ShellFleetMap.mark(of: terminal).said(answering: isAnswering))
        }
        return nil
    }
}

/// A runner and its connection, for the one sheet that needs both and is
/// presented by item: `Connection` is a class with no identity of its own to
/// offer `sheet(item:)`.
struct RunnerSheet: Identifiable {
    let host: Runner
    let connection: Connection
    var id: UUID { host.id }
}
