import Foundation

/// The one `fleet.json`, assembled from every runner the phone is polling.
///
/// # What this replaces
///
/// One file, rewritten WHOLE on every poll, by whichever connection polled
/// last. That was correct while there was one connection and became the
/// clobbering the port had to answer: three runners polling every three seconds
/// would each overwrite the other two, and the widget, the lock screen card and
/// the watch complication would show whichever runner's fleet happened to land
/// most recently — flickering between them, with no error and nothing in the
/// file to say what had happened.
///
/// So the merge is here: a projection per runner, kept in memory in the app,
/// combined into the one snapshot the surfaces read. Nothing on disk changed,
/// and deliberately: `FleetSnapshot` is decoded by four out-of-process targets
/// and by builds already installed, and a new per-runner field would be a wire
/// change bought for something a dictionary in the writing process answers.
///
/// # Why this could not land earlier
///
/// **Merging before the store was live would have been actively wrong.** With
/// one connection, keying by runner leaves every runner nobody is polling
/// contributing its last agents to the lock screen forever — the shape of merge
/// bug worth naming. Correct only once something knows which runners are live,
/// which is what `keeping(runners:)` is: a runner that has been retired stops
/// contributing the moment it is retired, not whenever its rows next happen to
/// be overwritten.
///
/// # Where the interesting decisions are
///
/// Every field has an answer and none of them is "the last writer's":
///
/// - **`agents`** concatenate, in the order runners were recorded. A runner
///   going quiet does not remove its rows; that is what keeps a mental map of
///   the fleet stable while a laptop sleeps, and it is what `confidence(in:at:)`
///   already ages honestly using each agent's own `observedAt`.
/// - **`complete`** is true only when EVERY live runner has been heard from,
///   and none of them has stopped answering since. It
///   means "these are all the agents there are", and a fleet where one of three
///   runners has never answered is a fleet with agents missing from it. The
///   surfaces hedge on it — "from notifications" on the widget, a partial footer
///   on the watch — and hedging is exactly right for that state.
/// - **`lostRunners`** names the runners whose link was lost, which is the
///   other reason `complete` is false and needs its own words: "Lost touch
///   with Studio", not "from notifications". See `FleetSnapshot.hedge`.
/// - **`reviewsWaiting`** sums the runners that answered and ignores the ones
///   that did not. Nil only when nobody answered at all, because nil means "not
///   told" and one modern runner reporting 3 is being told something.
/// - **`needsYou`** is `NeedsYou.merge` over the lists the app's store holds,
///   by runner, for the live runners that have one. Nil until the store has
///   handed over any list at all, so a surface goes on counting blocked agents
///   until the app has something better to say. See `record(needsYou:)`.
/// - **A runner that isn't answering** keeps its rows, marked
///   `runnerAnswering == false`, so no surface counts them as working. See
///   `keeping(runners:answering:)`.
/// - **`fleetTrace`** is summed onto one axis by `ActivityTrace.summing`, which
///   is where that arithmetic and what it costs are written down — placing
///   each runner by its `fleetTraceAnchor` where it sent one, and where the
///   poll that brought it could vouch for it (`ActivityTrace.trusted`, against
///   that snapshot's own `capturedAt`).
public struct FleetPublication {
    /// One runner's own projection, as its connection last polled it.
    private struct Contribution {
        var snapshot: FleetSnapshot
        /// What to call the runner when its link is lost. See
        /// `FleetSnapshot.lostRunners`.
        var name: String?
        /// The link this was polled over has gone since, and no poll has
        /// landed on a new one yet. See `keeping(runners:answering:)`.
        var lost = false
    }

    private var byRunner: [String: Contribution] = [:]
    /// The order runners were first recorded in, so the merged list does not
    /// reshuffle when a laptop wakes up. Not a dictionary's own order, which is
    /// a hash order and changes between launches.
    private var order: [String] = []
    /// The runners currently being polled, or nil before anyone has said.
    ///
    /// Nil rather than "all of them" so that a publication nobody has told
    /// about liveness behaves exactly as one runner's writer always did.
    private var live: Set<String>?
    /// Each runner's Needs You list, by runner id, as the app's store last
    /// handed them over. Nil before it has.
    private var needsYouByRunner: [String: [NeedsYouItem]]?

    public init() {}

    /// Record every runner's Needs You list at once: the app's store holds
    /// them all, so it hands over the whole dictionary, keyed by the runner
    /// ids `record(runner:snapshot:named:)` uses. A runner with no entry
    /// hasn't answered yet, and contributes nothing rather than zero.
    ///
    /// - Returns: whether anything changed, so the caller writes the file and
    ///   wakes the widgets and the watch only when there's news. The store
    ///   republishes on every change any connection makes, several times a
    ///   second while an agent works; see `KeptMembership` for what writing
    ///   on each of those cost.
    @discardableResult
    public mutating func record(needsYou lists: [String: [NeedsYouItem]]) -> Bool {
        guard lists != needsYouByRunner else { return false }
        needsYouByRunner = lists
        return true
    }

    /// Record what one runner just polled, and what to call it.
    ///
    /// `named` is what the surfaces say when this runner's link is lost:
    /// "Lost touch with Studio". A runner recorded without one is still
    /// lost, and still makes the merge incomplete, but it can't be named.
    public mutating func record(runner: String, snapshot: FleetSnapshot, named name: String? = nil) {
        if byRunner[runner] == nil { order.append(runner) }
        byRunner[runner] = Contribution(snapshot: snapshot, name: name)
    }

    /// Keep only these runners, forgetting anything the store has retired.
    ///
    /// **The half that makes merging correct rather than accumulating.** A
    /// runner removed in settings, edited into a different runner, or filtered
    /// out by the battery gate is a runner nobody is polling — and its agents
    /// on a lock screen would go on claiming to be working, forever, with
    /// nothing left to correct them. `FleetMembership.published` refuses the
    /// same thing one layer up and for the same reason.
    ///
    /// - Returns: whether the surfaces have anything to be told about this.
    ///   **False is the LAUNCH, and it is the whole of why this answers at
    ///   all.** The membership is settled before any runner has been polled —
    ///   `FleetStore.publish` reconciles first and the first `fleet` call
    ///   returns an SSH round trip later — so the first call of every launch
    ///   arrives with nothing recorded and nothing to forget. A merge assembled
    ///   there is an empty fleet carrying a REAL `capturedAt`, which is not
    ///   "no agents observed" but "no observation", and the two are the same
    ///   bytes on disk: `FleetEntry.hasSnapshot` reads a real date as a real
    ///   look at the fleet, so the widget answers "No agents" and the watch a
    ///   confident 0 — over the last good snapshot, which the write destroyed.
    ///   Durable for as long as the first poll never lands, which is every
    ///   offline launch and every foreground the person backs straight out of.
    ///
    ///   True in both of the other cases, and both are observations. Something
    ///   was dropped, so the fleet on the lock screen has genuinely lost rows
    ///   and must be told even when what is left is nothing at all — a store
    ///   that has just retired its last runner has no next poll from anybody.
    ///   Or something is still recorded, and `live` moving changes whether that
    ///   merge is `complete`.
    @discardableResult
    public mutating func keeping(runners: Set<String>) -> Bool {
        let held = !isEmpty
        live = runners
        byRunner = byRunner.filter { runners.contains($0.key) }
        order = order.filter { runners.contains($0) }
        return held || !isEmpty
    }

    /// The same, and which of those runners are answering right now.
    ///
    /// **A runner that stops answering has its rows marked lost**, and they
    /// stay marked until a poll records it again. Not until it answers again:
    /// a link comes up a whole SSH round trip before the first poll over it
    /// lands, and in that gap the rows are still the ones read before the link
    /// went. Answering is `.connected` and nothing weaker (a runner that stays
    /// down spends most of its outage reconnecting, which is what let the
    /// Mac's status bar count a dead runner's panes for most of an outage),
    /// and on the phone its fleet read on this link as well: see
    /// `ShellRunnerLabel.answering`.
    ///
    /// - Returns: what `keeping(runners:)` returns, and true as well when a
    ///   recorded runner has just been marked lost, which changes what the
    ///   surfaces draw.
    @discardableResult
    public mutating func keeping(runners: Set<String>, answering: Set<String>) -> Bool {
        var told = keeping(runners: runners)
        for runner in order where !answering.contains(runner) {
            guard byRunner[runner]?.lost == false else { continue }
            byRunner[runner]?.lost = true
            told = true
        }
        return told
    }

    /// Whether anything has been recorded at all.
    ///
    /// The difference between an empty fleet and the absence of one, asked of
    /// the merge before it is assembled — `merged(at:)` cannot be asked it,
    /// because the snapshot it returns spells both the same way. See
    /// `keeping(runners:)`, which is the reader.
    public var isEmpty: Bool { byRunner.isEmpty }

    /// The one snapshot every out-of-process surface renders from.
    ///
    /// `at` is the assembly moment — what the widgets' "as of" footer reports
    /// and what tells a real snapshot from `empty`. It is deliberately NOT what
    /// vouches for any agent: each row carries its own `observedAt`, stamped by
    /// the runner's own poll, so a fleet reassembled because one runner
    /// answered does not silently re-date the other two's rows. That is the
    /// same distinction `merging(_:at:)` draws on the push path.
    public func merged(at now: Date) -> FleetSnapshot {
        let contributions = order.compactMap { byRunner[$0] }

        // Every live runner heard from, and not merely "somebody was". See the
        // header: `complete` asserts that these are all the agents there are.
        // And a runner that stopped answering has not been heard from: its
        // rows are the last ones read, and it may have started agents since.
        let expected = live?.count ?? contributions.count
        let complete =
            contributions.count >= expected
            && !contributions.isEmpty
            && contributions.allSatisfy { $0.snapshot.complete && !$0.lost }

        let counted = contributions.compactMap(\.snapshot.reviewsWaiting)

        // Told apart from "not heard from" here, where the difference is
        // known: see `FleetSnapshot.lostRunners`.
        let lost = contributions.filter(\.lost).compactMap(\.name)

        let fleet = ActivityTrace.summing(
            anchored: contributions.compactMap { contribution in
                ActivityTrace(contribution.snapshot.fleetTrace).map { trace in
                    (
                        trace,
                        ActivityTrace.trusted(
                            contribution.snapshot.fleetTraceAnchor, span: trace.span,
                            heardAt: contribution.snapshot.capturedAt)
                    )
                }
            })

        return FleetSnapshot(
            agents: contributions.flatMap { contribution in
                // "Can't say" for every agent a lost runner last reported, and
                // nothing written for one that answered: nil already reads as
                // answering, and an old reader never meets the key.
                guard contribution.lost else { return contribution.snapshot.agents }
                return contribution.snapshot.agents.map { agent in
                    var unheard = agent
                    unheard.runnerAnswering = false
                    return unheard
                }
            },
            capturedAt: now,
            complete: complete,
            reviewsWaiting: counted.isEmpty ? nil : counted.reduce(0, +),
            fleetTrace: fleet?.trace.encoded,
            fleetTraceAnchor: fleet?.anchor,
            lostRunners: lost.isEmpty ? nil : lost,
            // Only the runners still being polled, for `agents`' reason: a
            // retired runner's asks must not stay on a lock screen forever.
            needsYou: needsYouByRunner.map { lists in
                NeedsYou.merge(lists.filter { live?.contains($0.key) ?? true })
            })
    }
}

/// The membership the snapshot writer last acted on, and whether a new one
/// moved from it.
///
/// **The guard is load-bearing rather than thrifty.** `FleetStore.publish`
/// runs on every change any connection publishes — several times a second
/// while an agent is producing — and a write ends in
/// `WidgetCenter.reloadAllTimelines()` and a round trip to the watch. Called
/// unguarded it wedged the main thread badly enough to time a UI test out.
/// Adding or removing a runner, or a runner's link going or coming back, is
/// not a per-poll event and must not be priced like one. Here rather than in
/// `FleetSnapshotWriter`, where it was, because the iOS target has no tests.
public struct KeptMembership: Sendable {
    /// Nil before anyone has said, which is what makes the first call news.
    private var runners: Set<String>?
    private var answering: Set<String>?

    public init() {}

    /// Whether this membership differs from the last one acted on. Records it
    /// when it does.
    public mutating func moved(runners: Set<String>, answering: Set<String>) -> Bool {
        guard runners != self.runners || answering != self.answering else { return false }
        self.runners = runners
        self.answering = answering
        return true
    }
}
