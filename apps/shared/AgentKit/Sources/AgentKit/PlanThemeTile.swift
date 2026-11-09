import Foundation

// A card's mark on a theme tile and in the sidebar: the site's three. Which
// cards a tile shows, which themes lead and which are finished are the Rust
// core's (`PlanView.Themes`, ov-1016); the sidebar's theme tree still reads
// the mark from the board's status here (ov-1015 phase 3).

/// A card's mark on a tile and in the sidebar: the site's three.
public enum PlanCardGlyph: String, Sendable {
    case done, inProgress, notStarted

    /// In review counts as in progress; waiting on a decision hasn't started,
    /// and the tile's ask line says who it waits on.
    public init(_ status: TaskStatus) {
        switch status {
        case .done: self = .done
        case .inProgress, .inReview: self = .inProgress
        case .backlog, .todo, .needsDecision, .cancelled: self = .notStarted
        }
    }
}

extension PlanCardGlyph {
    /// The view's mark for a tile's card.
    public init(_ glyph: PlanView.Glyph) {
        switch glyph {
        case .done: self = .done
        case .inProgress: self = .inProgress
        case .notStarted: self = .notStarted
        }
    }
}
