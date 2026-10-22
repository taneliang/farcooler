import XCTest

/// Captures of the Plan as sections (ov-503, phase 7): an iPhone's strip, the
/// sheet at both detents, the Plan down its whole length, and the pushed Themes
/// and Lanes lists; an iPad's three columns in landscape and two in portrait with the sidebar
/// shown, each in light and dark, with the long real titles of
/// `test/fixtures/plan-phones-ov503.json`.
///
/// Off unless `TEST_RUNNER_FARCOOLER_CAPTURE_OUT` names a folder, so it is a
/// capture tool and not a test: with it unset every method skips, which is why
/// the class sits in CAPTURES in `scripts/ios-ui-shards.py`, outside the shards.
/// nightly.yml's `captures` job sets it and uploads the images.
/// `TEST_RUNNER_FARCOOLER_CAPTURE_TAG` names the device in the files.
final class PlanSectionsCapturesUITests: XCTestCase {
    private var out: URL!
    private var tag = "device"

    override func setUpWithError() throws {
        continueAfterFailure = false
        let environment = ProcessInfo.processInfo.environment
        let path = environment["FARCOOLER_CAPTURE_OUT"]
        try XCTSkipUnless(path != nil, "a capture tool: set TEST_RUNNER_FARCOOLER_CAPTURE_OUT")
        out = URL(fileURLWithPath: path!)
        tag = environment["FARCOOLER_CAPTURE_TAG"] ?? "device"
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        XCUIDevice.shared.orientation = UIDevice.current.userInterfaceIdiom == .pad ? .landscapeLeft : .portrait
    }

    override func tearDown() {
        XCUIDevice.shared.orientation = .portrait
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-ov503.json").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func shoot(_ name: String, _ scheme: String, expecting state: CaptureState) {
        guard requireState(state, for: "\(tag)-\(name)-\(scheme)") else { return }
        Thread.sleep(forTimeInterval: 1)
        let url = out.appendingPathComponent("\(tag)-\(name)-\(scheme).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func launch(_ scheme: String, _ extra: [String] = []) -> XCUIApplication {
        XCUIDevice.shared.appearance = scheme == "dark" ? .dark : .light
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-plan-file", Self.fixture,
             "-phone-appearance", scheme] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        return app
    }

    /// The plan with the runner's link down, and the plan with nothing landing (ov-631, ov-639), in the sheet the strip opens.
    func testReconnectingAndEmptyLanding() throws {
        // The "next-empty" case of the shared fixture, as a harness file.
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        let cases = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-sections-cases.json"))) as! [String: Any]
        let empty = (cases["cases"] as! [[String: Any]]).first { $0["name"] as? String == "next-empty" }!
        let file = out.appendingPathComponent("next-empty-plan.json")
        try JSONSerialization.data(withJSONObject: ["plan": empty["plan"]!]).write(to: file)
        for scheme in ["light", "dark"] {
            XCUIDevice.shared.appearance = scheme == "dark" ? .dark : .light
            for (name, flags) in [("reconnecting", ["-phone-plan-down", "-phone-plan-file", Self.fixture]), ("empty-landing", ["-phone-plan", "-phone-plan-file", file.path])] {
                let app = XCUIApplication.phoneHarness(
                    ["-phone-empty-inbox", "-phone-billing-led", "-phone-board-reads", "-phone-appearance", scheme] + flags)
                let billing = app.buttons["workspace-row-Billing"]
                XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
                billing.tap()
                let strip = app.buttons["plan-strip"]
                XCTAssertTrue(strip.waitForExistence(timeout: 30), "no strip")
                strip.tap()
                XCTAssertTrue(element(app, "plan-sheet-orchestrator").waitForExistence(timeout: 20), "no sheet")
                // The sections sit below the tasks in the sheet: scroll to them, and take the sheet large.
                XCTAssertTrue(app.reveal(element(app, name == "reconnecting" ? "plan-reconnecting" : "plan-landing")), "no \(name)")
                shoot(name, scheme, expecting: .shows(element(app, name == "reconnecting" ? "plan-reconnecting" : "plan-landing")))
                XCTAssertTrue(element(app, name == "reconnecting" ? "plan-reconnecting" : "plan-landing").exists, "no \(name)")
                app.terminate()
            }
        }
    }

    private static func seeded(_ file: String) -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/\(file)").path
    }

    /// The Plan sheet's Cost section (ov-687, fourth batch), in light and dark,
    /// over `plan-phones-sheet.json`, the plan with every section the sheet draws.
    func testQuietCost() throws {
        try XCTSkipIf(UIDevice.current.userInterfaceIdiom == .pad, "the phone's sheet")
        for scheme in ["light", "dark"] {
            XCUIDevice.shared.appearance = scheme == "dark" ? .dark : .light
            let app = XCUIApplication.phoneHarness(
                ["-phone-empty-inbox", "-phone-billing-led", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-rulings",
                 "-phone-plan-file", Self.seeded("plan-phones-sheet.json"), "-phone-appearance", scheme])
            let billing = app.buttons["workspace-row-Billing"]
            XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
            billing.tap()
            let strip = app.buttons["plan-strip"]
            XCTAssertTrue(strip.waitForExistence(timeout: 30), "no strip")
            strip.tap()
            XCTAssertTrue(element(app, "plan-sheet-orchestrator").waitForExistence(timeout: 10), "no sheet")
            let bar = app.navigationBars["Plan"]
            bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                .press(forDuration: 0.1, thenDragTo: app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
            let header = element(app, "plan-cost-header")
            XCTAssertTrue(app.reveal(header), "no Cost in the sheet")
            shoot("quiet-cost", scheme, expecting: .when("the Cost section is in the tree") { header.exists })
            let before = XCUIScreen.main.screenshot().pngRepresentation
            app.swipeUp(velocity: .slow)
            shoot("quiet-cost-2", scheme, expecting: .changed(from: before, "the sheet scrolled down the Cost section"))
            app.terminate()
        }
    }

    /// An orchestrator's page, whose quiet text is `Ink.quiet` (ov-687, fourth
    /// batch), in light and dark. The Cost section is `NavigationCapturesUITests`'s
    /// `testPlanSheet`.
    func testQuietPage() throws {
        try XCTSkipIf(UIDevice.current.userInterfaceIdiom == .pad, "the phone's sheet")
        for scheme in ["light", "dark"] {
            XCUIDevice.shared.appearance = scheme == "dark" ? .dark : .light
            let app = XCUIApplication.phoneHarness(
                ["-phone-empty-inbox", "-phone-plan", "-phone-plan-file", Self.seeded("pages-seeded.json"), "-phone-pages", "-phone-appearance", scheme])
            let row = app.buttons["workspace-row-Billing"]
            XCTAssertTrue(row.waitForExistence(timeout: 30), "no Billing row")
            row.tap()
            XCTAssertTrue(app.reveal(element(app, "plan-landing")), "no plan")
            let train = element(app, "plan-page-train")
            for _ in 0..<14 where !(train.exists && train.isHittable) { app.swipeUp() }
            train.tap()
            XCTAssertTrue(element(app, "page-train").waitForExistence(timeout: 10), "no page")
            shoot("quiet-page", scheme, expecting: .shows(element(app, "page-train")))
            app.swipeUp()
            shoot("quiet-page-2", scheme, expecting: .shows(element(app, "page-train")))
            app.terminate()
        }
    }

    /// One screenful at a time down the list, until a swipe moves nothing.
    private func walk(_ app: XCUIApplication, _ name: String, _ scheme: String, pages: Int = 9) {
        var previous: Data?
        for page in 1...pages {
            shoot(
                "\(name)-\(page)", scheme,
                expecting: previous.map { .changed(from: $0, "the list scrolled to page \(page)") }
                    ?? .shows(app.navigationBars["Plan"]))
            let before = app.screenshot().pngRepresentation
            previous = XCUIScreen.main.screenshot().pngRepresentation
            app.swipeUp(velocity: .slow)
            app.swipeUp(velocity: .slow)
            if app.screenshot().pngRepresentation == before { break }
        }
    }

    func testThePlan() throws {
        let pad = UIDevice.current.userInterfaceIdiom == .pad
        for scheme in ["light", "dark"] {
            let app = launch(scheme)
            if pad {
                XCTAssertTrue(element(app, "plan-landing").waitForExistence(timeout: 20), "no plan")
                shoot("pad-landscape", scheme, expecting: .shows(element(app, "plan-landing")))
                let canvas = element(app, "pad-canvas")
                for page in 2...6 {
                    let before = XCUIScreen.main.screenshot().pngRepresentation
                    canvas.swipeUp(velocity: .slow)
                    canvas.swipeUp(velocity: .slow)
                    shoot("pad-landscape-plan-\(page)", scheme, expecting: .changed(from: before, "the plan scrolled to page \(page)"))
                }
                XCUIDevice.shared.orientation = .portrait
                Thread.sleep(forTimeInterval: 2)
                shoot("pad-portrait", scheme, expecting: .gone(element(app, "pad-tree"), "two columns, the tree away"))
                let toggle = app.buttons["pad-tree-toggle"]
                if toggle.waitForExistence(timeout: 5) {
                    toggle.tap()
                    XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10), "no sidebar")
                    shoot("pad-portrait-sidebar", scheme, expecting: .shows(element(app, "pad-tree")))
                }
            } else {
                let strip = app.buttons["plan-strip"]
                XCTAssertTrue(strip.waitForExistence(timeout: 30), "no strip")
                shoot("strip", scheme, expecting: .shows(strip))
                strip.tap()
                XCTAssertTrue(element(app, "plan-sheet-orchestrator").waitForExistence(timeout: 10), "no sheet")
                shoot("sheet-medium", scheme, expecting: .shows(element(app, "plan-sheet-orchestrator")))
                let bar = app.navigationBars["Plan"]
                let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                from.press(forDuration: 0.1, thenDragTo: app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
                shoot("sheet-large", scheme, expecting: .when("the Plan sheet at its large detent") {
                    bar.frame.minY < app.windows.firstMatch.frame.height * 0.2
                })
                walk(app, "sheet", scheme, pages: 8)
                // The Plan is the sheet alone (ov-616); its Lanes list is in it.
                let lanes = element(app, "plan-browse-lanes")
                for _ in 0..<14 where !(lanes.exists && lanes.isHittable) { app.swipeUp() }
                lanes.tap()
                XCTAssertTrue(element(app, "plan-browse-lanes-list").waitForExistence(timeout: 10), "no Lanes list")
                shoot("lanes-list", scheme, expecting: .shows(element(app, "plan-browse-lanes-list")))
            }
            app.terminate()
        }
    }
}
