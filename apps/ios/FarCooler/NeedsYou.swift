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
/// ## The row is a workspace
///
/// It was two row kinds — a blocked agent, and a worktree whose branch had
/// moved — and that produced FOUR rows for one piece of work when a workspace
/// held three blocked agents and an unread diff. The two kinds bought exactly
/// one thing between them: a different destination. Now that both open the same
/// screen they buy only a different starting TAB, which is a property of the
/// tap and not a reason for a row.
///
/// So one row per workspace: its name and branch, `+N -M` when the inbox says
/// there is a diff, and a `TerminalRow` for each blocked agent inside it. A
/// workspace that is both blocked and unread appears once, saying both.
///
/// **Blocked agents** come from the fleet this connection is already polling.
/// **The counts** come from `changes.inbox`, which the daemon has answered since
/// the review surface landed and which this app already used to draw `+82 -13`
/// on a workspace header in `FleetList`.
///
/// A `done` agent is deliberately NOT what puts a workspace on this list, and
/// that is worth spelling out because `AgentActivity.wantsAttention` includes it
/// and this screen does not. What a finished agent leaves behind is a diff, and
/// a diff already puts the workspace here through the second tier below. Whether
/// a `done` agent whose worktree is CLEAN — one that answered a question or
/// investigated something — belongs on the front door is an open decision, in
/// `needs-planning/done-agents-on-the-front-door.md`. The workspace row changes
/// its economics: such an agent would ride a row that already exists rather than
/// adding one, which was the objection to it. Today it is reachable through
/// Workspaces, where the whole fleet still is.
///
/// ## Ordering
///
/// Two tiers. **Blocked first**, a workspace ranked by the lowest `sortRank`
/// among its blocked agents; then **unread diffs**, workspaces where
/// `changedSinceReviewed && hasDiff` and nothing is blocked, in the order the
/// fleet arrived in.
///
/// `rank` decides the first tier. That number is computed on the host —
/// `farcooler_core::feed::rank`, blocked before done before working, oldest
/// first inside a tier — precisely so a widget with room for one agent and a
/// list with room for twelve cannot disagree about which one matters. Nothing
/// here re-scores. The one thing this adds is a tiebreak on the id, because
/// ranks genuinely collide — two agents that entered the same tier in the same
/// second get the same number — and `sort` is not stable, so without it two
/// equally-ranked rows could swap places on any poll. A row that moves under a
/// finger already travelling toward it is a tap that lands on something else.
///
/// The second tier has no rank of its own — `InboxRow` is a workspace's counts,
/// not an agent's state — and inventing one from the workspace's terminals would
/// sort a diff by how blocked some agent in the same worktree happens to be,
/// which is not a fact about the diff.
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

    /// One row: a workspace, and every reason it is on this screen.
    ///
    /// A struct rather than the two-case enum this replaced, because the two
    /// cases were two rows for one piece of work — see the note above. The
    /// tiers are an ordering over these, not a difference in kind.
    struct Item: Identifiable {
        let workspace: Workspace
        /// Its blocked agents, most urgent first. Empty on a row that is here
        /// only for its diff.
        let blocked: [Terminal]
        /// `Workspace.ordinals()`, computed once per row rather than once per
        /// agent inside it.
        let ordinals: [String: Int]
        /// What the branch changed, when the runner has said. Nil is "no answer
        /// yet", which is different from zero and is drawn as nothing either
        /// way.
        let counts: InboxRow?

        var id: String { workspace.id }
    }

    /// How many blocked agents a row shows before it starts counting them.
    ///
    /// Three, because each one is a `TerminalRow` up to four bands tall and a
    /// workspace with eight blocked agents would otherwise be the whole screen.
    /// What is lost by truncating is small: the tab strip on the other side of
    /// the tap holds every one of them, labelled.
    private static let agentsPerRow = 3

    /// The workspaces this screen will speak about at all. See the note on
    /// hidden worktrees above.
    private var visible: [Workspace] {
        connection.fleet.workspaces.filter { !$0.isHidden }
    }

    private var items: [Item] {
        var blocked: [(rank: UInt32, item: Item)] = []
        var unread: [Item] = []

        for workspace in visible {
            let counts = connection.inbox[workspace.id]
            let waiting = workspace.terminals
                .filter { $0.agent == .blocked }
                .sorted { ($0.sortRank, $0.id) < ($1.sortRank, $1.id) }
            let item = Item(
                workspace: workspace, blocked: waiting, ordinals: workspace.ordinals(),
                counts: counts)

            if let first = waiting.first {
                // The lowest rank among them, which `waiting` has already put
                // in front. A workspace is as urgent as its most urgent agent;
                // anything else — an average, a count — would let a worktree
                // with six working agents outrank one with a single agent that
                // has been stuck for an hour.
                blocked.append((first.sortRank, item))
            } else if let counts, counts.changedSinceReviewed, counts.hasDiff {
                // Both conditions, and they are not the same one. `hasDiff` is
                // "this branch differs from its base", which is true of every
                // worktree with work on it and stays true after you have read
                // it. `changedSinceReviewed` is the daemon's watermark — the
                // cheap two-syscall gate, never a fleet-wide `git status` — and
                // it is what makes this an INBOX rather than a list of every
                // branch in flight.
                unread.append(item)
            }
        }
        blocked.sort { ($0.rank, $0.item.id) < ($1.rank, $1.item.id) }

        return blocked.map(\.item) + unread
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

    /// One workspace, and every reason it is on this screen.
    ///
    /// ONE tap target for the whole row, and the tap carries no opinion about
    /// which tab opens — `Route.Focus.none` means "apply the rule", and the rule
    /// lands on the most urgent blocked agent, or the diff if there is an unread
    /// one, or the top pane. See `Route.Focus.rule(for:inbox:)`.
    ///
    /// One target rather than one per agent and one for the counts. The counts
    /// stay a label: a small target beside a large one, on a row somebody is
    /// tapping while walking, is a coin toss with a wrong side. And the agents
    /// inside are here to be READ — they are what the agent said it did, which
    /// is the larger half of reviewing its work — not to be aimed at. Every one
    /// of them is a labelled chip on the other side of the tap.
    private func row(for item: Item) -> some View {
        NavigationLink(value: Route.workspace(id: item.workspace.id, focus: .none)) {
            workspaceRow(item)
        }
        .accessibilityIdentifier("needs-you-workspace-\(item.workspace.id)")
    }

    /// The row's contents: what the worktree is, what it changed, and who in it
    /// is waiting.
    ///
    /// The counts get the monospaced green-and-red treatment `FleetList`'s
    /// workspace header gives them, so `+82 -13` is the same shape wherever it
    /// appears — and they are absent on a clean worktree, because `+0 -0` on
    /// every row is noise in the shape of information.
    ///
    /// Deliberately no amber anywhere on this row. That color is reserved across
    /// the widget, the Live Activity, the complication and this app for an agent
    /// waiting on an answer, and `TerminalRow` already spends it on exactly
    /// those, inside. A second mark up here would say the same thing twice and
    /// weaken it both times.
    private func workspaceRow(_ item: Item) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(item.workspace.task)
                        .font(.body)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    Spacer(minLength: 0)

                    if let counts = item.counts, counts.hasDiff {
                        HStack(spacing: 4) {
                            Text("+\(counts.insertions)").foregroundStyle(.green)
                            Text("-\(counts.deletions)").foregroundStyle(.red)
                        }
                        .font(.caption.monospaced())
                    }
                }

                // The main checkout has a branch like everything else, but
                // WHICH branch is not the useful fact about it — that it is the
                // repository itself is. `FleetList`'s header says the same
                // thing the same way.
                Text(item.workspace.isMainCheckout ? "Primary checkout" : item.workspace.branch)
                    .font(.caption2.monospaced())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            // `TerminalRow` and nothing new. It is four bands — the label with
            // a ticking clock, the signal line the host composed, the agent's
            // last three utterances, and its running subagents — and a second
            // way to draw an agent is a second chance for two screens to say
            // different things about one pane. Its `TimelineView` also keeps the
            // schedule that matters: one second for a row with a clock, an hour
            // for one without. Every agent here has a clock, since blocked is
            // one of the two states that has one.
            //
            // This is the part of the row that answers "what did it do", which
            // the owner is explicit is most of what reviewing an agent's work
            // is. It is worth the height.
            ForEach(item.blocked.prefix(Self.agentsPerRow)) { terminal in
                TerminalRow(terminal: terminal, ordinal: item.ordinals[terminal.id])
                    // Indented under the workspace, so the row reads as one
                    // thing containing several rather than as a heading that
                    // happens to sit above some agents.
                    .padding(.leading, 8)
            }

            if item.blocked.count > Self.agentsPerRow {
                Text(overflow(item.blocked.count - Self.agentsPerRow))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .padding(.leading, 26)
            }
        }
        .padding(.vertical, 2)
    }

    /// The agents this row ran out of room for. A sentence rather than a bare
    /// number, because "+2" under a list of agents reads as two more of
    /// something and does not say what.
    private func overflow(_ count: Int) -> String {
        count == 1 ? "1 more agent needs you" : "\(count) more agents need you"
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
