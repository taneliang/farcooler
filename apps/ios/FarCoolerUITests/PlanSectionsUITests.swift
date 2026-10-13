import XCTest

/// The Plan as sections on an iPhone (ov-503, phase 7), over the canned runner
/// and `test/fixtures/plan-phones-ov503.json`: the sections in
/// Shipping, In Review, Building, Waiting, On a Landed Train, Agents and Themes
/// order, a train's place on its ladder, a held train, the agents list, the
/// themes grid in one column, and Themes and Lanes as two rows that push lists.
/// The words are AgentKit's (`PlanSections`, `PlanGlanceSentence`), pinned by
/// its tests; these pin that the phone draws them where they belong.
final class PlanSectionsUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-ov503.json").path
    }

    /// The first match: an identifier on a container reaches its text too.
    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// A short drag up, so a lazy list draws every row it passes.
    private func scrollDown(_ app: XCUIApplication) {
        let window = app.windows.firstMatch
        window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
            .press(forDuration: 0.05, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)))
    }

    /// Billing's board with the Plan chosen: the phone's list of sections.
    private func openPlan() -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-plan-file", Self.fixture])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control: \(app.debugDescription)")
        control.buttons["Plan"].tap()
        XCTAssertTrue(element(app, "plan-shipping").waitForExistence(timeout: 10), "no Shipping: \(app.debugDescription)")
        return app
    }

    /// Scroll down until `id` can be touched.
    @discardableResult
    private func reach(_ app: XCUIApplication, _ id: String, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let target = element(app, id)
        // On screen, below the segments and above the home indicator: a header isn't "hittable".
        let window = app.windows.firstMatch.frame
        func shown() -> Bool { target.exists && target.frame.minY > window.height * 0.2 && target.frame.maxY < window.height * 0.95 }
        for _ in 0..<40 where !shown() { scrollDown(app) }
        XCTAssertTrue(shown(), "can't reach \(id) (frame \(target.frame), exists \(target.exists)): \(app.debugDescription)", file: file, line: line)
        return target
    }

    // MARK: The band

    /// **The band is Now and Next in two lines, and what needs the owner is its one mark**, the
    /// words AgentKit's: the furthest-along train leads Now, and Next names the first ranked lane.
    func testTheBandSaysNowAndNextAndWhatNeedsYou() {
        let app = openPlan()
        let band = element(app, "plan-band")
        XCTAssertTrue(band.waitForExistence(timeout: 10), "no band: \(app.debugDescription)")
        // One element, read as the sentence: Now, Next, then what needs the owner.
        XCTAssertTrue(band.label.contains("Train 86 waiting for CI"), band.label)
        XCTAssertTrue(band.label.contains(" building") && band.label.contains(" in review"), band.label)
        XCTAssertTrue(band.label.contains("Next: A real window capture on the front page, then Onboarding"), band.label)
        XCTAssertTrue(band.label.hasSuffix(" needs you."), "the band doesn't say what needs you: \(band.label)")
    }

    // MARK: The sections

    /// **The sections come in the owner's order**: Shipping, In Review, Building, Waiting, then
    /// On a Landed Train, Agents and Themes. Each header is below the one before it.
    func testTheSectionsComeInOrder() {
        let app = openPlan()
        let order = [
            "plan-shipping", "plan-in-review", "plan-building", "plan-waiting", "plan-on-landed-train", "plan-agents",
            "plan-themes",
        ]
        var last: CGFloat?
        var lastID = ""
        for id in order {
            let header = reach(app, id)
            let previous = element(app, lastID)
            if last != nil, previous.exists, previous.frame.maxY > 0 {
                XCTAssertLessThan(previous.frame.minY, header.frame.minY, "\(lastID) isn't above \(id)")
            }
            last = header.frame.minY
            lastID = id
        }
        XCTAssertNotNil(last)
    }

    /// **A train says where it stands on the ladder, compactly**, and a held one says what holds it
    /// and for how long; a lane on it says how it stands to the train.
    func testShippingDrawsTheLadderAndTheHeldTrain() {
        let app = openPlan()
        let ready = reach(app, "plan-train-integ-86")
        XCTAssertTrue(ready.label.contains("Waiting for CI \u{00B7} step 4 of 5"), ready.label)
        XCTAssertTrue(ready.label.contains("3f78754f"), "the commit isn't said: \(ready.label)")
        XCTAssertTrue(ready.label.contains("Integrator \u{00B7} Opus"), ready.label)
        XCTAssertTrue(ready.label.contains("The team directory; the iPhone composer"), ready.label)
        XCTAssertTrue(reach(app, "plan-lane-team-dir").label.contains("Ready"), "the lane doesn't say it's ready")

        let held = reach(app, "plan-train-integ-89")
        XCTAssertTrue(held.label.contains("Waiting for Lanes \u{00B7} step 1 of 5"), held.label)
        XCTAssertTrue(held.label.contains("Holding for ios-overwrite"), "the hold isn't said: \(held.label)")
        let holder = reach(app, "plan-lane-ios-overwrite")
        XCTAssertTrue(holder.label.contains("Fixing \u{00B7} round 2"), holder.label)

        XCTAssertTrue(reach(app, "plan-train-integ-84").label.contains("Stalled \u{00B7} step 2 of 5"))
        // The slug is for a person who needs it, and nowhere in the text.
        XCTAssertFalse(ready.label.contains("integ-86"), ready.label)
    }

    /// **Waiting is ranked, then not ranked**: each group is named once something else shares the
    /// section, and the ranked lanes carry their place and why.
    func testWaitingRanksAndNamesItsGroups() {
        let app = openPlan()
        XCTAssertTrue(reach(app, "plan-next-up").exists, "no Next Up group")
        let first = reach(app, "plan-lane-hero-shot")
        XCTAssertTrue(first.label.contains("Unblocked once the site is one project"), first.label)
        XCTAssertTrue(first.label.hasPrefix("1"), "the first ranked lane isn't numbered 1: \(first.label)")
        XCTAssertTrue(reach(app, "plan-not-ranked").exists, "no Not Ranked group")
        XCTAssertTrue(reach(app, "plan-waiting-on-you").exists, "the lane whose card needs a decision isn't Waiting on You")
        XCTAssertFalse(element(app, "plan-lane-web-site").label.hasPrefix("1"))
    }

    /// **Lanes on a landed train are folded**, one disclosure that says how many.
    func testALandedTrainsLanesAreFolded() {
        let app = openPlan()
        let header = reach(app, "plan-on-landed-train")
        XCTAssertEqual(header.value as? String, "Collapsed")
        XCTAssertFalse(element(app, "plan-lane-old-queued").exists, "a folded lane is drawn")
        header.tap()
        XCTAssertEqual(element(app, "plan-on-landed-train").value as? String, "Expanded")
        XCTAssertTrue(element(app, "plan-lane-old-queued").waitForExistence(timeout: 5), "opening drew nothing")
    }

    /// **Agents are a list of who, for what and how many tokens**, working ones first, and the
    /// finished ones fold.
    func testAgentsListWhoWorksForWhat() {
        let app = openPlan()
        let header = reach(app, "plan-agents-header")
        XCTAssertTrue(header.label.contains("working"), header.label)
        let builder = reach(app, "plan-agent-pane:b1")
        XCTAssertTrue(builder.label.contains("Builder \u{00B7} Sonnet"), builder.label)
        XCTAssertTrue(builder.label.contains("tile-grid"), "the lane it works for isn't said: \(builder.label)")
        XCTAssertTrue(builder.label.contains("380K tokens"), builder.label)
        let finished = reach(app, "plan-agents-finished")
        XCTAssertEqual(finished.value as? String, "Collapsed")
        XCTAssertFalse(element(app, "plan-agent-agent:s1").exists)
        finished.tap()
        let loose = element(app, "plan-agent-agent:s1")
        XCTAssertTrue(loose.waitForExistence(timeout: 5), "the finished agents didn't open")
        XCTAssertTrue(loose.label.contains("Design ov-503") && loose.label.contains("Not on a lane"), loose.label)
    }

    /// **Themes are a grid of tiles, one column on a phone**: each says its progress, its cards and
    /// the one amber line for what waits on the owner; the finished ones are one tile.
    func testThemesAreATileColumn() {
        let app = openPlan()
        let first = reach(app, "plan-theme-Teams and accounts")
        let marketing = reach(app, "plan-theme-Marketing")
        XCTAssertTrue(first.label.contains("done"), first.label)
        XCTAssertTrue(marketing.label.contains("Needs you: How do we ship the site"), marketing.label)
        XCTAssertEqual(first.frame.minX, marketing.frame.minX, accuracy: 1, "the tiles aren't in one column")
        XCTAssertGreaterThan(first.frame.width, 280, "a tile is squeezed: \(first.frame)")
        let tile = reach(app, "plan-theme-finished")
        XCTAssertTrue(tile.label.contains("Mac look and feel"), "the finished tile doesn't name its themes: \(tile.label)")
        XCTAssertFalse(element(app, "plan-theme-Mac look and feel").exists, "a finished theme has a tile of its own")
    }

    // MARK: Themes and Lanes

    /// **Themes and Lanes are two rows that push lists**, as a phone has no sidebar: Themes holds each
    /// theme's cards, and a theme's header opens its page.
    func testThemesPushesItsList() {
        let app = openPlan()
        reach(app, "plan-browse-themes").tap()
        let list = element(app, "plan-browse-themes-list")
        XCTAssertTrue(list.waitForExistence(timeout: 10), "Themes didn't push: \(app.debugDescription)")
        XCTAssertTrue(element(app, "plan-browse-row-ov-487").waitForExistence(timeout: 5), "no card under its theme")
        XCTAssertTrue(element(app, "plan-browse-Teams and accounts").exists, "no theme header")
        element(app, "plan-browse-Teams and accounts").tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10), "a theme's header didn't open its page")
    }

    /// **Lanes groups them as the Plan does**: by train, then In Review, Building and the folded
    /// Waiting, each lane with its agents; a lane opens its page.
    func testLanesPushesItsList() {
        let app = openPlan()
        reach(app, "plan-browse-lanes").tap()
        XCTAssertTrue(element(app, "plan-browse-lanes-list").waitForExistence(timeout: 10), "Lanes didn't push")
        XCTAssertTrue(element(app, "plan-browse-Train 86").waitForExistence(timeout: 5), "no train group")
        let lane = element(app, "plan-browse-row-team-dir")
        XCTAssertTrue(lane.exists, "no lane under its train")
        XCTAssertTrue(lane.label.contains("Ready"), "the lane doesn't say how it stands to the train: \(lane.label)")
        lane.tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10), "a lane didn't open its page")
    }

    /// **Waiting starts folded in Lanes**, as in the sidebar, and opens on a tap.
    func testWaitingStartsFoldedInLanes() {
        let app = openPlan()
        reach(app, "plan-browse-lanes").tap()
        XCTAssertTrue(element(app, "plan-browse-lanes-list").waitForExistence(timeout: 10), "Lanes didn't push")
        let waiting = reach(app, "plan-browse-Waiting")
        XCTAssertEqual(waiting.value as? String, "Collapsed")
        XCTAssertFalse(element(app, "plan-browse-row-hero-shot").exists, "a folded group is drawn")
        waiting.tap()
        XCTAssertTrue(element(app, "plan-browse-row-hero-shot").waitForExistence(timeout: 5), "opening drew nothing")
    }
}
