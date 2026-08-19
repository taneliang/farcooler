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
}
