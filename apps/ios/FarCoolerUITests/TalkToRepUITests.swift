import XCTest

/// A member's Talk opens their own rep (ov-526), over the canned runner
/// (`-phone-rep`): this phone is Bob's, and Billing is led by Alexandra, so
/// the runner would refuse Bob the orchestrator's composer (`not_lead`).
/// Bob meets his rep instead, started first when it isn't running. The lead
/// keeps the orchestrator.
final class TalkToRepUITests: XCTestCase {
    /// The refusal, as the app says it on every platform (`RunnerPeople.repNotStarted`).
    enum RepCopy {
        static let notStarted = "The runner didn’t start your rep. Check that this device is still on the team, then try again."
    }

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ extra: [String] = [], dark: Bool = false) -> XCUIApplication {
        .phoneHarness(["-phone-empty-inbox", "-phone-rep"] + [] + extra)
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

    private func openBilling(_ app: XCUIApplication, named name: String = "Billing") {
        let row = app.buttons["workspace-row-\(name)"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no \(name) row: \(app.debugDescription)")
        row.tap()
        XCTAssertTrue(app.navigationBars[name].waitForExistence(timeout: 10), "the workspace did not open")
    }

    /// The rep's Start button: the one thing that asks for the rep to start.
    private func tapTalk(_ app: XCUIApplication) {
        let start = app.buttons["rep-start"]
        XCTAssertTrue(start.waitForExistence(timeout: 10), "no Start Your Rep")
        start.tap()
    }

    /// Whom the conversation is with, as the bar says it under the
    /// workspace's name (ov-616): "Your Rep" or "Orchestrator".
    private func seat(_ app: XCUIApplication) -> [String] {
        app.navigationBars.firstMatch.staticTexts.allElementsBoundByIndex.map(\.label)
    }

    private func sent(_ app: XCUIApplication) -> String {
        element(app, "harness-sent").value as? String ?? ""
    }

    private func waitFor(_ app: XCUIApplication, _ id: String, _ seconds: TimeInterval = 15) {
        XCTAssertTrue(element(app, id).waitForExistence(timeout: seconds), "no \(id): \(app.debugDescription)")
    }

    /// **Talk starts the rep, and says whose account it runs on**: the bar
    /// says "Your Rep" under the workspace's name, Starting Your Rep… shows the rep's first screen
    /// (what a rep is for, whose Claude account the work runs on) while the
    /// runner starts it, and then its pane opens. No orchestrator composer
    /// appears, and nothing starts an orchestrator.
    func testAMembersTalkStartsTheirRepAndOpensItsPane() {
        let app = launch()
        openBilling(app)
        XCTAssertTrue(seat(app).contains("Your Rep"), "\(seat(app))")
        // Opening on Talk asks for nothing: the first screen waits, with its button.
        waitFor(app, "rep-start")
        XCTAssertFalse(sent(app).contains("rep.start"), "opening the workspace started the rep: \(sent(app))")
        tapTalk(app)
        waitFor(app, "rep-starting")
        XCTAssertEqual(element(app, "rep-account").label, "Work here runs on alice@example.com’s Claude account.")
        XCTAssertTrue(element(app, "rep-purpose").label.hasPrefix("A rep passes a person"), element(app, "rep-purpose").label)
        keep(app, "rep-first-screen")
        waitFor(app, "rep-pane", 30)
        XCTAssertTrue(sent(app).contains("rep.start bob billing"), sent(app))
        XCTAssertFalse(element(app, "orchestrator-pane").exists, "a member met the orchestrator's pane")
        XCTAssertFalse(element(app, "start-orchestrator").exists, "a member was offered an orchestrator")
        XCTAssertFalse(sent(app).contains("start billing"), sent(app))
        keep(app, "rep-pane")
    }

    /// **A running rep opens at once**, with no start sent, even where the
    /// workspace's orchestrator is running too: Bob never meets its pane.
    func testARunningRepOpensWithoutAStart() {
        let app = launch(["-phone-rep-running", "-phone-billing-led"])
        openBilling(app)
        waitFor(app, "rep-pane")
        XCTAssertFalse(element(app, "orchestrator-pane").exists, "a member met the orchestrator's pane")
        XCTAssertFalse(sent(app).contains("rep.start"), sent(app))
    }

    /// **A stopped rep is started again** by Talk.
    func testAStoppedRepIsStartedAgain() {
        let app = launch(["-phone-rep-stopped"])
        openBilling(app)
        // A rep the member stopped stays stopped until they ask.
        waitFor(app, "rep-stopped")
        XCTAssertEqual(app.buttons["rep-start"].label, "Resume Your Rep")
        XCTAssertEqual(element(app, "rep-stopped").label, "Your rep isn’t running. Resume picks up where it left off.")
        XCTAssertFalse(sent(app).contains("rep.start"), "a stopped rep was started on opening: \(sent(app))")
        tapTalk(app)
        waitFor(app, "rep-pane", 30)
        XCTAssertTrue(sent(app).contains("rep.start bob billing"), sent(app))
    }

    /// **The lead keeps the orchestrator**: where Bob leads Billing, Talk is
    /// the orchestrator's pane and no rep is started.
    func testTheLeadKeepsTheOrchestrator() {
        let app = launch(["-phone-rep-lead", "-phone-billing-led"])
        openBilling(app)
        waitFor(app, "orchestrator-pane")
        XCTAssertTrue(seat(app).contains("Orchestrator"), "\(seat(app))")
        XCTAssertFalse(element(app, "rep-pane").exists)
        XCTAssertFalse(sent(app).contains("rep.start"), sent(app))
    }

    /// **A runner that won't start the rep says so and offers Try Again**,
    /// in the app's words and not the runner's.
    func testARefusedStartSaysSoAndOffersTryAgain() {
        let app = launch(["-phone-rep-refused"])
        openBilling(app)
        tapTalk(app)
        waitFor(app, "rep-refusal", 30)
        XCTAssertEqual(element(app, "rep-refusal").label, RepCopy.notStarted)
        XCTAssertEqual(app.buttons["rep-start"].label, "Try Again")
        XCTAssertFalse(element(app, "rep-pane").exists)
        keep(app, "rep-refused")
    }

    /// **Long names fit**: a long workspace name and a long account.
    func testLongNamesFit() {
        let app = launch(["-phone-rep-long", "-phone-rep-refused"])
        openBilling(app, named: "Billing and Collections for the Enterprise Platform Team")
        tapTalk(app)
        waitFor(app, "rep-refusal", 30)
        XCTAssertTrue(element(app, "rep-account").label.contains("platform-team-shared-account@example-corporation.com"))
        let screen = app.windows.firstMatch.frame
        for id in ["rep-account", "rep-purpose", "rep-start"] {
            let frame = element(app, id).frame
            XCTAssertGreaterThanOrEqual(frame.minX, screen.minX, "\(id) runs off the left")
            XCTAssertLessThanOrEqual(frame.maxX, screen.maxX, "\(id) runs off the right")
        }
        keep(app, "rep-long-names")
    }

    /// **Captures**, light and dark, with long names: the rep's first screen
    /// where the runner refused it (the longest the screen gets), and the
    /// rep's pane. For a person to look at; the checks are the tests above.
    func testCapturesInBothAppearances() {
        let long = "Billing and Collections for the Enterprise Platform Team"
        for (mode, dark) in [("light", false), ("dark", true)] {
            XCUIDevice.shared.appearance = dark ? .dark : .light
            let refused = launch(["-phone-rep-long", "-phone-rep-refused"], dark: dark)
            openBilling(refused, named: long)
            tapTalk(refused)
            waitFor(refused, "rep-refusal", 30)
            keep(refused, "iphone-rep-first-screen-long-\(mode)")
            refused.terminate()

            let running = launch(["-phone-rep-long", "-phone-rep-running"], dark: dark)
            openBilling(running, named: long)
            waitFor(running, "rep-pane", 30)
            keep(running, "iphone-rep-pane-long-\(mode)")
            running.terminate()
        }
    }

    /// **A start that races another device's** (the runner says the rep is
    /// running) says nothing and opens the pane.
    func testARacedStartOpensThePaneWithoutARefusal() {
        let app = launch(["-phone-rep-raced"])
        openBilling(app)
        tapTalk(app)
        waitFor(app, "rep-pane", 30)
        XCTAssertFalse(element(app, "rep-refusal").exists, "a rep that was running was called refused")
    }

    /// **While the people are being read, nothing is typed to anyone**: no
    /// orchestrator pane, though one is running, and no rep pane.
    func testWhileReadingThereIsNothingToTypeInto() {
        let app = launch(["-phone-rep-slow-list", "-phone-billing-led", "-phone-rep-running"])
        openBilling(app)
        waitFor(app, "talk-reading", 6)
        XCTAssertFalse(element(app, "orchestrator-pane").exists, "the orchestrator was offered while reading")
        XCTAssertFalse(element(app, "rep-pane").exists)
        XCTAssertFalse(seat(app).contains("Your Rep"), "the word flashed Your Rep while reading")
        waitFor(app, "rep-pane", 30)
        XCTAssertTrue(seat(app).contains("Your Rep"), "\(seat(app))")
    }

    /// **A list that can't be read says so**, offers Try Again, and never
    /// offers the orchestrator's pane.
    func testAListThatCannotBeReadOffersNoComposer() {
        let app = launch(["-phone-rep-list-fails", "-phone-billing-led"])
        openBilling(app)
        waitFor(app, "talk-try-again", 15)
        XCTAssertTrue(app.staticTexts["Can’t Tell Who’s on the Team"].exists, app.debugDescription)
        XCTAssertFalse(element(app, "orchestrator-pane").exists)
        XCTAssertFalse(element(app, "rep-pane").exists)
        keep(app, "iphone-talk-unknown")
    }

    /// **The owner's phone has its orchestrator**, and once the runner has
    /// said it's the owner's, a later launch keeps it with the list slow or
    /// failing: the owner never waits on `member.list`.
    func testTheOwnersPhoneNeverWaitsOnThePeople() {
        let first = launch(["-phone-rep-owner", "-phone-billing-led"])
        openBilling(first)
        waitFor(first, "orchestrator-pane", 30)
        XCTAssertTrue(seat(first).contains("Orchestrator"), "\(seat(first))")
        first.terminate()

        let later = launch(["-phone-rep-owner", "-phone-billing-led", "-phone-rep-keep-you", "-phone-rep-list-fails"])
        openBilling(later)
        waitFor(later, "orchestrator-pane", 10)
        XCTAssertFalse(element(later, "talk-try-again").exists, "a failed read walled the owner out")
        XCTAssertFalse(element(later, "talk-reading").exists)
    }
}
