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
