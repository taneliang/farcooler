import Combine
import Foundation

// What the reader wants to tell the agent about the diff, and how it gets
// there.
//
// Written for the phone, in `apps/ios/FarCooler/ChangesReview.swift`, and moved
// here whole when the Mac's diff pane grew the same feature. The reasoning in
// each doc comment below is the phone's and is kept as it was argued: comments
// are ANCHORED so a sentence has a referent, they are BATCHED so an agent gets
// one turn rather than five, the queue PERSISTS on every write because the app
// holding it can be terminated between the window a note is written in and the
// window it would be sent in, and nothing here is EVER retried automatically
// because the protocol has no delivery receipt to retry against.
//
// One thing is abstracted rather than moved: the send itself. The phone calls
// `terminal.agent_prompt` through the FFI (`ClientCore.call`) and the Mac shells
// out to `farcooler terminal agent-prompt`; the same two paths every other call
// in these two apps already takes, for the reasons `DaemonClient` and
// `AgentStream` each record about themselves. So the queue takes a closure that
// hands one message to one pane and answers with a sentence when it could not.
// The mapping from a platform's own error type to that sentence stays on the
// platform, because only the platform knows what a `CoreError` or a non-zero
// exit means.
//
// What did NOT move: `ReviewPosition` and `ReviewBookmarks`, which are the
// phone's resume card. A Mac session is not terminated between ninety-second
// windows — see the parity inventory's do-not-copy list — so a Mac has no use
// for a bookmark, and a type nobody on this side would ever construct is not
// shared code, it is the phone's code stored somewhere further away.
//
// `Workspace.reviewAgentTargets()` did not move either, and that one is not a
// judgment call: `Workspace` and `Terminal` are declared separately per app and
// the two declarations do not agree here. Still true now that the phone's pair
// sits in this same package, in `CoreModel.swift` — they are `internal` there
// precisely so that being package-mates cannot be mistaken for being one type.
// The phone's `canSwitchPaneMode`
// is `chatCapable == true`; the Mac's is that AND `!isChangesPane`, because a
// Mac can put a diff in a pane and the daemon refuses to switch that one. The
// naming differs too — the phone joins a label to an ordinal, the Mac has only
// just learned how. What both sides share is `ReviewAgentTarget`, which is
// here, so the two lists are at least the same SHAPE.

// MARK: - What a comment is attached to

/// The part of the diff a comment is about.
///
/// The whole difference between a comment and a prompt. "Handle 429 as well"
/// sent on its own is a sentence with no referent, and an agent receiving it has
/// to guess which of the eleven files it just wrote is meant; the same sentence
/// carrying `push.ts`, around lines 120-148, and the line that was on screen is
/// an instruction that can be acted on without a search.
///
/// The quoted line comes out of the diff the daemon already sent and is capped
/// at `quoteLimit` here — a client-side cut for the size of a prompt, on text
/// the host had already decided to show. Nothing here re-reads a file, widens a
/// hunk, or recovers anything the daemon chose to truncate or redact.
///
/// A range of one line is the anchor a pointer can produce and a thumb cannot,
/// and it is the BETTER instruction: `firstLine == lastLine` says exactly which
/// line, where a hunk's range says which twenty-eight to look through. Nothing
/// in this type had to change to allow it — see `placeDescription`, which has
/// always had a sentence for it.
public struct ReviewAnchor: Codable, Equatable, Sendable {
    public var file: String
    /// The commit it was written against, when it was written against one.
    /// A comment on the branch as a whole carries no sha, truthfully.
    public var commit: String?
    /// The hunk's line range in the new file, when the comment was written on a
    /// hunk rather than on the file as a whole.
    public var firstLine: Int?
    public var lastLine: Int?
    /// One line of the hunk, so the agent is told WHERE in a 300-line file
    /// rather than only which file.
    public var quote: String?

    private static let quoteLimit = 120

    public init(
        file: String, commit: String? = nil, firstLine: Int? = nil, lastLine: Int? = nil,
        quote: String? = nil
    ) {
        self.file = file
        self.commit = commit
        self.firstLine = firstLine
        self.lastLine = lastLine
        self.quote = quote
    }

    public static func quoting(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard trimmed.count > quoteLimit else { return trimmed }
        return String(trimmed.prefix(quoteLimit)) + "…"
    }

    /// How this reads in the app, above the composer and in the outbox.
    public var placeDescription: String {
        guard let firstLine else { return "the whole file" }
        guard let lastLine, lastLine > firstLine else { return "line \(firstLine)" }
        return "lines \(firstLine)-\(lastLine)"
    }

    /// How this reads in the message the agent is sent.
    ///
    /// Backticked because the receiver is an agent reading markdown, and a path
    /// in prose is a path it has to guess the boundaries of.
    public var promptDescription: String {
        var out = "`\(file)`"
        if let commit { out += " (commit \(commit.prefix(8)))" }
        if firstLine != nil { out += ", around \(placeDescription)" }
        return out
    }
}

/// One thing the reader wants to say, not yet said.
public struct ReviewComment: Codable, Equatable, Identifiable, Sendable {
    public var id: UUID = UUID()
    public var anchor: ReviewAnchor
    public var text: String
    public var writtenAt: Double = Date().timeIntervalSince1970

    public init(
        id: UUID = UUID(), anchor: ReviewAnchor, text: String,
        writtenAt: Double = Date().timeIntervalSince1970
    ) {
        self.id = id
        self.anchor = anchor
        self.text = text
        self.writtenAt = writtenAt
    }
}

/// A batch that left the queue, kept so the app can show WHAT was handed over.
///
/// Required rather than nice: `session/prompt` is sent with `request_no_wait`
/// and its response signals end-of-turn, not receipt, so nothing anywhere can
/// confirm that an agent received a prompt. The only honest thing this screen
/// can offer is the text it handed over and the time it did so, which is what
/// this is. See `ReviewCommentQueue.send`.
public struct SentReviewBatch: Codable, Equatable, Identifiable, Sendable {
    public var id: UUID = UUID()
    public var text: String
    public var agentName: String
    public var sentAt: Double
    public var count: Int
    /// Set when the batch went into an agent's COMPOSER rather than to the
    /// agent — the Mac's second way out of the outbox, where the notes land
    /// beside the diff for the reader to send themselves.
    ///
    /// Optional, and that is not decoration: this type is persisted, so a
    /// non-optional field would fail to decode every receipt written before it
    /// existed and lose the whole queue with them. Absent means "sent", which
    /// is what every receipt written before this meant.
    public var placedInComposer: Bool?

    public init(
        id: UUID = UUID(), text: String, agentName: String, sentAt: Double, count: Int,
        placedInComposer: Bool? = nil
    ) {
        self.id = id
        self.text = text
        self.agentName = agentName
        self.sentAt = sentAt
        self.count = count
        self.placedInComposer = placedInComposer
    }
}

/// Why a batch did not go, in words worth reading.
///
/// Two fields rather than one string: a sentence this app wrote, and — only
/// where it has no account of its own — the runner's own words underneath. The
/// raw error is never the sentence. What a failed send hands back is a Rust
/// word for an empty session slot or a subprocess's stderr, and the person
/// reading it wanted to send a note to an agent; dropping it entirely is the
/// other mistake, because a failure nobody can diagnose is a failure nobody can
/// fix. So it travels in the box, and only on the arm that has nothing else to
/// say.
///
/// The phone's `ChangesStore.Trouble` IS this type — it is a typealias now
/// rather than a second declaration — so the diff screen's own error and the
/// queue's stay one shape on that side, which is what they were before this
/// moved.
public struct ReviewTrouble: Equatable, Sendable {
    public let sentence: String
    public var transcript: String?

    public init(sentence: String, transcript: String? = nil) {
        self.sentence = sentence
        self.transcript = transcript
    }
}

// MARK: - The queue

/// An agent pane a review comment can be sent to.
///
/// A value rather than a `Terminal`, so a diff view does not have to hold the
/// connection to know what it can send to — holding it would re-evaluate the
/// diff list's body on every three-second fleet poll, which is the one thing a
/// screen built for scrolling a long patch should not do.
public struct ReviewAgentTarget: Identifiable, Equatable, Sendable {
    /// Whatever this platform's send call names a pane by: the terminal's id
    /// through the FFI, its short id on a command line.
    public var id: String
    public var name: String
    /// Whether this pane is being DRAWN as a chat right now.
    ///
    /// Only the Mac reads it, and only to decide whether to offer "Put in
    /// Composer" beside "Send": a composer exists in a pane showing a chat, and
    /// putting text into one that is showing its raw terminal would be handing
    /// a note to a field nobody can see. The pane is still a perfectly good
    /// target for a SEND — `terminal.agent_prompt` reaches the agent either
    /// way — which is why this narrows one button rather than the list.
    public var showsChat: Bool

    public init(id: String, name: String, showsChat: Bool = false) {
        self.id = id
        self.name = name
        self.showsChat = showsChat
    }
}

/// Comments written across a review, collected until they are sent as one.
///
/// **Collect, then send** is the whole design, and it is about the receiving end
/// rather than the sending one. Firing a prompt per thought interrupts an agent
/// five times over ten minutes and produces five turns, each one re-reading the
/// files the last one just touched; the same five notes delivered together are
/// one turn against one snapshot of the branch. It also matches how reviewing
/// actually goes — the notes are made while reading and the decision to send is
/// a separate, later one.
///
/// Persisted on every write, because the app holding it can go away between the
/// two. On a phone that is near-certain — iOS terminates the app between
/// ninety-second windows, and an unsent comment lost to a process death is
/// worse than no comment feature at all. On a Mac it is a quit, a crash, or a
/// runner that goes and comes back with a new `DaemonClient` and therefore a
/// new store; the store is rebuilt from `UserDefaults` and the notes are still
/// there.
@MainActor
public final class ReviewCommentQueue: ObservableObject {
    /// Hand one message to one pane. Nil means it went; a `ReviewTrouble` means
    /// it did not, in words a person can read.
    ///
    /// The one platform-specific line in this file. See the note at the top for
    /// why it is a closure rather than two implementations of this class.
    public typealias Deliver = @MainActor (ReviewAgentTarget, String) async -> ReviewTrouble?

    /// Written, not yet sent.
    @Published public private(set) var pending: [ReviewComment] = [] { didSet { save() } }

    /// The last few batches that went, newest first.
    ///
    /// Capped at `sentKept`, because this is a receipt and not a history: the
    /// question it answers is "what did I just send", asked within a minute of
    /// sending it.
    @Published public private(set) var sent: [SentReviewBatch] = [] { didSet { save() } }

    /// Set while a send is in flight, so the button can say so and cannot be
    /// pressed twice — the one way this app could produce a duplicate prompt on
    /// a protocol that has no way to notice one.
    @Published public private(set) var sending = false

    /// A send that did not go.
    ///
    /// NOT cleared by anything on a timer and never acted on automatically.
    /// There is no acknowledgment on this path — see `SentReviewBatch` — so an
    /// automatic retry would be the app deciding, with no evidence, that a
    /// prompt which may well have arrived should be delivered a second time.
    /// The comments stay in `pending` and the reader is the one who decides.
    @Published public var failure: ReviewTrouble?

    private static let sentKept = 5

    private let deliver: Deliver
    private let workspace: String
    private let defaults: UserDefaults
    private var loading = true

    /// `defaults` is injected for the tests and for nothing else: a suite that
    /// wrote into the real `UserDefaults` would leave a queue of notes behind
    /// in whichever app ran it.
    public init(
        workspace: String, defaults: UserDefaults = .standard, deliver: @escaping Deliver
    ) {
        self.workspace = workspace
        self.defaults = defaults
        self.deliver = deliver
        if let data = defaults.data(forKey: Self.key(workspace)),
            let stored = try? JSONDecoder().decode(Stored.self, from: data)
        {
            pending = stored.pending
            sent = stored.sent
        }
        loading = false
    }

    public func add(_ comment: ReviewComment) {
        pending.append(comment)
        // A new comment is evidence the last failure has been read and moved
        // past. Left up, it would sit above a queue it no longer describes.
        failure = nil
    }

    public func remove(_ comment: ReviewComment) {
        pending.removeAll { $0.id == comment.id }
    }

    /// Everything queued, as one message.
    ///
    /// Numbered and grouped in the order they were written, which is reading
    /// order — the order the reader went through the diff in, and therefore the
    /// order in which the notes make sense to each other.
    public func message(branch: String) -> String {
        var lines: [String] = []
        let count = pending.count
        let noun = count == 1 ? "note" : "notes"
        if branch.isEmpty {
            lines.append("Review \(noun) from Far Cooler (\(count)):")
        } else {
            lines.append("Review \(noun) on `\(branch)` from Far Cooler (\(count)):")
        }
        lines.append("")
        for (index, comment) in pending.enumerated() {
            lines.append("\(index + 1). In \(comment.anchor.promptDescription):")
            if let quote = comment.anchor.quote {
                lines.append("   > \(quote)")
            }
            lines.append("   \(comment.text)")
            lines.append("")
        }
        return lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Hand the batch to an agent, once.
    ///
    /// The call `deliver` makes is `terminal.agent_prompt` on both platforms,
    /// the same one a typed message makes, so a comment batch arrives in the
    /// transcript exactly as a typed message does — there is no separate
    /// "review comment" channel to keep in step, and the agent's own history
    /// shows what it was told.
    ///
    /// On failure the comments STAY in `pending` and nothing is retried. The
    /// reader can press Try Again, and that is the only thing that ever sends
    /// this batch a second time: `request_no_wait` means a failure here is
    /// "this client did not get an answer", which is not the same as "the agent
    /// did not get the prompt", and only a person can weigh the difference.
    public func send(to target: ReviewAgentTarget, branch: String) async {
        guard !pending.isEmpty, !sending else { return }
        let text = message(branch: branch)
        let count = pending.count
        sending = true
        defer { sending = false }
        if let trouble = await deliver(target, text) {
            failure = trouble
            return
        }
        // Recorded BEFORE the queue is emptied, so a crash between the two
        // loses the receipt rather than the comments.
        record(text: text, target: target, count: count, placedInComposer: nil)
        pending = []
        failure = nil
    }

    /// Take the batch out of the queue for a composer to hold instead.
    ///
    /// The Mac's other way out, and the reason it is worth having: the agent
    /// pane is a few inches away, so the notes can land somewhere the reader
    /// can see them, edit them and press Return on themselves — at which point
    /// the missing delivery receipt this whole file is careful about stops
    /// mattering, because the message is visibly in a transcript.
    ///
    /// It empties `pending` like a send does. The text is not lost by that: it
    /// is in the composer, and it is in the receipt this returns with, which is
    /// the same receipt a send leaves behind and discloses the full message.
    /// Nothing here is described as sent, because nothing was.
    public func putInComposer(_ target: ReviewAgentTarget, branch: String) -> String? {
        guard !pending.isEmpty, !sending else { return nil }
        let text = message(branch: branch)
        record(text: text, target: target, count: pending.count, placedInComposer: true)
        pending = []
        failure = nil
        return text
    }

    private func record(
        text: String, target: ReviewAgentTarget, count: Int, placedInComposer: Bool?
    ) {
        sent.insert(
            SentReviewBatch(
                text: text, agentName: target.name,
                sentAt: Date().timeIntervalSince1970, count: count,
                placedInComposer: placedInComposer),
            at: 0)
        if sent.count > Self.sentKept { sent = Array(sent.prefix(Self.sentKept)) }
    }

    // MARK: Storage

    private struct Stored: Codable {
        var pending: [ReviewComment]
        var sent: [SentReviewBatch]
    }

    private static func key(_ workspace: String) -> String {
        "changes.comments.\(workspace)"
    }

    private func save() {
        // The `didSet`s above fire while `init` is assigning what was just
        // read, and writing it straight back would be a round trip through
        // `UserDefaults` for a value that came out of it.
        guard !loading else { return }
        guard let data = try? JSONEncoder().encode(Stored(pending: pending, sent: sent))
        else { return }
        defaults.set(data, forKey: Self.key(workspace))
    }
}
