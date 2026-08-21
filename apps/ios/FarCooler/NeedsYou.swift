import SwiftUI

/// What the phone opens onto: the things on this runner that want a person.
///
/// The app used to open into a terminal. `FleetView` picked one on connect —
/// `fleet.landingTerminal` — and the worktree list was only ever the fallback
/// for a runner with no terminals at all. That is the right front door for
/// exactly one of the four situations this app is used in: on the couch, about
/// to drive an agent. In the other three — in transit, standing with the phone
/// in hand, at the gym between sets — the first question is *what needs me*,
/// and a terminal is an answer to a question nobody asked. See
/// `docs/jobs-to-be-done.md`, where those are the Reassure and Unblock jobs.
///
/// ## Two row kinds, one list
///
/// **Blocked agents** come from the fleet this connection is already polling.
/// **Reviews** come from `changes.inbox`, which the daemon has answered since
/// the review surface landed and which this app used for one thing: drawing
/// `+82 -13` on a workspace header in `FleetList`. The rows are the same fact
/// at two different scales — an agent that stopped to ask, and a branch that
/// moved since anyone last read it — and both are "you, now", so they belong in
/// one list rather than in two screens somebody has to choose between.
///
/// A `done` agent is deliberately NOT a row here, and that is worth spelling
/// out because `AgentActivity.wantsAttention` includes it and this screen does
/// not. What a finished agent leaves behind is a diff, and a diff already has a
/// row: the review one, which says how big it is and opens what you would
/// actually read. Listing the agent as well would put two rows on the front door
/// for one piece of work, and the second of them would open a transcript rather
/// than the change. A `done` agent whose worktree is clean — one that answered a
/// question or investigated something — is reachable through Workspaces, which
/// is where the whole fleet still is.
///
/// ## Ordering
///
/// `rank` decides it. That number is computed on the host —
/// `farcooler_core::feed::rank`, blocked before done before working, oldest
/// first inside a tier — precisely so a widget with room for one agent and a
/// list with room for twelve cannot disagree about which one matters. Nothing
/// here re-scores. The one thing this adds is a tiebreak on the terminal id,
/// because ranks genuinely collide — two agents that entered the same tier in
/// the same second get the same number — and `sort` is not stable, so without
/// it two equally-ranked rows could swap places on any poll. A row that moves
/// under a finger already travelling toward it is a tap that lands on something
/// else.
///
/// Reviews follow every blocked row, in the order the fleet itself arrived in.
/// They have no rank of their own — `InboxRow` is a workspace's counts, not an
/// agent's state — and inventing one from the workspace's terminals would sort
/// a diff by how blocked some agent in the same worktree happens to be, which
/// is not a fact about the diff.
///
/// ## Hidden worktrees are not here
///
/// A hidden workspace is one the user asked not to see. `FleetList` keeps them
/// behind a disclosure triangle and marks the triangle when something under it
/// wants attention; the front door does neither, because a front door that
/// shows what you hid is not honoring the hiding. They are one tap away, in
/// Workspaces, exactly where they were.
@MainActor
struct NeedsYouView: View {
    @ObservedObject var connection: Connection
    /// The runner this list speaks for, and the only one it can speak for.
    ///
    /// Held whole rather than as a label string because the count is qualified
    /// with the runner's name — see `header`. This build is single-runner, and
    /// the merged-across-runners inbox is a later task; a number on this screen
    /// must not silently change meaning when that lands.
    let runner: Runner
    let store: RunnerStore

    /// One entry in the list.
    ///
    /// An enum rather than two `ForEach`es over two arrays, because the two
    /// kinds interleave by rule — blocked ahead of reviews — and two sections
    /// would draw a divider between them that says they are different lists.
    /// They are one list of one thing: work that is waiting on a person.
    enum Item: Identifiable {
        case blocked(workspace: Workspace, terminal: Terminal, ordinal: Int?)
        case review(workspace: Workspace, counts: InboxRow)

        /// Prefixed by kind. A workspace id and a terminal id are different
        /// namespaces today, and a collision between them would silently give
        /// two rows one identity — which SwiftUI resolves by drawing one of
        /// them.
        var id: String {
            switch self {
            case .blocked(_, let terminal, _): return "blocked:\(terminal.id)"
            case .review(let workspace, _): return "review:\(workspace.id)"
            }
        }
    }

    /// The workspaces this screen will speak about at all. See the note on
    /// hidden worktrees above.
    private var visible: [Workspace] {
        connection.fleet.workspaces.filter { !$0.isHidden }
    }

    private var items: [Item] {
        var blocked: [(rank: UInt32, item: Item)] = []
        for workspace in visible {
            let numbering = workspace.ordinals()
            for terminal in workspace.terminals where terminal.agent == .blocked {
                blocked.append(
                    (
                        terminal.sortRank,
                        .blocked(
                            workspace: workspace, terminal: terminal,
                            ordinal: numbering[terminal.id])
                    ))
            }
        }
        blocked.sort { ($0.rank, $0.item.id) < ($1.rank, $1.item.id) }

        let reviews: [Item] = visible.compactMap { workspace in
            // Both conditions, and they are not the same one. `hasDiff` is
            // "this branch differs from its base", which is true of every
            // worktree with work on it and stays true after you have read it.
            // `changedSinceReviewed` is the daemon's watermark — the cheap
            // two-syscall gate, never a fleet-wide `git status` — and it is
            // what makes this an INBOX rather than a list of every branch in
            // flight.
            guard let counts = connection.inbox[workspace.id],
                counts.changedSinceReviewed, counts.hasDiff
            else { return nil }
            return .review(workspace: workspace, counts: counts)
        }

        return blocked.map(\.item) + reviews
    }

    /// How many agents are mid-turn.
    ///
    /// This used to label the row at the bottom of this screen, which is the
    /// bug `workspacesRow` records. It now says exactly one thing, in exactly
    /// one place: the sentence under "Nothing Needs You", where it is a
    /// statement about the runner rather than a label on a door.
    ///
    /// Non-hidden only, to agree with the rest of this screen: `FleetList`
    /// draws hidden workspaces behind a disclosure triangle, and a count that
    /// included them would be a number you cannot find by counting rows.
    private var workingCount: Int {
        visible.flatMap(\.terminals).filter { $0.agent == .working }.count
    }

    var body: some View {
        List {
            Section {
                if items.isEmpty {
                    reassurance
                } else {
                    ForEach(items) { row(for: $0) }
                }
            } header: {
                // The count, qualified, and absent when there is nothing to
                // count — an empty section with "0 ON STUDIO" over the
                // reassurance would be the same sentence twice, once in
                // numbers.
                //
                // "3 on studio" rather than a bare "3", and this is task 4 of
                // the spec rather than decoration. The number can only ever
                // speak for the runner this `Connection` is attached to, and
                // when the merged inbox lands an unqualified 3 would silently
                // start meaning something else. The precedent is
                // `FleetSnapshot.complete`, which says whether a snapshot is
                // all the agents there are for exactly this reason: drawing a
                // partial fleet as the fleet asserts that the rest do not
                // exist.
                if !items.isEmpty {
                    Text("\(items.count) on \(runner.label)")
                }
            }

            Section {
                // A value link, so the tap APPENDS `.workspaces` to
                // `FleetView.path`. A terminal opened from that list appends
                // again and lands at depth 2, ON the list rather than in place
                // of it — which is the whole of the Back bug this row was the
                // reported route into.
                NavigationLink(value: Route.workspaces) { workspacesRow }
            } footer: {
                if let sentence = workspacesFooter {
                    Text(sentence)
                }
            }
        }
        // Let the theme's ground show through; a List paints an opaque
        // background of its own that would sit on top of it. The same reason
        // `WorkspaceListView` does it.
        .scrollContentBackground(.hidden)
        .refreshable { await connection.refresh() }
        .navigationTitle("Needs You")
        .navigationBarTitleDisplayMode(.inline)
        // The app's escape hatch, and on this screen also the thing that names
        // the runner. The title says what the list is about; this says whose
        // fleet it is, offers the switcher, and carries `LinkStatusChip` —
        // which is how a reconnect is visible without the list flinching. See
        // `HostSwitcherBar`.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            HostSwitcherBar(hosts: store, connection: connection)
        }
    }

    @ViewBuilder
    private func row(for item: Item) -> some View {
        switch item {
        case .blocked(let workspace, let terminal, let ordinal):
            // A value link: the tap appends `.terminal` to `FleetView.path`,
            // and appending is the whole navigation model here — see `Route`.
            //
            // This was a Button with a hand-drawn chevron, because the pane had
            // exactly ONE destination in the stack and a second
            // `navigationDestination` declared on this screen would have fought
            // it. A path has one destination per route TYPE, declared once at
            // the stack's root and usable from any depth, so neither worry
            // survives and the system draws the chevron again.
            NavigationLink(value: Route.terminal(id: terminal.id)) {
                blockedRow(workspace: workspace, terminal: terminal, ordinal: ordinal)
            }
            .accessibilityIdentifier("needs-you-terminal-\(terminal.id)")

        case .review(let workspace, let counts):
            // By value for the same reason, and by workspace ID rather than by
            // building the screen here: a route has to survive being written to
            // disk and read back into a different launch. `FleetView` resolves
            // it — and keeps this exactly the same review a `changes` pane in
            // that worktree would show, because the store is keyed by workspace
            // on `Connection`. See `ChangesStores`.
            NavigationLink(value: Route.review(workspace: workspace.id)) {
                reviewRow(workspace: workspace, counts: counts)
            }
            .accessibilityIdentifier("needs-you-review-\(workspace.id)")
        }
    }

    /// A blocked agent, drawn by the row the fleet list already uses.
    ///
    /// `TerminalRow` and nothing new. It is four bands — the label with a
    /// ticking clock, the signal line the host composed, the agent's last three
    /// utterances, and its running subagents — and a second way to draw an
    /// agent is a second chance for two screens to say different things about
    /// one pane. Its `TimelineView` also keeps the schedule that matters: one
    /// second for a row with a clock, an hour for one without, so idle rows do
    /// not redraw every second to show nothing new. Every row here has a clock,
    /// since blocked is one of the two states that has one.
    ///
    /// The only thing added is which worktree it is in. The fleet list answers
    /// that with a section header per workspace; this list is ordered by
    /// urgency and cuts across workspaces, so each row has to carry it.
    private func blockedRow(workspace: Workspace, terminal: Terminal, ordinal: Int?)
        -> some View
    {
        VStack(alignment: .leading, spacing: 3) {
            Text(workspace.task)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                // The width of `TerminalRow`'s state dot plus its gutter, so
                // the worktree name starts on the same column as the agent's
                // label directly under it rather than half a dot to its left.
                .padding(.leading, 18)
            TerminalRow(terminal: terminal, ordinal: ordinal)
        }
    }

    /// A branch that has moved since anyone read it.
    ///
    /// Laid out to line up with the blocked rows above it: a leading mark in
    /// the dot's column, then the name, then the detail. The counts get the
    /// same monospaced green-and-red treatment `FleetList`'s workspace header
    /// gives them, so `+82 -13` is the same shape wherever it appears.
    ///
    /// Deliberately not amber. That color is reserved across the widget, the
    /// Live Activity, the complication and this app for an agent that is
    /// waiting on an answer, so that a glance at any surface answers "does this
    /// need me" without reading a word. A diff waiting to be read is worth a
    /// row; it is not worth the color that means someone is stuck.
    private func reviewRow(workspace: Workspace, counts: InboxRow) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "doc.text.magnifyingglass")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .frame(width: 8)
                .padding(.top, 5)

            VStack(alignment: .leading, spacing: 3) {
                Text(workspace.task)
                    .font(.body)
                    .lineLimit(1)
                    .truncationMode(.middle)

                HStack(spacing: 4) {
                    Text("+\(counts.insertions)").foregroundStyle(.green)
                    Text("-\(counts.deletions)").foregroundStyle(.red)
                }
                .font(.caption.monospaced())

                // The main checkout has a branch like everything else, but
                // WHICH branch is not the useful fact about it — that it is the
                // repository itself is. `FleetList`'s header says the same
                // thing the same way.
                Text(workspace.isMainCheckout ? "Primary checkout" : workspace.branch)
                    .font(.caption2.monospaced())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "\(workspace.task), \(counts.insertions) added, \(counts.deletions) removed")
    }

    /// The most common state, and it is doing a job rather than filling a gap.
    ///
    /// "Is anything wrong?" is the question this app is opened with most often,
    /// and the honest answer is usually no. A blank screen answers it too, and
    /// answers it badly: an empty list is indistinguishable from a list that
    /// has not loaded, from a runner that stopped talking, and from a bug. So
    /// this says it in words, over a count of what is still running — which is
    /// the difference between "nothing needs you" and "nothing is happening".
    ///
    /// Inside the list rather than instead of it, so the Working row stays
    /// exactly where it is in both states and does not move to the middle of
    /// the screen and back as the last agent finishes.
    private var reassurance: some View {
        VStack(spacing: 10) {
            Image(systemName: "checkmark.circle")
                .font(.system(size: 34, weight: .thin))
                .foregroundStyle(.tertiary)
            Text("Nothing Needs You")
                .font(.title3.weight(.semibold))
            Text(reassuranceDetail)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 28)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .accessibilityElement(children: .combine)
    }

    /// Named runner and all, for the same reason the count is qualified: this
    /// screen can only speak for the runner it is connected to.
    private var reassuranceDetail: String {
        switch workingCount {
        case 0: return "Nothing is running on \(runner.label)."
        case 1: return "One agent is working on \(runner.label)."
        default: return "\(workingCount) agents are working on \(runner.label)."
        }
    }

    /// The door to the whole fleet, counting the thing that is actually behind
    /// it.
    ///
    /// This said "Working" over a count of agents mid-turn, and led to a screen
    /// listing every workspace, every terminal — idle, exited and hidden — and
    /// the two buttons that start new work. The number described a strict
    /// subset of the destination, so at 3am with nothing running it read
    /// `Working 0` on the only way in: a label telling you not to open the one
    /// door you needed.
    ///
    /// So it counts what the destination lists, and the destination is titled
    /// with the same word. Nothing is lost by dropping the old number, because
    /// the app was already saying it one line above, better: `reassuranceDetail`
    /// gives it as a sentence naming the runner, which is what a count of
    /// working agents is actually for.
    ///
    /// Hidden workspaces are left out, so this is a number you can find by
    /// counting rows — `FleetList` keeps them behind a disclosure triangle.
    private var workspacesRow: some View {
        HStack {
            Text("Workspaces")
            Spacer()
            Text("\(visible.count)")
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
        }
    }

    /// Why a count of zero is still worth a tap.
    ///
    /// A count can say "there is nothing behind this" and be right about the
    /// list while being wrong about the screen: the quick task and the
    /// new-workspace form live in that screen's toolbar and nowhere else on the
    /// phone, so an empty fleet is the state in which going there matters most.
    /// One sentence, and only in the state that needs it — a footer under a
    /// number that already speaks for itself is noise.
    ///
    /// Two sentences rather than one, because "no workspaces" over a list with
    /// rows in it would be the same kind of lie the count used to tell.
    private var workspacesFooter: String? {
        guard visible.isEmpty else { return nil }
        if connection.fleet.workspaces.isEmpty {
            return "No workspaces on \(runner.label) yet. This is where you start one."
        }
        return "Every workspace on \(runner.label) is hidden. They’re still in here."
    }
}

/// A worktree's changes, opened from the inbox instead of from a pane.
///
/// `ChangesView` was reachable one way: a `changes` pane the host had opened,
/// drawn by `TerminalView` and given its toolbar by `PaneHost`. That is the
/// right home for it while you are driving a workspace, and it is the wrong one
/// for the job this screen exists to serve — at the gym, where the thing you
/// came to do is read a branch and there is no reason to go through a pane to
/// reach it, or to need one to exist.
///
/// So this wraps the same view with the title and the toolbar item the pane
/// host supplies, and nothing else. The store is `Connection`'s, keyed by
/// workspace, so this and a `changes` pane on the same worktree are one review:
/// what has been read, what is folded, where you were, and the notes you have
/// written are all on the store rather than in either view.
///
/// A plain `let` rather than an `@ObservedObject`, deliberately. Both children
/// observe the store themselves; observing it here as well would re-evaluate
/// this wrapper — and therefore rebuild the forty-card lazy stack's enclosing
/// view — on every one of the store's many publishes. `ChangesView` carries the
/// same argument about why `agents` arrives as plain values rather than as the
/// `Connection` they were derived from.
struct ReviewScreen: View {
    let store: ChangesStore
    let workspaceName: String
    var agents: [ReviewAgentTarget] = []

    var body: some View {
        ChangesView(store: store, workspaceName: workspaceName, agents: agents)
            .navigationTitle(workspaceName)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    ChangesToolbarMenu(store: store)
                }
            }
    }
}
