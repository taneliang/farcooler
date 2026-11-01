import Foundation

// The track line (ov-331): one plain-words line per theme that says whether
// the work is moving, for the owner who wants to stay on track. It is computed
// from facts the plan already carries, never from a stored health flag: an
// agent's "at risk" is a guess the owner can't check, and "No ticket · quiet for
// 3 days" is something they can.
//
// Amber is for what the owner has to act on, and a track line is only ever
// amber for a theme gone over its token budget. An ask is not a track state:
// it has its own line, in amber, so saying it twice would be noise. Being quiet
// is said in words and never colored, badged or counted in a title bar.
//
// The design is `.claude/agent/reports/theme-detail/design.md`, 2.2.

/// How long a theme sits with no ticket and nothing moving before it reads as
/// quiet (the owner's ruling D3, 5 Oct): a day.
public let planQuietAfterMs: Int64 = 24 * 3_600_000

/// Where a theme stands in motion: the first rule that matches.
public enum PlanTrack: Equatable, Sendable {
    /// Past its token budget.
    case overBudget(PlanBudget)
    /// Every ticket working it has sat in one stage for over an hour: the one
    /// that has sat longest is named.
    case stuck(ticket: String, since: Int64)
    /// At least one ticket is being built, reviewed, fixed or landed for it,
    /// and is still moving. A ticket that has sat over an hour beside them is
    /// named after: the others are moving, and the line says so first.
    case moving(tickets: [PlanTrackTicket], stalled: PlanTrackStalled?)
    /// Nothing is working it and the plan has a ticket waiting for it.
    case queued(rank: Int?)
    /// Open cards, no ticket in flight, and nothing moved for a day or more.
    case quiet(since: Int64)
    /// Open cards, no ticket in flight, and something moved within the day.
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

/// The working ticket that has sat longest in one stage, named beside tickets
/// that are moving.
public struct PlanTrackStalled: Equatable, Sendable {
    public var ticket: String
    public var since: Int64
}

/// A ticket moving a theme, as the track line names it.
public struct PlanTrackTicket: Equatable, Sendable {
    public var name: String
    public var stage: TicketStage
    /// A builder has it again for a fix round.
    public var fixing: Bool
    public var fixRounds: Int
}

extension PlanModel {
    /// The tickets for `theme`'s cards being worked right now: building, in
    /// review, waiting on the owner or landing. Waiting tickets are waiting,
    /// not working.
    public func workingLines(in theme: PlanTheme) -> [PlanTicketLine] {
        lines(in: theme).filter { $0.stage.isPastWaiting }
    }

    /// When `theme` last moved, in ms: the newest of its story, the tickets on
    /// its cards (a dropped ticket is gone, and doesn't count), its rulings
    /// (made or settled), the board's own word on its cards (`activity`, by
    /// task id) and the runner's, if it sent one. 0 for a theme nothing has
    /// ever touched.
    public func lastMoved(_ theme: PlanTheme, activity: [String: Int64] = [:]) -> Int64 {
        var at = max(theme.storyAt, theme.lastMovedAt ?? 0)
        let tasks = Set(theme.cards.map(\.task))
        for task in tasks { at = max(at, activity[task] ?? 0) }
        for line in lines where line.stage != .dropped && tasks.contains(line.task) {
            at = max(at, line.stageSince)
        }
        for ruling in rulings(in: theme) { at = max(at, ruling.createdAt, ruling.settledAt ?? 0) }
        return at
    }

    /// Where `theme` stands in motion, as of the runner's clock.
    public func track(of theme: PlanTheme, activity: [String: Int64] = [:]) -> PlanTrack {
        if theme.state == "paused" { return .paused }
        if theme.state == "done" { return .done }
        if let over = PlanWords.overBudget(theme) { return .overBudget(over) }
        let working = workingLines(in: theme)
        // The ticket stalled longest, not the first in plan order; and it only
        // stands alone when nothing else is moving (an hour is routine for a
        // build, so it never hides the tickets that are).
        let stalled = working.filter(\.stale).min { $0.stageSince < $1.stageSince }
        if let stalled, working.allSatisfy(\.stale) { return .stuck(ticket: stalled.heading, since: stalled.stageSince) }
        if !working.isEmpty {
            return .moving(
                tickets: working.map {
                    PlanTrackTicket(name: $0.heading, stage: $0.stage, fixing: $0.isFixing, fixRounds: $0.fixRounds)
                },
                stalled: stalled.map { PlanTrackStalled(ticket: $0.heading, since: $0.stageSince) })
        }
        let queued = lines(in: theme).filter { $0.stage == .waiting }
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

    /// The tickets that landed on `theme`'s cards in the last seven days,
    /// newest first.
    public func landedThisWeek(in theme: PlanTheme) -> [PlanTicketLine] {
        lines(in: theme)
            .filter { $0.stage == .landed && nowMs - $0.stageSince < 7 * 24 * 3_600_000 }
            .sorted { $0.stageSince > $1.stageSince }
    }

    /// What the week did outside every theme: the tickets (live, or landed in
    /// the last week) whose cards are in no theme, and the open cards in no
    /// theme, from the board's statuses by task id.
    public func outsideThemes(statuses: [String: TaskStatus]) -> PlanOutside {
        let inThemes = Set(shownThemes.flatMap { $0.cards.map(\.task) })
        let outsideLines = lines.filter { line in
            line.stage != .dropped && !inThemes.contains(line.task)
                && (line.stage.isLive || nowMs - line.stageSince < 7 * 24 * 3_600_000)
        }
        let open = statuses.filter { id, status in
            !inThemes.contains(id) && status != .done && status != .cancelled
        }.count
        return PlanOutside(tickets: outsideLines.map(\.heading), openCards: open, tidy: offTheLine + landedNotClosed)
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

/// The themes as a list draws them: the active ones in the board's order, and
/// paused and done ones folded into one closed row after them. Dropped themes
/// are in neither.
public struct PlanThemeGroups: Equatable, Sendable {
    public var active: [PlanTheme]
    public var closed: [PlanTheme]
}

extension PlanModel {
    /// The Mac's Themes, the iPhone's and Android's all split on this one rule.
    public var themeGroups: PlanThemeGroups {
        let shown = shownThemes
        return PlanThemeGroups(active: shown.filter { $0.state == "active" }, closed: shown.filter { $0.state != "active" })
    }
}

/// The work outside every theme.
public struct PlanOutside: Equatable, Sendable {
    /// The tickets' headings, in the line's order.
    public var tickets: [String]
    public var openCards: Int
    /// The cards the CLI's "Worth a look" lists: in progress or in review and
    /// off the line, or whose ticket landed.
    public var tidy: [PlanFlaggedCard]

    public var isEmpty: Bool { tickets.isEmpty && openCards == 0 && tidy.isEmpty }
    /// Everything "to tidy".
    public var tidyCount: Int { tidy.count }
}

extension PlanWords {
    /// "2 tickets moving", or "Fix gestures is fixing, round 1": a ticket
    /// named when one is the whole story.
    public static func track(_ track: PlanTrack, now: Int64) -> String {
        switch track {
        case .overBudget(let budget):
            return budgetLine(budget)
        case .stuck(let ticket, let since):
            return "\(ticket): \(noMove(since: since, now: now))"
        case .moving(let tickets, let stalled):
            let tail = stalled.map { " · \($0.ticket): \(noMove(since: $0.since, now: now))" } ?? ""
            guard tickets.count == 1, let ticket = tickets.first else { return "\(tickets.count) tickets moving" + tail }
            let verb =
                switch ticket.stage {
                case .building: ticket.fixing ? (ticket.fixRounds > 0 ? "fixing, round \(ticket.fixRounds)" : "fixing") : "building"
                case .reviewing: "in review"
                case .needsYou: "waiting on you"
                case .landing: "landing"
                default: "moving"
                }
            return "\(ticket.name) is \(verb)" + tail
        case .queued(let rank):
            return rank.map { "Queued, \(ordinal($0)) up" } ?? "Queued"
        case .quiet(let since):
            let days = max(1, max(0, now - since) / 86_400_000)
            return days == 1 ? "No ticket · quiet for 1 day" : "No ticket · quiet for \(days) days"
        case .idle: return "No ticket yet"
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

    /// "Landed this week": ticket titles, newest first, "+3 more" past `limit`.
    public static func ticketsLine(_ tickets: [PlanTicketLine], limit: Int = 3) -> String {
        let names = tickets.prefix(limit).map(\.heading).joined(separator: ", ")
        return tickets.count > limit ? "\(names), +\(tickets.count - limit) more" : names
    }

    /// "Outside any theme: 2 tickets this week · 11 open cards"; nil when empty.
    /// The tickets are the week's, and the open cards are every one, however old.
    public static func outside(_ outside: PlanOutside) -> String? {
        var parts: [String] = []
        if !outside.tickets.isEmpty {
            parts.append((outside.tickets.count == 1 ? "1 ticket" : "\(outside.tickets.count) tickets") + " this week")
        }
        if outside.openCards > 0 { parts.append(outside.openCards == 1 ? "1 open card" : "\(outside.openCards) open cards") }
        return parts.isEmpty ? nil : "Outside any theme: " + parts.joined(separator: " · ")
    }

    /// "11 cards to tidy".
    public static func tidy(_ n: Int) -> String { n == 1 ? "1 card to tidy" : "\(n) cards to tidy" }

    /// "11 cards to tidy": the cards "Worth a look" names.
    public static func tidy(_ outside: PlanOutside) -> String { tidy(outside.tidy.count) }
}
