import Foundation
import Testing

@testable import AgentKit

/// The plan layer as the Mac's Plan view reads it (ov-273): the CLI's
/// `plan --json`, and every rule the overview, the theme page, the lane page
/// and a task page's line draw.
struct PlanModelTests {
    static let now: Int64 = 1_800_000_000_000
    static let hour: Int64 = 3_600_000

    /// `test/fixtures/plan.json`, which `plan_json_is_the_shape_the_mac_reads`
    /// in `crates/cli/src/plan_tests.rs` writes byte for byte.
    static func fixture() throws -> PlanModel {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try PlanModel.decode(Data(contentsOf: root.appendingPathComponent("test/fixtures/plan.json")))
    }

    // MARK: Building plans

    static func lane(
        _ name: String, _ state: String, cards: [String], rank: Int? = nil, since: Int64 = now - 10 * 60_000,
        train: String? = nil, sha: String? = nil, fixRounds: Int = 0, stale: Bool = false
    ) -> [String: Any] {
        [
            "id": "lane-\(name)", "short": name, "name": name, "state": state, "reason": "Why \(name)",
            "plan_rank": rank as Any, "worktree_path": "", "branch": name, "harness": "claude", "model": "sonnet",
            "train": train as Any, "landed_sha": sha as Any, "state_since": since, "stale": stale,
            "fix_rounds": fixRounds,
            "cards": cards.map { ["task": $0, "key": "ov-\($0.dropFirst())", "slice": ""] },
            "agents": [], "spend": ["input_tokens": 0, "runs": 0, "unmeasured_agents": 0],
        ]
    }

    static func theme(_ name: String, cards: [String], ordinal: Int, state: String = "active") -> [String: Any] {
        [
            "id": "theme-\(name)", "short": name, "name": name, "outcome": "", "story": "", "story_at": 0,
            "next": "", "owner_ask": "", "state": state, "ordinal": ordinal,
            "cards": cards.map { ["task": $0, "key": "ov-\($0.dropFirst())"] },
            "counts": ["backlog": 0, "todo": 0, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": 0,
                       "cancelled": 0],
        ]
    }

    static func plan(themes: [[String: Any]], lanes: [[String: Any]], order: [String] = []) throws -> PlanModel {
        let object: [String: Any] = [
            "now_ms": now, "themes": themes, "lanes": lanes, "order": order.map { "lane-\($0)" }, "cards": [],
        ]
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    // MARK: The wire

    @Test("plan --json decodes whole: themes, lanes with agents and spend, the order and the cards")
    func decodesTheCLIsShape() throws {
        let plan = try Self.fixture()
        #expect(plan.nowMs == Self.now)
        #expect(plan.themes.count == 1)
        let theme = try #require(plan.themes.first)
        #expect(theme.name == "Visual language")
        #expect(theme.outcome == "Every Mac surface reads as one app.")
        #expect(theme.ownerAsk == "Should the sidebar tint follow the terminal theme?")
        #expect(theme.storyAt == Self.now - Self.hour)
        #expect(theme.cards.map(\.key) == ["ov-1", "ov-2", "ov-3"])
        #expect(theme.counts.done == 1 && theme.counts.backlog == 2)
        #expect(plan.lanes.map(\.state) == [.queued, .review, .landed])
        let review = plan.lanes[1]
        #expect(review.name == "mac-ux" && review.train == "integ-9" && review.fixRounds == 1)
        #expect(review.cards == [PlanCardRef(task: plan.cards[0].task, key: "ov-1", slice: "Mac")])
        #expect(review.agents.map(\.role) == ["build", "review"])
        #expect(review.agents[0].endedAt == Self.now - Self.hour && review.agents[1].endedAt == nil)
        #expect(review.spend.totalTokens == 470_000 && review.spend.costMicros == 31_000_000)
        #expect(review.spend.unmeasuredAgents == 1)
        #expect(plan.lanes[2].landedSha == "4d3c8cb1e2")
        #expect(plan.nextUp.map(\.name) == ["mac-fu3"])
        #expect(plan.nextUp.first?.planRank == 1)
        #expect(plan.cards.map(\.key) == ["ov-1", "ov-2", "ov-3"])
    }

    @Test("A state this build has no word for decodes as unknown, not as a failed read")
    func unknownState() throws {
        let plan = try Self.plan(themes: [], lanes: [Self.lane("x", "orbiting", cards: ["t1"])])
        #expect(plan.lanes.first?.state == .unknown)
    }

    // MARK: The overview

    @Test("Next Up is the plan's order; Now is every live lane past queued; queued lanes outside the plan are apart")
    func sections() throws {
        let plan = try Self.plan(
            themes: [],
            lanes: [
                Self.lane("a", "queued", cards: ["t1"], rank: 2), Self.lane("b", "building", cards: ["t2"]),
                Self.lane("c", "queued", cards: ["t3"], rank: 1), Self.lane("d", "landed", cards: ["t4"]),
                Self.lane("e", "fixing", cards: ["t5"]), Self.lane("f", "queued", cards: ["t6"]),
                Self.lane("g", "dropped", cards: ["t7"]),
            ],
            order: ["c", "a"])
        #expect(plan.nextUp.map(\.name) == ["c", "a"])
        #expect(plan.working.map(\.name) == ["b", "e"])
        #expect(plan.unranked.map(\.name) == ["f"])
    }

    @Test("Landed Today is the lanes landed on the runner's day, newest first")
    func landedToday() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        // 1_800_000_000_000 is 15 Jan 2027, 08:00 UTC.
        let plan = try Self.plan(
            themes: [],
            lanes: [
                Self.lane("early", "landed", cards: ["t1"], since: Self.now - 7 * Self.hour),
                Self.lane("late", "landed", cards: ["t2"], since: Self.now - Self.hour),
                Self.lane("yesterday", "landed", cards: ["t3"], since: Self.now - 9 * Self.hour),
                Self.lane("live", "landing", cards: ["t4"], since: Self.now - Self.hour),
            ])
        #expect(plan.landedToday(calendar: calendar).map(\.name) == ["late", "early"])
    }

    @Test("Themes list active, then paused, then done, each in board order; dropped ones not at all")
    func themeOrder() throws {
        let plan = try Self.plan(
            themes: [
                Self.theme("done", cards: [], ordinal: 0, state: "done"),
                Self.theme("second", cards: [], ordinal: 2), Self.theme("paused", cards: [], ordinal: 1, state: "paused"),
                Self.theme("first", cards: [], ordinal: 1), Self.theme("gone", cards: [], ordinal: 0, state: "dropped"),
            ],
            lanes: [])
        #expect(plan.shownThemes.map(\.name) == ["first", "second", "paused", "done"])
    }

    @Test("A lane serves the theme most of its cards are in; a tie goes to the earlier theme")
    func themeOfALane() throws {
        let plan = try Self.plan(
            themes: [
                Self.theme("Reliability", cards: ["t1"], ordinal: 1),
                Self.theme("Visual language", cards: ["t2", "t3"], ordinal: 2),
                Self.theme("Phones", cards: ["t4"], ordinal: 3),
            ],
            lanes: [
                Self.lane("mostly-visual", "building", cards: ["t1", "t2", "t3"]),
                Self.lane("tie", "building", cards: ["t4", "t1"]),
                Self.lane("loose", "queued", cards: ["t9"]),
            ])
        #expect(plan.theme(of: plan.lanes[0])?.name == "Visual language")
        #expect(plan.theme(of: plan.lanes[1])?.name == "Reliability")
        #expect(plan.theme(of: plan.lanes[2]) == nil)
    }

    @Test("A theme's lanes: live as listed, then queued in plan order, then finished newest first")
    func lanesOfATheme() throws {
        let plan = try Self.plan(
            themes: [Self.theme("T", cards: ["t1", "t2"], ordinal: 1)],
            lanes: [
                Self.lane("old", "landed", cards: ["t1"], since: Self.now - 5 * Self.hour),
                Self.lane("q2", "queued", cards: ["t2"], rank: 2), Self.lane("live", "review", cards: ["t1"]),
                Self.lane("q1", "queued", cards: ["t1"], rank: 1),
                Self.lane("new", "landed", cards: ["t2"], since: Self.now - Self.hour),
                Self.lane("elsewhere", "building", cards: ["t9"]),
            ])
        #expect(plan.lanes(in: plan.themes[0]).map(\.name) == ["live", "q1", "q2", "new", "old"])
    }

    @Test("A task page's line names the lane working it, else the one queued, else the last that landed it, and its theme")
    func taskLine() throws {
        let plan = try Self.plan(
            themes: [Self.theme("Visual language", cards: ["t1", "t2", "t3"], ordinal: 1)],
            lanes: [
                Self.lane("landed-old", "landed", cards: ["t1", "t3"], since: Self.now - 5 * Self.hour),
                Self.lane("mac-ux", "review", cards: ["t1"]),
                Self.lane("queued-2", "queued", cards: ["t2"], rank: 2),
                Self.lane("queued-1", "queued", cards: ["t2"], rank: 1),
                Self.lane("landed-new", "landed", cards: ["t3"], since: Self.now - Self.hour),
                Self.lane("dropped", "dropped", cards: ["t4"]),
            ])
        #expect(plan.taskLine("t1")?.text == "In lane mac-ux · Visual language")
        #expect(plan.taskLine("t2")?.text == "Queued in queued-1 · Visual language")
        #expect(plan.taskLine("t3")?.text == "Landed in landed-new · Visual language")
        #expect(plan.taskLine("t4") == nil, "a dropped lane names nothing")
        #expect(plan.taskLine("t9") == nil)
    }

    @Test("A live lane waits on the owner when one of its cards needs a decision; a landed one never does")
    func waitsOnOwner() throws {
        let plan = try Self.plan(
            themes: [],
            lanes: [
                Self.lane("asks", "review", cards: ["t1", "t2"]), Self.lane("quiet", "review", cards: ["t3"]),
                Self.lane("done", "landed", cards: ["t1"]),
            ])
        let statuses: [String: TaskStatus] = ["t1": .needsDecision, "t2": .inReview, "t3": .inProgress]
        #expect(plan.lanes.map { plan.waitsOnOwner($0, statuses: statuses) } == [true, false, false])
    }

    // MARK: Words

    @Test("A lane's status says its round, its place, its commit and its train")
    func status() throws {
        let plan = try Self.plan(
            themes: [],
            lanes: [
                Self.lane("a", "fixing", cards: [], fixRounds: 1), Self.lane("b", "landing", cards: [], train: "integ-8"),
                Self.lane("c", "queued", cards: [], rank: 2), Self.lane("d", "landed", cards: [], sha: "ac840108ffee"),
                Self.lane("e", "review", cards: [], train: "integ-9"), Self.lane("f", "building", cards: []),
            ])
        #expect(plan.lanes.map(PlanWords.status) == [
            "Fixing · round 1", "Landing · train integ-8", "Queued · 2nd", "Landed · ac840108",
            "In Review · train integ-9", "Building",
        ])
    }

    @Test("Ordinals", arguments: [(1, "1st"), (2, "2nd"), (3, "3rd"), (4, "4th"), (11, "11th"), (12, "12th"), (13, "13th"), (21, "21st"), (22, "22nd"), (111, "111th")])
    func ordinals(n: Int, said: String) { #expect(PlanWords.ordinal(n) == said) }

    @Test("Progress counts done over every card but the canceled; the bar runs done, review, progress, not started")
    func progress() {
        var c = PlanCounts()
        c.done = 4
        c.inReview = 5
        c.inProgress = 9
        c.cancelled = 2
        c.backlog = 1
        c.needsDecision = 1
        #expect(PlanWords.progress(c) == "4 of 20 done")
        #expect(PlanWords.segments(c).map(\.kind) == [.done, .inReview, .inProgress, .notStarted])
        #expect(PlanWords.segments(c).map(\.count) == [4, 5, 9, 2])
        c.inReview = 0
        #expect(!PlanWords.segments(c).map(\.kind).contains(.inReview), "an empty part is left out")
        #expect(PlanWords.breakdown(c) == "4 done · 9 in progress · 1 need a decision · 1 backlog")
    }

    @Test("Spend is tokens first; a price is an estimate; nothing measured is Not reported, never zero")
    func spend() {
        let us = Locale(identifier: "en_US")
        var s = PlanSpend()
        s.unmeasuredAgents = 2
        #expect(PlanWords.spend(s, locale: us) == "Not reported")
        s.inputTokens = 300_000
        s.outputTokens = 170_000
        s.costMicros = 31_000_000
        #expect(PlanWords.spend(s, locale: us) == "470K tokens · about $31.00 estimated · 2 agents not reported")
        s.unmeasuredAgents = 0
        s.costMicros = nil
        #expect(PlanWords.spend(s, locale: us) == "470K tokens")
    }

    @Test("An agent reads as its role, its model and how long it ran, to now while it's open")
    func agents() throws {
        let plan = try Self.fixture()
        let words = plan.lanes[1].agents.map { PlanWords.agent($0, now: plan.nowMs) }
        #expect(words == ["Builder Opus, 2 h", "Reviewer Sonnet, 25 min"])
        #expect(PlanWords.model("claude-opus-5-5") == "Opus")
        #expect(PlanWords.model("gpt-5.5") == "gpt-5.5")
    }

    @Test("Only a stale lane gets a warning, and it says how long")
    func stale() throws {
        let plan = try Self.plan(
            themes: [],
            lanes: [
                Self.lane("a", "review", cards: [], since: Self.now - 3 * Self.hour, stale: true),
                Self.lane("b", "review", cards: [], since: Self.now - 70 * 60_000, stale: true),
                Self.lane("c", "review", cards: [], since: Self.now - 3 * Self.hour),
            ])
        #expect(plan.lanes.map { PlanWords.stale($0, now: Self.now) } == ["No move in 3 h", "No move in an hour", nil])
    }

    // MARK: Records

    static func record(_ events: [(Int64, String, String)], to: String? = nil) throws -> PlanRecord {
        var list: [[String: Any]] = []
        for (at, kind, body) in events {
            let extra: [String: Any] = kind == "story" ? ["to": to ?? ""] : [:]
            list.append(["at": at, "actor": "manager", "kind": kind, "body": body, "extra": extra])
        }
        return try PlanRecord.decode(JSONSerialization.data(withJSONObject: ["name": "x", "events": list]))
    }

    @Test("A run of cards added in one write is one line; a story rewrite is said once")
    func timeline() throws {
        let t = Self.now
        let record = try Self.record([
            (t, "state", "Started building."),
            (t, "cards", "Added ov-155."), (t, "cards", "Added ov-156."), (t, "cards", "Added ov-157."),
            (t, "cards", "Added ov-158."),
            (t + 1, "cards", "Removed ov-9."),
            (t + 2 * Self.hour, "cards", "Added ov-200."),
            (t + 3 * Self.hour, "story", "The old story."),
            (t + 3 * Self.hour, "state", "Moved to review. One high finding"),
        ])
        #expect(record.timeline.map(\.text) == [
            "Started building.", "Added ov-155, ov-156 and 2 more cards.", "Removed ov-9.", "Added ov-200.",
            "Rewrote where it stands.", "Moved to review. One high finding",
        ])
        #expect(record.timeline.map(\.id) == Array(0..<6))
    }

    @Test("What Changed is the story the last rewrite replaced")
    func previousStory() throws {
        let none = try Self.record([(Self.now, "state", "Created.")])
        #expect(none.previousStory == nil)
        let first = try Self.record([(Self.now, "story", "")], to: "The first story.")
        #expect(first.previousStory == nil, "writing the first story replaced nothing")
        let twice = try Self.record([
            (Self.now, "story", ""), (Self.now + 1, "story", "Tokens are on main."),
        ])
        #expect(twice.previousStory?.story == "Tokens are on main.")
        #expect(twice.previousStory?.at == Self.now + 1)
    }
}
