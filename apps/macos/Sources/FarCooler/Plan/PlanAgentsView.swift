import AgentKit
import SwiftUI

// Agents (ov-503, design 6; ov-518): one row per agent that still exists,
// working ones first. An agent that finished or exited is not listed. Each
// row says who (role and model), what for (its tickets, or "On no
// ticket"), its state and age, then its tokens and, second, the
// API-equivalent dollars, in columns that line up down the section. A working
// agent has a filled dot; an idle one, a ring. The rows, their order and every
// word are the Rust core's (`PlanView.Agents`, ov-1016).

struct PlanAgentsView: View {
    let agents: PlanView.Agents
    /// The ticket each agent's row opens, by agent id: its first.
    let opens: [String: String]
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if agents.rows.isEmpty {
                PlanEmptyLine(words: agents.header).identified("plan-agents-empty")
            } else {
                Text(agents.header)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, NavigatorGrid.textInset)
                    .padding(.vertical, NavigatorRhythm.air)
                    .identified("plan-agents-header")
                ForEach(agents.rows) { PlanAgentRow(agent: $0, opens: opens[$0.id], onOpen: onOpen) }
            }
        }
    }
}

struct PlanAgentRow: View {
    let agent: PlanView.AgentRow
    /// The task its row opens, or nil for an agent on no ticket.
    let opens: String?
    let onOpen: (PlanPage) -> Void
    @State private var hovering = false

    /// The columns on the right: tokens, then dollars.
    static let tokensWidth: CGFloat = 48
    static let costWidth: CGFloat = 58

    var body: some View {
        let row = content
            .frame(maxWidth: .infinity, alignment: .leading)
            .navigatorRow(selected: false, keyed: false, leading: 0)
            .background {
                if hovering && opens != nil { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
            }
        return Group {
            if let opens {
                Button { onOpen(.ticket(opens)) } label: { row }
                    .buttonStyle(.plain)
                    .onHover { hovering = $0 }
                    .accessibilityAddTraits(.isButton)
            } else {
                row
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel([agent.accessibility, agent.cost].compactMap { $0 }.joined(separator: ", "))
        .identified("plan-agent-\(agent.id)")
    }

    private var content: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Image(systemName: agent.working ? "circle.fill" : "circle")
                .font(.system(size: 6))
                .foregroundStyle(agent.working ? AnyShapeStyle(.primary) : AnyShapeStyle(.quietInk))
                .glyphColumn()
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    Text(agent.who)
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(1)
                    Spacer(minLength: 0)
                    column(agent.tokens, width: Self.tokensWidth)
                    column(agent.cost, width: Self.costWidth)
                }
                Text(agent.second)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func column(_ text: String?, width: CGFloat) -> some View {
        Text(text ?? "")
            .font(.system(size: WorkspaceStyle.PaneText.secondary).monospacedDigit())
            .foregroundStyle(.quietInk)
            .lineLimit(1)
            .frame(width: width, alignment: .trailing)
    }
}

/// What a section says with nothing in it: one quiet line, in the text column.
struct PlanEmptyLine: View {
    let words: String

    var body: some View {
        Text(words)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.quietInk)
            .padding(.leading, NavigatorGrid.textInset)
            .padding(.vertical, NavigatorRhythm.air)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
