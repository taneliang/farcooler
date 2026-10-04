import Foundation
import os

/// What a click asked a runner to do, said in the banner's own words.
///
/// The verb is half of an action's key (`ActionKey`), so a second Stop of the
/// same terminal replaces the first one's result and a Hide of a worktree
/// beside it does not; and it is the start of the sentence a failure is shown
/// as (`ActionCopy.sentence`).
enum ActionVerb: Hashable {
    case stop
    case restart
    /// Stop, then remove the record: closing a terminal.
    case close
    case dismissLost
    /// Name a terminal.
    case rename
    case hide
    case unhide
    case reorder
    /// Remove a worktree. Its sheet shows the runner's refusals itself; only
    /// a runner already known not to answer reaches a banner.
    case removeWorktree
    /// Every layout command: focus, zoom, split, move, resize, select.
    case arrange
    case switchMode
    case newTerminal
    case openChanges
    case resumeBranch
    /// Give a worktree to another workspace.
    case move
    case startOrchestrator
    case wakeOnAnswer
    /// Use as Orchestrator, and Stop Being Orchestrator.
    case setRole
    /// A sentence the app wrote for something that isn't one action on one
    /// target. One slot, cleared on navigation (`ActionOutcomes.clearNotice`).
    case notice

    /// The first sentence of a failure: what didn't happen, to `subject`.
    func lead(_ subject: String) -> String {
        switch self {
        case .stop: "Couldn’t stop \(subject)."
        case .restart: "Couldn’t restart \(subject)."
        case .close: "Couldn’t close \(subject)."
        case .dismissLost: "Couldn’t dismiss \(subject)."
        case .rename: "Couldn’t rename \(subject)."
        case .hide: "Couldn’t hide \(subject)."
        case .unhide: "Couldn’t show \(subject) again."
        case .reorder: "Couldn’t reorder the worktrees beside \(subject)."
        case .removeWorktree: "Couldn’t remove \(subject)."
        case .arrange: "Couldn’t change the layout in \(subject)."
        case .switchMode: "Couldn’t switch \(subject) between terminal and chat."
        case .newTerminal: "Couldn’t open a terminal in \(subject)."
        case .openChanges: "Couldn’t open the changes in \(subject)."
        case .resumeBranch: "Couldn’t pick up \(subject)."
        case .move: "Couldn’t move \(subject)."
        case .startOrchestrator: "Couldn’t start the orchestrator for \(subject)."
        case .wakeOnAnswer: "Couldn’t change Wake the Agent When You Answer for \(subject)."
        case .setRole: "Couldn’t change what \(subject) is."
        case .notice: ""
        }
    }

    /// What `target` is the id of, so a result can go when its target does
    /// (`ActionOutcomes.prune`).
    enum Target { case terminal, worktree, workspace, other }

    var target: Target {
        switch self {
        case .stop, .restart, .close, .dismissLost, .rename, .switchMode, .setRole: .terminal
        case .hide, .unhide, .reorder, .removeWorktree, .arrange, .newTerminal, .openChanges, .move: .worktree
        case .startOrchestrator, .wakeOnAnswer: .workspace
        case .resumeBranch, .notice: .other
        }
    }

    /// Verbs that answer one question about their target, so one that
    /// worked settles the others: a Close that worked means a Stop refused
    /// a moment before is no longer the state of anything.
    var family: Set<ActionVerb> {
        switch self {
        case .stop, .restart, .close, .dismissLost: [.stop, .restart, .close, .dismissLost]
        case .hide, .unhide: [.hide, .unhide]
        default: [self]
        }
    }
}

/// One action on one target on one runner: what a result is filed under.
///
/// A later result under the same key replaces the earlier one, and so does a
/// success of its verb's family (`ActionVerb.family`) on the same target.
/// Nothing else does, except the target leaving the fleet. So a refused Stop
/// stays up through the refresh after it, through a focus that worked, and
/// through a Stop of a different terminal.
struct ActionKey: Hashable {
    let verb: ActionVerb
    let host: String
    /// The id of what was acted on: a terminal, a worktree, a workspace, a
    /// branch.
    let target: String

    static let notice = ActionKey(verb: .notice, host: "", target: "")
}

/// A failed action, as the banner shows it.
struct ActionFailure: Identifiable, Equatable {
    let key: ActionKey
    let sentence: String
    var id: ActionKey { key }
}

/// The failures of the commands one action ran, and only that action's.
///
/// Set as a task-local (`ActionReporting.current`) for the length of the action by
/// `ActionOutcomes.perform`, and written by `DaemonClient.run` when a command
/// it runs in the foreground fails. That is what makes the result the
/// action's own: a background poll is not inside any action and writes
/// nothing, and a second action running at the same time has a report of its
/// own. The shared `lastError` this replaces was written by every command and
/// cleared by every successful refresh, so a refused Stop vanished when the
/// refresh after it worked, and a focus that worked showed whatever a
/// background read had failed with a minute before.
@MainActor
final class ActionReport {
    /// The first failure's raw message. The first, because it is the one the
    /// rest followed from: a Close whose stop was refused fails its remove
    /// too, and the remove's words are about the wrong thing.
    private(set) var failure: String?
    /// Set when the action has been settled. A task spawned inside it
    /// inherits the task-local and may finish later; what it says then is
    /// about nothing on screen, so it is dropped.
    fileprivate var closed = false

    /// Note a command's failure against the action running now, if any.
    static func note(_ message: String) {
        guard let report = ActionReporting.current, !report.closed else { return }
        if report.failure == nil { report.failure = message }
    }
}

/// Where the running action's report is found: a task-local, so it is the
/// action's own task's and a concurrent one's is another.
enum ActionReporting {
    @TaskLocal static var current: ActionReport?
}

/// Each action's result, for the banner over the detail pane.
@MainActor
final class ActionOutcomes: ObservableObject {
    /// How many banners show at once. More would cover the pane they're
    /// about; the rest wait behind a Show All.
    static let visibleLimit = 3

    /// Every result filed, oldest first.
    @Published private(set) var shown: [ActionFailure] = []
    /// Whether every result is on screen rather than the latest few.
    @Published var expanded = false

    /// What the banners draw: the latest `visibleLimit`, or all of them.
    var visible: [ActionFailure] {
        expanded ? shown : Array(shown.suffix(Self.visibleLimit))
    }

    /// How many results `visible` leaves out.
    var hiddenCount: Int { shown.count - visible.count }

    /// Run `body` as the action `key` against `client`, and file its result.
    ///
    /// `subject` is how the sentence names the target: a terminal's label, a
    /// worktree's name in quotes.
    func perform<T>(
        _ key: ActionKey, subject: String, on client: DaemonClient,
        _ body: (DaemonClient) async -> T
    ) async -> T {
        let report = ActionReport()
        let result = await ActionReporting.$current.withValue(report) { await body(client) }
        report.closed = true
        settle(key, failure: report.failure.map { ActionCopy.sentence(key.verb, subject: subject, message: $0) })
        return result
    }

    /// File `key`'s result: a sentence to show, or nil for one that worked,
    /// which takes down whatever that key, and its verb's family on the same
    /// target, showed before. A new failure goes last, where the latest is.
    func settle(_ key: ActionKey, failure: String?) {
        guard let failure else {
            shown.removeAll {
                $0.key.host == key.host && $0.key.target == key.target && key.verb.family.contains($0.key.verb)
            }
            return
        }
        shown.removeAll { $0.key == key }
        shown.append(ActionFailure(key: key, sentence: failure))
    }

    func dismiss(_ key: ActionKey) {
        shown.removeAll { $0.key == key }
        if shown.count <= Self.visibleLimit { expanded = false }
    }

    func dismissAll() {
        shown.removeAll()
        expanded = false
    }

    /// Drop the results about things no longer there: a terminal that was
    /// closed from tmux or another client, a worktree removed. `exists` is
    /// asked of each result whose target is one of those, by kind.
    func prune(_ exists: (ActionVerb.Target, ActionKey) -> Bool) {
        let kept = shown.filter { failure in
            let kind = failure.key.verb.target
            return kind == .other || exists(kind, failure.key)
        }
        if kept != shown { shown = kept }
    }

    /// The app's own sentence for something that isn't one action's result.
    var notice: String? {
        get { shown.first { $0.key == .notice }?.sentence }
        set { settle(.notice, failure: newValue) }
    }

    /// Navigation takes the notice down: it described the pane you left.
    /// Action results stay; each names what it was about.
    func clearNotice() { dismiss(.notice) }
}

/// The words an action's failure is shown in.
///
/// **The runner's words are not the sentence.** The CLI's stderr is ssh's
/// or the daemon's text, written for a log. Under `--json` it carries a
/// `code:` line, the stable word of `farcooler_core::error::word`, and that
/// is what the reason is chosen by. The raw text goes to the log, where a
/// person debugging the runner can find it, and never to the screen.
enum ActionCopy {
    private static let log = Logger(subsystem: "com.farcooler.FarCooler", category: "actions")

    /// The banner's sentence for `verb` on `subject` failing with `message`,
    /// the CLI's stderr. Logs `message`.
    static func sentence(_ verb: ActionVerb, subject: String, message: String) -> String {
        log.error("\(String(describing: verb), privacy: .public) \(subject, privacy: .private) failed: \(message, privacy: .private)")
        return "\(verb.lead(subject)) \(reason(message))"
    }

    /// Said for a failure nothing more can be said about.
    static let neutral = "Something went wrong. Try again."

    /// Why, by the `code:` word: every word `farcooler_core::error::word`
    /// has. A word this build doesn't know (a newer runner's) is `neutral`.
    /// No word at all is the CLI or ssh failing before the daemon answered,
    /// and only a connection failure is said to be one (`isConnection`).
    static func reason(_ message: String) -> String {
        switch TaskFailure.code(in: message) {
        case "not-found":
            "It isn’t on this runner anymore."
        case "running-processes":
            "Something is still running in it. Stop it first, then try again."
        case "dirty-worktree":
            "It has changes that aren’t committed. Commit or discard them, then try again."
        case "branch-exists":
            "A branch with that name already exists on this runner."
        case "worktree-exists":
            "A worktree with that name already exists on this runner."
        case "repository-locked":
            "Git is busy with something else in this repository. Try again when it’s done."
        case "workspaces-exist":
            "It still has worktrees. Remove them first, then try again."
        case "path-not-allowed":
            "That folder isn’t one this runner lets Far Cooler use."
        case "sensitive-root":
            "Far Cooler won’t use that folder because it holds personal or system files."
        case "base-unresolvable":
            "The runner couldn’t find the branch to start from."
        case "confirmation-required":
            "The runner needs you to confirm this first."
        case "tmux-unavailable":
            "The runner can’t reach tmux. Install tmux there, then try again."
        case "capability-unsupported":
            "This runner’s Far Cooler is too old for this. Update it there, then try again."
        case "version-incompatible":
            "This runner’s Far Cooler and this app are different versions. Update the older one, then try again."
        case "scope-denied":
            "This runner lets Far Cooler see it but not change it."
        case "auth-required":
            "This runner didn’t accept Far Cooler’s sign-in."
        case "resource-conflict":
            // `not_lost` is a Dismiss that came after the terminal stopped
            // being lost: the runner named it, so retrying is no help.
            TaskFailure.what(in: message) == "not_lost"
                ? "It was already restarted or dismissed."
                : "It changed while you were doing that. Try again."
        case "host-offline":
            "The runner is offline."
        case "agent-not-connected":
            "Its agent isn’t connected right now."
        case "agent-stopped":
            "Its agent stopped. Restart it, then try again."
        case "attachment-limit":
            "That’s more than the runner takes at once."
        case "diff-too-large":
            "The changes are too large to show."
        case "diff-unsupported":
            "The runner can’t show these changes."
        case "pr-state-unavailable":
            "The runner couldn’t read the pull request."
        case "dispatch-unknown":
            "The runner didn’t say whether it went through. Check before trying again."
        case "output-gap", "client-too-slow":
            "The runner fell behind. Try again."
        case "operation-failed":
            "The runner tried and it didn’t work. Try again."
        case "invalid-argument", "idempotency-mismatch":
            "The runner couldn’t take the request as Far Cooler sent it. That’s a problem in the app, not in anything you did."
        case .some:
            neutral
        case nil:
            isConnection(message) ? "Check that the runner is reachable, then try again." : neutral
        }
    }

    /// Whether stderr with no `code:` is the link failing: ssh's own words,
    /// or the transport's (`farcooler_transport::ClientError`).
    static func isConnection(_ message: String) -> Bool {
        let lowered = message.lowercased()
        return [
            "ssh:", "connection refused", "timed out", "could not resolve hostname", "no route to host",
            "network is unreachable", "host is down", "connection reset", "connection closed", "broken pipe",
            "kex_exchange_identification", "permission denied (publickey", "could not reach the daemon",
            "the daemon closed the connection", "peer closed",
        ].contains { lowered.contains($0) }
    }

    /// Why nothing was sent: the runner was already known not to answer.
    /// `HostState.refusal` is mostly the stderr of whatever last failed, so
    /// it goes to the log and this goes on screen.
    static func refused(_ state: HostState?) -> String {
        switch state {
        case nil:
            return "That runner isn’t set up anymore."
        case .notInstalled:
            return "Far Cooler isn’t installed on that runner yet."
        case .unreachable(let why):
            log.error("refused, runner unreachable: \(why, privacy: .private)")
            return "That runner can’t be reached right now. Far Cooler will keep trying."
        case .connected, .connecting, .reconnecting:
            return "That runner isn’t ready yet. Try again in a moment."
        }
    }
}

extension ActionOutcomes {
    /// `prune`, against the merged fleet: a result goes once the terminal,
    /// worktree or workspace it names isn't listed on its runner. A runner
    /// without workspaces lists none, and keeps its workspace results.
    func prune(in fleet: Fleet) {
        prune { kind, key in
            switch kind {
            case .terminal:
                return fleet.worktrees.contains {
                    ($0.host ?? "") == key.host && $0.terminals.contains { $0.id == key.target }
                }
            case .worktree:
                return fleet.worktrees.contains { ($0.host ?? "") == key.host && $0.id == key.target }
            case .workspace:
                guard let listed = fleet.runnerWorkspaces[key.host] else { return true }
                return listed.contains { $0.id == key.target }
            case .other:
                return true
            }
        }
    }
}
