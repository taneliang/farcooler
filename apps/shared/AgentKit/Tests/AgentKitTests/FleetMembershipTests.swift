import Foundation
import Testing

@testable import AgentKit

/// The rules a reconcile has to keep, on a runner shaped like the app's but
/// with nothing to connect to.
///
/// Two fields rather than one, because the whole point of the plan is that they
/// move independently: `id` is what a connection is keyed by and what survives
/// an edit, and `address` is a detail that changing must cost a rebuild.
private struct Runner: Identifiable, Equatable {
    var id: String
    var address: String = "10.0.0.1"
}

@MainActor
struct FleetMembershipTests {
    // MARK: - The battery gate

    /// The default, and the shape the whole port is for: every runner at once.
    @Test func everyRunnerAtOnceWantsEveryRunner() {
        let all = [Runner(id: "a"), Runner(id: "b"), Runner(id: "c")]
        let wanted = FleetMembership.wanted(all: all, selected: "b", everyRunnerAtOnce: true)
        #expect(wanted.map(\.id) == ["a", "b", "c"])
    }

    /// The gate off is the phone on a train: one runner, the one that was
    /// picked, and the selection is what decides it rather than the list order.
    @Test func theGateOffWantsOnlyTheSelectedRunner() {
        let all = [Runner(id: "a"), Runner(id: "b"), Runner(id: "c")]
        let wanted = FleetMembership.wanted(all: all, selected: "b", everyRunnerAtOnce: false)
        #expect(wanted.map(\.id) == ["b"])
    }

    /// Nothing picked, and the gate off, is not an invitation to pick one for
    /// somebody. See `wanted`.
    @Test func theGateOffWithNothingSelectedWantsNothing() {
        let all = [Runner(id: "a"), Runner(id: "b")]
        let wanted = FleetMembership.wanted(all: all, selected: nil, everyRunnerAtOnce: false)
        #expect(wanted.isEmpty)
    }

    /// A selection left pointing at a runner somebody removed. The filter
    /// matches nothing, which is the honest answer — and notably NOT the whole
    /// list, which is what a `firstOrNil`-shaped fallback would have produced.
    @Test func theGateOffWithAStaleSelectionWantsNothing() {
        let all = [Runner(id: "a"), Runner(id: "b")]
        let wanted = FleetMembership.wanted(all: all, selected: "gone", everyRunnerAtOnce: false)
        #expect(wanted.isEmpty)
    }

    // MARK: - The plan

    @Test func aRunnerWithNoConnectionIsStarted() {
        let plan = FleetMembership.plan(
            wanted: [Runner(id: "a"), Runner(id: "b")], existing: ["a": Runner(id: "a")])
        #expect(plan.started == ["b"])
        #expect(plan.kept == ["a"])
        #expect(plan.rebuilt.isEmpty)
        #expect(plan.retired.isEmpty)
    }

    /// **The rule with no visible symptom.** A reconcile that rebuilt on every
    /// runner-list change would drop a live connection and its fleet, come back
    /// a second later, and look identical in a screenshot — while costing a full
    /// SSH bring-up per keystroke in the runner editor beside it. Nothing but an
    /// assertion on `kept` catches that.
    @Test func anUnchangedRunnerIsKeptAndNotRebuilt() {
        let same = Runner(id: "a", address: "10.0.0.1")
        let plan = FleetMembership.plan(wanted: [same], existing: ["a": same])
        #expect(plan.kept == ["a"])
        #expect(plan.rebuilt.isEmpty)
        #expect(plan.started.isEmpty)
        #expect(plan.retired.isEmpty)
    }

    /// Correcting a mistyped address is as much a change of runner as picking a
    /// different one. Reusing the connection would leave the old session running
    /// under the new details, which is the state nobody can debug.
    @Test func anEditedRunnerIsRebuilt() {
        let plan = FleetMembership.plan(
            wanted: [Runner(id: "a", address: "10.0.0.9")],
            existing: ["a": Runner(id: "a", address: "10.0.0.1")])
        #expect(plan.rebuilt == ["a"])
        #expect(plan.kept.isEmpty)
        #expect(plan.started.isEmpty)
        // Not also retired: the four lists are disjoint, so a caller that
        // retires `retired` and starts `started` would do nothing at all to a
        // rebuilt runner if this leaked into either.
        #expect(plan.retired.isEmpty)
    }

    @Test func aRunnerNoLongerWantedIsRetired() {
        let plan = FleetMembership.plan(
            wanted: [Runner(id: "a")],
            existing: ["a": Runner(id: "a"), "b": Runner(id: "b")])
        #expect(plan.retired == ["b"])
        #expect(plan.kept == ["a"])
        #expect(plan.started.isEmpty)
        #expect(plan.rebuilt.isEmpty)
    }

    /// Turning the gate off does not remove the runners — it narrows what
    /// `wanted` hands over — so every other connection arrives here as
    /// something to retire. This is the two halves working as one.
    @Test func theGateOffRetiresEveryOtherConnection() {
        let all = [Runner(id: "a"), Runner(id: "b"), Runner(id: "c")]
        let existing = Dictionary(uniqueKeysWithValues: all.map { ($0.id, $0) })
        let wanted = FleetMembership.wanted(all: all, selected: "b", everyRunnerAtOnce: false)
        let plan = FleetMembership.plan(wanted: wanted, existing: existing)
        #expect(plan.kept == ["b"])
        #expect(plan.retired == ["a", "c"])
        #expect(plan.started.isEmpty)
    }

    /// The order runners are brought up in is the order they are listed in, so
    /// a fresh app with three runners does not start them in whatever order a
    /// dictionary happened to hash.
    @Test func startedFollowsTheRunnerListOrder() {
        let plan = FleetMembership.plan(
            wanted: [Runner(id: "c"), Runner(id: "a"), Runner(id: "b")], existing: [:])
        #expect(plan.started == ["c", "a", "b"])
    }

    /// One runner listed twice is one runner. The store holds one connection
    /// per id, so a plan that started two would leave the first with nothing
    /// pointing at it and no way to shut it down.
    @Test func aRunnerListedTwiceIsPlannedOnce() {
        let plan = FleetMembership.plan(
            wanted: [Runner(id: "a"), Runner(id: "a")], existing: [:])
        #expect(plan.started == ["a"])
    }

    /// Nothing wanted and nothing connected is not an error, and it is the
    /// state a phone with no runners added is in.
    @Test func anEmptyFleetPlansNothing() {
        let plan = FleetMembership.plan(wanted: [Runner](), existing: [:])
        #expect(plan == FleetMembership.Plan<String>())
    }

    // MARK: - The merged publish

    @Test func publishedFollowsTheRunnerListOrder() {
        let merged = FleetMembership.published(
            order: ["c", "a", "b"], live: ["a": "A", "b": "B", "c": "C"])
        #expect(merged == ["C", "A", "B"])
    }

    /// A runner the gate filtered out, or one whose bring-up has not happened
    /// yet, contributes nothing rather than a gap.
    @Test func aRunnerWithNoConnectionContributesNothing() {
        let merged = FleetMembership.published(order: ["a", "b", "c"], live: ["a": "A", "c": "C"])
        #expect(merged == ["A", "C"])
    }

    /// **The merge bug worth naming.** A connection left under an id that is no
    /// longer in the runner list must not reach the screen: that is a runner
    /// nobody is polling, contributing its last rows forever. Driving the walk
    /// from `order` is what makes it structurally impossible; driving it from
    /// `live` would put "Z" in this list.
    @Test func aConnectionUnderAForgottenRunnerIsNotPublished() {
        let merged = FleetMembership.published(
            order: ["a", "b"], live: ["a": "A", "b": "B", "zombie": "Z"])
        #expect(merged == ["A", "B"])
    }
}
