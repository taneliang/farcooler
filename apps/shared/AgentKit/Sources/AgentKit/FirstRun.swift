import Foundation

// First run (ov-205): which agents a runner can start, why an orchestrator's
// pane died at once, and the state of each row on the Mac's Welcome page.
//
// Pure, so `swift test --package-path apps/shared/AgentKit` reads every rule
// back. The words for each state are `FirstRunCopy`'s.

/// A coding agent an orchestrator can run: what `--harness` takes.
///
/// The Mac's and the phones' own `OrchestratorHarness` carry the same three
/// raw values. This one adds the program each launches, which is the word
/// `Host.agents_found` sends, and the name a person reads.
public enum AgentHarness: String, CaseIterable, Sendable {
    case claude, codex, cursor

    /// The program a launch runs, as `Host.agents_found` names it. Cursor's
    /// is `cursor-agent`, not `cursor`, which is the editor.
    public var program: String {
        switch self {
        case .claude: return "claude"
        case .codex: return "codex"
        case .cursor: return "cursor-agent"
        }
    }

    /// The product's own name, in menus and titles: "Claude Code", not the
    /// command.
    public var title: String {
        switch self {
        case .claude: return "Claude Code"
        case .codex: return "Codex"
        case .cursor: return "Cursor"
        }
    }

    /// What a person installs to get `program`, where it isn't `title`:
    /// installing Cursor, the editor, doesn't give you `cursor-agent`.
    public var installName: String {
        self == .cursor ? "the Cursor CLI" : title
    }

    /// `installName` at the head of a title.
    public var installTitle: String {
        self == .cursor ? "Cursor CLI" : title
    }
}

/// Which harnesses a runner can start, read off `agentsFound`.
///
/// `nil` is a runner that doesn't say (no `agents_found` capability). It
/// offers every harness, as every app did before the runner could tell: its
/// list is empty because it never sent one, and reading that as "nothing
/// installed" would grey out Start Orchestrator on a runner that can start one.
public struct HarnessAvailability: Equatable, Sendable {
    /// The programs the runner found, or nil when it didn't say.
    public let agentsFound: [String]?

    public init(agentsFound: [String]?) {
        self.agentsFound = agentsFound
    }

    /// Whether the runner said which agents it has.
    public var isKnown: Bool { agentsFound != nil }

    /// Whether `harness` can start here. True on a runner that didn't say.
    public func isInstalled(_ harness: AgentHarness) -> Bool {
        guard let agentsFound else { return true }
        return agentsFound.contains(harness.program)
    }

    /// The harnesses that can start here, in menu order.
    public var installed: [AgentHarness] {
        AgentHarness.allCases.filter(isInstalled)
    }

    /// The harnesses this runner said it doesn't have, in menu order. Empty
    /// on a runner that didn't say.
    public var missing: [AgentHarness] {
        AgentHarness.allCases.filter { !isInstalled($0) }
    }
}

/// Why an orchestrator's pane ended, when that's something the app can name.
public enum OrchestratorExit: Equatable, Sendable {
    /// The shell couldn't find the agent's command: exit status 127, POSIX's
    /// "command not found", from the `-ilc` shell the pane runs it in.
    case notInstalled

    /// How soon after starting a 127 still means "not installed". An agent
    /// that ran for a while and then ended with 127 started, so its pane is
    /// an ordinary stopped one.
    public static let window: TimeInterval = 15

    /// `.notInstalled` for a 127 within `window` of the start, nil otherwise.
    ///
    /// The runner sends no time of exit, so `ranFor` is the app's own: from
    /// when it asked for the start, or first saw the pane, to when it first
    /// saw the exit. A pane last seen hours ago is too old to call.
    public static func classify(exitCode: Int?, ranFor: TimeInterval) -> OrchestratorExit? {
        guard exitCode == 127, ranFor <= window else { return nil }
        return .notInstalled
    }
}

extension OrchestratorExit {
    /// The agent a quick exit 127 says isn't installed, or nil when the pane's
    /// end isn't that. `endedAfter` is nil until the app has seen the pane end
    /// after asking for it; the harness is the one asked for, else the pane's
    /// own preset.
    public static func missingAgent(
        exitCode: Int?, endedAfter: TimeInterval?, asked: AgentHarness?, preset: String
    ) -> AgentHarness? {
        guard let endedAfter, classify(exitCode: exitCode, ranFor: endedAfter) == .notInstalled else {
            return nil
        }
        return asked ?? AgentHarness.allCases.first { $0.rawValue == preset }
    }
}

/// What the Mac's local runner has said about itself so far.
public enum LocalRunnerState: Equatable, Sendable {
    /// `daemon ensure` hasn't answered yet.
    case checking
    /// The background service didn't start.
    case notRunning
    /// It answered. `tmuxFound` is false when its inventory can't run tmux.
    case running(tmuxFound: Bool, agents: HarnessAvailability)
}

/// The Welcome page's "This Mac" row.
public enum MacStep: Equatable, Sendable {
    case checking
    /// Ready, naming the agents it can run. Empty only when the runner
    /// didn't say, which the bundled one always does.
    case ready([AgentHarness])
    case noTmux
    case noAgent
    case serviceDown

    /// Whether the row needs fixing before anything can run. Drawn with the
    /// red mark, the one the status bar uses for a runtime that's down.
    public var isProblem: Bool {
        switch self {
        case .noTmux, .noAgent, .serviceDown: return true
        case .checking, .ready: return false
        }
    }
}

/// One setup row's state after "This Mac".
public enum StepState: Equatable, Sendable {
    /// An earlier row isn't done, so this one is dimmed.
    case waiting
    case toDo
    case done
}

/// The Welcome page's required rows, derived from what the runners said.
public struct WelcomeSteps: Equatable, Sendable {
    public let mac: MacStep
    public let repository: StepState
    public let orchestrator: StepState

    /// - Parameters:
    ///   - local: what This Mac's runner has said.
    ///   - hasRepositories: whether any runner lists a repository.
    ///   - orchestratorSeated: whether any workspace has an orchestrator.
    public init(local: LocalRunnerState, hasRepositories: Bool, orchestratorSeated: Bool) {
        switch local {
        case .checking:
            mac = .checking
        case .notRunning:
            mac = .serviceDown
        case .running(tmuxFound: false, _):
            mac = .noTmux
        case .running(tmuxFound: true, let agents):
            mac = agents.installed.isEmpty && agents.isKnown ? .noAgent : .ready(agents.isKnown ? agents.installed : [])
        }
        // A repository can be added whatever This Mac says: it may live on a
        // Linux runner, and adding one here does nothing that needs tmux.
        repository = hasRepositories ? .done : .toDo
        // Starting one does need This Mac ready, on the first run where the
        // only runner most people have is this one.
        if orchestratorSeated {
            orchestrator = .done
        } else if hasRepositories, case .ready = mac {
            orchestrator = .toDo
        } else {
            orchestrator = .waiting
        }
    }
}
