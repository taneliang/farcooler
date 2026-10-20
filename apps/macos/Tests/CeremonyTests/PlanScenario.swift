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
        /// A train from a runner before Train 119, which still sends `held_by`, a train whose CI failed, lanes in review and building,
        /// a ranked Next, a card waiting on the owner, a stranded lane, agents
        /// and themes with finished ones.
        case busy
        /// The same with nothing queued and nothing ranked.
        case emptyNext
        /// No lanes moving, nothing queued, no agents.
        case quiet
        /// A train in every state of the ladder, with long real titles, and a
        /// lane waiting on the owner and one landed aboard a train (ov-522).
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

    static func lane(
        _ n: Int, _ name: String, _ title: String, state: String, since: Int64 = 40, cards: [String] = [], train: String? = nil,
        rank: Int? = nil, reason: String = "", rounds: Int = 0, stale: Bool = false, landedSha: String? = nil
    ) -> [String: Any] {
        [
            "id": id(n), "short": String(format: "%08d", n), "name": name, "title": title, "state": state,
            "state_since": now - since * minute, "reason": reason, "plan_rank": rank as Any? ?? NSNull(),
            "worktree_path": "", "branch": name, "harness": "claude", "model": "opus", "train": train as Any? ?? NSNull(),
            "landed_sha": landedSha as Any? ?? NSNull(), "stale": stale, "fix_rounds": rounds, "budget_tokens": NSNull(),
            "cards": cards.map { ["task": id(Int($0.dropFirst(3)) ?? 0, 1), "key": $0, "slice": ""] }, "agents": [] as [Any],
            "spend": spend(0, dollars: nil),
        ]
    }

    static func train(
        _ n: Int, _ title: String, summary: String, lanes: [(Int, String)], phase: String, state: String, since: Int64,
        sha: String? = nil, held: [(Int, String)] = [], agent: Bool = true
    ) -> [String: Any] {
        var train: [String: Any] = [
            "id": id(n, 6), "short": String(format: "%08d", n), "name": "train-\(n)", "title": title, "summary": summary,
            "base": "origin/main", "pushed_sha": sha as Any? ?? NSNull(), "state": state, "state_since": now - since * minute,
            "phase": phase, "phase_since": now - since * minute, "held_by": held.map { ["lane": id($0.0), "name": $0.1] },
            "actor": "manager", "created_at": now - (since + 90) * minute, "landed_at": NSNull(),
            "lanes": lanes.map { ["lane": id($0.0), "name": $0.1] }, "ci_subject": sha.map { "sha:\($0)" } ?? "",
        ]
        if agent {
            train["agent"] = [
                "harness": "claude", "agent_id": "int-\(n)", "model": "opus", "started_at": now - (since + 60) * minute,
                "ended_at": NSNull(), "spend": spend(1_400_000, dollars: 6.3),
            ]
        }
        return train
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

    static func agent(
        _ id: String, _ role: String, model: String, lane: (Int, String)? = nil, train: (Int, String)? = nil, label: String = "",
        working: Bool, started: Int64, last: Int64 = 0, tokens: UInt64, dollars: Double?
    ) -> [String: Any] {
        [
            "id": id, "role": role, "harness": "claude", "model": model, "label": label,
            "lane": lane.map { ["id": PlanScenario.id($0.0), "name": $0.1] } as Any? ?? NSNull(),
            "train": train.map { ["id": PlanScenario.id($0.0, 6), "title": $0.1] } as Any? ?? NSNull(), "card": NSNull(),
            "terminal": NSNull(), "working": working, "started_at": now - started * minute, "last_at": now - last * minute,
            "spend": spend(tokens, dollars: dollars),
        ]
    }

    /// The scenario as the CLI's JSON.
    static func data(_ kind: Kind) throws -> Data {
        var plan = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        plan["now_ms"] = now
        plan["rulings"] = [] as [Any]
        plan["cost"] = NSNull()

        var lanes: [[String: Any]] = []
        var trains: [[String: Any]] = []
        var agents: [[String: Any]] = []
        var order: [String] = []
        if kind == .ladder {
            let long = "Relay: the team directory, so each person's asks reach the orchestrator, then skill-init writes the manager skill for every runner"
            lanes += [
                lane(21, "open-a", long, state: "landing", since: 15, cards: ["ov-501"], train: "train-90"),
                lane(22, "check-a", "iOS: a draft is never overwritten by the daemon's echo while someone is typing", state: "landing", since: 30, cards: ["ov-502"], train: "train-91"),
                lane(23, "check-b", "Mac: the Plan's sections in two columns from 960 points", state: "landed", since: 40, cards: ["ov-503"], train: "train-91", landedSha: "3f78754fabcdef12"),
                lane(24, "ci-a", "CI: the Rust jobs get 35 minutes on a slow runner", state: "landing", since: 20, cards: ["ov-504"], train: "train-92"),
                lane(25, "ready-a", "Phones: the Plan on iPhone, iPad and Android in one inset list", state: "landing", since: 10, cards: ["ov-505"], train: "train-93"),
                lane(26, "asks-b", "Accounts: how do we sign teams in, and what does the owner decide first", state: "building", since: 50, cards: ["ov-506"], train: "train-91"),
                lane(27, "loose-a", "Daemon: a lane in landing with no train yet", state: "landing", since: 8, cards: ["ov-507"]),
                lane(28, "main-a", "Daemon: the collecting train leaves once main is green again", state: "landing", since: 5, cards: ["ov-508"], train: "train-94"),
            ]
            trains += [
                train(90, "People on a shared runner; sign in on the web", summary: "Relay: the team directory", lanes: [(21, "open-a")], phase: "waiting_for_previous_train", state: "integrating", since: 15),
                train(91, "Train 91 \u{00B7} Agents tell the orchestrator when they are done", summary: "iOS drafts; Mac sections; Accounts", lanes: [(22, "check-a"), (23, "check-b"), (26, "asks-b")], phase: "gating_locally", state: "gating", since: 30),
                train(92, "CI minutes", summary: "CI: the Rust jobs get 35 minutes", lanes: [(24, "ci-a")], phase: "waiting_for_ci", state: "pushed", since: 20, sha: "9a1c0d22"),
                train(93, "The Plan on every phone", summary: "Phones: the Plan in one inset list", lanes: [(25, "ready-a")], phase: "ready_to_land", state: "green", since: 10, sha: "4d3c8cb1"),
                train(94, "Main's checks first", summary: "Daemon: the collecting train waits for main", lanes: [(28, "main-a")], phase: "waiting_for_main", state: "integrating", since: 5),
            ]
        }
        if kind != .quiet && kind != .ladder {
            lanes += [
                lane(1, "team-dir", "Relay: the team directory, so each person's asks reach the orchestrator", state: "landing", since: 50, cards: ["ov-487"], train: "train-86"),
                lane(2, "ios-overwrite", "iOS: a draft is never overwritten by the daemon's echo while someone is typing in the composer", state: "fixing", since: 130, cards: ["ov-492", "ov-493"], train: "train-86", rounds: 2, stale: true),
                lane(3, "ci-green", "CI: the Rust jobs get 35 minutes on a slow runner", state: "landing", since: 20, cards: ["ov-301"], train: "train-88"),
                lane(4, "mac-plan", "Plan: the sections, glance band, train card, agents table and themes grid on the Mac", state: "review", since: 25, cards: ["ov-503"]),
                lane(5, "tail-tunnel", "Tailcat: the tunnel is the default transport and falls back to SSH only on request", state: "review", since: 190, cards: ["ov-485", "ov-486", "ov-488", "ov-489"], stale: true),
                lane(6, "site-ship", "Site: one web project, previewed before it ships", state: "building", since: 70, cards: ["ov-490"]),
                lane(7, "phones-plan", "Phones: the Plan on iPhone, iPad and Android in one inset list", state: "building", since: 12, cards: ["ov-504"]),
                lane(8, "old-one", "Relay: a lane that stayed queued on a train that has landed", state: "queued", since: 3000, cards: ["ov-474"], train: "train-83"),
                lane(9, "old-two", "Daemon: the second lane stranded on Train 83", state: "landing", since: 3100, cards: ["ov-482"], train: "train-83"),
            ]
            trains += [
                train(86, "Teams on a shared runner", summary: "Relay: the team directory, so each person's asks reach the orchestrator; iOS: a draft is never overwritten by the daemon's echo", lanes: [(1, "team-dir"), (2, "ios-overwrite")], phase: "waiting_for_lanes", state: "integrating", since: 130, held: [(2, "ios-overwrite")]),
                train(88, "Train 88", summary: "CI: the Rust jobs get 35 minutes", lanes: [(3, "ci-green")], phase: "ci_failed", state: "red", since: 12, sha: "3f78754f"),
            ]
            var landed = train(83, "Train 83", summary: "Daemon relays", lanes: [(8, "old-one"), (9, "old-two")], phase: "landed", state: "landed", since: 3000, sha: "9a1c0d22", agent: false)
            landed["landed_at"] = now - 3000 * minute
            trains.append(landed)
            agents += [
                agent("pane:orchestrator", "orchestrator", model: "opus", working: true, started: 240, tokens: 2_100_000, dollars: 14.2),
                agent("agent:b1", "build", model: "sonnet", lane: (6, "site-ship"), working: true, started: 22, tokens: 380_000, dollars: 1.1),
                agent("agent:b2", "build", model: "opus", lane: (7, "phones-plan"), working: true, started: 12, tokens: 160_000, dollars: 2.4),
                agent("agent:i1", "integrator", model: "opus", train: (88, "Train 88"), working: true, started: 40, tokens: 1_400_000, dollars: 6.3),
                // A subagent resumed for another round; the reviewer that finished (610K, $3.90) is not listed.
                agent("agent:s1", "subagent", model: "sonnet", label: "Design ov-503", working: true, started: 25, last: 4, tokens: 220_000, dollars: 0.6),
                // A pane that still runs between turns.
                agent("pane:idle", "agent", model: "opus", working: false, started: 300, last: 50, tokens: 90_000, dollars: 0.5),
            ]
        }
        if kind == .busy {
            lanes += [
                lane(10, "reps", "Reps: each person's asks reach the orchestrator, then skill-init writes the manager skill", state: "queued", cards: ["ov-494"], rank: 1, reason: "Frees the Mac slot"),
                lane(11, "skill-init", "Onboarding: skill-init explains what the manager skill is for, in the owner's words", state: "queued", cards: ["ov-495"], rank: 2, reason: "Needs reps first"),
                lane(12, "watch-glance", "Watch: the glance sentence on the complication", state: "queued", cards: ["ov-496"], rank: 3),
                lane(13, "unranked-a", "Android: tablets match the iPad layout", state: "queued", cards: ["ov-497"]),
                lane(14, "asks-you", "Accounts: how do we sign teams in", state: "queued", cards: ["ov-1"]),
            ]
            order = [id(10), id(11), id(12)]
        }
        plan["lanes"] = lanes
        plan["trains"] = trains
        plan["agents"] = agents
        // The day's spend: the listed agents' and the ones that finished or exited.
        // The ladder scenario has no agent alive and a day that spent something: the header says so.
        plan["agents_spend"] = agents.isEmpty && kind != .ladder ? NSNull() : spend(kind == .ladder ? 1_200_000 : 6_300_000, dollars: kind == .ladder ? 12 : 31.7)
        plan["order"] = order
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
            // Train 92 is running its checks; Train 93's passed, which is why it is ready to land.
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
