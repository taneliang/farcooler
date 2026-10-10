import SwiftUI

// The agent's machinery in the conversation view (ov-452), set below its
// words as the Mac's is (`NativeToolRows.swift` there): tool calls, runs of
// them folded into one line, and the task list. Secondary and one line until
// tapped; each call opens to what it was asked and what came back.

/// The weight every machinery row is set in: a step under the reply's body.
enum MachineryStyle {
    static let font: Font = .callout
}

/// A row's header that opens what's under it: the whole line is the target,
/// with a chevron at the leading edge that turns as it opens.
private struct OpeningHeader<Label: View>: View {
    let open: Bool
    let opens: Bool
    let accessibilityLabel: String
    let toggle: () -> Void
    @ViewBuilder let label: () -> Label

    var body: some View {
        Button(action: toggle) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .rotationEffect(.degrees(open ? 90 : 0))
                    .opacity(opens ? 1 : 0)
                    .frame(width: 10)
                label()
            }
            .font(MachineryStyle.font)
            .foregroundStyle(.secondary)
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!opens)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityValue(opens ? (open ? "Expanded" : "Collapsed") : "")
    }
}

/// One tool call, opening to its input, its result and an edit's diff.
struct NativeToolRow: View {
    let id: String
    let tool: AgentRow.Tool
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            OpeningHeader(
                open: open, opens: tool.opens,
                accessibilityLabel: [tool.name, tool.summary].filter { !$0.isEmpty }.joined(separator: ", "),
                toggle: { withAnimation(.snappy) { open.toggle() } }
            ) {
                NativeStatusMark(status: tool.status)
                Text(tool.name).fontWeight(.medium)
                Text(tool.summary).lineLimit(1).truncationMode(.middle).layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                NativeRunTime(startedMs: tool.startedMs, endedMs: tool.endedMs)
            }
            if open {
                NativeToolDetail(tool: tool)
                    .padding(.leading, 10 + Spacing.group)
                    .transition(.opacity)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-tool-\(id)")
    }
}

/// What a call opens to.
private struct NativeToolDetail: View {
    let tool: AgentRow.Tool

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let input = tool.input { part("Input", input) }
            if !tool.diff.isEmpty { NativeDiff(hunks: tool.diff) }
            if let result = tool.result { part(tool.status == .failed ? "Error" : "Result", result) }
        }
        .accessibilityIdentifier("native-tool-detail")
    }

    private func part(_ title: String, _ text: String) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(title).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            Text(text)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(Spacing.group)
                .surface(.inset, in: .control)
        }
    }
}

/// A run of tool calls as one line: how many, and what for. Opens to each.
struct NativeToolGroupRow: View {
    let id: String
    let boxes: [AgentRowBox]
    @State private var open = false

    var body: some View {
        let tools = boxes.compactMap { box -> AgentRow.Tool? in
            if case .tool(let tool) = box.row.kind { return tool }
            return nil
        }
        let title = AgentConversation.groupTitle(tools)
        let purpose = AgentConversation.groupPurpose(tools)
        let status = AgentConversation.groupStatus(tools)
        VStack(alignment: .leading, spacing: Spacing.tight) {
            OpeningHeader(open: open, opens: true, accessibilityLabel: "\(title): \(purpose)", toggle: { withAnimation(.snappy) { open.toggle() } }) {
                NativeStatusMark(status: status)
                Text(title).fontWeight(.medium).fixedSize()
                Text(purpose).lineLimit(1).truncationMode(.tail).layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                NativeRunTime(startedMs: tools.first?.startedMs, endedMs: status == .running ? nil : tools.compactMap(\.endedMs).max())
            }
            if open {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(boxes) { box in
                        switch box.row.kind {
                        case .tool(let tool): NativeToolRow(id: box.id, tool: tool)
                        case .thinking(let thinking): NativeThinkingRow(thinking: thinking).padding(.leading, 10 + Spacing.group)
                        default: EmptyView()
                        }
                    }
                }
                .padding(.leading, 10 + Spacing.group)
                .transition(.opacity)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-tool-group")
    }
}

/// The agent's task list as a checklist.
struct NativeTasksRow: View {
    let tasks: AgentRow.Tasks

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(spacing: Spacing.group) {
                Text("Tasks").fontWeight(.medium)
                Text(AgentConversation.taskProgress(tasks))
            }
            .foregroundStyle(.secondary)
            ForEach(tasks.items.indices, id: \.self) { i in
                let item = tasks.items[i]
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    Group {
                        switch item.status {
                        case "Completed": Image(systemName: "checkmark.circle.fill").foregroundStyle(.secondary)
                        case "InProgress": Image(systemName: "circle.lefthalf.filled").foregroundStyle(.tint)
                        default: Image(systemName: "circle").foregroundStyle(.tertiary)
                        }
                    }
                    .accessibilityHidden(true)
                    Text(item.subject)
                        .strikethrough(item.status == "Completed")
                        .foregroundStyle(item.status == "InProgress" ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                }
                .accessibilityElement(children: .combine)
                .accessibilityValue(AgentConversation.taskState(item))
            }
        }
        .font(MachineryStyle.font)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-tasks")
    }
}

/// A message that may be long: its first lines, and Show More for the rest.
/// Whether it runs past them is measured at the width it's drawn at: the
/// text as shown against the same text unlimited, so a long line on a narrow
/// pane folds as surely as many short ones (ov-452 review).
struct FoldedText: View {
    let text: String
    var lines = AgentConversation.collapsedLines
    @State private var open = false
    @State private var whole: CGFloat = 0
    @State private var shown: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(text)
                .textSelection(.enabled)
                .lineLimit(open ? nil : lines)
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shown = $0 }
                .background(alignment: .topLeading) {
                    Text(text)
                        .fixedSize(horizontal: false, vertical: true)
                        .hidden()
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { whole = $0 }
                }
            if open || whole > shown + 1 {
                Button(open ? "Show Less" : "Show More") { withAnimation(.snappy) { open.toggle() } }
                    .font(.caption.weight(.semibold))
                    .frame(minHeight: 44)
                    .accessibilityIdentifier("native-show-more")
            }
        }
    }
}
