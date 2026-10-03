import Foundation
import os

/// What a click asked a runner to do, said in the banner's own words.
///
/// The verb is half of an action's key (`ActionKey`), so a second Stop of the
/// same terminal replaces the first one's result and a Restart beside it does
/// not; and it is the start of the sentence a failure is shown as
/// (`ActionCopy.sentence`).
enum ActionVerb: Hashable {
    case stop
    case restart
    /// Stop, then remove the record: closing a terminal.
    case close
    case dismissLost
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
        case .hide: "Couldn’t hide \(subject)."
        case .unhide: "Couldn’t show \(subject) again."
        case .reorder: "Couldn’t reorder the worktrees beside \(subject)."
        case .removeWorktree: "Couldn’t remove \(subject)."
        case .arrange: "Couldn’t change the layout in \(subject)."
        case .switchMode: "Couldn’t switch \(subject) between terminal and chat."
        case .newTerminal: "Couldn’t open a terminal in \(subject)."
        case .openChanges: "Couldn’t open the changes in \(subject)."
        case .resumeBranch: "Couldn’t pick up \(subject)."
        case .notice: ""
        }
    }
}

/// One action on one target on one runner: what a result is filed under.
///
/// A later result under the same key replaces the earlier one, and nothing
/// else does. So a refused Stop stays up through the refresh after it,
/// through a focus that worked, and through a Stop of a different terminal;
/// it goes when it is dismissed or when that terminal is stopped again.
struct ActionKey: Hashable {
    let verb: ActionVerb
    let host: String
    /// The id of what was acted on: a terminal, a worktree, a branch.
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
    /// What is on screen, oldest first.
    @Published private(set) var shown: [ActionFailure] = []

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
    /// which takes down whatever that key showed before.
    func settle(_ key: ActionKey, failure: String?) {
        guard let failure else {
            shown.removeAll { $0.key == key }
            return
        }
        let entry = ActionFailure(key: key, sentence: failure)
        if let index = shown.firstIndex(where: { $0.key == key }) {
            shown[index] = entry
        } else {
            shown.append(entry)
        }
    }

    func dismiss(_ key: ActionKey) {
        shown.removeAll { $0.key == key }
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
        log.error("\(String(describing: verb), privacy: .public) \(subject, privacy: .public) failed: \(message, privacy: .public)")
        return "\(verb.lead(subject)) \(reason(message))"
    }

    /// Why, by the `code:` word. A failure with no word never got the
    /// daemon's answer: the runner wasn't reached, or the CLI refused the
    /// line before sending it.
    static func reason(_ message: String) -> String {
        switch TaskFailure.code(in: message) {
        case "not-found":
            "It isn’t on this runner anymore."
        case "running-processes":
            "Something is still running in it. Stop it first, then try again."
        case "tmux-unavailable":
            "The runner can’t reach tmux. Install tmux there, then try again."
        case "capability-unsupported", "version-incompatible":
            "This runner’s Far Cooler is too old for this. Update it there, then try again."
        case "scope-denied":
            "This runner lets Far Cooler see it but not change it."
        case "auth-required":
            "This runner didn’t accept Far Cooler’s sign-in."
        case "resource-conflict":
            "It changed while you were doing that. Try again."
        case "host-offline":
            "The runner is offline."
        case "agent-not-connected":
            "Its agent isn’t connected right now."
        case "operation-failed":
            "The runner tried and it didn’t work. Try again."
        case .some:
            "The runner couldn’t take the request as Far Cooler sent it. That’s a problem in the app, not in anything you did."
        case nil:
            "Check that the runner is reachable, then try again."
        }
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
            log.error("refused, runner unreachable: \(why, privacy: .public)")
            return "That runner can’t be reached right now. Far Cooler will keep trying."
        case .connected, .connecting, .reconnecting:
            return "That runner isn’t ready yet. Try again in a moment."
        }
    }
}
