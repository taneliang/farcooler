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
                .foregroundStyle(.quietInk)
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
    @Environment(\.selectsText) private var selectsText

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
                .foregroundStyle(.quietInk)
            Text(text)
                .font(.caption.monospaced())
                .selectable(selectsText)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(Spacing.group)
                .surface(.inset, in: .control)
        }
    }
}

/// An edit's hunks, as unified-diff lines.
struct NativeDiff: View {
    let hunks: [AgentRow.Hunk]
    @Environment(\.selectsText) private var selectsText

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(hunks.indices, id: \.self) { h in
                ForEach(hunks[h].lines.indices, id: \.self) { l in
                    let line = hunks[h].lines[l]
                    Text(line.isEmpty ? " " : line)
                        .font(.caption.monospaced())
                        .foregroundStyle(line.hasPrefix("-") ? AnyShapeStyle(Ink.quiet) : AnyShapeStyle(.primary))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .selectable(selectsText)
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
                .foregroundStyle(.quietInk)
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
        .modifier(SelectsAtRest())
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
            .foregroundStyle(.quietInk)
            ForEach(tasks.items.indices, id: \.self) { i in
                let item = tasks.items[i]
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    TaskMark(status: item.status)
                    Text(item.subject)
                        .strikethrough(item.status == "Completed")
                        .foregroundStyle(item.status == "InProgress" ? AnyShapeStyle(.primary) : AnyShapeStyle(Ink.quiet))
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
            case "Completed": Image(systemName: "checkmark.circle.fill").foregroundStyle(.quietInk)
            case "InProgress": Image(systemName: "circle.lefthalf.filled").foregroundStyle(.tint)
            default: Image(systemName: "circle").foregroundStyle(.quietInk)
            }
        }
        .frame(width: 14)
        .accessibilityHidden(true)
    }
}

/// A message that may be long: its first lines, and Show More for the rest.
/// Whether it runs past them is measured at the width it's drawn at: the
/// text as shown against the same text unlimited, so a long line on a narrow
/// pane folds as surely as many short ones (ov-452 review).
struct FoldedText: View {
    let text: String
    var lines = AgentConversation.collapsedLines
    /// On the person's blue bubble (ov-610): Show More in the bubble's
    /// white, underlined, where the link color wouldn't read.
    var onBubble = false
    @State private var open = false
    @State private var whole: CGFloat = 0
    @State private var shown: CGFloat = 0
    @Environment(\.selectsText) private var selectsText

    var body: some View {
        // Laid out in one pass, Show More and all (ov-691, as the iPhone's
        // since ov-675): a lazy stack builds a row just above the screen as
        // the reader scrolls up, and a row that changed height on its second
        // pass moved every row on the screen.
        FoldStack(fold: .lines, open: open) {
            Text(verbatim: text)
                .selectable(selectsText)
                .lineLimit(open ? nil : lines)
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shown = $0 }
            button.foldButton(shown: open || whole > shown + 1, identified: "native-show-more")
            Text(verbatim: text)
                .fixedSize(horizontal: false, vertical: true)
                .hidden()
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { whole = $0 }
        }
    }

    @ViewBuilder private var button: some View {
        if onBubble {
            Button(open ? "Show Less" : "Show More") { open.toggle() }
                .buttonStyle(.plain)
                .underline()
                .font(.caption.weight(.semibold))
        } else {
            Button(open ? "Show Less" : "Show More") { open.toggle() }
                .buttonStyle(.link)
                .font(.caption)
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

    // `lines` lines and one paragraph break's worth: blocks are set apart,
    // so bare `lines` lines would end inside a gap with nothing under the fade.
    private var cap: CGFloat { line * CGFloat(lines + 1) }
    private var folds: Bool { line > 0 && whole > cap + 1 }

    var body: some View {
        // In one pass (ov-691): the fold used to wait a frame for the line's
        // height, so a report was laid out whole and then shrank, by 145 pt
        // for a report of eight paragraphs, and every row on the screen
        // jumped up as the reader scrolled past one.
        FoldStack(fold: .height(lines: lines), open: open) {
            MarkdownText(text: text)
                .identified("native-folded-markdown")
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
            Button(open ? "Show Less" : "Show More") { open.toggle() }
                .buttonStyle(.link)
                .font(.caption)
                .foldButton(shown: open || folds, identified: "native-show-more")
            Text(verbatim: "M").font(.body).hidden()
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { line = $0 }
        }
    }
}

extension View {
    /// AgentKit's `foldButton`, with the button identified only where it's
    /// shown: the hidden copy that keeps its room is never reported to a
    /// test, which looks for Show More to say a message folded.
    func foldButton(shown: Bool, identified id: String) -> some View {
        ZStack(alignment: .leading) {
            hidden()
            if shown { identified(id) }
        }
    }
}
