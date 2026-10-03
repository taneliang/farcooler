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
            .needsYou: (GlanceMark(attention: .needsYou, core: .atAPrompt), "Needs You", .attention),
            .failed: (GlanceMark(attention: .failed, core: .atAPrompt), "Failed", .attention),
            .finished: (GlanceMark(attention: .toReview, core: .atAPrompt), "Finished", .quiet),
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
        #expect(failed.tone == .attention)
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
