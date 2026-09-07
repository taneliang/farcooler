import Foundation

/// Why a runner said no to one request, and what a phone says about it.
///
/// **The runner sends a stable machine word; this file owns the sentence.** The
/// same rule `AgentFailure` follows for a pane with no agent in it and
/// `AdapterTestOutcome` follows for the Test button. The word is one of the
/// `ErrorCode` names in `proto/farcooler.proto`, spelled in kebab by
/// `farcooler_core::error::word`, carried across the FFI on the answer line as
/// `code`, and read off `ClientCore.CoreError` here.
///
/// **Why this is not `RunnerTrouble`.** That type is a different axis and its
/// inputs prove it: it is built from a connect-time message by substring, it
/// asks for a runner's name, address and port to say anything, and every next
/// move it offers is about keys and authorization. It answers "we could not
/// reach this runner at all". This answers "we reached it, it understood, and
/// it refused" — a live session, one control, and next moves that are mostly
/// things to do on the runner rather than buttons on a screen. Folding a
/// code-keyed table into a message-keyed enum would give one type two
/// constructors that can never both be right.
///
/// **The next move is IN the sentence, not beside it.** `RunnerTrouble` can
/// hand back a `NextMove` because both of its screens draw the same control in
/// the same place. These refusals arrive at a sheet, a full-pane state, an
/// inline notice and a red banner — four containers with nothing in common to
/// hang a button on — and the move that actually helps is usually not a button
/// at all ("install tmux there", "stop what's running first"). A `NextMove`
/// enum nothing could draw would be coverage with no reader.
///
/// **Only codes that can actually arrive are here.** Of the twenty-eight in the
/// proto, twelve cannot reach a phone at all: `host-offline` has no
/// `DomainError` variant to produce it, eight more have no production site
/// anywhere in the tree, `version-incompatible` is intercepted at the handshake
/// and arrives as `SessionError::VersionMismatch`, `client-too-slow` is raised
/// on the local send and never put in a response, and `idempotency-mismatch`
/// has no caller outside the store's own tests because no request path ever
/// sets an idempotency key. `confirmation-required` is real but the client core
/// turns both of its remove paths into structured outcomes before an app sees
/// them, and `operation-failed` is the generic — it is the caller's own
/// sentence, which is what `trouble(forWord:message:otherwise:)` falls back to.
/// Writing sentences for those would be copy nothing can ever show.
public enum RunnerRefusal: String, CaseIterable, Sendable {
    /// This device's access was withdrawn while a request was in flight.
    case authRequired = "auth-required"
    /// This device was enrolled to look, not to change things.
    case scopeDenied = "scope-denied"
    /// The runner is older than the feature being asked for.
    case capabilityUnsupported = "capability-unsupported"
    /// Whatever was being acted on is not there anymore.
    case notFound = "not-found"
    /// The runner has no tmux, which is what it runs everything in.
    case tmuxUnavailable = "tmux-unavailable"
    /// A directory of that name is already on disk.
    case worktreeExists = "worktree-exists"
    /// A branch of that name is already in the repository.
    case branchExists = "branch-exists"
    /// Panes are still alive under the thing being removed.
    case runningProcesses = "running-processes"
    /// Nothing is running, but records would be orphaned.
    case workspacesExist = "workspaces-exist"
    /// Outside every allowlisted root, or overlapping one.
    case pathNotAllowed = "path-not-allowed"
    /// A whole home directory or a system path, which never becomes allowable.
    case sensitiveRoot = "sensitive-root"
    /// No base to compare this branch against.
    case baseUnresolvable = "base-unresolvable"
    /// Somebody else moved it first.
    case resourceConflict = "resource-conflict"
    /// The request itself was malformed. Ours to fix, not the reader's.
    case invalidArgument = "invalid-argument"

    /// What Far Cooler says happened, and the one thing worth doing about it.
    ///
    /// Never quotes the machine word and never paraphrases the core's own
    /// `Display` text, which is written for whoever is reading a daemon log —
    /// "resource version is stale", "invalid argument: idempotency_key" — and
    /// is exactly what these exist to keep off a screen.
    ///
    /// No cause is invented where none is known, and no retry is promised that
    /// a screen cannot make good on: the same restraint `RunnerTrouble.detail`
    /// and `AgentFailureCopy` keep, and the reason `resource-conflict` says
    /// "take another look" rather than offering to redo it.
    public var sentence: String {
        switch self {
        case .authRequired:
            "This device’s access to this runner was withdrawn. Add it again to get it back."
        case .scopeDenied:
            "This device can only look at this runner. Changing anything needs control, which is "
                + "granted from a device that already has it."
        case .capabilityUnsupported:
            "This runner’s Far Cooler is too old for this. Update it there, then try again."
        case .notFound:
            "It isn’t on the runner anymore. Something else removed it while this was open."
        case .tmuxUnavailable:
            "The runner can’t reach tmux. Far Cooler runs every pane inside it, so install tmux "
                + "there and try again."
        case .worktreeExists:
            "There’s already a folder with that name on the runner. Pick another name."
        case .branchExists:
            "That branch already exists. Pick another name, or resume the branch you have."
        case .runningProcesses:
            "Something is still running there. Stop it first, then try this again."
        case .workspacesExist:
            "Its workspaces are still here. Remove those first, then remove the folder."
        case .pathNotAllowed:
            "That folder isn’t inside one you’ve added, or it overlaps one you already have."
        case .sensitiveRoot:
            "That’s a whole home folder or a system folder, and it can never be added. Pick a "
                + "folder inside it."
        case .baseUnresolvable:
            "Can’t find the branch this work is based on. Pick a base to compare against."
        case .resourceConflict:
            "Something else changed this first. Take another look and try again."
        case .invalidArgument:
            "This runner couldn’t make sense of what Far Cooler asked for. That’s a problem in "
                + "the app, not in anything you typed."
        }
    }

    /// The failure to put on a screen, given the word a runner sent.
    ///
    /// One function rather than an optional every caller unwraps, because the
    /// unwrapping is where the bug lives. A caller that writes `if let refusal
    /// = …` has, by construction, an `else` branch it must remember to fill —
    /// and the branch it would forget is the one a runner NEWER than this build
    /// takes, which is the failure that shipped in the agent-failure work and
    /// had to be fixed on macOS afterwards. Here the unknown word is not a
    /// branch anybody writes: it is the default, and it is the caller's own
    /// generic sentence with the runner's words in the box beneath it, which is
    /// exactly what every one of these screens already showed.
    ///
    /// So an unknown code degrades to a generic failure and never to silence,
    /// and `nil`/empty does too — that is a link that dropped or an argument
    /// this app refused before sending, and no runner refused anything.
    ///
    /// `transcript` is dropped wherever we have a diagnosis of our own. The
    /// same scoping `RunnerTrouble.showsTheRunnersOwnWords` uses and for the
    /// same reason: a log line under a sentence that already names the cause
    /// and the fix is noise. Nothing is hidden that anybody needs — the words
    /// under these fourteen are the core's own `Display`, which says less than
    /// the sentence above them does.
    public static func trouble(
        forWord word: String?,
        message: String,
        otherwise generic: String
    ) -> ReviewTrouble {
        guard let refusal = known(word) else {
            return ReviewTrouble(sentence: generic, transcript: message)
        }
        return ReviewTrouble(sentence: refusal.sentence, transcript: nil)
    }

    /// The same, with a sentence of the caller's own in front of it.
    ///
    /// For the screens whose failure has a STEP as well as a reason — "Created
    /// the worktree, but couldn't start Claude." — where dropping the step
    /// would lose the half that says how much of the job got done. Two
    /// sentences, both this app's, which is a different thing from splicing our
    /// prose onto a runner's: nothing here is quoted from the core.
    ///
    /// A refusal this build cannot read leaves the step's sentence standing
    /// alone with the runner's words in the box, which is exactly what these
    /// screens showed before.
    public static func trouble(
        forWord word: String?,
        message: String,
        after context: String
    ) -> ReviewTrouble {
        guard let refusal = known(word) else {
            return ReviewTrouble(sentence: context, transcript: message)
        }
        return ReviewTrouble(sentence: context + " " + refusal.sentence, transcript: nil)
    }

    /// A word this build has a sentence for, or nil for every other input —
    /// absent, empty, the generic, a code that cannot reach a phone, and a code
    /// from a runner newer than this build.
    private static func known(_ word: String?) -> RunnerRefusal? {
        guard let word, !word.isEmpty else { return nil }
        return RunnerRefusal(rawValue: word)
    }
}
