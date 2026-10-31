import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The plan C10f was drawn over (ov-867), on today's wire: real initiatives
/// with goals, next steps and asks; builders with what they do now; a stuck
/// lane; a reviewer with more waiting; a train landing six tickets; and the
/// board's counts. Long real titles throughout, so the column is judged with
/// what it will really say.
@MainActor
enum PlanColumnScenario {
    static let now = PlanScenario.now
    static let minute = PlanScenario.minute

    static func task(_ n: Int) -> String { PlanScenario.id(n, 1) }

    static func card(_ n: Int, _ title: String, status: String = "In Progress") -> [String: Any] {
        ["task": task(n), "key": "ov-\(n)", "title": title, "status": status]
    }

    static func theme(
        _ n: Int, _ name: String, goal: String, next: String = "", ask: String = "", done: Int, total: Int, cards: [Int]
    ) -> [String: Any] {
        var theme = PlanScenario.theme(
            n, name, counts: (done: done, progress: 0, backlog: total - done), cards: cards.map { "ov-\($0)" }, ask: ask)
        theme["outcome"] = goal
        theme["next"] = next
        return theme
    }

    static func lane(
        _ n: Int, _ name: String, state: String, cards: [Int], since: Int64 = 30, train: String? = nil, stale: Bool = false
    ) -> [String: Any] {
        PlanScenario.lane(n, name, "", state: state, since: since, cards: cards.map { "ov-\($0)" }, train: train, stale: stale)
    }

    static func agent(_ id: String, _ role: String, lane: Int? = nil, train: Int? = nil, label: String, started: Int64) -> [String: Any] {
        var agent = PlanScenario.agent(
            id, role, model: "sonnet", lane: lane.map { ($0, "lane-\($0)") }, train: train.map { ($0, "Train \($0)") }, label: label,
            working: true, started: started, tokens: 100_000, dollars: nil)
        agent["terminal"] = NSNull()
        return agent
    }

    /// Whether `plan` is this scenario as read.
    static func shown(_ plan: PlanModel) -> Bool { plan.themes.contains { $0.name == "Engineering health" } }

    static func data() throws -> Data {
        var plan = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        plan["now_ms"] = now
        plan["rulings"] = [] as [Any]
        plan["cost"] = NSNull()
        plan["ci"] = [] as [Any]
        plan["order"] = [] as [Any]
        plan["agents_spend"] = NSNull()
        plan["no_lane"] = [] as [Any]
        plan["landed_not_closed"] = [] as [Any]
        plan["lane_is_train"] = [] as [Any]
        plan["board_counts"] = [
            "backlog": 22, "todo": 10, "needs_decision": 1, "in_progress": 9, "in_review": 15, "done": 282, "cancelled": 6,
        ]
        plan["themes"] = [
            theme(1, "Web client", goal: "You and your teammates use Far Cooler fully from any browser",
                  next: "a shared computer signs in with a phone", done: 20, total: 29, cards: [869, 870, 871, 876]),
            theme(2, "Mobile experience", goal: "From the phone you check in on everything and make the calls only you can make",
                  ask: "the Play service account", done: 16, total: 24, cards: [880, 881, 882, 883, 884]),
            theme(3, "Team mode", goal: "Far Cooler works for teams that require PR review",
                  next: "you try per-person access on real devices", done: 55, total: 60, cards: [877, 890]),
            theme(4, "Engineering health", goal: "Main stays green and fast: CI under 20 minutes, no flakes",
                  next: "confirm tonight's nightly is green", done: 49, total: 51, cards: [892]),
            theme(5, "Plan v2", goal: "The plan stays current by itself and says more",
                  ask: "page actions and promotions wait on your direction", done: 15, total: 17, cards: [867, 868]),
            theme(6, "Trustworthy agent view", goal: "The agent view never freezes, never blanks, and matches the CLI",
                  done: 48, total: 52, cards: [885]),
            theme(7, "Mac look and feel", goal: "The Mac app feels like it was made by Apple", done: 33, total: 34, cards: [891]),
            theme(8, "Marketing", goal: "Someone who has never seen Far Cooler understands it in a minute", done: 6, total: 8, cards: []),
        ]
        plan["cards"] = [
            card(869, "An already-paired browser starts after an update"),
            card(870, "The web is organized by workspace, like the apps"),
            card(871, "A failed connection says why, and fails less", status: "In Review"),
            card(876, "Sign-in limits per visitor and account", status: "In Review"),
            card(880, "The Android app no longer crashes on a cold start after the Play update"),
            card(881, "Chat scrolling stutters now and then"),
            card(882, "The Task screen scrolls smoothly"),
            card(883, "Holding a photo doesn't hit its X"),
            card(884, "People is reachable on iPhone and Android", status: "In Review"),
            card(877, "A revoked device says it was removed", status: "In Review"),
            card(890, "Step-up on a Mac that isn't the runner", status: "In Review"),
            card(891, "A terminal doesn't garble when the app comes back", status: "In Review"),
            card(892, "A late tick loses no rows on a fling", status: "In Review"),
            card(885, "Finished subagents don't pile up at the end"),
            card(867, "The plan reads like an assembly line"),
            card(868, "Builder, reviewer and lander agents replace lanes"),
            card(894, "Queued: the watch says what's landing", status: "To Do"),
        ]
        plan["lanes"] = [
            lane(1, "web-paired", state: "building", cards: [869], since: 30),
            lane(2, "web-workspaces", state: "building", cards: [870], since: 23),
            lane(3, "ios-scroll", state: "building", cards: [881, 882], since: 2),
            lane(4, "agent-view", state: "building", cards: [885], since: 23),
            lane(5, "android-crash", state: "fixing", cards: [880], since: 150, stale: true),
            lane(6, "ios-photo", state: "building", cards: [883], since: 32),
            lane(7, "plan-c10", state: "building", cards: [867], since: 32),
            lane(8, "agents-model", state: "building", cards: [868], since: 6),
            lane(9, "web-errors", state: "review", cards: [871, 876], since: 8),
            lane(10, "team-revoke", state: "review", cards: [877], since: 40),
            lane(11, "people", state: "landing", cards: [884], since: 14, train: "train-120"),
            lane(12, "mac-terminal", state: "landing", cards: [891], since: 14, train: "train-120"),
            lane(13, "fling", state: "landing", cards: [892], since: 14, train: "train-120"),
            lane(14, "step-up", state: "landing", cards: [890], since: 14, train: "train-120"),
            lane(15, "watch", state: "queued", cards: [894], since: 5),
        ]
        var train = PlanScenario.train(
            120, "Train 120", summary: "People; terminals; fling; step-up",
            lanes: [(11, "people"), (12, "mac-terminal"), (13, "fling"), (14, "step-up")], phase: "gating_locally",
            state: "gating", since: 14)
        train["title"] = "Train 120"
        plan["trains"] = [train]
        plan["agents"] = [
            PlanScenario.agent("pane:orchestrator", "orchestrator", model: "opus", working: true, started: 240, tokens: 2_100_000, dollars: nil),
            agent("agent:b1", "build", lane: 1, label: "Finding why a paired browser sticks on Starting", started: 30),
            agent("agent:b2", "build", lane: 2, label: "Grouping the web's panes by workspace", started: 23),
            agent("agent:b3", "build", lane: 3, label: "Reading the chat list's scroll code", started: 2),
            agent("agent:b4", "build", lane: 4, label: "Keeping finished subagents in their turns", started: 23),
            agent("agent:b5", "fix", lane: 5, label: "Rerunning the Android unit tests", started: 47),
            agent("agent:b6", "build", lane: 6, label: "Moving the photo's X out of the hold area", started: 32),
            agent("agent:b7", "build", lane: 7, label: "Drawing the plan around agents and their tickets", started: 32),
            agent("agent:b8", "build", lane: 8, label: "Reading how lanes are dispatched today", started: 6),
            agent("agent:r1", "review", lane: 9, label: "Reading the diff for a failed web connection", started: 8),
            agent("agent:i1", "integrator", train: 120, label: "", started: 14),
        ]
        return try JSONSerialization.data(withJSONObject: plan)
    }
}
