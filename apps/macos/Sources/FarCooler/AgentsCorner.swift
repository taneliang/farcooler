import AgentKit
import SwiftUI

// The agents at work, in the orchestrator's top trailing corner (ov-829, the
// owner's pick of three options): a small glass control beside the pane's
// view switch, the two one group. It replaced a full-width glass card over
// the conversation that lined up with nothing.
//
// Glass, because it floats over the conversation as the switch does; small,
// because "in dense desktop environments, [capsules are] best used for
// standout actions" (WWDC25 356); and concentric with the paper's corner,
// because "for iPad and Mac, use a concentric shape that aligns with the
// window edge" (same session). The options and the research are in
// .claude/agent/reports/ov-829/.

/// The pane corner's metrics: one height for the agents control and the view
/// switch, its inset from the paper's edges, and the corner it draws.
enum PaneCorner {
    /// The controls' height and the switch's side.
    static let control: CGFloat = 28
    /// From the paper's top and trailing edges.
    static let inset: CGFloat = 6
    /// The paper's corner less the inset, never tighter than a control's.
    static var radius: CGFloat { max(Radius.medium - inset, Radius.small) }
    static var shape: RoundedRectangle { RoundedRectangle(cornerRadius: radius, style: .continuous) }
    /// What a conversation keeps clear at its top, so its first row starts
    /// under the corner's controls and a row scrolled up fades out before it
    /// reaches them: the controls and the inset, and a rhythm of air.
    static var clearance: CGFloat { inset + control + ColumnGrid.rhythm }
}

/// What a pane's corner carries beside its view switch: the agents control,
/// on the orchestrator's pane alone (`terminal`).
struct PaneCornerAccessory {
    let terminal: String
    let content: AnyView
}

extension EnvironmentValues {
    /// Set by the orchestrator's column for its own pane.
    @Entry var paneCornerAccessory: PaneCornerAccessory? = nil
}

/// The agents control: up to three dots, the ones that want you first, and
/// how many run, in words. One button; it opens the Agents panel.
struct AgentsCornerButton: View {
    let agents: RunningAgents
    let onChoose: (RunningAgent) -> Void
    @State private var viewing = false

    /// What VoiceOver says after the label: who wants you, when someone does.
    static func value(_ agents: RunningAgents) -> String {
        let waiting = agents.waiting
        return waiting == 0 ? "" : waiting == 1 ? "1 needs you" : "\(waiting) need you"
    }

    var body: some View {
        Button { viewing = true } label: {
            HStack(spacing: 6) {
                HStack(spacing: -2) {
                    ForEach(Array(agents.dots.enumerated()), id: \.offset) { _, mark in
                        GlanceMarkView(mark, inAppDiameter: 8)
                    }
                }
                Text(agents.summary)
                    .font(.system(size: 12, weight: .semibold).monospacedDigit())
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .fixedSize()
            }
            .padding(.horizontal, 9)
            .frame(height: PaneCorner.control)
            .contentShape(PaneCorner.shape)
        }
        .buttonStyle(.plain)
        .surface(.floating, in: PaneCorner.shape, interactive: true)
        .help("Show Agents")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(agents.summary)
        .accessibilityValue(Self.value(agents))
        .accessibilityHint("Shows the agents")
        .accessibilityAddTraits(.isButton)
        .popover(isPresented: $viewing, arrowEdge: .bottom) {
            AgentsPanel(agents: agents) { agent in
                viewing = false
                onChoose(agent)
            }
        }
        .identified("agents-corner")
    }
}

/// The corner's group: the accessory (the agents), then the switch when
/// there is one, on one piece of glass sampling, in from the paper's corner.
struct PaneCornerGroup<Switch: View>: View {
    let accessory: AnyView?
    @ViewBuilder let trailing: () -> Switch

    var body: some View {
        FloatingGroup(spacing: Spacing.tight) {
            HStack(spacing: Spacing.tight) {
                if let accessory { accessory }
                trailing()
            }
        }
        .padding(PaneCorner.inset)
        .identified("pane-corner")
    }
}

/// The view switch as the corner draws it beside the agents: the same
/// height, on glass.
struct CornerSwitchButton: View {
    let showing: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: showing ? "terminal" : "text.bubble")
                .font(.system(size: 12))
                .foregroundStyle(.primary)
                .frame(width: PaneCorner.control, height: PaneCorner.control)
                .contentShape(PaneCorner.shape)
        }
        .buttonStyle(.plain)
        .surface(.floating, in: PaneCorner.shape, interactive: true)
        .help(showing ? "Show Terminal" : "Show Conversation")
        .accessibilityLabel(showing ? "Show Terminal" : "Show Conversation")
        .identified("native-switch")
    }
}
