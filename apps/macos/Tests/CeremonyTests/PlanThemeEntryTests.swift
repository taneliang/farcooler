import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A theme in the canvas (ov-331): a short brief, not the sidebar's row. Its
/// outcome wraps to three lines (the owner's ruling, ov-273), what it says
/// follows `PlanThemeBrief`, folding it is kept per theme, and the canvas reads
/// at 680 pt however wide the window is.
@MainActor
@Suite(.serialized)
struct PlanThemeEntryTests {
    /// What each probed view drew, in a column `width` wide.
    static func frames(_ view: some View, width: CGFloat, height: CGFloat = 1200) async -> [String: CGRect] {
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: view
                .frame(width: width, alignment: .topLeading)
                .fixedSize(horizontal: false, vertical: true)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                })
        host.frame = CGRect(x: 0, y: 0, width: width, height: height)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.views
    }

    /// One line of the outcome's type, measured here, in this environment.
    static func line(width: CGFloat) async throws -> CGFloat {
        let one = await frames(
            Text("Outcome").font(PlanThemeEntry.secondaryFont).lineLimit(1).probed("line"), width: width)
        return try #require(one["line"]).height
    }

    static func model(_ edit: (inout [String: Any], inout [String: Any]) -> Void = { _, _ in }) throws -> PlanModel {
        try PlanModel.decode(try PlanCostViewTests.plan(edit))
    }

    static func entry(
        _ model: PlanModel, expanded: Bool = true, activity: [String: Int64] = [:]
    ) throws -> PlanThemeEntry {
        PlanThemeEntry(
            theme: try #require(model.themes.first), plan: model, activity: activity, expanded: expanded, selected: false,
            keyed: false, onToggle: { _ in }, onOpen: { _ in })
    }

    // MARK: The outcome

    @Test("A long outcome shows three lines in the entry at a narrow canvas")
    func threeLines() async throws {
        let long = String(repeating: "Every board shows its plan beside its tasks, legible on its own. ", count: 4)
        let model = try Self.model { _, theme in theme["outcome"] = long }
        let drawn = await Self.frames(try Self.entry(model), width: 260)
        let outcome = try #require(drawn["plan-theme-entry-Visual language-outcome"], "\(drawn.keys.sorted())")
        let lines = outcome.height / (try await Self.line(width: 260))
        #expect(lines > 2.5 && lines < 3.5, "the outcome drew \(lines) lines")
    }

    @Test("A short outcome takes the lines it needs")
    func shortOutcome() async throws {
        let model = try Self.model { _, theme in theme["outcome"] = "Plan beside tasks." }
        let drawn = await Self.frames(try Self.entry(model), width: 260)
        let outcome = try #require(drawn["plan-theme-entry-Visual language-outcome"])
        let lines = outcome.height / (try await Self.line(width: 260))
        #expect(lines > 0.5 && lines < 1.5, "the outcome drew \(lines) lines")
    }

    // MARK: What it says

    static let name = "Visual language"
    static func id(_ part: String) -> String { "plan-theme-entry-\(name)-\(part)" }

    @Test("Open, it says the outcome, the bar, the track, the story and its age, the ask, Next, the lanes and the ruling, in that order")
    func openComposition() async throws {
        let drawn = await Self.frames(try Self.entry(try Self.model()), width: 640)
        for part in ["head", "outcome", "story", "story-age", "next", "ruling-R-2", "lane-mac-ux", "landed", "over-budget"] {
            #expect(drawn[Self.id(part)] != nil, "\(part) is drawn: \(drawn.keys.sorted())")
        }
        #expect(drawn["plan-ask"] != nil, "the owner's ask")
        let parts = ["head", "outcome", "over-budget", "story", "story-age", "next", "lane-mac-ux", "landed", "ruling-R-2"]
        let order = parts.compactMap { drawn[Self.id($0)]?.minY }
        #expect(order.count == parts.count, "every part is drawn")
        #expect(order == order.sorted(), "top to bottom: \(order)")
        let ask = try #require(drawn["plan-ask"])
        let story = try #require(drawn[Self.id("story-age")])
        let next = try #require(drawn[Self.id("next")])
        #expect(story.maxY <= ask.minY && ask.maxY <= next.minY + 1, "story, then the ask, then Next")
    }

    @Test("Folded, it keeps its head, its track and one line of outcome, and the rest is gone")
    func folded() async throws {
        let model = try Self.model()
        let drawn = await Self.frames(try Self.entry(model, expanded: false), width: 640)
        for part in ["head", "outcome", "over-budget"] { #expect(drawn[Self.id(part)] != nil, "\(part) stays") }
        for part in ["story", "story-age", "next", "lane-mac-ux", "landed", "ruling-R-2"] {
            #expect(drawn[Self.id(part)] == nil, "\(part) folds away")
        }
    }

    @Test("A line with nothing to say takes no space: no story, no ask, no Next, no lane, no ruling")
    func emptyLines() async throws {
        let bare = try Self.model { object, theme in
            theme["story"] = ""
            theme["owner_ask"] = ""
            theme["next"] = ""
            theme["budget_tokens"] = NSNull()
            object["lanes"] = []
            object["rulings"] = []
        }
        let drawn = await Self.frames(try Self.entry(bare), width: 640)
        for part in ["story", "story-age", "next", "lane-mac-ux", "ruling-R-2", "landed"] {
            #expect(drawn[Self.id(part)] == nil, "\(part) is not drawn")
        }
        #expect(drawn["plan-ask"] == nil)
        #expect(drawn[Self.id("track")] != nil, "the track line still says something")
    }

    @Test("Amber is for an over-budget track and an ask; a moving lane's track line is neutral")
    func amberIsRare() async throws {
        let over = await Self.frames(try Self.entry(try Self.model()), width: 640)
        #expect(over[Self.id("over-budget")] != nil && over[Self.id("track")] == nil)
        let moving = try Self.model { _, theme in theme["budget_tokens"] = NSNull() }
        let drawn = await Self.frames(try Self.entry(moving), width: 640)
        #expect(drawn[Self.id("track")] != nil && drawn[Self.id("over-budget")] == nil, "\(drawn.keys.sorted())")
        let brief = PlanThemeBrief(moving.themes[0], in: moving)
        #expect(PlanWords.track(brief.track, now: moving.nowMs) == "mac-ux is in review")
    }

    @Test("A narrow canvas wraps the entry: nothing draws past its edges")
    func narrow() async throws {
        let long = String(repeating: "A long sentence that has to wrap. ", count: 12)
        let model = try Self.model { _, theme in
            theme["story"] = long
            theme["next"] = long
            theme["owner_ask"] = long
        }
        for width: CGFloat in [300, 440] {
            let drawn = await Self.frames(try Self.entry(model), width: width)
            #expect(drawn.count > 5)
            for (id, frame) in drawn where id.hasPrefix("plan-theme-entry") || id == "plan-ask" {
                #expect(frame.minX >= -0.5 && frame.maxX <= width + 0.5, "\(id) \(frame) in \(width)")
            }
            let story = try #require(drawn[Self.id("story")])
            let line = try await Self.line(width: width)
            let lines = story.height / line
            #expect(lines > 3.5 && lines < 4.5, "the story is clamped to exactly four lines: \(lines)")
        }
    }

    // MARK: In the canvas

    @Test("The canvas draws the entry, never the sidebar's row type")
    func canvasDrawsEntries() async throws {
        let drawn = try await PlanCostViewTests.drawn(PlanViewTests.fixture())
        #expect(drawn[Self.id("head")] != nil, "the canvas's entry: \(drawn.keys.sorted())")
        // The navigator's `PlanThemeRow` marks its progress `plan-theme-<name>-progress` and
        // itself `plan-theme-<name>`; the canvas draws neither.
        #expect(drawn["plan-theme-Visual language-progress"] == nil, "no sidebar row")
        #expect(drawn["plan-theme-Visual language"] == nil, "no sidebar row")
    }

    @Test("Themes in the canvas keep the board's order, active ones first, and paused and done fold into one row")
    func orderAndFold() async throws {
        let data = try PlanCostViewTests.edit { object in
            var themes = try #require(object["themes"] as? [[String: Any]])
            var second = themes[0]
            second["id"] = "00000000-0000-0000-0000-000000003002"
            second["name"] = "Alpha"
            second["ordinal"] = 5
            var paused = themes[0]
            paused["id"] = "00000000-0000-0000-0000-000000003003"
            paused["name"] = "Beta"
            paused["state"] = "paused"
            paused["ordinal"] = 1
            themes.append(contentsOf: [paused, second])
            object["themes"] = themes
        }
        let drawn = try await PlanCostViewTests.drawn(data)
        let first = try #require(drawn["plan-theme-entry-Visual language-head"])
        let alpha = try #require(drawn["plan-theme-entry-Alpha-head"], "\(drawn.keys.sorted())")
        #expect(first.minY < alpha.minY, "the board's order, not alphabetical: Visual language, then Alpha")
        #expect(drawn["plan-theme-entry-Beta-head"] == nil, "a paused theme is inside the closed fold")
        let fold = try #require(drawn["section-header-plan.themes.closed"], "one row for paused and done")
        #expect(fold.minY > alpha.maxY, "after the active themes")
    }

    @Test("An entry's fold is kept per theme and per board, open unless closed, and a paused theme starts closed")
    func foldIsKept() throws {
        let defaults = PlanViewTests.defaults()
        let model = try Self.model()
        var theme = try #require(model.themes.first)
        func open(_ t: PlanTheme, _ board: String = "ws") -> Bool {
            PlanThemeFold.isExpanded(theme: t, host: "local", workspace: board, in: defaults)
        }
        #expect(open(theme), "open by default")
        PlanThemeFold.set(false, theme: theme.id, host: "local", workspace: "ws", in: defaults)
        #expect(!open(theme) && open(theme, "other"), "closed here, still open on another board")
        var other = theme
        other.id = "someone-else"
        #expect(open(other), "and other themes are unaffected")
        theme.state = "paused"
        PlanThemeFold.set(true, theme: theme.id, host: "local", workspace: "ws", in: defaults)
        #expect(open(theme), "opened by the owner")
        var done = other
        done.state = "done"
        #expect(!open(done), "a done theme starts closed")
        #expect(PlanThemeFold.key(theme: "t", host: "h", workspace: "w") == "board.plan.theme.t.h.w")
    }

    @Test("A theme the owner closed draws folded in the overview, and one left alone draws open")
    func overviewHonorsTheFold() async throws {
        let calls = PlanViewTests.Calls()
        let defaults = PlanViewTests.defaults()
        let store = try await PlanViewTests.store(plan: true, defaults: defaults, calls: calls)
        await store.plan.reload()
        func drawn() async -> [String: CGRect] {
            await Self.frames(
                PlanOverviewView(plan: store.plan, statuses: store.board.statuses, selected: nil, onOpen: { _ in }, defaults: defaults),
                width: 640, height: 1600)
        }
        let open = await drawn()
        #expect(open[Self.id("story")] != nil)
        let theme = try #require(store.plan.plan.themes.first)
        PlanThemeFold.set(false, theme: theme.id, host: store.plan.host, workspace: store.plan.workspace.id, in: defaults)
        let closed = await drawn()
        #expect(closed[Self.id("story")] == nil && closed[Self.id("head")] != nil, "folded: \(closed.keys.sorted())")
    }

    @Test("The canvas reads at 680 pt on a wide window, leading-aligned, and wraps below that")
    func readingWidth() async throws {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let board = store
        func overview(canvas: CGFloat) async throws -> CGRect {
            let drawn = await Self.frames(
                PlanHome(board: board, needsYou: PlanNeedsYou(), onOpen: { _ in }).frame(height: 1200),
                width: canvas, height: 1200)
            return try #require(drawn["plan-overview"], "\(drawn.keys.sorted())")
        }
        let wide = try await overview(canvas: 1500)
        #expect(abs(wide.width - PlanMetrics.readingWidth) < 0.5, "680 pt wide on a 1,500 pt canvas, not \(wide.width)")
        let narrow = try await overview(canvas: 460)
        #expect(narrow.width < 460 && narrow.width > 300, "below 680 it takes the room it has, less its gutters: \(narrow.width)")
        #expect(wide.minX == narrow.minX, "leading-aligned at the same edge")
    }

    // MARK: The theme's page

    @Test("The theme's page reads in the entry's order, Where It Stands, Needs You, Next, and lists what was decided")
    func pageMatchesTheEntry() async throws {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let context = PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })
        let theme = try #require(store.plan.plan.themes.first)
        let drawn = await Self.frames(
            PlanPageView(plan: store.plan, page: .theme(theme.id), context: context), width: 800, height: 2400)
        let ask = try #require(drawn["plan-ask"], "\(drawn.keys.sorted())")
        let next = try #require(drawn["plan-theme-next"])
        let decided = try #require(drawn["plan-theme-decided"], "the theme's ruling R-2 is on its page")
        #expect(ask.minY < next.minY, "the ask comes before Next: \(ask) \(next)")
        #expect(next.minY < decided.minY, "Decided comes after")
    }

    // MARK: Around the entries

    @Test("The Themes summary is drawn when it has something to say, and not when only a done theme asks")
    func summaryAccessory() async throws {
        let asking = try await PlanCostViewTests.drawn(PlanViewTests.fixture())
        #expect(asking["plan-themes-summary"] != nil, "one active theme asks: \(asking.keys.sorted())")
        let done = try await PlanCostViewTests.drawn(PlanCostViewTests.plan { _, theme in theme["state"] = "done" })
        #expect(done["plan-themes-summary"] == nil, "a done theme's ask isn't counted: \(done.keys.sorted())")
    }

    @Test("The footer counts what's outside every theme, and lists the cards to tidy")
    func outsideFooter() async throws {
        let none = try await PlanCostViewTests.drawn(PlanViewTests.fixture())
        #expect(none["plan-outside"] == nil && none["plan-outside-tidy"] == nil, "every card is in a theme")
        let outside = try await PlanCostViewTests.drawn(
            PlanCostViewTests.edit { object in
                var themes = try #require(object["themes"] as? [[String: Any]])
                themes[0]["cards"] = []
                object["themes"] = themes
                object["no_lane"] = [["task": PlanViewTests.tasks[0].0, "key": "ov-1", "status": "in_review"]]
            })
        #expect(outside["plan-outside"] != nil, "open cards in no theme: \(outside.keys.sorted())")
        #expect(outside["plan-outside-tidy"] != nil, "and a cards-to-tidy button")
        let cards = [PlanFlaggedCard(task: "a", key: "ov-1", status: "in_review"), PlanFlaggedCard(task: "b", key: "ov-2", status: "in_progress")]
        let list = await Self.frames(PlanTidyList(cards: cards), width: 300)
        #expect(list["plan-tidy-ov-1"] != nil && list["plan-tidy-ov-2"] != nil, "each card is listed: \(list.keys.sorted())")
        let tidy = try #require(list["plan-tidy-ov-1"]).minY < (try #require(list["plan-tidy-ov-2"])).minY
        #expect(tidy, "in the CLI's order")
    }

    @Test("Option-click on a chevron folds every theme to match it; a plain click folds one")
    func optionClickFoldsAll() throws {
        let defaults = PlanViewTests.defaults()
        var themes = [try #require(try Self.model().themes.first)]
        for name in ["B", "C"] {
            var more = themes[0]
            more.id = "id-\(name)"
            more.name = name
            themes.append(more)
        }
        func open(_ t: PlanTheme) -> Bool { PlanThemeFold.isExpanded(theme: t, host: "h", workspace: "w", in: defaults) }
        let closed = PlanThemeFold.toggle(themes[0], all: false, among: themes, host: "h", workspace: "w", in: defaults)
        #expect(!closed && !open(themes[0]) && open(themes[1]) && open(themes[2]), "a plain click folds one")
        let all = PlanThemeFold.toggle(themes[1], all: true, among: themes, host: "h", workspace: "w", in: defaults)
        #expect(!all && themes.allSatisfy { !open($0) }, "option folds all to match the one clicked")
        PlanThemeFold.toggle(themes[2], all: true, among: themes, host: "h", workspace: "w", in: defaults)
        #expect(themes.allSatisfy(open), "and again opens them all")
    }
}
