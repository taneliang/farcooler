import SwiftUI

// Cost on the plan, drawn once for the Mac and the iPhone (ov-307). The words
// and rules are `PlanCost.swift`'s; these lay them out. Color is for the one
// state that needs attention, a budget gone over, and it always comes with a
// glyph and words. Every block is one accessibility element that says its
// numbers in words.

/// A theme's seven days as bars, oldest first and today last, each against the
/// busiest day. Neutral: the trend is a shape to read, not a state.
public struct PlanTrendBars: View {
    public let trend: PlanTrend
    public var height: CGFloat

    public init(trend: PlanTrend, height: CGFloat = 28) {
        self.trend = trend
        self.height = height
    }

    public var body: some View {
        let heights = trend.heights()
        HStack(alignment: .bottom, spacing: Spacing.tight) {
            ForEach(Array(heights.enumerated()), id: \.offset) { index, share in
                // Today, the last bar, is the darkest: the one that's still moving.
                Rectangle()
                    .fill(index == heights.count - 1 ? AnyShapeStyle(.secondary) : AnyShapeStyle(.tertiary))
                    .frame(maxWidth: .infinity)
                    .frame(height: max(share > 0 ? 2 : 0, height * share))
            }
        }
        .frame(height: height, alignment: .bottom)
        .background(alignment: .bottom) {
            // The baseline, so a day with nothing still has a place.
            Capsule().fill(Fill.inset()).frame(height: 2)
        }
        .accessibilityElement()
        .accessibilityLabel(PlanWords.trendSpoken(trend))
        .accessibilityIdentifier("plan-trend")
    }
}

/// "Over budget: 6.1M of 5M tokens" in amber with a glyph, or "1.2M of 5M
/// tokens budgeted" in the neutral secondary.
public struct PlanBudgetLine: View {
    public let budget: PlanBudget
    public var font: Font
    @Environment(\.colorScheme) private var scheme

    public init(budget: PlanBudget, font: Font = .footnote) {
        self.budget = budget
        self.font = font
    }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.tight + 2) {
            if budget.isOver {
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundStyle(Tint.attention(scheme))
                    .accessibilityHidden(true)
            }
            Text(PlanWords.budgetLine(budget))
                .fontWeight(budget.isOver ? .medium : .regular)
                .foregroundStyle(budget.isOver ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
        }
        .font(font)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.budgetSpoken(budget))
        .accessibilityIdentifier(budget.isOver ? "plan-budget-over" : "plan-budget")
    }
}

/// A theme's spend: what its lanes spent, the budget against it, and the last
/// seven days. Draw it only when `hasSomething`.
public struct PlanThemeSpend: View {
    public let theme: PlanTheme
    public var bodyFont: Font
    public var secondaryFont: Font

    public init(theme: PlanTheme, bodyFont: Font = .body, secondaryFont: Font = .footnote) {
        self.theme = theme
        self.bodyFont = bodyFont
        self.secondaryFont = secondaryFont
    }

    /// Spend, a budget or a trend: a theme with none has nothing to say.
    public static func hasSomething(_ theme: PlanTheme) -> Bool {
        (theme.spend?.totalTokens ?? 0) > 0 || theme.budgetTokens != nil || PlanWords.trend(theme.trendTokens) != nil
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let spend = theme.spend, spend.totalTokens > 0 {
                Text(PlanWords.spend(spend)).font(bodyFont).fixedSize(horizontal: false, vertical: true)
            }
            if let budget = PlanWords.budget(theme.spend, against: theme.budgetTokens) {
                PlanBudgetLine(budget: budget, font: secondaryFont)
            }
            if let trend = PlanWords.trend(theme.trendTokens) {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    PlanTrendBars(trend: trend)
                    Text("Last 7 days, today on the right")
                        .font(secondaryFont)
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(PlanWords.trendSpoken(trend))
            }
        }
        .accessibilityIdentifier("plan-theme-spend")
    }
}

/// The runner's week, and cost per finished card by harness and model. The
/// week has no percentage, and says why; a pair with fewer than three finished
/// cards isn't drawn, and the note says how many weren't.
public struct PlanCostBlock: View {
    public let cost: PlanCostRead
    public var bodyFont: Font
    public var secondaryFont: Font

    public init(cost: PlanCostRead, bodyFont: Font = .body, secondaryFont: Font = .footnote) {
        self.cost = cost
        self.bodyFont = bodyFont
        self.secondaryFont = secondaryFont
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.section) {
            if cost.weekTokens > 0 {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    Text(PlanWords.week(cost.weekTokens)).font(bodyFont)
                    Text(PlanWords.weekNote)
                        .font(secondaryFont)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("plan-cost-week")
            }
            if !cost.compare.isEmpty || cost.compareHeldBack > 0 {
                VStack(alignment: .leading, spacing: Spacing.group) {
                    Text("Cost per finished card")
                        .font(secondaryFont.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(cost.compare.map { PlanWords.compareRow($0) }) { row in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.title).font(bodyFont.weight(.medium))
                            Text(row.detail).font(secondaryFont).foregroundStyle(.secondary)
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityLabel(row.spoken)
                        .accessibilityIdentifier("plan-compare-\(row.id)")
                    }
                    if let held = PlanWords.heldBack(cost.compareHeldBack) {
                        Text(held)
                            .font(secondaryFont)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("plan-compare-held-back")
                    }
                    if !cost.compare.isEmpty {
                        Text(PlanWords.compareNote)
                            .font(secondaryFont)
                            .foregroundStyle(.tertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityIdentifier("plan-cost")
    }
}
