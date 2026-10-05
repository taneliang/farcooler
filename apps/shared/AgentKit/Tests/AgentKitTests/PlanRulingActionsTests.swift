import Foundation
import Testing

@testable import AgentKit

/// The owner's actions on a ruling (ov-333): Keep is local, Reverse sends the
/// recorded reversal to the orchestrator, Discuss leaves a quote unsent.
@MainActor
struct PlanRulingActionsTests {
    static let ruling = PlanRuling(
        id: "r", number: 12, decision: "The inbox is amber.", why: "One attention color.",
        reversal: "One token; every surface follows.")

    /// Reverse sends one message to a chat orchestrator, carrying the ruling's
    /// recorded reversal and how to mark it done, and leaves nothing in the
    /// composer or on the clipboard.
    @Test func reverseSendsTheRecordedReversalToAChatOrchestrator() async {
        var sent: [String] = []
        var pasted = 0
        var copied = 0
        let outcome = await RulingActions.reverse(
            Self.ruling, isAgentPane: true,
            send: { sent.append($0); return true },
            paste: { _ in pasted += 1; return .pasted },
            copy: { _ in copied += 1 })
        #expect(outcome == .sent)
        #expect(sent.count == 1)
        #expect(sent[0].contains("One token; every surface follows."))
        #expect(sent[0].contains("ruling R-12"))
        #expect(sent[0].contains("`plan ruling reverse R-12 --sha <commit>`"))
        #expect((pasted, copied) == (0, 0))
    }

    /// A chat orchestrator that doesn't take it is a failure the owner is told
    /// about, not a Reverse that looks done.
    @Test func aRefusedSendIsAFailure() async {
        let outcome = await RulingActions.reverse(
            Self.ruling, isAgentPane: true, send: { _ in false }, paste: { _ in .pasted }, copy: { _ in })
        #expect(outcome == .failed)
        #expect(RulingActions.notice(for: .failed, ruling: Self.ruling) == "Couldn’t reach the orchestrator. Try again.")
    }

    /// A terminal orchestrator gets it typed with no Enter, or on the
    /// clipboard when the runner won't, and a call that never answered claims
    /// nothing and copies nothing.
    @Test func aTerminalOrchestratorIsPastedNotSent() async {
        var sent = 0
        var copied: [String] = []
        let drafted = await RulingActions.reverse(
            Self.ruling, isAgentPane: false, send: { _ in sent += 1; return true }, paste: { _ in .pasted },
            copy: { copied.append($0) })
        #expect(drafted == .drafted)
        let declined = await RulingActions.reverse(
            Self.ruling, isAgentPane: false, send: { _ in sent += 1; return true }, paste: { _ in .declined },
            copy: { copied.append($0) })
        #expect(declined == .copied)
        let unknown = await RulingActions.reverse(
            Self.ruling, isAgentPane: false, send: { _ in sent += 1; return true }, paste: { _ in .unknown },
            copy: { copied.append($0) })
        #expect(unknown == .maybeDrafted)
        #expect(sent == 0, "a terminal pane is never sent a message")
        #expect(copied.count == 1 && copied[0].contains("One token"))
        #expect(RulingActions.notice(for: .maybeDrafted, ruling: Self.ruling) == "Typed, not sent. Check the orchestrator’s input for the request.")
    }

    /// Discuss quotes the ruling into the composer, prefilled and unsent: it
    /// offers a draft ending where the owner types, and never sends.
    @Test func discussQuotesTheRulingAndNeverSends() async {
        var offered: [String] = []
        let delivery = await RulingActions.discuss(
            Self.ruling, isAgentPane: true, offer: { offered.append($0) }, paste: { _ in .pasted }, copy: { _ in })
        #expect(delivery == .composer)
        #expect(offered == ["About ruling R-12 (“The inbox is amber.”): "])
    }

    /// A ruling's words are made safe before they reach a composer or a pane:
    /// an escape sequence that would close a bracketed paste, and line breaks.
    @Test func hostileWordsAreMadeSafe() async {
        let hostile = PlanRuling(
            id: "r", number: 3, decision: "Blue\u{1B}[201~ now\nsecond line", why: "",
            reversal: "Do it\r\n\u{1B}]0;title\u{07}fast")
        #expect(RulingActions.discussDraft(hostile) == "About ruling R-3 (“Blue now second line”): ")
        let text = RulingActions.reverseRequest(hostile)
        #expect(!text.contains("\u{1B}") && !text.contains("\n") && !text.contains("\r"))
        #expect(text.contains("Do it fast"))
    }

    /// The owner reads open, kept and reversed; the store's words stay on the
    /// wire. A reversal names its commit when it has one.
    @Test func statesReadInTheOwnerSWords() throws {
        #expect(PlanWords.rulingState(.standing) == "Open")
        #expect(PlanWords.rulingState(.confirmed) == "Kept")
        var reversed = Self.ruling
        reversed.state = .reversed
        #expect(PlanWords.rulingSettled(reversed) == "Reversed")
        reversed.reversedSha = "6e7e5618"
        #expect(PlanWords.rulingSettled(reversed) == "Reversed in 6e7e5618")
        #expect(PlanWords.rulingSignature(reversed) != PlanWords.rulingSignature(Self.ruling))
    }

    /// `reversed_sha` decodes from the CLI's and the phones' JSON, and an
    /// answer without it still reads.
    @Test func theReversingCommitDecodes() throws {
        var text = try String(contentsOf: PlanRulingsTests.fixtureURL(), encoding: .utf8)
        text = text.replacingOccurrences(of: #""state": "confirmed""#, with: #""state": "reversed""#)
        text = text.replacingOccurrences(
            of: #""reversed_sha": null,"#, with: #""reversed_sha": "6e7e5618","#)
        let plan = try PlanModel.decode(Data(text.utf8))
        #expect(plan.rulings.allSatisfy { $0.reversedSha == "6e7e5618" })
        let past = try PlanModelTests.fixture().pastRulings
        #expect(past.count == 1 && past[0].reversedSha == nil)
    }

    /// The runner's new capability has its wire name, and Open and Past
    /// Decisions split what the plan carries.
    @Test func openAndPastSplitTheList() throws {
        #expect(Capability.boardRulingActions.rawValue == "board_ruling_actions")
        let plan = try PlanModelTests.fixture()
        #expect(plan.openRulings.map(\.short) == ["R-2"])
        #expect(plan.pastRulings.map(\.short) == ["R-1"])
        #expect(PlanWords.pastDecisions == "Past Decisions")
    }

    /// Reverse asks first, naming the ruling and the reversal the request will carry.
    @Test func reverseConfirmsByNamingTheRulingAndItsReversal() {
        #expect(RulingActions.confirmTitle(Self.ruling) == "Reverse R-12?")
        let message = RulingActions.confirmMessage(Self.ruling)
        #expect(message.contains("The inbox is amber."))
        #expect(message.contains("One token; every surface follows."))
    }
}
