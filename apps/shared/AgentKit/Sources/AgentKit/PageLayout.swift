import SwiftUI

// The rules a page's layout follows, apart from the views so a test can hold
// them (ov-269 design 3.6, 3.7): when a table stacks, what a row says to
// VoiceOver, and which Markdown a text block draws.

/// Where a page degrades: the app owns layout, so it always can.
public enum PageLayout {
    /// Narrower than this, a wide table stacks and steps go down the page.
    /// About a phone in portrait, or the task column of a narrow Mac window.
    public static let narrow: CGFloat = 480

    /// Whether a table with `columns` draws as stacked rows at `width`: more
    /// than three columns on a narrow surface. A width not measured yet is
    /// wide.
    public static func stacks(columns: Int, width: CGFloat?) -> Bool {
        guard let width, width > 0 else { return false }
        return columns > 3 && width < narrow
    }

    /// Whether a row's state and live status go under its words rather than
    /// beside them: at accessibility text sizes, where beside them they'd be
    /// cut short (review M2).
    public static func trailerBelow(_ size: DynamicTypeSize) -> Bool { size.isAccessibilitySize }

    /// Whether steps go down the page rather than across.
    public static func stepsDown(width: CGFloat?) -> Bool {
        guard let width, width > 0 else { return false }
        return width < narrow
    }

    /// A table row as VoiceOver reads it, each value with its column's
    /// title: "Lane, ov-274-phones. Cards, ov-274. State, In review."
    /// Empty cells are left out.
    public static func spokenRow(columns: [PageColumn], cells: [PageCell], world: PageWorld) -> String {
        zip(columns, cells).compactMap { column, cell -> String? in
            let text = world.parts(cell).spoken
            guard !text.isEmpty else { return nil }
            return column.title.isEmpty ? "\(text)." : "\(column.title), \(text)."
        }.joined(separator: " ")
    }

    /// The entries in the order they're drawn: newest first, unless the
    /// orchestrator gave its own order.
    public static func ordered(_ entries: [PageEntry], given: Bool) -> [PageEntry] {
        given ? entries : entries.enumerated().sorted { a, b in
            a.element.at != b.element.at ? a.element.at > b.element.at : a.offset < b.offset
        }.map(\.element)
    }

    /// A timeline's time in the viewer's zone: "15:02" today, "Oct 3, 15:02"
    /// before.
    public static func time(_ ms: Int64, now: Int64, calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000)
        let today = Date(timeIntervalSince1970: Double(now) / 1000)
        var style: Date.FormatStyle =
            calendar.isDate(date, inSameDayAs: today)
            ? .dateTime.hour().minute() : .dateTime.month(.abbreviated).day().hour().minute()
        style.timeZone = calendar.timeZone
        return date.formatted(style)
    }

    /// A progress block's fraction, held inside 0...1.
    public static func fraction(done: Int, total: Int) -> Double {
        guard total > 0 else { return 0 }
        return min(1, max(0, Double(done) / Double(total)))
    }
}

/// The Markdown a text block draws (design 3.6): paragraphs, lists two
/// levels deep, bold, italic, code spans, `https` links and task keys. A
/// heading, a table, a fence or a quote draws as its plain words: those are
/// what the blocks are for. Any link but `https` (and a task link) loses its
/// link and keeps its words.
public enum PageMarkdown {
    public enum Piece: Equatable, Sendable {
        /// A paragraph, inline Markdown.
        case prose(String)
        /// A list item: its marker, inline Markdown, and 0 or 1 deep.
        case item(marker: String, text: String, depth: Int)
        /// Words drawn as they are.
        case plain(String)
    }

    public static func pieces(_ md: String) -> [Piece] {
        Markdown.blocks(md).compactMap { block -> Piece? in
            switch block {
            case .paragraph(let text): .prose(text)
            case .bullet(let text, let depth): .item(marker: "•", text: text, depth: min(depth, 1))
            case .numbered(let number, let text, let depth): .item(marker: "\(number).", text: text, depth: min(depth, 1))
            case .heading(_, let text): .plain(text)
            case .code(let text, _): .plain(text)
            case .quote(let text): .plain(text)
            case .rule: nil
            case .table(let header, let rows): .plain(([header] + rows).map { $0.joined(separator: "  ") }.joined(separator: "\n"))
            }
        }
    }

    /// Inline Markdown with only `https` and task links kept, and each web
    /// link followed by its domain in secondary text, so a label can't hide
    /// where it goes (design 7, the owner's ruling on Q5). A label that is
    /// already its domain isn't repeated.
    public static func inline(_ text: String) -> AttributedString {
        var parsed = Markdown.inline(text)
        for run in parsed.runs {
            guard let url = run.link else { continue }
            if PageLinks.https(url.absoluteString) == nil && TaskKeyLinks.parse(url) == nil {
                parsed[run.range].link = nil
            }
        }
        // Whole links, however many styled runs each spans; last first, so
        // an insertion doesn't move a range still to come.
        var domains: [(AttributedString.Index, String)] = []
        for (link, range) in parsed.runs[\.link] {
            guard let link, PageLinks.https(link.absoluteString) != nil, let host = link.host() else { continue }
            let label = String(parsed[range].characters).trimmingCharacters(in: .whitespaces).lowercased()
            if label == host.lowercased() || label == link.absoluteString.lowercased() { continue }
            domains.append((range.upperBound, host))
        }
        for (at, host) in domains.reversed() {
            var domain = AttributedString(" \(host)")
            domain.foregroundColor = .secondary
            parsed.insert(domain, at: at)
        }
        return parsed
    }
}

/// Children left to right, wrapping at the width offered: a stats row, a
/// row of links, steps across.
struct PageFlow: Layout {
    var spacing: CGFloat
    var lineSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        return arrange(subviews, width: width).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let placed = arrange(subviews, width: bounds.width)
        for (view, frame) in zip(subviews, placed.frames) {
            view.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: ProposedViewSize(frame.size))
        }
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> (frames: [CGRect], size: CGSize) {
        var frames: [CGRect] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var line: CGFloat = 0
        var widest: CGFloat = 0
        for view in subviews {
            var size = view.sizeThatFits(.unspecified)
            if size.width > width {
                // Wider than the row: as wide as it, and as tall as that makes it.
                size = view.sizeThatFits(ProposedViewSize(width: width, height: nil))
            }
            if x > 0, x + size.width > width {
                y += line + lineSpacing
                x = 0
                line = 0
            }
            frames.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
            x += size.width + spacing
            line = max(line, size.height)
            widest = max(widest, x - spacing)
        }
        return (frames, CGSize(width: width.isFinite ? min(widest, width) : widest, height: y + line))
    }
}
