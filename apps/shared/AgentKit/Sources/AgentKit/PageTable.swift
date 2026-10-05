import SwiftUI

// A page's table (ov-269 design 3.3, 3.7, 6.2): a grid on a wide surface,
// stacked rows on a narrow one. VoiceOver reads each row as one element with
// its column titles, either way: "Lane, ov-274-phones. Cards, ov-274. State,
// In review."

/// One cell's words: a reference's live name as a link, a lane's live state
/// or tokens as words, or the orchestrator's text.
struct PageCellView: View {
    let cell: PageCell
    let world: PageWorld
    let onOpen: (PageDestination) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    var body: some View {
        let text = world.cellText(cell)
        let resolved = cell.ref.map(world.resolve)
        // A reference drawn by name is a link; text the orchestrator wrote
        // over one, and live state words, are words.
        if cell.text == nil, cell.show == .name, let destination = resolved?.destination {
            Button { PageOpen.open(destination, onOpen: onOpen, openURL: openURL) } label: {
                Text(text)
                    .foregroundStyle(.tint)
                    .fixedSize(horizontal: false, vertical: true)
            }
                .buttonStyle(.plain)
                .font(cell.mono ? Font.body.monospaced() : nil)
                .help(resolved?.spoken ?? text)
        } else {
            Text(text)
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
                            .modifier(PageRowSpeech(first: c == 0, spoken: PageLayout.spokenRow(columns: columns, cells: row, world: world)))
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

    func body(content: Content) -> some View {
        if first {
            content
                .accessibilityElement(children: .combine)
                .accessibilityLabel(spoken)
        } else {
            content.accessibilityHidden(true)
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
                .accessibilityElement(children: .combine)
                .accessibilityLabel(PageLayout.spokenRow(columns: columns, cells: row, world: world))
                .accessibilityIdentifier("page-stacked-row-\(index)")
            }
        }
        .accessibilityIdentifier("page-stacked-table")
    }
}
