import AgentKit
import SwiftUI

// Starting an orchestrator from an empty state (ov-519): the Plan, the Tasks
// list and the conversation column all say what the orchestrator is for and
// offer the same Start Orchestrator, which checks that the agent it starts is
// installed before it starts one.

/// Which agents Start Orchestrator offers, from what the runner said it has.
enum OrchestratorChoices: Equatable {
    /// Every agent, nothing known to be missing (a runner that doesn't say).
    case all
    /// Some are installed: the rest are listed, dimmed, as not installed.
    case some(installed: [OrchestratorHarness], missing: [OrchestratorHarness])
    /// None are installed: nothing can start until one is.
    case none

    init(_ availability: HarnessAvailability) {
        func mine(_ list: [AgentHarness]) -> [OrchestratorHarness] {
            list.compactMap { OrchestratorHarness(rawValue: $0.rawValue) }
        }
        if !availability.isKnown || availability.missing.isEmpty {
            self = .all
        } else if availability.installed.isEmpty {
            self = .none
        } else {
            self = .some(installed: mine(availability.installed), missing: mine(availability.missing))
        }
    }

    /// The agents that can start.
    var installed: [OrchestratorHarness] {
        switch self {
        case .all: return OrchestratorHarness.allCases
        case .some(let installed, _): return installed
        case .none: return []
        }
    }

    /// The agents to list as not installed.
    var missing: [OrchestratorHarness] {
        switch self {
        case .all: return []
        case .some(_, let missing): return missing
        case .none: return OrchestratorHarness.allCases
        }
    }

    /// What to say beside the button: the fix for no agent at all, or which
    /// one is missing; nothing when all are there.
    func sentence(on place: RunnerPlace) -> String? {
        switch self {
        case .all: return nil
        case .none: return FirstRunCopy.Conversation.noneInstalled(on: place)
        case .some(_, let missing):
            let agents = missing.compactMap { AgentHarness(rawValue: $0.rawValue) }
            return FirstRunCopy.Conversation.someMissing(agents, on: place)
        }
    }
}

/// What an empty state needs to send someone to this workspace's orchestrator.
struct OrchestratorInvite {
    /// Whether an orchestrator is seated: running, starting or stopped.
    var seated: Bool
    /// False on a read-only runner, which offers nothing.
    var canAct: Bool
    var availability: HarnessAvailability
    var place: RunnerPlace
    var onStart: (OrchestratorHarness) -> Void
    /// Show Orchestrator: the conversation, where it asks and answers.
    var onShow: () -> Void = {}
    /// Ask the runner what's installed again, after somebody installs it.
    var onCheckAgain: () -> Void = {}
    /// Suggest Themes, where the orchestrator is live and its runner has the
    /// prompt (ov-519); nil elsewhere.
    var suggestThemes: SuggestThemes? = nil

    var choices: OrchestratorChoices { OrchestratorChoices(availability) }
}

extension EnvironmentValues {
    /// Whether the orchestrator's conversation column is on screen beside what
    /// is drawn: the Plan's empty states then say what to do and leave the
    /// button to the column, rather than repeating it (ov-519 review).
    @Entry var besideOrchestratorColumn = false

    /// The window's way to an orchestrator, for the empty states that say what
    /// to do next. Nil where there's no workspace to start one in.
    @Entry var orchestratorInvite: OrchestratorInvite?
}

/// Start Orchestrator, as a menu of the agents that are installed, with the
/// ones that aren't dimmed and said. With none installed it's off, and the
/// sentence under it says what to install, with Check Again.
struct StartOrchestratorMenu: View {
    let invite: OrchestratorInvite
    /// A caption under the button saying what's missing.
    var caption = true
    /// The caption's ink: quiet ink on paper, the plane's on the navigator's.
    var captionInk: Color = .quietInk

    var body: some View {
        let choices = invite.choices
        VStack(spacing: 8) {
            Menu(FirstRunCopy.Conversation.start) {
                ForEach(choices.installed) { harness in Button(harness.title) { invite.onStart(harness) } }
                ForEach(choices.missing) { harness in
                    Button("\(harness.title) · \(FirstRunCopy.Navigator.notInstalled)") {}.disabled(true)
                }
            }
            .fixedSize()
            .disabled(choices == .none)
            .identified("start-orchestrator")
            if caption, let sentence = choices.sentence(on: invite.place) {
                Text(sentence)
                    .font(.callout)
                    .foregroundStyle(captionInk)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: 320)
                    .identified("orchestrator-missing")
                if choices == .none {
                    Button(FirstRunCopy.Welcome.checkAgain, action: invite.onCheckAgain)
                        .identified("orchestrator-check-again")
                }
            }
        }
    }
}
