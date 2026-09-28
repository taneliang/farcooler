import SwiftUI

/// Allow and Deny for a claude TUI pane's permission ask, over its terminal.
///
/// When claude asks for permission in a pane, the daemon holds claude's
/// `PermissionRequest` hook and records the ask in the pane's event ring as a
/// `Permission` with a `hook-ask-` id. A chat pane already draws that from
/// `AgentView`; a terminal pane had nothing, so on the phone the only way to
/// answer was to type into claude's own dialog.
///
/// This reads the same ring through `AgentStream`, and draws the same
/// `ApprovalCard` a chat pane draws, so the question looks the same in both.
/// A tap goes through `AgentStream.answer`, which is `terminal.agent_answer`
/// with the hook-ask id, and takes the card down at once.
///
/// **Read only while the pane is blocked and on screen.** `AgentStream` polls
/// every 700 ms, and a terminal pane is usually not asking anything. The
/// daemon's `blocked` for a claude pane comes off its screen, which shows
/// claude's dialog for as long as the hook is held.
///
/// **Resolved somewhere else, the card goes.** An ask answered at the
/// keyboard, from the watch, from another phone or by the hold running out
/// ends in the daemon's `Resolved`, which `Transcript` applies by clearing the
/// pending permission. And the card is drawn only while the fleet says the
/// pane is blocked, so a stream stopped before its `Resolved` arrived cannot
/// leave the card up.
struct TerminalPermissionBar: View {
    let blocked: Bool
    let isVisible: Bool

    @StateObject private var stream: AgentStream

    init(terminalID: String, core: ClientCore, blocked: Bool, isVisible: Bool) {
        self.blocked = blocked
        self.isVisible = isVisible
        _stream = StateObject(wrappedValue: AgentStream(terminal: terminalID, core: core))
    }

    private var pending: PendingPermission? {
        blocked ? stream.transcript.pendingPermission : nil
    }

    var body: some View {
        VStack {
            if let pending {
                ApprovalCard(pending: pending) { optionID in
                    Task { await stream.answer(pending.id, optionID) }
                }
                // The card's own fill is a tint for a chat's plain ground. Over
                // terminal text it needs an opaque ground of its own to be read.
                .background(
                    Color(uiColor: .systemBackground),
                    in: RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius)
                )
                .shadow(color: .black.opacity(0.25), radius: 12, y: 4)
                .padding(PaneMetrics.edge)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("terminal-permission")
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .frame(maxWidth: .infinity)
        .background(alignment: .bottomLeading) {
            // What the pane in front is asking, for `TerminalPermissionTests`.
            // Only the visible pane publishes it: the shell mounts every pane
            // in the fleet, and several probes would be a guess at which one
            // is being read. `blocked` is the fleet's word, so a test can see
            // an answer take the pane off blocked.
            if isVisible {
                Rectangle()
                    .fill(Color.white.opacity(0.001))
                    .frame(width: 1, height: 1)
                    .accessibilityElement()
                    .accessibilityIdentifier("terminal-ask")
                    .accessibilityValue(
                        "blocked=\(blocked ? 1 : 0) ask=\(pending?.id ?? "-")")
            }
        }
        .animation(.spring(response: 0.3, dampingFraction: 0.85), value: pending?.id)
        .task(id: blocked && isVisible) {
            if blocked && isVisible { stream.start() } else { stream.stop() }
        }
        .onDisappear { stream.stop() }
    }
}
