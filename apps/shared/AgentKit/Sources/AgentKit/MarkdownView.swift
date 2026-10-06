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

        // `.newlines` counts the \r and the \n of a Windows line ending as
        // two separators, which read one break as a blank line between
        // paragraphs (ov-198). Android's `split("\n")` never did.
        var lines = text.replacingOccurrences(of: "\r\n", with: "\n")
            .components(separatedBy: .newlines)[...]
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

    /// One or more blocks drawn as a single view.
    ///
    /// Blocks were drawn one view each, and SwiftUI's text selection lives
    /// INSIDE a `Text` — enabling it on a container makes each of that
    /// container's `Text`s separately selectable, not the container. A reader
    /// could therefore select one paragraph and never two, which is the bug
    /// this type exists to fix.
    ///
    /// Adjacent paragraphs are therefore gathered into one run and drawn as one
    /// `Text`. Nothing else is: a list item's marker is a hanging indent, a
    /// quote has a rule beside it, a table is a grid and a fence is a box, and
    /// `Text` can express none of those. `MarkdownText.merged` explains what
    /// the paragraph gap costs.
    public enum Run: Equatable {
        /// Consecutive paragraphs, in order. Never empty.
        case prose([String])
        case block(Block)
    }

    /// Blocks, with adjacent paragraphs gathered.
    public static func runs(_ blocks: [Block]) -> [Run] {
        var runs: [Run] = []
        for block in blocks {
            guard case let .paragraph(text) = block else {
                runs.append(.block(block))
                continue
            }
            if case let .prose(paragraphs) = runs.last {
                runs[runs.count - 1] = .prose(paragraphs + [text])
            } else {
                runs.append(.prose([text]))
            }
        }
        return runs
    }

    /// `runs(blocks(text))`, computed once per distinct message.
    ///
    /// The entry point every renderer should use. `blocks` is a full line scan
    /// of the message and each prose run then goes through
    /// `AttributedString(markdown:)`; a SwiftUI `body` runs many times for one
    /// row, so doing this in the body made the cost proportional to scrolling
    /// rather than to the transcript. See `RenderMemo` for why the memo lives
    /// outside the view.
    ///
    /// The limit is a scroll's worth of rows, not a session's: enough that
    /// paging back and forth over the same screenful never re-parses, small
    /// enough that a long transcript does not pin every message it ever showed.
    @MainActor
    public static let runCache = RenderMemo<String, [Run]>(limit: 60)

    /// `runCache`'s twin for a task's own text (ov-98): a record draws every
    /// note at once, in no lazy stack, so a cache smaller than the record
    /// evicts each entry before it's drawn again and every redraw re-parses
    /// them all. Sized past the longest record, and still bounded, and apart
    /// from the chat's, so a long record doesn't flush a transcript's.
    @MainActor
    public static let documentRunCache = RenderMemo<String, [Run]>(limit: 1_000)

    /// `inline`, computed once per distinct line: acceptance lines,
    /// constraints and questions, drawn on every redraw of a task.
    @MainActor
    public static let inlineCache = RenderMemo<String, AttributedString>(limit: 1_000)

    /// Whether a link may be opened: the web, mail and a task link
    /// (`TaskKeyLinks`, ov-196), nothing else.
    ///
    /// Task text and replies are written by agents. A `file:` link to a
    /// `.command` runs it in Terminal on one click on a Mac, an app's own
    /// scheme hands off to that app, and none of it asks first (ov-98 review
    /// B1). So any other scheme isn't a link at all (`inline`), and the views
    /// refuse to open one besides (`openGuard`). A task link is the one
    /// `farcooler://` URL let through, and the guard opens it in the app
    /// itself, never through the system.
    public static func opens(_ url: URL) -> Bool {
        ["http", "https", "mailto"].contains(url.scheme?.lowercased() ?? "") || TaskKeyLinks.parse(url) != nil
    }

    /// The open-URL handler every Markdown view draws under: a task link
    /// goes to `linker`, opened in the app or not at all; the rest of
    /// `opens`'s links go to the system; everything else is dropped.
    public static func openGuard(_ linker: TaskKeyLinker) -> OpenURLAction {
        OpenURLAction { url in
            if TaskKeyLinks.parse(url) != nil {
                MainActor.assumeIsolated { _ = linker.follow(url) }
                return .handled
            }
            return opens(url) ? .systemAction : .discarded
        }
    }

    /// `openGuard(_:)` with no task links to follow.
    public static var openGuard: OpenURLAction { openGuard(.none) }

    @MainActor
    public static func cachedRuns(_ text: String, spacing: MarkdownSpacing = .reply) -> [Run] {
        let cache = spacing == .document ? documentRunCache : runCache
        return cache.value(for: text) { runs(blocks($0)) }
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
        var parsed = (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
        // A link to anywhere but the web or mail keeps its words and loses
        // its link (`opens`).
        for run in parsed.runs {
            if let url = run.link, !opens(url) { parsed[run.range].link = nil }
        }
        return parsed
    }
}

/// A rendered markdown reply.
public struct MarkdownText: View {
    public let text: String
    /// Reasoning is set smaller and dimmer than a reply, but is otherwise the
    /// same markdown — agents write lists and code in their thinking too.
    public var secondary: Bool = false
    /// How far apart its blocks sit: a reply's, or a task's text on the
    /// 8 pt rhythm (ov-98).
    public var spacing: MarkdownSpacing = .reply

    /// Still being written: the reply at the tail of a running turn. Its
    /// last paragraph is drawn apart from the rest, so a delta redraws only
    /// that paragraph (`Markdown.streamingRuns`).
    public var streaming: Bool = false

    /// The task keys its text links (ov-196), and where they go.
    @Environment(\.taskKeyLinker) private var linker

    public init(
        text: String, secondary: Bool = false, spacing: MarkdownSpacing = .reply, streaming: Bool = false
    ) {
        self.text = text
        self.secondary = secondary
        self.spacing = spacing
        self.streaming = streaming
    }

    public var body: some View {
        let pieces = Self.pieces(text, secondary: secondary, spacing: spacing, streaming: streaming)
        VStack(alignment: .leading, spacing: 0) {
            ForEach(pieces.indices, id: \.self) { index in
                pieces[index].equatable()
            }
        }
        .font(secondary ? .caption : .body)
        .foregroundStyle(secondary ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
        // A second line of defense behind `Markdown.inline`'s filter, and
        // the way a task key's link reaches its task.
        .environment(\.openURL, Markdown.openGuard(linker))
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// What `body` draws, a piece per run.
    ///
    /// Settled, it's the runs as `Markdown.cachedRuns` gathers them, every
    /// one selectable, a selection free to cross a prose run's paragraphs.
    /// Streaming, the last prose run is split by paragraph and the last
    /// piece is open (`MarkdownPiece.open`), so a delta changes one piece.
    @MainActor
    static func pieces(
        _ text: String, secondary: Bool = false, spacing: MarkdownSpacing = .reply, streaming: Bool = false
    ) -> [MarkdownPiece] {
        // A streaming reply's text is new on every delta, so it skips the
        // memo: each prefix would take a slot, and a turn's worth of them
        // evicts every settled row a reader is looking at.
        let runs =
            streaming
            ? Markdown.streamingRuns(Markdown.runs(Markdown.blocks(text)))
            : Markdown.cachedRuns(text, spacing: spacing)
        return runs.indices.map { index in
            MarkdownPiece(
                run: runs[index], secondary: secondary, open: streaming && index == runs.count - 1,
                gap: index == 0
                    ? 0
                    : MarkdownBlockSpacing.gap(
                        after: role(for: runs[index - 1]), before: role(for: runs[index]), style: spacing))
        }
    }

    /// Consecutive paragraphs as one attributed string, so one selection can
    /// cross them.
    ///
    /// The blank line between two paragraphs is the whole subtlety. SwiftUI's
    /// `Text` reads only a handful of `AttributedString` attributes — font,
    /// color, kern, baseline offset — and `NSParagraphStyle` is not among them:
    /// setting `paragraphSpacing` to 40 measured the same height as setting
    /// nothing (`ImageRenderer`, 360pt wide: 71pt either way). Inside one
    /// `Text` the only thing that makes vertical space is a line, and the only
    /// thing that sets a line's height is the font of the characters on it.
    ///
    /// So the separator is a blank line at a deliberately small fixed size,
    /// chosen to measure the same 16 points the `VStack` used to pad. Fixed
    /// rather than relative because the gap it replaces was a fixed 16 too —
    /// which is also why one number serves both the body and the caption
    /// rendering.
    ///
    /// What it costs, measured rather than assumed: over 40 combinations of
    /// width (240–680), paragraph count and text style, 27 rendered
    /// byte-identical to the old `VStack` and the remaining 13 differed by half
    /// a point per paragraph boundary — the blank line's height quantizes, and
    /// 16.0 is not exactly on the grid. `paragraphsMatchTheirOldSpacing` is the
    /// test that holds it there.
    static func merged(_ paragraphs: [String]) -> AttributedString {
        var separator = AttributedString("\n\n")
        separator.font = .system(size: MarkdownBlockSpacing.paragraphGapFont)
        var out = AttributedString()
        for (index, paragraph) in paragraphs.enumerated() {
            if index > 0 { out += separator }
            out += Markdown.inline(paragraph)
        }
        return out
    }

    /// A prose run is paragraphs and nothing else, so it relates to its
    /// neighbors exactly as the single paragraph at its edge used to.
    private static func role(for run: Markdown.Run) -> MarkdownBlockRole {
        switch run {
        case .prose: .paragraph
        case let .block(block): role(for: block)
        }
    }

    private static func role(for block: Markdown.Block) -> MarkdownBlockRole {
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
    public var streaming: Bool = false

    public init(text: String, trailingClearance: CGFloat, streaming: Bool = false) {
        self.text = text
        self.trailingClearance = trailingClearance
        self.streaming = streaming
    }

    public var body: some View {
        MarkdownText(text: text, streaming: streaming)
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

/// How far apart a piece of Markdown's blocks sit.
public enum MarkdownSpacing: Sendable {
    /// An agent's reply in a transcript: `MarkdownBlockSpacing.gap`'s own
    /// steps.
    case reply
    /// A task's own text, its intent and notes (ov-98): the same steps
    /// snapped to the 8 pt rhythm the rest of the task view keeps.
    case document
}

enum MarkdownBlockSpacing {
    /// `gap(after:before:)`, for `style`.
    static func gap(
        after previous: MarkdownBlockRole, before current: MarkdownBlockRole, style: MarkdownSpacing
    ) -> CGFloat {
        let step = gap(after: previous, before: current)
        switch style {
        case .reply: return step
        case .document:
            // To the nearest whole step, never none: 10 and 12 become 8 and
            // 16, 18 and 20 become 16 and 24.
            return max(rhythm, (step / rhythm).rounded() * rhythm)
        }
    }

    /// The task view's vertical step (`ColumnGrid.rhythm` on the Mac).
    static let rhythm: CGFloat = 8


    /// The point size of the blank line that separates two paragraphs drawn
    /// inside one `Text`.
    ///
    /// Not a typographic choice — a measurement. Two paragraphs in one `Text`
    /// can only be pushed apart by a line, and a line is as tall as the font of
    /// the characters on it, so this is the size that makes that line measure
    /// the same 16 points `gap(after: .paragraph, before: .paragraph)` used to
    /// pad. Sizes from 8.8 to 9.4 all land on the same rendered result — the
    /// height quantizes — and 9 is the middle of that plateau, which is the
    /// most room either side before a metric change moves the answer.
    ///
    /// Deliberately absolute rather than `@ScaledMetric`: the gap it replaces
    /// was an absolute 16 at every text size, so scaling this one would be the
    /// change, not the fidelity.
    static let paragraphGapFont: CGFloat = 9

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
struct MarkdownTable: View {
    let header: [String]
    let rows: [[String]]
    @Environment(\.taskKeyLinker) private var linker

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
            RoundedRectangle.control.strokeBorder(.quaternary)  // style-exempt: the table's one outer border, so the cells' shared rules close up
        }
        .clipShape(.control)
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
        return TaskKeyText(linker.linked(Markdown.inline(text)))
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
                // neighbors share one line instead of drawing two.
                if column > 0 {
                    Color.clear.separator(.grid, edge: .leading)
                }
            }
            .overlay(alignment: .bottom) {
                Color.clear.separator(.grid, edge: .bottom)
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
            if chrome { RoundedRectangle.control.fill(.quinary) }
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


// MARK: - A task's text

/// A task's own text, its intent, acceptance lines and notes, read as the
/// Markdown it's written in (ov-98), so the Mac and the phone draw the same
/// pieces. The parsing is `Markdown`'s, the chat's own.
public enum TaskProse {
    /// Intent or a note's body, split into blocks: what `MarkdownText`
    /// draws.
    public static func blocks(_ text: String) -> [Markdown.Block] { Markdown.blocks(text) }

    /// One line's inline Markdown: bold, italic, `code` and links. HTML is
    /// left as the characters it was typed as; nothing here renders it.
    @MainActor
    public static func inline(_ text: String) -> AttributedString {
        Markdown.inlineCache.value(for: text) { Markdown.inline($0) }
    }

    /// An acceptance line as its checklist row draws it: its inline
    /// Markdown, and struck through, all of it, once it's met. The color is
    /// the row's to choose (secondary when met), so it stays monochrome.
    @MainActor
    public static func acceptance(_ text: String, met: Bool) -> AttributedString {
        var line = inline(text)
        if met {
            line[AttributeScopes.SwiftUIAttributes.StrikethroughStyleAttribute.self] = .single
        }
        return line
    }

    /// What a line of task text says, without its markup: what VoiceOver
    /// reads for a row drawn from it, never "star star".
    @MainActor
    public static func plain(_ text: String) -> String { String(inline(text).characters) }

    /// The quiet line over a note in the record: "Finding · manager · 4m
    /// ago". No byline, no middle part.
    public static func noteLine(kind: String, byline: String, ago: String) -> String {
        [kind, byline, ago].filter { !$0.isEmpty }.joined(separator: " · ")
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
