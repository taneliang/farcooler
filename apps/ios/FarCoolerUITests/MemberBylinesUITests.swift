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

    private static func fixture(_ name: String = "plan-rulings-seeded.json") -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/\(name)").path
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

    /// Billing's Plan, scrolled to `row`: the owner's phone lands on it, a
    /// member's opens it from the strip.
    private func rulings(owner: Bool, fixture: String = MemberBylinesUITests.fixture(), row id: String = "plan-ruling-R-2") -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-plan-file", fixture, "-phone-rulings", "-phone-shared"]
                + (owner ? ["-phone-rep-owner"] : []))
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        // A member who doesn't lead Billing lands on their rep, with the Plan
        // a sheet behind the strip (ov-616); the owner, on the Plan itself.
        let strip = app.buttons["plan-strip"]
        if owner {
            XCTAssertTrue(app.reveal(element(app, "plan-landing")), "the plan didn't draw: \(app.debugDescription)")
            XCTAssertTrue(app.reveal(element(app, id)), "no \(id): \(app.debugDescription)")
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
        let row = element(app, id)
        let list = element(app, "plan-sheet").collectionViews.firstMatch
        for _ in 0..<10 where !(row.exists && window.frame.contains(CGPoint(x: row.frame.midX, y: row.frame.midY))) {
            (list.exists ? list : app).swipeUp()
        }
        XCTAssertTrue(row.exists, "no \(id) in the Plan sheet: \(app.debugDescription)")
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
        XCTAssertFalse(element(app, "plan-rulings-keep-all").exists, "Keep All offered to a member, by its identifier")
        element(app, "plan-ruling-R-2").swipeLeft()
        XCTAssertFalse(app.buttons["Keep"].firstMatch.waitForExistence(timeout: 3), "Keep offered to a member")
    }

    /// The owner's phone offers Keep All, found by its own identifier though
    /// the header around it carries `plan-rulings` (ov-637).
    func testTheOwnerIsOfferedKeepAllByItsIdentifier() {
        let app = rulings(owner: true)
        // The rows were revealed from above, so the header may sit just past
        // the top: back up to it, by its words.
        let shown = app.buttons["Keep All"].firstMatch
        for _ in 0..<8 where !shown.exists { app.swipeDown(velocity: .slow) }
        XCTAssertTrue(shown.exists, "no Keep All: \(app.debugDescription)")
        XCTAssertEqual(shown.identifier, "plan-rulings-keep-all")
        XCTAssertTrue(app.buttons["plan-rulings-keep-all"].exists, "Keep All isn't found by its identifier")
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

    /// The Plan (ov-637), light, on Bob's phone and the owner's, over the
    /// sheet fixture's long real content: Landing at the top, Marketing with
    /// its ask of the owner, and Decided For You, where only the owner is
    /// offered Keep All. Opt-in: `TEST_RUNNER_FC_CAPTURES=1`.
    func testPlanCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take them")
        XCUIDevice.shared.appearance = .light
        let fixture = Self.fixture("plan-phones-sheet.json")
        for (name, owner) in [("member", false), ("owner", true)] {
            for (place, id) in [("landing", "plan-landing"), ("rulings", "plan-ruling-R-4")] {
                let app = rulings(owner: owner, fixture: fixture, row: id)
                if place == "rulings" {
                    XCTAssertEqual(app.buttons["Keep All"].exists, owner, "Keep All on the \(name)'s phone")
                }
                keep(app, "member-plan-\(place)-\(name)-light", expecting: .shows(element(app, id)))
                app.terminateRetrying()
            }
            // Marketing's page, with its ask of the owner.
            let app = rulings(owner: owner, fixture: fixture, row: "plan-landing")
            let marketing = app.staticTexts["Marketing"].firstMatch
            XCTAssertTrue(app.reveal(marketing), "no Marketing: \(app.debugDescription)")
            marketing.tap()
            // A member's sheet lists the themes as the tree's levels, whose
            // Theme Page holds the ask; the owner's Plan names Marketing on a
            // lane, whose page links its theme.
            let own = app.buttons["tree-own"].firstMatch
            let lanes = app.buttons["plan-lane-theme"].firstMatch
            if own.waitForExistence(timeout: 5) { own.tap() } else if lanes.exists { lanes.tap() }
            let page = element(app, "plan-theme-page")
            XCTAssertTrue(page.waitForExistence(timeout: 10), "no Marketing page: \(app.debugDescription)")
            keep(app, "member-plan-ask-\(name)-light", expecting: .shows(page))
            app.terminateRetrying()
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
