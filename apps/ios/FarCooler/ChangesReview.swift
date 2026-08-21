import Foundation
import SwiftUI

// Where you were, and what you want to say about it.
//
// Two things that outlive the process, kept together because they exist for the
// same reason. `ChangesStore` and everything else in `Changes.swift` hangs off
// `Connection`, which is torn down whenever iOS decides the app has been in the
// background long enough — and the situation this review surface is built for
// is a dozen ninety-second windows across an hour, so that happens roughly a
// dozen times per session. Anything held only in memory is therefore held only
// until the next set.
//
// So a bookmark and a comment queue live in `UserDefaults`, the same place
// `RunnerStore` keeps the runners: none of it is secret, none of it is large,
// and the one thing on this screen that IS secret — the diff itself — is
// deliberately not written down here. What is stored is a path, a sha, and
// whatever the reader typed, per workspace.
//
// Nothing in this file records a JUDGMENT. There is no "reviewed" flag and no
// per-file checkmark, and that is a decision rather than an omission: an agent
// is still editing these files, so a mark saying "I read this" on a file that
// has changed twice since is a lie the app would be telling on the reader's
// behalf. The workspace-level `changed_since_reviewed` watermark the daemon
// already keeps is the one piece of review state that survives an edit, because
// it is invalidated BY the edit. This file remembers position only.

// MARK: - The bookmark

/// Where one worktree's review was when the app last had a chance to notice.
///
/// Anchors rather than pixels, and that is the load-bearing choice. The obvious
/// spelling of "remember the scroll" is a content offset, and a content offset
/// is a promise about a layout — it means "1,840 points down" in a list whose
/// shape is decided by which files an agent has touched since. Come back after
/// a set during which the agent added two files above the one being read and
/// that number lands somewhere else entirely, silently. A PATH does not move:
/// `scrollTo(path)` lands on the same file whatever happened above it, and when
/// the file is gone it can be SAID that it is gone, which a number cannot do.
///
/// `topFile` is the answer for the case with no expanded file — arriving at the
/// list, scrolling through twenty headings, and being interrupted. The file at
/// the top of the screen is what "how far down was I" means when nothing is
/// open, and it survives the list changing shape for the same reason.
///
/// `savedAt` exists so a bookmark can be judged stale rather than merely old.
/// Nothing expires it today; it is recorded because the offer to resume is
/// worth phrasing differently for a position from four days ago, and the field
/// is free while the record is being written anyway.
struct ReviewPosition: Codable, Equatable {
    /// `DiffScope.wire` — `branch`, `local`, or a full sha.
    var scope: String
    /// The file that was open, if one was.
    var file: String?
    /// The file that was at the top of the screen, when none was open.
    var topFile: String?
    var savedAt: Double

    /// The sha a saved scope names, if it names one.
    ///
    /// `DiffScope.wire` is the protocol's own rule — `branch` and `local` are
    /// names and anything else is a sha, which is what `Session::file_diff`
    /// matches on the other end — so reading it back is the same rule in
    /// reverse, written here rather than at the one call site so the two halves
    /// sit together.
    static func sha(in scope: String) -> String? {
        switch scope {
        case "branch", "local", "staged", "unstaged": return nil
        default: return scope.isEmpty ? nil : scope
        }
    }

    /// Whether this says anything worth offering.
    ///
    /// A bookmark on `branch` with nothing open and nothing scrolled to is the
    /// position everybody starts at, and offering to restore it would be an
    /// interruption that resolves to a no-op.
    var isSomewhere: Bool {
        file != nil || topFile != nil || scope != "branch"
    }
}

/// The bookmarks, one per workspace, in `UserDefaults`.
///
/// A free function over a namespaced key rather than an object: nothing
/// observes a bookmark. It is written when the position changes and read once,
/// when a store is created, and an `ObservableObject` in between would only
/// publish changes nobody is watching.
enum ReviewBookmarks {
    private static func key(_ workspace: String) -> String {
        "changes.position.\(workspace)"
    }

    static func read(_ workspace: String) -> ReviewPosition? {
        guard let data = UserDefaults.standard.data(forKey: key(workspace)) else { return nil }
        return try? JSONDecoder().decode(ReviewPosition.self, from: data)
    }

    static func write(_ position: ReviewPosition, for workspace: String) {
        guard let data = try? JSONEncoder().encode(position) else { return }
        UserDefaults.standard.set(data, forKey: key(workspace))
    }

    static func forget(_ workspace: String) {
        UserDefaults.standard.removeObject(forKey: key(workspace))
    }
}

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
struct ReviewAnchor: Codable, Equatable {
    var file: String
    /// The commit it was written against, when it was written against one.
    /// A comment on the branch as a whole carries no sha, truthfully.
    var commit: String?
    /// The hunk's line range in the new file, when the comment was written on a
    /// hunk rather than on the file as a whole.
    var firstLine: Int?
    var lastLine: Int?
    /// One line of the hunk, so the agent is told WHERE in a 300-line file
    /// rather than only which file.
    var quote: String?

    private static let quoteLimit = 120

    static func quoting(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard trimmed.count > quoteLimit else { return trimmed }
        return String(trimmed.prefix(quoteLimit)) + "…"
    }

    /// How this reads in the app, above the composer and in the outbox.
    var placeDescription: String {
        guard let firstLine else { return "the whole file" }
        guard let lastLine, lastLine > firstLine else { return "line \(firstLine)" }
        return "lines \(firstLine)-\(lastLine)"
    }

    /// How this reads in the message the agent is sent.
    ///
    /// Backticked because the receiver is an agent reading markdown, and a path
    /// in prose is a path it has to guess the boundaries of.
    var promptDescription: String {
        var out = "`\(file)`"
        if let commit { out += " (commit \(commit.prefix(8)))" }
        if firstLine != nil { out += ", around \(placeDescription)" }
        return out
    }
}

/// One thing the reader wants to say, not yet said.
struct ReviewComment: Codable, Equatable, Identifiable {
    var id: UUID = UUID()
    var anchor: ReviewAnchor
    var text: String
    var writtenAt: Double = Date().timeIntervalSince1970
}

/// A batch that went, kept so the app can show WHAT was sent.
///
/// Required rather than nice: `session/prompt` is sent with `request_no_wait`
/// and its response signals end-of-turn, not receipt, so nothing anywhere can
/// confirm that an agent received a prompt. The only honest thing this screen
/// can offer is the text it handed over and the time it did so, which is what
/// this is. See `ReviewCommentQueue.send`.
struct SentReviewBatch: Codable, Equatable, Identifiable {
    var id: UUID = UUID()
    var text: String
    var agentName: String
    var sentAt: Double
    var count: Int
}

// MARK: - The queue

/// An agent pane a review comment can be sent to.
///
/// A value rather than a `Terminal`, so `ChangesView` does not have to hold the
/// `Connection` to know what it can send to — holding it would re-evaluate the
/// diff list's body on every three-second fleet poll, which is the one thing a
/// screen built for scrolling a long patch should not do.
struct ReviewAgentTarget: Identifiable, Equatable {
    var id: String
    var name: String
}

extension Workspace {
    /// The panes in this worktree a review note can be handed to.
    ///
    /// `isAgentPane` OR `canSwitchPaneMode`, because both are the daemon's word
    /// for "an agent is in here" and only the first is about what is currently
    /// DRAWN. A claude the user has flipped back to its raw terminal is still an
    /// agent holding an ACP session, and `terminal.agent_prompt` reaches it;
    /// excluding it would mean a review with nowhere to send to for the sole
    /// reason that somebody wanted to watch the tty.
    ///
    /// Here rather than beside either caller, because there are two now: a
    /// review reached through a `changes` pane (`TerminalView`) and one reached
    /// from the inbox (`NeedsYouView`). Two copies of this filter is two
    /// chances for the same worktree to offer different agents depending on
    /// which door you came through.
    func reviewAgentTargets() -> [ReviewAgentTarget] {
        let numbering = ordinals()
        return terminals
            .filter { $0.isAgentPane || $0.canSwitchPaneMode }
            .map {
                ReviewAgentTarget(id: $0.id, name: $0.displayName(ordinal: numbering[$0.id]))
            }
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
/// Persisted for the reason at the top of this file: the app is very likely
/// terminated between the set in which a comment was written and the set in
/// which it would have been sent, and an unsent comment lost to a process death
/// is worse than no comment feature at all.
@MainActor
final class ReviewCommentQueue: ObservableObject {
    /// Written, not yet sent.
    @Published private(set) var pending: [ReviewComment] = [] { didSet { save() } }

    /// The last few batches that went, newest first.
    ///
    /// Capped at `sentKept`, because this is a receipt and not a history: the
    /// question it answers is "what did I just send", asked within a minute of
    /// sending it.
    @Published private(set) var sent: [SentReviewBatch] = [] { didSet { save() } }

    /// Set while a send is in flight, so the button can say so and cannot be
    /// pressed twice — the one way this app could produce a duplicate prompt on
    /// a protocol that has no way to notice one.
    @Published private(set) var sending = false

    /// A send that did not go, in words worth reading.
    ///
    /// NOT cleared by anything on a timer and never acted on automatically.
    /// There is no acknowledgment on this path — see `SentReviewBatch` — so an
    /// automatic retry would be the app deciding, with no evidence, that a
    /// prompt which may well have arrived should be delivered a second time.
    /// The comments stay in `pending` and the reader is the one who decides.
    ///
    /// `ChangesStore.Trouble`, not a second declaration of the same two
    /// fields: this queue and that store are one screen, and the sentence and
    /// the runner's words have to stay apart here for the same reason they do
    /// there.
    @Published var failure: ChangesStore.Trouble?

    private static let sentKept = 5

    private let core: ClientCore
    private let workspace: String
    private var loading = true

    init(core: ClientCore, workspace: String) {
        self.core = core
        self.workspace = workspace
        if let data = UserDefaults.standard.data(forKey: Self.key(workspace)),
            let stored = try? JSONDecoder().decode(Stored.self, from: data)
        {
            pending = stored.pending
            sent = stored.sent
        }
        loading = false
    }

    func add(_ comment: ReviewComment) {
        pending.append(comment)
        // A new comment is evidence the last failure has been read and moved
        // past. Left up, it would sit above a queue it no longer describes.
        failure = nil
    }

    func remove(_ comment: ReviewComment) {
        pending.removeAll { $0.id == comment.id }
    }

    func clearPending() {
        pending = []
    }

    /// Everything queued, as one message.
    ///
    /// Numbered and grouped in the order they were written, which is reading
    /// order — the order the reader went through the diff in, and therefore the
    /// order in which the notes make sense to each other.
    func message(branch: String) -> String {
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
    /// The call is `terminal.agent_prompt`, the same one `AgentStream.send`
    /// makes, so a comment batch arrives in the transcript exactly as a typed
    /// message does — there is no separate "review comment" channel to keep in
    /// step, and the agent's own history shows what it was told.
    ///
    /// On failure the comments STAY in `pending` and nothing is retried. The
    /// reader can press Try Again, and that is the only thing that ever sends
    /// this batch a second time: `request_no_wait` means a failure here is
    /// "this client did not get an answer", which is not the same as "the agent
    /// did not get the prompt", and only a person can weigh the difference.
    func send(to target: ReviewAgentTarget, branch: String) async {
        guard !pending.isEmpty, !sending else { return }
        let text = message(branch: branch)
        let count = pending.count
        sending = true
        defer { sending = false }
        do {
            _ = try await core.call(
                "terminal.agent_prompt", ["terminal": target.id, "text": text])
            // Recorded BEFORE the queue is emptied, so a crash between the two
            // loses the receipt rather than the comments.
            sent.insert(
                SentReviewBatch(
                    text: text, agentName: target.name,
                    sentAt: Date().timeIntervalSince1970, count: count),
                at: 0)
            if sent.count > Self.sentKept { sent = Array(sent.prefix(Self.sentKept)) }
            pending = []
            failure = nil
        } catch {
            failure = Self.trouble(for: error)
        }
    }

    /// The core's answer, as something worth putting on a phone screen.
    ///
    /// Never the raw error AS the sentence: what the FFI hands back is a Rust
    /// word for an empty session slot wrapped in Swift enum syntax, and the
    /// person reading it wanted to send a sentence to an agent. That was never
    /// a reason to drop it, though, and this used to — the first two arms know
    /// what happened and say so, and the third knew nothing and said so in a
    /// sentence built out of a string it then threw away. It travels in the
    /// box now, and only on the arm with nothing written about it: under
    /// either of the other two it would be a transcript beneath a sentence
    /// that already names the cause and what to do next.
    private static func trouble(for error: Error) -> ChangesStore.Trouble {
        if let core = error as? ClientCore.CoreError, case .disconnected = core {
            return ChangesStore.Trouble(
                sentence: "The connection to this runner dropped, so these are still here. "
                    + "Try again once it’s back.")
        }
        let text = error.localizedDescription.lowercased()
        if text.contains("not found") || text.contains("unknown method") {
            return ChangesStore.Trouble(
                sentence:
                    "That pane isn’t running an agent anymore, so there was nothing to send to.")
        }
        return ChangesStore.Trouble(
            sentence: "Couldn’t send these. They’re still here.",
            transcript: error.localizedDescription)
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
        UserDefaults.standard.set(data, forKey: Self.key(workspace))
    }
}
