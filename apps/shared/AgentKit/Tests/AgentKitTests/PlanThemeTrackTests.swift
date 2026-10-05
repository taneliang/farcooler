import Foundation
import Testing

@testable import AgentKit

/// The track line and the rules around it (ov-331): every phrase, which rule
/// wins when two match, and what "last moved" counts.
struct PlanThemeTrackTests {
    static let now = PlanModelTests.now
    static let hour = PlanModelTests.hour
    static let day = 24 * hour

    /// One active theme "T" holding card c1 and c2 (one done unless `done`
    /// says otherwise), with `lanes`, `extra` merged over the plan's keys and
    /// `edit` applied to the theme.
    static func plan(
        lanes: [[String: Any]] = [], done: Int = 0, open: Int = 1, state: String = "active", storyAt: Int64 = 0,
        extra: [String: Any] = [:], edit: (inout [String: Any]) -> Void = { _ in }
    ) throws -> PlanModel {
        var theme = PlanModelTests.theme("T", cards: ["c1", "c2"], ordinal: 0, state: state)
        theme["counts"] = Self.counts(todo: open, done: done)
        theme["story_at"] = storyAt
        edit(&theme)
        var object: [String: Any] = [
            "now_ms": now, "themes": [theme], "lanes": lanes, "order": lanes.compactMap { $0["id"] },
            "cards": [],
        ]
        for (key, value) in extra { object[key] = value }
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    static func counts(todo: Int = 0, done: Int = 0, cancelled: Int = 0) -> [String: Any] {
        ["backlog": 0, "todo": todo, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": done,
         "cancelled": cancelled]
    }

    static func lane(_ name: String, _ state: String, rank: Int? = nil, since: Int64 = now - 10 * 60_000,
                     fixRounds: Int = 0, stale: Bool = false, card: String = "c1") -> [String: Any] {
        PlanModelTests.lane(name, state, cards: [card], rank: rank, since: since, fixRounds: fixRounds, stale: stale)
    }

    static func track(_ plan: PlanModel, activity: [String: Int64] = [:]) -> PlanTrack {
        plan.track(of: plan.themes[0], activity: activity)
    }

    static func words(_ plan: PlanModel, activity: [String: Int64] = [:]) -> String {
        PlanWords.track(track(plan, activity: activity), now: now)
    }

    // MARK: Phrases

    @Test("Moving: one lane is named with its state, several are counted")
    func moving() throws {
        #expect(Self.words(try Self.plan(lanes: [Self.lane("skill-323", "building")])) == "skill-323 is building")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "review")])) == "a is in review")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("fix-gestures", "fixing", fixRounds: 1)]))
            == "fix-gestures is fixing, round 1")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "fixing")])) == "a is fixing")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "landing")])) == "a is landing")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "building"), Self.lane("b", "review")]))
            == "2 lanes moving")
    }

    @Test("Stuck: a stale lane is named with how long, as an hour or hours")
    func stuck() throws {
        let hourAndABit = try Self.plan(lanes: [Self.lane("skill-323", "building", since: Self.now - 70 * 60_000, stale: true)])
        #expect(Self.words(hourAndABit) == "skill-323: no move in an hour")
        let three = try Self.plan(lanes: [Self.lane("skill-323", "building", since: Self.now - 3 * Self.hour, stale: true)])
        #expect(Self.words(three) == "skill-323: no move in 3 h")
    }

    @Test("A stale lane among moving ones is named after them, and never hides them")
    func staleBesideMoving() throws {
        let plan = try Self.plan(lanes: [
            Self.lane("fine", "building"), Self.lane("slow", "review", since: Self.now - 4 * Self.hour, stale: true),
        ])
        #expect(Self.words(plan) == "2 lanes moving · slow no move in 4 h")
        #expect(Self.track(plan).isMoving)
    }

    @Test("Queued: the best rank among its queued lanes, or none said")
    func queued() throws {
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "queued", rank: 1)])) == "Queued, 1st up")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "queued", rank: 3), Self.lane("b", "queued", rank: 2)]))
            == "Queued, 2nd up")
        #expect(Self.words(try Self.plan(lanes: [Self.lane("a", "queued")])) == "Queued")
    }

    @Test("A working lane beats a queued one")
    func movingBeatsQueued() throws {
        let plan = try Self.plan(lanes: [Self.lane("a", "queued", rank: 1, card: "c2"), Self.lane("b", "building")])
        #expect(Self.words(plan) == "b is building")
    }

    @Test("Quiet: open cards, no lane, and a day with nothing, said in days")
    func quiet() throws {
        let storyAt = Self.now - 3 * Self.day - Self.hour
        #expect(Self.words(try Self.plan(storyAt: storyAt)) == "No lane · quiet for 3 days")
        #expect(Self.words(try Self.plan(storyAt: Self.now - 25 * Self.hour)) == "No lane · quiet for 1 day")
        #expect(Self.track(try Self.plan(storyAt: Self.now - Self.day)).isQuiet, "exactly 24 h is quiet")
    }

    @Test("Idle: open cards and no lane, but something moved in the last day")
    func idle() throws {
        #expect(Self.words(try Self.plan(storyAt: Self.now - 23 * Self.hour)) == "No lane yet")
        #expect(!Self.track(try Self.plan(storyAt: Self.now - 23 * Self.hour)).isQuiet)
    }

    @Test("A theme nothing has ever touched is idle, not quiet for fifty years")
    func untouched() throws {
        #expect(Self.words(try Self.plan(storyAt: 0)) == "No lane yet")
    }

    @Test("A card the board moved lately keeps a theme from being quiet; the plan alone can't see it")
    func activityKeepsItAwake() throws {
        let old = Self.now - 5 * Self.day
        let plan = try Self.plan(storyAt: old)
        #expect(Self.track(plan).isQuiet)
        #expect(Self.track(plan, activity: ["c2": Self.now - Self.hour]) == .idle)
    }

    @Test("A landed lane is not moving it, and its landing counts as a move")
    func landedIsNotMoving() throws {
        let plan = try Self.plan(lanes: [Self.lane("a", "landed", since: Self.now - 2 * Self.hour)], storyAt: Self.now - 9 * Self.day)
        #expect(Self.track(plan) == .idle)
    }

    @Test("A dropped lane's time doesn't count as the theme moving")
    func droppedIsGone() throws {
        let plan = try Self.plan(lanes: [Self.lane("a", "dropped", since: Self.now - Self.hour)], storyAt: Self.now - 9 * Self.day)
        #expect(Self.track(plan).isQuiet)
    }

    @Test("All done: every counted card done; canceled cards count on neither side")
    func allDone() throws {
        #expect(Self.words(try Self.plan(done: 2, open: 0)) == "Every card is done")
        #expect(Self.words(try Self.plan(done: 2, open: 0, edit: { $0["counts"] = Self.counts(done: 2, cancelled: 3) }))
            == "Every card is done")
    }

    @Test("A theme with no cards says no lane yet, not that everything is done")
    func empty() throws {
        #expect(Self.words(try Self.plan(done: 0, open: 0)) == "No lane yet")
    }

    @Test("Paused and done say so, whatever else is true of them")
    func pausedAndDone() throws {
        let busy = [Self.lane("a", "building")]
        #expect(Self.words(try Self.plan(lanes: busy, state: "paused")) == "Paused")
        #expect(Self.words(try Self.plan(lanes: busy, state: "done")) == "Done")
    }

    @Test("Over budget is the one amber state, and wins over a moving lane")
    func overBudget() throws {
        let plan = try Self.plan(lanes: [Self.lane("a", "building")]) { theme in
            theme["budget_tokens"] = 100
            theme["spend"] = ["input_tokens": 600, "runs": 1, "unmeasured_agents": 0]
        }
        let track = Self.track(plan)
        #expect(track.needsAttention)
        #expect(PlanWords.track(track, now: Self.now) == "Over budget: 600 of 100 tokens")
        #expect(!Self.track(try Self.plan(lanes: [Self.lane("a", "building")])).needsAttention)
        for other in [PlanTrack.paused, .done, .idle, .allDone, .queued(rank: 1), .quiet(since: 1)] {
            #expect(!other.needsAttention, "\(other) is never amber")
        }
    }

    @Test("Every state has its own glyph, so color never carries it alone")
    func glyphs() {
        let tracks: [PlanTrack] = [
            .overBudget(.over(used: 2, budget: 1)), .stuck(lane: "a", since: 0), .moving(lanes: [], stalled: nil), .queued(rank: nil),
            .quiet(since: 0), .idle, .allDone, .paused,
        ]
        #expect(Set(tracks.map(\.symbol)).count == tracks.count)
    }

    @Test("Spoken, the middle dot is a comma")
    func spoken() throws {
        let quiet = PlanTrack.quiet(since: Self.now - 3 * Self.day)
        #expect(PlanWords.trackSpoken(quiet, now: Self.now) == "No lane, quiet for 3 days")
    }

    // MARK: Last moved

    @Test("Last moved is the newest of the story, the lanes, the rulings, the cards and the runner's word")
    func lastMovedCountsEverything() throws {
        let base = Self.now - 10 * Self.day
        var plan = try Self.plan(storyAt: base)
        #expect(plan.lastMoved(plan.themes[0]) == base)
        #expect(plan.lastMoved(plan.themes[0], activity: ["c2": base + 5]) == base + 5)
        #expect(plan.lastMoved(plan.themes[0], activity: ["other": base + 99]) == base, "a card in another theme isn't ours")

        plan = try Self.plan(lanes: [Self.lane("a", "landed", since: base + 7)], storyAt: base)
        #expect(plan.lastMoved(plan.themes[0]) == base + 7)

        plan = try Self.plan(storyAt: base) { $0["last_moved_at"] = base + 11 }
        #expect(plan.lastMoved(plan.themes[0]) == base + 11, "the runner's own word counts")

        let ruling: [String: Any] = [
            "id": "r1", "short": "R-1", "number": 1, "decision": "d", "why": "w", "reversal": "r", "cards": [],
            "theme_id": "theme-T", "theme": "T", "state": "confirmed", "note": "", "actor": "manager",
            "created_at": base + 20, "settled_at": base + 30,
        ]
        plan = try Self.plan(storyAt: base, extra: ["rulings": [ruling]])
        #expect(plan.lastMoved(plan.themes[0]) == base + 30, "settling a ruling is a move")
        #expect(plan.rulings(in: plan.themes[0]).map(\.short) == ["R-1"])
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

    // MARK: Around the themes

    @Test("Landed this week: the theme's landed lanes inside seven days, newest first")
    func landedThisWeek() throws {
        let plan = try Self.plan(lanes: [
            Self.lane("old", "landed", since: Self.now - 8 * Self.day), Self.lane("new", "landed", since: Self.now - Self.hour),
            Self.lane("mid", "landed", since: Self.now - 2 * Self.day), Self.lane("live", "building"),
        ])
        #expect(plan.landedThisWeek(in: plan.themes[0]).map(\.name) == ["new", "mid"])
        #expect(PlanWords.lanesLine(plan.lanes) == "old, new, mid, +1 more")
        #expect(PlanWords.lanesLine(Array(plan.lanes.prefix(2))) == "old, new")
    }

    @Test("Outside any theme: lanes with no card in a theme, open cards in none, and the CLI's tidy list")
    func outside() throws {
        let plan = try Self.plan(
            lanes: [Self.lane("in", "building"), Self.lane("out", "building", card: "z1"),
                    Self.lane("gone", "dropped", card: "z2"), Self.lane("oldout", "landed", since: Self.now - 9 * Self.day, card: "z3"),
                    Self.lane("newout", "landed", since: Self.now - Self.day, card: "z4")],
            extra: ["no_lane": [["task": "z1", "key": "ov-9", "status": "in_review"]],
                    "landed_not_closed": [["task": "z4", "key": "ov-10", "status": "in_progress"]]])
        let outside = plan.outsideThemes(statuses: ["c1": .todo, "z1": .todo, "z2": .backlog, "z3": .done, "z5": .cancelled])
        #expect(outside.lanes == ["out", "newout"])
        #expect(outside.openCards == 2, "z1 and z2: c1 is in a theme, z3 is done, z5 canceled")
        #expect(outside.tidy.map(\.key) == ["ov-9", "ov-10"])
        #expect(PlanWords.outside(outside) == "Outside any theme: 2 lanes this week · 2 open cards")
        #expect(PlanWords.tidy(11) == "11 cards to tidy" && PlanWords.tidy(1) == "1 card to tidy")
        #expect(PlanWords.outside(PlanOutside(lanes: [], openCards: 0, tidy: [])) == nil)
        #expect(PlanWords.outside(PlanOutside(lanes: ["a"], openCards: 1, tidy: [])) == "Outside any theme: 1 lane this week · 1 open card")
    }

    @Test("An older runner sends no flags, and the plan reads with none")
    func flagsAreOptional() throws {
        let plan = try Self.plan()
        #expect(plan.noLane.isEmpty && plan.landedNotClosed.isEmpty)
    }

    @Test("The summary counts asks, moving themes and quiet ones, and leaves out zeros")
    func summary() throws {
        func themes(_ specs: [(String, String, String)]) throws -> PlanModel {
            let themes = specs.enumerated().map { index, spec -> [String: Any] in
                var t = PlanModelTests.theme(spec.0, cards: ["c\(index)"], ordinal: index, state: spec.1)
                t["owner_ask"] = spec.2
                t["counts"] = Self.counts(todo: 1)
                t["story_at"] = Self.now - 5 * Self.day
                return t
            }
            let lanes = [PlanModelTests.lane("w", "building", cards: ["c0"])]
            let object: [String: Any] = ["now_ms": Self.now, "themes": themes, "lanes": lanes, "order": [], "cards": []]
            return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
        }
        let plan = try themes([("A", "active", "?"), ("B", "active", ""), ("C", "active", "?"), ("D", "paused", "")])
        #expect(plan.trackSummary() == "2 waiting on you · 1 moving · 2 quiet")
        let hidden = try themes([("A", "active", ""), ("B", "done", "?"), ("C", "paused", "?")])
        #expect(hidden.trackSummary() == "1 moving", "a done or paused theme's ask is in the fold, and not counted")
        let calm = try themes([("A", "active", ""), ("B", "done", "")])
        #expect(calm.trackSummary() == "1 moving")
        #expect(try Self.plan(done: 2, open: 0).trackSummary() == nil)
    }

    @Test("The story's age is printed only when there's a story")
    func storyAge() throws {
        let plan = try Self.plan(storyAt: Self.now - 3 * Self.hour) { $0["story"] = "Going well." }
        #expect(PlanWords.storyAge(plan.themes[0], now: Self.now) == "Updated 3 h ago")
        #expect(PlanWords.storyAge(try Self.plan(storyAt: Self.now - Self.hour).themes[0], now: Self.now) == nil)
    }

    // MARK: The entry's brief

    @Test("The brief lists three lanes and two rulings, says how many more, and leaves empty lines out")
    func brief() throws {
        func ruling(_ n: Int, state: String = "standing") -> [String: Any] {
            ["id": "r\(n)", "short": "R-\(n)", "number": n, "decision": "d\(n)", "why": "w", "reversal": "r", "cards": [],
             "theme_id": "theme-T", "theme": "T", "state": state, "note": "", "actor": "manager", "created_at": n]
        }
        let lanes = (1...5).map { Self.lane("l\($0)", "building") }
        let plan = try Self.plan(
            lanes: lanes, extra: ["rulings": [ruling(1), ruling(2), ruling(3), ruling(4, state: "confirmed")]]
        ) { $0["story"] = "Going well."; $0["story_at"] = Self.now - Self.hour }
        let brief = PlanThemeBrief(plan.themes[0], in: plan)
        #expect(brief.moving.map(\.name) == ["l1", "l2", "l3"] && brief.movingMore == 2)
        #expect(brief.decided.map(\.short) == ["R-1", "R-2"] && brief.decidedMore == 1, "standing only: R-4 is settled")
        #expect(brief.story == "Going well." && brief.storyAge == "Updated 1 h ago")
        #expect(brief.outcome == nil && brief.ask == nil && brief.next == nil && brief.landed == nil && brief.budget == nil)
        let within = try Self.plan { $0["budget_tokens"] = 1000 }
        #expect(PlanThemeBrief(within.themes[0], in: within).budget != nil)
        let over = try Self.plan { $0["budget_tokens"] = 100; $0["spend"] = ["input_tokens": 600, "runs": 1, "unmeasured_agents": 0] }
        #expect(PlanThemeBrief(over.themes[0], in: over).budget == nil, "over budget is the track line's, not said twice")
    }

    // MARK: Shared cases

    /// The cases `PlanTrackTest` (Kotlin) reads too: a plan, the first theme's
    /// track line and the Themes summary, so a phrase or a threshold can't
    /// change on one platform alone.
    @Test("Every case in test/fixtures/plan-track-cases.json reads as written")
    func sharedCases() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-track-cases.json"))
        let file = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let cases = try #require(file["cases"] as? [[String: Any]])
        #expect(cases.count >= 30)
        for item in cases {
            let name = item["name"] as? String ?? "?"
            let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(item["plan"])))
            if let want = item["track"] as? String {
                #expect(Self.words(plan) == want, "\(name)")
            }
            if item.keys.contains("summary") {
                #expect(plan.trackSummary() == item["summary"] as? String, "\(name): the summary")
            }
        }
    }
}
