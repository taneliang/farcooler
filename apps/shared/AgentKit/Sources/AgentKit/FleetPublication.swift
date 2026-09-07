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
/// - **`complete`** is true only when EVERY live runner has been heard from. It
///   means "these are all the agents there are", and a fleet where one of three
///   runners has never answered is a fleet with agents missing from it. The
///   surfaces hedge on it — "from notifications" on the widget, a partial footer
///   on the watch — and hedging is exactly right for that state.
/// - **`reviewsWaiting`** sums the runners that answered and ignores the ones
///   that did not. Nil only when nobody answered at all, because nil means "not
///   told" and one modern runner reporting 3 is being told something.
/// - **`fleetTrace`** is summed onto one axis by `ActivityTrace.summing`, which
///   is where that arithmetic and what it costs are written down.
public struct FleetPublication {
    /// One runner's own projection, as its connection last polled it.
    private struct Contribution {
        var snapshot: FleetSnapshot
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

    public init() {}

    /// Record what one runner just polled.
    public mutating func record(runner: String, snapshot: FleetSnapshot) {
        if byRunner[runner] == nil { order.append(runner) }
        byRunner[runner] = Contribution(snapshot: snapshot)
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
        let expected = live?.count ?? contributions.count
        let complete =
            contributions.count >= expected
            && !contributions.isEmpty
            && contributions.allSatisfy(\.snapshot.complete)

        let counted = contributions.compactMap(\.snapshot.reviewsWaiting)

        return FleetSnapshot(
            agents: contributions.flatMap(\.snapshot.agents),
            capturedAt: now,
            complete: complete,
            reviewsWaiting: counted.isEmpty ? nil : counted.reduce(0, +),
            fleetTrace: ActivityTrace.summing(
                contributions.compactMap { ActivityTrace($0.snapshot.fleetTrace) }
            )?.encoded)
    }
}
