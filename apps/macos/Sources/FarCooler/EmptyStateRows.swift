import AgentKit
import SwiftUI

/// What an empty state says under its title, as short rows rather than a
/// paragraph (ov-205).
///
/// The owner read the first version, one five-line paragraph under "No
/// Workspace Selected", as "too many words": nobody scans a paragraph. So
/// an empty state says what the thing is for in one short line, then two or
/// three rows, each a symbol and a few words, then its button.
struct EmptyStateCopy: Equatable {
    /// One row: an SF Symbol and a few words, in sentence case with no
    /// period, since a row reads as an item in a list.
    struct Row: Equatable, Hashable {
        let symbol: String
        let text: String
    }

    /// The thing's purpose, in one short sentence; nil where the title says
    /// it already.
    let lede: String?
    let rows: [Row]
}

/// `EmptyStateCopy`, drawn: the lede centered, then the rows as one
/// leading-aligned block, centered under it, each symbol in a fixed-width
/// column so the words line up. The block is as wide as its widest row, so a
/// short row never sits off-center under the title and lede (owner, 4 October:
/// screens are never seen side by side, so a shared left edge across them buys
/// nothing). `width` caps it so nothing wraps to a one-word line; a column
/// narrower than a row wraps it.
struct EmptyStateRows: View {
    let copy: EmptyStateCopy

    var body: some View {
        VStack(spacing: Spacing.inset) {
            if let lede = copy.lede {
                Text(lede)
                    .multilineTextAlignment(.center)
            }
            VStack(alignment: .leading, spacing: Spacing.group) {
                ForEach(copy.rows, id: \.self) { row in
                    HStack(spacing: Spacing.group) {
                        Image(systemName: row.symbol)
                            .foregroundStyle(.secondary)
                            .frame(width: 20)
                            .accessibilityHidden(true)
                        Text(row.text)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            // More room under the lede than between rows, so it doesn't read
            // as another row.
            .padding(.top, copy.lede == nil ? 0 : Spacing.group)
        }
        .frame(minWidth: 0, idealWidth: Self.width, maxWidth: Self.width)
    }

    /// The most the block is wide, where the column allows it: wide enough that
    /// no lede or row wraps to a one-word line.
    static let width: CGFloat = 360
}

/// An empty state's title and symbol, for a `ContentUnavailableView`'s label.
///
/// The view draws its whole label in the secondary color, and a title that is
/// as grey as the sentence under it reads as part of it. The title is the one
/// thing that says what this is, so it's primary (owner, 4 October); the symbol
/// stays secondary, as the system draws it.
struct EmptyStateTitle: View {
    let title: String
    let symbol: String

    init(_ title: String, symbol: String) {
        self.title = title
        self.symbol = symbol
    }

    var body: some View {
        Label {
            Text(title).foregroundStyle(.primary)
        } icon: {
            Image(systemName: symbol)
        }
    }
}
