import Foundation
import Testing

@testable import AgentKit

/// Live data on a page (ov-306): CI by main, SHA or run, the board's card
/// counts, and a lane's or a theme's spend, drawn from the plan the app holds,
/// so a figure the board or the runner knows is never typed in. Read from
/// `test/fixtures/pages/normalized/live.json`, what the runner stores.
struct PageLiveDataTests {
    static func world(plan: Bool = true) throws -> PageWorld {
        var mac = PlanModelTests.lane("mac-ux", "review", cards: ["t1"], train: "integ-13b")
        mac["spend"] = ["input_tokens": 400_000, "output_tokens": 70_000, "runs": 2]
        var theme = PlanModelTests.theme("Visual language", cards: ["t1"], ordinal: 0)
        theme["counts"] = ["backlog": 7, "todo": 0, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": 3, "cancelled": 0]
        theme["spend"] = ["input_tokens": 1_200_000, "runs": 5]
        let object: [String: Any] = [
            "now_ms": PlanModelTests.now, "themes": [theme], "lanes": [mac], "order": [], "cards": [],
            "ci": [
                [
                    "subject": "main", "sha": "898ae57d485040fbd9872379c3c12ca95202698a", "status": "running",
                    "url": "https://github.com/o/r/actions/runs/1", "fetched_at": 0, "changed_at": 0,
                    "jobs": [["name": "CI / Rust", "state": "running", "url": ""], ["name": "CI / iOS", "state": "passed", "url": ""],
                             ["name": "Canary", "state": "passed", "url": ""]],
                ],
                [
                    "subject": "sha:c85bf83dce46a6b71d7312afc623899ae7914658", "sha": "c85bf83dce46a6b71d7312afc623899ae7914658",
                    "status": "failed", "url": "https://github.com/o/r/actions/runs/37275435256", "fetched_at": 0, "changed_at": 0,
                    "jobs": [["name": "CI / Swift", "state": "failed", "url": ""], ["name": "CI / Android", "state": "passed", "url": ""]],
                ],
            ],
            "board_counts": ["backlog": 4, "todo": 0, "needs_decision": 0, "in_progress": 2, "in_review": 3, "done": 11, "cancelled": 0],
        ]
        let model = try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
        return PageWorld(plan: plan ? model : nil, nowMs: PlanModelTests.now)
    }

    static func live() throws -> PageDoc {
        try PageDoc.decode(Data(contentsOf: PageModelTests.root.appendingPathComponent("test/fixtures/pages/normalized/live.json")))
    }

    @Test("the live page decodes whole: figures with references and their fallbacks, CI and card targets")
    func decodes() throws {
        let doc = try Self.live()
        guard case .stats(let items) = doc.blocks[1] else {
            Issue.record("the second block isn't the figures")
            return
        }
        #expect(items.count == 6)
        // The runner gives a live figure the value and the reference the label
        // an older app draws (review train-1005c M4).
        #expect(items[0].ref == PageRef(.ci("main"), label: "Main") && items[0].value == "Main")
        #expect(items[1].ref == PageRef(.ci("c85bf83d"), label: "c85bf83d") && items[1].detail == "pushed at midnight")
        #expect(items[2].ref == PageRef(.cards("in_review"), label: "In review") && items[2].value == "In review")
        #expect(items[4].ref == PageRef(.theme("Visual language")) && items[4].show == .spend)
    }

    @Test("figures draw live: CI's status with its jobs, failed in amber; a count; a theme's and a lane's spend")
    func figuresAreLive() throws {
        let world = try Self.world()
        guard case .stats(let items) = try Self.live().blocks[1] else { return }
        let shown = items.map(world.statText)
        #expect(shown[0].value == "Running" && shown[0].detail == "2 of 3 jobs done" && shown[0].tone == .neutral)
        #expect(shown[1].value == "Failed" && shown[1].detail == "pushed at midnight" && shown[1].tone == .attention)
        #expect(shown[2].value == "3" && shown[3].value == "9")
        #expect(shown[4].value == "1.2M" && shown[5].value == "470K")
    }

    @Test("a CI reference opens its run, says its status, and is amber only when it failed")
    func ciResolves() throws {
        let world = try Self.world()
        let failed = world.resolve(PageRef(.ci("c85bf83d")))
        #expect(failed.name == "c85bf83d" && failed.status == "Failed · 1 of 2 jobs failed" && failed.statusTone == .attention)
        #expect(failed.destination == .url(URL(string: "https://github.com/o/r/actions/runs/37275435256")!))
        let main = world.resolve(PageRef(.ci("main"), label: "Main's CI"))
        #expect(main.name == "Main's CI" && main.statusTone == .neutral)
        #expect(world.cellText(PageCell(ref: PageRef(.ci("main")))) == "Running · 2 of 3 jobs done")
        #expect(world.cellText(PageCell(ref: PageRef(.cards("done")))) == "11")
        #expect(world.cellText(PageCell(ref: PageRef(.theme("Visual language")), show: .spend)) == "1.2M")
    }

    @Test("without the plan, or before the runner read it, each draws its name as plain text")
    func unreadIsPlainText() throws {
        let world = try Self.world(plan: false)
        let run = world.resolve(PageRef(.ci("run:37275435256")))
        #expect(run.name == "Run 37275435256" && run.status == nil && run.destination == nil)
        #expect(world.resolve(PageRef(.cards("in_review"))) == PageResolved(name: "In Review"))
        guard case .stats(let items) = try Self.live().blocks[1] else { return }
        #expect(world.statText(items[0]).value == "Main")
        #expect(try Self.world().resolve(PageRef(.ci("run:9"))).status == nil, "a run nobody read")
    }
}
