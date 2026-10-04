import AgentKit
import AppKit
import SwiftUI

/// A file's text, read-only (ov-189): monospaced, numbered, every line drawn
/// by `CodeLineRow`, the row the diff draws with.
///
/// Built for a file of any length the runner sends (512 KiB): a lazy stack
/// whose width is stated rather than measured (`ChangesPane.diffBody`'s rule),
/// so only the lines on screen are ever built and a 15,000-line file scrolls
/// as a 15-line one does.
///
/// Selection is by line, the way a reader quotes code: click a line number,
/// Shift-click another to take the lines between, then Copy. Text inside one
/// line can be selected as well. Find matches are marked in the line, the
/// current one more strongly.
struct CodeView: View {
    let lines: [String]
    /// The longest line's length in characters (`FilesLogic.widest`), worked
    /// out once when the file arrives rather than on every redraw.
    let widest: Int
    let font: NSFont
    let selection: ClosedRange<Int>?
    let matches: [FilesLogic.Match]
    let currentMatch: Int?
    let scroll: FilesModel.ScrollRequest?
    var onClickLine: (Int, Bool) -> Void = { _, _ in }

    /// The matches on each line, for the rows on screen to look up.
    private var matchesByLine: [Int: [(Int, FilesLogic.Match)]] {
        Dictionary(grouping: matches.enumerated().map { ($0.offset, $0.element) }, by: { $0.1.line })
    }

    var body: some View {
        let byLine = matchesByLine
        let swiftFont = Font(font as CTFont)
        let advance = font.maximumAdvancement.width
        let gutter = Self.gutter(lines: lines.count, advance: advance)
        let width = gutter + 12 + CGFloat(widest + 2) * advance
        ScrollViewReader { proxy in
            GeometryReader { geo in
                ScrollView([.vertical, .horizontal]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(lines.indices, id: \.self) { index in
                            row(index, font: swiftFont, gutter: gutter, matches: byLine[index] ?? [])
                                .id(index)
                        }
                    }
                    .textSelection(.enabled)
                    .padding(.vertical, 4)
                    .frame(width: max(geo.size.width, width), alignment: .leading)
                    .frame(minHeight: geo.size.height, alignment: .topLeading)
                }
                .defaultScrollAnchor(.topLeading)
            }
            .onChange(of: scroll) { _, request in
                guard let request else { return }
                proxy.scrollTo(request.line, anchor: .center)
            }
            .onAppear {
                if let scroll { proxy.scrollTo(scroll.line, anchor: .center) }
            }
        }
        .background(WorkspaceStyle.document)
    }

    private func row(
        _ index: Int, font: Font, gutter: CGFloat, matches: [(Int, FilesLogic.Match)]
    ) -> some View {
        let selected = selection?.contains(index) ?? false
        return CodeLineRow(
            numbers: [index + 1], text: text(index, matches: matches),
            gutter: gutter, font: font,
            wash: selected ? Fill.selection(active: true) : .clear)
            .overlay(alignment: .leading) {
                // The gutter is the line's handle: a click takes the line,
                // Shift-click grows the selection to it.
                Color.clear
                    .frame(width: gutter)
                    .contentShape(Rectangle())
                    .onTapGesture {
                        onClickLine(index, NSEvent.modifierFlags.contains(.shift))
                    }
                    .accessibilityLabel("Line \(index + 1)")
                    .accessibilityAddTraits(.isButton)
            }
    }

    /// The line's text, its find matches marked.
    private func text(_ index: Int, matches: [(Int, FilesLogic.Match)]) -> Text {
        let line = lines[index]
        guard !line.isEmpty else { return Text(" ") }
        guard !matches.isEmpty else { return Text(line) }
        var styled = AttributedString(line)
        for (number, match) in matches {
            guard
                let lower = AttributedString.Index(match.range.lowerBound, within: styled),
                let upper = AttributedString.Index(match.range.upperBound, within: styled)
            else { continue }
            styled[lower..<upper].backgroundColor =
                number == currentMatch ? Tint.findCurrent : Tint.findMatch
        }
        return Text(styled)
    }

    /// One gutter column, wide enough for the last line's number.
    static func gutter(lines: Int, advance: CGFloat) -> CGFloat {
        max(26, CGFloat(String(max(lines, 1)).count) * advance + 12)
    }
}

extension Tint {
    /// A find match in a file, and the one the find is on. The system's own
    /// find colors: the yellow every Mac text view marks a match with.
    static var findMatch: Color { Color(nsColor: .findHighlightColor).opacity(0.35) }
    static var findCurrent: Color { Color(nsColor: .findHighlightColor) }
}
