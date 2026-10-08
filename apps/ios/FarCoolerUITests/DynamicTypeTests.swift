import XCTest

/// Accessibility-size Dynamic Type, which broke three controls: the workspace
/// segment bar ("Orches-/trator"), the queued message's actions ("Queu/ed",
/// "Re-/move") and the composer's chips ("Ma… S… Hi…").
///
/// The size comes in as a launch argument, which the app reads as its own
/// preferred content size. Each test asserts a SHAPE rather than a pixel: a
/// label that cannot wrap is as tall as the label beside it that has one word,
/// and a chip that is not truncated is as wide as its own text.
final class DynamicTypeTests: XCTestCase {
    private static let size = [
        "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL",
    ]

    /// **The segment bar keeps every title on one line.** "Orchestrator" is the
    /// longest word and "Board" the shortest, so a bar that lets the long one
    /// wrap makes its button taller than the other's.
    func testTheSegmentBarNeverWrapsATitle() throws {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox"] + Self.size)
        let row = app.buttons["workspace-row-Main"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no Main row")
        row.tap()
        let board = app.buttons["segment-board"]
        XCTAssertTrue(board.waitForExistence(timeout: 10), "the workspace did not open")
        for id in ["segment-orchestrator", "segment-tree"] {
            let segment = app.buttons[id]
            XCTAssertTrue(segment.exists, "no \(id)")
            XCTAssertEqual(
                segment.frame.height, board.frame.height, accuracy: 2,
                "\(id) is taller than a one-word title: it wrapped")
        }
    }

    /// **The queue's actions and the composer's chips hold up.** Each action is
    /// one line tall, and each chip is as wide as its own text needs, which a
    /// truncated "Ma…" is not.
    func testTheQueueAndTheChipsAreNotBroken() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness"] + Self.size
        app.launchDrawn()
        let send = app.buttons["Send Now"]
        XCTAssertTrue(send.waitForExistence(timeout: 30), "the queued message was not drawn")
        let edit = app.buttons["Edit"]
        XCTAssertEqual(
            send.frame.height, edit.frame.height, accuracy: 2, "Send Now wrapped")
        XCTAssertEqual(
            app.buttons["Remove"].frame.height, edit.frame.height, accuracy: 2, "Remove wrapped")
        for name in ["Manual", "Sonnet", "High"] {
            let chip = app.buttons[name]
            XCTAssertTrue(chip.waitForExistence(timeout: 5), "no \(name) chip")
            XCTAssertGreaterThan(
                chip.frame.width, CGFloat(name.count) * 14,
                "\(name) was squeezed to \(chip.frame.width) points")
        }
        XCTAssertFalse(
            app.descendants(matching: .any)["adapter-badge"].exists,
            "the ACP badge still takes room at this size")
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
        XCTAssertTrue(app.buttons["segment-tree"].waitForExistence(timeout: 10), "no Themes segment")
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

    /// **The segment bar stays a small part of the screen** (ov-423): three
    /// stacked rows at the largest size took a third of it.
    func testTheSegmentBarIsASmallPartOfTheScreenAtTheLargestSize() throws {
        let app = openBilling([], size: Self.largest)
        let bar = app.descendants(matching: .any)["workspace-segments"]
        XCTAssertTrue(bar.waitForExistence(timeout: 10), "no segment bar")
        let share = bar.frame.height / app.frame.height
        XCTAssertLessThanOrEqual(share, 0.15, "the bar takes \(Int(share * 100)) percent of the screen")
        for id in ["segment-orchestrator", "segment-tree", "segment-board"] {
            XCTAssertTrue(app.buttons[id].exists, "no \(id)")
        }
    }

    /// **No icon sits over a title** (ov-424): in the Themes tree and in the
    /// plan's Now card the icon was drawn over the title's first letters.
    func testNoRowIconOverlapsItsTitleAtTheLargestSize() throws {
        let app = openBilling([], size: Self.largest)
        app.buttons["segment-tree"].tap()
        let icon = app.descendants(matching: .any)["tree-row-icon"]
        XCTAssertTrue(icon.waitForExistence(timeout: 15), "the tree drew no row")
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

    func testThePlanCardIconDoesNotOverlapItsNameAtTheLargestSize() throws {
        let app = openBilling(["-phone-board-reads"], size: Self.largest, plan: "plan-rulings-long.json")
        app.buttons["segment-board"].tap()
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control")
        control.buttons["Plan"].tap()
        let mark = app.descendants(matching: .any)["plan-lane-mark"]
        XCTAssertTrue(mark.waitForExistence(timeout: 15), "the plan drew no lane")
        for mark in every(app, "plan-lane-mark") {
            for name in every(app, "plan-lane-name") {
                XCTAssertFalse(
                    mark.frame.intersects(name.frame),
                    "a mark \(mark.frame) overlaps a name \(name.frame)")
            }
        }
    }
}
