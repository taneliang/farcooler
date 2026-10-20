import XCTest

/// The plan layer on a phone (ov-274), over the canned runner, whose plan is a
/// real board's: `test/fixtures/plan-seeded.json` is the CLI's own output for
/// the board `.claude/agent/reports/ov-273/seed.sh` seeds.
///
/// Only the wiring a unit test cannot reach (ov-538): a runner without
/// `board_plan` shows nothing new (the smoke test), a plan notice reads the
/// plan again, and a lane and a theme open their pages. What the Plan says
/// (the words of a lane, a train, a tile and the cost, the default segment, a
/// read that fails, which boards a notice reads) is AgentKit's, pinned in
/// `PlanPhoneBoardTests`.
final class PlanUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    /// `test/fixtures/plan-seeded.json`, from this file's own place.
    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Billing's Plan, its screen while it has no orchestrator (ov-616), on a
    /// runner that advertises `board_plan` unless `extra` says otherwise.
    private func openBoard(_ extra: [String] = ["-phone-plan"], fixture: String = PlanUITests.fixture) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan-file", fixture] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        XCTAssertTrue(element(app, "plan-filters").waitForExistence(timeout: 10), "no Plan")
        return app
    }

    /// The Plan's own sections are in the list already: there is no Tasks |
    /// Plan control to choose them by (ov-616).
    private func showPlan(_ app: XCUIApplication) {
        XCTAssertFalse(app.segmentedControls["plan-switch"].exists, "a Tasks | Plan control on the phone")
    }

    func testARunnerWithoutBoardPlanShowsNothingNew() {
        let app = openBoard([])
        XCTAssertTrue(app.buttons["tree-row-No Theme"].waitForExistence(timeout: 10), "no tasks")
        XCTAssertFalse(element(app, "plan-landing").exists)
        XCTAssertFalse(element(app, "plan-loading").exists, "a plan read on a runner with no plan")
    }

    /// A runner notice, as the harness takes one: a Darwin notification.
    private static func notify(_ which: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.\(which)" as CFString), nil, nil, true)
    }

    private func sent(_ app: XCUIApplication) -> [String] {
        (element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
    }

    /// A `plan` notice reaches the connection and the plan on screen is read again.
    func testAPlanNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones")))
        Self.notify("plan-news")
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones-v2")), "the notice didn't re-read the plan")
    }

    /// A task notice moves counts the plan derives: it reads the plan again too.
    func testATaskNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones")))
        Self.notify("task-news")
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones-v2")), "the task notice didn't re-read the plan")
    }

    /// A notice reads the record of the page on screen, not every page opened since.
    func testANoticeReadsOnlyThePageOnScreen() {
        let app = openBoard()
        showPlan(app)
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(app.reveal(lane))
        lane.tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10))
        element(app, "plan-lane-theme").tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
        let gets = sent(app).filter { $0 == "plan.get" }.count
        let records = sent(app).filter { $0.hasPrefix("plan.events") }.count
        XCTAssertEqual(records, 2, "one record per page opened: \(sent(app))")
        Self.notify("plan-news")
        let deadline = Date().addingTimeInterval(15)
        while sent(app).filter({ $0.hasPrefix("plan.events") }).count == records, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        }
        RunLoop.current.run(until: Date().addingTimeInterval(2))
        let after = sent(app)
        XCTAssertGreaterThan(after.filter { $0 == "plan.get" }.count, gets, "the plan wasn't read again")
        XCTAssertEqual(after.filter { $0.hasPrefix("plan.events") }.count, records + 1, "more than the open page was read: \(after)")
        // Back twice returns to the plan.
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        // The notice renamed the first Next Up lane, as a new plan does.
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones-v2")), "Back didn't return to the plan")
    }

    // MARK: Pull requests (ov-317)

    /// `test/fixtures/plan-phones-ov317.json`: lanes in every stage a row says, two of them drafts.
    private static var stages: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-ov317.json").path
    }

    private static let draftLane = "00000000-0000-0000-0000-000000002002"

    /// Billing's Plan, with the pull request actions on unless `extra` says otherwise.
    private func openStages(_ extra: [String] = ["-phone-plan", "-phone-pr-actions"]) -> XCUIApplication {
        let app = openBoard(extra, fixture: Self.stages)
        showPlan(app)
        XCTAssertTrue(app.reveal(element(app, "plan-lane-lane-2")), "no lanes: \(app.debugDescription)")
        return app
    }

    private func openLanePage(_ app: XCUIApplication, _ name: String) {
        let row = element(app, "plan-lane-\(name)")
        XCTAssertTrue(app.reveal(row), "no row for \(name)")
        row.tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10), "\(name) didn't open its page")
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// **A lane row says where its pull request stands, by the Mac's rules** (ov-316): Building is
    /// the state's word already, so it's said once; a problem leads and the gray line keeps
    /// "Approved"; a pull request the runner couldn't read is added to the state and never
    /// replaces it. The words are AgentKit's (`PlanPrStage`, pinned in `PhonePrActionsTests`).
    func testLaneRowsSayWhereTheirPullRequestStands() {
        let app = openStages()
        // In the order the section draws them: a lazy list is only ever revealed downward.
        var labels: [String: String] = [:]
        for name in ["lane-1", "lane-2", "lane-5", "lane-6", "lane-8", "lane-14", "lane-silent"] {
            let row = element(app, "plan-lane-\(name)")
            XCTAssertTrue(app.reveal(row), "no row for \(name)")
            labels[name] = row.label
        }
        func label(_ name: String) -> String { labels[name] ?? "" }
        XCTAssertTrue(label("lane-2").contains("Agent Review"), label("lane-2"))
        XCTAssertTrue(label("lane-6").contains("Changes Requested"), label("lane-6"))
        let failing = label("lane-8")
        XCTAssertTrue(failing.contains("Checks Failing") && failing.contains("Approved"), failing)
        XCTAssertFalse(failing.contains("Approved, Checks Failing"), "the problem leads: \(failing)")
        let unknown = label("lane-14")
        XCTAssertTrue(unknown.contains("PR State Unknown") && unknown.contains("In Review"), unknown)
        XCTAssertFalse(label("lane-1").contains("Building"), "Building is the state's word already, so the stage leaves it out: \(label("lane-1"))")
        XCTAssertTrue(label("lane-silent").contains("In Review") && !label("lane-silent").contains("Waiting"), label("lane-silent"))
        let long = label("lane-5")
        XCTAssertTrue(long.contains("Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg"), long)
        keep(app, "stages-plan")
    }

    /// **A draft lane's page offers Mark Ready, asks first, and marks the number the page showed**
    /// (ov-317). The harness refuses a lane or a number that isn't a draft in the file, and from
    /// then on says the lane waits on alice, so the page showing it is the plan read again.
    func testADraftLanePageMarksItsPullRequestReadyAfterAsking() {
        let app = openStages()
        openLanePage(app, "lane-2")
        XCTAssertEqual(element(app, "plan-lane-stage").label, "Agent Review")
        XCTAssertEqual(element(app, "plan-lane-stage-detail").label, "PR 41")
        keep(app, "stages-draft-lane")
        element(app, "plan-lane-mark-ready").tap()
        XCTAssertTrue(app.staticTexts["Mark PR 41 Ready for Review?"].waitForExistence(timeout: 10), "it didn't ask first: \(app.debugDescription)")
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.ready") }, "sent before the confirmation: \(sent(app))")
        keep(app, "stages-mark-ready-asks")
        app.buttons.matching(identifier: "plan-lane-mark-ready-confirm").firstMatch.tap()
        XCTAssertTrue(
            wait(30) { sent(app).contains("pr.ready \(Self.draftLane) 41") }, "the lane and the number weren't sent: \(sent(app))")
        XCTAssertTrue(
            wait(30) { element(app, "plan-lane-stage").label == "Waiting on alice" }, "the page didn't read where it stands now")
        XCTAssertFalse(element(app, "plan-lane-mark-ready").exists, "Mark Ready stays on a pull request that's ready")
        keep(app, "stages-marked-ready")
    }

    /// Cancel sends nothing; a pull request that needs a person, a runner without the actions and
    /// a Read grant offer no Mark Ready, and the link to GitHub is still there where there is one.
    func testMarkReadyIsOfferedOnlyWhereItCanWork() {
        var app = openStages()
        openLanePage(app, "lane-2")
        element(app, "plan-lane-mark-ready").tap()
        app.buttons.matching(identifier: "plan-lane-mark-ready-cancel").firstMatch.tap()
        XCTAssertTrue(element(app, "plan-lane-mark-ready").waitForExistence(timeout: 5))
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.ready") }, "\(sent(app))")
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        openLanePage(app, "lane-6")
        XCTAssertEqual(element(app, "plan-lane-stage-attention").label, "Changes Requested")
        XCTAssertFalse(element(app, "plan-lane-mark-ready").exists, "changes requested is not a draft")
        XCTAssertTrue(element(app, "plan-lane-review-on-github").exists)
        app.terminateRetrying()

        // A runner from before the actions.
        app = openStages(["-phone-plan"])
        openLanePage(app, "lane-2")
        XCTAssertTrue(element(app, "plan-lane-stage").exists)
        XCTAssertFalse(element(app, "plan-lane-mark-ready").exists, "offered where the runner can't do it")
        XCTAssertTrue(element(app, "plan-lane-review-on-github").exists, "the link needs no runner")
        app.terminateRetrying()

        // A Read grant.
        app = openStages(["-phone-plan", "-phone-pr-actions", "-phone-read-scope"])
        openLanePage(app, "lane-2")
        XCTAssertFalse(element(app, "plan-lane-mark-ready").exists, "offered to a phone that can only look")
    }

    /// **Open on GitHub hands the pull request's github.com link to the system** (ov-317), which
    /// gives it to GitHub Mobile when it is installed. The harness records the URL it was handed.
    func testOpenOnGitHubHandsTheLinkToTheSystem() {
        let app = openStages()
        openLanePage(app, "lane-2")
        XCTAssertEqual(element(app, "plan-lane-review-on-github").label, "Open on GitHub")
        element(app, "plan-lane-review-on-github").tap()
        XCTAssertTrue(
            wait(30) { sent(app).contains("opened https://github.com/acme/app/pull/41") }, "the link wasn't handed over: \(sent(app))")
    }

    private func openNeedsYou(_ extra: [String] = ["-phone-pr-actions"]) -> XCUIApplication {
        XCUIApplication.phoneHarness(["-phone-pr-needs-you"] + extra)
    }

    private static let thread = "pr-thread:01000000-0000-7000-8000-00000000000e"
    private static let review = "pr-review:0123456789abcdef"

    /// **A review request opens GitHub and a thread offers Reply beside it** (ov-317).
    func testNeedsYouOffersReviewOnGitHubAndReply() {
        let app = openNeedsYou()
        XCTAssertTrue(element(app, "needs-you-item-\(Self.review)").waitForExistence(timeout: 30), "no review request: \(app.debugDescription)")
        XCTAssertEqual(element(app, "needs-you-github-\(Self.review)").label, "Review on GitHub", "somebody else's pull request is reviewed there")
        XCTAssertFalse(element(app, "needs-you-reply-\(Self.review)").exists, "a review request is not a thread")
        XCTAssertEqual(element(app, "needs-you-github-\(Self.thread)").label, "Open on GitHub", "a thread is opened there")
        XCTAssertTrue(element(app, "needs-you-reply-\(Self.thread)").exists, "no Reply on a thread")
        XCTAssertTrue(element(app, "needs-you-action-\(Self.thread)-Retry").exists, "the thread still takes the agent's options")
        keep(app, "stages-needs-you")
        element(app, "needs-you-github-\(Self.review)").tap()
        XCTAssertTrue(
            wait(30) { sent(app).contains("opened https://github.com/acme/billing/pull/213") }, "the link wasn't handed over: \(sent(app))")
    }

    /// **A reply is confirmed with the pull request and the login before anything is posted** (ov-317).
    func testAReplyIsPreviewedThenConfirmedThenSent() {
        let app = openNeedsYou()
        let reply = element(app, "needs-you-reply-\(Self.thread)")
        XCTAssertTrue(reply.waitForExistence(timeout: 30), app.debugDescription)
        reply.tap()
        let text = app.textViews["pr-reply-text"]
        XCTAssertTrue(text.waitForExistence(timeout: 10), "no reply sheet: \(app.debugDescription)")
        XCTAssertFalse(element(app, "pr-reply-continue").isEnabled, "Continue with nothing to say")
        if text.value(forKey: "hasKeyboardFocus") as? Bool != true { text.tap() }
        XCTAssertTrue(wait(30) { text.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the reply never took the keyboard")
        app.typeText("Retrying only on 503: a 500 may have charged.")
        keep(app, "stages-reply-sheet")
        element(app, "pr-reply-continue").tap()
        XCTAssertTrue(app.staticTexts["Reply on acme/billing #213?"].waitForExistence(timeout: 15), "no confirmation: \(app.debugDescription)")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "as e-liang on GitHub")).firstMatch.exists,
            "the confirmation doesn't name the login: \(app.debugDescription)")
        XCTAssertTrue(sent(app).contains("pr.thread_reply preview"), "\(sent(app))")
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.thread_reply send") }, "posted before the confirmation: \(sent(app))")
        keep(app, "stages-reply-confirm")
        app.buttons.matching(identifier: "pr-reply-send").firstMatch.tap()
        XCTAssertTrue(wait(30) { sent(app).contains("pr.thread_reply send 45") }, "the reply wasn't sent whole: \(sent(app))")
        XCTAssertTrue(wait(15) { !element(app, "pr-reply-sheet").exists }, "the sheet stayed open after a reply went")
        // The reply answered the card, so its row left Needs You (the runner's list, read again).
        XCTAssertTrue(wait(30) { !element(app, "needs-you-item-\(Self.thread)").exists }, "the answered thread is still listed")
        XCTAssertTrue(element(app, "needs-you-item-\(Self.review)").exists, "only the thread left")
    }

    /// A thread the runner can't read to one place says so in the sheet and posts nothing; a runner
    /// without the actions and a Read grant offer no Reply.
    func testAReplyThatCannotBePostedSaysSoAndOldRunnersOfferNone() {
        var app = openNeedsYou(["-phone-pr-actions", "-phone-pr-refused"])
        let reply = element(app, "needs-you-reply-\(Self.thread)")
        XCTAssertTrue(reply.waitForExistence(timeout: 30), app.debugDescription)
        reply.tap()
        let text = app.textViews["pr-reply-text"]
        XCTAssertTrue(text.waitForExistence(timeout: 10))
        if text.value(forKey: "hasKeyboardFocus") as? Bool != true { text.tap() }
        XCTAssertTrue(wait(30) { text.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the reply never took the keyboard")
        app.typeText("Thanks.")
        element(app, "pr-reply-continue").tap()
        let failure = element(app, "pr-reply-failure")
        XCTAssertTrue(failure.waitForExistence(timeout: 15), "a refused thread said nothing: \(app.debugDescription)")
        XCTAssertTrue(failure.label.contains("Open it there to reply"), failure.label)
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.thread_reply send") }, "\(sent(app))")
        keep(app, "stages-reply-refused")
        app.terminateRetrying()

        app = openNeedsYou([])
        XCTAssertTrue(element(app, "needs-you-item-\(Self.thread)").waitForExistence(timeout: 30))
        XCTAssertTrue(element(app, "needs-you-github-\(Self.thread)").exists, "the link needs no runner")
        XCTAssertFalse(element(app, "needs-you-reply-\(Self.thread)").exists, "Reply on a runner that can't post")
        app.terminateRetrying()

        app = openNeedsYou(["-phone-pr-actions", "-phone-read-scope"])
        XCTAssertTrue(element(app, "needs-you-item-\(Self.thread)").waitForExistence(timeout: 30))
        XCTAssertFalse(element(app, "needs-you-reply-\(Self.thread)").exists, "Reply to a phone that can only look")
        XCTAssertFalse(element(app, "needs-you-github-\(Self.review)").exists, "a link the runner never sent")
    }

    /// The sheets: the overview, its themes, a lane and a theme, light and dark.
    ///
    /// Opt-in, like the Android Roborazzi captures: it makes screenshots and
    /// asserts nothing a behavior test does not, and it flips the whole
    /// device's appearance, so it stays out of CI and of an ordinary run.
    /// `TEST_RUNNER_FC_CAPTURES=1 xcodebuild test …` turns it on.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openBoard()
            showPlan(app)
            XCTAssertTrue(element(app, "plan-landing").waitForExistence(timeout: 10))
            keep(app, "capture-overview-\(name)")
            let theme = app.buttons["tree-row-Visual language"]
            XCTAssertTrue(app.reveal(theme))
            keep(app, "capture-themes-\(name)")
            theme.tap()
            app.buttons["tree-own"].tap()
            XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
            keep(app, "capture-theme-page-\(name)")
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
            let lane = element(app, "plan-lane-mac-vis")
            for _ in 0..<8 where !(lane.exists && lane.isHittable) { app.swipeDown() }
            lane.tap()
            XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10))
            keep(app, "capture-lane-page-\(name)")
            app.terminateRetrying()
        }
    }
}
