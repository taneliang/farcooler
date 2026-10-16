import Foundation

// A lane's pull request marked ready, and a review thread replied to, from the
// iPhone (ov-317): `pr.ready` and `pr.thread_reply` through the client core, the
// runner running `gh` as its own login. `PhonePrActions` (AgentKit) holds what is
// sent and said; this sends it and says what the runner refused.

extension Connection {
    /// Whether this phone may act on pull requests here: the runner has them
    /// (`pr_actions`) and this device holds Control. Otherwise the buttons are
    /// not drawn, rather than drawn to be refused.
    var mayActOnPullRequests: Bool {
        guard let build = knownBuild else { return false }
        return build.can(.prActions) && build.grantedScope != "read"
    }

    /// Mark the pull request `stage` names ready for review, on `lane`. Answers
    /// the sentence for a refusal, nil when it went through, and reads the plan
    /// again either way so the lane shows where it stands now.
    func markReady(lane: PlanLane, stage: PlanPrStage, in summary: WorkspaceSummary) async -> String? {
        var refused: String?
        do {
            _ = try await rpc(PhonePrActions.readyMethod, PhonePrActions.readyRequest(lane: lane.id, number: stage.prNumber))
        } catch let ClientCore.CoreError.rejected(_, word, what) {
            refused = PhonePrActions.refusal(word: word, what: what, action: "mark it ready")
        } catch {
            refused = PhonePrActions.refusal(word: nil, what: nil, action: "mark it ready")
        }
        await readPlan(summary)
        return refused
    }

    /// Where `task`'s thread reply would go and as whom, posting nothing; or the
    /// sentence for why it can't be said.
    func previewReply(task: String) async -> Result<PhonePrActions.ReplyPreview, PrActionFailure> {
        do {
            let data = try await rpc(PhonePrActions.replyMethod, PhonePrActions.replyRequest(task: task, body: "", send: false))
            guard let preview = PhonePrActions.ReplyPreview(json: data) else {
                return .failure(PrActionFailure(PhonePrActions.refusal(word: nil, what: nil, action: "reply")))
            }
            return .success(preview)
        } catch let ClientCore.CoreError.rejected(_, word, what) {
            return .failure(PrActionFailure(PhonePrActions.refusal(word: word, what: what, action: "reply")))
        } catch {
            return .failure(PrActionFailure(PhonePrActions.refusal(word: nil, what: nil, action: "reply")))
        }
    }

    /// Post `body` to `task`'s thread, the one `shown` was the preview of. Answers
    /// the sentence for a refusal, nil when it was posted (and the card answered).
    func sendReply(task: String, body: String, shown: PhonePrActions.ReplyPreview) async -> String? {
        guard let body = PhonePrActions.reply(body) else {
            return PhonePrActions.refusal(word: nil, what: "reply_body", action: "reply")
        }
        do {
            _ = try await rpc(PhonePrActions.replyMethod, PhonePrActions.replyRequest(task: task, body: body, send: true, shown: shown))
            // The reply answered the card: the row is gone from the runner's list.
            await loadNeedsYou()
            return nil
        } catch let ClientCore.CoreError.rejected(_, word, what) {
            await loadNeedsYou()
            return PhonePrActions.refusal(word: word, what: what, action: "reply")
        } catch {
            return PhonePrActions.refusal(word: nil, what: nil, action: "reply")
        }
    }
}

/// Why a reply couldn't be previewed, as a sentence.
struct PrActionFailure: Error, Equatable {
    let sentence: String
    init(_ sentence: String) { self.sentence = sentence }
}
