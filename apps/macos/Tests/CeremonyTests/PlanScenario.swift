import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Plans for the Plan's sections (ov-503, phase 5): the fixture's shape with
/// the board's own kind of content, long real titles included, so a section is
/// judged at every width with what it will really say. Each is the CLI's
/// `plan --json`, which `PlanModel` decodes as it decodes the runner's.
@MainActor
enum PlanScenario {
    /// The plan's clock; the fixture's.
    static let now: Int64 = 1_800_000_000_000
    static let minute: Int64 = 60_000

    enum Kind {
        /// A landing whose CI failed, tickets in review and building, a ranked
        /// Next, a card waiting on the owner, a ticket stranded on a landed
        /// landing, agents and themes with finished ones.
        case busy
        /// The same with nothing queued and nothing ranked.
        case emptyNext
        /// No tickets moving, nothing queued, no agents.
        case quiet
        /// A landing in every state of the ladder, with long real titles, and
        /// a ticket waiting on the owner and one landed aboard a landing
        /// (ov-522).
        case ladder
    }

    static func id(_ n: Int, _ kind: Int = 2) -> String { String(format: "00000000-0000-0000-0000-%012d", kind * 1000 + n) }

    static func spend(_ tokens: UInt64, dollars: Double?) -> [String: Any] {
        [
            "input_tokens": tokens, "output_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0,
            "cost_micros": dollars.map { Int64($0 * 1_000_000) } as Any? ?? NSNull(), "runs": 1, "unmeasured_agents": 0,
            "shared_agents": 0,
        ]
    }

    /// The task id of card `ov-<card>`.
    static func task(_ card: Int) -> String { id(card, 1) }

    /// One ticket on the line, as the CLI's `plan --json` sends it: the card
    /// is `ov-<card>`, `handle` its branch, `stage` the wire's word
    /// (`waiting`, `building`, `reviewing`, `needs_you`, `landing`, `landed`).
    /// A `fixing` agent puts a fix on it, open, so it reads as a fix round.
    static func line(
        _ card: Int, _ handle: String, _ title: String, stage: String, since: Int64 = 40, landing: Int? = nil,
        rank: Int? = nil, reason: String = "", rounds: Int = 0, stale: Bool = false, landedSha: String? = nil,
        fixing: String? = nil
    ) -> [String: Any] {
        let status =
            switch stage {
            case "waiting": "Backlog"
            case "building": "In Progress"
            case "landed": "Done"
            default: "In Review"
            }
        return [
            "task": task(card), "key": "ov-\(card)", "title": title, "status": status, "stage": stage, "reason": reason,
            "plan_rank": rank as Any? ?? NSNull(), "worktree_path": "", "branch": handle, "stack_parent": NSNull(),
            "same_branch": NSNull(), "pr_number": NSNull(), "landed_sha": landedSha as Any? ?? NSNull(),
            "stage_since": now - since * minute, "held_off": false, "stale": stale, "fix_rounds": rounds,
            "landing": landing as Any? ?? NSNull(), "spend": spend(0, dollars: nil), "budget_tokens": NSNull(),
            "work": fixing.map {
                [["agent": $0, "role": "fix", "started_at": now - since * minute, "ended_at": NSNull(), "verdict": "", "linked_by_description": false]]
            } ?? [] as [[String: Any]],
        ]
    }

    /// A landing: `cards` are the numbers of the tickets it carries.
    static func landing(
        _ n: Int, _ title: String, summary: String, cards: [Int], phase: String, state: String, since: Int64,
        sha: String? = nil, lander: String? = nil
    ) -> [String: Any] {
        [
            "id": id(n, 6), "short": String(format: "%08d", n), "number": n, "title": title, "summary": summary,
            "route": "push", "base": "origin/main", "pushed_sha": sha as Any? ?? NSNull(), "state": state,
            "state_since": now - since * minute, "phase": phase, "phase_since": now - since * minute, "collecting": false,
            "actor": "runner", "created_at": now - (since + 90) * minute, "landed_at": NSNull(),
            "tickets": cards.map { ["task": task($0), "key": "ov-\($0)"] }, "ci_subject": sha.map { "sha:\($0)" } ?? "",
            "lander": lander as Any? ?? NSNull(), "spend": spend(0, dollars: nil),
        ]
    }

    static func theme(
        _ n: Int, _ name: String, state: String = "active", counts: (done: Int, progress: Int, backlog: Int), cards: [String],
        ask: String = "", budget: UInt64? = nil, spent: UInt64 = 0
    ) -> [String: Any] {
        [
            "id": id(n, 3), "short": String(format: "%08d", n), "name": name, "outcome": "", "story": "", "story_at": 0, "next": "",
            "owner_ask": ask, "state": state, "ordinal": n, "last_moved_at": now - Int64(n) * 30 * minute,
            "cards": cards.map { ["task": id(Int($0.dropFirst(3)) ?? 0, 1), "key": $0] },
            "counts": [
                "backlog": counts.backlog, "todo": 0, "needs_decision": 0, "in_progress": counts.progress, "in_review": 0,
                "done": counts.done, "cancelled": 0,
            ],
            "budget_tokens": budget as Any? ?? NSNull(), "spend": spend(spent, dollars: nil), "trend_tokens": [] as [Any],
        ]
    }

    /// An agent as `plan --json` lists it: its role (`orchestrator`, `builder`,
    /// `reviewer`, `lander`, or anything else for a pane that is just an
    /// agent) and the tickets it works, each as (card, work role).
    static func agent(
        _ id: String, _ role: String, model: String, tickets: [(Int, String)] = [], title: String = "", doing: String = "",
        working: Bool, started: Int64, last: Int64 = 0, tokens: UInt64, dollars: Double?
    ) -> [String: Any] {
        [
            "id": id, "short": String(id.suffix(8)), "role": role, "harness": "claude", "model": model, "title": title,
            "terminal": NSNull(), "subagent_id": "", "parent_terminal": NSNull(), "started_at": now - started * minute,
            "stop_asked_at": NSNull(),
            "tickets": tickets.map { ["task": task($0.0), "key": "ov-\($0.0)", "role": $0.1, "started_at": now - started * minute] },
            "working": working, "doing_now": doing, "last_at": now - last * minute, "spend": spend(tokens, dollars: dollars),
        ]
    }

    /// Whether `plan` is this scenario as read: what only this kind has, so a capture of it waits for it (ov-642).
    @MainActor static func shown(_ kind: Kind, in plan: PlanModel) -> Bool {
        let ranked = plan.lines.filter { $0.planRank != nil }
        return switch kind {
        case .busy: ranked.count == 3 && !plan.landings.isEmpty && !plan.agents.isEmpty
        case .emptyNext: ranked.isEmpty && !plan.landings.isEmpty && !plan.agents.isEmpty
        case .quiet: plan.lines.isEmpty && plan.landings.isEmpty && plan.agents.isEmpty
        case .ladder: plan.landings.count == 5
        }
    }

    /// The scenario as the CLI's JSON.
    static func data(_ kind: Kind) throws -> Data {
        var plan = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        plan["now_ms"] = now
        plan["rulings"] = [] as [Any]
        plan["cost"] = NSNull()

        var lines: [[String: Any]] = []
        var landings: [[String: Any]] = []
        var agents: [[String: Any]] = []
        if kind == .ladder {
            let long = "Relay: the team directory, so each person's asks reach the orchestrator, then skill-init writes the manager skill for every runner"
            lines += [
                line(501, "open-a", long, stage: "landing", since: 15, landing: 90),
                line(502, "check-a", "iOS: a draft is never overwritten by the daemon's echo while someone is typing", stage: "landing", since: 30, landing: 91),
                line(503, "check-b", "Mac: the Plan's sections in two columns from 960 points", stage: "landed", since: 40, landing: 91, landedSha: "3f78754fabcdef12"),
                line(504, "ci-a", "CI: the Rust jobs get 35 minutes on a slow runner", stage: "landing", since: 20, landing: 92),
                line(505, "ready-a", "Phones: the Plan on iPhone, iPad and Android in one inset list", stage: "landing", since: 10, landing: 93),
                line(506, "asks-b", "Accounts: how do we sign teams in, and what does the owner decide first", stage: "building", since: 50, landing: 91),
                line(507, "loose-a", "Daemon: a ticket in landing with no landing yet", stage: "landing", since: 8),
                line(508, "main-a", "Daemon: the collecting landing leaves once main is green again", stage: "landing", since: 5, landing: 94),
            ]
            landings += [
                landing(90, "People on a shared runner; sign in on the web", summary: "Relay: the team directory", cards: [501], phase: "waiting_for_previous_landing", state: "open", since: 15),
                landing(91, "Landing 91 \u{00B7} Agents tell the orchestrator when they are done", summary: "iOS drafts; Mac sections; Accounts", cards: [502, 503, 506], phase: "gating_locally", state: "gating", since: 30),
                landing(92, "CI minutes", summary: "CI: the Rust jobs get 35 minutes", cards: [504], phase: "waiting_for_ci", state: "pushed", since: 20, sha: "9a1c0d22"),
                landing(93, "The Plan on every phone", summary: "Phones: the Plan in one inset list", cards: [505], phase: "ready_to_land", state: "green", since: 10, sha: "4d3c8cb1"),
                landing(94, "Main's checks first", summary: "Daemon: the collecting landing waits for main", cards: [508], phase: "waiting_for_main", state: "open", since: 5),
            ]
        }
        if kind != .quiet && kind != .ladder {
            lines += [
                line(487, "team-dir", "Relay: the team directory, so each person's asks reach the orchestrator", stage: "landing", since: 50, landing: 86),
                line(492, "ios-overwrite", "iOS: a draft is never overwritten by the daemon's echo while someone is typing in the composer", stage: "building", since: 130, landing: 86, rounds: 2, stale: true, fixing: "agent:b3"),
                line(301, "ci-green", "CI: the Rust jobs get 35 minutes on a slow runner", stage: "landing", since: 20, landing: 88),
                line(503, "mac-plan", "Plan: the sections, glance band, landing card, agents table and themes grid on the Mac", stage: "reviewing", since: 25),
                line(485, "tail-tunnel", "Tailcat: the tunnel is the default transport and falls back to SSH only on request", stage: "reviewing", since: 190, stale: true),
                line(490, "site-ship", "Site: one web project, previewed before it ships", stage: "building", since: 70),
                line(504, "phones-plan", "Phones: the Plan on iPhone, iPad and Android in one inset list", stage: "building", since: 12),
                line(474, "old-one", "Relay: a ticket that stayed waiting on a landing that has landed", stage: "waiting", since: 3000, landing: 83),
                line(482, "old-two", "Daemon: the second ticket stranded on Landing 83", stage: "landing", since: 3100, landing: 83),
            ]
            landings += [
                landing(86, "Teams on a shared runner", summary: "Relay: the team directory, so each person's asks reach the orchestrator; iOS: a draft is never overwritten by the daemon's echo", cards: [487, 492], phase: "waiting_for_tickets", state: "open", since: 130),
                landing(88, "Landing 88", summary: "CI: the Rust jobs get 35 minutes", cards: [301], phase: "ci_failed", state: "red", since: 12, sha: "3f78754f", lander: "agent:i1"),
            ]
            var landed = landing(83, "Landing 83", summary: "Daemon relays", cards: [474, 482], phase: "landed", state: "landed", since: 3000, sha: "9a1c0d22")
            landed["landed_at"] = now - 3000 * minute
            landings.append(landed)
            agents += [
                agent("pane:orchestrator", "orchestrator", model: "opus", working: true, started: 240, tokens: 2_100_000, dollars: 14.2),
                agent("agent:b1", "builder", model: "sonnet", tickets: [(490, "build")], doing: "Wiring the preview deploy", working: true, started: 22, tokens: 380_000, dollars: 1.1),
                agent("agent:b2", "builder", model: "opus", tickets: [(504, "build")], doing: "Drawing the phone sections", working: true, started: 12, tokens: 160_000, dollars: 2.4),
                agent("agent:i1", "lander", model: "opus", tickets: [(301, "land")], working: true, started: 40, tokens: 1_400_000, dollars: 6.3),
                // A subagent resumed for another round; the reviewer that finished (610K, $3.90) is not listed.
                agent("agent:s1", "builder", model: "sonnet", title: "Design ov-503", working: true, started: 25, last: 4, tokens: 220_000, dollars: 0.6),
                // A pane that still runs between turns.
                agent("pane:idle", "unknown", model: "opus", working: false, started: 300, last: 50, tokens: 90_000, dollars: 0.5),
            ]
        }
        if kind == .busy {
            lines += [
                line(494, "reps", "Reps: each person's asks reach the orchestrator, then skill-init writes the manager skill", stage: "waiting", rank: 1, reason: "Frees the Mac slot"),
                line(495, "skill-init", "Onboarding: skill-init explains what the manager skill is for, in the owner's words", stage: "waiting", rank: 2, reason: "Needs reps first"),
                line(496, "watch-glance", "Watch: the glance sentence on the complication", stage: "waiting", rank: 3),
                line(497, "unranked-a", "Android: tablets match the iPad layout", stage: "waiting"),
                line(1, "asks-you", "Accounts: how do we sign teams in", stage: "waiting"),
            ]
        }
        plan["lines"] = lines
        plan["landings"] = landings
        plan["agents"] = agents
        // The day's spend: the listed agents' and the ones that finished or exited.
        // The ladder scenario has no agent alive and a day that spent something: the header says so.
        plan["agents_spend"] = agents.isEmpty && kind != .ladder ? NSNull() : spend(kind == .ladder ? 1_200_000 : 6_300_000, dollars: kind == .ladder ? 12 : 31.7)
        plan["ci"] = [
            [
                "subject": "sha:3f78754f", "sha": "3f78754fa0b3c1d2e3f40516273849a5b6c7d8e9", "status": "failed",
                "url": "https://github.com/taneliang/farcooler/actions/runs/37275435256", "fetched_at": now - 2 * minute,
                "changed_at": now - 12 * minute, "asked_at": now - 2 * minute,
                "jobs": [
                    ["name": "CI / Rust (ubuntu-latest)", "state": "failed", "url": ""],
                    ["name": "CI / Swift (shared + macOS)", "state": "passed", "url": ""],
                    ["name": "CI / Android", "state": "passed", "url": ""],
                ],
            ] as [String: Any]
        ]
        if kind == .ladder {
            // Landing 92 is running its checks; Landing 93's passed, which is why it is ready to land.
            func read(_ sha: String, _ status: String, _ states: [String]) -> [String: Any] {
                [
                    "subject": "sha:\(sha)", "sha": sha + "fa0b3c1d2e3f40516273849a5b6c7d8e9", "status": status,
                    "url": "https://github.com/taneliang/farcooler/actions/runs/\(sha)", "fetched_at": now - minute,
                    "changed_at": now - 8 * minute, "asked_at": now - minute,
                    "jobs": states.enumerated().map { ["name": "CI / Job \($0.offset + 1)", "state": $0.element, "url": ""] },
                ]
            }
            plan["ci"] = (plan["ci"] as? [[String: Any]] ?? []) + [read("9a1c0d22", "running", ["passed", "running", "queued"]), read("4d3c8cb1", "passed", ["passed", "passed", "passed"])]
        }

        let themeCards = [
            ("Teams and accounts", "ov-487", ["ov-487", "ov-485", "ov-486", "ov-480", "ov-481"], (6, 2, 2)),
            ("Trustworthy agent view", "ov-492", ["ov-492", "ov-493", "ov-470", "ov-471"], (45, 2, 1)),
            ("Mac look and feel", "ov-503", ["ov-503", "ov-460", "ov-461"], (26, 1, 1)),
            ("A website that explains Far Cooler to someone who has never seen it", "ov-490", ["ov-490", "ov-440"], (3, 1, 6)),
            ("Phones", "ov-504", ["ov-504", "ov-420"], (10, 1, 4)),
            ("Cost you can read", "ov-307", ["ov-307"], (8, 0, 0)),
            ("Onboarding", "ov-495", ["ov-495", "ov-496"], (2, 0, 5)),
        ]
        var themes: [[String: Any]] = []
        var cards: [[String: Any]] = []
        var seen = Set<String>()
        for (index, row) in themeCards.enumerated() {
            let state = row.0 == "Cost you can read" ? "done" : "active"
            themes.append(
                theme(
                    index + 1, row.0, state: state, counts: (done: row.3.0, progress: row.3.1, backlog: row.3.2), cards: row.2,
                    ask: index == 3 ? "Should the site ship now, or wait for the owner's preview?" : "",
                    budget: index == 0 ? 250_000 : nil, spent: index == 0 ? 320_000 : 0))
            for (offset, key) in row.2.enumerated() where seen.insert(key).inserted {
                let status = offset == 0 ? "In Progress" : offset == 1 ? "Done" : "Backlog"
                cards.append([
                    "task": id(Int(key.dropFirst(3)) ?? 0, 1), "key": key, "status": state == "done" ? "Done" : status,
                    "title": "\(key): \(["Questions open and read in place", "Messages queue while the runner is away", "Copy follows the guidelines everywhere", "A composer that never loses a draft"][offset % 4])",
                ])
            }
        }
        for (index, name) in ["Release", "Docs", "Feedback", "Billing"].enumerated() {
            themes.append(theme(20 + index, name, state: "done", counts: (done: 4, progress: 0, backlog: 0), cards: []))
        }
        plan["themes"] = themes
        plan["cards"] = cards
        return try JSONSerialization.data(withJSONObject: plan)
    }
}
