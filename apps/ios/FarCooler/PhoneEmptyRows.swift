import SwiftUI

/// An empty state's lede and icon rows (ov-245): the Mac's `EmptyStateRows`.
///
/// One short sentence of purpose, then rows of an SF Symbol and a few words,
/// the rows as one leading-aligned block as wide as the cap allows rather than
/// as wide as its longest row, so every state's symbols share one left edge
/// (ov-266), each symbol in a fixed-width column so the words line up. A column
/// too narrow for a row wraps it. The words and their shape are `PhoneEmptyStates`', where they are
/// tested.
struct PhoneEmptyRows: View {
    let copy: PhoneEmptyCopy
    /// Footnote-sized, for a state inside a list row rather than a full pane.
    var compact = false

    var body: some View {
        VStack(spacing: compact ? 8 : 12) {
            Text(copy.lede)
                .multilineTextAlignment(.center)
                .accessibilityIdentifier("empty-lede")
            if !copy.rows.isEmpty {
                VStack(alignment: .leading, spacing: compact ? 6 : 10) {
                    ForEach(copy.rows, id: \.self) { row in
                        HStack(spacing: 10) {
                            Image(systemName: row.symbol)
                                .foregroundStyle(.secondary)
                                .frame(width: 22)
                                .accessibilityHidden(true)
                            Text(row.text)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("empty-row")
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                // More room under the lede than between rows, so it doesn't
                // read as another row.
                .padding(.top, compact ? 4 : 8)
            }
        }
        .font(compact ? .footnote : .callout)
        .foregroundStyle(.secondary)
        .frame(maxWidth: 320)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("empty-rows")
    }
}
