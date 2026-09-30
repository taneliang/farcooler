import Foundation
import Testing

@testable import AgentKit

/// The lock-screen card's payload, decoded the way the phone decodes it.
///
/// **This type had no tests at all**, and could not have had any: it lived
/// inside `AgentActivityAttributes`, which cannot compile on macOS, and this
/// suite runs on macOS. So the one part of the push contract with real logic in
/// it — a hand-written decoder that tells seconds from milliseconds by
/// magnitude, defaults every string rather than throwing, and has to tell an
/// absent count from a zero one — was verified by reading it.
///
/// Every case below is a real shape the relay sends or a real shape an older one
/// sent. The bytes are written out rather than round-tripped through the encoder,
/// because a fixture minted by the same code it checks agrees with any value at
/// all, including the wrong one. `services/relay/test/relay.test.ts` asserts the
/// other end of each of these.
private func decode(_ json: String) throws -> AgentCardState {
    try JSONDecoder().decode(AgentCardState.self, from: Data(json.utf8))
}

@Test func aFleetShapedCardDecodesEveryFieldTheRelaySends() throws {
    // Exactly what `pushActivity` composes, keys and all.
    let state = try decode(
        """
        {"terminal":"term-1","label":"auth-refactor","machine":"studio",
         "status":"blocked","detail":"Force-push to origin/main?",
         "startedAt":1755000000000,
         "blocked":2,"review":3,"working":3,"more":6,
         "insertions":391,"deletions":112,"commits":41}
        """)

    #expect(state.terminal == "term-1")
    #expect(state.label == "auth-refactor")
    #expect(state.machine == "studio")
    #expect(state.status == "blocked")
    #expect(state.detail == "Force-push to origin/main?")
    #expect(state.blocked == 2)
    #expect(state.review == 3)
    #expect(state.working == 3)
    // `more` is on the wire and deliberately not on this type: it counts the
    // fleet against the ROWS the relay sent, and this card draws one agent. A
    // key with no reader decodes to nothing and costs nothing, which is what
    // lets the relay run ahead of the card that will draw them.
    #expect(state.insertions == 391)
    #expect(state.deletions == 112)
    #expect(state.commits == 41)
    #expect(state.knowsFleet)
}

@Test func aCardFromARelayWithNoRosterKnowsItWasToldNothing() throws {
    // The compatibility case, and the one that decides whether the card draws
    // the pushed counts or falls back to this phone's own snapshot. An absent
    // count must not read as zero: "nobody is blocked" and "nothing counted" are
    // different facts and only one of them entitles a card to state a number.
    let state = try decode(
        """
        {"terminal":"term-1","label":"claude","machine":"studio",
         "status":"working","detail":"Running tests"}
        """)

    #expect(!state.knowsFleet)
    #expect(state.blocked == -1)
    #expect(state.working == -1)
    #expect(state.insertions == nil)
    #expect(state.deletions == nil)
}

@Test func anEmptyFleetIsNotTheSameAsAnUncountedOne() throws {
    // The other half of the same distinction, and the one a naive `?? 0` would
    // get wrong in the direction nobody notices: a relay that counted and found
    // nothing has still counted.
    let state = try decode(
        """
        {"status":"done","detail":"","blocked":0,"review":0,"working":0,"more":0}
        """)

    #expect(state.knowsFleet)
    #expect(state.blocked == 0)
}

@Test func aCardFromBeforeTheFleetRestructureStillDecodes() throws {
    // ActivityKit persists a content state and decodes it back into THIS type
    // when an upgraded app enumerates `Activity.activities`. A card started by a
    // build older than the per-install rekey has `{status, detail}` and nothing
    // else, and a strict decode would throw there — which does not merely lose a
    // field, it keeps the activity out of that list, so `reapDuplicates` can
    // never end the card it exists to end. The leniency is the feature.
    let state = try decode(#"{"status":"blocked","detail":"Create haiku.txt?"}"#)

    #expect(state.status == "blocked")
    #expect(state.detail == "Create haiku.txt?")
    #expect(state.terminal.isEmpty)
    #expect(state.label.isEmpty)
    #expect(state.machine.isEmpty)
    #expect(state.startedAt == nil)
}

@Test func aFieldOfTheWrongShapeCostsThatFieldAndNeverTheCard() throws {
    // The rule the whole decoder is written around. Three programs write these
    // keys and only one of them is Swift, so a type that drifts is not a build
    // error anywhere — it is a card that stops arriving, which looks from every
    // side like a relay that sent nothing.
    let state = try decode(
        """
        {"terminal":42,"label":null,"machine":"studio","status":"working",
         "detail":"Running tests","startedAt":"1755000000000",
         "blocked":"two","insertions":"many"}
        """)

    #expect(state.terminal.isEmpty)
    #expect(state.label.isEmpty)
    #expect(state.machine == "studio")
    #expect(state.detail == "Running tests")
    // A date STRING costs the timer, not the card: the app renders `startedAt`
    // as a native timer and a card with no clock is better than one counting
    // from a number nobody meant.
    #expect(state.startedAt == nil)
    // And a count of the wrong shape reads as uncounted, which drops the card
    // back to the snapshot rather than to a fabricated number.
    #expect(!state.knowsFleet)
    #expect(state.insertions == nil)
}

@Test func theTurnClockIsToldFromSecondsByMagnitude() throws {
    // The one seam where two timestamp conventions meet: the daemon's turn clock
    // is Unix MILLISECONDS and every other stamp in `services/relay/src/push.ts`
    // is Unix SECONDS. Swift's own default is neither — seconds since 2001 — so
    // left alone a plausible number decodes decades out and the card counts
    // nonsense, which is a WRONG timer and the one thing `startedAt` being
    // optional was meant to avoid.
    let millis = try decode(#"{"status":"working","detail":"","startedAt":1755000000000}"#)
    let seconds = try decode(#"{"status":"working","detail":"","startedAt":1755000000}"#)

    #expect(millis.startedAt == Date(timeIntervalSince1970: 1_755_000_000))
    #expect(seconds.startedAt == Date(timeIntervalSince1970: 1_755_000_000))

    // Zero is a decodable instant and must not become one: a card counting up
    // from January 1970 is worse than a card with no clock on it.
    let epoch = try decode(#"{"status":"working","detail":"","startedAt":0}"#)
    #expect(epoch.startedAt == nil)
}

@Test func aStateRoundTripsToTheSameThingItWasDecodedFrom() throws {
    // ActivityKit persists a content state by ENCODING it and reads it back by
    // decoding it, so the two halves have to agree — a pair that wrote
    // seconds-since-2001 and read milliseconds would move every card's timer by
    // decades across a single app launch.
    //
    // And the counts have to survive it in the same shape: encoding `-1` for a
    // card that was told nothing would turn "the relay said nothing" into a
    // stored number, and the next decode would read it back as an answer.
    let told = try decode(
        """
        {"terminal":"t","label":"a","machine":"m","status":"blocked","detail":"?",
         "startedAt":1755000000000,"blocked":1,"review":0,"working":2,"more":0,
         "insertions":10,"deletions":2,"commits":1}
        """)
    let untold = try decode(#"{"status":"working","detail":"x"}"#)

    for state in [told, untold] {
        let back = try JSONDecoder().decode(
            AgentCardState.self, from: JSONEncoder().encode(state))
        #expect(back == state)
        #expect(back.knowsFleet == state.knowsFleet)
    }

    // Explicitly: nothing about the fleet is written for a card that was told
    // nothing about it.
    let keys = try JSONSerialization.jsonObject(with: JSONEncoder().encode(untold))
    #expect((keys as? [String: Any])?["blocked"] == nil)
}

// MARK: - The headline's ask (ov-57, T0 contract C1 and C4.2)

/// A relay older than the ask sends no `ask` key, and the card is exactly the
/// card it was: nothing else moves, and there is no ask.
///
/// Mutation: `ask` decoded to a placeholder rather than nil when absent.
@Test func aStateWithoutAnAskDecodesAsBefore() throws {
    let state = try decode(
        """
        {"terminal":"t","label":"claude","machine":"studio","workspace":"Billing",
         "status":"blocked","detail":"Run this command?","blocked":1,"review":0,"working":0}
        """)
    #expect(state.ask == nil)
    #expect(state.status == "blocked")
    #expect(state.workspace == "Billing")
    #expect(state.detail == "Run this command?")
}

/// The ask is the headline's, carried as the relay carries it: an id, maybe a
/// tool, and when the daemon's hold ends in Unix milliseconds.
///
/// Mutation: `until` read as seconds. Red: a date 1000 times too far out.
@Test func anAskDecodesItsIdToolAndHoldEnd() throws {
    let state = try decode(
        """
        {"terminal":"t","status":"blocked","detail":"",
         "ask":{"id":"hook-ask-0199a1b2-7c3d-7e4f-8a9b-0c1d2e3f4a5b","tool":"Bash",
                "until":1790551063000}}
        """)
    #expect(state.ask?.id == "hook-ask-0199a1b2-7c3d-7e4f-8a9b-0c1d2e3f4a5b")
    #expect(state.ask?.tool == "Bash")
    #expect(state.ask?.until == Date(timeIntervalSince1970: 1_790_551_063))
}

/// Every way an ask can be wrong costs the ask, or only its tool, and never the
/// card: an activity whose state throws is one nothing can end.
///
/// Mutation: `CardAsk.init(from:)` decoding strictly. Red: the decode throws.
@Test func aMalformedAskCostsTheAskNotTheCard() throws {
    let noAsk = [
        #""ask":"hook-ask-1""#,  // not an object
        #""ask":{"tool":"Bash","until":1790551063000}"#,  // no id
        #""ask":{"id":"hook-ask-1","tool":"Bash"}"#,  // no until
        #""ask":{"id":"ask-1","until":1790551063000}"#,  // not a hook ask
        #""ask":{"id":"hook-ask-","until":1790551063000}"#,  // nothing after the prefix
        #""ask":{"id":"hook-ask-a b","until":1790551063000}"#,  // outside the id's alphabet
        "\"ask\":{\"id\":\"hook-ask-\(String(repeating: "a", count: 56))\",\"until\":1790551063000}",
        #""ask":{"id":"hook-ask-1","until":0}"#,  // not a time
        #""ask":{"id":"hook-ask-1","until":-5}"#,
        #""ask":{"id":"hook-ask-1","until":"soon"}"#,
        #""ask":null"#,
    ]
    for field in noAsk {
        let state = try decode(#"{"terminal":"t","status":"blocked","detail":"Run?","#
            + field + "}")
        #expect(state.ask == nil, "\(field)")
        #expect(state.terminal == "t" && state.detail == "Run?", "\(field)")
    }

    // A tool outside the vocabulary costs the tool and keeps the ask.
    let badTools = [
        #""tool":"rm -rf /""#, #""tool":"""#, #""tool":7"#,
        "\"tool\":\"\(String(repeating: "a", count: 65))\"",
    ]
    for tool in badTools {
        let state = try decode(
            #"{"status":"blocked","detail":"","ask":{"id":"hook-ask-1","until":1790551063000,"#
                + tool + "}}")
        #expect(state.ask?.id == "hook-ask-1", "\(tool)")
        #expect(state.ask?.tool == nil, "\(tool)")
    }

    // The widest legal values pass: a 55-character id tail and a 64-byte MCP tool.
    let widest = try decode(
        "{\"status\":\"blocked\",\"detail\":\"\",\"ask\":{\"id\":\"hook-ask-"
            + String(repeating: "a", count: 55) + "\",\"tool\":\"mcp__"
            + String(repeating: "b", count: 59) + "\",\"until\":1790551063000}}")
    #expect(widest.ask?.id.count == 64)
    #expect(widest.ask?.tool?.count == 64)
}

/// ActivityKit persists a card by encoding it, so the ask has to come back as
/// it went in: `until` in milliseconds, and a missing tool still missing. A
/// card with no ask writes no `ask` key, never `"ask": null`.
///
/// Mutation: `ask` left out of `encode(to:)`. Red: the round trip loses it.
@Test func anAskRoundTripsThroughPersistence() throws {
    let withTool = try decode(
        #"{"terminal":"t","status":"blocked","detail":"","ask":{"id":"hook-ask-1","tool":"Edit","until":1790551063250}}"#)
    let withoutTool = try decode(
        #"{"terminal":"t","status":"blocked","detail":"","ask":{"id":"hook-ask-2","until":1790551063000}}"#)
    let none = try decode(#"{"terminal":"t","status":"blocked","detail":""}"#)

    for state in [withTool, withoutTool, none] {
        let back = try JSONDecoder().decode(
            AgentCardState.self, from: JSONEncoder().encode(state))
        #expect(back == state)
    }

    let written = try JSONSerialization.jsonObject(with: JSONEncoder().encode(withTool))
    let ask = (written as? [String: Any])?["ask"] as? [String: Any]
    #expect((ask?["until"] as? NSNumber)?.int64Value == 1_790_551_063_250)
    let bare = try JSONSerialization.jsonObject(with: JSONEncoder().encode(none))
    #expect((bare as? [String: Any]).map { $0.keys.contains("ask") } == false)
}

/// A Live Activity redraws only on an update or at its stale date, so a card
/// whose ask-clear push was lost would keep its buttons past the hold. Its
/// stale date is therefore the ask's `until` when that comes first: iOS redraws
/// the card then, and `CardLeaderAsk` draws no buttons past `until`.
///
/// Mutation: the ask ignored. Red: the hour-long stale date is kept.
@Test func aCardWithAnAskGoesStaleWhenItsHoldEnds() throws {
    let until = Date(timeIntervalSince1970: 1_790_551_063)
    let hour = until.addingTimeInterval(3600)
    let asking = AgentCardState(status: "blocked", detail: "", ask: CardAsk(id: "hook-ask-1", until: until))
    #expect(asking.staleDate(capping: hour) == until)
    #expect(asking.staleDate(capping: nil) == until)
    #expect(asking.staleDate(capping: until.addingTimeInterval(-5)) == until.addingTimeInterval(-5))
    let quiet = AgentCardState(status: "blocked", detail: "")
    #expect(quiet.staleDate(capping: hour) == hour)
    #expect(quiet.staleDate(capping: nil) == nil)
}

// MARK: - A runner that stopped beating (ov-71)

/// The relay names the runners on the card that stopped beating under
/// `quiet`. Absent is none, which is every card from a relay older than
/// this; a value of the wrong shape costs the names and never the card; and
/// none is written back as no key, so ActivityKit's persisted round trip
/// gives back what it was handed.
///
/// Mutation: `quiet` not decoded (always empty). Red.
@Test func theQuietRunnersDecodeAndRoundTrip() throws {
    let named = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":0,
         "quiet":["Studio Mac","Attic"]}
        """)
    #expect(named.quiet == ["Studio Mac", "Attic"])
    let back = try JSONDecoder().decode(AgentCardState.self, from: JSONEncoder().encode(named))
    #expect(back.quiet == ["Studio Mac", "Attic"])

    #expect(try decode(#"{"status":"working","detail":""}"#).quiet == [])
    let wrong = try decode(#"{"status":"blocked","detail":"?","quiet":"Studio"}"#)
    #expect(wrong.quiet == [])
    #expect(wrong.status == "blocked")

    let keys = try JSONSerialization.jsonObject(
        with: JSONEncoder().encode(AgentCardState(status: "working", detail: "")))
    #expect((keys as? [String: Any])?["quiet"] == nil)
}

/// A card whose every line went to a runner that stopped beating can't vouch
/// for anything as now, any more than a stale one: the relay took the quiet
/// runner's working agents out of the rows, so a card with names and no rows
/// is headlining one of them. Rows, or no names, and it vouches as before.
///
/// Mutation: `unvouched(stale:)` returning `stale` alone. Red.
@Test func aCardWithOnlyQuietWorkVouchesForNothing() throws {
    let quietOnly = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":0,"more":1,
         "rows":[],"quiet":["Studio Mac"]}
        """)
    #expect(quietOnly.unvouched(stale: false))
    #expect(quietOnly.unvouched(stale: true))

    let withRows = try decode(
        """
        {"status":"blocked","detail":"","blocked":1,"review":0,"working":0,"more":1,
         "rows":[{"terminal":"z","label":"zeno","status":"blocked","detail":""}],
         "quiet":["Studio Mac"]}
        """)
    #expect(!withRows.unvouched(stale: false))
    #expect(withRows.unvouched(stale: true))

    let nobodyQuiet = try decode(
        """
        {"status":"working","detail":"","blocked":0,"review":0,"working":1,"rows":[]}
        """)
    #expect(!nobodyQuiet.unvouched(stale: false))
}
