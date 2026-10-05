import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Orchestrator pages on the Mac (ov-269 design 6.1, ov-284): a Pages section
/// in a planned board's navigator (ov-298), a page in the main area, anchored
/// pages inside their theme, Hide Page on this Mac only, and on a runner
/// without the plan a board that draws exactly as it did.
@MainActor
@Suite(.serialized)
struct PlanPagesTests {
    static let themeID = "00000000-0000-0000-0000-000000003001"

    /// Three pages: `train` as `test/fixtures/page.json` has it (anchored to a
    /// theme this plan doesn't have, so it's listed), `risks` anchored to the
    /// plan's Visual language, and `spend` of its own.
    static func pagesJSON() throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let train = try String(contentsOf: root.appendingPathComponent("test/fixtures/page.json"), encoding: .utf8)
        let risks = try String(contentsOf: root.appendingPathComponent("test/fixtures/pages/normalized/risks.json"), encoding: .utf8)
        let spend = try String(contentsOf: root.appendingPathComponent("test/fixtures/pages/normalized/spend.json"), encoding: .utf8)
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        func row(_ slot: String, _ title: String, anchor: String, doc: String) -> String {
            #"{"id":"p-\#(slot)","short":"\#(slot)","slot":"\#(slot)","title":"\#(title)","summary":"About \#(slot)","anchor_kind":"\#(anchor.isEmpty ? "" : "theme")","anchor":"\#(anchor)","revision":1,"ordinal":0,"actor":"manager","updated_at_ms":\#(now - 600_000),"doc":\#(doc)}"#
        }
        return Data(
            #"{"pages":[\#(train),\#(row("risks", "Risks", anchor: themeID, doc: risks)),\#(row("spend", "Spend", anchor: "", doc: spend))]}"#
                .utf8)
    }

    /// A board on a runner with the plan and, unless `pages` is false, pages.
    static func store(
        pages: Bool = true, defaults: UserDefaults, calls: PlanViewTests.Calls = PlanViewTests.Calls()
    ) async throws -> TaskBoardStore {
        let store = try await PlanViewTests.store(plan: true, defaults: defaults, calls: calls)
        let answer = store.client.commandRunnerForTesting
        let pagesData = try pagesJSON()
        store.client.commandRunnerForTesting = { args in
            if args.starts(with: ["page", "list"]) {
                calls.args.append(args)
                return (pagesData, nil)
            }
            return await answer!(args)
        }
        store.client.daemonBuild = DaemonBuild(
            version: "test", matches: true, platform: "macos",
            capabilities: Set(Capability.allCases.map(\.rawValue).filter { pages || $0 != "board_pages" }))
        return store
    }

    @Test("Pages lists pages of their own and those whose theme is gone; an anchored page draws in its theme")
    func listedAndAnchored() async throws {
        let store = try await Self.store(defaults: PlanViewTests.defaults())
        await store.plan.reload()
        #expect(store.plan.pages.map(\.slot) == ["train", "risks", "spend"])
        #expect(store.plan.listedPages.map(\.slot) == ["train", "spend"])
        #expect(store.plan.anchoredPages(to: Self.themeID).map(\.slot) == ["risks"])
        #expect(store.plan.title(.page("spend")) == "Spend")
    }

    @Test("A dropped theme's pages fall into the Pages section")
    func droppedThemeFallsBack() throws {
        let page = BoardPage(id: "p", slot: "risks", title: "Risks", anchorKind: "theme", anchor: Self.themeID)
        var plan = try PlanModel.decode(PlanViewTests.fixture())
        #expect(PlanStore.listed([page], plan: plan, hidden: []).isEmpty)
        plan.themes[0].state = "dropped"
        #expect(PlanStore.listed([page], plan: plan, hidden: []).map(\.slot) == ["risks"])
        #expect(PlanStore.anchored([page], to: Self.themeID, plan: plan, hidden: []).isEmpty)
        #expect(PlanStore.listed([page], plan: .empty, hidden: []).map(\.slot) == ["risks"], "the plan layer gone")
    }

    @Test("Hide Page is kept on this Mac, per board, and never reaches the runner")
    func hidePageIsLocal() async throws {
        let defaults = PlanViewTests.defaults()
        let calls = PlanViewTests.Calls()
        let store = try await Self.store(defaults: defaults, calls: calls)
        await store.plan.reload()
        let before = calls.args.count
        store.plan.hide("spend")
        store.plan.hide("risks")
        #expect(store.plan.listedPages.map(\.slot) == ["train"])
        #expect(store.plan.anchoredPages(to: Self.themeID).isEmpty)
        #expect(store.plan.hiddenCount == 2)
        #expect(calls.args.count == before, "hiding asked the runner: \(calls.args.suffix(2))")
        let again = PlanStore(client: store.client, workspace: store.workspace, host: store.plan.host, defaults: defaults)
        #expect(again.hiddenPages == ["spend", "risks"])
        let other = PlanStore(client: store.client, workspace: .implicit(repository: "other"), host: store.plan.host, defaults: defaults)
        #expect(other.hiddenPages.isEmpty)
        store.plan.showHiddenPages()
        #expect(PlanStore(client: store.client, workspace: store.workspace, host: store.plan.host, defaults: defaults).hiddenPages.isEmpty)
    }

    @Test("A planned board's navigator draws Pages after Themes; a row opens its page")
    func pagesSection() async throws {
        let defaults = PlanViewTests.defaults()
        let store = try await Self.store(defaults: defaults)
        let drawn = await PlanViewTests.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        for _ in 0..<20 where !drawn.ids.contains("plan-pages") { await drawn.settle() }
        #expect(drawn.ids.isSuperset(of: ["plan-pages", "plan-page-train", "plan-page-spend"]), "\(drawn.ids)")
        #expect(!drawn.ids.contains("plan-page-risks"), "an anchored page is drawn in its theme, not listed")
        let themes = try #require(drawn.seen.views["plan-themes"])
        let pages = try #require(drawn.seen.views["plan-pages"])
        #expect(pages.minY >= themes.maxY - 0.5, "Pages comes after Themes")
        #expect(drawn.press("plan-page-spend"))
        await drawn.settle()
        #expect(drawn.opened.last == .page("spend"))
    }

    @Test("A runner without board_pages draws no Pages section and is never asked for pages")
    func noPagesOnAnOldRunner() async throws {
        let defaults = PlanViewTests.defaults()
        let calls = PlanViewTests.Calls()
        let store = try await Self.store(pages: false, defaults: defaults, calls: calls)
        let drawn = await PlanViewTests.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        for _ in 0..<20 where !drawn.ids.contains("plan-themes") { await drawn.settle() }
        #expect(drawn.ids.contains("plan-themes"))
        #expect(!drawn.ids.contains("plan-pages"))
        #expect(!calls.args.contains { $0.first == "page" })
    }

    /// A board with pages and nothing else planned is planned: its pages
    /// are listed, and the task list becomes the index (ov-298).
    @Test("Pages alone make a board planned")
    func pagesAlonePlan() async throws {
        let defaults = PlanViewTests.defaults()
        let calls = PlanViewTests.Calls()
        calls.plan = try PlanViewTests.emptyPlan()
        let store = try await Self.store(defaults: defaults, calls: calls)
        let drawn = await PlanViewTests.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        for _ in 0..<20 where !drawn.ids.contains("plan-pages") { await drawn.settle() }
        #expect(store.plan.planned)
        #expect(drawn.ids.isSuperset(of: ["plan-pages", "plan-page-spend"]))
        #expect(!drawn.ids.contains("plan-themes"))
    }

    /// What `PlanPageView` draws for `page`, by identifier.
    static func drawnPage(_ store: TaskBoardStore, _ page: PlanPage) async -> Set<String> {
        let seen = NavigatorFilterTests.Seen()
        let context = PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })
        let host = NSHostingView(
            rootView: PlanPageView(plan: store.plan, page: page, context: context)
                .frame(width: 800, height: 2400)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    let _ = seen.views = Dictionary(probed.map { ($0.id, .zero) }, uniquingKeysWith: { a, _ in a })
                    Color.clear
                })
        host.frame = CGRect(x: 0, y: 0, width: 800, height: 2400)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return Set(seen.views.keys)
    }

    @Test("A theme's page draws its anchored pages; a page opens in the main area; one gone says Page Not Found")
    func pagesInTheMainArea() async throws {
        let store = try await Self.store(defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let theme = await Self.drawnPage(store, .theme(Self.themeID))
        #expect(theme.isSuperset(of: ["plan-theme-page", "plan-anchored-risks", "plan-anchored-open-risks"]), "\(theme)")
        #expect(!theme.contains("plan-anchored-spend"))
        let page = await Self.drawnPage(store, .page("spend"))
        #expect(page.contains("plan-orchestrator-page") && !page.contains("plan-page-not-found"), "\(page)")
        let gone = await Self.drawnPage(store, .page("retired"))
        #expect(gone.contains("plan-page-not-found") && !gone.contains("plan-page-reading"), "\(gone)")
    }

    @Test("A read cancelled while it asks for pages isn't a read: the next view to ask reads them")
    func aCancelledReadReadsAgain() async throws {
        let store = try await Self.store(defaults: PlanViewTests.defaults())
        let answer = store.client.commandRunnerForTesting
        store.client.commandRunnerForTesting = { args in
            if args.starts(with: ["page", "list"]) {
                try? await Task.sleep(for: .seconds(2))
                if Task.isCancelled { return (nil, "Cancelled.") }
            }
            return await answer!(args)
        }
        let first = Task { await store.plan.readIfNeverRead() }
        try await Task.sleep(for: .milliseconds(300))
        first.cancel()
        await first.value
        #expect(store.plan.pages.isEmpty && !store.plan.hasRead)
        store.client.commandRunnerForTesting = answer
        await store.plan.readIfNeverRead()
        #expect(store.plan.pages.map(\.slot) == ["train", "risks", "spend"])
    }

    @Test("A waiting question opens in Needs You at its own item; an answered one, or another reference, doesn't")
    func askOpensNeedsYou() {
        func item(_ id: String, task: String, kind: NeedsYouKind) -> NeedsYouItem {
            NeedsYouItem(
                id: id, kind: kind, rank: 1, since: nil, task: NeedsYouTask(id: task, key: "ov-1", title: "T", status: "needs_decision"),
                question: "Which?", runner: "local")
        }
        let items = [item("review:t-2", task: "t-2", kind: .review), item("decision:t-1", task: "t-1", kind: .decision)]
        #expect(ContentView.askItem(.ask("t-1"), in: items)?.itemID == "decision:t-1")
        #expect(ContentView.askItem(.ask("t-9"), in: items) == nil, "answered: not in Needs You")
        #expect(ContentView.askItem(.task("t-1"), in: items) == nil, "a card reference opens the card")
        #expect(ContentView.askItem(.ask("t-2"), in: items) == nil, "a review isn't the question")
    }

    @Test("A hidden slot the runner no longer has stops being hidden")
    func hiddenSlotsArePruned() async throws {
        let store = try await Self.store(defaults: PlanViewTests.defaults())
        store.plan.hide("spend")
        store.plan.hide("retired")
        await store.plan.reload()
        #expect(store.plan.hiddenPages == ["spend"])
    }

    @Test("A pages read that fails says so, with Try Again, rather than Page Not Found")
    func aFailedReadSaysSo() async throws {
        let store = try await Self.store(defaults: PlanViewTests.defaults())
        let answer = store.client.commandRunnerForTesting
        store.client.commandRunnerForTesting = { args in
            if args.starts(with: ["page", "list"]) { return (nil, "error: the runner went away") }
            return await answer!(args)
        }
        await store.plan.reload()
        #expect(store.plan.pagesTrouble == "Far Cooler couldn’t read this board’s pages.")
        let drawn = await Self.drawnPage(store, .page("spend"))
        #expect(drawn.contains("plan-pages-unavailable") && !drawn.contains("plan-page-not-found"), "\(drawn)")
        store.client.commandRunnerForTesting = answer
        await store.plan.reload()
        #expect(store.plan.pagesTrouble == nil && store.plan.page("spend") != nil)
    }

    @Test("A page is kept across a relaunch, and its crumb is its title")
    func pageKept() {
        let page = ContentView.Selection.workspace(host: "h", workspace: "w", focus: .plan(.page("train")))
        #expect(SelectionMemory.encode(page) == "h|w|plan:page:train")
        #expect(SelectionMemory.decode("h|w|plan:page:train") == page)
        #expect(SelectionMemory.decode("h|w|plan:page:") == nil)
    }

    @Test("A page line from the runner moves that board's plan, as a plan line does")
    func pageLineRereads() {
        var heard: [String] = []
        EventStream.dispatch(
            Data(#"{"kind":"pages","workspace":"ws-1","slot":"train","revision":3,"actor":"manager","removed":false}"#.utf8),
            decoder: JSONDecoder(), onPlan: { heard.append($0) })
        #expect(heard == ["ws-1"])
    }

    @Test("A reference opens what the app opens: a card or its question, a page, a terminal by its name")
    func destinations() {
        let worktree = Worktree(
            id: "wt-1", short: "wt1", task: "integ-10", branch: "integ-10", repository: "r", host: "", path: "/tmp/integ-10",
            state: "active",
            terminals: [Terminal(id: "t-build", short: "tb", title: "build", preset: "zsh", state: "running", epoch: 0)],
            repositoryID: "r", workspace: "w")
        func go(_ d: PageDestination) -> ContentView.Selection? {
            ContentView.pageSelection(d, host: "h", workspace: "w", worktrees: [worktree])
        }
        // A question no longer in Needs You opens its task.
        #expect(go(.ask("t-1")) == .workspace(host: "h", workspace: "w", focus: .task("t-1")))
        #expect(go(.page("spend")) == .workspace(host: "h", workspace: "w", focus: .plan(.page("spend"))))
        #expect(go(.lane("l-1")) == .workspace(host: "h", workspace: "w", focus: .plan(.lane("l-1"))))
        #expect(go(.terminal(worktree: "wt-1", name: "build")) == .workspace(host: "h", workspace: "w", focus: .worktree("wt-1", terminal: "t-build")))
        #expect(go(.url(URL(string: "https://github.com")!)) == nil, "the system opens the web, not the window")

        let context = PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in }, worktrees: [worktree])
        let plan = PlanStore(client: DaemonClient(target: "", notifications: NotificationCenter()), workspace: .implicit(repository: "r"), host: "h")
        let world = context.world(plan)
        #expect(world.resolve(PageRef(.terminal(worktree: "integ-10", name: "build"))).destination == .terminal(worktree: "wt-1", name: "build"))
        #expect(world.resolve(PageRef(.worktree("integ-10"))).destination == .worktree("wt-1"))
        #expect(world.plan == nil, "a plan never read draws lanes and themes as plain text")
    }
}
