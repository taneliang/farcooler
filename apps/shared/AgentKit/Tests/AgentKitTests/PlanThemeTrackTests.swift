import Foundation
import Testing

@testable import AgentKit

/// What a theme's week holds (ov-331): its rulings, what landed, the work outside every theme. The
/// track line's phrases and rules are the core's, tested in `crates/client` against
/// `test/fixtures/plan-track-cases.json` (ov-1038).
struct PlanThemeTrackTests {
    static let now = PlanModelTests.now
    static let hour = PlanModelTests.hour
    static let day = 24 * hour

    /// One active theme "T" holding cards c1 and c2 (more with `cards`; one
    /// done unless `done` says otherwise), with `lines`, `extra` merged over
    /// the plan's keys and `edit` applied to the theme.
    static func plan(
        lines: [[String: Any]] = [], cards: [String] = ["c1", "c2"], done: Int = 0, open: Int = 1, state: String = "active",
        storyAt: Int64 = 0, extra: [String: Any] = [:], edit: (inout [String: Any]) -> Void = { _ in }
    ) throws -> PlanModel {
        var theme = PlanModelTests.theme("T", cards: cards, ordinal: 0, state: state)
        theme["counts"] = Self.counts(todo: open, done: done)
        theme["story_at"] = storyAt
        edit(&theme)
        var object: [String: Any] = ["now_ms": now, "themes": [theme], "lines": lines, "cards": []]
        for (key, value) in extra { object[key] = value }
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    static func counts(todo: Int = 0, done: Int = 0, cancelled: Int = 0) -> [String: Any] {
        ["backlog": 0, "todo": todo, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": done,
         "cancelled": cancelled]
    }

    /// A ticket on `card`, called `title` (its heading, which the track line names).
    static func line(_ title: String, _ stage: String, rank: Int? = nil, since: Int64 = now - 10 * 60_000,
                     fixRounds: Int = 0, stale: Bool = false, fixing: Bool = false, card: String = "c1") -> [String: Any] {
        PlanModelTests.line(
            card, stage, rank: rank, since: since, fixRounds: fixRounds, stale: stale, fixing: fixing, title: title)
    }

    @Test("A runner's last_moved_at decodes; one without it leaves nil")
    func lastMovedDecodes() throws {
        let with = try Self.plan { $0["last_moved_at"] = 42 }
        #expect(with.themes[0].lastMovedAt == 42)
        #expect(try Self.plan().themes[0].lastMovedAt == nil)
        // The runner's own bytes (`test/fixtures/plan.json`, which the CLI's test writes).
        let fixture = try PlanModelTests.fixture()
        #expect(fixture.themes[0].lastMovedAt == Self.now - Self.hour)
    }

    @Test("Landed this week: the theme's landed tickets inside seven days, newest first")
    func landedThisWeek() throws {
        let plan = try Self.plan(lines: [
            Self.line("old", "landed", since: Self.now - 8 * Self.day, card: "c1"),
            Self.line("new", "landed", since: Self.now - Self.hour, card: "c2"),
            Self.line("mid", "landed", since: Self.now - 2 * Self.day, card: "c3"), Self.line("live", "building", card: "c4"),
        ], cards: ["c1", "c2", "c3", "c4"])
        #expect(plan.landedThisWeek(in: plan.themes[0]).map(\.heading) == ["new", "mid"])
        #expect(PlanWords.ticketsLine(plan.lines) == "old, new, mid, +1 more")
        #expect(PlanWords.ticketsLine(Array(plan.lines.prefix(2))) == "old, new")
    }

    @Test("Outside any theme: tickets with no card in a theme, open cards in none, and the CLI's tidy list")
    func outside() throws {
        let plan = try Self.plan(
            lines: [Self.line("in", "building"), Self.line("out", "building", card: "z1"),
                    Self.line("gone", "dropped", card: "z2"), Self.line("oldout", "landed", since: Self.now - 9 * Self.day, card: "z3"),
                    Self.line("newout", "landed", since: Self.now - Self.day, card: "z4")],
            extra: ["off_the_line": [["task": "z1", "key": "ov-9", "status": "in_review"]],
                    "landed_not_closed": [["task": "z4", "key": "ov-10", "status": "in_progress"]]])
        let outside = plan.outsideThemes(statuses: ["c1": .todo, "z1": .todo, "z2": .backlog, "z3": .done, "z5": .cancelled])
        #expect(outside.tickets == ["out", "newout"])
        #expect(outside.openCards == 2, "z1 and z2: c1 is in a theme, z3 is done, z5 canceled")
        #expect(outside.tidy.map(\.key) == ["ov-9", "ov-10"])
        #expect(PlanWords.outside(outside) == "Outside any theme: 2 tickets this week \u{00B7} 2 open cards")
        #expect(PlanWords.tidy(11) == "11 cards to tidy" && PlanWords.tidy(1) == "1 card to tidy")
        #expect(PlanWords.outside(PlanOutside(tickets: [], openCards: 0, tidy: [])) == nil)
        #expect(PlanWords.outside(PlanOutside(tickets: ["a"], openCards: 1, tidy: [])) == "Outside any theme: 1 ticket this week \u{00B7} 1 open card")
    }

    @Test("An older runner sends no flags, and the plan reads with none")
    func flagsAreOptional() throws {
        let plan = try Self.plan()
        #expect(plan.offTheLine.isEmpty && plan.landedNotClosed.isEmpty)
    }

    @Test("The story's age is printed only when there's a story")
    func storyAge() throws {
        let plan = try Self.plan(storyAt: Self.now - 3 * Self.hour) { $0["story"] = "Going well." }
        #expect(PlanWords.storyAge(plan.themes[0], now: Self.now) == "Updated 3 h ago")
        #expect(PlanWords.storyAge(try Self.plan(storyAt: Self.now - Self.hour).themes[0], now: Self.now) == nil)
    }

    @Test("Themes split into active ones in the board's order and one closed fold of paused and done; dropped are in neither")
    func themeGroups() throws {
        let themes = [
            ("Zed", "active", 2), ("Beta", "paused", 0), ("Alpha", "active", 1), ("Gone", "dropped", 3), ("Fin", "done", 4),
        ].map { name, state, ordinal in PlanModelTests.theme(name, cards: [], ordinal: ordinal, state: state) }
        let plan = try PlanModelTests.plan(themes: themes, lines: [])
        let groups = plan.themeGroups
        #expect(groups.active.map(\.name) == ["Alpha", "Zed"], "the board's order, not alphabetical")
        #expect(groups.closed.map(\.name) == ["Beta", "Fin"], "paused, then done")
        #expect(!(groups.active + groups.closed).contains { $0.name == "Gone" })
    }
}
