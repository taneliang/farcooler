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
/// column so the words line up. A column too narrow for a row wraps it.
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
        }
    }
}
