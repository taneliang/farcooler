import AgentKit
import SwiftUI

// Agents (ov-503, design 6; ov-518): one row per agent that still exists,
// working ones first. An agent that finished or exited is not listed. Each
// row says who (role and model), what for (a lane, a train, the Plan, or "Not
// on a lane"), its state and age, then its tokens and, second, the
// API-equivalent dollars, in columns that line up down the section. A working agent has a filled dot; an idle one, a ring.

struct PlanAgentsView: View {
    let fleet: PlanFleet
    let now: Int64
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if fleet.isEmpty {
                PlanEmptyLine(words: fleet.header()).identified("plan-agents-empty")
            } else {
                Text(fleet.header())
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.quietInk)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, NavigatorGrid.textInset)
                    .padding(.vertical, NavigatorRhythm.air)
                    .identified("plan-agents-header")
                ForEach(fleet.agents) { PlanAgentRow(agent: $0, now: now, onOpen: onOpen) }
            }
        }
    }
}

struct PlanAgentRow: View {
    let agent: PlanFleetAgent
    let now: Int64
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
                if hovering && agent.lane != nil { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
            }
        return Group {
            if let lane = agent.lane {
                Button { onOpen(.lane(lane.id)) } label: { row }
                    .buttonStyle(.plain)
                    .onHover { hovering = $0 }
                    .accessibilityAddTraits(.isButton)
            } else {
                row
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
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
                    column(agent.tokens(), width: Self.tokensWidth)
                    column(agent.cost(), width: Self.costWidth)
                }
                Text(Self.second(agent, now: now))
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

    /// "“Design ov-503” · Not on a lane · Working · 22 min": what it was told, what
    /// it works for, and its state.
    static func second(_ agent: PlanFleetAgent, now: Int64) -> String { agent.second(now: now) }

    private var accessibility: String {
        [agent.who, Self.second(agent, now: now), agent.tokens().map { "\($0) tokens" }, agent.cost()]
            .compactMap { $0 }.joined(separator: ", ")
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
