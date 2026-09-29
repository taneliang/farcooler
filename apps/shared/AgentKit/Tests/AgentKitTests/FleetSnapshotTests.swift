import Foundation
import Testing

@testable import AgentKit

/// The rules every glance surface renders by.
///
/// Pure on purpose: a widget cannot be stepped through in a debugger and a
/// lock screen cannot be asserted against, so the decisions they make live
/// here where they can be.
struct FleetSnapshotTests {
    private func agent(
        _ id: String,
        status: String,
        rank: UInt32 = 0,
        activityChangedAt: Date? = nil,
        observedAt: Date? = nil
    ) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: "claude", machine: "orchard", status: status,
            glyph: "●", headline: "claude 4m", line: "Writing fruit.txt",
            feed: ["Reading watch.rs."], rank: rank, turnFailed: false,
            activityChangedAt: activityChangedAt, observedAt: observedAt)
    }

    @Test func aSnapshotRoundTripsThroughJson() throws {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")],
            capturedAt: Date(timeIntervalSince1970: 1_000_000),
            complete: true)
        let data = try JSONEncoder().encode(snapshot)
        #expect(try JSONDecoder().decode(FleetSnapshot.self, from: data) == snapshot)
    }

    /// A newer daemon's extra key must not take the whole snapshot down — the
    /// same rule the wire types follow, for the same reason.
    @Test func anUnknownKeyDoesNotFailTheDecode() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"working","glyph":"●","headline":"claude 4m","line":"x",
        "feed":[],"rank":0,"turnFailed":false,"somethingNewer":42}],
        "capturedAt":1000000,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.agents.count == 1)
    }

    /// A payload with no `planDone`/`planTotal` still decodes, and reads as
    /// "not told" rather than as zero.
    ///
    /// The rule that governs every field added to this type, tested on the two
    /// just added. Swift's synthesized `Decodable` throws on a missing key for a
    /// non-optional, so one absent field would fail the WHOLE snapshot — every
    /// agent on it, on every surface — rather than cost one row a bar. And the
    /// payload here is not hypothetical: it is exactly what a snapshot written
    /// by a build that predates these fields looks like on disk, and what a
    /// phone talking to a daemon too old to send them writes today.
    ///
    /// Nil rather than 0 is the second half, and it is not pedantry. `0` of `7`
    /// is an agent that has written seven tasks and finished none, which draws
    /// an empty bar; nil is an agent nobody has said anything about, which
    /// draws no bar and reserves no room for one. A default of zero would turn
    /// every codex pane, every cursor pane and every claude session with no
    /// task list into a progress claim no host ever made.
    @Test func aPayloadWithoutThePlanCountsStillDecodes() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"working","glyph":"●","headline":"claude 4m","line":"x",
        "feed":[],"rank":0,"turnFailed":false}],
        "capturedAt":1000000,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.agents.count == 1)
        #expect(snapshot.agents[0].planDone == nil)
        #expect(snapshot.agents[0].planTotal == nil)
    }

    /// And when they ARE there they survive the trip, zero included.
    ///
    /// The round trip above covers the default-nil agent this file builds; this
    /// pins the other case, because `0` is the value most likely to be lost by
    /// an encoder or a projection that treats it as empty. An agent seven tasks
    /// into seven and an agent zero tasks into seven are both real rows, and
    /// they must not arrive as the same one.
    @Test func thePlanCountsSurviveAJsonRoundTripIncludingZero() throws {
        var starting = agent("t1", status: "working")
        starting.planDone = 0
        starting.planTotal = 7
        var finishing = agent("t2", status: "working")
        finishing.planDone = 7
        finishing.planTotal = 7
        let snapshot = FleetSnapshot(
            agents: [starting, finishing],
            capturedAt: Date(timeIntervalSince1970: 1_000_000),
            complete: true)
        let decoded = try JSONDecoder().decode(
            FleetSnapshot.self, from: JSONEncoder().encode(snapshot))
        #expect(decoded == snapshot)
        #expect(decoded.agents[0].planDone == 0)
        #expect(decoded.agents[0].planTotal == 7)
        #expect(decoded.agents[1].planDone == 7)
    }

    /// Blocked and done are LATCHED: an agent waiting on you is still waiting
    /// an hour later, and nothing but a person changes that.
    @Test func aLatchedStatusStaysConfidentWhenOld() {
        let old = Date(timeIntervalSince1970: 0)
        let now = old.addingTimeInterval(FleetSnapshot.staleAfter + 1)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked"), agent("t2", status: "done")],
            capturedAt: old, complete: true)
        #expect(snapshot.confidence(in: snapshot.agents[0], at: now) == .known)
        #expect(snapshot.confidence(in: snapshot.agents[1], at: now) == .known)
    }

    /// Working is VOLATILE: an agent working an hour ago has very likely
    /// finished, and a widget that keeps asserting it is working is lying.
    @Test func aWorkingStatusDegradesWhenOld() {
        let old = Date(timeIntervalSince1970: 0)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: old, complete: true)
        #expect(
            snapshot.confidence(in: snapshot.agents[0], at: old.addingTimeInterval(60))
                == .known)
        #expect(
            snapshot.confidence(
                in: snapshot.agents[0],
                at: old.addingTimeInterval(FleetSnapshot.staleAfter + 1)) == .lastSeen)
    }

    @Test func agentsSortByRankSmallestFirst() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working", rank: 900),
                     agent("t2", status: "blocked", rank: 10)],
            capturedAt: Date(), complete: true)
        #expect(snapshot.ranked.map(\.id) == ["t2", "t1"])
    }

    /// Two agents can share a rank — same tier, same second — and `sorted` is
    /// not stable, so without a tiebreak a complication could name a different
    /// agent on every reload with nothing having changed.
    @Test func agentsSharingARankStayInOneOrder() {
        let tied = { (ids: [String]) in
            FleetSnapshot(
                agents: ids.map { agent($0, status: "blocked", rank: 7) },
                capturedAt: Date(), complete: true)
        }
        #expect(tied(["t3", "t1", "t2"]).ranked.map(\.id) == ["t1", "t2", "t3"])
        // The same agents handed over in a different order sort the same way,
        // which is what the tiebreak has to mean: the fleet arrives in whatever
        // order the daemon listed its worktrees, and that is not a promise.
        #expect(tied(["t2", "t3", "t1"]).ranked.map(\.id) == ["t1", "t2", "t3"])
    }

    /// A push carries ONE agent. Merging it must not be how the other five
    /// disappear from the widget.
    @Test func mergingOneAgentKeepsTheOthers() {
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working"), agent("t2", status: "working")],
            capturedAt: Date(timeIntervalSince1970: 0), complete: true)
        let now = Date(timeIntervalSince1970: 500)
        let after = before.merging(agent("t2", status: "blocked"), at: now)
        #expect(after.agents.count == 2)
        #expect(after.agents.first { $0.id == "t2" }?.status == "blocked")
        #expect(after.agents.first { $0.id == "t1" }?.status == "working")
        // Still what it has always meant: when this file was last assembled.
        // It is no longer what vouches for an agent — see the test below.
        #expect(after.capturedAt == now)
    }

    /// A push comes through the relay, not over the phone's link, so the
    /// runner it came from is still lost after it. Dropping `lostRunners`
    /// would put "from notifications" back on the widget with the first push.
    ///
    /// Mutation: `merging` not carrying `lostRunners`. Red.
    @Test func mergingKeepsTheLostRunners() {
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working")],
            capturedAt: Date(timeIntervalSince1970: 0), complete: false,
            lostRunners: ["Orchard"])
        let after = before.merging(
            agent("t1", status: "blocked"), at: Date(timeIntervalSince1970: 500))
        #expect(after.hedge == .lostTouch(["Orchard"]))
    }

    // MARK: - The hedge (ov-50)

    /// The two reasons a fleet isn't whole get different words: a lost runner
    /// is named, and a fleet not yet heard from in full keeps "from
    /// notifications". A complete fleet with nobody lost says nothing.
    ///
    /// Mutations: `hedge` ignoring `lostRunners`; either `footer` or
    /// `sentence` saying "from notifications" for a lost runner. Red.
    @Test func aLostRunnerHasItsOwnWords() {
        let lost = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: false,
            lostRunners: ["Orchard"])
        #expect(lost.hedge?.footer == "lost touch with Orchard")
        #expect(lost.hedge?.sentence == "Lost touch with Orchard, so its agents may have changed.")

        let partial = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: false)
        #expect(partial.hedge?.footer == "from notifications")
        #expect(partial.hedge?.sentence == "From notifications, so other agents may be missing.")

        let whole = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: true)
        #expect(whole.hedge == nil)
    }

    /// Two lost runners are both named; more than that are counted, which is
    /// what fits a widget's footer.
    @Test func severalLostRunnersAreNamedThenCounted() {
        #expect(
            FleetSnapshot.Hedge.lostTouch(["Studio", "Orchard"]).sentence
                == "Lost touch with Studio and Orchard, so their agents may have changed.")
        #expect(
            FleetSnapshot.Hedge.lostTouch(["Studio", "Orchard", "Attic"]).footer
                == "lost touch with 3 runners")
    }

    /// A snapshot written before `lostRunners` existed decodes, and hedges the
    /// way it always did.
    @Test func aSnapshotWithoutLostRunnersStillDecodes() throws {
        let json = """
            {"agents":[],"capturedAt":0,"complete":false}
            """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.lostRunners == nil)
        #expect(snapshot.hedge == .fromNotifications)
    }

    /// Merging must not re-vouch for the agents the push was not about.
    ///
    /// The one this file exists to defend. A push about A used to stamp
    /// `capturedAt = now` for the whole snapshot, so B — last actually heard
    /// from six hours ago, and `working` when it was — came back to `.known` and
    /// every widget asserted it again.
    @Test func mergingDoesNotRefreshTheAgentsItIsNotAbout() throws {
        let old = Date(timeIntervalSince1970: 0)
        let now = old.addingTimeInterval(FleetSnapshot.staleAfter * 6)
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working", activityChangedAt: old)],
            capturedAt: old, complete: true)

        let after = before.merging(agent("t2", status: "working"), at: now)
        let stale = try #require(after.agents.first { $0.id == "t1" })
        #expect(after.confidence(in: stale, at: now) == .lastSeen)

        // And the agent the push WAS about is current, with no timestamp of its
        // own to say so: `merging` stamps the fold-in rather than leaving it to
        // whichever caller assembled it.
        let fresh = try #require(after.agents.first { $0.id == "t2" })
        #expect(fresh.activityChangedAt == now)
        #expect(after.confidence(in: fresh, at: now) == .known)
    }

    /// An agent the host never dated falls back to the snapshot's own capture,
    /// which is exactly what every daemon older than `activitySince` gets.
    @Test func anUndatedAgentIsJudgedByTheSnapshot() {
        let old = Date(timeIntervalSince1970: 0)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: old, complete: true)
        #expect(
            snapshot.confidence(
                in: snapshot.agents[0], at: old.addingTimeInterval(FleetSnapshot.staleAfter + 1))
                == .lastSeen)
    }

    // MARK: - How old the news is, as against how long the state has held

    /// The bug this field was added for, stated as the user stated it.
    ///
    /// An agent that has been working for ten hours, on a fleet polled one
    /// second ago, is an agent we know perfectly well is working. It used to
    /// read "last seen working", because the only date the snapshot carried for
    /// it was when the state BEGAN — and since agents here work for many
    /// minutes at a time, that was the ordinary row rather than the odd one.
    /// The screen filled with a qualifier that meant nothing, which is how a
    /// qualifier stops being read on the row where it means something.
    @Test func anAgentWorkingForHoursIsCurrentIfWeJustHeardFromIt() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let began = now.addingTimeInterval(-FleetSnapshot.staleAfter * 10)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working", activityChangedAt: began, observedAt: now)],
            capturedAt: now, complete: true)
        #expect(snapshot.confidence(in: snapshot.agents[0], at: now) == .known)
        // And it still expires an hour after we last HEARD, not an hour after
        // it started — the staleness rule is intact, it is being measured from
        // the right date.
        #expect(
            snapshot.confidence(
                in: snapshot.agents[0], at: now.addingTimeInterval(FleetSnapshot.staleAfter + 1))
                == .lastSeen)
    }

    /// The other direction, which is the one the rule exists for: a phone out
    /// of range hears nothing, and nothing is what makes `working` unsafe to
    /// assert. A state that began recently does not rescue it.
    @Test func aFreshStateDoesNotVouchForAnAgentNobodyHasHeardFrom() {
        let heard = Date(timeIntervalSince1970: 1_000_000)
        let now = heard.addingTimeInterval(FleetSnapshot.staleAfter + 1)
        let snapshot = FleetSnapshot(
            agents: [
                // As if the host had dated this state later than we heard about
                // it. `observedAt` leads the ladder, so it decides.
                agent("t1", status: "working", activityChangedAt: now, observedAt: heard)
            ],
            capturedAt: heard, complete: true)
        #expect(snapshot.confidence(in: snapshot.agents[0], at: now) == .lastSeen)
    }

    /// A poll hears about the whole fleet at once, so every agent expires
    /// together — one moment, not one per agent.
    ///
    /// Worth pinning because the per-agent list was itself a fix, and a reader
    /// meeting a one-entry timeline could take it for that fix having been
    /// undone. It has not: the list is per agent, and the agents happen to
    /// agree because they were genuinely heard about in the same instant.
    @Test func aPolledFleetGoesStaleAllAtOnce() {
        let now = Date(timeIntervalSince1970: 10_000)
        let snapshot = FleetSnapshot(
            agents: [
                agent(
                    "t1", status: "working",
                    activityChangedAt: now.addingTimeInterval(-3_000), observedAt: now),
                agent(
                    "t2", status: "working",
                    activityChangedAt: now.addingTimeInterval(-90), observedAt: now),
            ],
            capturedAt: now, complete: true)
        #expect(
            snapshot.stalenessMoments(after: now)
                == [now.addingTimeInterval(FleetSnapshot.staleAfter)])
    }

    /// `merging` is the one place that knows which row is news, so it is the one
    /// place that can date it — and it must date only that row.
    @Test func mergingRecordsWhenWeHeardAboutTheAgentItWasGiven() throws {
        let heard = Date(timeIntervalSince1970: 1_000_000)
        let now = heard.addingTimeInterval(FleetSnapshot.staleAfter * 2)
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working", activityChangedAt: heard, observedAt: heard)],
            capturedAt: heard, complete: true)

        let after = before.merging(
            // Dated by the host as having begun long ago, which is exactly the
            // case that used to be mis-read: the state is old, the NEWS is not.
            agent(
                "t2", status: "working",
                activityChangedAt: heard.addingTimeInterval(-FleetSnapshot.staleAfter * 9)),
            at: now)

        let fresh = try #require(after.agents.first { $0.id == "t2" })
        #expect(fresh.observedAt == now)
        #expect(after.confidence(in: fresh, at: now) == .known)
        // Its state date is untouched, because a push that dated the state was
        // telling the truth about when it began.
        #expect(fresh.activityChangedAt != now)

        // And the agent this push was not about is left exactly where it was.
        let carried = try #require(after.agents.first { $0.id == "t1" })
        #expect(carried.observedAt == heard)
        #expect(after.confidence(in: carried, at: now) == .lastSeen)
    }

    /// A snapshot from a build that predates this field decodes, and behaves the
    /// way that build behaved rather than claiming to have just heard.
    @Test func aPayloadWithoutWhenWeHeardStillDecodes() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"working","glyph":"●","headline":"claude 4m","line":"x",
        "feed":[],"rank":0,"turnFailed":false,"activityChangedAt":0}],
        "capturedAt":0,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.agents[0].observedAt == nil)
        // Judged by `activityChangedAt`, which is what that build did. It
        // understates rather than overstates, which is the right way for a
        // fallback to be wrong.
        //
        // `timeIntervalSinceReferenceDate`, because the zero in that payload is
        // JSON written by `JSONEncoder`'s default date strategy — seconds since
        // 2001, not since 1970. Reading it as a Unix epoch puts `now` thirty-one
        // years BEFORE the snapshot, and `age(of:at:)` floors a negative age at
        // zero, so the test would pass for the wrong reason.
        #expect(
            snapshot.confidence(
                in: snapshot.agents[0],
                at: Date(timeIntervalSinceReferenceDate: FleetSnapshot.staleAfter + 1))
                == .lastSeen)
    }

    @Test func whenWeHeardSurvivesAJsonRoundTrip() throws {
        let now = Date(timeIntervalSince1970: 1_234_567)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working", activityChangedAt: now, observedAt: now)],
            capturedAt: now, complete: true)
        let decoded = try JSONDecoder().decode(
            FleetSnapshot.self, from: JSONEncoder().encode(snapshot))
        #expect(decoded == snapshot)
        #expect(decoded.agents[0].observedAt == now)
    }

    // MARK: - Whether a fleet would draw differently

    /// The push guard's question. Two snapshots that differ only in when the
    /// phone last heard draw identically, and the wrist has no reason to be
    /// woken for one.
    @Test func twoAgentsHeardAtDifferentTimesStillSayTheSame() {
        let then = Date(timeIntervalSince1970: 1_000)
        let now = Date(timeIntervalSince1970: 2_000)
        let early = agent("t1", status: "working", activityChangedAt: then, observedAt: then)
        let late = agent("t1", status: "working", activityChangedAt: then, observedAt: now)
        #expect(early.saysTheSame(as: late))
        #expect(early != late)
    }

    /// …and it is a real test, not one that says yes to everything. Anything a
    /// surface actually draws is still a difference.
    @Test func anythingElseIsADifference() {
        let now = Date(timeIntervalSince1970: 2_000)
        let base = agent("t1", status: "working", activityChangedAt: now, observedAt: now)
        var blocked = base
        blocked.status = "blocked"
        #expect(!base.saysTheSame(as: blocked))
        var renamed = base
        renamed.line = "Writing something else"
        #expect(!base.saysTheSame(as: renamed))
        var restated = base
        restated.activityChangedAt = now.addingTimeInterval(-60)
        #expect(!base.saysTheSame(as: restated))
    }

    /// The guard that keeps a wrist off Bluetooth twenty times a minute has to
    /// survive the trace joining the value, and this is what "survive" means.
    ///
    /// Three separate claims, because the interesting one is the middle:
    ///
    ///   1. A trace the watch would draw differently is a difference.
    ///      `saysTheSame` asks "would the watch draw anything different", the
    ///      wrist's `accessoryRectangular` is a row and §03 says a row is "mark,
    ///      label, trace" — so bytes that changed are news. Blanking them beside
    ///      `observedAt` would be a wrist drawing last hour's history.
    ///   2. **The idle case still says the same**, which is the entire point of
    ///      the guard: `WatchLinkHost.send(snapshot:)` says it "buys an IDLE
    ///      fleet only". `farcooler_core::trace` buckets on absolute wall-clock
    ///      precisely so two polls of a quiet agent inside one bucket produce
    ///      byte-identical output, and its module header names this function as
    ///      the reason. If that ever stopped holding, this assertion is where it
    ///      shows.
    ///   3. Absent and quiet are still different values, all the way up.
    @Test func aChangedTraceIsNewsAndAnUnchangedOneIsNot() {
        let now = Date(timeIntervalSince1970: 2_000)
        let quiet = ActivityTraceTests.encoded()
        let busy = ActivityTraceTests.encoded(code: Array(repeating: 300, count: 13))

        func withTrace(_ trace: Data?) -> FleetSnapshot.Agent {
            var one = agent("t1", status: "working", activityChangedAt: now, observedAt: now)
            one.trace = trace
            return one
        }

        // Two polls three seconds apart, inside one bucket: byte-identical, and
        // the watch is not written to.
        var later = withTrace(quiet)
        later.observedAt = now.addingTimeInterval(3)
        #expect(withTrace(quiet).saysTheSame(as: later))

        // A bucket that moved is a picture that moved.
        #expect(!withTrace(quiet).saysTheSame(as: withTrace(busy)))
        // And an agent that has started producing something the trace can see
        // is not the same as one that never has.
        #expect(!withTrace(nil).saysTheSame(as: withTrace(quiet)))
    }

    @Test func aFleetComparesRowForRowAndNeverMatchesNothing() {
        let now = Date(timeIntervalSince1970: 2_000)
        let fleet = { (ids: [String], observed: Date) in
            FleetSnapshot(
                agents: ids.map {
                    self.agent($0, status: "working", activityChangedAt: now, observedAt: observed)
                },
                capturedAt: observed, complete: true)
        }
        let one = fleet(["t1", "t2"], now)
        #expect(one.agentsSayTheSame(as: fleet(["t1", "t2"], now.addingTimeInterval(3))))
        // An agent that left, an agent that arrived, and the same agents in a
        // different order are all changes to what a list draws.
        #expect(!one.agentsSayTheSame(as: fleet(["t1"], now)))
        #expect(!one.agentsSayTheSame(as: fleet(["t1", "t2", "t3"], now)))
        #expect(!one.agentsSayTheSame(as: fleet(["t2", "t1"], now)))
        #expect(!one.agentsSayTheSame(as: nil))
    }

    /// A widget can only learn it has gone stale from a wake-up it schedules
    /// itself, and there has to be one for EVERY agent.
    ///
    /// Scheduling only the earliest was the shape of a real bug: the widget
    /// renders the last entry it was given from then on, so every agent that
    /// expired after that one moment stayed drawn as current for good — on
    /// exactly the surface these dates exist for, since a working push sends no
    /// alert and so triggers no reload.
    @Test func everyAgentGetsAMomentOfItsOwn() {
        let now = Date(timeIntervalSince1970: 10_000)
        let snapshot = FleetSnapshot(
            agents: [
                agent("t1", status: "working", activityChangedAt: now.addingTimeInterval(-600)),
                agent("t2", status: "working", activityChangedAt: now),
                // Same second as t2, so one moment covers both.
                agent("t4", status: "working", activityChangedAt: now),
                // Latched: it never stops being true, so it never needs a wake.
                agent("t3", status: "blocked", activityChangedAt: now.addingTimeInterval(-900)),
            ],
            capturedAt: now, complete: true)
        #expect(
            snapshot.stalenessMoments(after: now) == [
                now.addingTimeInterval(FleetSnapshot.staleAfter - 600),
                now.addingTimeInterval(FleetSnapshot.staleAfter),
            ])
    }

    /// A moment already behind us is not a wake-up, it is a render that has
    /// already happened — and an entry dated in the past is one WidgetKit drops.
    @Test func momentsAlreadyPassedAreNotScheduled() {
        let now = Date(timeIntervalSince1970: 10_000)
        let snapshot = FleetSnapshot(
            agents: [
                agent(
                    "t1", status: "working",
                    activityChangedAt: now.addingTimeInterval(-FleetSnapshot.staleAfter - 60)),
                agent("t2", status: "working", activityChangedAt: now),
            ],
            capturedAt: now, complete: true)
        #expect(
            snapshot.stalenessMoments(after: now)
                == [now.addingTimeInterval(FleetSnapshot.staleAfter)])
    }

    @Test func nothingVolatileNeedsNoWake() {
        let now = Date()
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked", activityChangedAt: now)],
            capturedAt: now, complete: true)
        #expect(snapshot.stalenessMoments(after: now).isEmpty)
        #expect(FleetSnapshot.empty.stalenessMoments(after: now).isEmpty)
    }

    /// Each moment has to state what that render is allowed to say, because the
    /// widget draws every entry from this same snapshot at that entry's date.
    @Test func eachMomentIsWhereOneAgentStopsBeingAsserted() {
        let now = Date(timeIntervalSince1970: 10_000)
        let early = agent("t1", status: "working", activityChangedAt: now.addingTimeInterval(-600))
        let late = agent("t2", status: "working", activityChangedAt: now)
        let snapshot = FleetSnapshot(agents: [early, late], capturedAt: now, complete: true)
        let moments = snapshot.stalenessMoments(after: now)

        #expect(snapshot.confidence(in: early, at: moments[0]) == .lastSeen)
        #expect(snapshot.confidence(in: late, at: moments[0]) == .known)
        #expect(snapshot.confidence(in: late, at: moments[1]) == .lastSeen)
    }

    @Test func mergingAnUnknownAgentAddsIt() {
        let before = FleetSnapshot.empty
        let after = before.merging(agent("t9", status: "blocked"), at: Date())
        #expect(after.agents.map(\.id) == ["t9"])
    }

    /// A snapshot assembled only from pushes knows about the agents that
    /// happened to notify and nothing else. Rendering it as the fleet would
    /// assert that the other five do not exist.
    @Test func aSnapshotBuiltOnlyFromPushesIsNeverComplete() {
        var snapshot = FleetSnapshot.empty
        #expect(snapshot.complete == false)
        for id in ["t1", "t2", "t3"] {
            snapshot = snapshot.merging(agent(id, status: "blocked"), at: Date())
        }
        #expect(snapshot.complete == false)
    }

    @Test func mergingIntoACompleteSnapshotKeepsItComplete() {
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: Date(), complete: true)
        #expect(before.merging(agent("t1", status: "done"), at: Date()).complete)
    }

    @Test func needingYouCountsOnlyBlockedAgents() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked"), agent("t2", status: "working"),
                     agent("t3", status: "done"), agent("t4", status: "blocked")],
            capturedAt: Date(), complete: true)
        #expect(snapshot.needingYou == 2)
    }

    // MARK: - Reviews

    /// The compatibility rule the whole optional exists for. A snapshot written
    /// by a build that predates review counts — or by a phone whose runner is
    /// too old to answer `changes.inbox` — must still decode, and must read as
    /// "not told" rather than as a confident zero. Swift's synthesized
    /// `Decodable` throws on a missing key for a non-optional, so getting this
    /// wrong does not cost a review line: it costs the whole widget.
    @Test func aSnapshotWithoutReviewCountsStillDecodes() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"working","glyph":"●","headline":"claude 4m","line":"x",
        "feed":[],"rank":0,"turnFailed":false}],
        "capturedAt":1000000,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.agents.count == 1)
        #expect(snapshot.needsReview == nil)
        // And it still renders: the fleet has something to say, and what it says
        // is about the working agent rather than about reviews it knows nothing
        // of.
        #expect(snapshot.glance(at: Date(timeIntervalSince1970: 1_000_010)) == .working(1))
    }

    /// Nil and zero are different answers, and every surface branches on the
    /// difference. Zero is "nothing is waiting"; nil is "nobody told me".
    @Test func anAbsentReviewCountIsNotZero() {
        let base = FleetSnapshot(agents: [], capturedAt: Date(), complete: true)
        #expect(base.needsReview == nil)
        let told = FleetSnapshot(
            agents: [], capturedAt: Date(), complete: true, reviewsWaiting: 0)
        #expect(told.needsReview == 0)
    }

    /// A review count survives a JSON round trip, so the file a widget reads
    /// says what the app wrote.
    @Test func aReviewCountRoundTripsThroughJson() throws {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")],
            capturedAt: Date(timeIntervalSince1970: 1_000_000),
            complete: true, reviewsWaiting: 3)
        let data = try JSONEncoder().encode(snapshot)
        #expect(try JSONDecoder().decode(FleetSnapshot.self, from: data) == snapshot)
    }

    /// A push is about ONE agent's turn ending and says nothing about whether
    /// some other worktree's diff moved. Clearing the count on every push would
    /// take the review line off every surface each time an unrelated agent
    /// notified — a worse answer than a count that is a poll or two old.
    @Test func mergingKeepsTheReviewCountItWasNotToldAbout() {
        let before = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: Date(),
            complete: true, reviewsWaiting: 3)
        #expect(before.merging(agent("t2", status: "blocked"), at: Date()).needsReview == 3)
    }

    /// And a snapshot that never knew stays not-knowing: `merging` has nothing
    /// to learn a count from either.
    @Test func mergingIntoASnapshotWithoutReviewCountsTellsItNothing() {
        let after = FleetSnapshot.empty.merging(agent("t1", status: "blocked"), at: Date())
        #expect(after.needsReview == nil)
    }

    // MARK: - The glance

    /// Blocked outranks everything. An agent that cannot continue is the one
    /// thing a surface with room for one number is for.
    @Test func blockedLeadsOverReviewsAndWork() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked"), agent("t2", status: "working")],
            capturedAt: Date(), complete: true, reviewsWaiting: 9)
        #expect(snapshot.glance(at: Date()) == .blocked(1))
    }

    /// The user's specific ask: with nothing blocked, the reviews are what the
    /// circular slot shows — and `.review` rather than `.working` is what makes
    /// the two tellable apart, because the case carries the glyph and the tint.
    @Test func reviewsLeadWhenNothingIsBlocked() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working"), agent("t2", status: "working")],
            capturedAt: Date(), complete: true, reviewsWaiting: 3)
        #expect(snapshot.glance(at: Date()) == .review(3))
    }

    /// Work is the last rung, and it is reached both by a fleet told there is
    /// nothing to review and by one never told anything.
    @Test func workLeadsWhenNothingIsBlockedOrWaiting() {
        let agents = [agent("t1", status: "working"), agent("t2", status: "done")]
        let told = FleetSnapshot(
            agents: agents, capturedAt: Date(), complete: true, reviewsWaiting: 0)
        #expect(told.glance(at: Date()) == .working(1))
        let untold = FleetSnapshot(agents: agents, capturedAt: Date(), complete: true)
        #expect(untold.glance(at: Date()) == .working(1))
    }

    /// The working rung is the only volatile one, so it obeys the same rule
    /// every row does: an agent this snapshot can no longer vouch for is not
    /// counted. Past that point the fleet-wide claim stops being made at all
    /// rather than degrading into a reassuring "0 working".
    @Test func workIsNotCountedOnceItCannotBeAsserted() {
        let began = Date(timeIntervalSince1970: 0)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working", activityChangedAt: began)],
            capturedAt: began, complete: true)
        #expect(snapshot.glance(at: began.addingTimeInterval(60)) == .working(1))
        #expect(snapshot.glance(at: began.addingTimeInterval(FleetSnapshot.staleAfter + 1)) == nil)
    }

    /// Blocked and waiting-to-be-reviewed are LATCHED: an agent stopped an hour
    /// ago is still stopped, and a diff nobody reviewed is still unreviewed. Age
    /// must not take either of them off a surface.
    @Test func theLatchedRungsSurviveAnOldSnapshot() {
        let began = Date(timeIntervalSince1970: 0)
        let old = began.addingTimeInterval(FleetSnapshot.staleAfter * 5)
        let blocked = FleetSnapshot(
            agents: [agent("t1", status: "blocked", activityChangedAt: began)],
            capturedAt: began, complete: true, reviewsWaiting: 2)
        #expect(blocked.glance(at: old) == .blocked(1))
        let reviews = FleetSnapshot(
            agents: [agent("t1", status: "done", activityChangedAt: began)],
            capturedAt: began, complete: true, reviewsWaiting: 2)
        #expect(reviews.glance(at: old) == .review(2))
    }

    /// An empty fleet is about nothing, and a nil glance is what lets each
    /// surface keep the sentence it already had for that — "No agents", "Open
    /// <app>", or the top agent in the past tense.
    @Test func aFleetWithNothingToSayHasNoGlance() {
        #expect(FleetSnapshot.empty.glance(at: Date()) == nil)
        let quiet = FleetSnapshot(
            agents: [agent("t1", status: "done")], capturedAt: Date(),
            complete: true, reviewsWaiting: 0)
        #expect(quiet.glance(at: Date()) == nil)
    }

    // MARK: - The rendering rule

    /// The table itself, asserted. These three symbols and these three words are
    /// the whole cross-surface contract: a widget, a lock screen accessory and a
    /// complication draw them from here so they cannot come to differ, and a
    /// change to any of them is a change to what four surfaces mean.
    @Test func eachStateHasItsOwnMarkAndWords() {
        #expect(FleetSnapshot.Glance.blocked(2).symbol == "exclamationmark.triangle.fill")
        #expect(FleetSnapshot.Glance.review(3).symbol == "plus.forwardslash.minus")
        #expect(FleetSnapshot.Glance.working(4).symbol == "checkmark")
        #expect(FleetSnapshot.Glance.blocked(2).phrase == "2 need you")
        #expect(FleetSnapshot.Glance.review(3).phrase == "3 to review")
        #expect(FleetSnapshot.Glance.working(4).phrase == "4 working")
    }

    /// One agent is not "1 need you". These lines are read as sentences on a
    /// lock screen, and a surface that cannot conjugate reads as broken.
    @Test func oneOfSomethingIsSaidInTheSingular() {
        #expect(FleetSnapshot.Glance.blocked(1).phrase == "1 needs you")
        #expect(FleetSnapshot.Glance.blocked(1).caption == "needs you")
        #expect(FleetSnapshot.Glance.review(1).caption == "worktree to review")
        #expect(FleetSnapshot.Glance.working(1).caption == "agent working")
    }

    /// Reviews are counted in WORKTREES and what needs you in items, because
    /// they are counts of different things — `changes.inbox` answers per
    /// worktree, and an item can be a decision with no agent. A caption that
    /// called both of them agents would make "2 need you" and "3 to review"
    /// look like five agents.
    @Test func theTwoCountsAreCountsOfDifferentThings() {
        #expect(FleetSnapshot.Glance.blocked(2).caption == "need you")
        #expect(FleetSnapshot.Glance.review(3).caption == "worktrees to review")
        #expect(FleetSnapshot.Glance.working(4).caption == "agents working")
    }

    /// `count` is the number a one-number family draws, whichever rung it came
    /// from. It has to come off the same value the glyph and the tint do, or a
    /// circular slot ends up with an amber triangle over a review count.
    @Test func theNumberComesOffTheSameAnswerAsTheMark() {
        #expect(FleetSnapshot.Glance.blocked(2).count == 2)
        #expect(FleetSnapshot.Glance.review(3).count == 3)
        #expect(FleetSnapshot.Glance.working(4).count == 4)
    }

    // MARK: - A runner that isn't answering

    /// A working agent on a runner the phone has lost is "can't say" at once,
    /// not after an hour. The hour is for a phone that has stopped looking;
    /// here the phone looked and could not reach it.
    ///
    /// Mutation: `confidence(in:at:)` passing `answering: true`. Red: the
    /// working agent reads `.known`.
    @Test func aLostRunnersWorkingAgentIsNotVouchedFor() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        var working = agent("t1", status: "working", observedAt: now)
        working.runnerAnswering = false
        let snapshot = FleetSnapshot(agents: [working], capturedAt: now, complete: true)

        #expect(snapshot.confidence(in: working, at: now) == .lastSeen)
        #expect(snapshot.glance(at: now) == nil)
        #expect(snapshot.stalenessMoments(after: now).isEmpty)
    }

    /// Blocked and done hold, exactly as they do at any age: an agent that
    /// stopped for you is still stopped for you, whether or not the link held.
    @Test func aLostRunnersLatchedAgentsHold() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        var blocked = agent("t1", status: "blocked", observedAt: now)
        blocked.runnerAnswering = false
        var done = agent("t2", status: "done", observedAt: now)
        done.runnerAnswering = false
        let snapshot = FleetSnapshot(agents: [blocked, done], capturedAt: now, complete: true)

        #expect(snapshot.confidence(in: blocked, at: now) == .known)
        #expect(snapshot.confidence(in: done, at: now) == .known)
        #expect(snapshot.glance(at: now) == .blocked(1))
    }

    /// Nil is "not told" and reads as answering, which is every snapshot
    /// written before the field and every push folded in since.
    @Test func anAgentNobodySaidAnythingAboutIsAnswering() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"working","glyph":"●","headline":"claude 4m","line":"x",
        "feed":[],"rank":0,"turnFailed":false,"observedAt":1000000}],
        "capturedAt":1000000,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        let now = Date(timeIntervalSince1970: 1_000_000)
        #expect(snapshot.agents.first?.runnerAnswering == nil)
        #expect(snapshot.glance(at: now) == .working(1))
    }

    /// The flag survives the trip to disk and to the watch.
    @Test func aLostRunnerRoundTripsThroughJson() throws {
        var working = agent("t1", status: "working")
        working.runnerAnswering = false
        let snapshot = FleetSnapshot(
            agents: [working], capturedAt: Date(timeIntervalSince1970: 1_000_000), complete: true)
        let data = try JSONEncoder().encode(snapshot)
        let back = try JSONDecoder().decode(FleetSnapshot.self, from: data)
        #expect(back.agents.first?.runnerAnswering == false)
    }

    /// A push about an agent is news about it, through the relay, whatever the
    /// app last knew of its runner's link.
    ///
    /// Mutation: `merging(_:at:)` without `incoming.runnerAnswering = nil`.
    /// Red: the pushed agent keeps `false`.
    @Test func aPushedAgentIsHeardFromWhateverItsRunnersLink() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        var lost = agent("t1", status: "working", observedAt: now)
        lost.runnerAnswering = false
        let snapshot = FleetSnapshot(agents: [lost], capturedAt: now, complete: true)

        let merged = snapshot.merging(lost, at: now)
        #expect(merged.agents.first?.runnerAnswering == nil)
        #expect(merged.glance(at: now) == .working(1))
    }

    // MARK: - Needs You (ov-55 4C.1)

    private func item(
        _ id: String, _ kind: NeedsYouKind, rank: UInt32, terminal: String? = nil,
        runner: String = "r1"
    ) -> NeedsYouItem {
        NeedsYouItem(
            id: id, kind: kind, rank: rank, since: nil, workspaceName: "Billing",
            terminal: terminal.map {
                NeedsYouTerminal(
                    id: $0, worktreeID: nil, label: "claude", role: "agent",
                    paneMode: "terminal", chatCapable: true)
            },
            question: "Allow touch x", askID: kind == .ask ? "hook-ask-1" : nil,
            actions: kind == .ask
                ? [NeedsYouAction(id: "allow", title: "Allow touch x", destructive: false, primary: true),
                   NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false)]
                : [],
            runner: runner)
    }

    /// Every snapshot on disk today, and every one an older build writes. It
    /// must decode, say it holds no list, and count blocked agents exactly as
    /// before.
    ///
    /// Mutation: `storedNeedsYou` non-optional. Red: the decode throws on the
    /// missing key.
    @Test("A snapshot without needsYou decodes as before")
    func aSnapshotWithoutNeedsYouDecodesAsBefore() throws {
        let json = """
        {"agents":[{"id":"t1","label":"claude","machine":"orchard",
        "status":"blocked","glyph":"?","headline":"claude","line":"x",
        "feed":[],"rank":0,"turnFailed":false}],
        "capturedAt":1000000,"complete":true}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.needsYou == nil)
        #expect(snapshot.needingYou == 1)
        #expect(snapshot.glance(at: Date()) == .blocked(1))
    }

    /// The widget, the complication and the watch show the app's number: the
    /// items, not the blocked agents. Here one agent is blocked on an ask, and
    /// a decision and a review have no agent at all.
    ///
    /// Mutation: `needingYou` counting blocked agents whatever `needsYou`
    /// says. Red: 1, not 3.
    @Test("The widget's count is the item count")
    func theWidgetsCountIsTheItemCount() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked"), agent("t2", status: "working")],
            capturedAt: Date(), complete: true,
            needsYou: [
                item("ask:1", .ask, rank: 5, terminal: "t1"),
                item("decision:7", .decision, rank: 200_000_005),
                item("review:8", .review, rank: 300_000_005),
            ])
        #expect(snapshot.needingYou == 3)
        #expect(snapshot.glance(at: Date()) == .blocked(3))
    }

    /// An empty list is an answer: nothing needs you, whatever an agent's
    /// status word says. Only nil falls back to counting agents.
    ///
    /// Mutation: `needingYou` falling back when the list is empty. Red: 1.
    @Test func anEmptyListIsAnAnswerNotAFallback() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked")], capturedAt: Date(), complete: true,
            needsYou: [])
        #expect(snapshot.needingYou == 0)
    }

    /// The file keeps what the wire doesn't carry: which runner an item came
    /// from, and whether the app derived it. Written the way `SnapshotStore`
    /// writes, seconds since 1970.
    ///
    /// Mutation: `StoredItem.init(_:)` dropping `runner`. Red: "" back.
    @Test func anItemRoundTripsWithItsRunnerAndWhetherItWasDerived() throws {
        var derived = item("blocked:t2", .blocked, rank: 100_000_001, terminal: "t2", runner: "r2")
        derived.isDerived = true
        derived.since = Date(timeIntervalSince1970: 1_789_999_940)
        let snapshot = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1_000_000), complete: true,
            needsYou: [item("ask:1", .ask, rank: 5, terminal: "t1"), derived])
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        let back = try decoder.decode(FleetSnapshot.self, from: encoder.encode(snapshot))
        #expect(back == snapshot)
        #expect(back.needsYou?.map(\.runner) == ["r1", "r2"])
        #expect(back.needsYou?.last?.isDerived == true)
        #expect(back.needsYou?.first?.actions.map(\.id) == ["allow", "deny"])
    }

    /// A kind a later build writes reads as unknown rather than failing the
    /// file: the rule the wire's decoder keeps, kept on disk.
    @Test func anUnknownKindInTheFileReadsAsUnknown() throws {
        let json = """
        {"agents":[],"capturedAt":1000000,"complete":true,
        "needsYou":[{"id":"x:1","kind":"summons","also":[],"rank":5,
        "workspaceName":"","question":"?","actions":[],"runner":"r1","derived":false}]}
        """
        let snapshot = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.needsYou?.first?.kind == .unknown)
        #expect(snapshot.needingYou == 1)
    }

    /// A push about an agent that blocked while the app was closed counts at
    /// once, as it did before the count was items.
    ///
    /// Mutation: `merging` carrying `needsYou` unchanged. Red: 1, not 2.
    @Test func aPushedBlockCountsBeforeTheAppReadsItsList() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "working")], capturedAt: now, complete: true,
            needsYou: [item("decision:7", .decision, rank: 200_000_005)])
        let merged = snapshot.merging(agent("t1", status: "blocked", rank: 30), at: now)
        #expect(merged.needingYou == 2)
        #expect(merged.needsYou?.map(\.kind) == [.blocked, .decision])
        #expect(merged.needsYou?.first?.isDerived == true)
    }

    /// An agent already counted by its ask isn't counted twice for blocking.
    ///
    /// Mutation: the guard on an item already about the terminal. Red: 2.
    @Test func aPushedBlockOnAnAgentWithAnAskCountsOnce() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked")], capturedAt: now, complete: true,
            needsYou: [item("ask:1", .ask, rank: 5, terminal: "t1")])
        #expect(snapshot.merging(agent("t1", status: "blocked"), at: now).needingYou == 1)
    }

    /// An agent that's working again holds no ask and isn't blocked, so its
    /// items go; a decision about its task stays.
    ///
    /// Mutation: the filter keeping every item. Red: 2, not 1.
    @Test func aPushedAgentWorkingAgainTakesItsAskAway() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked")], capturedAt: now, complete: true,
            needsYou: [
                item("ask:1", .ask, rank: 5, terminal: "t1"),
                item("decision:7", .decision, rank: 200_000_005),
            ])
        let merged = snapshot.merging(agent("t1", status: "working"), at: now)
        #expect(merged.needsYou?.map(\.itemID) == ["decision:7"])
    }

    /// No list stays no list: a push can't make one out of one agent.
    @Test func aPushIntoASnapshotWithNoListLeavesNoList() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let snapshot = FleetSnapshot(agents: [], capturedAt: now, complete: true)
        #expect(snapshot.merging(agent("t1", status: "blocked"), at: now).needsYou == nil)
    }
}
