import SwiftUI

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
            // that it folds (ov-788): an agent's report drawn whole under the
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

