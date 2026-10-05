import Foundation
import Testing

@testable import AgentKit

/// References draw live (ov-269 Q4), open only what the app opens (Q3), leave
/// for the web only over `https` with the domain shown (Q5), and a reference
/// to something gone is plain text (ov-284).
struct PageLiveTests {
    static let now: Int64 = PlanModelTests.now

    static func row(_ key: String, _ status: TaskStatus, title: String = "Title") -> TaskRow {
        TaskRow(id: "id-\(key)", key: key, title: title, status: status, statusSince: Date())
    }

    static func world(plan: Bool = true) throws -> PageWorld {
        var fixing = PlanModelTests.lane("ov-181-review", "fixing", cards: ["t181"], fixRounds: 1)
        fixing["spend"] = ["input_tokens": 300_000, "output_tokens": 170_000, "runs": 2, "unmeasured_agents": 0]
        var theme = PlanModelTests.theme("Visual language", cards: ["t1"], ordinal: 0)
        theme["counts"] = ["backlog": 2, "todo": 0, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": 3, "cancelled": 0]
        let model = try PlanModelTests.plan(
            themes: [theme], lanes: [PlanModelTests.lane("ov-274-phones", "review", cards: ["t274"], train: "integ-10"), fixing])
        return PageWorld(
            tasks: [row("ov-274", .needsDecision, title: "Plan on phones"), row("ov-222", .inProgress), row("ov-113", .inReview)],
            plan: plan ? model : nil,
            pages: [BoardPage(id: "p1", slot: "spend", title: "Spend")],
            worktrees: ["integ-10": "wt-1"],
            terminals: [PageWorld.terminalKey(worktree: "wt-1", name: "build")],
            nowMs: now)
    }

    @Test("a card draws its key, opens the task, and says its title and live status")
    func aTaskIsLive() throws {
        let r = try Self.world().resolve(PageRef(.task("OV-113")))
        #expect(r == PageResolved(name: "ov-113", status: "In Review", destination: .task("id-ov-113"), spoken: "ov-113, Title, In Review"))
    }

    @Test("an open question reads Needs you in amber and opens it; an answered one reads Answered and opens the task")
    func anAskFollowsTheCard() throws {
        var world = try Self.world()
        let open = world.resolve(PageRef(.ask("ov-274")))
        #expect(open.status == "Needs you" && open.statusTone == .attention && open.destination == .ask("id-ov-274"))
        world.tasks["ov-274"]?.status = .inProgress
        let answered = world.resolve(PageRef(.ask("ov-274")))
        #expect(answered.status == "Answered" && answered.statusTone == .neutral && answered.destination == .task("id-ov-274"))
    }

    @Test("a lane draws its current state words, a theme its progress; a label renames either")
    func lanesAndThemesAreLive() throws {
        let world = try Self.world()
        let lane = world.resolve(PageRef(.lane("ov-181-review")))
        #expect(lane.name == "ov-181-review" && lane.status == "Fixing · round 1" && lane.destination == .lane("lane-ov-181-review"))
        #expect(world.cellText(PageCell(ref: PageRef(.lane("ov-274-phones")), show: .state)) == "In Review · train integ-10")
        #expect(world.cellText(PageCell(ref: PageRef(.lane("ov-181-review")), show: .spend)) == "470K")
        let theme = world.resolve(PageRef(.theme("Visual language"), label: "Theme"))
        #expect(theme.name == "Theme" && theme.status == "3 of 5 done" && theme.destination == .theme("theme-Visual language"))
    }

    @Test("a page, a worktree and a terminal resolve by slot and name")
    func placesResolve() throws {
        let world = try Self.world()
        #expect(world.resolve(PageRef(.page("spend"))) == PageResolved(name: "Spend", destination: .page("spend")))
        #expect(world.resolve(PageRef(.worktree("integ-10"))).destination == .worktree("wt-1"))
        let terminal = world.resolve(PageRef(.terminal(worktree: "integ-10", name: "build"), label: "Build terminal"))
        #expect(terminal.name == "Build terminal" && terminal.destination == .terminal(worktree: "wt-1", name: "build"))
    }

    @Test("a reference to something gone draws its label or its name as plain text, with nowhere to go")
    func goneIsPlainText() throws {
        let world = try Self.world()
        let gone: [(PageRef, String)] = [
            (PageRef(.lane("dropped-lane")), "dropped-lane"),
            (PageRef(.task("zz-9"), label: "Old card"), "Old card"),
            (PageRef(.ask("zz-9")), "zz-9"),
            (PageRef(.theme("Gone")), "Gone"),
            (PageRef(.page("nope")), "nope"),
            (PageRef(.worktree("w")), "w"),
            (PageRef(.terminal(worktree: "integ-10", name: "other")), "integ-10/other"),
            (PageRef(.unknown, label: "W"), "W"),
        ]
        for (ref, name) in gone {
            let r = world.resolve(ref)
            #expect(r.name == name && r.destination == nil && r.status == nil, "\(ref)")
        }
        // A cell showing a gone lane's state draws its name instead.
        #expect(world.cellText(PageCell(ref: PageRef(.lane("dropped-lane")), show: .state)) == "dropped-lane")
    }

    @Test("without the plan layer, lane and theme references are plain text; cards still resolve")
    func withoutThePlanLayer() throws {
        let world = try Self.world(plan: false)
        #expect(world.resolve(PageRef(.lane("ov-181-review"))).destination == nil)
        #expect(world.resolve(PageRef(.theme("Visual language"))).destination == nil)
        #expect(world.cellText(PageCell(ref: PageRef(.lane("ov-181-review")), show: .spend)) == "ov-181-review")
        #expect(world.resolve(PageRef(.task("ov-113"))).destination == .task("id-ov-113"))
    }

    @Test("an https link opens with its domain beside its label; any other link is plain text")
    func onlyHTTPSLeaves() throws {
        let world = try Self.world()
        let labeled = world.resolve(PageRef(.url("https://github.com/example/overnight/pull/12"), label: "Pull request"))
        #expect(labeled.name == "Pull request" && labeled.status == "github.com")
        #expect(labeled.spoken == "Pull request, link to github.com")
        #expect(labeled.destination == .url(URL(string: "https://github.com/example/overnight/pull/12")!))
        #expect(world.resolve(PageRef(.url("https://example.com/path?q=1#top"))).name == "example.com")
        #expect(world.resolve(PageRef(.url("https://xn--pple-43d.com/x"))).name == "xn--pple-43d.com", "punycode shows as itself")
        for raw in ["https://\u{0430}pple.com/x", "http://github.com/x", "javascript:alert(1)", "file:///etc/passwd", "https://user:pw@github.com/x", "mailto:a@b.c"] {
            #expect(world.resolve(PageRef(.url(raw), label: "L")).destination == nil, "\(raw)")
        }
    }

    @Test("Updated, and past the page's limit, Not updated for")
    func updatedWords() {
        var page = BoardPage(id: "p", slot: "s", title: "T", updatedAtMs: Self.now - 12 * 60_000)
        #expect(PageWords.updated(page, now: Self.now) == "Updated 12 min ago")
        page.doc = PageDoc(title: "T", staleAfterMin: 120, blocks: [])
        #expect(PageWords.updated(page, now: Self.now) == "Updated 12 min ago")
        page.updatedAtMs = Self.now - 3 * 3_600_000
        #expect(PageWords.updated(page, now: Self.now) == "Not updated for 3 hours")
        #expect(PageWords.isStale(page, now: Self.now))
    }

    @Test("a table cell with a web link shows its domain after its label; one that is its domain doesn't repeat it")
    func cellLinksShowTheirDomain() throws {
        let world = try Self.world()
        let labeled = world.parts(PageCell(ref: PageRef(.url("https://evil.example/x"), label: "Docs")))
        #expect(labeled.text == "Docs" && labeled.domain == "evil.example")
        #expect(labeled.destination == .url(URL(string: "https://evil.example/x")!))
        let bare = world.parts(PageCell(ref: PageRef(.url("https://example.com/a"))))
        #expect(bare.text == "example.com" && bare.domain == nil)
        let overText = world.parts(PageCell(text: "the run", ref: PageRef(.url("https://ci.example/812"))))
        #expect(overText.text == "the run" && overText.domain == "ci.example")
        #expect(world.parts(PageCell(ref: PageRef(.url("http://evil.example/x"), label: "Docs"))).destination == nil)
    }

    @Test("text over a reference links it; live state and tokens are words")
    func textOverAReferenceIsALink() throws {
        let world = try Self.world()
        #expect(world.parts(PageCell(text: "the card", ref: PageRef(.task("ov-113")))).destination == .task("id-ov-113"))
        #expect(world.parts(PageCell(ref: PageRef(.lane("ov-181-review")), show: .state)).destination == nil)
        #expect(world.parts(PageCell(text: "plain")).destination == nil)
    }

    @Test("a row offers one Open action per link, every column")
    func rowActions() throws {
        let world = try Self.world()
        let row = [
            PageCell(ref: PageRef(.lane("ov-274-phones"))), PageCell(ref: PageRef(.task("ov-274"))), PageCell(text: "gate"),
            PageCell(ref: PageRef(.lane("ov-274-phones")), show: .state), PageCell(ref: PageRef(.lane("gone"))),
        ]
        #expect(world.actions(row).map(\.name) == ["Open ov-274-phones", "Open ov-274"])
    }
}
