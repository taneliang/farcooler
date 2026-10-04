import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The agent chat (ov-223): messages, a tool call waiting on approval, a
/// running subagent, a gap notice, and the plan, queue and composer that rest
/// over the transcript, plus the approval and failure cards. A rendering,
/// written by `VisualSpecimen`.
@MainActor
struct AgentChatSpecimenTests {
    @Test("Write the agent chat sheet")
    func writeSheet() throws {
        try VisualSpecimen.shoot("agent-chat", size: CGSize(width: 560, height: 980), Specimen())
    }

    private static func transcript() -> Transcript {
        var transcript = Transcript()
        let options = [
            PermissionOption(id: "allow", name: "Allow", kind: "allow_once"),
            PermissionOption(id: "deny", name: "Deny", kind: "reject_once"),
        ]
        let events: [AgentEvent] = [
            .message(role: .user, text: "Move the board's rules into spacing and show me the diff.", parent: nil),
            .message(role: .agent, text: "I'll start with the navigator, then the Changes pane.", parent: nil),
            .toolCall(
                id: "t1", title: "Read DesignTokens.swift", kind: "read", status: .completed, locations: ["DesignTokens.swift"],
                parent: nil, subagent: false),
            .toolUpdate(
                id: "t1", status: .completed, title: nil, content: "public enum Radius {\n    public static let small: CGFloat = 6\n}",
                diff: nil, locations: [], parent: nil, subagent: nil),
            .toolCall(
                id: "t2", title: "Run swift test", kind: "execute", status: .pending, locations: [], parent: nil, subagent: false),
            .permission(id: "p1", toolCall: "t2", options: options),
            .toolCall(
                id: "s1", title: "Explore the Changes pane", kind: "think", status: .inProgress, locations: [], parent: nil,
                subagent: true),
            .toolCall(
                id: "s1c", title: "Grep Divider()", kind: "search", status: .completed, locations: [], parent: "s1",
                subagent: false),
            .gap(.loadEmpty),
            .plan(entries: [
                PlanEntry(content: "Convert the navigator rows", priority: "high", status: "completed"),
                PlanEntry(content: "Convert the Changes pane", priority: "high", status: "in_progress"),
                PlanEntry(content: "Delete the baseline", priority: "low", status: "pending"),
            ]),
            .promptQueue(items: [QueuedPrompt(id: "q1", text: "Also check dark mode.")]),
        ]
        transcript.apply(events.enumerated().map { Sequenced(seq: UInt64($0.offset + 1), event: $0.element) })
        return transcript
    }

    private struct Specimen: View {
        @Environment(\.colorScheme) private var scheme
        private let transcript = AgentChatSpecimenTests.transcript()
        @StateObject private var stream = AgentStream(terminal: "specimen")
        @State private var prefill: String?
        @State private var appendix: String?

        var body: some View {
            let terminal = Terminal(id: "a", short: "a", title: "Agent", preset: "claude", state: "running", epoch: 0)
            VStack(alignment: .leading, spacing: 8) {
                ForEach(transcript.rows) { row in
                    AgentRowView(
                        row: row, isLast: false,
                        pending: transcript.pendingPermission.flatMap { row.id == pendingRow ? $0 : nil }, onAnswer: { _ in })
                }
                Spacer(minLength: 0)
                if let pending = transcript.pendingPermission {
                    ApprovalCard(pending: pending, onChoose: { _ in })
                }
                AgentFailureRow(failure: .notAuthenticated, agent: "Claude").padding(-16)
                GlassEffectContainer(spacing: 8) {
                    VStack(spacing: 8) {
                        PlanPanel(entries: transcript.plan).padding(.horizontal, 10)
                        ForEach(transcript.queue) { queued in
                            QueuedRow(queued: queued, onEdit: { _ in }, onCancel: {}, onSteer: {}).padding(.horizontal, 10)
                        }
                        AgentComposer(
                            stream: stream, terminal: terminal, isFocused: true, unreachable: nil, searchFiles: { _ in [] },
                            width: 560, prefill: $prefill, appendix: $appendix
                        )
                        .padding(10)
                    }
                }
            }
            .padding(16)
            .background(WorkspaceStyle.document)
        }

        private var pendingRow: Int? {
            transcript.rows.first {
                if case .tool(let tool) = $0.kind { return tool.id == transcript.pendingPermission?.toolCall }
                return false
            }?.id
        }
    }
}
