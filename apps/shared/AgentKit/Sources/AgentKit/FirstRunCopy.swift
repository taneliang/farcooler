import Foundation

/// Where an agent would run, as a sentence names it.
public enum RunnerPlace: Equatable, Sendable {
    case thisMac
    /// A runner by the name the app shows for it.
    case runner(String)

    var phrase: String {
        switch self {
        case .thisMac: return "this Mac"
        case .runner(let name): return name
        }
    }
}

/// The words for first run and empty states (ov-205), on the Mac and iPhone.
///
/// Apple casing: title case for titles, buttons, menu items and headers,
/// sentence case with a period for body text. Android's sentence-case copy is
/// `model/FirstRun.kt`. `FirstRunTests` reads every string here back against
/// the voice rules: no "!", no straight apostrophes, no spaced dash, no stock
/// phrases.
public enum FirstRunCopy {
    /// The Mac's detail while no runner lists a repository.
    public enum Welcome {
        public static let title = "Welcome to Far Cooler"
        public static let body =
            "Far Cooler runs coding agents in terminals on this Mac or on a Linux computer you reach over SSH. "
            + "They keep working after you close the app."
        public static let macTitle = "This Mac"
        public static let macChecking = "Checking this Mac…"
        public static let macNoTmux =
            "Far Cooler needs tmux to keep agents running after the app closes. Install it with Homebrew, then check again."
        /// What Copy Command copies.
        public static let tmuxCommand = "brew install tmux"
        public static let copyCommand = "Copy Command"
        public static let macNoAgent =
            "No coding agent is installed on this Mac. Install Claude Code, Codex, or the Cursor CLI, then check again."
        public static let checkAgain = "Check Again"
        /// The raw output goes in a Details disclosure, never here.
        public static let macServiceDown = "Far Cooler’s background service didn’t start."
        public static let tryAgain = "Try Again"
        public static let repositoryTitle = "Add a Repository"
        public static let repositoryBody =
            "Choose a Git repository on this Mac. Agents work in worktrees of their own, so your checkout stays as you left it."
        public static let repositoryButton = "Add Repository…"
        public static let orchestratorTitle = "Start the Orchestrator"
        public static let orchestratorBody =
            "The orchestrator is the agent you talk to. It plans tasks and starts other agents to do them."
        public static let optional = "Optional"
        public static let phoneTitle = "Add Your Phone"
        public static let phoneBody = "See what needs you and answer agents from an iPhone or Android phone."
        public static let phoneButton = "Add Another Device…"
        public static let cliTitle = "Command-Line Tools"
        public static let cliBody = "Use the farcooler command in Terminal. You can also install it later in Settings."
        public static let cliButton = "Install"
        public static let linuxPrompt = "Have a Linux computer?"
        public static let linuxButton = "Add a Runner by Address…"

        /// "Ready to run Claude Code and Codex."
        public static func macReady(_ agents: [AgentHarness]) -> String {
            agents.isEmpty ? "Ready to run coding agents." : "Ready to run \(list(agents))."
        }
    }

    /// The Mac's navigator: the orchestrator row and an empty Tasks.
    public enum Navigator {
        public static let orchestratorTitle = "Orchestrator"
        public static let notStarted = "Not Started"
        public static let orchestratorBody = "Tell it what you want done. It plans the tasks and starts agents on them."
        public static let start = "Start Orchestrator"
        /// Under a harness the runner doesn't have, which is disabled.
        public static let notInstalled = "Not Installed"
        public static let useAsOrchestrator = "Use as Orchestrator"
        public static let notStartedAccessibility = "Orchestrator, not started"
        /// Under the skeleton rows, with no orchestrator.
        public static let tasksNone = "The orchestrator’s tasks appear here."
        public static let tasksStarting = "No tasks yet."
        public static let tasksRunning = "No tasks yet. Tell the orchestrator what you want done."
        /// Only about a board with at least one task.
        public static let caughtUp = "You’re all caught up."
        /// Only while the main checkout is the workspace's one worktree.
        public static let worktreesCaption =
            "Each agent works in a worktree of its own, listed here. Open a terminal in the main checkout when you need one."
        public static let noTerminals = "No terminals"
    }

    /// The Mac's conversation column.
    public enum Conversation {
        public static let noneTitle = "No Orchestrator Yet"
        public static let noneBody =
            "The orchestrator is the agent you talk to about this workspace. "
            + "Tell it what you want done, and it plans the tasks and starts agents on them."
        public static let charter =
            "The first time it starts, it asks a few questions about how you work, such as how changes land "
            + "and who reviews them. Your answers become the workspace’s charter."
        public static let start = "Start Orchestrator"
        public static let useRunningTerminal = "Use Running Terminal"
        public static let startingTitle = "Starting Orchestrator…"
        public static let startingBody = "The first time, it asks a few questions before it plans anything."
        public static let startingSlow = "This is taking longer than usual."
        public static let tryAgain = "Try Again"
        public static let useAnotherAgent = "Use Another Agent"
        public static let stoppedTitle = "Orchestrator Stopped"

        /// "Codex and Cursor aren’t installed on this Mac."
        public static func someMissing(_ missing: [AgentHarness], on place: RunnerPlace) -> String {
            "\(list(missing)) \(missing.count == 1 ? "isn’t" : "aren’t") installed on \(place.phrase)."
        }

        /// With Start Orchestrator disabled.
        public static func noneInstalled(on place: RunnerPlace) -> String {
            "No coding agent is installed on \(place.phrase). Install Claude Code, Codex, or the Cursor CLI first."
        }

        public static func notInstalledTitle(_ harness: AgentHarness) -> String {
            "\(harness.installTitle) Isn’t Installed"
        }

        public static func notInstalledBody(_ harness: AgentHarness, on place: RunnerPlace) -> String {
            let program = harness.program
            switch place {
            case .thisMac:
                return "The orchestrator couldn’t start because there’s no \(program) command on this Mac. "
                    + "Install \(harness.installName), check that \(program) runs in Terminal, then try again."
            case .runner(let name):
                return "The orchestrator couldn’t start because there’s no \(program) command on \(name). "
                    + "Install \(harness.installName) there, check that \(program) runs when you connect with SSH, then try again."
            }
        }
    }

    /// The iPhone. Android says the same in sentence case (`FirstRun.kt`).
    public enum Phone {
        public static let onboardingTitle = "Connect a Runner"
        public static let onboardingPrimary = "Connect This Device"
        public static let onboardingSecondary = "More Ways to Add…"
        public static let noRepositoriesBody =
            "Add one here, or in the Mac app with File > Add Repository. Each one starts with a workspace called Main."
        public static let addRepository = "Add Repository…"
        public static let nothingNeedsYou = "Nothing Needs You"
        public static let noOrchestratorRunning = "No orchestrator is running yet. Open a workspace below to start one."
        public static let orchestratorTitle = "No Orchestrator Yet"
        public static let orchestratorBody =
            "Tell the orchestrator what you want done, and it plans the tasks and starts agents on them. "
            + "The first time, it asks a few questions about how you work."
        public static let start = "Start Orchestrator"
        public static let tryAgain = "Try Again"
        public static let boardTitle = "No Tasks"
        public static let boardNoOrchestrator = "Start the orchestrator and tell it what you want done. Its tasks appear here."
        /// ov-184's `BoardForm.blankLine`, kept. One of the two goes when both land.
        public static let boardWithOrchestrator = "Ask your orchestrator to plan the work. Its tasks appear here, grouped by status."
        public static let showOrchestrator = "Show Orchestrator"
        public static let pushBody =
            "Notifications arrive only while Far Cooler is open. To get them when it’s closed, sign in, "
            + "then turn on notifications for your runner in Far Cooler on your Mac."
        public static let signIn = "Sign In"

        /// `device` is the model's name: "iPhone" or "iPad".
        public static func onboardingBody(device: String) -> String {
            "A runner is where your agents run: Far Cooler on a Mac or a Linux computer. "
                + "Connect this \(device) to one to see what they’re doing and answer them."
        }

        public static func noRepositoriesTitle(_ runner: String) -> String {
            "No Repositories on \(runner)"
        }

        public static func notInstalledBody(_ harness: AgentHarness, on runner: String) -> String {
            "There’s no \(harness.program) command on \(runner). Install \(harness.installName) there, then try again."
        }
    }

    /// "Claude Code", "Claude Code and Codex", "Claude Code, Codex, and
    /// Cursor": the serial comma, US style. Not `ListFormatter`, whose
    /// output follows the device's locale while every other word here is
    /// English.
    static func list(_ agents: [AgentHarness]) -> String {
        let names = agents.map(\.title)
        switch names.count {
        case 0, 1: return names.first ?? ""
        case 2: return "\(names[0]) and \(names[1])"
        default: return names.dropLast().joined(separator: ", ") + ", and " + names[names.count - 1]
        }
    }

    /// Every string above, each function at every harness and place, for the
    /// voice check.
    static var all: [String] {
        let places: [RunnerPlace] = [.thisMac, .runner("build-01")]
        let harnesses = AgentHarness.allCases
        var strings: [String] = [
            Welcome.title, Welcome.body, Welcome.macTitle, Welcome.macChecking, Welcome.macNoTmux,
            Welcome.tmuxCommand, Welcome.copyCommand, Welcome.macNoAgent, Welcome.checkAgain,
            Welcome.macServiceDown, Welcome.tryAgain, Welcome.repositoryTitle, Welcome.repositoryBody,
            Welcome.repositoryButton, Welcome.orchestratorTitle, Welcome.orchestratorBody, Welcome.optional,
            Welcome.phoneTitle, Welcome.phoneBody, Welcome.phoneButton, Welcome.cliTitle, Welcome.cliBody,
            Welcome.cliButton, Welcome.linuxPrompt, Welcome.linuxButton,
            Welcome.macReady([]), Welcome.macReady(harnesses),
            Navigator.orchestratorTitle, Navigator.notStarted, Navigator.orchestratorBody, Navigator.start,
            Navigator.notInstalled, Navigator.useAsOrchestrator, Navigator.notStartedAccessibility,
            Navigator.tasksNone, Navigator.tasksStarting, Navigator.tasksRunning, Navigator.caughtUp,
            Navigator.worktreesCaption, Navigator.noTerminals,
            Conversation.noneTitle, Conversation.noneBody, Conversation.charter, Conversation.start,
            Conversation.useRunningTerminal, Conversation.startingTitle, Conversation.startingBody,
            Conversation.startingSlow, Conversation.tryAgain, Conversation.useAnotherAgent,
            Conversation.stoppedTitle,
            Phone.onboardingTitle, Phone.onboardingPrimary, Phone.onboardingSecondary, Phone.noRepositoriesBody,
            Phone.addRepository, Phone.nothingNeedsYou, Phone.noOrchestratorRunning, Phone.orchestratorTitle,
            Phone.orchestratorBody, Phone.start, Phone.tryAgain, Phone.boardTitle, Phone.boardNoOrchestrator,
            Phone.boardWithOrchestrator, Phone.showOrchestrator, Phone.pushBody, Phone.signIn,
            Phone.onboardingBody(device: "iPhone"), Phone.noRepositoriesTitle("build-01"),
            NotificationAsk.title, NotificationAsk.message, NotificationAsk.allow, NotificationAsk.decline,
        ]
        for place in places {
            strings.append(Conversation.noneInstalled(on: place))
            strings.append(Conversation.someMissing([.cursor], on: place))
            strings.append(Conversation.someMissing([.codex, .cursor], on: place))
        }
        for harness in harnesses {
            strings.append(Conversation.notInstalledTitle(harness))
            strings.append(Phone.notInstalledBody(harness, on: "build-01"))
            for place in places { strings.append(Conversation.notInstalledBody(harness, on: place)) }
        }
        return strings
    }
}
