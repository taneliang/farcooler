import AgentKit
import SwiftUI

// The agents at work in a workspace, on the Mac (ov-641): the count in the
// orchestrator's top corner (ov-829, `AgentsCorner.swift`), and the panel it opens.
// The iPhone's (`WorkspaceCards`, `AgentsView`), with the same count, the same
// rows and the same words, because the counting rule is AgentKit's
// `RunningAgents`; only the data's way in is the Mac's, since the Mac decodes
// the CLI's JSON into its own `Terminal`, not the phones'.
//
// Everything counted is from the fleet, which every poll and every event
// replaces whole, so the count is right after a reconnect without anything else
// being asked. The conversation, when this window holds it live, adds each
// subagent's type, newest call and start, joined on the call id.

/// Where the agents at work come from.
enum MacAgents {
    /// `workspace`'s agent panes on `host`, as `RunningAgents` reads them: the
    /// alive ones that run an agent, in the fleet's order. Shells and exited
    /// panes are not agents at work.
    static func panes(workspace: String, host: String, fleet: Fleet) -> [AgentPane] {
        var out: [AgentPane] = []
        for worktree in fleet.worktrees where (worktree.host ?? "") == host {
            for terminal in worktree.terminals where isAlive(terminal) && isAgent(terminal) {
                guard owner(of: terminal, in: worktree) == workspace else { continue }
                out.append(pane(terminal, in: worktree))
            }
        }
        return out
    }

    /// The workspace a pane is counted under: its own, else its worktree's.
    /// Not the implicit workspace of a runner without workstreams, whose
    /// agents are nobody's orchestrated work.
    static func owner(of terminal: Terminal, in worktree: Worktree) -> String? {
        if let own = terminal.workspace, !own.isEmpty { return own }
        if let owner = worktree.workspace, !owner.isEmpty { return owner }
        return nil
    }

    // `isAgent`, `isAlive`, `owner(of:in:)` and `pane` repeat AgentKit's `RunningAgents.isAgent`, `isAlive` and
    // `AgentPane.init(_:in:workspace:)` over this app's own `Terminal`, whose declaration differs from the phones' (see
    // CoreModel.swift). A change to what counts as an agent at work is made in both; `WorkspaceAgentsTests` and AgentKit's
    // `RunningAgentsTests` read the same cases.

    static func isAgent(_ terminal: Terminal) -> Bool {
        terminal.isOrchestrator || terminal.role == "agent" || terminal.agent.isAgent
    }

    /// A pane whose process is still there: not exited, lost or failed.
    static func isAlive(_ terminal: Terminal) -> Bool {
        switch StateKind.parse(terminal.state) {
        case .lost, .exited, .error: false
        default: true
        }
    }

    static func pane(_ terminal: Terminal, in worktree: Worktree) -> AgentPane {
        AgentPane(
            terminal: terminal.id, worktree: worktree.id, label: terminal.label, isOrchestrator: terminal.isOrchestrator,
            workspace: terminal.workspace ?? worktree.workspace,
            activity: terminal.line.flatMap { $0.isEmpty ? nil : $0 } ?? terminal.status.label,
            startedMs: terminal.turnStartedAt.map { Int64($0) }
                ?? (terminal.agent == .working ? terminal.activitySince.map { Int64($0) } : nil),
            // A failed turn wants a person as a blocked one does.
            mark: terminal.status.glanceMark ?? GlanceMark(attention: .failed, core: .atAPrompt),
            subagents: terminal.subagents ?? [], subagentIds: terminal.subagentIds ?? [],
            subagentsRunning: terminal.subagentsRunning)
    }

    /// The agents at work for `workspace` on `host`. `rows` is a pane's
    /// conversation's tray, by terminal, when this window holds it.
    static func running(
        workspace: String, host: String, fleet: Fleet, rows: (String) -> [AgentTray.Entry]? = { _ in nil }
    ) -> RunningAgents {
        RunningAgents.workspace(workspace, runner: host, panes: panes(workspace: workspace, host: host, fleet: fleet), rows: rows)
    }

    /// The tray of a pane whose conversation this window follows, while the
    /// runner answers it; nil otherwise, and the fleet's names stand in.
    @MainActor
    static func followed(_ terminal: String, in agents: NativeAgents) -> [AgentTray.Entry]? {
        guard let model = agents.model(ifMade: terminal), model.store.phase == .live else { return nil }
        return AgentTray.entries(model.store)
    }
}

/// What the agents panel says.
enum AgentsWords {
    static let title = "Agents"
    static let noneTitle = "No Agents Running"
    static let noneBody = "Agents the orchestrator starts, and the subagents they start, show up here while they work."
    static let orchestratorSection = "Orchestrator’s Subagents"
    static let panesSection = "Agents"
}

/// Now/Next over a workspace's conversation while the canvas is folded
/// away: the plan's strip, the column's width. Its edges are the paper's
/// below it (`Pane.inset` in from the column, the same gap above), so the
/// strip and the conversation read as one column. The agents at work are the
/// corner's (`AgentsCornerButton`, ov-829), not a card in this row.
struct WorkspaceStripRow<NowNext: View>: View {
    @ViewBuilder let nowNext: () -> NowNext

    var body: some View {
        nowNext()
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, Pane.inset)
            .padding(.top, Pane.inset)
            .identified("workspace-cards")
    }
}

/// The agents view: each agent's status dot, its name, what it's doing and how
/// long its run has gone; a row opens its agent.
struct AgentsPanel: View {
    let agents: RunningAgents
    let onChoose: (RunningAgent) -> Void

    /// A workspace's: its orchestrator's subagents, then its agents.
    static func sections(_ agents: RunningAgents) -> [(id: String, title: String?, agents: [RunningAgent], nests: Bool)] {
        let theirs = Array(agents.agents.prefix { $0.kind == .subagent })
        let rest = Array(agents.agents.dropFirst(theirs.count))
        var out: [(id: String, title: String?, agents: [RunningAgent], nests: Bool)] = []
        if !theirs.isEmpty { out.append(("orchestrator", AgentsWords.orchestratorSection, theirs, false)) }
        if !rest.isEmpty { out.append(("panes", theirs.isEmpty ? nil : AgentsWords.panesSection, rest, true)) }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(AgentsWords.title)
                .font(.headline)
                .padding(.horizontal, Spacing.inset)
                .padding(.top, Spacing.inset)
                .padding(.bottom, Spacing.group)
            if agents.agents.isEmpty {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    Text(AgentsWords.noneTitle).font(.callout.weight(.medium))
                    Text(AgentsWords.noneBody).font(.callout).foregroundStyle(.secondary)
                }
                .padding(Spacing.inset)
                .identified("agents-none")
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Self.sections(agents), id: \.id) { section in
                            if let title = section.title {
                                Text(title)
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, Spacing.inset)
                                    .padding(.top, Spacing.group)
                                    .padding(.bottom, Spacing.tight)
                            }
                            ForEach(section.agents) { agent in
                                AgentsRow(agent: agent, nested: section.nests && agent.kind == .subagent) { onChoose(agent) }
                            }
                        }
                    }
                    .padding(.bottom, Spacing.group)
                }
                .scrollBounceBehavior(.basedOnSize)
                .frame(maxHeight: 420)
                .identified("agents-list")
            }
        }
        .frame(width: 380)
        .identified("agents-panel")
    }
}

/// One agent: its dot, its name, what it's doing and its run's time.
struct AgentsRow: View {
    let agent: RunningAgent
    /// A subagent under the pane listed above it.
    let nested: Bool
    let onChoose: () -> Void

    /// Its own, from its id (`<runner>/<terminal>/<row or name>`): two of
    /// one name are still two rows.
    static func identifier(_ agent: RunningAgent) -> String {
        agent.id.split(separator: "/", omittingEmptySubsequences: false).dropFirst().joined(separator: "-")
    }

    var body: some View {
        Button(action: onChoose) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                GlanceMarkView(agent.mark, inAppDiameter: 8)
                    .frame(width: 14)
                    .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                VStack(alignment: .leading, spacing: 1) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(agent.name)
                            .fontWeight(agent.kind == .orchestrator ? .semibold : .medium)
                            .lineLimit(1)
                        Spacer(minLength: Spacing.group)
                        if agent.startedMs != nil { RunTimeChip(startedMs: agent.startedMs, endedMs: nil) }
                    }
                    if !agent.activity.isEmpty {
                        Text(agent.activity)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }
                Image(systemName: "chevron.forward")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .padding(.leading, nested ? Spacing.inset + 14 : Spacing.inset)
            .padding(.trailing, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .identified("agent-row-\(Self.identifier(agent))")
    }
}
