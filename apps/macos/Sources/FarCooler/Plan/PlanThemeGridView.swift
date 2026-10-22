import AgentKit
import SwiftUI

// The themes grid (ov-503, design 5): a tile a theme, modeled on the site's
// "What it's building now". Each tile says the theme's name, how much is done,
// a meter, up to three cards and, in amber, what waits on the owner. A click
// opens the theme's page, where the story now lives. The rules (which cards,
// which themes lead, which are finished) are AgentKit's `PlanThemeGrid`.

/// Places its subviews in as many equal columns as fit at least `minimum` wide,
/// every tile in a row as tall as the tallest, which a lazy grid can't do.
struct PlanTileLayout: Layout {
    var minimum: CGFloat = PlanMetrics.tile
    var gap: CGFloat = Spacing.inset

    private func columns(_ width: CGFloat, count: Int) -> Int {
        max(1, min(count, Int((width + gap) / (minimum + gap))))
    }

    private func rows(_ proposal: ProposedViewSize, _ subviews: Subviews) -> (width: CGFloat, columns: Int, heights: [CGFloat]) {
        let width = proposal.width ?? minimum
        let count = columns(width, count: subviews.count)
        let each = (width - gap * CGFloat(count - 1)) / CGFloat(count)
        let sizes = subviews.map { $0.sizeThatFits(ProposedViewSize(width: each, height: nil)).height }
        let heights = stride(from: 0, to: sizes.count, by: count).map { start in sizes[start..<min(start + count, sizes.count)].max() ?? 0 }
        return (width, count, heights)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard !subviews.isEmpty else { return .zero }
        let laid = rows(proposal, subviews)
        return CGSize(width: laid.width, height: laid.heights.reduce(0, +) + gap * CGFloat(max(0, laid.heights.count - 1)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard !subviews.isEmpty else { return }
        let laid = rows(ProposedViewSize(width: bounds.width, height: nil), subviews)
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
    let model: PlanModel
    let selected: PlanPage?
    let keyed: Bool
    let onOpen: (PlanPage) -> Void

    var body: some View {
        PlanTileLayout {
            ForEach(grid.tiles) { tile in
                PlanThemeTileView(
                    tile: tile, budget: model.themes.first { $0.id == tile.id }.flatMap { PlanWords.overBudget($0) }.map { PlanWords.budgetLine($0) },
                    selected: selected == .theme(tile.id), keyed: keyed
                ) { onOpen(.theme(tile.id)) }
                    .changeWashed(tile.id)
            }
            if let title = grid.finishedTitle, let words = grid.finishedWords {
                PlanFinishedTile(title: title, words: words, names: grid.finished.map(\.name))
            }
        }
        .listChanges(grid.tiles.map { tile in
            let spent = model.themes.first { $0.id == tile.id }?.spend?.totalTokens ?? 0
            let cards = tile.cards.map { "\($0.key)\($0.status)" }.joined(separator: ",")
            return ListChangeRow(
                id: tile.id, signature: [tile.name, tile.progress, tile.ownerAsk ?? "", "\(spent)", cards].joined(separator: "\u{1}"))
        })
        .identified("plan-theme-grid")
    }
}

/// A card's mark on a tile: the site's three, a shape before any color.
struct PlanCardMark: View {
    let glyph: PlanCardGlyph

    var body: some View {
        Image(systemName: name)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.quietInk)
            .frame(width: NavigatorGrid.mark - NavigatorGrid.gap, alignment: .center)
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
                Capsule().fill(Fill.inset())
                Capsule().fill(.secondary).frame(width: proxy.size.width * CGFloat(min(1, max(0, fraction))))
            }
        }
        .frame(height: PlanMetrics.bar)
        .accessibilityHidden(true)
    }
}

struct PlanThemeTileView: View {
    let tile: PlanThemeTile
    /// "Over budget: 6.1M of 5M tokens", or nil: amber, with its words.
    var budget: String?
    @Environment(\.colorScheme) private var scheme
    let selected: Bool
    let keyed: Bool
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: Spacing.group) {
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    Text(tile.name)
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(tile.progress)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.quietInk)
                        .lineLimit(1)
                        .identified("plan-tile-\(tile.name)-progress")
                }
                PlanMeter(fraction: tile.fraction)
                if !tile.cards.isEmpty {
                    VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                        ForEach(tile.cards) { card in
                            HStack(alignment: .firstTextBaseline, spacing: NavigatorGrid.gap) {
                                PlanCardMark(glyph: card.glyph)
                                Text(card.title.isEmpty ? card.key : card.title)
                                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                    .foregroundStyle(card.glyph == .done ? .quietInk : .primary)
                                    .lineLimit(1)
                                    .truncationMode(.tail)
                                    .help("\(card.key) \u{00B7} \(PlanCardMark.word(card.glyph))")
                            }
                        }
                    }
                }
                if let budget {
                    Text(budget)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                        .foregroundStyle(Tint.attention(scheme))
                        .fixedSize(horizontal: false, vertical: true)
                        .identified("plan-tile-\(tile.name)-over-budget")
                }
                if let ask = tile.ownerAsk { PlanAsk(text: ask, lines: 3) }
                Spacer(minLength: 0)
            }
            .padding(Spacing.inset)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .contentShape(.card)
            .surface(.inset, in: .card)
            .overlay {
                if selected { RoundedRectangle.card.fill(Fill.selection(active: keyed)) }
            }
            .overlay {
                if hovering && !selected { RoundedRectangle.card.fill(Fill.hover) }
            }
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .identified("plan-theme-tile-\(tile.name)")
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
        VStack(alignment: .leading, spacing: Spacing.group) {
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                Text(title).font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
                Text(words).font(.system(size: WorkspaceStyle.PaneText.secondary)).foregroundStyle(.quietInk)
            }
            Text(names.joined(separator: ", "))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.quietInk)
                .lineLimit(5)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(Spacing.inset)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .surface(.inset, in: .card)
        .accessibilityElement(children: .combine)
        .identified("plan-theme-finished")
    }
}
