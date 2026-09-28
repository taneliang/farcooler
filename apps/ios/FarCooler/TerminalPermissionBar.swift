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
/// with the hook-ask id. The card comes down when the runner takes the answer;
/// if it doesn't, the card stays and says so.
///
/// **The card is the ask, keyed by its id.** It is drawn while the stream
/// holds an unresolved ask, and it goes when the daemon's `Resolved` for that
/// ask arrives, or a newer ask supersedes it: answered at the keyboard, from
/// the watch or another phone, or withdrawn when the hold ran out. Not when the
/// fleet stops saying blocked, which comes first; see `TerminalAskCard.reads`
/// for why the stream keeps reading until the `Resolved`.
///
/// **Read only while it can be seen**, and only while the pane is blocked or
/// still holds an ask: `AgentStream` polls every 700 ms, and a terminal pane is
/// usually not asking anything.
struct TerminalPermissionBar: View {
    let blocked: Bool
    let isVisible: Bool

    @StateObject private var stream: AgentStream
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.shellOverviewShowing) private var overviewShowing

    init(terminalID: String, core: ClientCore, blocked: Bool, isVisible: Bool) {
        self.blocked = blocked
        self.isVisible = isVisible
        _stream = StateObject(wrappedValue: AgentStream(terminal: terminalID, core: core))
    }

    private var onScreen: Bool { isVisible && !overviewShowing && scenePhase == .active }

    private var held: PendingPermission? { stream.transcript.pendingPermission }

    private var reads: Bool {
        TerminalAskCard.reads(onScreen: onScreen, blocked: blocked, asking: held != nil)
    }

    private var pending: PendingPermission? {
        TerminalAskCard.shows(asking: held != nil, caughtUp: stream.caughtUp) ? held : nil
    }

    var body: some View {
        VStack {
            if let pending {
                ApprovalCard(
                    pending: pending,
                    failure: stream.answering.sentence(for: pending.id),
                    sending: stream.answering.sending == pending.id
                ) { optionID in
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
                .id(pending.id)
            }
        }
        .frame(maxWidth: .infinity)
        .background(alignment: .bottomLeading) {
            // What the pane in front is asking, for `TerminalPermissionTests`.
            // Only the visible pane publishes it: the shell mounts every pane
            // in the fleet, and several probes would be a guess at which one
            // is being read. `blocked` is the fleet's word, so a test can see
            // an answer take the pane off blocked; `resolved` is what the
            // daemon recorded the last ask as ended by (`none` for the
            // keyboard or the hold running out), so a test can see which
            // answer landed.
            if isVisible {
                Rectangle()
                    .fill(Color.white.opacity(0.001))
                    .frame(width: 1, height: 1)
                    .accessibilityElement()
                    .accessibilityIdentifier("terminal-ask")
                    .accessibilityValue(
                        "blocked=\(blocked ? 1 : 0) ask=\(pending?.id ?? "-") "
                            + "resolved=\(resolvedWord)")
            }
        }
        .animation(.spring(response: 0.3, dampingFraction: 0.85), value: pending?.id)
        .task(id: reads) {
            if reads { stream.start() } else { stream.stop() }
        }
        .onDisappear { stream.stop() }
    }

    private var resolvedWord: String {
        guard let resolution = stream.lastResolution else { return "-" }
        return resolution.chosen.isEmpty ? "none" : resolution.chosen
    }
}
