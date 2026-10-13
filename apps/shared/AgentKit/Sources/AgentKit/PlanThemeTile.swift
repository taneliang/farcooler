import Foundation

// The themes grid (ov-503, design 5): a tile a theme, modeled on the site's
// "What it's building now". The tile replaces the prose briefs on the Plan:
// the name, how much is done, a meter, up to three cards and the owner's ask.
// The rules for which cards, which themes lead and which are finished are
// values here, so the Mac, the phones and the site's generator pick the same.

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

public struct PlanThemeTile: Equatable, Identifiable, Sendable {
    public struct Card: Equatable, Identifiable, Sendable {
        public var task: String
        public var key: String
        public var title: String
        public var status: TaskStatus
        public var glyph: PlanCardGlyph { PlanCardGlyph(status) }
        public var id: String { task }
    }

    public var id: String
    public var name: String
    public var done: Int
    /// The cards that count: all but the canceled.
    public var total: Int
    /// Up to three, in progress first, then the most recent done, then the
    /// next not started.
    public var cards: [Card]
    /// What waits on the owner: the one amber line. Nil when nothing does.
    public var ownerAsk: String?
    /// A lane of it is shipping, in review or building.
    public var isMoving: Bool
    /// When it last moved, in ms.
    public var lastMoved: Int64

    /// How many cards a tile shows.
    public static let cardsShown = 3

    /// "6 of 10 done".
    public var progress: String { "\(done) of \(total) done" }

    /// How full the meter is, 0 to 1.
    public var fraction: Double { total > 0 ? Double(done) / Double(total) : 0 }

    /// Every card done, or the theme marked so.
    public var isFinished: Bool

    init(
        theme: PlanTheme, plan: PlanModel, moving: Set<String>, statuses: [String: TaskStatus], activity: [String: Int64]
    ) {
        let total = PlanWords.total(theme.counts)
        id = theme.id
        name = theme.name
        done = theme.counts.done
        self.total = total
        ownerAsk = theme.ownerAsk.isEmpty ? nil : theme.ownerAsk
        lastMoved = plan.lastMoved(theme, activity: activity)
        isMoving = plan.lanes(in: theme).contains { moving.contains($0.id) }
        isFinished = theme.state == "done" || (total > 0 && theme.counts.done == total)

        // The board's status wins; else the plan's own word for it. A card
        // the plan can't place, or one that was canceled, isn't shown.
        let listed: [Card] = theme.cards.compactMap { ref in
            let known = plan.card(ref.task)
            guard let status = statuses[ref.task] ?? known.flatMap({ OneTreeTask.status(word: $0.status) }),
                status != .cancelled
            else { return nil }
            return Card(task: ref.task, key: ref.key, title: known?.title ?? "", status: status)
        }
        let recent = { (card: Card) in activity[card.task] ?? 0 }
        let notStarted = listed.first { $0.glyph == .notStarted }.map { [$0] } ?? []
        let inProgress = Array(listed.filter { $0.glyph == .inProgress }.prefix(Self.cardsShown - notStarted.count))
        let room = Self.cardsShown - notStarted.count - inProgress.count
        // Later in a theme's list is later added; the board's word on when it
        // moved breaks that.
        let finished = listed.enumerated().filter { $0.element.glyph == .done }
            .sorted { (recent($0.element), $0.offset) > (recent($1.element), $1.offset) }
            .prefix(room).map(\.element)
        cards = inProgress + finished + notStarted
    }
}

/// The tiles, and the one tile for what's finished.
public struct PlanThemeGrid: Equatable, Sendable {
    /// Themes in motion first, then the rest by last move.
    public var tiles: [PlanThemeTile]
    /// Done themes, which the grid folds into "Finished".
    public var finished: [PlanThemeTile]

    public init(plan: PlanModel, moving: Set<String>, statuses: [String: TaskStatus] = [:], activity: [String: Int64] = [:]) {
        let all = plan.themes.filter { $0.state != "dropped" }.map {
            PlanThemeTile(theme: $0, plan: plan, moving: moving, statuses: statuses, activity: activity)
        }
        let ordinal = Dictionary(uniqueKeysWithValues: plan.themes.map { ($0.id, $0.ordinal) })
        tiles = all.filter { !$0.isFinished }.sorted {
            if $0.isMoving != $1.isMoving { return $0.isMoving }
            if $0.lastMoved != $1.lastMoved { return $0.lastMoved > $1.lastMoved }
            return (ordinal[$0.id] ?? 0) < (ordinal[$1.id] ?? 0)
        }
        finished = all.filter(\.isFinished).sorted { $0.lastMoved > $1.lastMoved }
    }

    public var isEmpty: Bool { tiles.isEmpty && finished.isEmpty }

    /// "Finished", the one tile's title; nil when nothing is.
    public var finishedTitle: String? { finished.isEmpty ? nil : "Finished" }

    /// "7 themes", "1 theme".
    public var finishedWords: String? {
        finished.isEmpty ? nil : (finished.count == 1 ? "1 theme" : "\(finished.count) themes")
    }
}
