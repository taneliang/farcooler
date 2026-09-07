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
}
