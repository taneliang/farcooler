import XCTest

/// Accessibility-size Dynamic Type, which broke the workspace segment bar
/// ("Orches-/trator") before the cards over a conversation took its place
/// (ov-616). (The chat pane's queue and chips, which it broke too, went with
/// chat mode, ov-377.)
///
/// The size comes in as a launch argument, which the app reads as its own
/// preferred content size. Each test asserts a SHAPE rather than a pixel.
final class DynamicTypeTests: XCTestCase {
    private static let size = [
        "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL",
    ]

    /// **At an accessibility size the two cards stack** (ov-619): Agents
    /// under Now/Next, each the row's whole width, rather than either cut.
    func testTheCardsStackAtAnAccessibilitySize() throws {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox", "-phone-native", "-native-agents"] + Self.size)
        let row = app.buttons["workspace-row-Main"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no Main row")
        row.tap()
        let plan = app.buttons["plan-strip"], agents = app.buttons["agents-card"]
        XCTAssertTrue(plan.waitForExistence(timeout: 10), "the workspace did not open")
        XCTAssertTrue(agents.waitForExistence(timeout: 15), "no Agents card")
        XCTAssertGreaterThanOrEqual(agents.frame.minY, plan.frame.maxY - 1, "the cards didn't stack")
        XCTAssertEqual(agents.frame.width, plan.frame.width, accuracy: 2, "the stacked cards aren't one width")
    }

    // MARK: The largest size (ov-422, ov-423, ov-424)

    /// Accessibility XXXL, the largest the system offers.
    private static let largest = [
        "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL",
    ]

    private static func fixture(_ name: String) -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/\(name)").path
    }

    private func every(_ app: XCUIApplication, _ id: String) -> [XCUIElement] {
        app.descendants(matching: .any).matching(identifier: id).allElementsBoundByIndex
    }

    /// Billing over `plan` (a file in test/fixtures). The long one has one lane in
    /// Now and none in Next Up, so the lane that draws a glyph, not a rank, is on screen.
    private func openBilling(
        _ extra: [String], size: [String], plan: String = "plan-seeded.json"
    ) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file", Self.fixture(plan)]
                + extra + size)
        let billing = app.buttons["workspace-row-Billing"]
        for _ in 0..<10 where !(billing.exists && billing.isHittable) { app.swipeUp() }
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        XCTAssertTrue(app.buttons["plan-strip"].waitForExistence(timeout: 15), "no Now/Next card")
        return app
    }

    /// **A Needs You row keeps its age on one line and its context uncut**
    /// (ov-422). The age wrapped to "2m / ago" and the context was cut to
    /// "Billing · b…", losing the task key. Each text probe reports the lines
    /// it drew and whether any text was cut off.
    func testNeedsYouRowKeepsItsAgeAndContextAtTheLargestSize() throws {
        let app = XCUIApplication.phoneHarness(Self.largest)
        let ages = app.descendants(matching: .any).matching(identifier: "needs-you-age")
        XCTAssertTrue(ages.firstMatch.waitForExistence(timeout: 30), "no row drew an age")
        for id in ["needs-you-age", "needs-you-context"] {
            let probes = every(app, id)
            XCTAssertFalse(probes.isEmpty, "no \(id)")
            for probe in probes {
                let value = probe.value as? String ?? ""
                XCTAssertTrue(value.hasSuffix("cut=false"), "\(id) was cut off: \(value)")
                if id == "needs-you-age" {
                    XCTAssertTrue(value.hasPrefix("lines=1 "), "the age wrapped: \(value)")
                }
            }
        }
    }

    /// **The Now/Next card stays a small part of the screen** (ov-423):
    /// three stacked segments at the largest size took a third of it. The
    /// card's Now and Next are a line each, so it's about a sixth.
    func testTheCardsAreASmallPartOfTheScreenAtTheLargestSize() throws {
        let app = openBilling([], size: Self.largest)
        let card = app.buttons["plan-strip"]
        let share = card.frame.height / app.frame.height
        XCTAssertLessThanOrEqual(share, 0.2, "the card takes \(Int(share * 100)) percent of the screen")
        XCTAssertFalse(app.descendants(matching: .any)["workspace-segments"].exists, "the segment bar is back")
    }

    /// **No icon sits over a title** (ov-424): in the tree the icon was
    /// drawn over the title's first letters.
    func testNoRowIconOverlapsItsTitleAtTheLargestSize() throws {
        let app = openBilling([], size: Self.largest)
        app.buttons["plan-strip"].tap()
        let bar = app.navigationBars["Plan"]
        XCTAssertTrue(bar.waitForExistence(timeout: 10), "no Plan sheet")
        bar.swipeUp()
        let icon = app.descendants(matching: .any)["tree-row-icon"]
        XCTAssertTrue(app.reveal(icon), "the tree drew no row")
        let icons = every(app, "tree-row-icon"), titles = every(app, "tree-row-title")
        XCTAssertFalse(titles.isEmpty, "the tree drew no title")
        for icon in icons {
            for title in titles {
                XCTAssertFalse(
                    icon.frame.intersects(title.frame),
                    "an icon \(icon.frame) overlaps a title \(title.frame)")
            }
        }
    }
}
