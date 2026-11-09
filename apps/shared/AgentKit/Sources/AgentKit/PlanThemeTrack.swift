import Foundation

// What a theme's week holds (ov-331): its rulings, what landed, the work outside every theme.
// The track line is gone from the apps (ov-1038): nothing drew it, and the folded Themes
// section's summary is the core's (`PlanView.pages.trackSummary`).

extension PlanModel {
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
