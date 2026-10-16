import Foundation

// A phone acting on a pull request as the runner's own `gh` login (ov-317, team
// mode T6): mark a lane's draft pull request ready for review, and reply to a
// review thread an agent escalated. The runner does both (`pr.ready` and
// `pr.thread_reply`, `pr_actions`, Control scope); this holds what a phone sends,
// what it asks before sending, and what it says when the runner refuses.
// Android's `PrActions`, word for word.
//
// Review on GitHub is not here: it is `GitHubLink.url`, opened by the system,
// which hands a github.com link to GitHub Mobile when it is installed.

enum PhonePrActions {
    static let readyMethod = "pr.ready"
    static let replyMethod = "pr.thread_reply"

    /// The longest reply, in Unicode scalar values, which is how the runner counts
    /// (`pr_actions::REPLY_MAX`): not Swift's characters, which join a flag or a
    /// family into one.
    static let replyMax = 2000

    /// `pr.ready`'s arguments: the lane, and the number of the draft the lane
    /// row showed, so a lane with two drafts readies the one the person saw.
    static func readyRequest(lane: String, number: Int) -> [String: Any] {
        ["lane": lane, "pr_number": number]
    }

    /// `pr.thread_reply`'s arguments for the card whose Needs You row is the
    /// thread. With `send` false the runner posts nothing and says where the
    /// reply would go and as whom.
    ///
    /// A send carries what the preview answered (`shown`): the runner posts only to
    /// the thread the person was shown, and refuses when it has changed.
    static func replyRequest(task: String, body: String, send: Bool, shown: ReplyPreview? = nil) -> [String: Any] {
        var request: [String: Any] = ["task": task, "body": body, "send": send]
        if let shown {
            request["note_id"] = shown.noteID
            request["digest"] = shown.digest
        }
        return request
    }

    /// A reply, trimmed, or nil when it is empty or past the cap.
    static func reply(_ text: String) -> String? {
        let body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return body.isEmpty || body.unicodeScalars.count > replyMax ? nil : body
    }

    /// Where a reply would go and as whom: `pr.thread_reply` with `send` false.
    struct ReplyPreview: Equatable, Sendable {
        /// `owner/name`.
        var repository: String
        var number: Int
        /// The runner's `gh` login: the name the reply is posted under.
        var login: String
        /// In the comment's thread (true), or on the pull request (false).
        var inThread: Bool
        /// The waiting question the thread is on, and a digest of where its link
        /// resolves: handed back with the send.
        var noteID: String
        var digest: String

        init(repository: String, number: Int, login: String, inThread: Bool, noteID: String = "", digest: String = "") {
            self.repository = repository
            self.number = number
            self.login = login
            self.inThread = inThread
            self.noteID = noteID
            self.digest = digest
        }

        /// From the runner's answer; nil for anything that does not name all three.
        init?(json data: Data) {
            guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let repository = object["repository"] as? String, !repository.isEmpty,
                let number = (object["number"] as? NSNumber)?.intValue, number > 0,
                let login = object["login"] as? String, !login.isEmpty,
                let noteID = object["note_id"] as? String, !noteID.isEmpty,
                let digest = object["digest"] as? String, !digest.isEmpty
            else { return nil }
            self.init(
                repository: repository, number: number, login: login,
                inThread: object["in_thread"] as? Bool ?? false, noteID: noteID, digest: digest)
        }

        /// "acme/billing #213".
        var pullRequest: String { "\(repository) #\(number)" }

        /// The confirm sheet's title: which pull request, and nothing else.
        var title: String { "Reply on \(pullRequest)?" }

        /// What the confirm sheet says it will do, naming the login.
        var message: String {
            inThread
                ? "Posts your reply in this thread as \(login) on GitHub."
                : "Posts your reply on the pull request as \(login) on GitHub."
        }
    }

    /// The confirm sheet's title for Mark Ready.
    static func readyTitle(_ stage: PlanPrStage) -> String {
        stage.prNumber > 0 ? "Mark PR \(stage.prNumber) Ready for Review?" : "Mark Ready for Review?"
    }

    /// What it does: reviewers are asked the moment it is ready, so it asks first.
    static let readyMessage = "It stops being a draft, and the people asked to review it are notified."

    static let markReady = "Mark Ready"
    /// Somebody else's pull request, on a Needs You row: reviewed there.
    static let reviewOnGitHub = "Review on GitHub"
    /// A thread, and a lane's own pull request: opened there.
    static let openOnGitHub = "Open on GitHub"
    static let reply = "Reply\u{2026}"
    static let sendReply = "Send Reply"

    /// The sentence for a refusal: by the runner's `what` where it named one,
    /// else by its code. `action` finishes "Couldn’t …".
    static func refusal(word: String?, what: String?, action: String) -> String {
        switch what {
        case "lane_pr": return "This lane has no draft pull request to mark ready. It may already be ready, or closed."
        case "thread_link": return "Couldn’t find that thread on GitHub. Open it there to reply."
        case "already_answered": return "This thread was already answered."
        case "reply_body": return "A reply has to be between 1 and \(replyMax) characters."
        default: break
        }
        switch word {
        case "pr-state-unavailable":
            return "GitHub didn’t answer on the runner. Check that its GitHub login still works, then try again."
        case nil, "":
            return "Couldn’t \(action). Try again."
        case let word?:
            return RunnerRefusal.trouble(forWord: word, message: "", after: "Couldn’t \(action).").sentence
        }
    }
}
