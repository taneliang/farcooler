import SwiftUI

/// The terminal detail pane: a header of facts, the live surface, a hint bar.
struct TerminalPane: View {
    @ObservedObject private var preferences = Preferences.shared

    let terminal: Terminal
    let workspace: Workspace
    let binary: String?
    let environment: [String: String]
    let onGeometry: (Int, Int) async -> Void
    let onAction: (TerminalAction) -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }
    private var isLive: Bool { kind == .running || kind == .starting }

    var body: some View {
        VStack(spacing: 0) {
            header

            if isLive {
                TerminalSurface(
                    terminal: terminal.short,
                    binary: binary,
                    environment: environment,
                    onResize: onGeometry,
                    fontRevision: preferences.revision
                )
                .id(terminal.id)
            } else {
                inactive
            }

            footer
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            StateDot(state: terminal.state)

            VStack(alignment: .leading, spacing: 1) {
                Text(terminal.title).font(.system(size: 14, weight: .semibold))
                HStack(spacing: 6) {
                    Text(workspace.task)
                    Text("·")
                    Text(workspace.branch)
                    if terminal.epoch > 0 {
                        Text("·")
                        Text("epoch \(terminal.epoch)")
                    }
                }
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }

            Spacer(minLength: 12)

            switch kind {
            case .lost:
                Button("Dismiss loss") { onAction(.dismissLost) }
                Button("Restart") { onAction(.restart) }
                    .buttonStyle(.borderedProminent)
            case .exited, .error:
                Button("Restart") { onAction(.restart) }
                    .buttonStyle(.borderedProminent)
            default:
                Button("Restart") { onAction(.restart) }
                Button("Stop", role: .destructive) { onAction(.stop) }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
        .background(.bar)
    }

    private var inactive: some View {
        VStack(spacing: 12) {
            Spacer()
            Image(systemName: kind == .lost ? "questionmark.circle" : "stop.circle")
                .font(.system(size: 38))
                .foregroundStyle(kind == .lost ? Color.red.opacity(0.75) : .secondary)
            Text(headline).font(.title3.weight(.medium))
            Text(explanation)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 470)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
    }

    private var headline: String {
        switch kind {
        case .lost: return "Lost"
        case .exited: return "Exited"
        case .error: return "Never started"
        default: return terminal.state
        }
    }

    private var explanation: String {
        switch kind {
        case .lost:
            return
                "This terminal was expected to be running, but no live tagged pane proves it. "
                + "Overnight will not guess: it says lost rather than claiming an exit. "
                + "Restart begins a new epoch, or dismiss to acknowledge the loss without "
                + "recording an exit that was never observed."
        case .exited:
            return "The command exited and Overnight observed it. Restart begins a new epoch."
        case .error:
            return "Creation never established a live runtime."
        default:
            return "Not accepting input."
        }
    }

    @ViewBuilder
    private var footer: some View {
        Divider()
        HStack(spacing: 6) {
            Image(systemName: "keyboard").font(.system(size: 10))
            Text(
                isLive
                    ? "Typing goes straight to the terminal. Ctrl-C, arrows, Tab and Esc all pass through."
                    : "This terminal is not accepting input."
            )
            Spacer()
            Text(terminal.preset).foregroundStyle(.tertiary)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.vertical, 7)
    }
}
