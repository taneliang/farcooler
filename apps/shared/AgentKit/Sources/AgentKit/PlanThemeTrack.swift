import Foundation

// The track line (ov-331): one plain-words line per theme that says whether
// the work is moving, for the owner who wants to stay on track. It is computed
// from facts the plan already carries, never from a stored health flag: an
// agent's "at risk" is a guess the owner can't check, and "No lane · quiet for
// 3 days" is something they can.
//
// Amber is for what the owner has to act on, and a track line is only ever
// amber for a theme gone over its token budget. An ask is not a track state:
// it has its own line, in amber, so saying it twice would be noise. Being quiet
// is said in words and never colored, badged or counted in a title bar.
//
// The design is `.claude/agent/reports/theme-detail/design.md`, 2.2.

/// How long a theme sits with no lane and nothing moving before it reads as
/// quiet (the owner's ruling D3, 5 Oct): a day.
public let planQuietAfterMs: Int64 = 24 * 3_600_000

/// Where a theme stands in motion: the first rule that matches.
public enum PlanTrack: Equatable, Sendable {
    /// Past its token budget.
    case overBudget(PlanBudget)
    /// Every lane working it has sat in one state for over an hour: the one
    /// that has sat longest is named.
    case stuck(lane: String, since: Int64)
    /// At least one lane is building, in review, fixing or landing it, and
    /// is still moving. A lane that has sat over an hour beside them is
    /// named after: the others are moving, and the line says so first.
    case moving(lanes: [PlanTrackLane], stalled: PlanTrackStalled?)
    /// No lane is working it and the plan has one queued for it.
    case queued(rank: Int?)
    /// Open cards, no lane, and nothing moved for a day or more.
    case quiet(since: Int64)
    /// Open cards, no lane, and something moved within the day.
    case idle
    /// Every card is done.
    case allDone
    /// The theme's state says so; shown only inside the fold.
    case paused
    case done

    /// Amber, and only this: a budget gone over.
    public var needsAttention: Bool {
        if case .overBudget = self { return true }
        return false
    }

    /// Being worked on right now, stuck or not: what "moving" counts.
    public var isMoving: Bool {
        switch self {
        case .moving, .stuck: true
        default: false
        }
    }

    public var isQuiet: Bool {
        if case .quiet = self { return true }
        return false
    }

    /// The glyph beside the words: a shape to scan by, so the state never
    /// rests on color alone. SF Symbols on the Mac and the iPhone.
    public var symbol: String {
        switch self {
        case .overBudget: "exclamationmark.circle.fill"
        case .stuck: "exclamationmark.circle"
        case .moving: "arrow.triangle.2.circlepath"
        case .queued: "clock"
        case .quiet: "moon.zzz"
        case .idle: "circle.dashed"
        case .allDone, .done: "checkmark.circle"
        case .paused: "pause.circle"
        }
    }
}

/// The working lane that has sat longest in one state, named beside lanes
/// that are moving.
public struct PlanTrackStalled: Equatable, Sendable {
    public var lane: String
    public var since: Int64
}

/// A lane moving a theme, as the track line names it.
public struct PlanTrackLane: Equatable, Sendable {
    public var name: String
    public var state: LaneState
    public var fixRounds: Int
}

extension PlanModel {
    /// The lanes working any of `theme`'s cards right now: building, in
    /// review, fixing or landing. Queued lanes are waiting, not working.
    public func workingLanes(in theme: PlanTheme) -> [PlanLane] {
        lanes(in: theme).filter { $0.state.isLive && $0.state != .queued }
    }

    /// When `theme` last moved, in ms: the newest of its story, the lanes on
    /// its cards (a dropped lane is gone, and doesn't count), its rulings
    /// (made or settled), the board's own word on its cards (`activity`, by
    /// task id) and the runner's, if it sent one. 0 for a theme nothing has
    /// ever touched.
    public func lastMoved(_ theme: PlanTheme, activity: [String: Int64] = [:]) -> Int64 {
        var at = max(theme.storyAt, theme.lastMovedAt ?? 0)
        let tasks = Set(theme.cards.map(\.task))
        for task in tasks { at = max(at, activity[task] ?? 0) }
        for lane in lanes where lane.state != .dropped && lane.cards.contains(where: { tasks.contains($0.task) }) {
            at = max(at, lane.stateSince)
        }
        for ruling in rulings(in: theme) { at = max(at, ruling.createdAt, ruling.settledAt ?? 0) }
        return at
    }

    /// Where `theme` stands in motion, as of the runner's clock.
    public func track(of theme: PlanTheme, activity: [String: Int64] = [:]) -> PlanTrack {
        if theme.state == "paused" { return .paused }
        if theme.state == "done" { return .done }
        if let over = PlanWords.overBudget(theme) { return .overBudget(over) }
        let working = workingLanes(in: theme)
        // The lane stalled longest, not the first in plan order; and it only
        // stands alone when nothing else is moving (an hour is routine for a
        // build, so it never hides the lanes that are).
        let stalled = working.filter(\.stale).min { $0.stateSince < $1.stateSince }
        if let stalled, working.allSatisfy(\.stale) { return .stuck(lane: stalled.name, since: stalled.stateSince) }
        if !working.isEmpty {
            return .moving(
                lanes: working.map { PlanTrackLane(name: $0.name, state: $0.state, fixRounds: $0.fixRounds) },
                stalled: stalled.map { PlanTrackStalled(lane: $0.name, since: $0.stateSince) })
        }
        let queued = lanes(in: theme).filter { $0.state == .queued }
        if !queued.isEmpty {
            return .queued(rank: queued.compactMap(\.planRank).min())
        }
        let total = PlanWords.total(theme.counts)
        if total > 0, theme.counts.done == total { return .allDone }
        let moved = lastMoved(theme, activity: activity)
        if total > theme.counts.done, moved > 0, nowMs - moved >= planQuietAfterMs { return .quiet(since: moved) }
        return .idle
    }

    /// The standing and settled rulings that name `theme`, in the plan's order.
    public func rulings(in theme: PlanTheme) -> [PlanRuling] {
        rulings.filter { $0.themeId == theme.id }
    }

    /// The lanes that landed on `theme`'s cards in the last seven days,
    /// newest first.
    public func landedThisWeek(in theme: PlanTheme) -> [PlanLane] {
        lanes(in: theme)
            .filter { $0.state == .landed && nowMs - $0.stateSince < 7 * 24 * 3_600_000 }
            .sorted { $0.stateSince > $1.stateSince }
    }

    /// What the week did outside every theme: the lanes (live, or landed in
    /// the last week) none of whose cards is in one, and the open cards in no
    /// theme, from the board's statuses by task id.
    public func outsideThemes(statuses: [String: TaskStatus]) -> PlanOutside {
        let inThemes = Set(shownThemes.flatMap { $0.cards.map(\.task) })
        let outsideLanes = lanes.filter { lane in
            lane.state != .dropped && !lane.cards.contains { inThemes.contains($0.task) }
                && (lane.state.isLive || nowMs - lane.stateSince < 7 * 24 * 3_600_000)
        }
        let open = statuses.filter { id, status in
            !inThemes.contains(id) && status != .done && status != .cancelled
        }.count
        return PlanOutside(lanes: outsideLanes.map(\.name), openCards: open, tidy: noLane + landedNotClosed)
    }

    /// "3 waiting on you · 4 moving · 1 quiet": the Themes section's one
    /// line when it's folded, or nil when none of it applies.
    public func trackSummary(activity: [String: Int64] = [:]) -> String? {
        let shown = shownThemes
        // Only active themes: a paused theme's ask is inside the closed fold,
        // and Needs You lists it.
        let active = shown.filter { $0.state == "active" }
        let asking = active.filter { !$0.ownerAsk.isEmpty }.count
        let tracks = active.map { track(of: $0, activity: activity) }
        let parts = [
            (asking, "waiting on you"), (tracks.filter(\.isMoving).count, "moving"),
            (tracks.filter(\.isQuiet).count, "quiet"),
        ].filter { $0.0 > 0 }.map { "\($0.0) \($0.1)" }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

/// The work outside every theme.
public struct PlanOutside: Equatable, Sendable {
    /// The lanes' names, in the plan's order.
    public var lanes: [String]
    public var openCards: Int
    /// The cards the CLI's "Worth a look" lists: in review with no lane
    /// working them, or whose lanes all landed.
    public var tidy: [PlanFlaggedCard]

    public var isEmpty: Bool { lanes.isEmpty && openCards == 0 && tidy.isEmpty }
}

extension PlanWords {
    /// "2 lanes moving", or "fix-gestures is fixing, round 1": a lane named
    /// when one is the whole story.
    public static func track(_ track: PlanTrack, now: Int64) -> String {
        switch track {
        case .overBudget(let budget):
            return budgetLine(budget)
        case .stuck(let lane, let since):
            return "\(lane): \(noMove(since: since, now: now))"
        case .moving(let lanes, let stalled):
            let tail = stalled.map { " · \($0.lane) \(noMove(since: $0.since, now: now))" } ?? ""
            guard lanes.count == 1, let lane = lanes.first else { return "\(lanes.count) lanes moving" + tail }
            let verb =
                switch lane.state {
                case .building: "building"
                case .review: "in review"
                case .fixing: lane.fixRounds > 0 ? "fixing, round \(lane.fixRounds)" : "fixing"
                case .landing: "landing"
                default: "moving"
                }
            return "\(lane.name) is \(verb)" + tail
        case .queued(let rank):
            return rank.map { "Queued, \(ordinal($0)) up" } ?? "Queued"
        case .quiet(let since):
            let days = max(1, max(0, now - since) / 86_400_000)
            return days == 1 ? "No lane · quiet for 1 day" : "No lane · quiet for \(days) days"
        case .idle: return "No lane yet"
        case .allDone: return "Every card is done"
        case .paused: return "Paused"
        case .done: return "Done"
        }
    }

    /// "no move in an hour", "no move in 3 h".
    static func noMove(since: Int64, now: Int64) -> String {
        let minutes = max(0, now - since) / 60_000
        return minutes < 120 ? "no move in an hour" : "no move in \(minutes / 60) h"
    }

    /// The same for VoiceOver and TalkBack: a middle dot isn't read well.
    public static func trackSpoken(_ track: PlanTrack, now: Int64) -> String {
        switch track {
        case .overBudget(let budget): budgetSpoken(budget)
        default: Self.track(track, now: now).replacingOccurrences(of: " · ", with: ", ")
        }
    }

    /// "Updated 1 h ago": the story's age, printed and never judged.
    public static func storyAge(_ theme: PlanTheme, now: Int64) -> String? {
        theme.storyAt > 0 && !theme.story.isEmpty ? "Updated \(ago(theme.storyAt, now: now))" : nil
    }

    /// "Landed this week": lane names, newest first, "+3 more" past `limit`.
    public static func lanesLine(_ lanes: [PlanLane], limit: Int = 3) -> String {
        let names = lanes.prefix(limit).map(\.name).joined(separator: ", ")
        return lanes.count > limit ? "\(names), +\(lanes.count - limit) more" : names
    }

    /// "Outside any theme: 2 lanes this week · 11 open cards"; nil when empty.
    /// The lanes are the week's, and the open cards are every one, however old.
    public static func outside(_ outside: PlanOutside) -> String? {
        var parts: [String] = []
        if !outside.lanes.isEmpty {
            parts.append((outside.lanes.count == 1 ? "1 lane" : "\(outside.lanes.count) lanes") + " this week")
        }
        if outside.openCards > 0 { parts.append(outside.openCards == 1 ? "1 open card" : "\(outside.openCards) open cards") }
        return parts.isEmpty ? nil : "Outside any theme: " + parts.joined(separator: " · ")
    }

    /// "11 cards to tidy".
    public static func tidy(_ n: Int) -> String { n == 1 ? "1 card to tidy" : "\(n) cards to tidy" }
}
