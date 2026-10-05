import Foundation

// What the owner can do with an open ruling (ov-333), and the words each
// action leaves for the orchestrator. The rules live here, where a test
// reaches them, because the Mac and the iPhone offer the same three
// (Android says the same sentences from `model/PlanRulings.kt`).
//
//   Keep     the owner's own mark, instant, like read state. It changes
//            nothing in the plan, so it never reaches the orchestrator: the
//            screens call the runner's `plan ruling keep` / `ruling.keep`.
//   Reverse  a request to the orchestrator, sent: it does the work, then marks
//            the ruling reversed with its commit. Reversing never marks the
//            ruling itself.
//   Discuss  a quote of the ruling left in the orchestrator's composer, for
//            the owner to finish and send.
//
// A ruling's words are the orchestrator's, so they travel as `AskAboutTask`
// makes any task's words safe: one line, no escape sequences.

public enum RulingActions {
    /// The words left in the composer for Discuss: a reference and the
    /// decision, ending where the owner goes on typing.
    public static func discussDraft(_ ruling: PlanRuling) -> String {
        "About ruling \(ruling.short) (“\(AskAboutTask.oneLine(ruling.decision))”): "
    }

    /// The request Reverse sends. It carries the ruling's recorded reversal,
    /// what it said reversing costs, and says how to mark the work done.
    public static func reverseRequest(_ ruling: PlanRuling) -> String {
        let short = AskAboutTask.oneLine(ruling.short)
        return "Please reverse ruling \(short) (“\(AskAboutTask.oneLine(ruling.decision))”). "
            + "Reversing it: \(AskAboutTask.oneLine(ruling.reversal)) "
            + "When it's done, mark it with `plan ruling reverse \(short) --sha <commit>` (no commit: leave out --sha)."
    }

    /// Where a Reverse went.
    public enum Reversal: Equatable, Sendable {
        /// Sent to a chat orchestrator.
        case sent
        /// Typed into a terminal orchestrator's input, with no Enter: the
        /// owner presses Return.
        case drafted
        /// Onto the clipboard: the runner couldn't prove the pane safe.
        case copied
        /// The runner never answered in time, so it may have been typed.
        case maybeDrafted
        /// A chat orchestrator didn't take it.
        case failed
    }

    /// Send Reverse's request to the orchestrator, whichever kind of pane it
    /// is.
    ///
    /// A chat pane is sent it, as a typed message is. A terminal pane is asked
    /// of the runner, which types it with no Enter only past the gate that
    /// types an answer; anything less copies it instead. Nothing here marks
    /// the ruling reversed: only the orchestrator does, when the work is done.
    @MainActor
    public static func reverse(
        _ ruling: PlanRuling, isAgentPane: Bool,
        send: (String) async -> Bool,
        paste: (String) async -> AskAboutTask.DraftResult,
        copy: (String) -> Void
    ) async -> Reversal {
        let text = reverseRequest(ruling)
        if isAgentPane { return await send(text) ? .sent : .failed }
        switch await paste(text) {
        case .pasted: return .drafted
        case .unknown: return .maybeDrafted
        case .declined:
            copy(text)
            return .copied
        }
    }

    /// Leave Discuss's draft in the orchestrator's composer, or paste it into
    /// a terminal orchestrator's input.
    @MainActor
    public static func discuss(
        _ ruling: PlanRuling, isAgentPane: Bool,
        offer: (String) -> Void,
        paste: (String) async -> AskAboutTask.DraftResult,
        copy: (String) -> Void
    ) async -> AskAboutTask.Delivery {
        await AskAboutTask.deliver(
            text: discussDraft(ruling), isAgentPane: isAgentPane, offer: offer, paste: paste, copy: copy)
    }

    /// What Reverse asks before it sends anything (ruling R-18): the ruling
    /// named, and the reversal the request will carry. Keep and Discuss don't
    /// ask.
    public static func confirmTitle(_ ruling: PlanRuling) -> String { "Reverse \(ruling.short)?" }

    public static func confirmMessage(_ ruling: PlanRuling) -> String {
        "This asks the orchestrator to reverse “\(AskAboutTask.oneLine(ruling.decision))”. "
            + "Reversing it: \(AskAboutTask.oneLine(ruling.reversal))"
    }

    /// What the window says after a Reverse, or nil when nothing needs saying.
    public static func notice(for reversal: Reversal, ruling: PlanRuling) -> String? {
        switch reversal {
        case .sent: "Asked the orchestrator to reverse \(ruling.short)."
        case .drafted: "Put the request in the orchestrator’s input. Press Return to send it."
        case .copied: "Copied the request to reverse \(ruling.short). Paste it into the orchestrator."
        case .maybeDrafted: "Typed, not sent. Check the orchestrator’s input for the request."
        case .failed: "Couldn’t reach the orchestrator. Try again."
        }
    }
}
