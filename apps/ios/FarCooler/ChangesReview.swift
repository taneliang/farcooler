import Foundation
import SwiftUI

// Where you were, and where what you want to say goes.
//
// Both outlive the process, and for the same reason. `ChangesStore` and
// everything else in `Changes.swift` hangs off `Connection`, which is torn down
// whenever iOS decides the app has been in the background long enough — and the
// situation this review surface is built for is a dozen ninety-second windows
// across an hour, so that happens roughly a dozen times per session. Anything
// held only in memory is therefore held only until the next set.
//
// So a bookmark lives in `UserDefaults`, the same place `RunnerStore` keeps the
// runners and the same place `ReviewCommentQueue` keeps unsent notes: none of
// it is secret, none of it is large, and the one thing on this screen that IS
// secret — the diff itself — is deliberately not written down anywhere. What is
// stored is a path, a sha, and whatever the reader typed, per workspace.
//
// The queue itself is no longer declared here. It is in
// `AgentKit/ReviewComments.swift` now, because the Mac's diff pane grew the
// same feature and two copies of a comment queue is two ways for one worktree's
// notes to be filed under two different keys. What is still here is the half
// that is this app's: the bookmark, the pane filter, and the FFI call the queue
// is handed.
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

// MARK: - The comment machinery, which is shared now

// `ReviewAnchor`, `ReviewComment`, `SentReviewBatch`, `ReviewAgentTarget` and
// `ReviewCommentQueue` were declared here and now live in
// `AgentKit/ReviewComments.swift`, which the Mac's diff pane compiles too. The
// reasoning moved with them, unchanged: why a comment is anchored, why notes
// are batched rather than sent one at a time, why the queue writes itself down
// on every change, and why a failed send is never retried on its own.
//
// The one thing that could not move is the send. This app calls
// `terminal.agent_prompt` through the FFI and the Mac shells out to the CLI, so
// the queue takes a closure — see `ReviewCommentQueue.phone` at the bottom of
// this file, which is this app's half of it.
//
// What stayed above: the bookmark, which is about a process iOS terminates
// between ninety-second windows and has no meaning on a desktop.

// MARK: - Where a note can be sent

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
    ///
    /// Not shared with the Mac, though `ReviewAgentTarget` is. `Workspace` and
    /// `Terminal` are declared once per app and the two do not agree here: a
    /// Mac's `canSwitchPaneMode` excludes a pane showing a diff, because a Mac
    /// can put one there and the daemon refuses to switch it. One filter over
    /// two different meanings of a word would be worse than two filters.
    func reviewAgentTargets() -> [ReviewAgentTarget] {
        let numbering = ordinals()
        return terminals
            .filter { $0.isAgentPane || $0.canSwitchPaneMode }
            .map {
                ReviewAgentTarget(
                    id: $0.id, name: $0.displayName(ordinal: numbering[$0.id]),
                    // Read by the Mac only, where a chat on screen is a
                    // composer a note can be dropped into. Filled in here
                    // anyway: a field one platform leaves at its default is a
                    // field that is wrong the first time this app wants it.
                    showsChat: $0.isAgentPane)
            }
    }
}

// MARK: - This app's half of the send

extension ReviewCommentQueue {
    /// The queue as this app builds it: the send is `terminal.agent_prompt`
    /// over the FFI, the same call `AgentStream.send` makes, so a comment batch
    /// arrives in the transcript exactly as a typed message does.
    static func phone(core: ClientCore, workspace: String) -> ReviewCommentQueue {
        ReviewCommentQueue(workspace: workspace) { target, text in
            do {
                _ = try await core.call(
                    "terminal.agent_prompt", ["terminal": target.id, "text": text])
                return nil
            } catch {
                return trouble(for: error)
            }
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
    ///
    /// Here rather than in `AgentKit` because `ClientCore.CoreError` is this
    /// app's, and because the CLI the Mac runs fails in its own words.
    private static func trouble(for error: Error) -> ReviewTrouble {
        if let core = error as? ClientCore.CoreError, case .disconnected = core {
            return ReviewTrouble(
                sentence: "The connection to this runner dropped, so these are still here. "
                    + "Try again once it\u{2019}s back.")
        }
        let text = error.localizedDescription.lowercased()
        if text.contains("not found") || text.contains("unknown method") {
            return ReviewTrouble(
                sentence:
                    "That pane isn\u{2019}t running an agent anymore, so there was nothing to send to.")
        }
        return ReviewTrouble(
            sentence: "Couldn\u{2019}t send these. They\u{2019}re still here.",
            transcript: error.localizedDescription)
    }
}
