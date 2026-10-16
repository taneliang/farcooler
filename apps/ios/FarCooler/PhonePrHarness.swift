#if DEBUG
import Foundation

// The canned runner's pull request actions, for the UI suite (`-phone-harness`
// with `-phone-pr-actions`, ov-317).
//
//   -phone-pr-actions        the runner advertises `pr_actions` and answers `pr.ready`
//                            and `pr.thread_reply`. `pr.ready` checks it is sent the lane
//                            and the number of a draft in the plan file, and from then on
//                            the plan says that lane waits on alice. `pr.thread_reply`
//                            checks the card and a reply of 1 to 2,000 characters, and
//                            answers acme/billing #213 as e-liang. Each call is in the
//                            harness's `sent`: `pr.ready <lane> <number>`, and
//                            `pr.thread_reply preview|send <chars>`.
//   -phone-pr-needs-you      Needs You also holds a pull request waiting for this runner's
//                            review and a review thread an agent escalated, as the runner
//                            sends them (`test/fixtures/needs-you.json`)
//   -phone-pr-refused        `pr.thread_reply` refuses with `thread_link`, as the runner
//                            does for a link it can't read to one place
//
// Without `-phone-pr-actions` the runner is one from before them: no capability,
// so the buttons are not drawn.

enum PhonePrHarness {
    static var advertised: Bool { CommandLine.arguments.contains("-phone-pr-actions") }
    static var needsYou: Bool { CommandLine.arguments.contains("-phone-pr-needs-you") }

    /// The lanes marked ready this run, by id.
    static var readied: Set<String> = []

    /// A thread replied to this run: its card is answered, so its row has left.
    static var answered = false

    static let thread = "01000000-0000-7000-8000-00000000000e"
    static let noteID = "01000000-0000-7000-8000-000000000015"
    static let digest = "9f2c41d07b3a5e8812ac6d00"

    /// A lane `pr.ready` marked ready reads as waiting on alice, as the runner's
    /// next read of GitHub would say.
    static func marking(_ plan: Any) -> Any {
        guard !readied.isEmpty, var plan = plan as? [String: Any], var lanes = plan["lanes"] as? [[String: Any]] else { return plan }
        func waiting(_ stage: [String: Any]) -> [String: Any] {
            var stage = stage
            stage["kind"] = "waiting_on_reviewer"
            stage["label"] = "Waiting on alice"
            stage["reviewers"] = ["alice"]
            return stage
        }
        for index in lanes.indices where readied.contains(lanes[index]["id"] as? String ?? "") {
            if let stage = lanes[index]["stage"] as? [String: Any] { lanes[index]["stage"] = waiting(stage) }
            if var cards = lanes[index]["cards"] as? [[String: Any]] {
                for card in cards.indices {
                    if let stage = cards[card]["stage"] as? [String: Any] { cards[card]["stage"] = waiting(stage) }
                }
                lanes[index]["cards"] = cards
            }
        }
        plan["lanes"] = lanes
        return plan
    }

    /// The items the runner adds to Needs You.
    static func items() -> [[String: Any]] {
        guard needsYou else { return [] }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let items: [[String: Any]] = [
            [
                "id": "pr-thread:\(thread)", "kind": "pr_thread", "rank": 399_999_099, "since": now - 900_000,
                "workspace_id": HarnessRunner.billing, "workspace_name": "Billing",
                "repository_id": HarnessRunner.repository,
                "task": ["id": thread, "key": "bil-11", "title": "Refund API", "status": "needs_decision"],
                "question": "alice asked, says the agent: \u{201C}Why not retry on 5xx?\u{201D}",
                "detail": "acme/billing #212", "link": "https://github.com/acme/billing/pull/212#discussion_r1",
                "actions": [["id": "Retry", "title": "Retry", "destructive": false, "primary": false]],
            ],
            [
                "id": "pr-review:0123456789abcdef", "kind": "pr_review", "rank": 499_998_199, "since": now - 1_800_000,
                "workspace_id": NSNull(), "workspace_name": "", "repository_id": NSNull(),
                "task": NSNull(), "terminal": NSNull(), "worktree": NSNull(),
                "question": "Add refund endpoint", "detail": "acme/billing #213 \u{00B7} bob",
                "link": "https://github.com/acme/billing/pull/213", "actions": [],
            ],
        ]
        return answered ? items.filter { $0["kind"] as? String != "pr_thread" } : items
    }

    /// The runner's answer to `method`, and the line for the harness's `sent`.
    static func answer(_ method: String, _ args: [String: Any], plan: HarnessPlan?) throws -> (Data, String) {
        func refuse(_ word: String, _ what: String? = nil) -> ClientCore.CoreError {
            .rejected("refused", word: word, what: what)
        }
        switch method {
        case "pr.ready":
            guard let lane = args["lane"] as? String, let number = (args["pr_number"] as? NSNumber)?.intValue,
                let plan, plan.draft(lane: lane, number: number)
            else { throw refuse("invalid-argument", "lane_pr") }
            readied.insert(lane)
            let url = "https://github.com/acme/app/pull/\(number)"
            let data = try JSONSerialization.data(withJSONObject: ["number": number, "url": url, "already_ready": false])
            return (data, "pr.ready \(lane) \(number)")
        default:
            guard args["task"] as? String == thread else { throw refuse("not-found") }
            let send = args["send"] as? Bool ?? false
            if answered { throw refuse("invalid-argument", "already_answered") }
            // A send is bound to a preview: what it hands back is what the preview answered.
            if send, args["note_id"] as? String != noteID || args["digest"] as? String != digest {
                throw refuse("invalid-argument", "thread_link")
            }
            let body = args["body"] as? String ?? ""
            if CommandLine.arguments.contains("-phone-pr-refused") { throw refuse("invalid-argument", "thread_link") }
            if send, body.isEmpty || body.count > PhonePrActions.replyMax { throw refuse("invalid-argument", "reply_body") }
            let data = try JSONSerialization.data(withJSONObject: [
                "repository": "acme/billing", "number": 213, "login": "e-liang", "sent": send, "in_thread": true,
                "note_id": noteID, "digest": digest,
            ])
            if send { answered = true }
            return (data, send ? "pr.thread_reply send \(body.count)" : "pr.thread_reply preview")
        }
    }
}

extension HarnessPlan {
    /// Whether `lane` has a draft pull request numbered `number` in the file.
    func draft(lane: String, number: Int) -> Bool {
        guard let plan = capture["plan"] as? [String: Any], let lanes = plan["lanes"] as? [[String: Any]],
            let found = lanes.first(where: { $0["id"] as? String == lane }),
            let stage = found["stage"] as? [String: Any]
        else { return false }
        let kind = stage["kind"] as? String
        return (kind == "agent_review" || kind == "fixing") && (stage["pr_number"] as? NSNumber)?.intValue == number
    }
}
#endif
