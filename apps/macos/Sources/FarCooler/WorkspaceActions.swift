import AgentKit
import Foundation

// What a workspace offers from the sidebar: its header's menu, starting its
// orchestrator from the "No orchestrator" row, and opening its charter.
//
// Worked out here as values, and drawn by `WorkspaceHeader` and
// `OrchestratorRow`, so the rules are the ones `WorkspaceActionsTests` pins.

/// What a workspace header's menu offers, in order.
enum WorkspaceMenu {
    enum Item: Hashable {
        case showBoard, startOrchestrator, replaceOrchestrator, showCharter

        var title: String {
            switch self {
            case .showBoard: return "Show Board"
            case .startOrchestrator: return "Start Orchestrator"
            case .replaceOrchestrator: return "Replace Orchestrator"
            case .showCharter: return "Show Charter"
            }
        }
    }

    /// Show Board only where the sidebar draws a Board row (a runner with
    /// `tasks`). Start or Replace, never both: a workspace has at most one
    /// orchestrator, and the runner refuses a second start without
    /// `--replace`. Show Charter always, disabled when this Mac can't open it
    /// (`CharterAccess`), so the item says why rather than vanishing.
    static func items(hasBoard: Bool, hasOrchestrator: Bool) -> [Item] {
        (hasBoard ? [.showBoard] : [])
            + [hasOrchestrator ? .replaceOrchestrator : .startOrchestrator, .showCharter]
    }
}

/// The harnesses an orchestrator can run: what `--harness` takes.
enum OrchestratorHarness: String, CaseIterable, Identifiable {
    case claude, codex, cursor
    var id: String { rawValue }

    var title: String {
        switch self {
        case .claude: return "Claude"
        case .codex: return "Codex"
        case .cursor: return "Cursor"
        }
    }
}

/// Whether Show Charter can open a workspace's charter on this Mac.
enum CharterAccess: Equatable {
    /// The charter's file, to hand to the default editor.
    case open(URL)
    /// Why not, for the disabled item's tooltip.
    case unavailable(String)

    /// The path is on the runner's own disk, sent to `host_admin` clients
    /// only. So it opens only when the runner is this Mac (`host` empty) and
    /// the runner said where it is.
    static func of(
        _ workspace: WorkspaceSummary, host: String,
        exists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
    ) -> CharterAccess {
        guard host.isEmpty else {
            return .unavailable("This charter is on \(host), so it can’t be opened on this Mac.")
        }
        guard let path = workspace.charter else {
            return .unavailable("This runner didn’t say where the charter is. Updating Far Cooler on it may help.")
        }
        // The runner sends the path whether or not the file is there: Main
        // has none until the repository has a manager file, and a new
        // workspace's is the orchestrator's to write. Not made here — an
        // empty charter would skip the orchestrator's interview.
        guard exists(path) else {
            return .unavailable("No charter yet. The orchestrator writes one when you first talk to it.")
        }
        return .open(URL(fileURLWithPath: path))
    }
}

extension DaemonClient {
    /// `farcooler workspace start-orchestrator`, by the workspace's id,
    /// which the CLI takes as it takes a prefix; a name could be another
    /// workspace's. `--json` for the `code:` line a refusal carries.
    static func startOrchestratorArguments(
        _ workspace: WorkspaceSummary, harness: OrchestratorHarness, replace: Bool
    ) -> [String] {
        ["workspace", "start-orchestrator", workspace.id, "--harness", harness.rawValue]
            + (replace ? ["--replace"] : []) + ["--json"]
    }

    /// Why an orchestrator didn't start, by the `code:` word on the CLI's
    /// stderr, in this app's words. An `invalid-argument` is told apart by the
    /// CLI's sentence for it, since the code line carries no finer word: a
    /// seat already taken (a start raced another), or a workspace folder the
    /// runner couldn't make. A failure with no word never reached the
    /// daemon's answer.
    static func orchestratorRefusal(
        _ message: String?, workspace: WorkspaceSummary, replace: Bool
    ) -> String {
        let name = workspace.name
        let said = (message ?? "").lowercased()
        switch TaskFailure.code(in: message) {
        case "invalid-argument" where said.contains("already has an orchestrator"):
            return "\(name) already has an orchestrator. Choose Replace Orchestrator to start a new one."
        case "invalid-argument" where said.contains("folder"):
            return "The runner couldn’t make \(name)’s folder, so no orchestrator started."
        case "not-found":
            return "\(name) isn’t on this runner anymore."
        case "capability-unsupported":
            return "This runner’s Far Cooler is too old to start an orchestrator. Update it there, then try again."
        case "scope-denied":
            return "This runner lets Far Cooler see its workspaces but not change them."
        case "resource-conflict":
            return "\(name) changed while its orchestrator was starting. Try again."
        case .some:
            return "This runner couldn’t start an orchestrator for \(name). That’s a problem in the app, not in anything you did."
        // A workspace deleted since the sidebar drew it: the CLI can't find
        // its id, so it never asks the daemon and there's no code to read.
        // Matched on `resolve`'s sentence in crates/cli/src/main.rs.
        case nil where said.contains("no workspace matching"):
            return "\(name) isn’t on this runner anymore."
        case nil:
            return replace
                ? "Couldn’t replace \(name)’s orchestrator. Check that the runner is reachable, then try again."
                : "Couldn’t start an orchestrator for \(name). Check that the runner is reachable, then try again."
        }
    }
}

extension Notifier {
    /// Where a notification says its pane is: the workspace first, then the
    /// worktree — "Billing · fc-3-webhooks". An orchestrator is its workspace
    /// alone, "Billing": it runs in the main checkout, which every
    /// orchestrator shares, so the checkout says nothing about which one it
    /// is. The worktree alone when no listed workspace owns the pane: an
    /// unclaimed worktree, or a runner without workspaces.
    static func place(of terminal: Terminal, in worktree: Worktree, workspaces: [WorkspaceSummary]?) -> String {
        let owner = (terminal.workspace ?? worktree.workspace).flatMap { id in
            workspaces?.first { $0.id == id && !$0.isImplicit }
        }
        guard let owner else { return worktree.task }
        return terminal.isOrchestrator ? owner.name : "\(owner.name) · \(worktree.task)"
    }

    /// Who a notification is about: the pane's title, or "Orchestrator" for
    /// one — the runner titles every orchestrator `orchestrator`, and the
    /// sidebar calls it Orchestrator.
    static func speaker(_ terminal: Terminal) -> String {
        terminal.isOrchestrator ? "Orchestrator" : terminal.title
    }

    /// A blocked or finished agent's notification, or nil for any other
    /// state. `place` leads the body.
    static func words(for terminal: Terminal, place: String) -> (title: String, body: String)? {
        let who = speaker(terminal)
        switch terminal.agent {
        case .blocked:
            // Capitalized to match the daemon's own `watch::notification`, which
            // writes this same sentence into the push it sends when this app is
            // closed. One person gets whichever of the two is delivered, about
            // one pane, and two casings of one sentence is two notifications.
            return ("\(who) needs you", "\(place) — Waiting for your answer")
        case .done:
            // How the turn ENDED, which `activity` alone cannot say — the
            // daemon reads it out of the agent's own log and sends it beside
            // this. Telling someone an agent finished when its turn died is
            // the same lie the green dot used to tell.
            if terminal.status == .failedTurn {
                return ("\(who) failed", "\(place) — Its last turn didn’t finish")
            }
            // What it finished, where there is an answer to that: the agent's
            // own last words, already redacted and cut from their start by the
            // daemon. `lastSaid`, not `recentSteps.last`, whose last row is the
            // END of the message. See `Terminal.lastSaid`.
            if let said = terminal.lastSaid, !said.isEmpty {
                return ("\(who) finished", "\(place) — \(said)")
            }
            return ("\(who) finished", place)
        default:
            return nil
        }
    }
}

/// A Replace Orchestrator the person hasn't confirmed yet.
struct OrchestratorReplacement: Identifiable {
    let host: String
    let workspace: WorkspaceSummary
    let harness: OrchestratorHarness
    var id: String { "\(host)\u{1}\(workspace.id)" }
}

/// What choosing a harness from the header's menu does: Start goes to the
/// runner, Replace only asks. It's the confirmation that sends `--replace`,
/// because the orchestrator running now closes.
enum OrchestratorRequest: Equatable {
    case start(OrchestratorHarness)
    case confirmReplace(OrchestratorHarness)

    init(harness: OrchestratorHarness, replace: Bool) {
        self = replace ? .confirmReplace(harness) : .start(harness)
    }
}

/// The orchestrator starts this app has asked for and the runner hasn't
/// answered, by runner and workspace: a second click while one is in flight
/// is dropped, and the row reads "Starting Orchestrator…" meanwhile.
struct OrchestratorStarts {
    private var inFlight: Set<String> = []

    private static func key(_ workspace: WorkspaceSummary, _ host: String) -> String {
        "\(host)\u{1}\(workspace.id)"
    }

    /// Whether this start may go ahead: false while another is in flight.
    mutating func begin(_ workspace: WorkspaceSummary, host: String) -> Bool {
        inFlight.insert(Self.key(workspace, host)).inserted
    }

    mutating func end(_ workspace: WorkspaceSummary, host: String) {
        inFlight.remove(Self.key(workspace, host))
    }

    func isStarting(_ workspace: WorkspaceSummary, host: String) -> Bool {
        inFlight.contains(Self.key(workspace, host))
    }
}
