import XCTest

/// A member signs in to Claude from the runner's settings (ov-584): the
/// section above People, the sheet's steps, and the owner's per-member rows.
/// Over the canned runner (`-phone-harness -phone-claude`), which holds a
/// sign-in as the real one does and answers a code that starts "refused" as
/// claude refusing it.
///
/// The last test is a capture tool: off unless `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`
/// names a folder, it shoots each state in light and dark with a long email and
/// a long runner name, tagged by `TEST_RUNNER_FARCOOLER_CAPTURE_TAG`.
final class ClaudeSignInUITests: XCTestCase {
    private static let page = "opened https://claude.com/cai/oauth/authorize?code=true&client_id=harness&state=abc"

    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    /// The toolbar's Sign In, asked as a button. On iOS 26 a toolbar item is an
    /// `Other` wrapper carrying the identifier around the real `Button`, and the
    /// wrapper reads enabled whatever the button is; `.any` finds the wrapper
    /// first, so `isEnabled` through `element` is true even while Disabled.
    private func finishButton(_ app: XCUIApplication) -> XCUIElement {
        app.buttons["claude-sign-in-finish"].firstMatch
    }

    private func sent(_ app: XCUIApplication) -> [String] {
        (element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
    }

    /// Settings, then Runner Settings, as a reader gets there.
    private func runnerSettings(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(["-phone-claude"] + arguments)
        let gear = app.buttons["Settings"].firstMatch
        XCTAssertTrue(gear.waitForExistence(timeout: 30), "no Settings button: \(app.debugDescription)")
        gear.tap()
        let link = app.buttons["Runner Settings"].firstMatch
        XCTAssertTrue(app.reveal(link), "no Runner Settings row: \(app.debugDescription)")
        link.tap()
        return app
    }

    /// The Sign In to Claude button, once the people have been read.
    private func openSheet(_ app: XCUIApplication, again: Bool = false) {
        let button = element(app, again ? "claude-sign-in-again" : "claude-sign-in")
        XCTAssertTrue(app.reveal(button), "no sign-in button: \(app.debugDescription)")
        button.tap()
        XCTAssertTrue(
            element(app, "claude-open-page").waitForExistence(timeout: 30), "no link came: \(app.debugDescription)")
    }

    private func paste(_ app: XCUIApplication, _ code: String) {
        let field = element(app, "claude-code-field")
        XCTAssertTrue(field.waitForExistence(timeout: 10), "no code field: \(app.debugDescription)")
        field.tap()
        field.typeText(code)
    }

    func testAMemberSignsInToClaudeAndSeesWhoTheyAre() {
        let app = runnerSettings([])
        let state = element(app, "claude-login-state")
        XCTAssertTrue(app.reveal(state), "no Claude section: \(app.debugDescription)")
        XCTAssertEqual(
            state.label, "Sign in so your rep and the lanes you start use your own Claude account and its connectors.")
        openSheet(app)
        // Sign In waits for a code.
        XCTAssertTrue(finishButton(app).waitForExistence(timeout: 10), "no Sign In button: \(app.debugDescription)")
        XCTAssertFalse(finishButton(app).isEnabled)
        // Opening the page goes to Safari, not into the app.
        element(app, "claude-open-page").tap()
        XCTAssertTrue(
            Self.holds(within: 10) { self.sent(app).contains(Self.page) }, "the page wasn't opened: \(self.sent(app))")
        paste(app, "abc123#def456")
        XCTAssertTrue(finishButton(app).isEnabled)
        finishButton(app).tap()
        let done = element(app, "claude-sign-in-done-line")
        XCTAssertTrue(done.waitForExistence(timeout: 30), "never signed in: \(app.debugDescription)")
        XCTAssertEqual(done.label, "Signed in as bob@claude.example.")
        // The code reached the runner whole, and is not in what the screens sent.
        XCTAssertTrue(sent(app).contains("claude.sign_in_finish length=13"), "\(sent(app))")
        XCTAssertFalse(sent(app).joined().contains("abc123#def456"))
        element(app, "claude-sign-in-done").tap()
        // The section reads the runner again and says who they are.
        XCTAssertTrue(
            Self.holds(within: 20) { self.element(app, "claude-login-state").label == "Claude · bob@claude.example" },
            "the row never changed: \(self.element(app, "claude-login-state").label)")
    }

    func testACodeWithNoHashIsSaidIncompleteAndNothingIsSent() {
        let app = runnerSettings([])
        openSheet(app)
        paste(app, "abcdef")
        element(app, "claude-sign-in-finish").tap()
        let notice = element(app, "claude-code-notice")
        XCTAssertTrue(notice.waitForExistence(timeout: 10), "no notice: \(app.debugDescription)")
        XCTAssertEqual(notice.label, "That code isn’t complete. Copy the whole code and try again.")
        XCTAssertFalse(sent(app).contains { $0.hasPrefix("claude.sign_in_finish") }, "\(sent(app))")
        // The sign-in goes on: the whole code is taken from the same sheet.
        let field = element(app, "claude-code-field")
        field.tap()
        field.typeText("#ghi")
        element(app, "claude-sign-in-finish").tap()
        XCTAssertTrue(element(app, "claude-sign-in-done-line").waitForExistence(timeout: 30))
    }

    func testARefusedCodeOffersTryAgainWhichAsksForANewLink() {
        let app = runnerSettings([])
        openSheet(app)
        paste(app, "refused#x")
        element(app, "claude-sign-in-finish").tap()
        let failure = element(app, "claude-sign-in-failure")
        XCTAssertTrue(failure.waitForExistence(timeout: 30), "no failure: \(app.debugDescription)")
        XCTAssertEqual(failure.label, "That code didn’t work. Open the sign-in page again to get a new one.")
        element(app, "claude-sign-in-retry").tap()
        XCTAssertTrue(element(app, "claude-open-page").waitForExistence(timeout: 30), "no new link")
        XCTAssertEqual(sent(app).filter { $0 == "claude.sign_in bob" }.count, 2, "\(sent(app))")
    }

    func testCancelEndsTheSignInOnTheRunner() {
        let app = runnerSettings([])
        openSheet(app)
        element(app, "claude-sign-in-cancel").tap()
        XCTAssertTrue(
            Self.holds(within: 10) { self.sent(app).contains("claude.sign_in_cancel") }, "\(sent(app))")
        XCTAssertTrue(element(app, "claude-sign-in").waitForExistence(timeout: 10))
    }

    func testSignOutAsksFirstAndThenSignsOut() {
        let app = runnerSettings(["-phone-claude-signed-in"])
        let state = element(app, "claude-login-state")
        XCTAssertTrue(app.reveal(state), "no Claude section: \(app.debugDescription)")
        XCTAssertEqual(state.label, "Claude · bob@claude.example")
        let out = element(app, "claude-sign-out")
        XCTAssertTrue(app.reveal(out))
        out.tap()
        XCTAssertTrue(
            app.staticTexts["Your rep and lanes there stop until you sign in again."].waitForExistence(timeout: 10),
            "no confirmation: \(app.debugDescription)")
        // Asking is not signing out.
        XCTAssertFalse(sent(app).contains("claude.sign_out bob"))
        app.buttons.matching(NSPredicate(format: "label == 'Sign Out' AND identifier != 'claude-sign-out'")).firstMatch.tap()
        XCTAssertTrue(
            Self.holds(within: 20) { self.element(app, "claude-sign-in").exists }, "still signed in: \(app.debugDescription)")
        // The probe is sampled twice a second: wait for the line, don't read it once.
        XCTAssertTrue(
            Self.holds(within: 10) { self.sent(app).contains("claude.sign_out bob") }, "\(sent(app))")
    }

    func testTheOwnerSeesEachMembersLoginAndNoSectionOfTheirOwn() {
        let app = runnerSettings(["-phone-claude-owner", "-phone-claude-signed-in"])
        let line = element(app, "runner-people-account")
        XCTAssertTrue(line.waitForExistence(timeout: 30), "no People: \(app.debugDescription)")
        XCTAssertEqual(line.label, "Each member’s agents use that member’s own Claude login.")
        let alexandra = element(app, "runner-person-alexandra")
        XCTAssertTrue(app.reveal(alexandra))
        XCTAssertTrue(alexandra.label.contains("Claude · alexandra@claude.example"), alexandra.label)
        let bob = element(app, "runner-person-bob")
        XCTAssertTrue(app.reveal(bob))
        XCTAssertTrue(bob.label.contains("Claude · bob@claude.example"), bob.label)
        XCTAssertFalse(element(app, "claude-sign-in").exists, "the owner is offered a sign-in")
        XCTAssertFalse(element(app, "claude-login-state").exists)
    }

    func testAMemberReadsOtherMembersStateAloneAndNeverTheirEmail() {
        let app = runnerSettings([])
        let alexandra = element(app, "runner-person-alexandra")
        XCTAssertTrue(app.reveal(alexandra), "no row: \(app.debugDescription)")
        XCTAssertTrue(alexandra.label.contains("Signed in to Claude"), alexandra.label)
        XCTAssertFalse(alexandra.label.contains("claude.example"), alexandra.label)
        // Their own row says nothing of Claude: the section above does.
        XCTAssertFalse(element(app, "runner-person-bob").label.contains("Claude"))
    }

    func testARunnerThatCannotSignInShowsNoSection() {
        let app = XCUIApplication.phoneHarness(["-phone-shared"])
        app.buttons["Settings"].firstMatch.tap()
        let link = app.buttons["Runner Settings"].firstMatch
        XCTAssertTrue(app.reveal(link))
        link.tap()
        XCTAssertTrue(element(app, "runner-people-account").waitForExistence(timeout: 30), "no People")
        XCTAssertFalse(element(app, "claude-sign-in").exists, "a runner without sign-in offers it")
        XCTAssertEqual(
            element(app, "runner-people-account").label, "Work here runs on alice@example.com’s Claude account.")
    }

    // MARK: - Captures

    private func shoot(_ dir: URL, _ tag: String, _ name: String, _ scheme: String) {
        Thread.sleep(forTimeInterval: 1)
        let url = dir.appendingPathComponent("\(tag)-\(name)-\(scheme).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    /// The Done step and the section at an accessibility text size, where the
    /// email may take three lines.
    func testAccessibilityCaptures() throws {
        let environment = ProcessInfo.processInfo.environment
        let path = environment["FARCOOLER_CAPTURE_OUT"]
        try XCTSkipUnless(path != nil, "a capture tool: set TEST_RUNNER_FARCOOLER_CAPTURE_OUT")
        let dir = URL(fileURLWithPath: path!)
        let tag = environment["FARCOOLER_CAPTURE_TAG"] ?? "device"
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let size = ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"]
        var app = runnerSettings(["-phone-rep-long", "-phone-host-long", "-phone-claude-signed-in"] + size)
        XCTAssertTrue(app.reveal(element(app, "claude-sign-out")))
        shoot(dir, tag, "a11y-section-signed-in", "light")
        app.terminate()
        app = runnerSettings(["-phone-rep-long"] + size)
        openSheet(app)
        paste(app, "abc#def")
        element(app, "claude-sign-in-finish").tap()
        XCTAssertTrue(element(app, "claude-sign-in-done-line").waitForExistence(timeout: 30))
        shoot(dir, tag, "a11y-sheet-done", "light")
    }

    func testCaptures() throws {
        let environment = ProcessInfo.processInfo.environment
        let path = environment["FARCOOLER_CAPTURE_OUT"]
        try XCTSkipUnless(path != nil, "a capture tool: set TEST_RUNNER_FARCOOLER_CAPTURE_OUT")
        let dir = URL(fileURLWithPath: path!)
        let tag = environment["FARCOOLER_CAPTURE_TAG"] ?? "device"
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let long = ["-phone-rep-long", "-phone-host-long"]
        let code = "Hk3dP9aQ7rWm2VxZ4tN8sLbC6yFeJ1uGoI5pDnRqAh0#8f3b2c91-4d7e-4a6b-9c15-e2a7d4f60b83"
        for scheme in ["light", "dark"] {
            let appearance = ["-phone-appearance", scheme]
            // Not signed in, the sheet at each step, and signed in.
            var app = runnerSettings(long + appearance)
            XCTAssertTrue(app.reveal(element(app, "claude-sign-in")))
            shoot(dir, tag, "section-none", scheme)
            openSheet(app)
            shoot(dir, tag, "sheet-page", scheme)
            paste(app, code)
            shoot(dir, tag, "sheet-code", scheme)
            element(app, "claude-sign-in-finish").tap()
            XCTAssertTrue(element(app, "claude-sign-in-done-line").waitForExistence(timeout: 30))
            shoot(dir, tag, "sheet-done", scheme)
            element(app, "claude-sign-in-done").tap()
            XCTAssertTrue(
                Self.holds(within: 20) { self.element(app, "claude-login-state").label.hasPrefix("Claude · ") })
            shoot(dir, tag, "section-signed-in", scheme)
            app.terminate()

            // The link on its way.
            app = runnerSettings(long + appearance + ["-phone-claude-slow"])
            XCTAssertTrue(app.reveal(element(app, "claude-sign-in")))
            element(app, "claude-sign-in").tap()
            XCTAssertTrue(element(app, "claude-sign-in-starting").waitForExistence(timeout: 10))
            shoot(dir, tag, "sheet-starting", scheme)
            app.terminate()

            // A code claude refused.
            app = runnerSettings(long + appearance)
            openSheet(app)
            paste(app, "refused#x")
            element(app, "claude-sign-in-finish").tap()
            XCTAssertTrue(element(app, "claude-sign-in-failure").waitForExistence(timeout: 30))
            shoot(dir, tag, "sheet-refused", scheme)
            app.terminate()

            // A runner that refuses the start, and a link that expired under the code.
            app = runnerSettings(long + appearance + ["-phone-claude-too-often"])
            XCTAssertTrue(app.reveal(element(app, "claude-sign-in")))
            element(app, "claude-sign-in").tap()
            XCTAssertTrue(element(app, "claude-sign-in-failure").waitForExistence(timeout: 30))
            XCTAssertEqual(element(app, "claude-sign-in-failure").label, "Too many sign-in attempts. Wait a minute, then try again.")
            XCTAssertFalse(element(app, "claude-sign-in-retry").exists, "too often offers Try Again")
            shoot(dir, tag, "sheet-too-often", scheme)
            app.terminate()
            app = runnerSettings(long + appearance + ["-phone-claude-expired"])
            openSheet(app)
            paste(app, "x#y")
            element(app, "claude-sign-in-finish").tap()
            XCTAssertTrue(element(app, "claude-sign-in-failure").waitForExistence(timeout: 30))
            XCTAssertEqual(element(app, "claude-sign-in-failure").label, "The sign-in link expired.")
            shoot(dir, tag, "sheet-expired", scheme)
            app.terminate()

            // Signed in, then the confirmation to sign out.
            app = runnerSettings(long + appearance + ["-phone-claude-signed-in"])
            XCTAssertTrue(app.reveal(element(app, "claude-sign-out")))
            element(app, "claude-sign-out").tap()
            XCTAssertTrue(
                app.staticTexts["Your rep and lanes there stop until you sign in again."].waitForExistence(timeout: 10))
            shoot(dir, tag, "sign-out-confirm", scheme)
            app.terminate()

            // The owner's People.
            app = runnerSettings(long + appearance + ["-phone-claude-owner", "-phone-claude-signed-in"])
            XCTAssertTrue(element(app, "runner-people-account").waitForExistence(timeout: 30))
            shoot(dir, tag, "owner-people", scheme)
            app.terminate()
        }
    }
}
