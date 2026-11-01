import XCTest

/// The plan layer on a phone (ov-274), over the canned runner, whose plan is a
/// real board's: `test/fixtures/plan-seeded.json` is the CLI's own output for
/// the board `.claude/agent/reports/ov-273/seed.sh` seeds.
///
/// Only the wiring a unit test cannot reach (ov-538): a runner without
/// `board_plan` shows nothing new (the smoke test), a plan notice reads the
/// plan again, and a ticket and a theme open their pages. What the Plan says
/// (the words of a ticket, a landing, a tile and the cost, the default segment, a
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

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
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
        // A runner with a plan draws the plan column, whose first line is the facts; one without, the tree.
        let drawn = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier IN %@", extra.contains("-phone-plan") ? ["plan-facts", "plan-initiatives", "plan-pipeline"] : ["plan-filters"])
        ).firstMatch
        XCTAssertTrue(drawn.waitForExistence(timeout: 30), "no Plan")
        return app
    }

    /// The Plan's own sections are in the list already: there is no Tasks |
    /// Plan control to choose them by (ov-616).
    private func showPlan(_ app: XCUIApplication) {
        XCTAssertFalse(app.segmentedControls["plan-switch"].exists, "a Tasks | Plan control on the phone")
    }

    func testARunnerWithoutBoardPlanShowsNothingNew() {
        let app = openBoard([])
        // Below the fold on a short phone: on an iPhone Duo's 678-pt screen,
        // Start Orchestrator and Unread fill it, and the list draws no row
        // under them until it's scrolled to (ov-709).
        let row = app.buttons["tree-row-No Theme"]
        if !row.waitForExistence(timeout: 10) { app.reveal(row) }
        XCTAssertTrue(row.waitForExistence(timeout: 10), "no tasks: \(app.debugDescription)")
        XCTAssertFalse(element(app, "plan-landing").exists)
        XCTAssertFalse(element(app, "plan-loading").exists, "a plan read on a runner with no plan")
    }

    /// A runner whose link is down is reconnecting, not a plan that can't be read (ov-631).
    func testAPlanReadWithTheLinkDownSaysReconnecting() {
        // The Plan is the sheet the strip opens (ov-616).
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-board-reads", "-phone-plan-down", "-phone-plan-file", Self.fixture])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 30), "no strip")
        strip.tap()
        XCTAssertTrue(app.reveal(element(app, "plan-reconnecting")), "no reconnecting: \(app.debugDescription)")
        XCTAssertFalse(element(app, "plan-unavailable").exists, "a down link said the plan couldn't be read")
        XCTAssertTrue(element(app, "plan-retry").exists, "no Try Again")
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

    /// Back, found by its name and not in the navigation bar: on an iPhone
    /// Duo (iOS 27.1) Back can stand in a rail at the screen's edge, outside
    /// the bar (ov-709).
    private func back(_ app: XCUIApplication) {
        app.tapBack()
    }

    /// How many times the phone has asked the runner for the plan.
    private func planReads(_ app: XCUIApplication) -> Int { sent(app).filter { $0 == "plan.get" }.count }

    /// Wait for the plan to be read more times than `before`.
    private func readsAgain(_ app: XCUIApplication, after before: Int) -> Bool {
        let deadline = Date().addingTimeInterval(20)
        while planReads(app) <= before, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.3)) }
        return planReads(app) > before
    }

    /// A `plan` notice reaches the connection and the plan on screen is read again.
    func testAPlanNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(element(app, "plan-facts").waitForExistence(timeout: 30), "the plan never drew: \(app.debugDescription)")
        let before = planReads(app)
        XCTAssertGreaterThan(before, 0, "the plan was never read: \(sent(app))")
        Self.notify("plan-news")
        XCTAssertTrue(readsAgain(app, after: before), "the notice didn't re-read the plan: \(sent(app))")
    }

    /// A task notice moves counts the plan derives: it reads the plan again too.
    func testATaskNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(element(app, "plan-facts").waitForExistence(timeout: 30), "the plan never drew: \(app.debugDescription)")
        let before = planReads(app)
        Self.notify("task-news")
        XCTAssertTrue(readsAgain(app, after: before), "the task notice didn't re-read the plan: \(sent(app))")
    }

    /// A notice reads the record of the page on screen, not every page opened since.
    func testANoticeReadsOnlyThePageOnScreen() {
        let app = openBoard()
        showPlan(app)
        // An initiative opens its page, and its ticket opens the ticket's.
        let initiative = element(app, "plan-initiative-Plan layer")
        XCTAssertTrue(app.reveal(initiative), "no initiative: \(app.debugDescription)")
        initiative.tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 30))
        let ticket = element(app, "plan-page-ticket-ov-274")
        XCTAssertTrue(app.reveal(ticket), "no ticket on the theme's page")
        ticket.tap()
        XCTAssertTrue(element(app, "plan-ticket-page").waitForExistence(timeout: 30))
        element(app, "plan-ticket-theme").tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 30))
        let gets = sent(app).filter { $0 == "plan.get" }.count
        let records = sent(app).filter { $0.hasPrefix("plan.events") }.count
        XCTAssertEqual(records, 3, "one record per page opened: \(sent(app))")
        Self.notify("plan-news")
        let deadline = Date().addingTimeInterval(15)
        while sent(app).filter({ $0.hasPrefix("plan.events") }).count == records, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        }
        RunLoop.current.run(until: Date().addingTimeInterval(2))
        let after = sent(app)
        XCTAssertGreaterThan(after.filter { $0 == "plan.get" }.count, gets, "the plan wasn't read again")
        XCTAssertEqual(after.filter { $0.hasPrefix("plan.events") }.count, records + 1, "more than the open page was read: \(after)")
        // Back three times returns to the plan.
        back(app)
        back(app)
        back(app)
        XCTAssertTrue(app.reveal(initiative) || initiative.waitForExistence(timeout: 15), "Back didn't return to the plan")
    }

    // MARK: Pull requests (ov-317)

    /// `test/fixtures/plan-phones-ov317.json`: tickets in every stage a row says, two of them drafts.
    private static var stages: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-ov317-themed.json").path
    }

    private static let draftTicket = "00000000-0000-0000-0000-000000001002"

    /// Billing's Plan, with the pull request actions on unless `extra` says otherwise.
    private func openStages(_ extra: [String] = ["-phone-plan", "-phone-pr-actions"]) -> XCUIApplication {
        let app = openBoard(extra, fixture: Self.stages)
        showPlan(app)
        // The tickets are the initiative's: it opens a page that lists each one.
        let initiative = element(app, "plan-initiative-Pull requests")
        XCTAssertTrue(app.reveal(initiative), "no initiative: \(app.debugDescription)")
        initiative.tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 30), "no theme page: \(app.debugDescription)")
        return app
    }

    private func openTicketPage(_ app: XCUIApplication, _ name: String) {
        let row = element(app, "plan-page-ticket-\(name)")
        XCTAssertTrue(app.reveal(row), "no row for \(name)")
        row.tap()
        XCTAssertTrue(element(app, "plan-ticket-page").waitForExistence(timeout: 30), "\(name) didn't open its page")
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// **A draft ticket's page offers Mark Ready, asks first, and marks the number the page showed**
    /// (ov-317). The harness refuses a ticket or a number that isn't a draft in the file, and from
    /// then on says the ticket waits on alice, so the page showing it is the plan read again.
    func testADraftTicketPageMarksItsPullRequestReadyAfterAsking() {
        let app = openStages()
        openTicketPage(app, "ov-2")
        XCTAssertEqual(element(app, "plan-ticket-stage").label, "Agent Review")
        XCTAssertEqual(element(app, "plan-ticket-stage-detail").label, "PR 41")
        keep(app, "stages-draft-ticket", expecting: .all(.reads(element(app, "plan-ticket-stage"), "Agent Review"), .reads(element(app, "plan-ticket-stage-detail"), "PR 41")))
        element(app, "plan-ticket-mark-ready").tap()
        XCTAssertTrue(app.staticTexts["Mark PR 41 Ready for Review?"].waitForExistence(timeout: 10), "it didn't ask first: \(app.debugDescription)")
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.ready") }, "sent before the confirmation: \(sent(app))")
        keep(app, "stages-mark-ready-asks", expecting: .shows(app.staticTexts["Mark PR 41 Ready for Review?"]))
        app.buttons.matching(identifier: "plan-ticket-mark-ready-confirm").firstMatch.tap()
        XCTAssertTrue(
            wait(30) { sent(app).contains("pr.ready \(Self.draftTicket) 41") }, "the ticket and the number weren't sent: \(sent(app))")
        XCTAssertTrue(
            wait(30) { element(app, "plan-ticket-stage").label == "Waiting on alice" }, "the page didn't read where it stands now")
        XCTAssertFalse(element(app, "plan-ticket-mark-ready").exists, "Mark Ready stays on a pull request that's ready")
        keep(app, "stages-marked-ready", expecting: .all(.reads(element(app, "plan-ticket-stage"), "Waiting on alice"), .gone(element(app, "plan-ticket-mark-ready"))))
    }

    /// Cancel sends nothing; a pull request that needs a person, a runner without the actions and
    /// a Read grant offer no Mark Ready, and the link to GitHub is still there where there is one.
    func testMarkReadyIsOfferedOnlyWhereItCanWork() {
        var app = openStages()
        openTicketPage(app, "ov-2")
        element(app, "plan-ticket-mark-ready").tap()
        app.buttons.matching(identifier: "plan-ticket-mark-ready-cancel").firstMatch.tap()
        XCTAssertTrue(element(app, "plan-ticket-mark-ready").waitForExistence(timeout: 5))
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.ready") }, "\(sent(app))")
        back(app)
        openTicketPage(app, "ov-6")
        XCTAssertEqual(element(app, "plan-ticket-stage-attention").label, "Changes Requested")
        XCTAssertFalse(element(app, "plan-ticket-mark-ready").exists, "changes requested is not a draft")
        XCTAssertTrue(element(app, "plan-ticket-review-on-github").exists)
        app.terminate()

        // A runner from before the actions.
        app = openStages(["-phone-plan"])
        openTicketPage(app, "ov-2")
        XCTAssertTrue(element(app, "plan-ticket-stage").exists)
        XCTAssertFalse(element(app, "plan-ticket-mark-ready").exists, "offered where the runner can't do it")
        XCTAssertTrue(element(app, "plan-ticket-review-on-github").exists, "the link needs no runner")
        app.terminate()

        // A Read grant.
        app = openStages(["-phone-plan", "-phone-pr-actions", "-phone-read-scope"])
        openTicketPage(app, "ov-2")
        XCTAssertFalse(element(app, "plan-ticket-mark-ready").exists, "offered to a phone that can only look")
    }

    /// **A member's phone is offered no Mark Ready** (ov-712): it posts as the owner's `gh` login,
    /// which the runner refuses a member (ov-637). Bob leads Billing here, so he lands on its Plan
    /// as the owner does; the link to GitHub is still his.
    func testAMembersPhoneIsOfferedNoMarkReady() {
        let app = openStages(["-phone-plan", "-phone-pr-actions", "-phone-shared", "-phone-rep-lead"])
        openTicketPage(app, "ov-2")
        XCTAssertEqual(element(app, "plan-ticket-stage").label, "Agent Review")
        XCTAssertTrue(element(app, "plan-ticket-review-on-github").exists, "the link needs no say")
        XCTAssertFalse(element(app, "plan-ticket-mark-ready").exists, "Mark Ready offered to a member")
    }

    /// **Open on GitHub hands the pull request's github.com link to the system** (ov-317), which
    /// gives it to GitHub Mobile when it is installed. The harness records the URL it was handed.
    func testOpenOnGitHubHandsTheLinkToTheSystem() {
        let app = openStages()
        openTicketPage(app, "ov-2")
        XCTAssertEqual(element(app, "plan-ticket-review-on-github").label, "Open on GitHub")
        element(app, "plan-ticket-review-on-github").tap()
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
        keep(app, "stages-needs-you", expecting: .all(.shows(element(app, "needs-you-reply-\(Self.thread)")), .reads(element(app, "needs-you-github-\(Self.thread)"), "Open on GitHub")))
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
        keep(app, "stages-reply-sheet", expecting: .when("the reply is typed") { (text.value as? String ?? "").contains("Retrying only on 503") })
        element(app, "pr-reply-continue").tap()
        XCTAssertTrue(app.staticTexts["Reply on acme/billing #213?"].waitForExistence(timeout: 15), "no confirmation: \(app.debugDescription)")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "as e-liang on GitHub")).firstMatch.exists,
            "the confirmation doesn't name the login: \(app.debugDescription)")
        XCTAssertTrue(sent(app).contains("pr.thread_reply preview"), "\(sent(app))")
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("pr.thread_reply send") }, "posted before the confirmation: \(sent(app))")
        keep(app, "stages-reply-confirm", expecting: .shows(app.staticTexts["Reply on acme/billing #213?"]))
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
        keep(app, "stages-reply-refused", expecting: .reads(failure, "Open it there to reply"))
        app.terminate()

        app = openNeedsYou([])
        XCTAssertTrue(element(app, "needs-you-item-\(Self.thread)").waitForExistence(timeout: 30))
        XCTAssertTrue(element(app, "needs-you-github-\(Self.thread)").exists, "the link needs no runner")
        XCTAssertFalse(element(app, "needs-you-reply-\(Self.thread)").exists, "Reply on a runner that can't post")
        app.terminate()

        // A member's phone (ov-712): the reply would post as the owner.
        app = openNeedsYou(["-phone-pr-actions", "-phone-shared"])
        XCTAssertTrue(element(app, "needs-you-item-\(Self.thread)").waitForExistence(timeout: 30))
        XCTAssertTrue(element(app, "needs-you-github-\(Self.thread)").exists, "the link needs no say")
        // Until the runner says who this phone is, it's taken for the owner's (as Keep is, ov-707).
        XCTAssertTrue(wait(15) { !element(app, "needs-you-reply-\(Self.thread)").exists }, "Reply offered to a member")
        app.terminate()

        app = openNeedsYou(["-phone-pr-actions", "-phone-read-scope"])
        XCTAssertTrue(element(app, "needs-you-item-\(Self.thread)").waitForExistence(timeout: 30))
        XCTAssertFalse(element(app, "needs-you-reply-\(Self.thread)").exists, "Reply to a phone that can only look")
        XCTAssertFalse(element(app, "needs-you-github-\(Self.review)").exists, "a link the runner never sent")
    }
}
