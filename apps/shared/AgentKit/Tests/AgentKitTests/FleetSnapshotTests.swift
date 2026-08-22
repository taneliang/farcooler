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
        activityChangedAt: Date? = nil
    ) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: "claude", machine: "orchard", status: status,
            glyph: "●", headline: "claude 4m", line: "Writing fruit.txt",
            feed: ["Reading watch.rs."], rank: rank, turnFailed: false,
            activityChangedAt: activityChangedAt)
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
        // order the daemon listed its workspaces, and that is not a promise.
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
        #expect(FleetSnapshot.Glance.blocked(1).caption == "agent needs you")
        #expect(FleetSnapshot.Glance.review(1).caption == "workspace to review")
        #expect(FleetSnapshot.Glance.working(1).caption == "agent working")
    }

    /// Reviews are counted in WORKSPACES and blocked agents in agents, because
    /// they are counts of different things — `changes.inbox` answers per
    /// workspace. A caption that called both of them agents would make "2 need
    /// you" and "3 to review" look like five agents.
    @Test func theTwoCountsAreCountsOfDifferentThings() {
        #expect(FleetSnapshot.Glance.blocked(2).caption == "agents need you")
        #expect(FleetSnapshot.Glance.review(3).caption == "workspaces to review")
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
}
