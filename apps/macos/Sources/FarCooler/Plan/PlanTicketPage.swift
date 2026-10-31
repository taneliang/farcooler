import AgentKit
import SwiftUI

/// A ticket opened from the plan, as one scrolling page (ov-873): its header
/// with the title whole, who is working it and where its branch stands, then
/// today's Overview (intent, acceptance, the record). There is no tab strip
/// here, so what the Agent and Changes tabs gave is reached by the links in
/// the work section, and each lands as a step Back undoes.
struct PlanTicketPage<Line: View, Card: View>: View {
    let row: TaskRow
    var ask: AskOrchestrator.Action = .unavailable
    var agent: BoardPane?
    var work: PlanTicketWork
    @ViewBuilder let line: () -> Line
    @ViewBuilder let card: () -> Card

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                TaskViewHeader(row: row, ask: ask, agent: agent, wrapsTitle: true)
                line()
                PlanTicketWorkView(work: work)
                card().padding(TaskTypography.inset)
            }
        }
        // The record runs past the window on a long task: bars that stay, and
        // a soft edge that says there is more below.
        .scrollIndicators(.visible)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        .background(WorkspaceStyle.paper)
        .identified("plan-ticket-page")
    }
}

/// What the work section says and offers, a value so a test reads it without
/// drawing it.
struct PlanTicketWork {
    /// "Builder 1", or the agent program's own name when the ticket wasn't
    /// opened under an agent's card.
    var agent: String?
    /// What it is doing now.
    var doing: String?
    /// The worktree's branch.
    var branch: String?
    /// The chat of the agent, where there is one.
    var openChat: (() -> Void)?
    /// Its terminal.
    var openTerminal: (() -> Void)?
    /// What its branch changes so far, where the runner can read them.
    var openChanges: (() -> Void)?

    var isEmpty: Bool { agent == nil && branch == nil }

    /// The agent's name and doing-now: the plan card's words when the ticket
    /// was opened under it, else the pane's own.
    static func who(origin: PlanStack.Origin?, task: String, pane: BoardPane?) -> (name: String, doing: String)? {
        if let origin, origin.task == task { return (origin.agent, origin.doing) }
        guard let pane else { return nil }
        let line = TaskColumnModel.agentLine(pane)?.text
        return (Terminal.name(of: pane.terminal.preset), line.map { $0.split(separator: " ").dropFirst().joined(separator: " ") } ?? "")
    }
}

/// "Being Worked On" and "Branch", under the header.
private struct PlanTicketWorkView: View {
    let work: PlanTicketWork

    var body: some View {
        if !work.isEmpty {
            VStack(alignment: .leading, spacing: ColumnGrid.rhythm * 2) {
                if let agent = work.agent {
                    section("Being worked on") {
                        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                            (Text(agent).fontWeight(.semibold)
                                + Text(work.doing.map { $0.isEmpty ? "" : "  " + $0 } ?? ""))
                                .font(TaskTypography.body)
                                .fixedSize(horizontal: false, vertical: true)
                            HStack(spacing: ColumnGrid.rhythm * 2) {
                                if let open = work.openChat {
                                    Button("Open Chat", action: open)
                                }
                                if let open = work.openTerminal {
                                    Button(action: open) { Image(systemName: "terminal") }
                                        .help("Open Terminal")
                                        .accessibilityLabel("Open Terminal")
                                }
                            }
                            .buttonStyle(.link)
                            .font(TaskTypography.meta)
                        }
                    }
                    .identified("plan-ticket-agent")
                }
                if let branch = work.branch {
                    section("Branch") {
                        HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm * 2) {
                            Text(branch)
                                .font(TaskTypography.body.monospaced())
                                .textSelection(.enabled)
                            if let open = work.openChanges {
                                Button("Show Changes", action: open)
                                    .buttonStyle(.link)
                                    .font(TaskTypography.meta)
                            }
                        }
                    }
                    .identified("plan-ticket-branch")
                }
            }
            .padding(.horizontal, TaskTypography.inset.leading)
            .padding(.top, ColumnGrid.rhythm)
            .frame(maxWidth: TaskTypography.measure + 2 * TaskTypography.inset.leading, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: TaskTypography.labelGap) {
            Text(title)
                .font(TaskTypography.label)
                .foregroundStyle(.quietInk)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }
}
