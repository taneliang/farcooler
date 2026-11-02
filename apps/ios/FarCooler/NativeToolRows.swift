import SwiftUI

// The agent's machinery in the conversation view (ov-452), set below its
// words as the Mac's is (`NativeToolRows.swift` there): tool calls, runs of
// them folded into one line, and the task list. Secondary and one line until
// tapped; each call opens to what it was asked and what came back.

/// The weight every machinery row is set in: a step under the reply's body.
enum MachineryStyle {
    static let font: Font = .callout

    /// A line that is cut at the default size takes its words in full from
    /// `.xLarge` up (more than `.large`, the default): the audit's "may be clipped at larger Dynamic Type
    /// sizes", and the person who chose the size (ov-685).
    static func lines(_ type: DynamicTypeSize, _ base: Int) -> Int { type > .large ? 4 : base }
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
            .foregroundStyle(.quietInk)
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
    @Environment(\.dynamicTypeSize) private var type

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            OpeningHeader(
                open: open, opens: tool.opens,
                accessibilityLabel: tool.presentationAccessibility,
                toggle: { withAnimation(.snappy) { open.toggle() } }
            ) {
                if type.isAccessibilitySize || tool.operation != nil {
                    // The words under the name, where they may take lines:
                    // beside it they were cut to a few letters (ov-685).
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            NativeStatusMark(status: tool.presentationStatus)
                            Text(tool.presentationName).fontWeight(.medium)
                            Spacer(minLength: Spacing.group)
                            NativeRunTime(startedMs: tool.startedMs, endedMs: tool.endedMs)
                        }
                        Text(verbatim: tool.presentationSummary).lineLimit(4).truncationMode(.middle)
                    }
                } else {
                    NativeStatusMark(status: tool.presentationStatus)
                    Text(tool.presentationName).fontWeight(.medium)
                    Text(verbatim: tool.presentationSummary).lineLimit(1).truncationMode(.middle).layoutPriority(-1)
                    Spacer(minLength: Spacing.group)
                    NativeRunTime(startedMs: tool.startedMs, endedMs: tool.endedMs)
                }
            }
            if let operation = tool.operation { LiteralOperationCaption(operation: operation) }
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
            if let input = tool.input { part(tool.operation == nil ? "Input" : "Input excerpt", input) }
            if !tool.diff.isEmpty { NativeDiff(hunks: tool.diff) }
            if let operation = tool.operation, tool.result == nil { Text(operation.outputAvailable ? "Output is empty" : "Output unavailable").font(.caption) }
            if let result = tool.result { part(tool.operation == nil ? (tool.status == .failed ? "Error" : "Result") : (tool.presentationStatus == .failed ? "Error excerpt" : "Output excerpt"), result) }
        }
        .accessibilityIdentifier("native-tool-detail")
    }

    private func part(_ title: String, _ text: String) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(title).font(.caption.weight(.medium)).foregroundStyle(.quietInk)
            NativeOutputText(text: text)
                .font(.caption.monospaced())
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
    @Environment(\.dynamicTypeSize) private var type

    var body: some View {
        let tools = boxes.compactMap { box -> AgentRow.Tool? in
            if case .tool(let tool) = box.row.kind { return tool }
            return nil
        }
        let title = AgentConversation.groupTitle(tools)
        let purpose = AgentConversation.groupPurpose(tools)
        let status = AgentConversation.groupStatus(tools)
        VStack(alignment: .leading, spacing: Spacing.tight) {
            OpeningHeader(open: open, opens: true, accessibilityLabel: "\(title): \(purpose), \(AgentConversation.recognizedGroupLabel(tools) ?? "")", toggle: { withAnimation(.snappy) { open.toggle() } }) {
                if type.isAccessibilitySize {
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            NativeStatusMark(status: status)
                            Text(title).fontWeight(.medium)
                            Spacer(minLength: Spacing.group)
                            NativeRunTime(startedMs: tools.first?.startedMs, endedMs: status == .running ? nil : tools.compactMap(\.endedMs).max())
                        }
                        Text(purpose).lineLimit(4).truncationMode(.tail)
                    }
                } else {
                    NativeStatusMark(status: status)
                    Text(title).fontWeight(.medium).fixedSize()
                    Text(purpose).lineLimit(1).truncationMode(.tail).layoutPriority(-1)
                    Spacer(minLength: Spacing.group)
                    NativeRunTime(startedMs: tools.first?.startedMs, endedMs: status == .running ? nil : tools.compactMap(\.endedMs).max())
                }
            }
            if let provenance = AgentConversation.recognizedGroupLabel(tools) { Text(provenance).font(.caption).fixedSize(horizontal: false, vertical: true) }
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

/// A tool's input or result: range-selecting where the conversation selects
/// ranges (ov-609), `Text` with the whole-text Copy elsewhere.
struct NativeOutputText: View {
    let text: String
    @Environment(\.selectsRanges) private var selectsRanges

    var body: some View {
        if selectsRanges {
            SelectableText(verbatim: text)
        } else {
            Text(text).textSelection(.enabled)
        }
    }
}

private struct LiteralOperationCaption: View {
    let operation: LiteralOperation
    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text("Recognized command · \(operation.lifecycle.words)").font(.caption).fixedSize(horizontal: false, vertical: true)
            if let preview = operation.preview {
                NativeOutputText(text: preview).font(.callout).environment(\.layoutDirection, .leftToRight)
                if operation.previewClipped { Text("Content excerpt clipped").font(.caption) }
                if operation.containsControls { Text("Contains direction or control characters").font(.caption) }
            } else if operation.kind.hasPrefix("task_note_") || operation.kind == "message_orchestrator" {
                Text("Preview unavailable in excerpt").font(.caption)
            }
            if operation.inputClipped || operation.outputClipped { Text("Input or output excerpt clipped").font(.caption) }
        }.foregroundStyle(.quietInk)
    }
}
