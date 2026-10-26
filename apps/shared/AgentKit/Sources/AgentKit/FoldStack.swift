import SwiftUI

// A long message or report folded under Show More, laid out in one pass
// (ov-675, ov-691): both apps' conversations fold this way, so a row is as
// tall on its first layout as it will stay.

/// How a `FoldStack` folds its words.
public enum Fold: Equatable, Sendable {
    /// The words fold themselves (a line limit); the third view is the same
    /// words unlimited, and they fold where it is taller.
    case lines
    /// The words are clipped to `lines` lines and a paragraph break, as tall
    /// as the third view (one line of body text) says a line is.
    case height(lines: Int)
}

/// Words, Show More under them, and a measure, laid out in one pass: whether
/// the words fold, and so the row's height, is known on the first layout
/// rather than the frame after. The button is always there, placed and laid
/// out only where the words fold; `foldButton` hides it otherwise.
///
/// A lazy list builds a row as it nears the screen. A row that folded on its
/// second pass was laid out whole first and then shrank (by 344 pt for a
/// report of eight paragraphs on the iPhone, 145 pt on the Mac), and built
/// just above the screen as the reader scrolled up, it moved every row on
/// the screen.
public struct FoldStack: Layout {
    let fold: Fold
    let open: Bool
    var spacing: CGFloat

    public init(fold: Fold, open: Bool, spacing: CGFloat = Spacing.tight) {
        self.fold = fold
        self.open = open
        self.spacing = spacing
    }

    private struct Plan {
        var width: CGFloat
        var words: CGFloat
        var button: CGSize?
    }

    private func plan(_ proposal: ProposedViewSize, _ subviews: Subviews) -> Plan {
        guard subviews.count == 3 else { return Plan(width: 0, words: 0) }
        let width = ProposedViewSize(width: proposal.width, height: nil)
        let words = subviews[0].sizeThatFits(width)
        let folds: Bool
        var height = words.height
        switch fold {
        case .lines:
            folds = open || subviews[2].sizeThatFits(width).height > words.height + 1
        case .height(let lines):
            let line = subviews[2].sizeThatFits(.unspecified).height
            let cap = line * CGFloat(lines + 1)
            folds = line > 0 && words.height > cap + 1
            if folds && !open { height = cap }
        }
        let button = folds ? subviews[1].sizeThatFits(width) : nil
        return Plan(width: max(self.width(proposal, words: words), button?.width ?? 0), words: height, button: button)
    }

    /// The width the stack takes. Words that fold by a line limit (a
    /// message) are as wide as their longest line, never the whole proposal,
    /// so a bubble around a short message hugs it, as `Text` does (ov-809).
    /// Reading `proposal.width` here instead made every bubble as wide as
    /// its cap since ov-675. A report's blocks (`.height`) fill their column.
    private func width(_ proposal: ProposedViewSize, words: CGSize) -> CGFloat {
        switch fold {
        case .lines: min(words.width, proposal.width ?? words.width)
        case .height: proposal.width ?? words.width
        }
    }

    public func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let plan = plan(proposal, subviews)
        return CGSize(width: plan.width, height: plan.words + (plan.button.map { spacing + $0.height } ?? 0))
    }

    public func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 3 else { return }
        let plan = plan(proposal, subviews)
        subviews[0].place(at: bounds.origin, proposal: ProposedViewSize(width: bounds.width, height: plan.words))
        let below = CGPoint(x: bounds.minX, y: bounds.minY + plan.words + spacing)
        subviews[1].place(at: below, proposal: plan.button.map { ProposedViewSize($0) } ?? .zero)
        subviews[2].place(at: bounds.origin, proposal: ProposedViewSize(width: bounds.width, height: nil))
    }
}

extension View {
    /// A `FoldStack`'s Show More: in the tree only where the words fold, so
    /// it is never seen, clicked or read (even inside a combined element)
    /// under words that don't. A hidden copy keeps its size either way, so
    /// the stack's room for it never waits on this.
    public func foldButton(shown: Bool) -> some View {
        ZStack(alignment: .leading) {
            hidden()
            if shown { self }
        }
    }
}
