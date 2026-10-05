import Foundation

// What a theme's entry in the canvas says, and in what order (ov-331, design
// 2.1): its outcome, its story with its age, what it needs from the owner,
// what's next, the lanes moving it, what landed this week, what was decided
// and, against a budget, its spend. An empty line takes no space: a theme
// with no story says nothing of one.
//
// The Mac draws it as the canvas's entry and the phones draw the parts of it
// their rows keep; this is the one place that says which parts there are.

public struct PlanThemeBrief: Equatable, Sendable {
    /// How many lanes the entry lists before "+2 more".
    public static let laneLimit = 3
    /// How many standing rulings it lists.
    public static let rulingLimit = 2
    /// The outcome's lines when the entry is open, and the story's.
    public static let outcomeLines = 3
    public static let storyLines = 4

    public var theme: PlanTheme
    public var track: PlanTrack
    public var outcome: String?
    public var story: String?
    /// "Updated 1 h ago".
    public var storyAge: String?
    public var ask: String?
    public var next: String?
    /// The lanes moving it, then queued for it: at most `laneLimit`.
    public var moving: [PlanLane]
    public var movingMore: Int
    /// "rulings, plan-v2-glance, +2 more"; nil when none landed this week.
    public var landed: String?
    /// The standing rulings that name it: at most `rulingLimit`.
    public var decided: [PlanRuling]
    public var decidedMore: Int
    /// Its spend against a budget it has and is within; over budget is the
    /// track line's.
    public var budget: PlanBudget?

    public init(_ theme: PlanTheme, in plan: PlanModel, activity: [String: Int64] = [:]) {
        self.theme = theme
        track = plan.track(of: theme, activity: activity)
        outcome = theme.outcome.isEmpty ? nil : theme.outcome
        story = theme.story.isEmpty ? nil : theme.story
        storyAge = PlanWords.storyAge(theme, now: plan.nowMs)
        ask = theme.ownerAsk.isEmpty ? nil : theme.ownerAsk
        next = theme.next.isEmpty ? nil : theme.next
        let lanes = plan.lanes(in: theme).filter { $0.state.isLive }
        moving = Array(lanes.prefix(Self.laneLimit))
        movingMore = max(0, lanes.count - Self.laneLimit)
        let landed = plan.landedThisWeek(in: theme)
        self.landed = landed.isEmpty ? nil : PlanWords.lanesLine(landed)
        let standing = plan.rulings(in: theme).filter(\.isStanding)
        decided = Array(standing.prefix(Self.rulingLimit))
        decidedMore = max(0, standing.count - Self.rulingLimit)
        let budget = PlanWords.budget(theme.spend, against: theme.budgetTokens)
        self.budget = budget?.isOver == false ? budget : nil
    }
}
