import XCTest

/// Captures for the phone polish sweep (ov-412): each shipped screen at the
/// iPhone's two extremes (Dynamic Type at its largest accessibility size,
/// which is the narrowest the text ever gets, and landscape, the widest the
/// screen ever is) and at the normal size, light and dark, with the long real
/// content of `test/fixtures/plan-rulings-long.json`.
///
/// Off unless `TEST_RUNNER_FARCOOLER_CAPTURE_OUT` names a folder, so it is a
/// capture tool and not a test: with it unset every method skips, which is why
/// the class sits in LOCAL in `scripts/ios-ui-shards.py`.
final class PolishSweepUITests: XCTestCase {
    private struct Variant {
        let name: String
        let dark: Bool
        let big: Bool
        let landscape: Bool
        var arguments: [String] {
            var args: [String] = []
            if dark { args += ["-AppleInterfaceStyle", "Dark"] }
            if big { args += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"] }
            return args
        }
    }

    private static let variants = [
        Variant(name: "light", dark: false, big: false, landscape: false),
        Variant(name: "dark", dark: true, big: false, landscape: false),
        Variant(name: "light-narrow", dark: false, big: true, landscape: false),
        Variant(name: "dark-narrow", dark: true, big: true, landscape: false),
        Variant(name: "light-wide", dark: false, big: false, landscape: true),
        Variant(name: "dark-wide", dark: true, big: false, landscape: true),
    ]

    private var out: URL!

    override func setUpWithError() throws {
        continueAfterFailure = false
        let path = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]
        try XCTSkipUnless(path != nil, "a capture tool: set TEST_RUNNER_FARCOOLER_CAPTURE_OUT")
        out = URL(fileURLWithPath: path!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        XCUIDevice.shared.orientation = .portrait
    }

    override func tearDown() {
        XCUIDevice.shared.orientation = .portrait
    }

    private static func fixture(_ name: String) -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/\(name)").path
    }

    private func shoot(_ screen: String, _ variant: Variant) {
        Thread.sleep(forTimeInterval: 1)
        let url = out.appendingPathComponent("phone-\(screen)-\(variant.name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func each(_ body: (Variant) -> Void) {
        for variant in Self.variants {
            XCUIDevice.shared.orientation = variant.landscape ? .landscapeLeft : .portrait
            body(variant)
        }
    }

    private func billing(_ variant: Variant, _ extra: [String]) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(variant.arguments + ["-phone-empty-inbox", "-phone-billing-led"] + extra)
        let row = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(row.waitForExistence(timeout: 8) || scrollTo(row, app), "no Billing row")
        row.tap()
        return app
    }

    /// The list is lazy and the narrow variant's rows are tall, so Billing can be below the fold.
    private func scrollTo(_ row: XCUIElement, _ app: XCUIApplication) -> Bool {
        for _ in 0..<10 where !(row.exists && row.isHittable) { app.swipeUp() }
        return row.exists
    }

    func testNeedsYou() {
        each { variant in
            let app = XCUIApplication.phoneHarness(variant.arguments)
            _ = app.buttons.firstMatch.waitForExistence(timeout: 20)
            shoot("needs-you", variant)
            app.terminate()
        }
    }

    func testPlanAndRulings() {
        each { variant in
            let app = billing(variant, ["-phone-board-reads", "-phone-plan", "-phone-rulings", "-phone-plan-file", Self.fixture("plan-rulings-long.json")])
            app.buttons["segment-board"].tap()
            let control = app.segmentedControls["plan-switch"]
            XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control")
            control.buttons["Plan"].tap()
            XCTAssertTrue(app.descendants(matching: .any)["plan-now"].waitForExistence(timeout: 10), "no plan")
            shoot("plan", variant)
            let rulings = app.descendants(matching: .any)["plan-rulings"]
            for _ in 0..<8 where !(rulings.exists && rulings.isHittable) { app.swipeUp() }
            shoot("rulings", variant)
            app.swipeUp()
            shoot("rulings-lower", variant)
            app.terminate()
        }
    }

    func testTheTree() {
        each { variant in
            let app = billing(variant, ["-phone-plan", "-phone-plan-file", Self.fixture("plan-rulings-long.json")])
            let tree = app.buttons["segment-tree"]
            XCTAssertTrue(tree.waitForExistence(timeout: 10), "no Themes segment")
            shoot("orchestrator", variant)
            tree.tap()
            Thread.sleep(forTimeInterval: 2)
            shoot("tree", variant)
            app.terminate()
        }
    }

    func testTheConversation() {
        for kind in ["permission", "question", "plan"] {
            each { variant in
                let app = XCUIApplication()
                app.launchArguments = variant.arguments + ["-native-agent-harness", "-native-held-ask", kind]
                app.launchDrawn()
                XCTAssertTrue(app.descendants(matching: .any)["native-row-ask:h1"].waitForExistence(timeout: 60), "no ask")
                shoot("conversation-\(kind)", variant)
                app.terminate()
            }
        }
    }
}
