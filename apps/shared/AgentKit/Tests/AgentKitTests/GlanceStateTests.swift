import Foundation
import Testing

@testable import AgentKit

/// The one table from an agent's state to what a glance surface draws, says and
/// colors (ov-125).
///
/// Every state is pinned, and the failed turn three ways — through the table,
/// through `GlanceMark(agent:)` that the widgets and the watch read, and through
/// the lock screen card's rows — because each of those was once a separate
/// place where a dead build came out as a calm "to review" or "Finished".
///
/// Mutation: `GlanceState.init` mapping `"done"` to `.finished` whatever
/// `failed` says. Red: `everyStateHasOneMarkOneWordAndOneTone`,
/// `aFailedTurnIsNeverDrawnOrNamedAsAFinishedOne`, `theWidgetAndWatchReadTheFlag`
/// and `theCardsRowsReadTheSnapshotsFlag`.
///
/// Mutation: `GlanceMark(agent:)` passing `failed: false`. Red:
/// `theWidgetAndWatchReadTheFlag`.
///
/// Mutation: `GlanceState.mark` drawing `.failed` with `.toReview`. Red:
/// `everyStateHasOneMarkOneWordAndOneTone` and
/// `aFailedTurnIsNeverDrawnOrNamedAsAFinishedOne`.
struct GlanceStateTests {
    /// The whole table, written out. `CaseIterable` makes a sixth state a
    /// failure here until somebody writes its row.
    @Test func everyStateHasOneMarkOneWordAndOneTone() {
        let table: [GlanceState: (GlanceMark, String?, GlanceState.Tone)] = [
            .needsYou: (GlanceMark(attention: .needsYou, core: .atAPrompt), "Needs you", .needsYou),
            .failed: (GlanceMark(attention: .failed, core: .atAPrompt), "Failed", .failed),
            .finished: (GlanceMark(attention: .toReview, core: .atAPrompt), "Done", .quiet),
            .working: (GlanceMark(attention: .quiet, core: .producing), "Working", .quiet),
            .unstated: (GlanceMark(attention: .quiet, core: .atAPrompt), nil, .quiet),
        ]
        #expect(Set(table.keys) == Set(GlanceState.allCases))
        for state in GlanceState.allCases {
            let (mark, title, tone) = table[state]!
            #expect(state.mark == mark, "\(state)")
            #expect(state.title == title, "\(state)")
            #expect(state.tone == tone, "\(state)")
        }
    }

    /// From the wire's words: three statuses, one flag read only for `done`,
    /// and anything else to the tier that claims the least.
    @Test func theWireLandsOnEveryState() {
        let cases: [(String, Bool, GlanceState)] = [
            ("blocked", false, .needsYou),
            // The flag belongs to the turn that ENDED. An agent blocked on its
            // next question is asking, not failing.
            ("blocked", true, .needsYou),
            ("done", true, .failed),
            ("done", false, .finished),
            ("working", false, .working),
            ("working", true, .working),
            ("idle", false, .unstated),
            ("compacting", true, .unstated),
            ("", false, .unstated),
        ]
        for (status, failed, want) in cases {
            #expect(GlanceState(status: status, failed: failed) == want, "\(status) \(failed)")
        }
        #expect(Set(cases.map(\.2)) == Set(GlanceState.allCases))
    }

    /// The finding itself: a turn that died is the attention tier, amber, and
    /// says "Failed" — to the eye, beside the badge and to VoiceOver — and is
    /// drawn as loudly as a blocked agent at every size.
    @Test func aFailedTurnIsNeverDrawnOrNamedAsAFinishedOne() {
        let failed = GlanceState(status: "done", failed: true)
        let finished = GlanceState(status: "done", failed: false)
        #expect(failed != finished)
        #expect(failed.mark.attention == .failed)
        #expect(failed.mark != finished.mark)
        #expect(failed.title == "Failed")
        #expect(failed.tone == .failed)
        #expect(failed.tone != GlanceState.needsYou.tone)
        #expect(failed.mark.phrase == "Failed, at a prompt")
        #expect(!failed.mark.isQuiet)
        for size in GlanceMarkSize.allCases {
            #expect(size.stroke(.failed) == size.stroke(.needsYou), "\(size)")
        }
        // Latched like needs-you: a runner going quiet does not withdraw it,
        // and an old snapshot does not dash it.
        #expect(failed.mark.said(answering: false) == failed.mark)
        #expect(FleetSnapshot.isLatched("done"))
    }

    /// `done` is drawn ONE way: the review ring. The Live Activity's leader
    /// badge drew it as the quiet hairline beside rows drawing the review ring.
    @Test func doneIsDrawnOneWay() {
        let done = GlanceState(status: "done", failed: false).mark
        #expect(done == GlanceMark(status: "done"))
        #expect(done.attention == .toReview)
        let row = AgentCardRow(terminal: "t", status: "done")
        let layout = AgentCardLayout(state: Self.card([row]), stale: false)
        #expect(layout?.rows.first?.mark == done)
    }

    /// The widgets, the watch's rows and its complication all draw
    /// `GlanceMark(agent:)`, which read `status` alone.
    @Test func theWidgetAndWatchReadTheFlag() {
        #expect(GlanceMark(agent: Self.agent("done", failed: true)).attention == .failed)
        #expect(GlanceMark(agent: Self.agent("done", failed: false)).attention == .toReview)
        #expect(
            GlanceMark(agent: Self.agent("done", failed: true), confidence: .lastSeen)
                == GlanceMark(attention: .failed, core: .atAPrompt, link: .broken))
    }

    /// The card's push carries no outcome; the snapshot the notification
    /// service writes does. A `done` row whose terminal failed is drawn failed,
    /// and only a `done` one.
    @Test func theCardsRowsReadTheSnapshotsFlag() {
        let snapshot = FleetSnapshot(
            agents: [
                Self.agent("done", failed: true, id: "dead"),
                Self.agent("done", failed: false, id: "fine"),
                // Back at work since: its old failure is not this turn's.
                Self.agent("working", failed: true, id: "again"),
            ],
            capturedAt: Date(), complete: true)
        #expect(snapshot.failedTurns == ["dead"])

        let rows = ["dead", "fine", "again"].map {
            AgentCardRow(terminal: $0, status: $0 == "again" ? "working" : "done")
        }
        var card = Self.card(rows)
        card.rows = Array(rows.prefix(2))
        let layout = AgentCardLayout(state: card, stale: false, failed: snapshot.failedTurns)
        #expect(layout?.rows.map(\.mark.attention) == [.failed, .toReview])
        // And a row the push says is working is never drawn failed, whatever
        // the set holds.
        card.rows = [rows[2]]
        let working = AgentCardLayout(state: card, stale: false, failed: ["again"])
        #expect(working?.rows.first?.mark.attention == .quiet)
    }

    /// The relay's word on a row beats the snapshot's, both ways: the file is
    /// written only when an alert reached the notification extension, so it
    /// can miss a failure and it can keep one a silent success has cleared.
    @Test func theRelaysWordBeatsTheSnapshot() {
        // Stale: the file still says the last turn failed; the relay says this
        // one finished.
        let cleared = AgentCardRow(terminal: "t", status: "done", failed: false)
        // Missed: the file never heard; the relay says it failed.
        let missed = AgentCardRow(terminal: "u", status: "done", failed: true)
        // Old relay: no word, so the file answers.
        let old = AgentCardRow(terminal: "v", status: "done")
        let known: Set<String> = ["t", "v"]
        #expect(cleared.turnFailed(known: known) == false)
        #expect(missed.turnFailed(known: []) == true)
        #expect(old.turnFailed(known: known) == true)
        #expect(old.turnFailed(known: []) == false)
        // Never for a row that is not done, whatever anything says.
        #expect(AgentCardRow(terminal: "t", status: "working", failed: true).turnFailed(known: known) == false)

        var card = Self.card([cleared, missed])
        card.failedTurns = 1
        let layout = AgentCardLayout(state: card, stale: false, failed: known)
        #expect(layout?.rows.map(\.mark.attention) == [.toReview, .failed])
    }

    /// The headline, the same way: the relay's `failed`, else the file.
    @Test func theLeaderReadsTheRelayFirst() {
        var card = AgentCardState(terminal: "t", status: "done", detail: "")
        #expect(GlanceState(card: card, known: ["t"]) == .failed)
        card.failed = false
        #expect(GlanceState(card: card, known: ["t"]) == .finished)
        card.failed = true
        #expect(GlanceState(card: card, known: []) == .failed)
        card.status = "working"
        #expect(GlanceState(card: card, known: ["t"]) == .working)
    }

    /// A failed turn is never counted as a calm "to review": the header says
    /// "1 failed" apart, its mark is the failed tier when nothing needs you, and
    /// the tally draws a failed ring rather than a review ring.
    @Test func theHeaderCountsAFailedTurnApart() {
        let rows = [
            AgentCardRow(terminal: "dead", status: "done", failed: true),
            AgentCardRow(terminal: "fine", status: "done", failed: false),
        ]
        var card = Self.card(rows)
        card.failedTurns = 1
        let layout = AgentCardLayout(state: card, stale: false)!
        #expect(layout.title == "1 failed")
        #expect(layout.counts == "1 to review")
        #expect(layout.mark.attention == .failed)
        #expect(layout.rings.map(\.attention) == [.failed, .toReview])

        // An older relay sends no count; the drawn rows the card knows failed
        // stand in for it.
        card.failedTurns = 0
        card.rows = [AgentCardRow(terminal: "dead", status: "done"), rows[1]]
        let old = AgentCardLayout(state: card, stale: false, failed: ["dead"])!
        #expect(old.title == "1 failed")

        // And needs-you still leads.
        card.blocked = 1
        card.needsYou = 1
        card.failedTurns = 1
        let both = AgentCardLayout(state: card, stale: false)!
        #expect(both.title == "1 needs you")
        #expect(both.counts == "1 failed · 1 to review")
        #expect(both.mark.attention == .needsYou)
    }

    /// The widgets' and the watch's one number: needs-you, then failed, then
    /// review. A fleet whose only trouble is a dead turn says so.
    @Test func theFleetGlanceCountsFailedTurns() {
        let snapshot = FleetSnapshot(
            agents: [Self.agent("done", failed: true, id: "a"), Self.agent("done", failed: false, id: "b")],
            capturedAt: Date(), complete: true, reviewsWaiting: 2)
        #expect(snapshot.glance(at: Date()) == .failed(1))
        #expect(FleetSnapshot.Glance.failed(1).phrase == "1 failed")
        #expect(GlanceMark(glance: .failed(1)).attention == .failed)
        let blocked = FleetSnapshot(
            agents: [Self.agent("done", failed: true, id: "a"), Self.agent("blocked", failed: false, id: "c")],
            capturedAt: Date(), complete: true)
        #expect(blocked.glance(at: Date()) == .blocked(1))
    }

    /// The new keys decode, absent is "not told", and a round trip keeps both.
    @Test func theOutcomeDecodesAndRoundTrips() throws {
        let json = Data(#"{"terminal":"t","status":"done","detail":"","failed":true,"failedTurns":2,"blocked":0,"review":2,"working":0,"rows":[{"terminal":"t","status":"done","failed":true},{"terminal":"u","status":"done"}]}"#.utf8)
        let card = try JSONDecoder().decode(AgentCardState.self, from: json)
        #expect(card.failed == true)
        #expect(card.failedTurns == 2)
        #expect(card.rows.map(\.failed) == [true, nil])
        let again = try JSONDecoder().decode(AgentCardState.self, from: JSONEncoder().encode(card))
        #expect(again == card)
        let bare = try JSONDecoder().decode(AgentCardState.self, from: Data(#"{"status":"done"}"#.utf8))
        #expect(bare.failed == nil)
        #expect(bare.failedTurns == 0)
    }

    private static func agent(
        _ status: String, failed: Bool, id: String = "t"
    ) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: "claude", machine: "orchard", status: status, glyph: "",
            headline: "", line: "", feed: [], rank: 0, turnFailed: failed,
            activityChangedAt: nil)
    }

    private static func card(_ rows: [AgentCardRow]) -> AgentCardState {
        AgentCardState(
            terminal: rows.first?.terminal ?? "", label: "claude", machine: "orchard",
            status: rows.first?.status ?? "", detail: "", blocked: 0, review: rows.count,
            working: 0, more: 0, rows: rows)
    }
}
