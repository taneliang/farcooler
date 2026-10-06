import XCTest

/// Working… sweeps on a layer, not on the main thread (ov-382).
///
/// `WorkingRow`'s sweep was a `TimelineView(.animation)`, SwiftUI's update at
/// the display's rate for the whole of a turn. It's a Core Animation mask now
/// (`ShimmerBand`); the row's debug value says whether that animation is on
/// the layer. On `-agent-layout-harness`, whose pane is working.
final class WorkingShimmerTests: XCTestCase {
    func testTheSweepRunsOnALayer() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launchDrawn()
        let row = app.descendants(matching: .any).matching(identifier: "agent-working").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 30), "the harness drew no Working… row")
        let onLayer = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value == %@", "sweep=layer"), object: row)
        XCTAssertEqual(
            XCTWaiter.wait(for: [onLayer], timeout: 10), .completed,
            "the sweep isn't on a layer: \(row.value ?? "no value")")
    }
}
