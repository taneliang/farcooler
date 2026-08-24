import SwiftUI

// Rendering, shared by both apps.
//
// `Transcript.swift` explains why the reducer is shared: a phone and a Mac that
// disagreed about one session is the failure this whole design exists to
// prevent. The same argument applies to how that session is DRAWN. The iOS app
// rendered agent replies as plain `Text`, so a table arrived as a wall of pipes
// and a heading as a line beginning with a hash — the same conversation,
// unreadable on one of the two clients.


/// Markdown, rendered as blocks rather than as one run of text.
///
/// `AttributedString(markdown:)` handles inline syntax — bold, italic, code
/// spans, links — and nothing else. Headings arrive as plain text, list items
/// lose their bullets, and every block is concatenated into a single
/// paragraph, which is why a reply full of structure rendered as one long
/// line with some words emphasised.
///
/// So blocks are split here and each is drawn as itself; inline parsing is
/// still `AttributedString`'s job, which it does well. This is not a complete
/// CommonMark implementation and is not trying to be — it covers what a chat
/// reply actually contains.
public enum Markdown {
    /// One renderable piece of a reply.
    public enum Block: Equatable {
        case paragraph(String)
        case heading(level: Int, text: String)
        case bullet(text: String, depth: Int)
        case numbered(number: String, text: String, depth: Int)
        case code(text: String, language: String)
        case quote(String)
        case rule
        /// A pipe table. The header row is kept apart from the body because it
        /// is drawn differently, not because the parser needs it separated.
        case table(header: [String], rows: [[String]])
    }

    /// Split one `| a | b |` line into its cells.
    ///
    /// The outer pipes are optional in GitHub's dialect and Claude writes them
    /// either way, so an empty cell from a leading or trailing pipe is dropped
    /// rather than rendered as a blank column.
    public static func cells(_ line: String) -> [String] {
        var trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("|") { trimmed.removeFirst() }
        if trimmed.hasSuffix("|") { trimmed.removeLast() }
        return trimmed.components(separatedBy: "|").map {
            $0.trimmingCharacters(in: .whitespaces)
        }
    }

    /// Whether a line is a table's `|---|:--:|` separator.
    ///
    /// This is what distinguishes a table from a paragraph that happens to
    /// contain a pipe — a shell command, most often, which must not be eaten as
    /// markup.
    public static func isTableRule(_ line: String) -> Bool {
        let parts = cells(line)
        guard !parts.isEmpty, line.contains("|") else { return false }
        return parts.allSatisfy { cell in
            !cell.isEmpty && cell.allSatisfy { $0 == "-" || $0 == ":" || $0 == " " }
                && cell.contains("-")
        }
    }

    /// Split markdown into blocks.
    ///
    /// Line-based, because the structures that matter here are line-based. A
    /// fenced code block swallows everything until its closing fence, so its
    /// contents are never mistaken for markup.
    public static func blocks(_ text: String) -> [Block] {
        var blocks: [Block] = []
        var paragraph: [String] = []

        func flushParagraph() {
            // Joined with a NEWLINE, not a space.
            //
            // CommonMark folds a single line break into a space and needs two
            // to make one. That rule exists for hand-written source files, and
            // Claude does not write to it — it breaks lines where it means
            // them to break. Honouring that is the difference between a
            // readable reply and one run-on paragraph.
            let joined = paragraph.joined(separator: "\n").trimmingCharacters(in: .whitespaces)
            if !joined.isEmpty { blocks.append(.paragraph(joined)) }
            paragraph = []
        }

        var lines = text.components(separatedBy: .newlines)[...]
        while let line = lines.first {
            lines = lines.dropFirst()
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            // A fence takes precedence over everything: its contents are not
            // markup and must not be read as any.
            if trimmed.hasPrefix("```") {
                flushParagraph()
                let language = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                var body: [String] = []
                while let next = lines.first {
                    lines = lines.dropFirst()
                    if next.trimmingCharacters(in: .whitespaces).hasPrefix("```") { break }
                    body.append(next)
                }
                blocks.append(.code(text: body.joined(separator: "\n"), language: language))
                continue
            }

            if trimmed.isEmpty {
                flushParagraph()
                continue
            }

            if trimmed == "---" || trimmed == "***" || trimmed == "___" {
                flushParagraph()
                blocks.append(.rule)
                continue
            }

            if let hashes = trimmed.firstIndex(where: { $0 != "#" }),
                trimmed.starts(with: "#"),
                trimmed[hashes] == " "
            {
                flushParagraph()
                let level = trimmed.distance(from: trimmed.startIndex, to: hashes)
                let body = String(trimmed[hashes...]).trimmingCharacters(in: .whitespaces)
                blocks.append(.heading(level: min(level, 3), text: body))
                continue
            }

            // A table is recognized by its SECOND line, not its first: the
            // header alone is indistinguishable from a sentence containing a
            // pipe. So the separator is peeked at before either is consumed.
            if trimmed.contains("|"), let next = lines.first, isTableRule(next) {
                flushParagraph()
                lines = lines.dropFirst()
                let header = cells(trimmed)
                var body: [[String]] = []
                while let row = lines.first,
                    row.trimmingCharacters(in: .whitespaces).contains("|"),
                    !row.trimmingCharacters(in: .whitespaces).isEmpty
                {
                    lines = lines.dropFirst()
                    body.append(cells(row))
                }
                blocks.append(.table(header: header, rows: body))
                continue
            }

            if trimmed.hasPrefix("> ") {
                flushParagraph()
                blocks.append(.quote(String(trimmed.dropFirst(2))))
                continue
            }

            // Indentation is what nests a list, so it is measured before the
            // marker is stripped.
            let indent = line.prefix(while: { $0 == " " || $0 == "\t" }).count
            let depth = min(indent / 2, 3)

            if let marker = ["- ", "* ", "+ "].first(where: { trimmed.hasPrefix($0) }) {
                flushParagraph()
                blocks.append(.bullet(text: String(trimmed.dropFirst(marker.count)), depth: depth))
                continue
            }

            if let dot = trimmed.firstIndex(of: "."),
                trimmed[trimmed.startIndex..<dot].allSatisfy(\.isNumber),
                trimmed.index(after: dot) < trimmed.endIndex,
                trimmed[trimmed.index(after: dot)] == " "
            {
                flushParagraph()
                let number = String(trimmed[trimmed.startIndex..<dot])
                let body = String(trimmed[trimmed.index(dot, offsetBy: 2)...])
                blocks.append(.numbered(number: number, text: body, depth: depth))
                continue
            }

            paragraph.append(trimmed)
        }
        flushParagraph()
        return blocks
    }

    /// Inline syntax only — bold, italic, code spans, links.
    ///
    /// Text that cannot be parsed is returned as itself rather than dropped: a
    /// stray bracket should cost a reader the emphasis, not the sentence.
    public static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: true,
            interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

/// A rendered markdown reply.
public struct MarkdownText: View {
    public let text: String
    /// Reasoning is set smaller and dimmer than a reply, but is otherwise the
    /// same markdown — agents write lists and code in their thinking too.
    public var secondary: Bool = false

    @ScaledMetric(relativeTo: .body)
    private var h1Size = MarkdownTypeScale.h1
    @ScaledMetric(relativeTo: .body)
    private var h2Size = MarkdownTypeScale.h2
    @ScaledMetric(relativeTo: .body)
    private var h3Size = MarkdownTypeScale.h3

    public init(text: String, secondary: Bool = false) {
        self.text = text
        self.secondary = secondary
    }

    public var body: some View {
        let blocks = Markdown.blocks(text)
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                view(for: block)
                    .padding(
                        .top,
                        index == 0
                            ? 0
                            : MarkdownBlockSpacing.gap(
                                after: role(for: blocks[index - 1]),
                                before: role(for: block)))
            }
        }
        .font(secondary ? .caption : .body)
        .foregroundStyle(secondary ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func view(for block: Markdown.Block) -> some View {
        switch block {
        case let .paragraph(text):
            Text(Markdown.inline(text))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(3)

        case let .heading(level, text):
            Text(Markdown.inline(text))
                .font(headingFont(level))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(2)

        case let .bullet(text, depth):
            marker("•", text: text, depth: depth)

        case let .numbered(number, text, depth):
            marker("\(number).", text: text, depth: depth)

        case let .code(text, _):
            // The same box a tool's output gets: one way of showing
            // monospaced text, not two.
            DetailBox(text: text)

        case let .quote(text):
            HStack(spacing: 8) {
                Rectangle().fill(.quaternary).frame(width: 2)
                Text(Markdown.inline(text))
                    .fixedSize(horizontal: false, vertical: true)
                    .lineSpacing(3)
            }

        case .rule:
            Divider()

        case let .table(header, rows):
            MarkdownTable(header: header, rows: rows)
        }
    }

    /// A marker and its text, aligned so a wrapped line does not slide back
    /// under the bullet.
    private func marker(_ symbol: String, text: String, depth: Int) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(symbol)
                .foregroundStyle(.secondary)
                .frame(minWidth: 14, alignment: .trailing)
            Text(Markdown.inline(text))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(3)
            Spacer(minLength: 0)
        }
        .padding(.leading, CGFloat(depth) * 14)
    }

    private func headingFont(_ level: Int) -> Font {
        // A heading must be distinct from both body and inline bold. Semantic
        // `.headline` is the same point size as body on Apple platforms, so a
        // weight-only H3 looked exactly like a bold phrase. This small custom
        // ramp keeps all three levels legible without turning chat into a title
        // page; `@ScaledMetric` preserves accessibility scaling.
        if secondary {
            switch level {
            case 1, 2: return .caption.weight(.semibold)
            default: return .caption.weight(.medium)
            }
        }

        switch level {
        case 1: return .system(size: h1Size, weight: .semibold)
        case 2: return .system(size: h2Size, weight: .semibold)
        default: return .system(size: h3Size, weight: .semibold)
        }
    }

    private func role(for block: Markdown.Block) -> MarkdownBlockRole {
        switch block {
        case .paragraph: .paragraph
        case let .heading(level, _): .heading(level: level)
        case .bullet, .numbered: .listItem
        case .code: .code
        case .quote: .quote
        case .rule: .rule
        case .table: .table
        }
    }
}

enum MarkdownTypeScale {
    #if os(macOS)
    static let h1: CGFloat = 20
    static let h2: CGFloat = 17
    static let h3: CGFloat = 15
    #else
    static let h1: CGFloat = 24
    static let h2: CGFloat = 21
    static let h3: CGFloat = 19
    #endif
}

/// A full-width agent reply inside a transcript row.
///
/// This is deliberately not an `HStack { MarkdownText; Spacer }`. A flexible
/// text view inside that stack is first measured with an unspecified width, so
/// SwiftUI reports its one-line height and only wraps it later when the final
/// width is assigned. The wrapped glyphs draw outside that reported height and
/// the transcript places the next row on top of them. Directly proposing the
/// row's width lets `Text` report every rendered line on both UIKit and AppKit.
public struct AgentReplyText: View {
    public let text: String
    public let trailingClearance: CGFloat

    public init(text: String, trailingClearance: CGFloat) {
        self.text = text
        self.trailingClearance = trailingClearance
    }

    public var body: some View {
        MarkdownText(text: text)
            // Keep long desktop panes in a comfortable reading measure. The
            // phone is narrower than this and therefore remains full width.
            .frame(maxWidth: 680, alignment: .leading)
            .padding(.trailing, trailingClearance)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

enum MarkdownBlockRole: Equatable {
    case paragraph
    case heading(level: Int)
    case listItem
    case code
    case quote
    case rule
    case table
}

enum MarkdownBlockSpacing {
    /// Space expresses the relationship between blocks, not a global constant.
    /// List items belong together; a heading hugs the content it introduces;
    /// a new section needs enough air to be found while scanning.
    static func gap(after previous: MarkdownBlockRole, before current: MarkdownBlockRole) -> CGFloat {
        if previous == .rule { return 16 }

        if case .heading = previous {
            if case .heading = current { return 12 }
            return 8
        }

        if case let .heading(level) = current {
            switch level {
            case 1: return 24
            case 2: return 20
            default: return 16
            }
        }

        if current == .rule { return 18 }
        if previous == .listItem, current == .listItem { return 8 }

        switch (previous, current) {
        case (.paragraph, .paragraph),
             (.listItem, .paragraph),
             (.table, .paragraph):
            return 16
        case (.paragraph, .listItem),
             (.quote, .paragraph),
             (.code, .paragraph):
            return 12
        default:
            return 10
        }
    }
}

/// A pipe table, drawn as native vertical and horizontal flow.
///
/// Equal flexible cells keep the rules aligned while each `HStack` takes the
/// height of its tallest wrapping cell. Most importantly, the `VStack` owns
/// row placement. There is no separately calculated Y coordinate that can
/// become stale when UIKit and AppKit produce different text metrics.
private struct MarkdownTable: View {
    let header: [String]
    let rows: [[String]]

    var body: some View {
        // The first version scrolled sideways with single-line cells, which is
        // fine for a two-column list of paths and useless for the tables an
        // agent actually writes — a cell of prose became "Adds roughly 4–8 hou…"
        // and the sentence was simply gone. Cells wrap and the table fits the
        // pane instead. Each native row expands around its wrapped content,
        // so the next row cannot begin until that content has taken its space.
        VStack(alignment: .leading, spacing: 0) {
            ForEach(allRows.indices, id: \.self) { row in
                HStack(alignment: .top, spacing: 0) {
                    ForEach(0..<columnCount, id: \.self) { column in
                        cellView(row: row, column: column)
                    }
                }
            }
        }
        .overlay {
            // One border around the outside; each cell draws its own leading
            // and bottom edge, so every interior rule is shared rather than
            // doubled.
            RoundedRectangle(cornerRadius: 5).strokeBorder(.quaternary)
        }
        .clipShape(RoundedRectangle(cornerRadius: 5))
        .padding(.vertical, 4)
    }

    /// The header is just the first row, so one loop draws the whole table.
    private var allRows: [[String]] { [header] + rows }

    /// One cell, as its own function, shared by header and body rows.
    private func cellView(row: Int, column: Int) -> some View {
        let text: String = {
            let cells = allRows[row]
            return column < cells.count ? cells[column] : ""
        }()
        return Text(Markdown.inline(text))
            .font(.callout)
            .lineSpacing(1)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .modifier(TableCell(column: column, isHeader: row == 0))
    }

    /// The widest row wins, so a row the agent wrote short leaves a blank cell
    /// rather than shifting every column after it.
    private var columnCount: Int {
        max(header.count, rows.map(\.count).max() ?? 0)
    }

}

/// One cell's padding and its share of the grid's rules.
private struct TableCell: ViewModifier {
    let column: Int
    let isHeader: Bool

    func body(content: Content) -> some View {
        content
            .fontWeight(isHeader ? .semibold : .regular)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .overlay(alignment: .leading) {
                // Every column but the first draws the rule to its left, so
                // neighbours share one line instead of drawing two.
                if column > 0 {
                    Rectangle().fill(.quaternary).frame(width: 1)
                }
            }
            .overlay(alignment: .bottom) {
                Rectangle().fill(.quaternary).frame(height: 1)
            }
            .background(isHeader ? AnyShapeStyle(.quinary) : AnyShapeStyle(.clear))
    }
}

/// A bounded, scrollable block of output.
///
/// Bounded because a tool can return a thousand lines and a transcript is not
/// a place to page through them; scrollable because truncating to a preview
/// throws away the half you needed. The same box serves reasoning, console
/// output and file contents, so they are not three inventions.
public struct DetailBox: View {
    public let text: String
    public var monospaced: Bool = true
    /// Its own fill and padding. Off when it is already inside a container that
    /// has both — a fill drawn on top of the same fill just muddies the edge
    /// that was doing the work.
    public var chrome: Bool = true

    public init(text: String, monospaced: Bool = true, chrome: Bool = true) {
        self.text = text
        self.monospaced = monospaced
        self.chrome = chrome
    }

    /// Beyond this, the tail only.
    ///
    /// Not a display preference — a hang. `Text` measures its WHOLE string on
    /// every layout pass, and a tool that returns a few thousand lines is
    /// inside an animated disclosure that re-measures it many times per frame.
    /// Expanding one wedged the app on the main thread. The box tops out at
    /// `ceiling` and scrolls, so nothing beyond this was ever on screen anyway.
    private static let maxLines = 400

    /// The tallest this box gets. A CEILING, not a reserve — see `body`.
    private static let ceiling: CGFloat = 220

    public var body: some View {
        let shown = Self.clamp(text)
        ScrollView {
            VStack(alignment: .leading, spacing: 4) {
                if shown.truncated {
                    // Said, not silently done. Output that stops early without
                    // saying so is output you can draw the wrong conclusion
                    // from.
                    Text("Showing the last \(Self.maxLines) lines.")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
                Text(shown.text)
                    .font(monospaced ? .caption.monospaced() : .caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(chrome ? 8 : 0)
        }
        .frame(maxHeight: Self.ceiling)
        // What makes the 220 a ceiling rather than a floor.
        //
        // A flexible frame with a `maxHeight` GROWS to whatever it is offered
        // and stops at the max — so in any container that proposes a definite
        // height, this box took 220 no matter how little was in it, and a
        // two-line ssh error got a 42-point block of words with 178 points of
        // nothing under it.
        // The `ScrollView` was not what did it; a bare `Text` under the same
        // frame does the same thing. Measured, with `ImageRenderer` driving a
        // 320-wide box at three proposals:
        //
        //     content   proposal    before    after
        //     2 lines   nil           42        42
        //     2 lines   600          220        42
        //     200 lines nil          220       220
        //     200 lines 600          220       220
        //
        // `fixedSize` in the vertical only. It hands the frame below it a
        // `nil` proposal whatever it was itself offered, which makes every
        // container behave the way the ones already proposing `nil` did — and
        // that is most of them, since every transcript row is inside a scroll
        // view. Which is why this only ever showed on the centered screens.
        // The frame stays underneath and is still what resolves that `nil`, so
        // tall output is still capped at 220 and still scrolls inside it: an
        // XCUITest on 60 lines of output measures the box at 219.67 and the
        // first line moving from y=409 to y=-269 on one swipe.
        //
        // The one thing given up: a container offering LESS than 220 no longer
        // squeezes the box into it, since ignoring what it was offered is what
        // `fixedSize` is. Tall output in a short container overflows rather
        // than shrinking. Nothing calls it that way today — the height caps
        // near these, `ChangesPane`'s 320 and `AddDeviceView`'s 460, are on
        // scroll views ABOVE the box and propose `nil` into it, and the frames
        // touching the box itself are all `maxWidth`. The alternative —
        // measuring the content and clamping the frame to it — would keep that
        // last case, and costs a first layout pass at the wrong height, which
        // inside `ToolRow`'s spring disclosure is a visible wobble on every
        // expand.
        .fixedSize(horizontal: false, vertical: true)
        .background {
            if chrome { RoundedRectangle(cornerRadius: 7).fill(.quinary) }
        }
    }

    /// The tail, because that is where a command says how it went.
    static func clamp(_ text: String, limit: Int = DetailBox.maxLines)
        -> (text: String, truncated: Bool)
    {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        guard lines.count > limit else { return (text, false) }
        return (lines.suffix(limit).joined(separator: "\n"), true)
    }
}


// MARK: - Plan status

/// What a plan entry's status means, in one place.
///
/// ACP sends the status as a free string and adapters spell it differently
/// (`in_progress`, `inProgress`, `active`), so every renderer has to interpret
/// it. Both apps were doing that interpretation themselves, with four identical
/// private helpers each — the exact duplication `Transcript` is shared to
/// avoid, since a phone and a Mac disagreeing about which task is running is a
/// disagreement about the same session.
public enum PlanStatus: Sendable {
    case pending
    case active
    case done

    public init(_ status: String) {
        let lowered = status.lowercased()
        if lowered.contains("done") || lowered.contains("complet") {
            self = .done
        } else if lowered.contains("progress") || lowered.contains("active") {
            self = .active
        } else {
            self = .pending
        }
    }

    /// The SF Symbol for this state.
    public var symbol: String {
        switch self {
        case .done: "checkmark.circle.fill"
        case .active: "circle.lefthalf.filled"
        case .pending: "circle"
        }
    }

    public var isDone: Bool { self == .done }

    /// Green finished, accent running, quiet otherwise — the same three colors
    /// on both clients.
    public var tint: Color {
        switch self {
        case .done: .green
        case .active: .accentColor
        case .pending: .secondary
        }
    }
}

extension Collection where Element == PlanEntry {
    /// How many are finished, for the "3 of 7" a reader actually wants.
    public var doneCount: Int { filter { PlanStatus($0.status).isDone }.count }

    /// The one being worked on now, which is what a collapsed list must still
    /// be able to say.
    public var active: PlanEntry? { first { PlanStatus($0.status) == .active } }
}
