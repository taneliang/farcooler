import SwiftUI

// The themes grid on a phone and an iPad (ov-503, design 5): a tile a theme,
// modeled on the site's "What it's building now". Each tile says the theme's
// name, how much is done, a meter, up to three cards and, in amber, what waits
// on the owner. A tap opens the theme's page, where the story now lives. The
// rules (which cards, which themes lead, which are finished) are AgentKit's
// `PlanThemeGrid`; one column on a phone, two in an iPad's plan column.

/// Places its subviews in `columns` equal columns, every tile in a row as tall
/// as the tallest, which a lazy grid can't do.
struct PlanTileLayout: Layout {
    var columns: Int
    var gap: CGFloat = PaneMetrics.card

    private func count(_ subviews: Subviews) -> Int { max(1, min(columns, subviews.count)) }

    private func rows(_ width: CGFloat, _ subviews: Subviews) -> (columns: Int, heights: [CGFloat]) {
        let across = count(subviews)
        let each = (width - gap * CGFloat(across - 1)) / CGFloat(across)
        let sizes = subviews.map { $0.sizeThatFits(ProposedViewSize(width: each, height: nil)).height }
        let heights = stride(from: 0, to: sizes.count, by: across).map { start in
            sizes[start..<min(start + across, sizes.count)].max() ?? 0
        }
        return (across, heights)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard !subviews.isEmpty else { return .zero }
        let width = proposal.width ?? 320
        let laid = rows(width, subviews)
        return CGSize(width: width, height: laid.heights.reduce(0, +) + gap * CGFloat(max(0, laid.heights.count - 1)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard !subviews.isEmpty else { return }
        let laid = rows(bounds.width, subviews)
        let each = (bounds.width - gap * CGFloat(laid.columns - 1)) / CGFloat(laid.columns)
        var y = bounds.minY
        for (row, height) in laid.heights.enumerated() {
            for column in 0..<laid.columns {
                let index = row * laid.columns + column
                guard index < subviews.count else { break }
                subviews[index].place(
                    at: CGPoint(x: bounds.minX + CGFloat(column) * (each + gap), y: y), anchor: .topLeading,
                    proposal: ProposedViewSize(width: each, height: height))
            }
            y += height + gap
        }
    }
}

struct PlanThemeGridView: View {
    let grid: PlanThemeGrid
    let plan: PlanModel
    let columns: Int
    let onOpen: (String) -> Void

    var body: some View {
        PlanTileLayout(columns: columns) {
            ForEach(grid.tiles) { tile in
                PlanThemeTileView(
                    tile: tile, columns: columns,
                    budget: plan.themes.first { $0.id == tile.id }.flatMap { PlanWords.overBudget($0) }.map { PlanWords.budgetLine($0) }
                ) { onOpen(tile.id) }
            }
            if let title = grid.finishedTitle, let words = grid.finishedWords {
                PlanFinishedTile(title: title, words: words, names: grid.finished.map(\.name))
            }
        }
        .padding(.vertical, PaneMetrics.step)
    }
}

/// A card's mark on a tile: the site's three, a shape before any color.
struct PlanCardMark: View {
    let glyph: PlanCardGlyph

    var body: some View {
        Image(systemName: name)
            .font(.footnote)
            .foregroundStyle(glyph == .notStarted ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.secondary))
            .frame(width: 16, alignment: .center)
            .accessibilityHidden(true)
    }

    private var name: String {
        switch glyph {
        case .done: "checkmark.circle"
        case .inProgress: "circle.lefthalf.filled"
        case .notStarted: "circle"
        }
    }

    /// What VoiceOver says for it.
    static func word(_ glyph: PlanCardGlyph) -> String {
        switch glyph {
        case .done: "Done"
        case .inProgress: "In progress"
        case .notStarted: "Not started"
        }
    }
}

/// The theme's cards by how far along they are, as one meter: the filled part
/// is what's done.
struct PlanMeter: View {
    let fraction: Double

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(.quaternary)
                Capsule().fill(.secondary).frame(width: proxy.size.width * CGFloat(min(1, max(0, fraction))))
            }
        }
        .frame(height: 4)
        .accessibilityHidden(true)
    }
}

/// The paper a tile sits on: the grouped list's own cell color, so a tile reads
/// as a card on the list's ground in light and dark.
private struct TileSurface: ViewModifier {
    func body(content: Content) -> some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .surface(.content, in: .card, fill: Color(uiColor: .secondarySystemGroupedBackground))
    }
}

struct PlanThemeTileView: View {
    let tile: PlanThemeTile
    let columns: Int
    /// "Over budget: 6.1M of 5M tokens", or nil: amber, with its words.
    var budget: String?
    let action: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: PaneMetrics.step) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(tile.name)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(tile.progress)
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .accessibilityIdentifier("plan-tile-\(tile.name)-progress")
                }
                PlanMeter(fraction: tile.fraction)
                if !tile.cards.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(tile.cards) { card in
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                PlanCardMark(glyph: card.glyph)
                                Text(card.title.isEmpty ? card.key : card.title)
                                    .font(.footnote)
                                    .foregroundStyle(card.glyph == .done ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                                    .lineLimit(columns > 1 ? 1 : 2)
                                    .truncationMode(.tail)
                                    .multilineTextAlignment(.leading)
                            }
                        }
                    }
                }
                if let budget {
                    Text(budget)
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(GlancePalette.amber(scheme))
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("plan-budget-over")
                }
                if let ask = tile.ownerAsk { PlanAsk(text: ask, lines: 3) }
                Spacer(minLength: 0)
            }
            .padding(PaneMetrics.card)
            .contentShape(.rect)
            .modifier(TileSurface())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-theme-\(tile.name)")
    }

    private var accessibility: String {
        var parts = [tile.name, tile.progress]
        parts += tile.cards.map { "\($0.title.isEmpty ? $0.key : $0.title), \(PlanCardMark.word($0.glyph).lowercased())" }
        if let budget { parts.append(budget) }
        if let ask = tile.ownerAsk { parts.append("Needs you: \(ask)") }
        return parts.joined(separator: ", ")
    }
}

/// The one tile for every finished theme: how many, and their names.
struct PlanFinishedTile: View {
    let title: String
    let words: String
    let names: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(words).font(.footnote).foregroundStyle(.secondary)
            }
            Text(names.joined(separator: ", "))
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(5)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(PaneMetrics.card)
        .modifier(TileSurface())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("plan-theme-finished")
    }
}
