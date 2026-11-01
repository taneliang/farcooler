import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The plan C10f was drawn over (ov-867), on today's wire: real initiatives
/// with goals, next steps and asks; builders with what they do now; a stuck
/// ticket; a reviewer with more waiting; a landing of four tickets; and the
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

    /// A ticket on the line, titled as its card is.
    static func line(
        _ card: Int, _ title: String, stage: String, since: Int64 = 30, landing: Int? = nil, stale: Bool = false,
        fixing: String? = nil
    ) -> [String: Any] {
        PlanScenario.line(
            card, "ov-\(card)", title, stage: stage, since: since, landing: landing, stale: stale, fixing: fixing)
    }

    static func agent(
        _ id: String, _ role: String, tickets: [(Int, String)], doing: String, started: Int64
    ) -> [String: Any] {
        PlanScenario.agent(
            id, role, model: "sonnet", tickets: tickets, doing: doing, working: true, started: started, tokens: 100_000,
            dollars: nil)
    }

    /// Whether `plan` is this scenario as read.
    static func shown(_ plan: PlanModel) -> Bool { plan.themes.contains { $0.name == "Engineering health" } }

    static func data() throws -> Data {
        var plan = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        plan["now_ms"] = now
        plan["rulings"] = [] as [Any]
        plan["cost"] = NSNull()
        plan["ci"] = [] as [Any]
        plan["agents_spend"] = NSNull()
        plan["off_the_line"] = [] as [Any]
        plan["landed_not_closed"] = [] as [Any]
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
            card(868, "Builder, reviewer and lander agents replace the old plan model"),
            card(894, "Queued: the watch says what's landing", status: "To Do"),
        ]
        let titles: [Int: String] = Dictionary(
            uniqueKeysWithValues: (plan["cards"] as? [[String: Any]] ?? []).compactMap { card in
                (card["key"] as? String).flatMap { Int($0.dropFirst(3)) }.flatMap { number in (card["title"] as? String).map { (number, $0) } }
            })
        func t(_ card: Int, _ stage: String, since: Int64 = 30, landing: Int? = nil, stale: Bool = false, fixing: String? = nil) -> [String: Any] {
            line(card, titles[card] ?? "", stage: stage, since: since, landing: landing, stale: stale, fixing: fixing)
        }
        plan["lines"] = [
            t(869, "building", since: 30),
            t(870, "building", since: 23),
            t(881, "building", since: 2),
            t(882, "building", since: 2),
            t(885, "building", since: 23),
            t(880, "building", since: 150, stale: true, fixing: "agent:b5"),
            t(883, "building", since: 32),
            t(867, "building", since: 32),
            t(868, "building", since: 6),
            t(871, "reviewing", since: 8),
            t(876, "reviewing", since: 8),
            t(877, "reviewing", since: 40),
            t(884, "landing", since: 14, landing: 120),
            t(891, "landing", since: 14, landing: 120),
            t(892, "landing", since: 14, landing: 120),
            t(890, "landing", since: 14, landing: 120),
            t(894, "waiting", since: 5),
        ]
        plan["landings"] = [
            PlanScenario.landing(
                120, "Landing 120", summary: "People; terminals; fling; step-up", cards: [884, 891, 892, 890],
                phase: "gating_locally", state: "gating", since: 14, lander: "agent:i1")
        ]
        plan["agents"] = [
            PlanScenario.agent("pane:orchestrator", "orchestrator", model: "opus", working: true, started: 240, tokens: 2_100_000, dollars: nil),
            agent("agent:b1", "builder", tickets: [(869, "build")], doing: "Finding why a paired browser sticks on Starting", started: 30),
            agent("agent:b2", "builder", tickets: [(870, "build")], doing: "Grouping the web's panes by workspace", started: 23),
            agent("agent:b3", "builder", tickets: [(881, "build"), (882, "build")], doing: "Reading the chat list's scroll code", started: 2),
            agent("agent:b4", "builder", tickets: [(885, "build")], doing: "Keeping finished subagents in their turns", started: 23),
            agent("agent:b5", "builder", tickets: [(880, "fix")], doing: "Rerunning the Android unit tests", started: 47),
            agent("agent:b6", "builder", tickets: [(883, "build")], doing: "Moving the photo's X out of the hold area", started: 32),
            agent("agent:b7", "builder", tickets: [(867, "build")], doing: "Drawing the plan around agents and their tickets", started: 32),
            agent("agent:b8", "builder", tickets: [(868, "build")], doing: "Reading how the old plan is dispatched today", started: 6),
            agent("agent:r1", "reviewer", tickets: [(871, "review"), (876, "review")], doing: "Reading the diff for a failed web connection", started: 8),
            agent("agent:i1", "lander", tickets: [(884, "land"), (891, "land"), (892, "land"), (890, "land")], doing: "", started: 14),
        ]
        return try JSONSerialization.data(withJSONObject: plan)
    }
}
