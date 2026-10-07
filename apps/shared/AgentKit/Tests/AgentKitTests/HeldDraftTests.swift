import Foundation
import Testing

@testable import AgentKit

// A draft held behind a dialog (ov-385): the answer read, the pane's bar,
// and that nothing is copied when the runner holds it.

/// `{"held": …}` as `draft_hold_json` writes it, in the FFI's and the CLI's
/// answers alike (`crates/client/src/session/draft_prompt.rs`).
private let heldAnswer = Data(
    #"{"held":{"id":"0198f2c0-0000-7000-8000-00000000f385","state":"waiting","heldMs":1000,"expiresMs":1801000,"endedMs":0}}"#
        .utf8)

@Test func aHeldAnswerIsReadAsHeldAndAnyOtherAsPasted() {
    #expect(HeldDraft.result(of: heldAnswer)
        == .held(DraftHold(id: "0198f2c0-0000-7000-8000-00000000f385", state: .waiting, expiresMs: 1_801_000)))
    #expect(HeldDraft.result(of: Data("{}".utf8)) == .pasted)
    #expect(HeldDraft.result(of: Data(#"{"pasted":true}"#.utf8)) == .pasted)
}

@Test func aFleetRowCarriesItsHoldAndAnUnknownStateIsNeverWaiting() throws {
    let row = #"{"id":"t","short":"t","title":"claude","preset":"claude","state":"running","epoch":1,"#
        + #""draftHold":{"id":"h","state":"sent","heldMs":1,"expiresMs":2,"endedMs":3}}"#
    let terminal = try JSONDecoder().decode(Terminal.self, from: Data(row.utf8))
    #expect(terminal.draftHold == DraftHold(id: "h", state: .sent, expiresMs: 2))
    let newer = try JSONDecoder().decode(DraftHold.self, from: Data(#"{"id":"h","state":"parked"}"#.utf8))
    #expect(newer.state == .expired)
    let none = try JSONDecoder().decode(Terminal.self, from: Data(row.replacingOccurrences(
        of: #","draftHold":{"id":"h","state":"sent","heldMs":1,"expiresMs":2,"endedMs":3}"#, with: "").utf8))
    #expect(none.draftHold == nil)
}

@MainActor
@Test func aHeldDraftIsNeitherCopiedNorClaimedPasted() async {
    var copied: [String] = []
    let hold = DraftHold(id: "h", state: .waiting)
    let delivery = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it", isAgentPane: false,
        offer: { _ in }, paste: { _ in .held(hold) }, copy: { copied.append($0) })
    #expect(delivery == .held(hold))
    #expect(copied.isEmpty)

    let ruling = PlanRuling(id: "r", number: 1, decision: "Use JSON", why: "", reversal: "Switch back")
    let reversal = await RulingActions.reverse(
        ruling, isAgentPane: false, send: { _ in true }, paste: { _ in .held(hold) }, copy: { copied.append($0) })
    #expect(reversal == .held(hold))
    #expect(RulingActions.notice(for: reversal, ruling: ruling) == nil)
    #expect(copied.isEmpty)
}

@Test func thePaneSaysWaitingThenSentAndOffersWithdrawOnlyWhileItWaits() {
    var watch = HeldDraft.Watch()
    #expect(watch.status(nil) == nil)
    let waiting = DraftHold(id: "h", state: .waiting)
    watch.observe(waiting)
    #expect(watch.status(waiting) == .waiting)
    #expect(HeldDraft.title(.waiting) == "Waiting for the dialog to close")
    let sent = DraftHold(id: "h", state: .sent)
    watch.observe(sent)
    #expect(watch.status(sent) == .sent)
    #expect(HeldDraft.title(.sent) == "Sent")
    watch.dismiss()
    #expect(watch.status(sent) == nil)
}

@Test func aHeldDraftThatEndsAnotherWayIsSaidOrLetGo() {
    let ids = "h"
    #expect(HeldDraft.status(of: ids, current: DraftHold(id: ids, state: .expired), last: .waiting) == .expired)
    #expect(HeldDraft.status(of: ids, current: DraftHold(id: ids, state: .failed), last: .waiting) == .failed)
    #expect(HeldDraft.status(of: ids, current: DraftHold(id: ids, state: .withdrawn), last: .waiting) == .gone)
    #expect(HeldDraft.status(of: ids, current: DraftHold(id: "newer", state: .waiting), last: .waiting) == .gone)
    #expect(HeldDraft.status(of: ids, current: nil, last: .waiting) == .lost)
    #expect(HeldDraft.status(of: ids, current: nil, last: nil) == .waiting)
    #expect(HeldDraft.title(.gone) == nil)
    #expect(HeldDraft.title(.expired) == "Not sent")
    #expect(HeldDraft.detail(.expired) == "It couldn’t go in within half an hour.")
}

/// The runner forgets a hold ten minutes after it ends (review 1, M1): "Sent"
/// stays "Sent", a withdrawn one says nothing, and only one last seen
/// waiting reads as lost.
@Test func anEndedHoldTheRunnerForgetsKeepsSayingHowItEnded() {
    var watch = HeldDraft.Watch()
    watch.observe(DraftHold(id: "h", state: .waiting))
    watch.observe(DraftHold(id: "h", state: .sent))
    watch.observe(nil)
    #expect(watch.status(nil) == .sent)

    var withdrawn = HeldDraft.Watch()
    withdrawn.observe(DraftHold(id: "h", state: .waiting))
    withdrawn.observe(DraftHold(id: "h", state: .withdrawn))
    #expect(withdrawn.status(nil) == nil)

    var restarted = HeldDraft.Watch()
    restarted.observe(DraftHold(id: "h", state: .waiting))
    #expect(restarted.status(nil) == .lost)
}
