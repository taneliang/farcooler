import XCTest

/// A member's writes carry their name (ov-707), over the canned runner: bil-7's
/// record on a shared runner (`-phone-shared -phone-member-notes`), read on
/// Bob's phone and on the owner's (`-phone-rep-owner`), and Decided For You,
/// where a member is offered no Keep. Which word each actor reads as is
/// AgentKit's, pinned in `TaskBoardModelTests` and `PlanPhoneRulingsTests`.
final class MemberBylinesUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-rulings-seeded.json").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    /// The first text on screen containing `words`.
    private func text(_ app: XCUIApplication, containing words: String) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", words)).firstMatch
    }

    private func record(owner: Bool) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-push-task", "bil-7", "-phone-shared", "-phone-member-notes"] + (owner ? ["-phone-rep-owner"] : []))
        let alexandra = text(app, containing: "Alexandra Montgomery-Fitzgerald")
        XCTAssertTrue(app.reveal(alexandra), "no byline with Alexandra's name: \(app.debugDescription)")
        return app
    }

    private func rulings(owner: Bool) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-plan-file", Self.fixture, "-phone-rulings", "-phone-shared"]
                + (owner ? ["-phone-rep-owner"] : []))
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        // A member who doesn't lead Billing lands on their rep, with the Plan
        // a sheet behind the strip (ov-616); the owner, on the Plan itself.
        let strip = app.buttons["plan-strip"]
        if owner {
            XCTAssertTrue(app.reveal(element(app, "plan-landing")), "the plan didn't draw: \(app.debugDescription)")
            XCTAssertTrue(app.reveal(element(app, "plan-ruling-R-2")), "no R-2: \(app.debugDescription)")
            return app
        }
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no plan strip: \(app.debugDescription)")
        strip.tap()
        let bar = app.navigationBars["Plan"]
        XCTAssertTrue(bar.waitForExistence(timeout: 10), "no Plan sheet")
        let window = app.windows.firstMatch
        for _ in 0..<3 where bar.frame.minY > window.frame.height * 0.2 {
            bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                .press(forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
            Thread.sleep(forTimeInterval: 1)
        }
        let row = element(app, "plan-ruling-R-2")
        let list = element(app, "plan-sheet").collectionViews.firstMatch
        for _ in 0..<10 where !(row.exists && window.frame.contains(CGPoint(x: row.frame.midX, y: row.frame.midY))) {
            (list.exists ? list : app).swipeUp()
        }
        XCTAssertTrue(row.exists, "no R-2 in the Plan sheet: \(app.debugDescription)")
        return app
    }

    func testAMembersNoteShowsTheirNameAndTheOwnersReadsOwner() {
        let app = record(owner: false)
        XCTAssertTrue(app.reveal(text(app, containing: "· Owner ·")), "the owner's comment isn't signed Owner")
        XCTAssertTrue(app.reveal(text(app, containing: "· You ·")), "Bob's own comment isn't signed You")
    }

    func testAMemberIsOfferedNoKeep() {
        let app = rulings(owner: false)
        XCTAssertFalse(app.buttons["Keep All"].exists, "Keep All offered to a member")
        element(app, "plan-ruling-R-2").swipeLeft()
        XCTAssertFalse(app.buttons["Keep"].firstMatch.waitForExistence(timeout: 3), "Keep offered to a member")
    }

    /// Light, on Bob's phone and the owner's. Opt-in: `TEST_RUNNER_FC_CAPTURES=1`.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take them")
        XCUIDevice.shared.appearance = .light
        for (name, owner) in [("member", false), ("owner", true)] {
            let record = record(owner: owner)
            keep(record, "member-record-\(name)-light", expecting: .shows(text(record, containing: "Alexandra")))
            record.terminateRetrying()
            let plan = rulings(owner: owner)
            element(plan, "plan-ruling-R-2").swipeLeft()
            Thread.sleep(forTimeInterval: 1)
            keep(plan, "member-rulings-\(name)-light", expecting: .shows(element(plan, "plan-ruling-R-2")))
            plan.terminateRetrying()
        }
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
