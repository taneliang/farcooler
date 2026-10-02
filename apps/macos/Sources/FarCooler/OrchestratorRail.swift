import AgentKit
import SwiftUI

/// The rail a workspace's orchestrator shrinks to beside a task or a
/// worktree opened (ov-79), drawn so it says what it is (ov-84): the
/// orchestrator's icon with its state on it, "Orchestrator" set sideways
/// down it, and a chevron pointing the way it opens.
///
/// The state is worked out here as a value, from what the conversation
/// column's header reads (`ConversationHeader`): the seat's status and the
/// unread dot, and the workspace's needs-you count the rail's dot always
/// counted. `OrchestratorRailTests` pins it.
enum OrchestratorRail {
    enum State: Equatable {
        /// No orchestrator runs this workspace.
        case none
        case starting
        case working
        /// Something in this workspace is waiting on you, the orchestrator
        /// included.
        case needsYou
        /// It finished a turn nobody has seen.
        case unread
        case idle
        /// Its pane exited, was lost, or can't be read.
        case stopped
    }

    /// The rail's state, from the orchestrator's seat and how many of this
    /// workspace's items wait on you.
    static func state(seat: BoardPane?, waiting: Int) -> State {
        guard let seat else { return .none }
        let status = seat.terminal.status
        if waiting > 0 || status == .blocked { return .needsYou }
        if ConversationColumn.unread(seat) { return .unread }
        switch status {
        case .starting: return .starting
        case .working: return .working
        case .lost, .exited, .failed, .failedRun, .unreadable: return .stopped
        default: return .idle
        }
    }

    /// What VoiceOver says the state is, after "Orchestrator, ".
    static func spoken(_ state: State) -> String {
        switch state {
        case .none: "not running"
        case .starting: "starting"
        case .working: "working"
        case .needsYou: "needs you"
        case .unread: "finished a turn you haven’t seen"
        case .idle: "idle"
        case .stopped: "stopped"
        }
    }

    static func accessibilityLabel(_ state: State) -> String { "Orchestrator, \(spoken(state))" }

    /// The sideways label: with the agent's name when there's room for it.
    static func titles(agent: String?) -> [String] {
        guard let agent, !agent.isEmpty else { return ["Orchestrator"] }
        return ["Orchestrator · \(agent)", "Orchestrator"]
    }

    static func help(open: Bool) -> String {
        open ? "Hide Orchestrator (⌥⌘1)" : "Show Orchestrator (⌥⌘1)"
    }

    static func action(open: Bool) -> String { open ? "Hide" : "Show" }
}

/// The rail itself. A click, or its accessibility action, toggles the
/// orchestrator open over what's opened, at any time, mid-flight included.
struct OrchestratorRailView: View {
    let state: OrchestratorRail.State
    let agent: String?
    let open: Bool
    var onToggle: () -> Void

    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        Button(action: onToggle) {
            VStack(spacing: 10) {
                icon
                ViewThatFits(in: .vertical) {
                    ForEach(OrchestratorRail.titles(agent: agent), id: \.self) { title in
                        Sideways {
                            Text(title)
                                .font(.system(size: 11, weight: .medium))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .fixedSize()
                        }
                    }
                }
                .frame(maxHeight: .infinity, alignment: .top)
                .clipped()
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(open ? 180 : 0))
                    .animation(OrchestratorPeek.spring, value: open)
            }
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background {
            Rectangle().fill(.quaternary).opacity(hovering ? 1 : open ? 0.5 : 0)
        }
        .background(WorkspaceStyle.canvas)
        .onHover { hovering = $0 }
        .pointerStyle(.link)
        .help(OrchestratorRail.help(open: open))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(OrchestratorRail.accessibilityLabel(state))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction(named: OrchestratorRail.action(open: open), onToggle)
        .accessibilityIdentifier("orchestrator-rail")
    }

    /// The orchestrator's icon, with its state on it: a spinner while it
    /// works or starts, the amber dot when it needs you or has news.
    private var icon: some View {
        Image(systemName: "person.wave.2")
            .font(.system(size: 13))
            .foregroundStyle(state == .none || state == .stopped ? .tertiary : .secondary)
            .frame(width: 20, height: 18)
            .overlay(alignment: .bottomTrailing) {
                switch state {
                case .working, .starting:
                    ProgressView()
                        .controlSize(.mini)
                        .scaleEffect(0.6)
                        .frame(width: 9, height: 9)
                        .background(Circle().fill(WorkspaceStyle.canvas).padding(-1))
                        .offset(x: 3, y: 3)
                case .needsYou, .unread:
                    Circle()
                        .fill(GlancePalette.amber(scheme))
                        .frame(width: 7, height: 7)
                        .background(Circle().fill(WorkspaceStyle.canvas).padding(-1.5))
                        .offset(x: 2, y: 2)
                case .none, .idle, .stopped:
                    EmptyView()
                }
            }
    }
}

/// Its one subview turned a quarter clockwise, laid out as it then stands:
/// as wide as the text is tall, and as tall as it's long.
private struct Sideways<Content: View>: View {
    @ViewBuilder let content: () -> Content

    var body: some View {
        SidewaysLayout { content().rotationEffect(.degrees(90)) }
    }
}

private struct SidewaysLayout: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let text = subviews.first else { return .zero }
        let size = text.sizeThatFits(.unspecified)
        return CGSize(width: size.height, height: size.width)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard let text = subviews.first else { return }
        let size = text.sizeThatFits(.unspecified)
        text.place(
            at: CGPoint(x: bounds.midX, y: bounds.midY), anchor: .center,
            proposal: ProposedViewSize(width: size.width, height: size.height))
    }
}
