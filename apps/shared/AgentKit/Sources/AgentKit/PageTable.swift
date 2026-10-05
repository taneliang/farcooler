import SwiftUI

// A page's table (ov-269 design 3.3, 3.7, 6.2): a grid on a wide surface,
// stacked rows on a narrow one. VoiceOver reads each row as one element with
// its column titles, either way: "Lane, ov-274-phones. Cards, ov-274. State,
// In review."

/// What a cell draws and where it goes: a pure reading of the cell, so the
/// rules can be held by a test.
public struct PageCellParts: Equatable, Sendable {
    /// Its words: its own text, or the target's live name, state or tokens.
    public var text: String
    /// A link's domain, drawn after the words when they aren't it already:
    /// a label never hides where a link goes (design 7).
    public var domain: String?
    /// Where it opens, or nil for words.
    public var destination: PageDestination?
    /// What VoiceOver hears for it: its words, and for a link whose words
    /// aren't its domain, where it goes (review L2).
    public var spoken: String { domain.map { "\(text), link to \($0)" } ?? text }
    /// What VoiceOver's action for it is called: "Open ov-274".
    public var action: String? { destination == nil ? nil : "Open \(spoken)" }
}

extension PageWorld {
    /// `cell` as it's drawn: a reference drawn by name, or text the
    /// orchestrator wrote over one, is a link (design 3.4: "with `text`, the
    /// reference is only the link"); a lane's live state or tokens are words.
    public func parts(_ cell: PageCell) -> PageCellParts {
        let text = cellText(cell)
        guard let ref = cell.ref, cell.text != nil || cell.show == .name else {
            return PageCellParts(text: text)
        }
        let resolved = resolve(ref)
        guard let destination = resolved.destination else { return PageCellParts(text: text) }
        var domain: String?
        if case .url(let url) = destination, let host = url.host(), text.lowercased() != host.lowercased() { domain = host }
        return PageCellParts(text: text, domain: domain, destination: destination)
    }

    /// One "Open …" action per link in a row, so VoiceOver reaches every
    /// link a row speaks for, not only the first.
    public func actions(_ cells: [PageCell]) -> [(name: String, destination: PageDestination)] {
        cells.compactMap { cell in
            let parts = parts(cell)
            guard let name = parts.action, let destination = parts.destination else { return nil }
            return (name, destination)
        }
    }
}

/// One cell's words: a link in the tint with its domain after it when it
/// leaves the app, or the orchestrator's words.
struct PageCellView: View {
    let cell: PageCell
    let world: PageWorld
    let onOpen: (PageDestination) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    var body: some View {
        let parts = world.parts(cell)
        if let destination = parts.destination {
            Button { PageOpen.open(destination, onOpen: onOpen, openURL: openURL) } label: {
                (Text(parts.text).foregroundStyle(.tint)
                    + Text(parts.domain.map { " \($0)" } ?? "").foregroundStyle(.secondary))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .buttonStyle(.plain)
            .font(cell.mono ? Font.body.monospaced() : nil)
            .help(parts.domain.map { "\(parts.text), \($0)" } ?? parts.text)
        } else {
            Text(parts.text)
                .font(cell.mono ? Font.body.monospaced() : nil)
                .fontWeight(cell.tone == .attention ? .medium : nil)
                .foregroundStyle(cell.tone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary))
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

extension PageColumn.Align {
    var horizontal: HorizontalAlignment {
        switch self {
        case .start: .leading
        case .center: .center
        case .end: .trailing
        }
    }

    var frame: Alignment {
        switch self {
        case .start: .topLeading
        case .center: .top
        case .end: .topTrailing
        }
    }
}

/// Each body row's top and height, measured in the table's own space, so
/// the alternate rows can be shaded edge to edge behind a `Grid`.
private struct PageRowFrames: PreferenceKey {
    static let defaultValue: [Int: CGRect] = [:]
    static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
        value.merge(nextValue(), uniquingKeysWith: { $1 })
    }
}

/// A table on a wide surface: titles over columns, `grow` columns taking the
/// width the others leave, every other row shaded.
struct PageGridTable: View {
    let columns: [PageColumn]
    let rows: [[PageCell]]
    let world: PageWorld
    let onOpen: (PageDestination) -> Void
    @Environment(\.colorSchemeContrast) private var contrast

    private static let space = "page-table"

    var body: some View {
        Grid(alignment: .topLeading, horizontalSpacing: Spacing.inset, verticalSpacing: 0) {
            GridRow {
                ForEach(Array(columns.enumerated()), id: \.offset) { _, column in
                    Text(column.title)
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: column.grow ? .infinity : nil, alignment: column.align.frame)
                        .gridColumnAlignment(column.align.horizontal)
                        .padding(.bottom, Spacing.tight)
                }
            }
            .accessibilityHidden(true)
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                GridRow {
                    ForEach(Array(columns.enumerated()), id: \.offset) { c, column in
                        cell(row, c, column)
                            .padding(.vertical, Spacing.group - 2)
                            .frame(maxWidth: column.grow ? .infinity : nil, maxHeight: c == 0 ? .infinity : nil, alignment: column.align.frame)
                            .background {
                                if c == 0 {
                                    GeometryReader { proxy in
                                        Color.clear.preference(
                                            key: PageRowFrames.self, value: [index: proxy.frame(in: .named(Self.space))])
                                    }
                                }
                            }
                            .modifier(
                                PageRowSpeech(
                                    first: c == 0, spoken: PageLayout.spokenRow(columns: columns, cells: row, world: world),
                                    actions: world.actions(row), onOpen: onOpen))
                            .accessibilityIdentifier(c == 0 ? "page-row-\(index)" : "")
                    }
                }
            }
        }
        .padding(.horizontal, Spacing.group)
        .coordinateSpace(.named(Self.space))
        .backgroundPreferenceValue(PageRowFrames.self) { frames in
            GeometryReader { proxy in
                ForEach(frames.keys.sorted().filter { $0 % 2 == 1 }, id: \.self) { index in
                    if let frame = frames[index] {
                        RoundedRectangle.control.fill(Fill.inset(contrast))
                            .frame(width: proxy.size.width, height: frame.height)
                            .offset(y: frame.minY)
                    }
                }
            }
        }
    }

    @ViewBuilder private func cell(_ row: [PageCell], _ c: Int, _ column: PageColumn) -> some View {
        if c < row.count {
            PageCellView(cell: row[c], world: world, onOpen: onOpen)
                .multilineTextAlignment(column.align == .end ? .trailing : column.align == .center ? .center : .leading)
        } else {
            Color.clear.frame(width: 0, height: 0)
        }
    }
}

/// The first cell of a row speaks for the whole row; the others are silent,
/// so VoiceOver reads a row as one sentence with its titles. Its links stay
/// reachable as the element's actions.
private struct PageRowSpeech: ViewModifier {
    let first: Bool
    let spoken: String
    let actions: [(name: String, destination: PageDestination)]
    let onOpen: (PageDestination) -> Void

    func body(content: Content) -> some View {
        if first {
            content
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(spoken)
                .modifier(PageRowActions(actions: actions, onOpen: onOpen))
        } else {
            content.accessibilityHidden(true)
        }
    }
}

/// A row's links as named actions on its one accessibility element.
struct PageRowActions: ViewModifier {
    let actions: [(name: String, destination: PageDestination)]
    let onOpen: (PageDestination) -> Void
    @Environment(\.openURL) private var openURL

    func body(content: Content) -> some View {
        content.accessibilityActions {
            ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                Button(action.name) { PageOpen.open(action.destination, onOpen: onOpen, openURL: openURL) }
            }
        }
    }
}

/// A table on a narrow surface: each row's first cell as its title, the
/// others as "Title  value" lines under it.
struct PageStackedTable: View {
    let columns: [PageColumn]
    let rows: [[PageCell]]
    let world: PageWorld
    let onOpen: (PageDestination) -> Void
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    if let first = row.first {
                        PageCellView(cell: first, world: world, onOpen: onOpen)
                            .fontWeight(.semibold)
                    }
                    Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: Spacing.inset, verticalSpacing: Spacing.tight / 2) {
                        ForEach(Array(zip(columns, row).enumerated().dropFirst()), id: \.offset) { _, pair in
                            if !world.cellText(pair.1).isEmpty {
                                GridRow {
                                    Text(pair.0.title)
                                        .font(.subheadline)
                                        .foregroundStyle(.secondary)
                                    PageCellView(cell: pair.1, world: world, onOpen: onOpen)
                                }
                            }
                        }
                    }
                    .padding(.leading, Spacing.inset)
                }
                .padding(.vertical, Spacing.group - 2)
                .padding(.horizontal, Spacing.group)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background {
                    if index % 2 == 1 { RoundedRectangle.control.fill(Fill.inset(contrast)) }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(PageLayout.spokenRow(columns: columns, cells: row, world: world))
                .modifier(PageRowActions(actions: world.actions(row), onOpen: onOpen))
                .accessibilityIdentifier("page-stacked-row-\(index)")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("page-stacked-table")
    }
}
