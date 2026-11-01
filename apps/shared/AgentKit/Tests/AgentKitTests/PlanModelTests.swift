import Foundation
import Testing

@testable import AgentKit

/// The plan layer as the Mac's Plan view reads it (ov-273): the CLI's
/// `plan --json`, and every rule the overview, the theme page, the ticket page
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

    /// A ticket on the line, as the wire sends it. `task` is "t1", whose key is
    /// "ov-1", as a theme's cards are. `fixing` leaves a fix open on it.
    static func line(
        _ task: String, _ stage: String, rank: Int? = nil, since: Int64 = now - 10 * 60_000,
        landing: Int? = nil, sha: String? = nil, fixRounds: Int = 0, stale: Bool = false, fixing: Bool = false,
        title: String = "", prNumber: Int? = nil
    ) -> [String: Any] {
        let work: [[String: Any]] =
            fixing ? [["agent": "agent-\(task)", "role": "fix", "started_at": since, "ended_at": NSNull(), "verdict": "", "linked_by_description": false]] : []
        return [
            "task": task, "key": "ov-\(task.dropFirst())", "title": title, "status": "In Progress", "stage": stage,
            "reason": "Why \(task)", "plan_rank": rank as Any, "worktree_path": "", "branch": "branch-\(task)",
            "stack_parent": NSNull(), "same_branch": NSNull(), "pr_number": prNumber as Any, "landed_sha": sha as Any,
            "stage_since": since, "held_off": false, "stale": stale, "fix_rounds": fixRounds,
            "landing": landing as Any, "work": work,
            "spend": ["input_tokens": 0, "runs": 0, "unmeasured_agents": 0],
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

    /// A landing on the wire, cutting `tickets` (tasks) into `state`.
    static func landing(
        _ number: Int, _ state: String, tickets: [String] = [], sha: String? = nil, phase: String? = nil,
        since: Int64 = now - 60_000, phaseSince: Int64? = nil, lander: String? = nil, title: String? = nil
    ) -> [String: Any] {
        [
            "id": "landing-\(number)", "short": "landing-\(number)", "number": number, "base": "origin/main",
            "pushed_sha": sha as Any, "route": "push", "state": state, "state_since": since, "phase": phase as Any,
            "phase_since": phase == nil ? NSNull() : (phaseSince ?? since) as Any, "collecting": false, "actor": "manager",
            "created_at": now - 3_600_000, "landed_at": NSNull(),
            "tickets": tickets.map { ["task": $0, "key": "ov-\($0.dropFirst())"] },
            "ci_subject": sha.map { "sha:\($0)" } ?? "", "title": title ?? "Landing \(number)", "summary": "",
            "lander": lander as Any, "spend": NSNull(),
        ]
    }

    /// An agent on the wire: with no pane unless `terminal` is given, working
    /// `tickets` (tasks) as the given roles.
    static func agent(
        _ id: String, role: String = "builder", model: String = "claude-opus-5-5", terminal: String? = nil,
        working: Bool = true, tickets: [(task: String, role: String)] = []
    ) -> [String: Any] {
        [
            "id": id, "short": id, "role": role, "harness": "claude", "model": model, "title": "",
            "terminal": terminal as Any, "subagent_id": "", "parent_terminal": NSNull(), "stop_asked_at": NSNull(),
            "tickets": tickets.map { ["task": $0.task, "key": "ov-\($0.task.dropFirst())", "role": $0.role, "started_at": 0] },
            "doing_now": "", "working": working, "started_at": 0, "last_at": 0,
            "spend": ["input_tokens": 0, "runs": 0, "unmeasured_agents": 0],
        ]
    }

    static func plan(
        themes: [[String: Any]], lines: [[String: Any]], landings: [[String: Any]] = [], agents: [[String: Any]] = [],
        ci: [[String: Any]] = []
    ) throws -> PlanModel {
        let object: [String: Any] = [
            "now_ms": now, "themes": themes, "lines": lines, "landings": landings, "agents": agents, "cards": [],
            "ci": ci,
        ]
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    // MARK: The wire

    private static func id(_ n: Int) -> String { String(format: "00000000-0000-0000-0000-%012d", n) }

    @Test("plan --json decodes whole: themes, the line with its work, landings, agents, CI, cost, rulings and the flagged cards")
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
        #expect(theme.budgetTokens == 250_000)
        #expect(theme.trendTokens == [0, 0, 0, 40_000, 120_000, 0, 160_000])

        // The line: one ticket per card, in the order the runner sent.
        #expect(plan.lines.map(\.key) == ["ov-2", "ov-1", "ov-4", "ov-3"])
        #expect(plan.lines.map(\.stage) == [.waiting, .reviewing, .landing, .landed])
        let waiting = plan.lines[0]
        #expect(waiting.task == Self.id(1002) && waiting.planRank == 1 && waiting.reason == "Frees the Mac slot")
        #expect(waiting.heading == "Title of ov-2" && waiting.status == "Backlog" && waiting.work.isEmpty)
        let review = plan.lines[1]
        #expect(review.branch == "ov-1-mac-polish" && review.fixRounds == 1 && review.landing == nil)
        #expect(review.budgetTokens == 500_000 && review.worktreePath == ".claude/worktrees/ov-1")
        #expect(review.card == PlanCardRef(task: plan.cards[0].task, key: "ov-1"))
        #expect(review.work.map(\.role) == [.build, .review])
        #expect(review.work.map(\.agent) == [Self.id(7002), Self.id(7003)])
        #expect(review.work[0].endedAt == Self.now - Self.hour && review.work[0].verdict == "finished")
        #expect(review.work[1].endedAt == nil && review.work[1].isOpen)
        #expect(review.spend.totalTokens == 470_000 && review.spend.costMicros == 31_000_000)
        #expect(review.spend.unmeasuredAgents == 1)
        #expect(review.spend.sharedAgents == 1)
        let landing = plan.lines[2]
        #expect(landing.landing == 9 && landing.prNumber == 31 && landing.branch == "ov-4-glance")
        #expect(landing.stackParent == PlanCardRef(task: Self.id(1001), key: "ov-1"))
        #expect(plan.lines[3].landedSha == "4d3c8cb1e2")
        #expect(plan.nextUp.map(\.key) == ["ov-2"])
        #expect(plan.nextUp.first?.planRank == 1)
        #expect(plan.cards.map(\.key) == ["ov-1", "ov-2", "ov-3", "ov-4", "ov-5"])

        // Landings: Landing 9 is red with its CI failing, Landing 8 landed.
        #expect(plan.landings.map(\.number) == [9, 8])
        let nine = plan.landings[0]
        #expect(nine.label == "Landing 9" && nine.state == .red && nine.phase == .ciFailed && nine.route == .push)
        #expect(nine.tickets == [PlanCardRef(task: Self.id(1004), key: "ov-4")])
        #expect(nine.pushedSha == "c85bf83d" && nine.ciSubject == "sha:c85bf83d" && nine.base == "origin/main")
        #expect(nine.lander == Self.id(7004) && nine.spend?.costMicros == 4_000_000)
        #expect(nine.summary == "Title of ov-4" && nine.subtitle == nil, "a title that is only its label says no more")
        let eight = plan.landings[1]
        #expect(eight.state == .landed && eight.lander == nil && eight.spend == nil)
        #expect(eight.subtitle == "Agents message the plan" && eight.carries == "Title of ov-3")
        #expect(plan.lines(on: nine).map(\.key) == ["ov-4"])

        // Agents: who works now and what each is doing.
        #expect(plan.agents.map(\.role) == [.orchestrator, .reviewer, .builder])
        let orchestrator = plan.agents[0]
        #expect(orchestrator.doingNow == "2/5 \u{00B7} Reviewing the plan" && orchestrator.working && orchestrator.tickets.isEmpty)
        #expect(orchestrator.terminal == Self.id(8001) && orchestrator.model == "opus")
        let reviewer = plan.agents[1]
        #expect(reviewer.title == "Review ov-1" && reviewer.subagentId == "a3" && reviewer.terminal == nil)
        #expect(reviewer.parentTerminal == Self.id(8001) && reviewer.doingNow == "Reading PlanColumn.swift")
        #expect(reviewer.tickets.map(\.key) == ["ov-1"] && reviewer.tickets.first?.role == .review)
        #expect(reviewer.tickets.first?.task == Self.id(1001) && reviewer.tickets.first?.startedAt == Self.now - 1_500_000)
        let builder = plan.agents[2]
        #expect(builder.harness == "codex" && builder.model == "gpt-5.6" && !builder.working)
        #expect(builder.stopAskedAt == Self.now - 60_000 && builder.doingNow.isEmpty)
        #expect(builder.spend.unmeasuredAgents == 1)
        #expect(plan.agents.count == 3, "the lander of Landing 9 is named but no longer exists")
        #expect(plan.lander(of: nine) == nil)
        #expect(plan.agentsSpend?.costMicros == 20_000_000 && plan.agentsSpend?.totalTokens == 3_000_000)

        // CI, cost, rulings and the flagged cards.
        let ci = try #require(plan.ci(of: nine))
        #expect(ci.status == .failed && ci.sha == "c85bf83dce46a6b71d7312afc623899ae7914658")
        #expect(ci.jobs.map(\.state) == ["failed", "passed", "passed"] && ci.needsAttention)
        let cost = try #require(plan.cost)
        #expect(cost.weekTokens == 34_200_000 && cost.weekCostMicros == 45_000_000 && cost.inFlightTokens == 1_200_000)
        #expect(cost.compare.map(\.model) == ["opus", "gpt-5.6"] && cost.compareHeldBack == 2)
        #expect(plan.rulings.map(\.short) == ["R-2", "R-1"] && plan.rulings.map(\.state) == [.standing, .confirmed])
        #expect(plan.rulings[0].cards.map(\.key) == ["ov-1"] && plan.rulings[0].theme == "Visual language")
        #expect(plan.boardCounts?.done == 11 && plan.boardCounts?.inReview == 3)
        #expect(plan.offTheLine.map(\.key) == ["ov-5"] && plan.offTheLine.first?.status == "In Progress")
        #expect(plan.landedNotClosed.map(\.key) == ["ov-3"])
    }

    @Test("A stage this build has no word for decodes as unknown, not as a failed read")
    func unknownStage() throws {
        let plan = try Self.plan(themes: [], lines: [Self.line("t1", "orbiting")])
        #expect(plan.lines.first?.stage == .unknown)
    }

    // MARK: The overview

    @Test("Next Up is the plan's rank; Now is every live ticket past waiting; waiting tickets outside the plan are apart")
    func sections() throws {
        let plan = try Self.plan(
            themes: [],
            lines: [
                Self.line("t1", "waiting", rank: 2), Self.line("t2", "building"),
                Self.line("t3", "waiting", rank: 1), Self.line("t4", "landed"),
                Self.line("t5", "building", fixing: true), Self.line("t6", "waiting"),
                Self.line("t7", "dropped"), Self.line("t8", "needs_you"),
            ])
        #expect(plan.nextUp.map(\.task) == ["t3", "t1"], "by rank, not by the line's order")
        #expect(plan.working.map(\.task) == ["t2", "t5", "t8"])
        #expect(plan.unranked.map(\.task) == ["t6"])
    }

    @Test("Landed Today is the tickets landed on the runner's day, newest first")
    func landedToday() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        // 1_800_000_000_000 is 15 Jan 2027, 08:00 UTC.
        let plan = try Self.plan(
            themes: [],
            lines: [
                Self.line("t1", "landed", since: Self.now - 7 * Self.hour),
                Self.line("t2", "landed", since: Self.now - Self.hour),
                Self.line("t3", "landed", since: Self.now - 9 * Self.hour),
                Self.line("t4", "landing", since: Self.now - Self.hour),
            ])
        #expect(plan.landedToday(calendar: calendar).map(\.task) == ["t2", "t1"])
    }

    @Test("Themes list active, then paused, then done, each in board order; dropped ones not at all")
    func themeOrder() throws {
        let plan = try Self.plan(
            themes: [
                Self.theme("done", cards: [], ordinal: 0, state: "done"),
                Self.theme("second", cards: [], ordinal: 2), Self.theme("paused", cards: [], ordinal: 1, state: "paused"),
                Self.theme("first", cards: [], ordinal: 1), Self.theme("gone", cards: [], ordinal: 0, state: "dropped"),
            ],
            lines: [])
        #expect(plan.shownThemes.map(\.name) == ["first", "second", "paused", "done"])
    }

    @Test("A ticket serves the theme its card is in; a card in two themes goes to the earlier; a dropped theme serves none")
    func themeOfATicket() throws {
        let plan = try Self.plan(
            themes: [
                Self.theme("Reliability", cards: ["t1", "t4"], ordinal: 1),
                Self.theme("Visual language", cards: ["t1", "t2"], ordinal: 2),
                Self.theme("Retired", cards: ["t5"], ordinal: 0, state: "dropped"),
            ],
            lines: [
                Self.line("t1", "building"), Self.line("t2", "building"), Self.line("t3", "waiting"),
                Self.line("t5", "building"),
            ])
        #expect(plan.theme(of: plan.lines[0])?.name == "Reliability", "listed by both: the earlier on the board")
        #expect(plan.theme(of: plan.lines[1])?.name == "Visual language")
        #expect(plan.theme(of: plan.lines[2]) == nil)
        #expect(plan.theme(of: plan.lines[3]) == nil, "its only theme was dropped")
    }

    @Test("A theme's tickets: live as listed, then waiting in plan order, then finished newest first")
    func linesOfATheme() throws {
        let plan = try Self.plan(
            themes: [Self.theme("T", cards: ["t1", "t2", "t3", "t4", "t5", "t6"], ordinal: 1)],
            lines: [
                Self.line("t1", "landed", since: Self.now - 5 * Self.hour),
                Self.line("t2", "waiting", rank: 2), Self.line("t3", "reviewing"),
                Self.line("t4", "waiting", rank: 1),
                Self.line("t5", "landed", since: Self.now - Self.hour),
                Self.line("t9", "building"),
                Self.line("t6", "needs_you"),
            ])
        #expect(plan.lines(in: plan.themes[0]).map(\.task) == ["t3", "t6", "t4", "t2", "t5", "t1"])
    }

    @Test("A task page's line names the ticket's stage and its theme; a dropped ticket names only the theme")
    func taskLine() throws {
        let plan = try Self.plan(
            themes: [Self.theme("Visual language", cards: ["t1", "t2", "t3", "t4"], ordinal: 1)],
            lines: [
                Self.line("t1", "reviewing"), Self.line("t2", "waiting", rank: 2),
                Self.line("t3", "landed", sha: "ac840108ffee"), Self.line("t4", "dropped"),
                Self.line("t5", "dropped"),
            ])
        #expect(plan.taskLine("t1")?.text == "In Review \u{00B7} Visual language")
        #expect(plan.taskLine("t2")?.text == "Waiting \u{00B7} 2nd \u{00B7} Visual language")
        #expect(plan.taskLine("t3")?.text == "Landed \u{00B7} ac840108 \u{00B7} Visual language")
        #expect(plan.taskLine("t4")?.text == "Visual language", "a dropped ticket is no line, but its card is in the theme")
        #expect(plan.taskLine("t5") == nil, "a dropped ticket in no theme names nothing")
        #expect(plan.taskLine("t9") == nil)
    }

    @Test("A live ticket waits on the owner when the runner says Needs You or its card needs a decision; a landed one never does")
    func waitsOnOwner() throws {
        let plan = try Self.plan(
            themes: [],
            lines: [
                Self.line("t1", "reviewing"), Self.line("t2", "reviewing"), Self.line("t3", "needs_you"),
                Self.line("t4", "landed"), Self.line("t5", "waiting"),
            ])
        let statuses: [String: TaskStatus] = ["t1": .needsDecision, "t2": .inReview, "t3": .inProgress, "t4": .needsDecision]
        #expect(plan.lines.map { plan.waitsOnOwner($0, statuses: statuses) } == [true, false, true, false, false])
    }

    // MARK: Words

    @Test("A ticket's status says its round, its place, its commit and its landing")
    func status() throws {
        let plan = try Self.plan(
            themes: [],
            lines: [
                Self.line("t1", "building", fixRounds: 1, fixing: true), Self.line("t2", "landing", landing: 8),
                Self.line("t3", "waiting", rank: 2), Self.line("t4", "landed", sha: "ac840108ffee"),
                Self.line("t5", "reviewing", landing: 9), Self.line("t6", "building"),
                Self.line("t7", "needs_you"),
            ])
        #expect(plan.lines.map(PlanWords.status) == [
            "Fixing \u{00B7} round 1", "Landing \u{00B7} in Landing 8", "Waiting \u{00B7} 2nd", "Landed \u{00B7} ac840108",
            "In Review \u{00B7} in Landing 9", "Building", "Needs You",
        ])
        #expect(plan.lines[0].isFixing && !plan.lines[5].isFixing)
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
        s.sharedAgents = 1
        #expect(PlanWords.spend(s, locale: us) == "470K tokens · 1 agent’s spend split with other tickets")
        s.sharedAgents = 2
        #expect(PlanWords.spend(s, locale: us) == "470K tokens · 2 agents’ spend split with other tickets")
    }

    @Test("Work reads as its role, the agent's model and how long it ran, to now while it's open")
    func work() throws {
        let plan = try Self.fixture()
        let review = try #require(plan.line(key: "ov-1"))
        let words = review.work.map { PlanWords.work($0, agent: plan.agent($0.agent), now: plan.nowMs) }
        // The builder is the codex agent (model gpt-5.6, not a family this build names); the open review ran 25 min.
        #expect(words == ["Builder gpt-5.6, 2 h", "Reviewer sonnet, 25 min"])
        var fix = review.work[0]
        fix.role = .fix
        #expect(PlanWords.work(fix, agent: nil, now: plan.nowMs) == "Fixer, 2 h", "no agent left: no model said")
        fix.role = .land
        #expect(PlanWords.work(fix, agent: plan.agent(fix.agent), now: plan.nowMs).hasPrefix("Lander "))
        #expect(PlanWords.model("gpt-6.1-sol") == "GPT-6.1 Sol")
        #expect(PlanWords.model("gpt-6.1-terra") == "GPT-6.1 Terra")
        #expect(PlanWords.model("gemini-2.5-pro") == "Gemini 2.5 Pro")
        #expect(PlanWords.model("gpt-6-luna") == "GPT-6 Luna")
        #expect(PlanWords.model("claude-opus-5-5-20261101") == "claude-opus-5-5-20261101")
        #expect(PlanWords.model("gpt-5.5") == "gpt-5.5")
    }

    @Test("Only a stale ticket gets a warning, and it says how long")
    func stale() throws {
        let plan = try Self.plan(
            themes: [],
            lines: [
                Self.line("t1", "reviewing", since: Self.now - 3 * Self.hour, stale: true),
                Self.line("t2", "reviewing", since: Self.now - 70 * 60_000, stale: true),
                Self.line("t3", "reviewing", since: Self.now - 3 * Self.hour),
            ])
        #expect(plan.lines.map { PlanWords.stale($0, now: Self.now) } == ["No move in 3 h", "No move in an hour", nil])
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
