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
/// ## The unit is a workspace, and everything in it is a target
///
/// It was two row kinds — a blocked agent, and a worktree whose branch had
/// moved — and that produced FOUR rows for one piece of work when a workspace
/// held three blocked agents and an unread diff. The two kinds bought exactly
/// one thing between them: a different destination. Now that both open the same
/// screen they buy only a different starting TAB, which is a property of the
/// tap and not a reason for a row.
///
/// So one `Section` per workspace: its name and branch in the header, a
/// `TerminalRow` for each agent inside it that wants a person — blocked first,
/// then finished — and a "Review Changes" row for the diff. A workspace that is
/// blocked and finished and unread appears once, saying all three — and every
/// part of what it says is something you can tap, which is what `section(for:)`
/// is about.
///
/// **Blocked agents** come from the fleet this connection is already polling.
/// **The counts** come from `changes.inbox`, which the daemon has answered since
/// the review surface landed and which this app already used to draw `+82 -13`
/// on a workspace header in `FleetList`.
///
/// **A finished agent is here too, always.** This screen was the one surface in
/// the product that disagreed with `AgentActivity.wantsAttention`, on the
/// argument that what a finished agent leaves behind is a diff and a diff
/// already brings its workspace here through the second tier. That is true of
/// the finish that CHANGED something and false of the finish that did not — the
/// question answered, the investigation that touched no files, the work already
/// reviewed. In the owner's words, *“often I ask questions and stuff and I do
/// want to know when an answer comes back”*: an answer arriving is not an edge
/// case this screen tolerates, it is the thing it most needs to say. Decided in
/// `.claude/agent/done/done-agents-on-needs-you.md`.
///
/// Deliberately NOT conditioned on the workspace having no other news. "Show a
/// finished agent only when nothing else covers its workspace" is one condition
/// cheaper and it suppresses the answer exactly when a busy worktree makes it
/// hardest to notice. It also buys nothing under this shape: a finished agent is
/// another row inside a header that already exists, which is what retired the
/// objection this question used to turn on — that it meant two rows for one
/// piece of work.
///
/// The amber does not spread with it. `attentionColor` gives `done` green and a
/// failed turn red, and orange stays what it has always been across the widget,
/// the Live Activity, the complication and this app: an agent waiting on an
/// answer. A finished agent is not waiting on anyone, and spending one color on
/// both would weaken it for both.
///
/// ## Ordering
///
/// Two tiers, and the first one holds two kinds now. **Agents wanting attention
/// first**, a workspace ranked by the lowest `sortRank` among them; then
/// **unread diffs**, workspaces where `changedSinceReviewed && hasDiff` and no
/// agent wants anything, in the order the fleet arrived in.
///
/// **Blocked before finished, and this screen does not arrange it.**
/// `farcooler_core::feed::rank` already tiers blocked, then done — a failed turn
/// included, on purpose; see `feed::tier` — then working, then idle, each a
/// whole `TIER_SPAN` apart. So filtering on the shared `wantsAttention` and
/// taking the lowest rank in a workspace yields blocked-above-finished for free,
/// both across sections and inside one.
///
/// **A finished agent goes above an unread diff.** A diff sits still: it was
/// true before the app was opened and stays true until it is read. A finished
/// turn is news with somebody on the other end waiting for it, and it expires
/// the moment it is read — `seen(Done) → Idle`, so looking at it is what ends
/// it. The perishable thing goes above the durable one. Across sections that
/// falls out of the two tiers below; inside a section it is the literal order of
/// `section(for:)`'s groups.
///
/// `rank` decides the first tier. That number is computed on the host —
/// `farcooler_core::feed::rank`, blocked before done before working, oldest
/// first inside a tier — precisely so a widget with room for one agent and a
/// list with room for twelve cannot disagree about which one matters. Nothing
/// here re-scores, and that is what keeps the two kinds in one tier honest: a
/// second opinion about which agent matters is the exact thing that number
/// exists to prevent. The one thing this adds is a tiebreak on the id, because
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
    /// with the runner's name — see `subtitle`. This build is single-runner, and
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
        /// Its blocked agents, most urgent first. Empty on a section that is
        /// here only for a finished agent or for its diff.
        let blocked: [Terminal]
        /// Its finished agents — `done`, which is finished and unseen — in the
        /// host's order. Empty on a section that is here for something else.
        ///
        /// A separate array rather than more entries in `blocked`, because the
        /// two are counted and worded separately when a section runs out of
        /// room: "2 more agents need you" and "2 more agents finished" are not
        /// the same sentence, and one list of five would have to say one of
        /// them about both.
        let finished: [Terminal]
        /// `Workspace.ordinals()`, computed once per row rather than once per
        /// agent inside it.
        let ordinals: [String: Int]
        /// What the branch changed, when the runner has said.
        ///
        /// Nil is "no answer yet", and it is not zero. Both are drawn as
        /// nothing on the line — there are no numbers to show either way — but
        /// they no longer DECIDE the same thing: a worktree the runner has
        /// called clean gets no "Review Changes" row, and one it has not
        /// answered for yet does. See `section(for:)`.
        let counts: InboxRow?

        var id: String { workspace.id }
    }

    /// How many agents of ONE KIND a workspace shows before it starts counting
    /// them.
    ///
    /// Three, because each one is a `TerminalRow` up to four bands tall and a
    /// workspace with eight blocked agents would otherwise be the whole screen.
    /// What is lost by truncating is small: the tab strip on the other side of
    /// the tap holds every one of them, labelled.
    ///
    /// Unchanged by giving each of them its own row. The cap is about density,
    /// and density is precisely what this shape already spends: a workspace
    /// that used to be one row is now a header, some agents and a diff.
    /// Raising the cap here would spend it twice.
    ///
    /// Spent PER KIND rather than over the agents together, which is the one
    /// thing finished agents changed about it. A shared budget of three, spent
    /// blocked-first, would hide an answer behind three open questions in the
    /// same worktree — which is the narrow rule this screen rejected, arriving
    /// through the back door as a truncation. The cost is a worst case one row
    /// taller than before, in a workspace that already has three blocked agents
    /// and three finished ones.
    private static let agentsPerWorkspace = 3

    /// The workspaces this screen will speak about at all. See the note on
    /// hidden worktrees above.
    private var visible: [Workspace] {
        connection.fleet.workspaces.filter { !$0.isHidden }
    }

    private var items: [Item] {
        var attention: [(rank: UInt32, item: Item)] = []
        var unread: [Item] = []

        for workspace in visible {
            let counts = connection.inbox[workspace.id]
            // `wantsAttention` and not a list of cases, because that property
            // IS the product's single definition of what is worth interrupting
            // someone for — `activity.rs:825` on the host, `AgentActivity` here
            // — and this screen writing its own copy of the answer is how it
            // came to be the only surface that disagreed with it.
            //
            // Sorted once, by the host's rank, and then split. The split does
            // not reorder: `rank` puts every blocked agent a whole tier below
            // every finished one, so the two slices come out already in the
            // order they are drawn in.
            let wanting = workspace.terminals
                .filter { $0.agent.wantsAttention }
                .sorted { ($0.sortRank, $0.id) < ($1.sortRank, $1.id) }
            let item = Item(
                workspace: workspace, blocked: wanting.filter { $0.agent == .blocked },
                finished: wanting.filter { $0.agent == .done }, ordinals: workspace.ordinals(),
                counts: counts)

            if let first = wanting.first {
                // The lowest rank among them, which `wanting` has already put
                // in front. A workspace is as urgent as its most urgent agent;
                // anything else — an average, a count — would let a worktree
                // with six working agents outrank one with a single agent that
                // has been stuck for an hour.
                //
                // A workspace whose only news is a finished agent ranks by that
                // agent, which lands it below every blocked workspace and above
                // every unread diff without this line knowing which kind it is
                // holding.
                attention.append((first.sortRank, item))
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
        attention.sort { ($0.rank, $0.item.id) < ($1.rank, $1.item.id) }

        return attention.map(\.item) + unread
    }

    /// Every row this screen is drawing, in order, by identity.
    ///
    /// The value the list's animation is keyed on — see `body`. It changes when
    /// a row arrives or leaves, including when the row leaving was the last one
    /// in its section, and it does not change when a row merely says something
    /// new. The workspace id is in there so a section appearing or disappearing
    /// counts as a change even in the cases where none of the row ids move.
    private var rowIdentities: [String] {
        items.flatMap { item in
            [item.id]
                + item.blocked.prefix(Self.agentsPerWorkspace).map(\.id)
                + item.finished.prefix(Self.agentsPerWorkspace).map(\.id)
        }
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
            if items.isEmpty {
                Section { reassurance }
            } else {
                // A `Section` per workspace, not a section holding a row per
                // workspace. Sections are what a `List` gives you for "these
                // rows belong to that thing", and what this screen needs is
                // exactly that: a header naming the worktree over a short stack
                // of targets inside it. See `section(for:)`.
                ForEach(items) { section(for: $0) }
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
        // The list keeps its own grouped ground, and letting the theme through
        // was a mistake this screen shared with `WorkspaceListView`.
        //
        // `.scrollContentBackground(.hidden)` sat here so the terminal palette
        // showed behind the rows. But an inset-grouped row draws
        // `secondarySystemGroupedBackground`, and that is a FIXED pair —
        // `#1C1C1E` in dark, `#FFFFFF` in light — not a theme color. Against
        // Nord's `#2E3440`, Dracula's `#282A36`, Gruvbox's `#282828`, Solarized
        // Dark's `#002B36` and Catppuccin's `#1E1E2E`, the card came out DARKER
        // than the ground it floats on, which inverts the one rule dark mode is
        // built on: the thing in front is the lighter one. Cards read as wells
        // sunk into the screen. And against the three light themes that ship a
        // `#FFFFFF` background — GitHub Light, Tomorrow, High Contrast Light —
        // card and ground were the same color to the byte, so the card had no
        // shape at all and the section grouping this screen was rebuilt around
        // was invisible on every one of them.
        //
        // So the theme stops here. It is the TERMINAL's palette — `TerminalView`
        // paints it, `WorkspaceView` paints it behind the panes — and an inbox
        // of workspaces is chrome. `preferredColorScheme` still follows the
        // theme app-wide, so this list goes dark when the terminal does; it just
        // uses the system's own grouped grounds to do it, which are the two
        // colors `secondarySystemGroupedBackground` was designed against.

        // A row that clears itself, faded rather than snapped.
        //
        // This list only ever gained rows before. A finished agent's row ENDS
        // when somebody looks at it — `seen(Done) → Idle`, it is defined as
        // unseen — so rows now leave on a poll as well.
        //
        // Usually that happens where nobody is looking. `markVisibleSeen` sends
        // `terminal.seen` only for the pane actually in front of the person,
        // which is the workspace screen pushed on top of this one, so the row
        // is already gone by the time this list is uncovered and there is
        // nothing to animate. What is left is the case where the row goes while
        // this screen IS the screen: the same agent read on the Mac, or one
        // that starts another turn and drops to `working`. That is a row
        // vanishing under a thumb, and an unannounced deletion takes every row
        // below it up by its height mid-reach.
        //
        // Keyed on `rowIdentities` and nothing else. `TerminalRow` re-evaluates
        // once a second under its own `TimelineView`, and a bare `.animation`
        // here would try to animate every tick of every clock on the screen;
        // keyed, a signal line the host rewrote or a `+82 -13` that moved
        // changes in place, and only arrivals and departures are animated.
        .animation(.default, value: rowIdentities)
        .refreshable { await connection.refresh() }
        .navigationTitle("Needs You")
        .navigationSubtitle(subtitle)
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

    /// The count, qualified, over the whole list rather than over one part of
    /// it.
    ///
    /// It used to head the single `Section` that held every item, and that
    /// place is gone: a section header now names a workspace, and a number
    /// sitting where a workspace name sits would read as a fact about the
    /// workspace under it. So it moves to the one thing on this screen that is
    /// still about the whole list — the title. "Needs You" says what the list
    /// is; this says how much of it there is, in the bar, where it does not
    /// scroll away from the rows it is counting.
    ///
    /// "3 on studio" rather than a bare "3", and this is task 4 of the spec
    /// rather than decoration. The number can only ever speak for the runner
    /// this `Connection` is attached to, and when the merged inbox lands an
    /// unqualified 3 would silently start meaning something else. The precedent
    /// is `FleetSnapshot.complete`, which says whether a snapshot is all the
    /// agents there are for exactly this reason: drawing a partial fleet as the
    /// fleet asserts that the rest do not exist.
    ///
    /// Empty when there is nothing to count, rather than "0 on studio" over the
    /// reassurance — that would be the same sentence twice, once in numbers.
    private var subtitle: String {
        items.isEmpty ? "" : "\(items.count) on \(runner.label)"
    }

    /// One workspace: what it is, who in it is waiting, and what it changed —
    /// each of those a target of its own.
    ///
    /// This replaced ONE tap target for the whole row, which routed with
    /// `Route.Focus.none` and let `rule(for:inbox:)` choose the pane. The
    /// objection that design was written down to answer is half right, and the
    /// half that is right is the reason this is a `Section`:
    ///
    /// > The counts stay a label: a small target beside a large one, on a row
    /// > somebody is tapping while walking, is a coin toss with a wrong side.
    ///
    /// A small target beside a large one IS a coin toss with a wrong side, and
    /// it is exactly the hazard this shape removes. Every target here is a
    /// full-width list row of the same kind — an agent, an agent, the diff —
    /// stacked, with nothing beside any of them to lose the toss against. The
    /// finished agents joined that stack without changing it, which is the
    /// whole reason they were cheap to add. The counts are not a small thing
    /// next to the row any more; they are the trailing end of the row that
    /// opens them.
    ///
    /// The other half of that objection stays true and constrains what is
    /// inside: the agent lines are here to be READ. They are what the agent
    /// said it did, which is the larger half of reviewing its work. Making each
    /// one tappable buys the second blocked agent a door of its own, and it
    /// must not cost a band of what the row says — so `TerminalRow` is
    /// unchanged and full height, and nothing here shrinks it to feel more like
    /// a button.
    ///
    /// Why the diff gets a row at all: until this, nothing in the app had ever
    /// opened a workspace on its diff. `.changes` was selected only from inside
    /// the workspace screen, so from the front door the diff cost two taps
    /// while `+82 -13` sat on the row looking like the control for it.
    /// `docs/jobs-to-be-done.md` F4 has the phone's review experience
    /// load-bearing rather than a scaled-down Mac feature, which makes the diff
    /// the most important target on this screen — and it was the least
    /// reachable one.
    private func section(for item: Item) -> some View {
        Section {
            // `TerminalRow` and nothing new, one per `NavigationLink`. It is
            // four bands — the label with a ticking clock, the signal line the
            // host composed, the agent's last three utterances, and its running
            // subagents — and a second way to draw an agent is a second chance
            // for two screens to say different things about one pane. Its
            // `TimelineView` also keeps the schedule that matters: one second
            // for a row with a clock, an hour for one without. Every agent here
            // has a clock, since blocked is one of the two states that has one.
            //
            // This is the part of the row that answers "what did it do", which
            // the owner is explicit is most of what reviewing an agent's work
            // is. It is worth the height, and it is worth the same height now
            // that the height is also a tap target.
            //
            // The `.padding(.leading, 8)` indent that used to sit here is gone.
            // It existed so the row "reads as one thing containing several
            // rather than as a heading that happens to sit above some agents",
            // and a `Section` header says that structurally — which is the
            // point of using one.
            ForEach(item.blocked.prefix(Self.agentsPerWorkspace)) { terminal in
                agentRow(item, terminal)
            }

            if item.blocked.count > Self.agentsPerWorkspace {
                overflowLine(blockedOverflow(item.blocked.count - Self.agentsPerWorkspace))
            }

            // The finished agents, below the blocked ones and above the diff.
            //
            // The same row, and deliberately the same row: `TerminalRow` reads
            // `Terminal.activityLabel` and `activitySymbol`, which already say
            // "Done" with a green check and "Failed" with a red cross, so a
            // finished agent is drawn here exactly as it is drawn in the fleet
            // list and the tab strip. A second way to draw one is a second
            // chance for two screens to say different things about one pane.
            //
            // Tapping one opens the agent, which is what ENDS it: `terminal.seen`
            // clears `done` to `idle` on the next poll, and the row goes. That
            // is the intended shape of this row and not a wrinkle in it — see
            // the animation note in `body` for what happens to the ones nobody
            // opened.
            ForEach(item.finished.prefix(Self.agentsPerWorkspace)) { terminal in
                agentRow(item, terminal)
            }

            if item.finished.count > Self.agentsPerWorkspace {
                overflowLine(
                    finishedOverflow(Array(item.finished.dropFirst(Self.agentsPerWorkspace))))
            }

            // Absent on a worktree the runner has called clean, which is the
            // rule the counts already follow: `+0 -0` on every row is noise in
            // the shape of information, and a "Review Changes" row onto an
            // empty diff is the same noise with more height.
            //
            // PRESENT while the runner has not answered yet, which is a
            // different state and gets the opposite answer. Nil is the absence
            // of a fact, not the fact that there is nothing; the Changes tab
            // exists either way, since a worktree's own diff needs nothing on
            // the runner to be there. And a row that appeared one poll later
            // would push every row under it down, under a thumb already
            // travelling toward one of them — the tap landing on something else
            // is the same failure `items` sorts to avoid. So it arrives without
            // numbers and grows them.
            if item.counts?.hasDiff != false {
                changesRow(item)
            }
        } header: {
            header(item)
        }
    }

    /// The workspace's own line: what the work is, and which branch it is on.
    ///
    /// In a header rather than on a row, and that is the whole difference
    /// between this and what it replaced. A header is not a target, so nothing
    /// here competes with the rows below it, and the rows below it read as
    /// things inside this worktree without an indent having to say so.
    ///
    /// The main checkout has a branch like everything else, but WHICH branch is
    /// not the useful fact about it — that it is the repository itself is.
    /// `FleetList`'s header says the same thing the same way.
    ///
    /// `.textCase(nil)`, against the grouped list's default. Uppercasing is
    /// right for the word "SETTINGS" and wrong for both halves of this: the
    /// task is a sentence somebody wrote, and `feat/add-auth` is an identifier
    /// whose case is not ours to change. The task also keeps `.font(.body)` and
    /// the primary color it had as the first line of the old row, so what moved
    /// is the position and not the reading.
    ///
    /// Deliberately no amber up here, and no counts either. That color is
    /// reserved across the widget, the Live Activity, the complication and this
    /// app for an agent waiting on an answer, and `TerminalRow` already spends
    /// it on exactly those, inside; a second mark here would say the same thing
    /// twice and weaken it both times. The counts moved down to the row that
    /// opens them, which is the point of the row.
    ///
    /// Spoken as the task alone. The branch is deliberately left out — a
    /// VoiceOver reader gets `feat/add-auth` letter by letter for a fact that is
    /// not why this workspace is on the screen, and a header is read on the way
    /// into its section, so leaving it in would spell a branch once per
    /// workspace on a screen that is otherwise all sentences.
    private func header(_ item: Item) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(item.workspace.task)
                .font(.body)
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.middle)

            Text(item.workspace.isMainCheckout ? "Primary checkout" : item.workspace.branch)
                .font(.caption2.monospaced())
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .truncationMode(.middle)
                // Gives up width before the task does. Which worktree this is
                // survives a narrow screen; which branch it is on is the fact
                // that can afford to be truncated.
                .layoutPriority(-1)

            Spacer(minLength: 0)
        }
        .textCase(nil)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(item.workspace.task)
    }

    /// The diff, as a row of its own.
    ///
    /// The first route in the app to open a workspace on its diff, and it needs
    /// no plumbing to be it: `Route.Focus.changes` already exists for the tab
    /// strip's chip and for `WorkspaceView`'s no-tab-visited fallback, and
    /// `WorkspaceRoute.resolve()` passes it straight through — `alive(_:in:)`
    /// lets `.changes` survive unconditionally, because a worktree's own diff
    /// needs nothing on the runner to exist.
    ///
    /// The symbol is `ChangesChip`'s and the counts get the monospaced
    /// green-and-red treatment `FleetList`'s workspace header gives them, so
    /// this row looks like the tab it opens and `+82 -13` is the same shape
    /// wherever it appears. That was already the stated rule and the code had
    /// drifted off it: the three other places this pair appears — the tab
    /// strip's chip, `FleetList`'s header, the commit rows in `ChangesView` —
    /// are all `.caption2.monospaced()`, and this one was a full step larger.
    ///
    /// One accessibility element saying one thing, because read aloud the parts
    /// are "plus 82" and "minus 13" — two numbers with nothing attaching them to
    /// a diff. The words are `ChangesChip`'s for the same reason they were
    /// before: the counts are spoken the same way on the row and on the tab it
    /// opens.
    private func changesRow(_ item: Item) -> some View {
        NavigationLink(value: Route.workspace(id: item.workspace.id, focus: .changes)) {
            HStack(alignment: .firstTextBaseline, spacing: RowGutter.gap) {
                // In the state dot's column rather than in a `Label`'s. A
                // `Label` sizes its symbol to the text beside it and adds a gap
                // of its own, which started these words about 24 points in
                // while the agent rows above them started at 18 and the
                // overflow line under them at 0 — three text edges inside one
                // section. An 8-point frame is exactly the width of the dot a
                // `TerminalRow` draws, so the symbol sits on that axis and the
                // words line up with theirs.
                Image(systemName: "doc.text.magnifyingglass")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(width: RowGutter.dot)

                Text("Review Changes")
                    .font(.body)

                Spacer(minLength: 0)

                if let counts = item.counts, counts.hasDiff {
                    HStack(spacing: 4) {
                        Text("+\(counts.insertions)").foregroundStyle(.green)
                        Text("-\(counts.deletions)").foregroundStyle(.red)
                    }
                    .font(.caption2.monospaced())
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(spokenChanges(item))
        }
        .accessibilityIdentifier("needs-you-changes-\(item.workspace.id)")
    }

    /// One agent, wherever it came from.
    ///
    /// Shared by the blocked group and the finished one because the row is the
    /// same row and the route is the same route: `Route.Focus.agent` opens the
    /// workspace on that pane, which is what a person wants from both — the
    /// answer to the question, or the answer to the question they asked.
    ///
    /// The identifier is per agent, where the old one was per workspace and had
    /// no readers at all. Nothing in `FarCoolerUITests` matches these yet; they
    /// are on the same pattern as `fleet-terminal-<id>` and `terminal-tab-<id>`,
    /// so a test that wants to open the second blocked agent — or the finished
    /// one under it — from the front door has a name to ask for.
    private func agentRow(_ item: Item, _ terminal: Terminal) -> some View {
        NavigationLink(
            value: Route.workspace(id: item.workspace.id, focus: .agent(terminal.id))
        ) {
            TerminalRow(terminal: terminal, ordinal: item.ordinals[terminal.id])
        }
        .accessibilityIdentifier("needs-you-agent-\(terminal.id)")
    }

    /// The line a group puts under itself when it ran out of room.
    ///
    /// On the same left edge the agents it is counting start on. It sat at 0
    /// while a `TerminalRow`'s words started at 18, so the sentence summarizing
    /// a group hung outside the group — and a screen whose whole job is to be
    /// read at a glance was asking the eye to find three margins in one
    /// section. `RowGutter` is where that 18 is named, beside the row that sets
    /// it.
    private func overflowLine(_ sentence: String) -> some View {
        Text(sentence)
            .font(.caption2)
            .foregroundStyle(.tertiary)
            .padding(.leading, RowGutter.text)
    }

    /// The blocked agents this section ran out of room for. A sentence rather
    /// than a bare number, because "+2" under a list of agents reads as two more
    /// of something and does not say what.
    private func blockedOverflow(_ count: Int) -> String {
        count == 1 ? "1 more agent needs you" : "\(count) more agents need you"
    }

    /// The finished agents this section ran out of room for.
    ///
    /// Says "failed" when any of the hidden ones did, and the words are the
    /// daemon's rather than a third set: `watch.rs` titles a finished turn
    /// "<name> finished" and a died-halfway one "<name> failed", and
    /// `Terminal.activityLabel` puts the same distinction on the rows above this
    /// line. A sentence that swept a failure into "finished" would hide the one
    /// thing in the group most worth going in for — and it would hide it in the
    /// only place on this screen where the agents are counted instead of shown.
    ///
    /// Takes the terminals rather than a count, because the count cannot answer
    /// which of them failed.
    private func finishedOverflow(_ hidden: [Terminal]) -> String {
        let failures = hidden.filter(\.turnDidFail).count
        if failures == hidden.count {
            return failures == 1 ? "1 more agent failed" : "\(failures) more agents failed"
        }
        if failures > 0 {
            return "\(hidden.count) more agents finished, \(failures) failed"
        }
        return hidden.count == 1 ? "1 more agent finished" : "\(hidden.count) more agents finished"
    }

    /// The changes row, said rather than spelled.
    ///
    /// Without the workspace's name in it, which is the one thing that changed
    /// when the row split. The old label combined the name and the counts
    /// because the whole row was one target; now the name is the section header
    /// a reader has just passed through, and repeating it here would say it
    /// again on every row of every workspace.
    ///
    /// The clause on the end says what the counts COUNT, which nothing on this
    /// screen did. This number is the fleet inbox's — everything the worktree
    /// has changed, committed, uncommitted and untracked — and not the branch
    /// total the Changes screen's Branch segment shows. The Mac says that in a
    /// tooltip on the row; a phone has no hover, so the label is the only place
    /// it can be said at all.
    private func spokenChanges(_ item: Item) -> String {
        guard let counts = item.counts, counts.hasDiff else { return "Review Changes" }
        return "Review Changes, \(counts.insertions) added, \(counts.deletions) removed, "
            + "including work that isn’t committed yet"
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
    ///
    /// It still reads true now that finished agents are listed, and the check is
    /// worth writing down because the sentence is an assertion about the whole
    /// runner. This branch is `items.isEmpty`, and a workspace with a finished
    /// agent in it produces an `Item` — so "Nothing Needs You" cannot be on
    /// screen while a `done` agent sits unseen. The one exception is the one
    /// this screen takes everywhere: a HIDDEN workspace is not counted, here or
    /// in `workingCount` or in the subtitle, because a front door that shows
    /// what you hid is not honoring the hiding.
    ///
    /// `reassuranceDetail` under it is untouched and stays honest for the same
    /// reason it always was: it counts agents that are `working`, which is
    /// disjoint from the two states that put a row on this screen. It is the
    /// answer to "is anything happening", asked only once nothing needs you.
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
        return "Every workspace on \(runner.label) is hidden. They’re still in there."
    }
}
