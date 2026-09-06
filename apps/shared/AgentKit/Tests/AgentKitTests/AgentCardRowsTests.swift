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
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow))

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
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow))
    #expect(layout.title == "2 need you")
    #expect(layout.counts == "3 to review · 3 in flight")
    #expect(layout.mark == GlanceMark(attention: .needsYou, core: .atAPrompt))
}

@Test func anEmptyTierIsDroppedRatherThanWrittenAsZero() throws {
    // "0 need you" is worse than silence on a lock screen, and a header that
    // led with it would spend the card's loudest line on nothing.
    let quiet = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":4,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
        """)
    let layout = try #require(AgentCardLayout(state: quiet, now: cardNow))
    #expect(layout.title == "4 in flight")
    #expect(layout.counts == nil)
    #expect(layout.mark == GlanceMark(attention: .quiet, core: .producing))

    let one = try decode(
        """
        {"status":"blocked","detail":"","blocked":1,"review":0,"working":0,"more":0,
         "rows":[{"terminal":"a","label":"a","status":"blocked","detail":""}]}
        """)
    #expect(try #require(AgentCardLayout(state: one, now: cardNow)).title == "1 needs you")
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
    let layout = try #require(AgentCardLayout(state: idle, now: cardNow))
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
    #expect(AgentCardLayout(state: old, now: cardNow) == nil)

    // And rows without counts, which nothing sends but which must not put a
    // header on the card claiming a fleet of minus one.
    let countless = try decode(
        """
        {"status":"working","detail":"",
         "rows":[{"terminal":"a","label":"a","status":"working","detail":""}]}
        """)
    #expect(!countless.knowsFleet)
    #expect(AgentCardLayout(state: countless, now: cardNow) == nil)
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
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow))
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
            now: cardNow.addingTimeInterval(FleetSnapshot.staleAfter)))
    #expect(aged.rows[0].mark.link == .live, "blocked holds at any age")
    #expect(aged.rows[1].mark.link == .broken, "working does not")
    // And the rule is the snapshot's own, not a second copy of it.
    #expect(
        FleetSnapshot.confidence(status: "working", heard: FleetSnapshot.staleAfter)
            == .lastSeen)
    #expect(FleetSnapshot.confidence(status: "blocked", heard: 86400) == .known)
}

@Test func theTailDrawsARingPerAgentInTierOrder() throws {
    let layout = try #require(AgentCardLayout(state: try decode(fleetPush()), now: cardNow))
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
    let layout = try #require(AgentCardLayout(state: big, now: cardNow))
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
        return try #require(AgentCardLayout(state: try decode(json), now: cardNow)).line
    }
    #expect(try line(more: 6, totals: true) == "+6 more · +391 −112")
    #expect(try line(more: 6, totals: false) == "+6 more")
    #expect(try line(more: 0, totals: true) == "+391 −112")
    #expect(try line(more: 0, totals: false) == nil)
}
