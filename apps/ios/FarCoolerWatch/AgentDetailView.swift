import SwiftUI

/// One agent, and what it did while you were away.
///
/// The screen the fleet list exists to get somebody to. It re-reads the agent
/// out of the CURRENT snapshot on every render rather than holding the copy that
/// was tapped — see `WatchRoute.agent` — so a fleet that changes while this is
/// open changes what is on it.
///
/// Nothing here is composed. `headline`, `line`, `feed` and the glyph are the
/// host's words; the only string this file writes is how long the agent has held
/// its state, and it writes that only when the host said when the state began.
///
/// **The buttons are disabled unless the phone is reachable.** Not greyed as a
/// hint — genuinely off, because `WatchState.canAct` is the same rule
/// `WatchLinkClient.send` enforces at the door. A watch that offers an Allow it
/// cannot deliver is worse than one that says it cannot reach the phone: the
/// person taps, sees nothing wrong, walks away believing they answered, and the
/// agent is still sitting there an hour later.
struct AgentDetailView<Client: FleetClient>: View {
    @ObservedObject var client: Client
    let terminal: String

    /// The agent as of right now, or nil if it has left the fleet.
    private var agent: FleetSnapshot.Agent? {
        client.state.snapshot?.agents.first { $0.id == terminal }
    }

    /// When this screen must redraw, computed when a snapshot lands rather than
    /// while one is being drawn.
    ///
    /// This was `.explicit([Date.now] + snapshot.stalenessMoments(after: .now))`
    /// inline in `body`, which is a schedule the `TimelineView` has never seen
    /// before on every single body evaluation: it drops the timeline it is
    /// running, starts the new one, and that renders the body again. Navigating
    /// into this screen is where that was worst, because pushing it evaluates
    /// the body while the transition is animating.
    ///
    /// The list behind it holds the identical schedule for the identical
    /// reason, and now literally the identical one — `refreshes(for:from:)` in
    /// `FleetListView.swift`, which is where the rule that `now` must lead is
    /// written down. Two screens that have to stop vouching for an agent at one
    /// instant should not each carry their own spelling of when that is.
    @State private var schedule: [Date] = [.now]

    var body: some View {
        Group {
            if let agent, let snapshot = client.state.snapshot {
                // The same clock the fleet list runs on, for the same reason: a
                // screen left open must stop asserting `working` at the moment
                // the snapshot stops vouching for it, and no news will arrive to
                // prompt that.
                TimelineView(.explicit(schedule)) { context in
                    detail(agent, snapshot.confidence(in: agent, at: context.date))
                }
                .navigationTitle(agent.label)
            } else {
                // A terminal that closed, or a watch that has never heard from
                // the phone at all. Said plainly rather than left as an empty
                // screen, which reads as an agent that has nothing to report.
                ContentUnavailableView(
                    "Not in the Fleet",
                    systemImage: "questionmark",
                    description: Text("This agent isn’t in the last fleet your iPhone sent."))
            }
        }
        .onChange(of: client.state.snapshot, initial: true) { _, snapshot in
            schedule = refreshes(for: snapshot, from: .now)
        }
    }

    private func detail(
        _ agent: FleetSnapshot.Agent, _ confidence: FleetSnapshot.Confidence
    ) -> some View {
        List {
            Section {
                Headline(agent: agent, confidence: confidence)
            }
            // The three most recent things the agent DID, oldest first, exactly
            // as the host ordered them.
            //
            // **Not what it said**, and the difference is worth being exact
            // about because this section used to claim otherwise. `feed` is the
            // host's compact activity log — `Read crates/core/src/feed.rs`,
            // `cargo test -p farcooler-core` — truncated to `feed::WIDTH` for a
            // widget. It answers "is it getting on with it", which is a real
            // question and the one this screen can answer with no round trip
            // and no phone in range. It cannot answer "what did it say", and it
            // was being read as though it could: the words the agent wrote are
            // a tap away under Transcript, and the phone has to be asked for
            // them.
            if !agent.feed.isEmpty {
                Section("Recent") {
                    // By offset, because two identical steps are two steps. An
                    // id of the string itself would collapse a repeated line to
                    // one row and quietly under-report what happened.
                    ForEach(Array(agent.feed.enumerated()), id: \.offset) { _, said in
                        Text(said).font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
            Section {
                // First, because reading is the common reason to be here. The
                // owner's sentence for this whole screen is "an agent completes
                // answering my question and I want to see what it said" — and
                // the other two rows are for the rarer case where the answer
                // needs something back.
                //
                // A noun where the other two are verbs, and it is the honest
                // label rather than a lapse: "Reply" and "Answer" name what you
                // will do to the agent, and there is nothing you do to an agent
                // here. Every verb that fits — "Read", "See" — needs an object
                // to be unambiguous, and a 41mm row has no width for one.
                NavigationLink(value: WatchRoute.transcript(terminal: agent.id)) {
                    Label("Transcript", systemImage: "quote.bubble")
                }
                NavigationLink(value: WatchRoute.compose(terminal: agent.id)) {
                    Label("Reply", systemImage: "text.bubble")
                }
                // Offered for every agent, not only a `blocked` one. Status on
                // this screen can be an hour old — that is the whole premise of
                // `confidence(in:at:)` — so hiding the button on a `working`
                // agent would hide it exactly when a stale snapshot is wrong
                // about the agent that has since blocked. Task 5's screen asks
                // the phone what is actually pending and says "nothing" when
                // nothing is, which is the honest place for that question.
                NavigationLink(value: WatchRoute.permission(terminal: agent.id)) {
                    Label("Answer", systemImage: "checkmark.circle")
                }
            } footer: {
                // A disabled button with nothing beside it is a dead end. The
                // fleet list says this once at the top; a person who scrolled
                // straight in from a notification never saw it.
                if !client.state.canAct {
                    Text("Can’t reach your iPhone, so these are off until it’s nearby.")
                        .font(.caption2)
                }
            }
            .disabled(!client.state.canAct)
        }
    }
}

/// What the agent is doing, how it is doing it, and how long it has been.
private struct Headline: View {
    let agent: FleetSnapshot.Agent
    let confidence: FleetSnapshot.Confidence

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                // 22pt, which §03 names for exactly this: "as a lone indicator
                // — strokes 5 / 4 / 1.5, core 8 — used by the detail header and
                // accessoryCircular."
                GlanceMarkView(GlanceMark(agent: agent, confidence: confidence), size: .watchLone)
                    .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 2 }
                // "last seen working" rather than "working", and the qualifier
                // in front — see `stated`. On this screen there is room for the
                // whole string, which is exactly why the rule cannot be argued
                // from here: it is the narrow row on the list behind this one
                // that a suffix would strand.
                Text(stated(agent, confidence))
                    .font(.headline)
            }
            if !agent.line.isEmpty, agent.line != agentTitle(agent) {
                Text(agent.line)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            // Only when the host dated it. A nil `activityChangedAt` is "not
            // told", which is a different thing from "just now" and must never
            // be rendered as it — an undated agent shown as "0 sec ago" is this
            // screen vouching for something nobody vouched for.
            //
            // It is when the STATE began, not when the snapshot was taken, so it
            // keeps counting up correctly on a fleet that is polling happily and
            // reporting no change.
            //
            // **And it is not how old the news is**, which is why the line says
            // "Unchanged for" and not "Last seen". Those two used to be the same
            // date and the confusion cost this screen a headline: an agent
            // working for ten minutes read "last seen working" on a snapshot a
            // second old. `confidence` now measures from `observedAt` and this
            // line still measures from here, because "it has been on this for
            // ten minutes" is genuinely worth knowing — it is just not the same
            // sentence as "we have not heard in an hour".
            if let changedAt = agent.activityChangedAt {
                Text("Unchanged for \(changedAt, style: .relative)")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
        }
        .opacity(confidence == .lastSeen ? 0.6 : 1)
    }
}

// Subagents are deliberately absent from this screen. `FleetSnapshot.Agent`
// carries no subagent list — the wire has `glyph`, `headline`, `line`, `feed`
// and the rest, and nothing else — so there is no honest way to name them here.
// What the host does send is the count, already folded into `line` by
// `farcooler_core::feed::signal` ("3/7 · Designing test matrix · 2 agents"),
// which this screen renders unchanged. Adding a field to the snapshot to say
// more is a change to the projection every surface shares, and it belongs on the
// host with the rest of the ladder rather than in a watch screen.

#if DEBUG
    /// A fleet with no phone in the room.
    ///
    /// The reason `FleetClient` is a protocol with one shipping conformance:
    /// there is no other way to look at these screens without a paired watch, a
    /// paired phone and a runner with agents on it. Every preview below is a
    /// layout somebody has to be able to check, and the two that matter most —
    /// a stale row and an unreachable phone — cannot be produced on demand from
    /// real hardware at all.
    @MainActor
    final class PreviewFleetClient: FleetClient {
        @Published private(set) var state: WatchState

        /// What the phone "answers", and how long it takes to answer it.
        ///
        /// Both are parameters because the two states this app is most likely to
        /// get wrong are the two nobody can produce on demand: a `.failed`
        /// sentence needs a phone that is present but cannot reach its runner,
        /// and the in-flight spinner needs a phone that is slow rather than
        /// absent. `latency` is what makes the second one hold still long enough
        /// to be looked at — with a zero-latency client the spinner exists for
        /// one frame and every screenshot of it is a screenshot of the result.
        private let latency: Duration
        private let reply: @Sendable (WatchRequest) -> WatchReply

        init(
            _ state: WatchState,
            latency: Duration = .zero,
            reply: @escaping @Sendable (WatchRequest) -> WatchReply = { _ in .sent }
        ) {
            self.state = state
            self.latency = latency
            self.reply = reply
        }

        func send(_ request: WatchRequest) async -> WatchReply {
            if latency > .zero { try? await Task.sleep(for: latency) }
            return reply(request)
        }
    }

    /// The widest strings the host can produce, so a preview shows the worst
    /// case rather than a comfortable one: `feed::HEADLINE_WIDTH` is 18 and
    /// `feed::WIDTH` is 40, and both are counted in characters.
    @MainActor
    enum PreviewFleet {
        static func agent(
            id: String, label: String, status: String, glyph: String,
            headline: String, line: String, ageMinutes: Double
        ) -> FleetSnapshot.Agent {
            FleetSnapshot.Agent(
                id: id, label: label, machine: "studio", status: status, glyph: glyph,
                headline: headline, line: line,
                feed: [
                    "Read crates/core/src/feed.rs",
                    "Edit apps/ios/FarCoolerWatch/FleetListView.swift",
                    "cargo test -p farcooler-core",
                ],
                rank: 0, turnFailed: false,
                activityChangedAt: Date().addingTimeInterval(-60 * ageMinutes))
        }

        static let snapshot = FleetSnapshot(
            agents: [
                agent(
                    id: "a", label: "claude", status: "blocked", glyph: "?",
                    headline: "claude needs you", line: "Run cargo test in the workspace root",
                    ageMinutes: 4),
                agent(
                    id: "b", label: "codex", status: "working", glyph: "*",
                    headline: "codex is working", line: "3/7 · Designing test matrix · 2 agents",
                    ageMinutes: 2),
                // Working and older than `staleAfter`, so it renders as
                // "last seen …" at 60% — the case no live fleet will show you
                // when you want to look at it.
                agent(
                    id: "c", label: "gemini-worktree-two", status: "working", glyph: "*",
                    headline: "gemini is working", line: "Writing docs/superpowers/plans/x.md",
                    ageMinutes: 90),
            ],
            capturedAt: Date().addingTimeInterval(-90), complete: true,
            // So the review row is on screen in every preview that uses this
            // fleet. It is the one row on the fleet list that no live watch
            // will show you on demand — it needs a runner with an unreviewed
            // diff on it — and it is the row most likely to be forgotten when
            // this screen is laid out again.
            reviewsWaiting: 3)

        /// A permission worded the way Claude Code words one.
        ///
        /// The names are `claude/normalize.rs`'s, verbatim: `Allow ` plus
        /// `tool_title(...)`, and a bare `Deny`. Kept exact because the point
        /// this fixture exists to prove is that the buttons say what the AGENT
        /// says — a fixture that quietly used "Allow"/"Deny" would show a
        /// hardcoded pair passing for the real thing.
        nonisolated static let claudePermission = WatchPermission(
            id: "req-1",
            // An opaque correlation id, which is what every producer actually
            // puts here and why nothing renders it. See `PermissionView`.
            toolCall: "toolu_01Q9xk3mBn",
            options: [
                WatchPermissionOption(
                    id: "allow", name: "Allow Bash(cargo test -p farcooler-core)",
                    kind: "allow_once"),
                WatchPermissionOption(id: "deny", name: "Deny", kind: "reject_once"),
            ])

        /// Three options, one of them a policy change — `codex/normalize.rs`'s
        /// wording, including the `Allow: ` prefix and the command after it.
        nonisolated static let codexPermission = WatchPermission(
            id: "req-2",
            toolCall: "item_7",
            options: [
                WatchPermissionOption(
                    id: "accept", name: "Allow: git push --force-with-lease", kind: "allow_once"),
                WatchPermissionOption(
                    id: "acceptForSession", name: "Allow for this session", kind: "allow_always"),
                WatchPermissionOption(id: "decline", name: "Decline", kind: "reject_once"),
            ])
    }

    #Preview("Fleet") {
        FleetListView(client: PreviewFleetClient(.live(PreviewFleet.snapshot)))
    }

    #Preview("Fleet, unreachable") {
        FleetListView(client: PreviewFleetClient(.cached(PreviewFleet.snapshot)))
    }

    #Preview("Fleet, nothing known") {
        FleetListView(client: PreviewFleetClient(.nothing))
    }

    #Preview("Agent") {
        NavigationStack {
            AgentDetailView(client: PreviewFleetClient(.live(PreviewFleet.snapshot)), terminal: "a")
                .watchRoutes(client: PreviewFleetClient(.live(PreviewFleet.snapshot)))
        }
    }

    #Preview("Agent, unreachable") {
        NavigationStack {
            AgentDetailView(
                client: PreviewFleetClient(.cached(PreviewFleet.snapshot)), terminal: "c")
        }
    }
#endif
