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
                accessibilityLabel: [tool.name, tool.summary].filter { !$0.isEmpty }.joined(separator: ", "),
                toggle: { withAnimation(.snappy) { open.toggle() } }
            ) {
                if type.isAccessibilitySize {
                    // The words under the name, where they may take lines:
                    // beside it they were cut to a few letters (ov-685).
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            NativeStatusMark(status: tool.status)
                            Text(tool.name).fontWeight(.medium)
                            Spacer(minLength: Spacing.group)
                            NativeRunTime(startedMs: tool.startedMs, endedMs: tool.endedMs)
                        }
                        Text(tool.summary).lineLimit(4).truncationMode(.middle)
                    }
                } else {
                    NativeStatusMark(status: tool.status)
                    Text(tool.name).fontWeight(.medium)
                    Text(tool.summary).lineLimit(1).truncationMode(.middle).layoutPriority(-1)
                    Spacer(minLength: Spacing.group)
                    NativeRunTime(startedMs: tool.startedMs, endedMs: tool.endedMs)
                }
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
            OpeningHeader(open: open, opens: true, accessibilityLabel: "\(title): \(purpose)", toggle: { withAnimation(.snappy) { open.toggle() } }) {
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
            .foregroundStyle(.quietInk)
            ForEach(tasks.items.indices, id: \.self) { i in
                let item = tasks.items[i]
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    Group {
                        switch item.status {
                        case "Completed": Image(systemName: "checkmark.circle.fill").foregroundStyle(.quietInk)
                        case "InProgress": Image(systemName: "circle.lefthalf.filled").foregroundStyle(.tint)
                        default: Image(systemName: "circle").foregroundStyle(.tertiary)
                        }
                    }
                    .accessibilityHidden(true)
                    Text(item.subject)
                        .strikethrough(item.status == "Completed")
                        .foregroundStyle(item.status == "InProgress" ? AnyShapeStyle(.primary) : AnyShapeStyle(Color.quietInk))
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
    /// The conversation selects ranges (ov-609): a long press on a word
    /// selects it, with handles, rather than copying the whole message.
    @Environment(\.selectsRanges) private var selectsRanges
    @Environment(\.selectableColor) private var selectableColor

    var body: some View {
        // Laid out in one pass, Show More and all (ov-675): a lazy list
        // builds a row just above the screen as the reader scrolls up, and a
        // row that changed height on its second pass moved every row under it.
        FoldStack(fold: .lines, open: open) {
            words
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shown = $0 }
            Button(open ? "Show Less" : "Show More") { withAnimation(.snappy) { open.toggle() } }
                // Its state, as a disclosure says it (ov-788).
                .accessibilityValue(open ? "Expanded" : "Collapsed")
                // On the person's bubble, the bubble's own text color,
                // underlined as on the Mac (review 1).
                .tint(selectableColor.map { Color(uiColor: $0) })
                .underline(selectableColor != nil)
                .font(.caption.weight(.semibold))
                .frame(minHeight: 44)
                .accessibilityIdentifier("native-show-more")
                .foldButton(shown: open || whole > shown + 1)
            Text(verbatim: text)
                .fixedSize(horizontal: false, vertical: true)
                .hidden()
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { whole = $0 }
        }
    }
}

extension FoldedText {
    @ViewBuilder
    fileprivate var words: some View {
        if selectsRanges {
            SelectableText(
                verbatim: text, color: selectableColor ?? .label, lineLimit: open ? nil : lines,
                tint: selectableColor)
        } else {
            Text(verbatim: text)
                .textSelection(.enabled)
                .lineLimit(open ? nil : lines)
        }
    }
}

/// A report's words, drawn as the agent's own prose is (`MarkdownText`:
/// lists, headings, code blocks, bold), folded to its first `lines` lines with
/// Show More for the rest. Blocks cannot take a line limit as one `Text`
/// can, so the fold is a height: `lines` lines of body text, measured here,
/// clipping the whole, with the last line fading so no line is cut through.
struct FoldedMarkdown: View {
    let text: String
    var lines = AgentConversation.collapsedLines
    @State private var open = false
    @State private var whole: CGFloat = 0
    @State private var line: CGFloat = 0
    @Environment(\.selectsRanges) private var selectsRanges

    // `lines` lines and one paragraph break's worth: blocks are set apart,
    // so bare `lines` lines would end inside a gap with nothing under the fade.
    private var cap: CGFloat { line * CGFloat(lines + 1) }
    private var folds: Bool { line > 0 && whole > cap + 1 }

    var body: some View {
        // In one pass (ov-675): the fold used to wait for the line's height,
        // so a report was laid out whole and then shrank, by 344 pt for a
        // report of eight paragraphs, moving every row under it.
        FoldStack(fold: .height(lines: lines), open: open) {
            // Folded, only the start that shows and enough past it to know
            // that it folds (ov-788): a lane's report drawn whole under the
            // fold was a third of the main thread while flinging past it.
            MarkdownText(text: open ? text : Markdown.foldPrefix(text, lines: lines))
                // Folded, it's drawn as `Text`, which the fading mask and the
                // clip below composite; a `UITextView` under them drew the
                // last line at full ink, or not at all (review 1). Open, its
                // words select ranges (ov-609).
                .environment(\.selectsRanges, selectsRanges && (open || !folds))
                .accessibilityIdentifier("native-folded-markdown")
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { whole = $0 }
                // As tall as the stack offers, its top showing: the fold.
                .frame(minHeight: 0, maxHeight: .infinity, alignment: .top)
                .mask {
                    if folds && !open {
                        LinearGradient(
                            stops: [
                                .init(color: .black, location: 0),
                                .init(color: .black, location: max(0, 1 - line / cap)),
                                .init(color: .clear, location: 1),
                            ],
                            startPoint: .top, endPoint: .bottom)
                    } else {
                        Rectangle()
                    }
                }
                .clipped()
            Button(open ? "Show Less" : "Show More") { withAnimation(.snappy) { open.toggle() } }
                // Its state, as a disclosure says it (ov-788).
                .accessibilityValue(open ? "Expanded" : "Collapsed")
                .font(.caption.weight(.semibold))
                .frame(minHeight: 44)
                .accessibilityIdentifier("native-show-more")
                .foldButton(shown: open || folds)
            Text(verbatim: "M").font(.body).hidden()
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { line = $0 }
        }
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
