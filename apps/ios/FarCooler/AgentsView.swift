import SwiftUI

// The agents at work (ov-619, ov-617): one view, drawn for a workspace from
// its Agents card and across every runner as All Agents on the workspace
// list. Far Cooler's own agents (a lane's or a task's pane) and the Claude
// subagents an agent started are listed alike: each with its status dot,
// its name, what it's doing and how long its run has gone. A row opens its
// agent: a pane opens its worktree on that pane, and a subagent of the
// orchestrator opens its own conversation in the orchestrator's place.
//
// What's listed is `RunningAgents`' (AgentKit), from the fleet, which every
// reconnect replaces whole.

extension Connection {
    /// The agents at work for `summary` (`RunningAgents.workspace`), with
    /// detail from each pane's conversation the phone follows.
    func runningAgents(_ summary: WorkspaceSummary, runner: String) -> RunningAgents {
        return RunningAgents.workspace(summary.id, runner: runner, fleet: fleet) { NativePanes.shared.followed($0) }
    }

    /// A workspace's name as its row says it: a repository's own for an
    /// implicit one.
    func workspaceName(_ id: String?) -> String {
        guard let id else { return AgentsWords.noWorkspace }
        guard let summary = workspace(id) else { return id }
        return summary.isImplicit ? repositoryNames[summary.id] ?? summary.name : summary.name
    }
}

extension NativePanes {
    /// The tray of a pane whose conversation this launch follows, while the
    /// runner answers it; nil otherwise, and the fleet's names stand in.
    func followed(_ terminal: String) -> [AgentTray.Entry]? {
        guard let model = held(terminal), model.store.phase == .live else { return nil }
        return AgentTray.entries(model.store)
    }
}

/// What the agents views say.
enum AgentsWords {
    static let title = "Agents"
    static let allTitle = "All Agents"
    static let noneTitle = "No Agents Running"
    static let noneBody = "Agents the orchestrator starts, and the subagents they start, show up here while they work."
    static let orchestratorSection = "Orchestrator’s Subagents"
    static let panesSection = "Agents"
    static let noWorkspace = "Not in a Workspace"
    /// Said of an agent whose runner isn't answering, beside its dimmed name.
    static let notAnswering = "Runner not answering"
}

/// The agents, as a list: sectioned by `sections`, each row opening its agent.
struct AgentsList: View {
    let sections: [AgentsSection]
    let onChoose: (RunningAgent) -> Void

    var body: some View {
        if sections.allSatisfy(\.agents.isEmpty) {
            ContentUnavailableView {
                Label(AgentsWords.noneTitle, systemImage: "circle.dashed")
            } description: {
                Text(AgentsWords.noneBody)
            }
            .accessibilityIdentifier("agents-none")
        } else {
            List {
                ForEach(sections) { section in
                    Section {
                        ForEach(section.agents) { agent in
                            AgentsRow(agent: agent, nested: section.nests && agent.kind == .subagent, answering: section.answering) {
                                onChoose(agent)
                            }
                        }
                    } header: {
                        if let title = section.title { Text(title) }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .accessibilityIdentifier("agents-list")
        }
    }
}

/// One section of the agents view.
struct AgentsSection: Identifiable {
    var id: String
    var title: String?
    var agents: [RunningAgent]
    /// Subagents sit indented under the pane before them.
    var nests = true
    /// Whether its runner is answering. One that isn't can't say what its
    /// agents are doing now: their marks say so and their names are dimmed,
    /// as the fleet's are elsewhere.
    var answering = true

    /// A workspace's: its orchestrator's subagents, then its agents.
    static func workspace(_ agents: RunningAgents, answering: Bool) -> [AgentsSection] {
        let theirs = Array(agents.agents.prefix { $0.kind == .subagent })
        let rest = Array(agents.agents.dropFirst(theirs.count))
        var out: [AgentsSection] = []
        if !theirs.isEmpty {
            out.append(
                AgentsSection(id: "orchestrator", title: AgentsWords.orchestratorSection, agents: theirs, nests: false, answering: answering))
        }
        if !rest.isEmpty {
            out.append(
                AgentsSection(id: "panes", title: theirs.isEmpty ? nil : AgentsWords.panesSection, agents: rest, answering: answering))
        }
        return out
    }
}

/// One agent: its dot, its name, what it's doing, and its run's time.
struct AgentsRow: View {
    let agent: RunningAgent
    /// A subagent under the pane listed above it.
    let nested: Bool
    let answering: Bool
    let onChoose: () -> Void

    @ScaledMetric(relativeTo: .body) private var markColumn: CGFloat = 16
    @ScaledMetric(relativeTo: .body) private var indent: CGFloat = 20
    @Environment(\.dynamicTypeSize) private var type

    var body: some View {
        Button(action: onChoose) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.inset) {
                GlanceMarkView(agent.mark.said(answering: answering), inAppDiameter: 8)
                    .frame(width: markColumn)
                    .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(agent.name)
                            .fontWeight(agent.kind == .orchestrator ? .semibold : .medium)
                            .foregroundStyle(answering ? .primary : .secondary)
                            .lineLimit(type.isAccessibilitySize ? 3 : 1)
                        Spacer(minLength: Spacing.group)
                        if !type.isAccessibilitySize, agent.startedMs != nil {
                            NativeRunTime(startedMs: agent.startedMs, endedMs: nil)
                        }
                    }
                    if !agent.activity.isEmpty {
                        Text(agent.activity)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    if type.isAccessibilitySize, agent.startedMs != nil {
                        NativeRunTime(startedMs: agent.startedMs, endedMs: nil)
                    }
                }
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .padding(.leading, nested ? indent : 0)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityValue(answering ? "" : AgentsWords.notAnswering)
        .accessibilityIdentifier("agent-row-\(agent.terminal)\(agent.kind == .subagent ? "-\(agent.row ?? agent.name)" : "")")
    }
}

/// A workspace's agents view, as a sheet from its Agents card, held on the
/// phone's navigator (`PhoneNavigator.agents`). A pane's row covers the stack
/// with its worktree once the sheet has gone; a subagent's opens its own
/// conversation in its parent's place.
struct AgentsSheetScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var navigator: PhoneNavigator
    let place: PhoneWorkspace

    var body: some View {
        NavigationStack {
            Group {
                if let connection = fleet.connection(for: place), let summary = connection.workspace(place.workspace) {
                    AgentsList(
                        sections: AgentsSection.workspace(
                            connection.runningAgents(summary, runner: place.runner), answering: connection.isAnswering)
                    ) { agent in choose(agent) }
                } else {
                    RunnerGone()
                }
            }
            .navigationTitle(AgentsWords.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { navigator.agents = nil }
                        .accessibilityIdentifier("agents-done")
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .accessibilityIdentifier("agents-sheet")
    }

    private func choose(_ agent: RunningAgent) {
        if agent.opensOwnConversation {
            navigator.agents = nil
            AgentsOpening.openOwn(agent)
        } else {
            AgentsOpening.openPane(agent, navigator: navigator)
        }
    }
}

/// All Agents (ov-617): every agent at work on every runner, by workspace,
/// each workspace's orchestrator first. Pushed from the workspace list.
struct AllAgentsScreen: View {
    @ObservedObject var fleet: FleetStore
    @Environment(\.phoneNavigator) private var navigator

    var body: some View {
        AgentsList(sections: sections) { agent in choose(agent) }
            .navigationTitle(AgentsWords.allTitle)
            .navigationBarTitleDisplayMode(.inline)
            .accessibilityIdentifier("all-agents")
    }

    private var sections: [AgentsSection] {
        let several = fleet.runners.count > 1
        return fleet.runners.flatMap { runner -> [AgentsSection] in
            let connection = runner.connection
            let id = runner.host.id.uuidString
            let agents = RunningAgents.runner(id, fleet: connection.fleet) { NativePanes.shared.followed($0) }
            return agents.byWorkspace.map { group in
                let name = connection.workspaceName(group.workspace)
                return AgentsSection(
                    id: "\(id)/\(group.workspace ?? "")", title: several ? "\(name) · \(runner.host.label)" : name,
                    agents: group.agents, answering: connection.isAnswering)
            }
        }
    }

    /// The orchestrator opens its workspace; anything else opens as it does
    /// from a workspace's own view.
    private func choose(_ agent: RunningAgent) {
        if agent.kind == .orchestrator || agent.opensOwnConversation, let workspace = agent.workspace {
            navigator?.open(.workspace(PhoneWorkspace(runner: agent.runner, workspace: workspace)))
            if agent.opensOwnConversation { AgentsOpening.openOwn(agent) }
            return
        }
        AgentsOpening.openPane(agent, navigator: navigator)
    }
}

/// Where choosing an agent goes.
@MainActor
enum AgentsOpening {
    /// Its pane, in its worktree.
    static func openPane(_ agent: RunningAgent, navigator: PhoneNavigator?) {
        navigator?.open(.worktree(runner: agent.runner, worktree: agent.worktree, landing: .terminal(agent.terminal)))
    }

    /// A subagent's own conversation, in its parent's place.
    static func openOwn(_ agent: RunningAgent) {
        guard let row = agent.row, let agentId = agent.agentId else { return }
        _ = NativePanes.shared.held(agent.terminal)?.openAgent(row: row, agentId: agentId)
    }
}
