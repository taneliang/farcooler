import Foundation
import Testing

@testable import AgentKit

/// The lock screen card's rows: what the relay sends, and what the card makes
/// of it.
///
/// **Every string the card draws is composed in `AgentCardLayout` so that this
/// file can read it back.** A sentence built inside a `View.body` is checked by
/// nothing here — the iOS UI suite is compiled in CI and never executed — and
/// this repo has shipped a run of guards that could not fail. So the split is
/// the point: the widget positions views, and every figure and every word it
/// positions comes from a function with a test on it.
///
/// The JSON below is written out rather than round-tripped through the encoder.
/// A fixture minted by the code it checks agrees with any value at all,
/// including the wrong one. `services/relay/test/relay.test.ts` asserts the
/// other end of the same payloads.
private func decode(_ json: String) throws -> AgentCardState {
    try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
}

/// Exactly what `withFleet` composes in `services/relay/src/index.ts`: a
/// headline, three tier counts, `more`, the fleet's totals, and four rows.
private let trace = ActivityTraceTests.encoded(
    code: [0, 0, 0, 0, 0, 8, 2, 900, 40, 7, 0, 0, 0],
    output: [0, 0, 0, 0, 0, 4, 1, 3, 60, 5, 0, 0, 0],
    commits: [0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 0, 0, 0]
).base64EncodedString()

private func fleetPush(now: Double = 1_755_000_000_000) -> String {
    """
    {"terminal":"term-1","label":"auth-refactor","machine":"studio",
     "status":"blocked","detail":"Force-push to origin/main?",
     "startedAt":\(now - 600_000),
     "blocked":2,"review":3,"working":3,"more":6,
     "insertions":391,"deletions":112,"commits":41,
     "rows":[
       {"terminal":"term-1","label":"auth-refactor","machine":"studio",
        "status":"blocked","detail":"Force-push to origin/main?",
        "insertions":142,"deletions":37,"commits":4,
        "startedAt":\(now - 600_000),"updatedAt":\(now - 20_000),"trace":"\(trace)"},
       {"terminal":"term-2","label":"schema-migrate","machine":"gpu-box",
        "status":"working","detail":"unreachable",
        "insertions":18,"deletions":4,
        "startedAt":\(now - 900_000),"updatedAt":\(now - 720_000)},
       {"terminal":"term-3","label":"docs-sweep","machine":"studio",
        "status":"working","detail":"Reading the manual","updatedAt":\(now - 5_000)},
       {"terminal":"term-4","label":"lint-pass","machine":"studio",
        "status":"done","detail":"","updatedAt":\(now - 5_000)}
     ]}
    """
}

private let cardNow = Date(timeIntervalSince1970: 1_755_000_000)

// MARK: - The wire

@Test func aPushedRowCarriesEveryFieldTheRelaySends() throws {
    let state = try decode(fleetPush())

    #expect(state.rows.count == 4)
    #expect(state.more == 6)
    let first = try #require(state.rows.first)
    #expect(first.terminal == "term-1")
    #expect(first.label == "auth-refactor")
    #expect(first.machine == "studio")
    #expect(first.status == "blocked")
    #expect(first.detail == "Force-push to origin/main?")
    #expect(first.insertions == 142)
    #expect(first.deletions == 37)
    #expect(first.commits == 4)
    // Unix MILLISECONDS on the wire, like the headline's own clock and unlike
    // every other timestamp in `push.ts`. Left to Swift's default this decodes
    // to a date in the year 57000 and the card's arithmetic about "as of" is
    // nonsense that still renders.
    #expect(first.updatedAt == cardNow.addingTimeInterval(-20))
    #expect(first.startedAt == cardNow.addingTimeInterval(-600))
    // The thirteen buckets, base64 of the wire's 66 bytes — the same encoding
    // `FleetSnapshot.trace` carries, so `ActivityTrace` reads it unchanged.
    #expect(first.trace?.count == ActivityTrace.encodedLength)
    let read = try #require(ActivityTrace(first.trace))
    #expect(read.code(7) == 900)
    #expect(read.output(8) == 60)
    #expect(read.commits(9) == 2)
}

@Test func anAbsentMeasurementStaysAbsentRatherThanBecomingZero() throws {
    let state = try decode(fleetPush())
    let second = try #require(state.rows.dropFirst().first)
    // The runner measured lines and no commits. Zero commits and no commits are
    // different facts — see `AgentCardRow.commits` — and only one of them
    // entitles a row to print a figure.
    #expect(second.insertions == 18)
    #expect(second.commits == nil)
    let third = try #require(state.rows.dropFirst(2).first)
    #expect(third.insertions == nil)
    #expect(third.deletions == nil)
    // No trace at all, which is not thirteen quiet buckets: a terminal with
    // nothing to show sends no field, and `ActivityTrace` refuses to build one.
    #expect(third.trace == nil)
    #expect(ActivityTrace(third.trace) == nil)
}

@Test func aTraceThisBuildCannotReadCostsTheTraceAndNotTheRow() throws {
    // Two separate failures, both of which must leave the row standing: a field
    // that is not base64 at all, and base64 of the wrong number of bytes. The
    // card is worth more than the drawing on one line of it.
    let state = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":1,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"working","detail":"","trace":"not base64!!"},
                 {"terminal":"b","label":"b","status":"working","detail":"","trace":"YWJj"}]}
        """)
    #expect(state.rows.count == 2)
    #expect(state.rows[0].trace == nil)
    #expect(ActivityTrace(state.rows[1].trace) == nil, "three bytes is not a trace")
}

@Test func aRowFromAnOlderRelayDecodesToDefaultsRatherThanThrowing() throws {
    // The upgrade case, and it is the one that cannot be allowed to throw: a
    // state that fails to decode keeps the whole activity out of
    // `Activity.activities`, where nothing can end it.
    let state = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":1,"more":0,
         "rows":[{"terminal":"a"}]}
        """)
    let row = try #require(state.rows.first)
    #expect(row.label.isEmpty)
    #expect(row.status.isEmpty)
    #expect(row.updatedAt == nil)
    #expect(row.age(at: cardNow) == 0, "nothing heard is not an eternity of silence")
}

@Test func aRowsTimestampIsReadInEitherUnitTheSendersUse() throws {
    // Seconds and milliseconds meet on this seam — `startedAt` comes off the
    // daemon's turn clock in milliseconds and `updatedAt` off the relay, and
    // both are accepted and told apart by magnitude.
    let state = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":1,"more":0,
         "rows":[{"terminal":"a","startedAt":1755000000,"updatedAt":1755000000000}]}
        """)
    let row = try #require(state.rows.first)
    #expect(row.startedAt == cardNow)
    #expect(row.updatedAt == cardNow)
}

@Test func aCardWithNoRowsCarriesNoRowsKeyThroughThePersistedRoundTrip() throws {
    // ActivityKit persists a state by encoding it and reads it back by
    // decoding it, and `AgentCardLayout.init?` reads `rows.isEmpty` to tell a
    // card that carries rows from one that never did. A round trip that
    // invented an empty array, or dropped a real one, would change which card
    // an upgraded phone draws.
    let plain = AgentCardState(status: "working", detail: "Running tests")
    let encoded = try JSONEncoder().encode(plain)
    #expect(!String(decoding: encoded, as: UTF8.self).contains("\"rows\""))
    #expect(!String(decoding: encoded, as: UTF8.self).contains("\"more\""))
    #expect(try JSONDecoder().decode(AgentCardState.self, from: encoded).rows.isEmpty)

    let withRows = try decode(fleetPush())
    let again = try JSONDecoder().decode(
        AgentCardState.self, from: JSONEncoder().encode(withRows))
    #expect(again.rows == withRows.rows)
    #expect(again.more == 6)
}

// MARK: - The card

@Test func theCardDrawsTwoRowsAndCountsEverybodyElse() throws {
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))

    // Two, and the wire's ceiling is a different number: `ROWS_SHOWN` is four
    // and the byte arithmetic beside it allows eight. This limit is the card's
    // height.
    #expect(AgentCardLayout.rowsDrawn == 2)
    #expect(layout.rows.map(\.name) == ["auth-refactor", "schema-migrate"])
    // Six the relay could not fit, plus the two it sent that this card has no
    // room for. Either number alone is a lie: `more` under a card drawing two
    // of four rows undercounts by two.
    #expect(layout.hidden == 8)
    #expect(layout.line == "+8 more · +391 −112")
}

@Test func theHeaderIsTheFleetInTheOrderItIsUrgentIn() throws {
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))
    #expect(layout.title == "2 need you")
    #expect(layout.counts == "3 to review · 3 in flight")
    #expect(layout.mark == GlanceMark(attention: .needsYou, core: .atAPrompt))
}

/// **A card the relay has stopped vouching for says nothing about now.**
/// ActivityKit's `isStale`: an hour since the last push, which is what a runner
/// that stays down looks like from the lock screen. "In flight" leaves the
/// header and every working ring and row goes to "can't say"; the three that
/// need you and the three to review hold, as they do at any age.
///
/// Mutation: the header's working clause without `&& !stale`. Red: the counts
/// read `3 to review · 3 in flight`.
@Test func aStaleCardStopsClaimingAnythingIsInFlight() throws {
    let fresh = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))
    let stale = try #require(
        AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: true))

    #expect(stale.title == "2 need you")
    #expect(stale.counts == "3 to review")
    #expect(stale.mark == fresh.mark, "the header is about the blocked tier, which holds")
    // Two blocked, three to review, three working: the working three go dashed
    // and nothing else moves.
    #expect(stale.rings.prefix(5) == fresh.rings.prefix(5))
    #expect(stale.rings.suffix(3) == [.unsaid, .unsaid, .unsaid])
    #expect(fresh.rings.suffix(3).allSatisfy { $0.link == .live })
    // The rows: the blocked one holds, the working one (heard from twelve
    // minutes ago, well inside the hour) is dashed only because the card is.
    #expect(stale.rows[0].mark == fresh.rows[0].mark)
    #expect(fresh.rows[1].mark.link == .live)
    #expect(stale.rows[1].mark.link == .broken)
    #expect(stale.line == fresh.line, "who has no line, and the totals, are history")
}

/// A fleet that is only working, on a stale card, falls back to the title a
/// fleet with nothing to report already has, under a "can't say" ring.
@Test func aStaleCardOfOnlyWorkingAgentsHasNothingToHeadline() throws {
    let working = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":4,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: working, now: cardNow, stale: true))
    #expect(layout.title == "Your agents")
    #expect(layout.counts == nil)
    #expect(layout.mark == .unsaid)
    #expect(layout.rows[0].mark.link == .broken)
}

@Test func anEmptyTierIsDroppedRatherThanWrittenAsZero() throws {
    // "0 need you" is worse than silence on a lock screen, and a header that
    // led with it would spend the card's loudest line on nothing.
    let quiet = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":4,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: quiet, now: cardNow, stale: false))
    #expect(layout.title == "4 in flight")
    #expect(layout.counts == nil)
    #expect(layout.mark == GlanceMark(attention: .quiet, core: .producing))

    let one = try decode(
        """
        {"status":"blocked","detail":"","blocked":1,"review":0,"working":0,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"blocked","detail":""}]}
        """)
    #expect(try #require(AgentCardLayout(state: one, now: cardNow, stale: false)).title == "1 needs you")
}

@Test func aFleetWithNothingInAnyTierStillHasATitle() throws {
    // The card is on screen either way and a blank header reads as a card that
    // failed to load. The relay's own `fleetHeader` falls back to the same two
    // words.
    let idle = try decode(
        """
        {"status":"done","detail":"","blocked":0,"review":0,"working":0,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"done","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: idle, now: cardNow, stale: false))
    #expect(layout.title == "Your agents")
    #expect(layout.counts == nil)
    #expect(layout.line == nil, "nothing hidden and nothing measured is nothing to say")
}

@Test func aRelayTooOldToSendRowsGetsTheCardItAlwaysGot() throws {
    // The compatibility path, and the reason `init?` is failable rather than
    // clamping: an app updated ahead of its relay reads no `rows` key at all,
    // and a card that drew an empty body would be worse than the headline it
    // used to draw.
    let old = try decode(
        """
        {"terminal":"t","label":"claude","machine":"studio",
         "status":"working","detail":"Running tests","blocked":0,"review":0,"working":1}
        """)
    #expect(old.rows.isEmpty)
    #expect(old.more == -1, "an absent count is not zero")
    #expect(AgentCardLayout(state: old, now: cardNow, stale: false) == nil)

    // And rows without counts, which nothing sends but which must not put a
    // header on the card claiming a fleet of minus one.
    let countless = try decode(
        """
        {"status":"working","detail":"",
         "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
        """)
    #expect(!countless.knowsFleet)
    #expect(AgentCardLayout(state: countless, now: cardNow, stale: false) == nil)
}

// MARK: - The figures on a row

@Test func aRowPrintsBothHalvesOfADiffOrNeither() {
    // `+142 −0` over a deletion nobody counted is a figure the card made up.
    #expect(AgentCardLayout.diff(insertions: 142, deletions: 37) == "+142 −37")
    #expect(AgentCardLayout.diff(insertions: 142, deletions: nil) == nil)
    #expect(AgentCardLayout.diff(insertions: nil, deletions: 37) == nil)
    #expect(AgentCardLayout.diff(insertions: 0, deletions: 0) == "+0 −0", "measured nothing")
    // U+2212, the character every other diff count in this product uses. A
    // hyphen here and a minus in the app is two shapes for one number.
    #expect(AgentCardLayout.diff(insertions: 1, deletions: 2)?.contains("\u{2212}") == true)
}

@Test func theSecondFigureIsCommitsWhenThereAreAnyAndAnAgeWhenThereAreNot() {
    func row(commits: Int? = nil, spokeAgo: TimeInterval?) -> AgentCardRow {
        AgentCardRow(
            terminal: "t", status: "working", commits: commits,
            updatedAt: spokeAgo.map { cardNow.addingTimeInterval(-$0) })
    }
    #expect(AgentCardLayout.footnote(row(commits: 4, spokeAgo: 20), at: cardNow) == "4 commits")
    #expect(AgentCardLayout.footnote(row(commits: 1, spokeAgo: 20), at: cardNow) == "1 commit")
    // Twelve minutes, which is the design's own example. `GlanceAge.fresh` is
    // the threshold and it is quoted rather than chosen: two minutes is where
    // this product's surfaces start saying how old a thing is.
    #expect(AgentCardLayout.footnote(row(spokeAgo: 720), at: cardNow) == "as of 12m")
    #expect(AgentCardLayout.footnote(row(spokeAgo: GlanceAge.fresh), at: cardNow) == "as of 2m")
    #expect(
        AgentCardLayout.footnote(row(spokeAgo: GlanceAge.fresh - 1), at: cardNow) == nil,
        "under two minutes a row says nothing, because `as of 0m` is noise")
    // Zero commits is a measurement of nothing, so the slot goes to the age.
    #expect(AgentCardLayout.footnote(row(commits: 0, spokeAgo: 720), at: cardNow) == "as of 12m")
    #expect(AgentCardLayout.footnote(row(spokeAgo: nil), at: cardNow) == nil)
}

@Test func aNamelessRowIsStillNamed() {
    // A card started by an older build carries neither name nor runner, and a
    // blank where a name goes is the one thing worse than an ugly name.
    #expect(AgentCardLayout.name(of: AgentCardRow(terminal: "t", label: "a")) == "a")
    #expect(AgentCardLayout.name(of: AgentCardRow(terminal: "t", machine: "studio")) == "studio")
    #expect(AgentCardLayout.name(of: AgentCardRow(terminal: "t")) == "t")
}

// MARK: - What the marks say

@Test func stateLivesInTheRingAndDecaysOnTheOneRuleTheProductHas() throws {
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))
    // Blocked is latched: an agent stopped twenty seconds ago and one stopped
    // an hour ago are both stopped, so the ring stays solid.
    #expect(layout.rows[0].mark == GlanceMark(attention: .needsYou, core: .atAPrompt))
    // Working is a claim about right now, and this row last spoke twelve
    // minutes ago — inside the hour, so it is still asserted.
    #expect(layout.rows[1].mark.link == .live)

    // The same fleet an hour later. Only the working claim is withdrawn.
    let aged = try #require(
        AgentCardLayout(
            state: try decode(fleetPush()),
            now: cardNow.addingTimeInterval(FleetSnapshot.staleAfter), stale: false))
    #expect(aged.rows[0].mark.link == .live, "blocked holds at any age")
    #expect(aged.rows[1].mark.link == .broken, "working does not")
    // And the rule is the snapshot's own, not a second copy of it.
    #expect(
        FleetSnapshot.confidence(status: "working", heard: FleetSnapshot.staleAfter, answering: true)
            == .lastSeen)
    #expect(FleetSnapshot.confidence(status: "blocked", heard: 86400, answering: true) == .known)
}

@Test func theTailDrawsARingPerAgentInTierOrder() throws {
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))
    // Two blocked, three to review, three in flight — the header's own three
    // numbers, said again as marks.
    #expect(layout.rings.count == 8)
    #expect(layout.rings.prefix(2).allSatisfy { $0.attention == .needsYou })
    #expect(layout.rings.dropFirst(2).prefix(3).allSatisfy { $0.attention == .toReview })
    #expect(layout.rings.dropFirst(5).allSatisfy { $0.attention == .quiet })
    // Never a core and never a dash. The counts say how many agents are in a
    // tier and nothing about any one of them, so the ring declines to state the
    // agent's own axis rather than guessing at it.
    #expect(layout.rings.allSatisfy { $0.core == nil && $0.link == .live })
}

@Test func theRingsStopBeforeTheyCrowdOutTheLineBesideThem() throws {
    let big = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":40,"more":39,
         "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: big, now: cardNow, stale: false))
    #expect(layout.rings.count == AgentCardLayout.ringsDrawn)
    #expect(AgentCardLayout.ringsDrawn == 12)
    // The count itself is never truncated — it is in the header and in the
    // tail, both of which count every agent.
    #expect(layout.title == "40 in flight")
    #expect(layout.hidden == 39)
}

@Test func theTailSaysWhicheverOfItsTwoThingsItKnows() throws {
    func line(more: Int, totals: Bool) throws -> String? {
        let json = """
            {"status":"working","detail":"","blocked":0,"review":0,"working":2,"more":\(more),
             \(totals ? "\"insertions\":391,\"deletions\":112," : "")
             "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
            """
        return try #require(AgentCardLayout(state: try decode(json), now: cardNow, stale: false)).line
    }
    #expect(try line(more: 6, totals: true) == "+6 more · +391 −112")
    #expect(try line(more: 6, totals: false) == "+6 more")
    #expect(try line(more: 0, totals: true) == "+391 −112")
    #expect(try line(more: 0, totals: false) == nil)
}

// MARK: - The headline presentations: the Island, and the old-relay card

/// **A stale card's tail hedges its verb.** The expanded Island counts the
/// relay's working agents into "+N more working"; an hour after the last push
/// that count is who the relay last knew about, not who is working. Qualified,
/// the line says "last seen working" and draws at 60%, as the snapshot's hedge
/// always has.
///
/// Mutation: the relay branch's `qualified: stale` back to `false`. Red.
@Test func aStaleTailSaysLastSeenWorking() throws {
    let working = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":4,"more":0}
        """)
    let fresh = FleetTail.current(for: working, snapshot: nil, now: cardNow, stale: false)
    #expect(fresh.others == 3)
    #expect(!fresh.qualified)
    #expect(fresh.line == "+3 more working")

    let stale = FleetTail.current(for: working, snapshot: nil, now: cardNow, stale: true)
    #expect(stale.others == 3, "the count is who the relay last knew about")
    #expect(stale.qualified)
    #expect(stale.line == "+3 more last seen working")
}

/// The same for a card from a relay too old to count, which falls back to the
/// App Group snapshot: a stale card hedges even a fresh snapshot.
///
/// Mutation: the snapshot branch's `stale ||` dropped. Red.
@Test func aStaleOldRelayCardHedgesItsSnapshotTail() {
    let state = AgentCardState(terminal: "lead", status: "working", detail: "")
    func agent(_ id: String) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: id, machine: "studio", status: "working", glyph: "", headline: "",
            line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil,
            observedAt: cardNow)
    }
    let snapshot = FleetSnapshot(
        agents: [agent("lead"), agent("a"), agent("b")], capturedAt: cardNow, complete: true)

    let fresh = FleetTail.current(for: state, snapshot: snapshot, now: cardNow, stale: false)
    #expect(fresh.line == "+2 more working")
    let stale = FleetTail.current(for: state, snapshot: snapshot, now: cardNow, stale: true)
    #expect(stale.line == "+2 more last seen working")
}

/// The leader's word and clock go on a stale card only for a working leader;
/// needs you and finished hold. An unrecognized word reads as working, as
/// `AgentStatus` folds it.
///
/// Mutation: `isStated` returning `!stale`. Red: blocked loses its word.
@Test func aStaleCardStopsStatingOnlyAWorkingLeader() {
    #expect(AgentCardLeader.isStated(status: "working", stale: false))
    #expect(!AgentCardLeader.isStated(status: "working", stale: true))
    #expect(!AgentCardLeader.isStated(status: "something-newer", stale: true))
    #expect(AgentCardLeader.isStated(status: "blocked", stale: true))
    #expect(AgentCardLeader.isStated(status: "done", stale: true))
}

/// The compact Island's "+N" dims only on a stale card with a count beside the
/// leader. A card that isn't stale is unchanged even when the old-relay
/// snapshot hedges its line, and a stale leader alone (a blocked one, say)
/// keeps full strength.
///
/// Mutation: `dimsCompactCount` returning `qualified`. Red: the hedged
/// snapshot tail on a card that isn't stale dims.
@Test func theCompactCountDimsOnlyOnAStaleCardWithACount() throws {
    let working = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":4,"more":0}
        """)
    let stale = FleetTail.current(for: working, snapshot: nil, now: cardNow, stale: true)
    #expect(stale.dimsCompactCount(stale: true))

    let alone = try decode(
        """
        {"status":"blocked","detail":"","blocked":1,"review":0,"working":0,"more":0}
        """)
    let lone = FleetTail.current(for: alone, snapshot: nil, now: cardNow, stale: true)
    #expect(lone.others == 0)
    #expect(!lone.dimsCompactCount(stale: true), "a blocked leader alone holds")

    // An old relay's card, not stale, over an incomplete snapshot: the line
    // is hedged, and the compact count is not.
    let old = AgentCardState(terminal: "lead", status: "working", detail: "")
    let agent = FleetSnapshot.Agent(
        id: "a", label: "a", machine: "studio", status: "working", glyph: "", headline: "",
        line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil,
        observedAt: cardNow)
    let partial = FleetSnapshot(agents: [agent], capturedAt: cardNow, complete: false)
    let hedged = FleetTail.current(for: old, snapshot: partial, now: cardNow, stale: false)
    #expect(hedged.qualified)
    #expect(!hedged.dimsCompactCount(stale: false))
}

/// **A "needs you" line is never dimmed.** The tail's line dims when it's
/// qualified — a stale card, or the old-relay snapshot hedge — and only when
/// nobody in it is blocked: a blocked count is latched and holds at any age,
/// so it must never look uncertain.
///
/// Mutation: `dimsLine` returning `qualified`. Red on both blocked cases.
@Test func aNeedsYouLineIsNeverDimmed() throws {
    // Stale, from a relay that counts: 2 blocked (one is the leader), 3 working.
    let blocked = try decode(
        """
        {"status":"blocked","detail":"","blocked":2,"review":0,"working":3,"more":0}
        """)
    let staleBlocked = FleetTail.current(for: blocked, snapshot: nil, now: cardNow, stale: true)
    #expect(staleBlocked.line == "+4 more · 1 needs you")
    #expect(!staleBlocked.dimsLine, "stale, with someone blocked: full strength")

    let working = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":4,"more":0}
        """)
    let staleWorking = FleetTail.current(for: working, snapshot: nil, now: cardNow, stale: true)
    #expect(staleWorking.dimsLine, "stale, nobody blocked: dimmed")

    // Hedged, not stale: an old relay's card over an incomplete snapshot.
    let old = AgentCardState(terminal: "lead", status: "working", detail: "")
    func agent(_ id: String, _ status: String) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: id, machine: "studio", status: status, glyph: "", headline: "",
            line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil,
            observedAt: cardNow)
    }
    let withBlocked = FleetSnapshot(
        agents: [agent("a", "blocked"), agent("b", "working")], capturedAt: cardNow,
        complete: false)
    let hedgedBlocked = FleetTail.current(
        for: old, snapshot: withBlocked, now: cardNow, stale: false)
    #expect(hedgedBlocked.qualified)
    #expect(hedgedBlocked.line == "+2 more · 1 needs you")
    #expect(!hedgedBlocked.dimsLine, "hedged, with someone blocked: full strength")

    let withoutBlocked = FleetSnapshot(
        agents: [agent("b", "working")], capturedAt: cardNow, complete: false)
    let hedgedWorking = FleetTail.current(
        for: old, snapshot: withoutBlocked, now: cardNow, stale: false)
    #expect(hedgedWorking.qualified)
    #expect(hedgedWorking.dimsLine, "hedged, nobody blocked: dimmed")
}

// MARK: - Needs You and workspaces (ov-55 4C.3)

/// The header's number is the relay's `needsYou` when it sends one: the item
/// count, decisions included, which is the app's number. Without it, the
/// blocked agents, as every card before this. The rings stay per agent tier.
///
/// Mutation: the title clause counting `state.blocked`. Red: "2 need you",
/// not "5 need you".
@Test("The header counts needsYou when the relay sends it, else blocked")
func theHeaderCountsNeedsYouWhenTheRelaySendsItElseBlocked() throws {
    let counted = try decode(
        """
        {"status":"blocked","detail":"","blocked":2,"review":0,"working":1,"more":0,
         "needsYou":5,
         "rows":[{"terminal":"a","label":"claude","status":"blocked","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: counted, now: cardNow, stale: false))
    #expect(counted.needsYou == 5)
    #expect(layout.title == "5 need you")
    #expect(layout.counts == "1 in flight")

    let older = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))
    #expect(try decode(fleetPush()).needsYou == -1)
    #expect(older.title == "2 need you")
}

/// A decision with no agent blocked is still amber: the header says it needs
/// you, and the mark says what the header says.
///
/// Mutation: `tier` reading `state.blocked`. Red: the working mark.
@Test func aDecisionAloneMakesTheHeaderAmber() throws {
    let decision = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":1,"more":0,
         "needsYou":1,
         "rows":[{"terminal":"a","label":"claude","status":"working","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: decision, now: cardNow, stale: false))
    #expect(layout.title == "1 needs you")
    #expect(layout.mark == GlanceMark(attention: .needsYou, core: .atAPrompt))
}

/// A row leads with its workspace, "Billing · claude", as every surface names
/// an agent now; a row with none is the bare name, never " · claude".
///
/// Mutation: `name(of:)` ignoring `workspace`. Red: "claude".
@Test("A row names its workspace")
func aRowNamesItsWorkspace() throws {
    let state = try decode(
        """
        {"status":"blocked","detail":"","blocked":1,"review":0,"working":1,"more":0,
         "workspace":"Billing",
         "rows":[
           {"terminal":"a","label":"claude","workspace":"Billing","status":"blocked","detail":""},
           {"terminal":"b","label":"codex","status":"working","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: state, now: cardNow, stale: false))
    #expect(layout.rows.map(\.name) == ["Billing · claude", "codex"])
    #expect(state.workspace == "Billing")
    #expect(AgentCardLayout.named(state.label, in: state.workspace) == "")
    #expect(AgentCardLayout.named("claude", in: "Billing") == "Billing · claude")
}

/// The new fields survive the round trip ActivityKit makes on every persisted
/// card, and an absent count stays absent rather than becoming a number.
///
/// Mutation: `encode` writing `needsYou` unconditionally. Red: the persisted
/// state carries a `needsYou` key the relay never sent.
@Test func needsYouAndWorkspaceRoundTripAndAbsenceStaysAbsent() throws {
    let state = AgentCardState(
        label: "claude", workspace: "Billing", status: "blocked", detail: "",
        blocked: 1, review: 0, working: 0, needsYou: 3,
        rows: [AgentCardRow(terminal: "a", label: "claude", workspace: "Billing")])
    let back = try JSONDecoder().decode(AgentCardState.self, from: JSONEncoder().encode(state))
    #expect(back == state)
    let silent = AgentCardState(status: "working", detail: "")
    let json = String(decoding: try JSONEncoder().encode(silent), as: UTF8.self)
    #expect(!json.contains("needsYou"))
    #expect(!json.contains("workspace"))
}

// MARK: - Fix round 1 (ov-55 4C)

/// A relay that counted nothing needs you says 0, even with agents blocked:
/// every runner sent a list and none had an item. Only an absent count falls
/// back to `blocked`.
///
/// Mutation: `headerCount` reading `needsYou > 0 ? needsYou : blocked`. Red:
/// "2 need you".
@Test func aRelayCountOfZeroIsZeroWhateverIsBlocked() throws {
    let state = try decode(
        """
        {"status":"working","detail":"","blocked":2,"review":0,"working":1,"more":0,
         "needsYou":0,
         "rows":[{"terminal":"a","label":"claude","status":"working","detail":""}]}
        """)
    #expect(state.headerCount == 0)
    let layout = try #require(AgentCardLayout(state: state, now: cardNow, stale: false))
    #expect(layout.title == "1 in flight")
}

/// The Island and the headline card count what the header counts. Two
/// decisions and a blocked leader: three need you, and the tail names the two
/// that aren't on the card, not the zero other blocked agents.
///
/// Mutation: `waiting` from `state.blocked`. Red: "+1 more working".
@Test func theIslandsTailCountsWhatTheHeaderCounts() throws {
    let state = try decode(
        """
        {"terminal":"a","status":"blocked","detail":"","blocked":1,"review":0,"working":1,
         "more":0,"needsYou":3}
        """)
    let tail = FleetTail.current(for: state, snapshot: nil, now: cardNow, stale: false)
    #expect(tail.blocked == 2)
    #expect(tail.line == "+1 more · 2 need you")
}

/// With only a decision waiting and nobody else running, the Island still
/// says so, rather than nothing.
///
/// Mutation: `line` returning nil whenever `others` is 0. Red.
@Test func aDecisionAloneStillHasALineOnTheIsland() throws {
    let state = try decode(
        """
        {"terminal":"a","status":"working","detail":"","blocked":0,"review":0,"working":1,
         "more":0,"needsYou":1}
        """)
    let tail = FleetTail.current(for: state, snapshot: nil, now: cardNow, stale: false)
    #expect(tail.line == "1 needs you")
}

/// The snapshot's branch counts its items, less the leader's own.
///
/// Mutation: that branch counting blocked agents. Red: 0, not 1.
@Test func theSnapshotTailCountsItemsLessTheLeaders() {
    let working = FleetSnapshot.Agent(
        id: "b", label: "codex", machine: "studio", status: "working", glyph: "", headline: "",
        line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: nil,
        observedAt: cardNow)
    let decision = NeedsYouItem(id: "decision:1", kind: .decision, rank: 5, since: nil, question: "?")
    let snapshot = FleetSnapshot(
        agents: [working], capturedAt: cardNow, complete: true, needsYou: [decision])
    let state = AgentCardState(terminal: "a", status: "working", detail: "")
    let tail = FleetTail.current(for: state, snapshot: snapshot, now: cardNow, stale: false)
    #expect(tail.blocked == 1)
}

/// A long workspace is cut, never the agent's name.
///
/// Mutation: `named` without the cap. Red.
@Test func aLongWorkspaceIsCutNotTheName() {
    let long = String(repeating: "w", count: 48)
    #expect(
        AgentCardLayout.named("claude", in: long)
            == String(repeating: "w", count: 15) + "… · claude")
}

// MARK: - The locked card never draws a blocked question (ov-57 fix 1, F3)

/// A blocked pane's `detail` is its screen question, and for a shell command
/// that question carries the command ("Run: cargo test?"). The card is on a
/// locked screen, so a blocked row draws the generic line instead; every
/// other row keeps its words.
///
/// Mutation: the row's detail drawn as sent. Red: "Force-push to origin/main?".
@Test func aBlockedRowNeverDrawsItsQuestion() throws {
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow, stale: false))
    let blocked = try #require(layout.rows.first { $0.row.status == "blocked" })
    #expect(blocked.detail == CardAskWording.generic)
    #expect(!blocked.detail.contains("origin"))
    let working = try #require(layout.rows.first { $0.row.status == "working" })
    #expect(working.detail == working.row.detail)
}
