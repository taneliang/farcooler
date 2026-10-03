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
            "Coding agents such as Claude Code keep working on this Mac after you close Far Cooler. "
            + "You step in only when one needs you, from here or from your phone."
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
            "Choose the Git repository you want agents to work on. "
            + "They work in separate folders, so your own checkout stays as you left it."
        public static let repositoryButton = "Add Repository…"
        public static let orchestratorTitle = "Start the Orchestrator"
        public static let orchestratorBody =
            "Instead of running each agent yourself, you tell one agent, the orchestrator, what you want done. "
            + "It splits the work into tasks and starts agents on them."
        public static let optional = "Optional"
        public static let phoneTitle = "Add Your Phone"
        public static let phoneBody = "Answer your agents from an iPhone or Android phone when you’re away from this Mac."
        public static let phoneButton = "Add Another Device…"
        public static let cliTitle = "Command-Line Tools"
        public static let cliBody = "Use the farcooler command in Terminal. You can also install it later in Settings."
        public static let cliButton = "Install"
        /// A sentence beside `linuxButton`, which names the runner it teaches.
        public static let linuxBody =
            "Agents can also run on a Linux computer you reach with SSH, so they keep working while this Mac is off."
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
        public static let orchestratorBody = "Tell it what you want done. It splits the work into tasks and starts an agent on each."
        public static let start = "Start Orchestrator"
        /// Under a harness the runner doesn't have, which is disabled.
        public static let notInstalled = "Not Installed"
        public static let useAsOrchestrator = "Use as Orchestrator"
        public static let notStartedAccessibility = "Orchestrator, not started"
        /// Under the skeleton rows, with no orchestrator.
        public static let tasksNone = "When the orchestrator splits up your work, each piece appears here as a task."
        public static let tasksStarting = "No tasks yet."
        public static let tasksRunning = "No tasks yet. Tell the orchestrator what you want done."
        /// Only about a board with at least one task.
        public static let caughtUp = "You’re all caught up."
        /// Only while the main checkout is the workspace's one worktree.
        public static let worktreesCaption =
            "Each agent the orchestrator starts gets its own folder and branch, called a worktree, "
            + "so agents don’t change each other’s files."
        /// The main checkout row's tooltip: the terminals a person starts.
        public static let mainCheckoutHelp = "Open a terminal here to run your own commands, such as a dev server."
        public static let noTerminals = "No terminals"
    }

    /// The Mac's conversation column.
    public enum Conversation {
        public static let noneTitle = "No Orchestrator Yet"
        public static let noneBody =
            "Instead of running each agent yourself, tell the orchestrator what you want done. "
            + "It splits the work into tasks and starts an agent on each. When it needs a decision, it asks you."
        public static let charter =
            "The first time, it asks how you like work done, such as who reviews changes before they’re merged. "
            + "It saves your answers as its charter and follows them from then on."
        public static let start = "Start Orchestrator"
        public static let useRunningTerminal = "Use Running Terminal"
        public static let startingTitle = "Starting Orchestrator…"
        public static let startingBody =
            "The first time, it asks a few questions about how you work. Answer them here, then tell it what you want done."
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

    /// The Mac's Needs You page with nothing waiting, under "Nothing Needs You".
    public enum NeedsYou {
        public static let emptyBody =
            "When an agent needs an answer or a review from you, it waits here. Until then, there’s nothing you need to do."
    }

    /// The iPhone. Android says the same in sentence case (`FirstRun.kt`).
    public enum Phone {
        public static let onboardingTitle = "Connect to Your Agents"
        public static let onboardingPrimary = "Connect This Device"
        public static let onboardingSecondary = "More Ways to Add…"
        public static let noRepositoriesBody =
            "Add the Git repository you want agents to work on, here or on your Mac with File > Add Repository."
        public static let addRepository = "Add Repository…"
        public static let nothingNeedsYou = "Nothing Needs You"
        public static let noOrchestratorRunning =
            "No agents are working yet. To give them work, open a workspace below and start its orchestrator."
        public static let orchestratorTitle = "No Orchestrator Yet"
        public static let orchestratorBody =
            "Instead of running each agent yourself, tell the orchestrator what you want done. "
            + "It splits the work into tasks and starts an agent on each. The first time, it asks how you like to work."
        public static let start = "Start Orchestrator"
        public static let tryAgain = "Try Again"
        public static let boardTitle = "No Tasks"
        public static let boardNoOrchestrator =
            "Start the orchestrator and tell it what you want done. Each piece of work it hands out appears here as a task."
        /// Duplicates ov-184's `BoardForm.blankLine`; one of the two goes when both land.
        public static let boardWithOrchestrator =
            "Tell the orchestrator what you want done. Each piece of work it hands out appears here as a task."
        public static let showOrchestrator = "Show Orchestrator"
        /// Shown only while this device isn't signed in: signing in pairs every
        /// runner for push (the owner's ruling, 3 Oct), so it's the one step.
        public static let pushBody =
            "To hear from your agents while Far Cooler is closed, sign in. Until then, notifications arrive only while it’s open."
        public static let signIn = "Sign In"

        /// `device` is the model's name: "iPhone" or "iPad".
        public static func onboardingBody(device: String) -> String {
            "Your agents work on a Mac or Linux computer that runs Far Cooler, called a runner. "
                + "Connect this \(device) to one to answer your agents while you’re away from it."
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
            Welcome.cliButton, Welcome.linuxBody, Welcome.linuxButton,
            Welcome.macReady([]), Welcome.macReady(harnesses),
            Navigator.orchestratorTitle, Navigator.notStarted, Navigator.orchestratorBody, Navigator.start,
            Navigator.notInstalled, Navigator.useAsOrchestrator, Navigator.notStartedAccessibility,
            Navigator.tasksNone, Navigator.tasksStarting, Navigator.tasksRunning, Navigator.caughtUp,
            Navigator.worktreesCaption, Navigator.mainCheckoutHelp, Navigator.noTerminals,
            NeedsYou.emptyBody,
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
