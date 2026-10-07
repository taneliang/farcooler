import XCTest

/// Orchestrator pages on a phone (ov-285, phase G4 of ov-269), over the canned
/// runner, whose plan, pages and cards are a real board's:
/// `test/fixtures/pages-seeded.json` is the CLI's own `plan --json`, `page list
/// --json` and `task list --json` for the board
/// `.claude/agent/reports/ov-284/seed-pages.sh` seeds, so the bytes the phone
/// decodes are the runner's. The renderer is AgentKit's, the Mac's own.
///
/// Pages live in the Plan view, after Themes; one anchored to a theme draws
/// inside it. References draw live, open what they name, and a question still
/// waiting goes back to Needs You; a web link shows its domain; a reference to
/// something gone is plain text. Each attaches a screenshot, kept.
final class PagesUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    /// `test/fixtures/pages-seeded.json`, from this file's own place.
    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/pages-seeded.json").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Billing's board with Plan chosen, on a runner that keeps a plan and,
    /// unless `extra` says otherwise, pages.
    private func openPlan(_ extra: [String] = ["-phone-pages"]) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-plan", "-phone-plan-file", Self.fixture] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control: \(app.debugDescription)")
        control.buttons["Plan"].tap()
        XCTAssertTrue(element(app, "plan-themes").waitForExistence(timeout: 10), "no plan: \(app.debugDescription)")
        return app
    }

    /// Scroll the list until `target` can be tapped: the way `down` says
    /// first, then back the other way.
    private func reach(_ app: XCUIApplication, _ target: XCUIElement, down: Bool = true) {
        for _ in 0..<8 where !(target.exists && target.isHittable) {
            if down { app.swipeUp() } else { app.swipeDown() }
        }
        for _ in 0..<12 where !(target.exists && target.isHittable) {
            if down { app.swipeDown() } else { app.swipeUp() }
        }
        XCTAssertTrue(target.exists && target.isHittable, "never reached \(target): \(app.debugDescription)")
    }

    private func openTrain(_ app: XCUIApplication) {
        let row = element(app, "plan-page-train")
        reach(app, row)
        row.tap()
        XCTAssertTrue(element(app, "page-train").waitForExistence(timeout: 10), "the page didn't open: \(app.debugDescription)")
    }

    func testPagesAreListedAfterThemesAndAnAnchoredOneIsNot() {
        let app = openPlan()
        let header = element(app, "plan-pages")
        reach(app, header)
        let train = element(app, "plan-page-train"), spend = element(app, "plan-page-spend")
        reach(app, spend)
        XCTAssertTrue(train.exists && spend.exists, "a page of its own isn't listed")
        XCTAssertTrue(
            train.label.hasPrefix("Page, Train integ-10, In review · 3 of 4 lanes green, Updated"), "the row says: \(train.label)")
        XCTAssertFalse(element(app, "plan-page-risks").exists, "a page anchored to a live theme is listed as well")
        // After Themes: the last theme sits above the section.
        XCTAssertLessThan(element(app, "plan-themes").frame.minY, header.frame.minY, "Pages isn't after Themes")
        keep(app, "pages-overview")
    }

    /// The train mockup, live: the lane's state and the card's question come
    /// from the board, the table stacks on a phone, the steps go down it, and
    /// a link shows its domain.
    func testATrainPageDrawsItsBlocksLive() {
        let app = openPlan()
        openTrain(app)
        XCTAssertEqual(app.navigationBars.firstMatch.identifier, "Train integ-10")
        XCTAssertTrue(app.staticTexts["Phones' Plan view and the LFS record, one build, one review."].exists)
        for step in ["Pick, Done", "Build, Done", "Review, Active", "Land, To Do"] {
            XCTAssertTrue(app.descendants(matching: .any)[step].exists, "no step \(step): \(app.debugDescription)")
        }
        // The question on ov-274 is still open: "Needs you", from the board.
        let ask = element(app, "page-item-0")
        reach(app, ask)
        XCTAssertTrue(ask.label.contains("Owner: should Plan hide Unread on phones?"), ask.label)
        XCTAssertTrue(ask.label.hasSuffix("Needs you"), "the question isn't live: \(ask.label)")
        // The web link says where it goes.
        let ci = element(app, "page-item-1")
        XCTAssertTrue(ci.label.hasSuffix("Link to github.com"), "the link hides its domain: \(ci.label)")
        // A four-column table on a phone stacks: each lane a row, each value
        // under its column's title, the lane's state live.
        let lane = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH 'Lane, ov-274-phones.'")).firstMatch
        reach(app, lane, down: false)
        XCTAssertTrue(lane.label.contains("State, In Review · in integ-10."), "the state isn't live: \(lane.label)")
        keep(app, "pages-train")
    }

    /// At the largest Dynamic Type size, a timeline entry's live status sits
    /// under its words, whole, rather than squeezed beside them into "Needs
    /// Deci…" (review M2).
    func testATimelineStatusIsWholeAtTheLargestTextSize() {
        let app = XCUIApplication.phoneHarness([
            "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", Self.fixture, "-phone-pages",
            "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL",
        ])
        // Rows this large run past the screen: scroll to each.
        let billing = app.buttons["workspace-row-Billing"]
        reach(app, billing)
        billing.tap()
        let board = app.buttons["segment-board"]
        XCTAssertTrue(board.waitForExistence(timeout: 10), "no Board segment: \(app.debugDescription)")
        board.tap()
        let control = app.segmentedControls["plan-switch"]
        reach(app, control, down: false)
        control.buttons["Plan"].tap()
        openTrain(app)
        let entry = element(app, "page-entry-0")
        reach(app, entry)
        let words = entry.staticTexts["Review: one high finding"]
        let status = entry.staticTexts["Needs Decision"]
        XCTAssertTrue(words.exists && status.exists, "the entry's parts aren't there: \(entry.debugDescription)")
        XCTAssertGreaterThanOrEqual(status.frame.minY, words.frame.maxY - 1, "the status is beside the words, not under them")
        XCTAssertLessThanOrEqual(status.frame.maxX, app.frame.maxX, "the status runs off the screen")
        keep(app, "pages-timeline-largest-text")
    }

    /// A question still waiting goes back to Needs You, where it's answered.
    func testAnAskReferenceOpensNeedsYou() {
        let app = openPlan()
        openTrain(app)
        let ask = element(app, "page-item-0")
        reach(app, ask)
        ask.tap()
        XCTAssertTrue(element(app, "needs-you").waitForExistence(timeout: 10), "the question didn't open Needs You")
        XCTAssertFalse(element(app, "page-train").exists, "the page is still up")
    }

    /// A card opens its task; a theme or another page opens its page.
    func testReferencesOpenWhatTheyName() {
        let app = openPlan()
        openTrain(app)
        let spend = app.buttons["Spend"]
        reach(app, spend)
        spend.tap()
        XCTAssertTrue(element(app, "page-spend").waitForExistence(timeout: 10), "a page reference didn't open the page")
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        let theme = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Visual language'")).firstMatch
        reach(app, theme)
        theme.tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10), "a theme reference didn't open the theme")
    }

    /// The terminal the train names is on a worktree this runner doesn't
    /// have: its chip is words, not a button, and the page still draws.
    func testAReferenceToSomethingGoneIsPlainText() {
        let app = openPlan()
        openTrain(app)
        let words = app.staticTexts["Build terminal"]
        reach(app, words)
        XCTAssertFalse(app.buttons["Build terminal"].exists, "a reference to a worktree that's gone is a link")
        XCTAssertTrue(app.buttons["Spend"].exists, "the links beside it went too")
    }

    /// The risks page, anchored to Visual language, draws inside it, after
    /// Needs You and before Lanes, and opens whole.
    func testAThemePageDrawsItsAnchoredPage() {
        let app = openPlan()
        let theme = element(app, "plan-theme-Visual language")
        reach(app, theme)
        theme.tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
        let open = element(app, "plan-anchored-open-risks")
        reach(app, open)
        XCTAssertTrue(open.label.hasPrefix("Open Risks, Updated"), open.label)
        let blocked = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH 'Blocked, Sidebar tint'")).firstMatch
        XCTAssertTrue(blocked.exists, "the anchored page's list isn't drawn: \(app.debugDescription)")
        XCTAssertTrue(blocked.label.hasSuffix("Needs you"), blocked.label)
        keep(app, "pages-theme")
        open.tap()
        XCTAssertTrue(element(app, "page-risks").waitForExistence(timeout: 10), "the anchored page didn't open whole")
    }

    /// A `pages` notice reads the board's pages again.
    func testAPagesNoticeReadsThePagesAgain() {
        let app = openPlan()
        let train = element(app, "plan-page-train")
        reach(app, train)
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName("com.farcooler.harness.pages-news" as CFString),
            nil, nil, true)
        let moved = NSPredicate(format: "label BEGINSWITH 'Page, Train integ-10 v2,'")
        expectation(for: moved, evaluatedWith: train)
        waitForExpectations(timeout: 15)
    }

    func testARunnerWithoutPagesShowsNoPagesSection() {
        let app = openPlan([])
        let landed = element(app, "plan-themes")
        reach(app, landed)
        for _ in 0..<6 { app.swipeUp() }
        XCTAssertFalse(element(app, "plan-pages").exists, "a Pages section on a runner without board_pages")
        XCTAssertFalse(element(app, "plan-pages-unavailable").exists)
    }

    func testARefusedPagesReadSaysSoAndOffersTryAgain() {
        let app = openPlan(["-phone-pages-fails"])
        let notice = element(app, "plan-pages-unavailable")
        reach(app, notice)
        XCTAssertEqual(notice.label, "Far Cooler couldn’t read this board’s pages.")
        XCTAssertTrue(element(app, "plan-pages-retry").exists)
        // Not "Pages 0": a read that failed doesn't know how many there are.
        let zero = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == 'plan-pages' AND label == '0'"))
        XCTAssertEqual(zero.count, 0, "a count above a read that failed: \(app.debugDescription)")
        keep(app, "pages-unavailable")
    }

    /// The design's three mockups, light and dark: the train board, spend on
    /// a phone, and a theme's risks, with the Pages section they're listed in.
    /// Pictures for a person, not a check, so only on request
    /// (`TEST_RUNNER_FC_CAPTURES=1` before the script): CI never runs it.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "Captures run on request: TEST_RUNNER_FC_CAPTURES=1")
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openPlan()
            reach(app, element(app, "plan-page-spend"))
            keep(app, "capture-pages-section-\(name)")
            element(app, "plan-page-train").tap()
            XCTAssertTrue(element(app, "page-train").waitForExistence(timeout: 10))
            keep(app, "capture-train-top-\(name)")
            app.swipeUp()
            keep(app, "capture-train-middle-\(name)")
            app.swipeUp()
            app.swipeUp()
            keep(app, "capture-train-bottom-\(name)")
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
            let spend = element(app, "plan-page-spend")
            reach(app, spend)
            spend.tap()
            XCTAssertTrue(element(app, "page-spend").waitForExistence(timeout: 10))
            keep(app, "capture-spend-\(name)")
            app.swipeUp()
            keep(app, "capture-spend-bottom-\(name)")
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
            let theme = element(app, "plan-theme-Visual language")
            reach(app, theme, down: false)
            theme.tap()
            XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
            let risks = element(app, "plan-anchored-open-risks")
            reach(app, risks)
            keep(app, "capture-risks-\(name)")
            app.terminateRetrying()
        }
    }
}
