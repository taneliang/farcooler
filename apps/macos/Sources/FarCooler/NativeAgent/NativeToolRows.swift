import AgentKit
import SwiftUI

// What the native view (ov-372) draws for the agent's machinery, set below
// its words (ov-452): tool calls, runs of them folded into one line, and the
// task list. Secondary, unfilled and one line until opened, so the reply is
// the first thing the eye lands on; each opens, as the old chat's tool
// blocks did (`AgentRows.swift`), to what was asked and what came back.

/// The weight every machinery row is set in: a step under the reply's body.
enum MachineryStyle {
    static let font: Font = .callout
}

/// One tool call: its state, its name, what it was for and how long it took,
/// opening to its input, its result and an edit's diff.
struct NativeToolRow: View {
    let id: String
    let tool: AgentRow.Tool
    @State private var open = false

    var body: some View {
        CollapsibleSection(
            id: "tool.\(id)", metrics: .inline, isExpanded: $open, canExpand: tool.opens,
            accessibilityLabel: [tool.name, tool.summary, NativeCopy.status(tool.status)].filter { !$0.isEmpty }.joined(separator: ", "),
            label: { _ in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    NativeStatusMark(status: tool.status)
                    Text(tool.name).fontWeight(.medium)
                    Text(tool.summary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .layoutPriority(-1)
                    Spacer(minLength: Spacing.group)
                    RunTimeChip(startedMs: tool.startedMs, endedMs: tool.endedMs)
                }
                .font(MachineryStyle.font)
                .foregroundStyle(.secondary)
            },
            accessory: { EmptyView() }
        ) {
            ToolDetail(tool: tool).padding(.leading, 14 + 14 + Spacing.group)
        }
        .identified("native-tool-\(id)")
    }
}

/// What a call opens to: its input, its result, and an edit's hunks.
struct ToolDetail: View {
    let tool: AgentRow.Tool

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let input = tool.input { part("Input", input) }
            if !tool.diff.isEmpty { NativeDiff(hunks: tool.diff) }
            if let result = tool.result { part(tool.status == .failed ? "Error" : "Result", result) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .identified("native-tool-detail")
    }

    private func part(_ title: String, _ text: String) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(title)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            Text(text)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(Spacing.group)
                .surface(.inset, in: .control)
        }
    }
}

/// An edit's hunks, as unified-diff lines.
struct NativeDiff: View {
    let hunks: [AgentRow.Hunk]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(hunks.indices, id: \.self) { h in
                ForEach(hunks[h].lines.indices, id: \.self) { l in
                    let line = hunks[h].lines[l]
                    Text(line.isEmpty ? " " : line)
                        .font(.caption.monospaced())
                        .foregroundStyle(line.hasPrefix("-") ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .textSelection(.enabled)
        .padding(Spacing.group)
        .surface(.inset, in: .control)
    }
}

/// A run of tool calls as one line, as claude's own "Ran 3 shell commands":
/// how many, and what for, from the calls' descriptions. Opens to each call,
/// which opens in turn. Each member is its own box, so a call finishing
/// redraws this line and that call, nothing else.
struct ToolGroupRow: View {
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
        CollapsibleSection(
            id: "toolgroup.\(id)", metrics: .inline, isExpanded: $open,
            accessibilityLabel: "\(title): \(purpose)",
            label: { _ in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    NativeStatusMark(status: AgentConversation.groupStatus(tools))
                    Text(title).fontWeight(.medium)
                    Text(purpose)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(-1)
                    Spacer(minLength: Spacing.group)
                    RunTimeChip(
                        startedMs: tools.first?.startedMs,
                        endedMs: AgentConversation.groupStatus(tools) == .running ? nil : tools.compactMap(\.endedMs).max())
                }
                .font(MachineryStyle.font)
                .foregroundStyle(.secondary)
            },
            accessory: { EmptyView() }
        ) {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                ForEach(boxes) { box in
                    NativeGroupMember(box: box)
                }
            }
            .padding(.leading, 14)
        }
        .identified("native-tool-group")
    }
}

/// One row inside a group: a call, or the thinking between two.
private struct NativeGroupMember: View {
    let box: AgentRowBox

    var body: some View {
        switch box.row.kind {
        case .tool(let tool): NativeToolRow(id: box.id, tool: tool)
        case .thinking(let thinking): NativeThinkingRow(thinking: thinking).padding(.leading, 14)
        default: EmptyView()
        }
    }
}

/// The agent's task list as a checklist (ov-452): each task's state as a
/// glyph, a finished one struck through.
struct NativeTasksRow: View {
    let tasks: AgentRow.Tasks

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(spacing: Spacing.group) {
                Text("Tasks").fontWeight(.medium)
                Text(AgentConversation.taskProgress(tasks))
            }
            .font(MachineryStyle.font)
            .foregroundStyle(.secondary)
            ForEach(tasks.items.indices, id: \.self) { i in
                let item = tasks.items[i]
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    TaskMark(status: item.status)
                    Text(item.subject)
                        .strikethrough(item.status == "Completed")
                        .foregroundStyle(item.status == "InProgress" ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .font(MachineryStyle.font)
                .accessibilityElement(children: .combine)
                .accessibilityValue(AgentConversation.taskState(item))
            }
        }
        .identified("native-tasks")
    }
}

private struct TaskMark: View {
    let status: String

    var body: some View {
        Group {
            switch status {
            case "Completed": Image(systemName: "checkmark.circle.fill").foregroundStyle(.secondary)
            case "InProgress": Image(systemName: "circle.lefthalf.filled").foregroundStyle(.tint)
            default: Image(systemName: "circle").foregroundStyle(.tertiary)
            }
        }
        .frame(width: 14)
        .accessibilityHidden(true)
    }
}

/// A message that may be long: its first lines, and Show More for the rest.
/// Whether it's long is estimated (`AgentConversation.isLong`), so nothing is
/// measured to decide.
struct FoldedText: View {
    let text: String
    var lines = AgentConversation.collapsedLines
    @State private var open = false

    var body: some View {
        let long = AgentConversation.isLong(text, lines: lines)
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(text)
                .textSelection(.enabled)
                .lineLimit(long && !open ? lines : nil)
                .fixedSize(horizontal: false, vertical: true)
            if long {
                Button(open ? "Show Less" : "Show More") { open.toggle() }
                    .buttonStyle(.link)
                    .font(.caption)
                    .identified("native-show-more")
            }
        }
    }
}
