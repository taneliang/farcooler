import Foundation
import Testing

@testable import AgentKit

/// A phone marks a lane's pull request ready and replies to a review thread
/// (ov-317). The plan is `test/fixtures/plan-phones-ov317.json`, which the
/// phones' UI test shares; the runner's answers are `test/fixtures/pr-actions/`,
/// written by `the_answers_are_the_bytes_the_phones_read` in `crates/client`
/// from the wire messages, so the decoder is fed what the runner sends.
struct PhonePrActionsTests {
    static func root() -> URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    static func plan() throws -> PlanModel {
        let data = try Data(contentsOf: root().appendingPathComponent("test/fixtures/plan-phones-ov317.json"))
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(object["plan"])))
    }

    static func lane(_ plan: PlanModel, _ name: String) throws -> PlanLane {
        try #require(plan.lanes.first { $0.name == name }, "\(name)")
    }

    static func bytes(_ name: String) throws -> Data {
        try Data(contentsOf: root().appendingPathComponent("test/fixtures/pr-actions/\(name)"))
    }

    @Test("Mark Ready and Reply are the owner's: a member's phone isn't offered them (ov-712)")
    func onlyTheOwnerActsOnPullRequests() {
        #expect(PhonePrActions.mayAct(prActions: true, grantedScope: "control", viewer: ""), "the owner's phone")
        #expect(PhonePrActions.mayAct(prActions: true, grantedScope: nil, viewer: nil), "not told yet, as Keep")
        #expect(!PhonePrActions.mayAct(prActions: true, grantedScope: "control", viewer: "bob"), "a member's phone")
        #expect(!PhonePrActions.mayAct(prActions: true, grantedScope: "control", viewer: "(released)"), "a released one")
        #expect(!PhonePrActions.mayAct(prActions: true, grantedScope: "read", viewer: ""), "a Read grant")
        #expect(!PhonePrActions.mayAct(prActions: false, grantedScope: "control", viewer: ""), "a runner without them")
    }

    @Test("A row says the problem alone when a person has to act, else one phrase")
    func rowWords() throws {
        let plan = try Self.plan()
        let words = try ["lane-1", "lane-2", "lane-3", "lane-5", "lane-6", "lane-8", "lane-14", "lane-silent"].map {
            try Self.lane(plan, $0).stage?.rowWords
        }
        #expect(
            words == [
                "Building", "Agent Review", "Fixing", "Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg",
                "Changes Requested", "Checks Failing", "PR State Unknown", nil,
            ])
        // The gray line keeps what the stage leaves to the state.
        #expect(try PlanWords.status(Self.lane(plan, "lane-8"), stage: false) == "Approved")
    }

    @Test("Mark Ready is offered for a draft the runner named, and for nothing else")
    func onlyADraftCanBeMarkedReady() throws {
        let plan = try Self.plan()
        let offered = try plan.lanes.filter { $0.stage?.canMarkReady == true }.map(\.name)
        #expect(offered == ["lane-2", "lane-3"], "agent review and fixing, both with a pull request")
        #expect(try Self.lane(plan, "lane-1").stage?.canMarkReady == false, "building has no pull request")
        #expect(try Self.lane(plan, "lane-14").stage?.canMarkReady == false, "a pull request that couldn't be read")
        var draft = try #require(Self.lane(plan, "lane-2").stage)
        draft.prNumber = 0
        #expect(!draft.canMarkReady, "a draft with no number to name has nothing to ready")
        #expect(PlanPrStageKind.allCases.filter { PlanPrStage(kind: $0, prNumber: 7).isDraft } == [.agentReview, .fixing])
    }

    @Test("Mark Ready names the number the row showed, and the lane; a reply names the card")
    func requests() throws {
        let draft = try #require(try Self.lane(try Self.plan(), "lane-2").stage)
        let ready = PhonePrActions.readyRequest(lane: "lane-id", number: draft.prNumber)
        #expect(ready["lane"] as? String == "lane-id" && ready["pr_number"] as? Int == 41)
        #expect(ready.keys.sorted() == ["lane", "pr_number"], "never a URL or a repository")
        let reply = PhonePrActions.replyRequest(task: "task-id", body: "Done.", send: false)
        #expect(reply["task"] as? String == "task-id" && reply["send"] as? Bool == false)
        #expect(reply.keys.sorted() == ["body", "send", "task"])
        #expect(PhonePrActions.readyTitle(draft) == "Mark PR 41 Ready for Review?")
    }

    @Test("A reply is trimmed, never empty, and at most 2,000 characters")
    func theReplyIsCapped() {
        #expect(PhonePrActions.reply("  Thanks, fixed.\n") == "Thanks, fixed.")
        #expect(PhonePrActions.reply("") == nil && PhonePrActions.reply(" \n ") == nil)
        #expect(PhonePrActions.reply(String(repeating: "x", count: 2000)) != nil)
        #expect(PhonePrActions.reply(String(repeating: "x", count: 2001)) == nil)
        #expect(PhonePrActions.reply(String(repeating: "\u{e9}", count: 2000)) != nil, "scalars, as the runner counts")
        // A family is one Character and seven scalars: 286 of them are past the cap.
        #expect(PhonePrActions.reply(String(repeating: "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}", count: 286)) == nil)
        #expect(PhonePrActions.reply(String(repeating: "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}", count: 285)) != nil)
    }

    @Test("The runner's preview names the pull request and the login the reply posts as")
    func previewDecodes() throws {
        let preview = try #require(PhonePrActions.ReplyPreview(json: try Self.bytes("reply-preview.json")))
        #expect(preview.pullRequest == "acme/billing #213" && preview.login == "e-liang" && preview.inThread)
        #expect(preview.noteID == "01000000-0000-7000-8000-000000000015" && preview.digest == "9f2c41d07b3a5e8812ac6d00")
        let send = PhonePrActions.replyRequest(task: "t", body: "Hi", send: true, shown: preview)
        #expect(send["note_id"] as? String == preview.noteID && send["digest"] as? String == preview.digest, "the send is bound to the preview")
        #expect(PhonePrActions.replyRequest(task: "t", body: "", send: false).keys.sorted() == ["body", "send", "task"])
        #expect(preview.title == "Reply on acme/billing #213?")
        #expect(preview.message == "Posts your reply in this thread as e-liang on GitHub.")
        var onThePullRequest = preview
        onThePullRequest.inThread = false
        #expect(onThePullRequest.message == "Posts your reply on the pull request as e-liang on GitHub.")
        for bad in [
            #"{}"#, #"{"repository":"a/b","number":0,"login":"x","note_id":"n","digest":"d"}"#,
            #"{"repository":"a/b","number":3,"login":"","note_id":"n","digest":"d"}"#,
            #"{"repository":"a/b","number":3,"login":"x"}"#, "nope",
        ] {
            #expect(PhonePrActions.ReplyPreview(json: Data(bad.utf8)) == nil, "\(bad)")
        }
    }

    @Test("Mark Ready's answer decodes with the same fixture the runner's test writes")
    func readyAnswerShape() throws {
        let object = try #require(JSONSerialization.jsonObject(with: try Self.bytes("ready.json")) as? [String: Any])
        #expect(object["number"] as? Int == 31 && object["already_ready"] as? Bool == false)
        #expect(GitHubLink.url(object["url"] as? String)?.host == "github.com")
    }

    @Test("A refusal says what happened in words, never the runner's")
    func refusals() {
        #expect(PhonePrActions.refusal(word: "invalid-argument", what: "lane_pr", action: "mark it ready").contains("no draft pull request"))
        #expect(PhonePrActions.refusal(word: "invalid-argument", what: "thread_link", action: "reply").contains("Open it there"))
        #expect(PhonePrActions.refusal(word: "invalid-argument", what: "reply_body", action: "reply").contains("2000"))
        #expect(PhonePrActions.refusal(word: "invalid-argument", what: "already_answered", action: "reply") == "This thread was already answered.")
        #expect(PhonePrActions.refusal(word: "pr-state-unavailable", what: nil, action: "reply").contains("GitHub didn’t answer"))
        #expect(PhonePrActions.refusal(word: nil, what: nil, action: "mark it ready") == "Couldn’t mark it ready. Try again.")
        #expect(PhonePrActions.refusal(word: "scope-denied", what: nil, action: "reply").contains("only look"))
        for said in [
            PhonePrActions.refusal(word: "pr-state-unavailable", what: nil, action: "reply"),
            PhonePrActions.refusal(word: "weird-new-code", what: nil, action: "reply"),
        ] {
            #expect(!said.contains("pr-state") && !said.contains("_") && !said.contains("weird-new-code"), "\(said)")
        }
    }
}
