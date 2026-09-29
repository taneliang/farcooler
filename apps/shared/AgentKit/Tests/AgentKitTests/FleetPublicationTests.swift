import Foundation
import Testing

@testable import AgentKit

/// The one file every out-of-process surface renders from, assembled from N
/// runners instead of overwritten by whichever polled last.
///
/// Each of these fails against the behavior this replaces — a whole-file
/// rewrite — and the two that matter most are the ones a single-connection app
/// could never reach: a second runner's agents surviving the first runner's
/// poll, and a RETIRED runner's agents not surviving anything.
struct FleetPublicationTests {
    private let now = Date(timeIntervalSince1970: 1_000_000)

    private func agent(_ id: String, machine: String) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: id, machine: machine, status: "working", glyph: "", headline: "",
            line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)
    }

    private func snapshot(
        _ agents: [FleetSnapshot.Agent], complete: Bool = true, reviews: Int? = nil,
        trace: Data? = nil
    ) -> FleetSnapshot {
        FleetSnapshot(
            agents: agents, capturedAt: now, complete: complete, reviewsWaiting: reviews,
            fleetTrace: trace)
    }

    // MARK: - The clobbering this replaces

    /// **The whole point.** One runner polling does not remove another runner's
    /// agents. Against the writer this replaces — `SnapshotStore.write` with one
    /// runner's whole projection — the second call leaves only `b`'s row.
    @Test func oneRunnersPollDoesNotEraseAnothers() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "laptop")]))
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "gpu-box")]))

        let merged = publication.merged(at: now)
        #expect(merged.agents.map(\.id) == ["t1", "t2"])
    }

    /// A runner polling again replaces its OWN rows rather than adding to them.
    /// An accumulating merge would resurrect an agent that has exited, on every
    /// poll, forever.
    @Test func aRunnerRepollingReplacesItsOwnRows() {
        var publication = FleetPublication()
        publication.record(
            runner: "a", snapshot: snapshot([agent("t1", machine: "l"), agent("t2", machine: "l")]))
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))

        #expect(publication.merged(at: now).agents.map(\.id) == ["t1"])
    }

    /// The order is the order runners were recorded in, not a dictionary's.
    /// Hash order changes between launches, and a lock screen list that
    /// reshuffled on relaunch would be a different fleet every morning.
    @Test func theOrderIsStableAcrossRepolls() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "g")]))
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))

        #expect(publication.merged(at: now).agents.map(\.id) == ["t1", "t2"])
    }

    // MARK: - Liveness, which is what made merging safe to do at all

    /// **The bug merging creates if liveness is not tracked**: a runner nobody
    /// is polling, with its agents on the lock screen forever. This is why the
    /// merge could not land before the store did.
    @Test func aRetiredRunnersAgentsLeaveTheSnapshot() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "g")]))

        publication.keeping(runners: ["a"])

        #expect(publication.merged(at: now).agents.map(\.id) == ["t1"])
    }

    /// And it stays gone. A record for a runner that is live again is what
    /// brings it back, which is exactly a reconnect.
    @Test func aRetiredRunnerComesBackByBeingPolled() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.keeping(runners: [])
        #expect(publication.merged(at: now).agents.isEmpty)

        publication.keeping(runners: ["a"])
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        #expect(publication.merged(at: now).agents.map(\.id) == ["t1"])
    }

    // MARK: - A runner that isn't answering

    /// **The reason `answering` exists.** A runner that has lost its link keeps
    /// its rows, so the fleet on the lock screen stays put, but none of its
    /// agents may be counted as working: the fleet they came from was read
    /// before the link went. Its neighbor, still answering, is untouched.
    ///
    /// Mutation: `merged(at:)` returning `contribution.snapshot.agents` without
    /// the `lost` branch. Red: `t2` reads nil and `.working(2)`.
    @Test func aLostRunnersAgentsStayButAreNotCountedAsWorking() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "laptop")]))
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "gpu-box")]))

        publication.keeping(runners: ["a", "b"], answering: ["a"])

        let merged = publication.merged(at: now)
        #expect(merged.agents.map(\.id) == ["t1", "t2"])
        #expect(merged.agents.map(\.runnerAnswering) == [nil, false])
        #expect(merged.glance(at: now) == .working(1))
    }

    /// Answering again is not enough to vouch for the old rows. A link is up a
    /// whole SSH round trip before the first poll over it lands, and until
    /// then these are the rows read before it went. The poll is what clears it.
    ///
    /// Mutation: `keeping(runners:answering:)` clearing `lost` for a runner in
    /// `answering`. Red on the second expectation.
    @Test func aRunnerBackOnlyVouchesForWhatItsNextPollSays() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.keeping(runners: ["a"], answering: [])
        #expect(publication.merged(at: now).agents.first?.runnerAnswering == false)

        publication.keeping(runners: ["a"], answering: ["a"])
        #expect(publication.merged(at: now).agents.first?.runnerAnswering == false)

        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        #expect(publication.merged(at: now).agents.first?.runnerAnswering == nil)
    }

    /// Losing a runner with rows is news for the surfaces; a launch where
    /// nothing has answered yet still is not. See
    /// `aMembershipSettledBeforeAnybodyPolledSaysNothing`.
    @Test func losingARunnerIsWorthTellingButALaunchIsNot() {
        var launch = FleetPublication()
        let launchTold = launch.keeping(runners: ["a"], answering: [])
        #expect(!launchTold)

        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        let told = publication.keeping(runners: ["a"], answering: [])
        #expect(told)
    }

    // MARK: - The launch that used to clobber the file

    /// **The regression this answers.** `FleetStore.publish` settles the
    /// membership before any runner has been polled, so the first call of every
    /// launch has nothing recorded and nothing to forget — and the merge it
    /// would produce is an empty fleet stamped with a real `capturedAt`, which
    /// every surface reads as a look at the fleet rather than as the absence of
    /// one. Answering false is what keeps that off the disk the widget and the
    /// watch read.
    @Test func aMembershipSettledBeforeAnybodyPolledSaysNothing() {
        var publication = FleetPublication()
        #expect(publication.keeping(runners: ["a", "b"]) == false)
        #expect(publication.isEmpty)
    }

    /// And it goes on saying nothing for as long as the first poll never lands
    /// — which is the whole of an offline launch. A runner added, then edited,
    /// then filtered out by the battery gate is three membership changes and
    /// still not one observation.
    @Test func everyMembershipChangeBeforeTheFirstPollSaysNothing() {
        var publication = FleetPublication()
        _ = publication.keeping(runners: ["a"])
        #expect(publication.keeping(runners: ["a", "b"]) == false)
        #expect(publication.keeping(runners: ["b"]) == false)
        #expect(publication.keeping(runners: []) == false)
    }

    /// A runner retiring IS an observation, and it is the one the surfaces
    /// would otherwise never hear: a store that has just dropped its last
    /// connection has no next poll from anybody, so the rows would stay on the
    /// lock screen claiming to be working forever.
    @Test func retiringTheLastRunnerIsWorthTelling() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))

        #expect(publication.keeping(runners: []) == true)
        #expect(publication.merged(at: now).agents.isEmpty)
    }

    /// So is a membership change over a publication that still holds rows: what
    /// `live` says decides whether the merge is `complete`, and that is a
    /// sentence on the widget.
    @Test func aMembershipChangeOverRecordedRowsIsWorthTelling() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))

        #expect(publication.keeping(runners: ["a", "b"]) == true)
        #expect(publication.merged(at: now).complete == false)
    }

    // MARK: - complete

    /// `complete` says "these are all the agents there are". Two runners live
    /// and one heard from is a fleet with agents missing, so the surfaces have
    /// to hedge — the widget's "from notifications", the watch's partial
    /// footer.
    @Test func aFleetMissingARunnerIsNotComplete() {
        var publication = FleetPublication()
        publication.keeping(runners: ["a", "b"])
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))

        #expect(publication.merged(at: now).complete == false)
    }

    /// **A runner that stopped answering is not heard from** (ov-22 M2). Its
    /// rows stay, marked lost, and it may have started agents since that
    /// nobody has been told about, so the merge can't say it has them all.
    /// It counted toward `complete` like an answering runner, and the widget
    /// dropped its hedge, the watch its partial footer.
    ///
    /// Mutation: `complete` ignoring `lost`. Red.
    @Test func aLostRunnerLeavesTheFleetIncomplete() {
        var publication = FleetPublication()
        publication.keeping(runners: ["a", "b"])
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "g")]))
        publication.keeping(runners: ["a", "b"], answering: ["a"])
        #expect(publication.merged(at: now).complete == false)

        // Heard from again, it vouches for its agents again.
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "g")]))
        #expect(publication.merged(at: now).complete)
    }

    // MARK: - Lost, as opposed to not heard from (ov-50)

    /// **A runner that was answering and stopped is named as lost**, and the
    /// hedge says so rather than "from notifications", which is the wrong
    /// reason: the phone did hear from it. Heard from again, it isn't lost.
    ///
    /// Mutation: `merged` writing no `lostRunners`. Red.
    @Test func aLostRunnerIsNamedAsLost() {
        var publication = FleetPublication()
        publication.keeping(runners: ["a", "b"])
        publication.record(
            runner: "a", snapshot: snapshot([agent("t1", machine: "l")]), named: "Studio")
        publication.record(
            runner: "b", snapshot: snapshot([agent("t2", machine: "g")]), named: "Orchard")
        publication.keeping(runners: ["a", "b"], answering: ["a"])

        let merged = publication.merged(at: now)
        #expect(merged.lostRunners == ["Orchard"])
        #expect(merged.hedge == .lostTouch(["Orchard"]))

        publication.record(
            runner: "b", snapshot: snapshot([agent("t2", machine: "g")]), named: "Orchard")
        #expect(publication.merged(at: now).lostRunners == nil)
        #expect(publication.merged(at: now).hedge == nil)
    }

    /// **A runner nobody has heard from yet is not lost.** The fleet is
    /// incomplete, and the hedge is still "from notifications".
    ///
    /// Mutation: `merged` naming every contribution, lost or not, and a
    /// `hedge` that reads `!complete` as lost. Red under each.
    @Test func aRunnerNotYetHeardFromIsNotLost() {
        var publication = FleetPublication()
        publication.keeping(runners: ["a", "b"], answering: ["a"])
        publication.record(
            runner: "a", snapshot: snapshot([agent("t1", machine: "l")]), named: "Studio")

        let merged = publication.merged(at: now)
        #expect(merged.complete == false)
        #expect(merged.lostRunners == nil)
        #expect(merged.hedge == .fromNotifications)
    }

    /// A lost runner with no agents leaves no rows marked "can't say", and is
    /// still named. The reason this is a list of names and not read back off
    /// `runnerAnswering`.
    @Test func aLostRunnerWithNoAgentsIsStillNamed() {
        var publication = FleetPublication()
        publication.record(
            runner: "a", snapshot: snapshot([agent("t1", machine: "l")]), named: "Studio")
        publication.record(runner: "b", snapshot: snapshot([]), named: "Orchard")
        publication.keeping(runners: ["a", "b"], answering: ["a"])

        #expect(publication.merged(at: now).hedge == .lostTouch(["Orchard"]))
    }

    @Test func everyLiveRunnerHeardFromIsComplete() {
        var publication = FleetPublication()
        publication.keeping(runners: ["a", "b"])
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.record(runner: "b", snapshot: snapshot([agent("t2", machine: "g")]))

        #expect(publication.merged(at: now).complete)
    }

    /// One runner's own snapshot being incomplete makes the merge incomplete.
    /// The claim is about the whole fleet, so the weakest contributor decides.
    @Test func oneIncompleteContributionMakesTheMergeIncomplete() {
        var publication = FleetPublication()
        publication.keeping(runners: ["a", "b"])
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))
        publication.record(
            runner: "b", snapshot: snapshot([agent("t2", machine: "g")], complete: false))

        #expect(publication.merged(at: now).complete == false)
    }

    /// Nothing recorded is not complete, which is what `FleetSnapshot.empty`
    /// says for the same reason.
    @Test func nothingRecordedIsNotComplete() {
        #expect(FleetPublication().merged(at: now).complete == false)
    }

    // MARK: - reviewsWaiting, where nil is not zero

    @Test func reviewCountsAddUpAcrossRunners() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([], reviews: 2))
        publication.record(runner: "b", snapshot: snapshot([], reviews: 3))

        #expect(publication.merged(at: now).reviewsWaiting == 5)
    }

    /// **Nil is "not told" and must never be rendered as zero** —
    /// `FleetSnapshot.reviewsWaiting` states the rule. A daemon too old to
    /// answer `changes.inbox` refuses it on every poll forever, and folding
    /// that in as a 0 would be this app inventing a number no host gave it.
    /// What it must ALSO not do is discard the runner that did answer.
    @Test func aRunnerThatWasNotToldDoesNotZeroTheCount() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([], reviews: 3))
        publication.record(runner: "b", snapshot: snapshot([], reviews: nil))

        #expect(publication.merged(at: now).reviewsWaiting == 3)
    }

    /// Nobody told is still nobody told.
    @Test func noRunnerToldIsStillNotTold() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([], reviews: nil))

        #expect(publication.merged(at: now).reviewsWaiting == nil)
    }

    // MARK: - capturedAt

    /// The assembly moment, which is what the "as of" footer reports. Not any
    /// agent's own age: those carry `observedAt`, so reassembling because one
    /// runner answered does not re-date the others' rows.
    @Test func capturedAtIsTheAssemblyMomentAndNotAnAgentsAge() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: snapshot([agent("t1", machine: "l")]))

        let later = now.addingTimeInterval(600)
        let merged = publication.merged(at: later)
        #expect(merged.capturedAt == later)
        #expect(merged.agents[0].observedAt == nil)
    }
}

/// Summing several runners' fleet traces onto one axis.
///
/// The daemon picks one width across every ring it holds, and it holds ONE
/// runner's. No daemon holds three runners' rings, so for a phone with three
/// connections the choice is between this and drawing nothing.
struct FleetTraceSumTests {
    /// 66 bytes: a version/span header, thirteen `u16` code, thirteen `u16`
    /// output, thirteen `u8` commits. Built here rather than fetched, so a
    /// change to the layout fails loudly instead of decoding to something
    /// plausible.
    private func trace(span: ActivityTrace.Span, code: [UInt16], commits: [UInt8]) -> ActivityTrace
    {
        var bytes = Data()
        bytes.append((1 << 4) | span.rawValue)
        for series in [code, [UInt16](repeating: 0, count: 13)] {
            for value in series {
                bytes.append(UInt8(value & 0xFF))
                bytes.append(UInt8(value >> 8))
            }
        }
        for value in commits { bytes.append(value) }
        return ActivityTrace(bytes)!
    }

    private var zeros: [UInt16] { [UInt16](repeating: 0, count: 13) }
    private var noCommits: [UInt8] { [UInt8](repeating: 0, count: 13) }

    @Test func nothingSumsToNothing() {
        #expect(ActivityTrace.summing([]) == nil)
    }

    /// One runner is byte-for-byte what the daemon sent. That is the
    /// single-runner case, which is every phone until somebody adds a second
    /// machine, and it must not be rewritten by arithmetic it does not need.
    @Test func oneTraceIsReturnedUntouched() {
        var code = zeros
        code[12] = 40
        let one = trace(span: .hour, code: code, commits: noCommits)

        #expect(ActivityTrace.summing([one])?.encoded == one.encoded)
    }

    /// Two runners on the same axis add, column by column.
    @Test func twoRunnersOnOneAxisAdd() {
        var a = zeros
        a[12] = 40
        var b = zeros
        b[12] = 2
        var commits = noCommits
        commits[12] = 3

        let summed = ActivityTrace.summing([
            trace(span: .hour, code: a, commits: commits),
            trace(span: .hour, code: b, commits: commits),
        ])

        #expect(summed?.code(12) == 42)
        #expect(summed?.commits(12) == 6)
        #expect(summed?.span == .hour)
    }

    /// The COARSEST span wins, because it is the only one every input can be
    /// summed onto: `rebucketed` refuses to go finer and hands its input back
    /// unchanged, which would leave two different windows on one axis.
    @Test func theCoarsestSpanIsTheAxis() {
        let fine = trace(span: .hour, code: zeros, commits: noCommits)
        let coarse = trace(span: .day, code: zeros, commits: noCommits)

        #expect(ActivityTrace.summing([fine, coarse])?.span == .day)
        #expect(ActivityTrace.summing([coarse, fine])?.span == .day)
    }

    /// A fine trace brought onto a coarse axis keeps its total. Nothing is
    /// interpolated, spread or invented — resolution is the whole cost, which
    /// is what `rebucketed` says about itself.
    @Test func summingOntoACoarserAxisKeepsTheTotal() {
        var fine = zeros
        for bucket in 0..<13 { fine[bucket] = 3 }
        let summed = ActivityTrace.summing([
            trace(span: .hour, code: fine, commits: noCommits),
            trace(span: .day, code: zeros, commits: noCommits),
        ])

        let total = (0..<13).reduce(0) { $0 + Int(summed!.code($1)) }
        #expect(total == 39)
    }

    /// The producer saturates on the way to the wire, and summing three runners
    /// can reach a ceiling one could not. It has to saturate at the encode and
    /// must not wrap in the accumulator — which is what `UInt16` arithmetic
    /// would have done.
    @Test func aSumPastTheCeilingSaturatesRatherThanWrapping() {
        var big = zeros
        big[12] = 60_000
        let summed = ActivityTrace.summing([
            trace(span: .hour, code: big, commits: noCommits),
            trace(span: .hour, code: big, commits: noCommits),
        ])

        #expect(summed?.code(12) == UInt16.max)
    }

    // MARK: - Anchored: several runners placed on one grid

    private var powers: [UInt16] { [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096] }
    private var fives: [UInt16] { [UInt16](repeating: 5, count: 13) }

    /// Two runners' fleet traces, each placed by its anchor. The five-minute
    /// one is anchored at 6002 — the third bucket of half hour 1000 — and the
    /// thirty-minute one at 998, two half hours behind it.
    ///
    /// By hand: the five-minute runner lands 1+2+4+8 in column 10, 16…512 in 11
    /// and 1024+2048+4096 in 12; the other lands its thirteen fives two columns
    /// early, in columns 0…10, its two oldest off the left. Packing both from
    /// the newest end would have put 8064 and a five in column 12.
    @Test func anchoredRunnersArePlacedNotPacked() throws {
        let summed = try #require(
            ActivityTrace.summing(anchored: [
                (trace(span: .hour, code: powers, commits: noCommits), 6002),
                (trace(span: .sixHours, code: fives, commits: noCommits), 998),
            ]))
        #expect(summed.trace.span == .sixHours)
        #expect(summed.trace.code(12) == 7168)
        #expect(summed.trace.code(11) == 1008)
        #expect(summed.trace.code(10) == 15 + 5)
        for column in 0...9 { #expect(summed.trace.code(column) == 5, "column \(column)") }
        // Every input was anchored, so the sum is, at the newest column.
        #expect(summed.anchor == 1000)
    }

    /// One runner too old to anchor its trace: that one is packed as before,
    /// and the sum carries no anchor, since not every bucket in it is placed.
    @Test func aSumWithAnUnanchoredRunnerHasNoAnchor() throws {
        let summed = try #require(
            ActivityTrace.summing(anchored: [
                (trace(span: .hour, code: powers, commits: noCommits), 6002),
                (trace(span: .sixHours, code: fives, commits: noCommits), nil),
            ]))
        #expect(summed.trace.code(12) == 7168 + 5)
        #expect(summed.anchor == nil)
    }

    /// An anchor outside `0...anchorLimit` is no anchor, whoever passes it: it
    /// neither sets the axis — which would push the honest runner out of the
    /// window and sum to thirteen measured zeroes — nor anchors the result.
    @Test func anOutOfRangeAnchorIsIgnoredBySumming() throws {
        let summed = try #require(
            ActivityTrace.summing(anchored: [
                (trace(span: .hour, code: powers, commits: noCommits), 6002),
                (trace(span: .sixHours, code: fives, commits: noCommits), Int.max),
            ]))
        #expect(summed.trace.code(12) == 7168 + 5)
        #expect(summed.anchor == nil)
    }

    /// End to end through the publication: each runner's anchor is checked
    /// against the poll that brought it, so one whose clock runs a day ahead is
    /// packed rather than allowed to set the axis — which would have pushed the
    /// other runner's history off the Island entirely.
    @Test func thePublicationTrustsOnlyAnchorsItsPollsCouldVouchFor() {
        // `now` is Unix second 1,000,000: five-minute bucket 3333, half hour 555.
        // 3332 is the third five-minute bucket of half hour 555, like 6002 above.
        func runner(_ trace: ActivityTrace, anchor: Int) -> FleetSnapshot {
            FleetSnapshot(
                agents: [], capturedAt: Date(timeIntervalSince1970: 1_000_000), complete: true,
                fleetTrace: trace.encoded, fleetTraceAnchor: anchor)
        }
        var honest = FleetPublication()
        honest.record(runner: "a", snapshot: runner(trace(span: .hour, code: powers, commits: noCommits), anchor: 3332))
        honest.record(runner: "b", snapshot: runner(trace(span: .sixHours, code: fives, commits: noCommits), anchor: 553))
        let merged = honest.merged(at: Date(timeIntervalSince1970: 1_000_000))
        let placed = ActivityTrace(merged.fleetTrace)
        #expect(placed?.code(12) == 7168)
        #expect(placed?.code(10) == 20)
        #expect(merged.fleetTraceAnchor == 555)

        var skewed = FleetPublication()
        skewed.record(runner: "a", snapshot: runner(trace(span: .hour, code: powers, commits: noCommits), anchor: 3332))
        // A day ahead of the poll that brought it.
        skewed.record(runner: "b", snapshot: runner(trace(span: .sixHours, code: fives, commits: noCommits), anchor: 555 + 48))
        let kept = skewed.merged(at: Date(timeIntervalSince1970: 1_000_000))
        let fleet = ActivityTrace(kept.fleetTrace)
        // 7168 placed, plus the fast runner's five packed into the newest column.
        #expect(fleet?.code(12) == 7173, "the fast runner's anchor set the axis")
        #expect(kept.fleetTraceAnchor == nil)
    }
}

/// **The writer acts on a membership only when it moved** (ov-22 M8).
/// `FleetStore.publish` runs on every change any connection publishes, several
/// times a second while an agent produces, and each write reloads every widget
/// timeline and crosses to the watch: unguarded, it wedged the main thread.
/// The guard lived in the app, where nothing could test it.
///
/// Mutation: `moved` always true. Red.
@Test func theWriterActsOnAMembershipOnlyWhenItMoves() {
    var kept = KeptMembership()
    let first = kept.moved(runners: ["a"], answering: ["a"])
    #expect(first, "the first is always news")
    let same = kept.moved(runners: ["a"], answering: ["a"])
    #expect(!same, "the same again is not")
    let dropped = kept.moved(runners: ["a"], answering: [])
    #expect(dropped, "a link going is")
    let still = kept.moved(runners: ["a"], answering: [])
    #expect(!still)
    let added = kept.moved(runners: ["a", "b"], answering: [])
    #expect(added, "a runner added is")
}

/// The Needs You lists in the merge (ov-55 4C.1).
struct FleetPublicationNeedsYouTests {
    private let now = Date(timeIntervalSince1970: 1_000_000)

    private func item(_ id: String, rank: UInt32) -> NeedsYouItem {
        NeedsYouItem(id: id, kind: .decision, rank: rank, since: nil, question: "?")
    }

    private var fleet: FleetSnapshot { FleetSnapshot(agents: [], capturedAt: now, complete: true) }

    /// Nothing handed over is nil, so a surface goes on counting agents.
    @Test func noListsIsNoList() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: fleet)
        #expect(publication.merged(at: now).needsYou == nil)
    }

    /// Every runner's list, merged by rank, each item carrying its runner.
    ///
    /// Mutation: `merged` concatenating the lists in dictionary order. Red:
    /// the order is the hash's, not the rank's.
    @Test func theListsMergeByRank() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: fleet)
        publication.record(runner: "b", snapshot: fleet)
        publication.record(needsYou: [
            "a": [item("decision:1", rank: 30), item("decision:2", rank: 50)],
            "b": [item("decision:3", rank: 40)],
        ])
        let merged = publication.merged(at: now).needsYou
        #expect(merged?.map(\.itemID) == ["decision:1", "decision:3", "decision:2"])
        #expect(merged?.map(\.runner) == ["a", "b", "a"])
    }

    /// A retired runner's items go with its agents.
    ///
    /// Mutation: `merged` without the `live` filter. Red: two items.
    @Test func aRetiredRunnersItemsGo() {
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: fleet)
        publication.record(runner: "b", snapshot: fleet)
        publication.record(needsYou: ["a": [item("decision:1", rank: 30)], "b": [item("decision:3", rank: 40)]])
        publication.keeping(runners: ["a"])
        #expect(publication.merged(at: now).needsYou?.map(\.itemID) == ["decision:1"])
    }

    /// The same lists again are no news, so the writer doesn't wake every
    /// widget on each of the store's publishes.
    ///
    /// Mutation: `record(needsYou:)` returning true always. Red.
    @Test func theSameListsAgainAreNoNews() {
        var publication = FleetPublication()
        let lists = ["a": [item("decision:1", rank: 30)]]
        let first = publication.record(needsYou: lists)
        let again = publication.record(needsYou: lists)
        let emptied = publication.record(needsYou: [:])
        #expect(first)
        #expect(!again)
        #expect(emptied)
    }
}
