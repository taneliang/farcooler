import XCTest

/// A task key in terminal output is a link, as a URL there is (ov-215).
///
/// `-phone-terminal-key` makes the harness runner answer the shell pane's
/// screen with a line naming bil-9, a task on Billing's board. A long press on
/// the key offers the task (the same dialog a URL gets), and Open Task opens it
/// in the app. A long press anywhere else on the screen offers nothing.
final class TerminalTaskKeyTests: XCTestCase {
    private static let shell = "0198f2c0-0000-7000-8000-00000000d003"

    private func launch() -> XCUIApplication {
        XCUIApplication.phoneHarness(["-phone-terminal-key", "-deep-link", Self.shell])
    }

    private func visibleSurface(_ app: XCUIApplication) -> XCUIElement? {
        let all = app.otherElements.matching(identifier: "terminal-surface")
        for i in 0..<all.count {
            let element = all.element(boundBy: i)
            if element.exists, let value = element.value as? String, value.contains("visible=1") { return element }
        }
        return nil
    }

    /// A press `x` points in and `y` points down from the surface's corner.
    private func press(_ surface: XCUIElement, x: CGFloat, y: CGFloat) {
        surface.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: y))
            .press(forDuration: 1.2)
    }

    private func waitForSurface(_ app: XCUIApplication) throws -> XCUIElement {
        let deadline = Date().addingTimeInterval(60)
        repeat {
            if let surface = visibleSurface(app) { return surface }
            Thread.sleep(forTimeInterval: 0.5)
        } while Date() < deadline
        throw HarnessFailure("the shell pane never drew: \(app.debugDescription)")
    }

    func testALongPressOnAKeyOffersItsTaskAndOpensIt() throws {
        let app = launch()
        let surface = try waitForSurface(app)
        // The key opens the first row, so its cells are the first few: well
        // inside them at this font and the grid's padding.
        // `cell=` is the row's height, which the surface publishes; the grid's
        // own padding is about six points.
        let value = (surface.value as? String) ?? ""
        let rowHeight = value.split(separator: " ").first { $0.hasPrefix("cell=") }
            .flatMap { Double($0.dropFirst(5)) } ?? 17
        press(surface, x: 24, y: 6 + CGFloat(rowHeight) / 2)
        let open = app.buttons["Open Task"]
        XCTAssertTrue(open.waitForExistence(timeout: 10), "a long press on bil-9 offered nothing; surface \(surface.frame) \(value)")
        XCTAssertTrue(app.buttons["Copy"].exists)
        XCTAssertFalse(app.buttons["Open Link"].exists, "a task key is not a URL")
        open.tap()
        let heading = app.descendants(matching: .any).matching(identifier: "task-heading").firstMatch
        XCTAssertTrue(heading.waitForExistence(timeout: 15), "Open Task did not open the task")
        XCTAssertTrue(heading.label.contains("bil-9"), heading.label)
    }

    func testALongPressAwayFromAKeyOffersNothing() throws {
        let app = launch()
        let surface = try waitForSurface(app)
        // The empty rows below the line.
        press(surface, x: 60, y: 160)
        XCTAssertFalse(app.buttons["Open Task"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["Open Link"].exists)
    }
}
