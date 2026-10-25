import XCTest

/// Captures for the phone polish sweep (ov-412): each shipped screen at the
/// iPhone's two extremes (Dynamic Type at its largest accessibility size,
/// which is the narrowest the text ever gets, and landscape, the widest the
/// screen ever is) and at the normal size, light and dark, with the long real
/// content of `test/fixtures/plan-rulings-long.json`.
///
/// Off unless `TEST_RUNNER_FARCOOLER_CAPTURE_OUT` names a folder, so it is a
/// capture tool and not a test: with it unset every method skips, which is why
/// the class sits in CAPTURES in `scripts/ios-ui-shards.py`, outside the shards.
/// nightly.yml's `captures` job sets it and uploads the images.
final class PolishSweepUITests: XCTestCase {
    private struct Variant {
        let name: String
        let dark: Bool
        let big: Bool
        let landscape: Bool
        var arguments: [String] {
            var args: [String] = []
            // The harness's own switch: `-AppleInterfaceStyle Dark` never reached it, so every dark capture
            // came out light, byte for byte (found by capture-duplicates.sh, ov-642).
            args += ["-phone-appearance", dark ? "dark" : "light"]
            // The conversation's ground is the terminal theme's, not the system's (ov-342): the window's
            // Light left every conversation and orchestrator capture dark, byte for byte the dark one's
            // (ov-732). ConversationCapturesUITests names its theme the same way.
            args += ["-app.theme", dark ? "Nord" : "Solarized Light"]
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

    private func shoot(_ screen: String, _ variant: Variant, expecting state: CaptureState) {
        guard requireState(state, for: "phone-\(screen)-\(variant.name)") else { return }
        Thread.sleep(forTimeInterval: 1)
        let url = out.appendingPathComponent("phone-\(screen)-\(variant.name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func each(_ names: [String]? = nil, _ body: (Variant) -> Void) {
        for variant in Self.variants where names?.contains(variant.name) ?? true {
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

    /// The Plan sheet, from the Now/Next card, at full height (ov-616).
    private func openPlan(_ app: XCUIApplication) {
        let card = app.buttons["plan-strip"]
        XCTAssertTrue(card.waitForExistence(timeout: 10), "no Now/Next card")
        card.tap()
        let bar = app.navigationBars["Plan"]
        XCTAssertTrue(bar.waitForExistence(timeout: 10), "no Plan sheet")
        // At its large detent, its bar near the top, before anything reads a
        // frame: a frame read while the sheet still moves is scaled, and a
        // drag then lands on the sheet rather than its list. Dragged up by its
        // grabber, the strip above the bar, and again if one is lost. Not by
        // the bar: at the largest type size a drag from the bar's middle was
        // lost on 1 sheet in 3 (Oct 25's nightly: 1 in light-narrow, all 3 in
        // dark-narrow, one in the tree's; locally 2 of 6 sheets), never at the
        // standard size. By the grabber, 8 sheets of 8 locally took one drag
        // (ov-703).
        let window = app.windows.firstMatch
        let sheet = app.descendants(matching: .any)["plan-sheet"]
        let large = { bar.exists && bar.frame.minY < window.frame.height * 0.2 }
        var drags = 0
        for _ in 0..<3 where !large() {
            drags += 1
            let grabber = window.coordinate(withNormalizedOffset: .zero)
                .withOffset(CGVector(dx: window.frame.midX, dy: sheet.frame.minY + 6))
            grabber.press(forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
            _ = XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in large() }, object: nil)], timeout: 3)
        }
        print("polish-sweep: the Plan sheet reached its large detent after \(drags) drag(s)")
        XCTAssertTrue(large(), "the Plan sheet didn't reach its large detent")
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
            shoot("needs-you", variant, expecting: .shows(
                app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'needs-you-item-'")).firstMatch,
                "Needs You with its asks"))
            app.terminate()
        }
    }

    // Two variants a method: the six in one took 254 s for five on an idle
    // Mac, too near the six-minute allowance a test gets on the nightly's
    // slower runner.
    func testPlanAndRulings() { planAndRulings(["light", "dark"]) }
    func testPlanAndRulingsNarrow() { planAndRulings(["light-narrow", "dark-narrow"]) }
    func testPlanAndRulingsWide() { planAndRulings(["light-wide", "dark-wide"]) }

    private func planAndRulings(_ names: [String]) {
        each(names) { variant in
            let app = billing(variant, ["-phone-board-reads", "-phone-plan", "-phone-rulings", "-phone-plan-file", Self.fixture("plan-rulings-long.json")])
            openPlan(app)
            // In the sheet (c602dfd49) the landing section sits under the
            // tree, below the first screen, and the list is lazy: scrolled to.
            let landing = app.descendants(matching: .any)["plan-landing"]
            XCTAssertTrue(landing.waitForExistence(timeout: 5) || app.reveal(landing), "no plan")
            shoot("plan", variant, expecting: .shows(landing))
            // Scrolled to, then brought up the screen by a drag that holds at
            // its end, and "in reach" is its frame on the screen. Its first
            // match: more than one element carries the id, so the old
            // `isHittable` on the query could never hold.
            let rulings = app.descendants(matching: .any)["plan-rulings"].firstMatch
            let window = app.windows.firstMatch
            let inView = { rulings.exists && rulings.frame.minY > window.frame.height * 0.1 && rulings.frame.minY < window.frame.height * 0.7 }
            XCTAssertTrue(rulings.exists || app.reveal(rulings), "no rulings")
            // Into view from either side: a swipe in reveal can carry it
            // past the top as well as leave it below the fold.
            let height = window.frame.height
            let origin = window.coordinate(withNormalizedOffset: .zero)
            let x = window.frame.width / 2
            for _ in 0..<6 where rulings.exists && !inView() {
                let below = rulings.frame.minY >= height * 0.7
                let (from, to) = below ? (0.75, 0.35) : (0.35, 0.75)
                origin.withOffset(CGVector(dx: x, dy: height * from)).press(
                    forDuration: 0.05, thenDragTo: origin.withOffset(CGVector(dx: x, dy: height * to)),
                    withVelocity: .slow, thenHoldForDuration: 0.3)
            }
            shoot("rulings", variant, expecting: .when("the rulings on the screen") { inView() })
            let beforeLower = XCUIScreen.main.screenshot().pngRepresentation
            // A drag in the window's own points: `app.swipeUp()` moved
            // nothing in landscape (light-wide, locally).
            origin.withOffset(CGVector(dx: x, dy: height * 0.75)).press(
                forDuration: 0.05, thenDragTo: origin.withOffset(CGVector(dx: x, dy: height * 0.35)),
                withVelocity: .slow, thenHoldForDuration: 0.3)
            shoot("rulings-lower", variant, expecting: .changed(from: beforeLower, "the rulings scrolled lower"))
            app.terminate()
        }
    }

    func testTheTree() {
        each { variant in
            // `-phone-native` serves the orchestrator's conversation; without it the pane answered "not in
            // the harness" and the shot was "Couldn't Load This Pane" in every variant (ov-732).
            let app = billing(variant, ["-phone-native", "-phone-plan", "-phone-plan-file", Self.fixture("plan-rulings-long.json")])
            XCTAssertTrue(app.buttons["plan-strip"].waitForExistence(timeout: 10), "no Now/Next card")
            // The composer only a loaded conversation has; the error pane has the strip too.
            let composer = app.descendants(matching: .any)["native-composer"]
            shoot("orchestrator", variant, expecting: .all(.shows(app.buttons["plan-strip"]), .shows(composer, "the orchestrator's composer")))
            openPlan(app)
            Thread.sleep(forTimeInterval: 2)
            shoot("tree", variant, expecting: .shows(app.navigationBars["Plan"]))
            app.terminate()
        }
    }

    // One method per kind of ask: the three in one method were eighteen
    // launches, 354 s of the six-minute allowance a test gets on the last
    // green night (Oct 16), the same overrun that took
    // ConversationCapturesUITests down every night after.
    func testTheConversationPermission() { conversation("permission") }
    func testTheConversationQuestion() { conversation("question") }
    func testTheConversationPlan() { conversation("plan") }

    private func conversation(_ kind: String) {
        each { variant in
            let app = XCUIApplication()
            app.launchArguments = variant.arguments + ["-native-agent-harness", "-native-held-ask", kind]
            app.launchDrawn()
            XCTAssertTrue(app.descendants(matching: .any)["native-row-ask:h1"].waitForExistence(timeout: 60), "no ask")
            shoot("conversation-\(kind)", variant, expecting: .shows(app.descendants(matching: .any)["native-row-ask:h1"]))
            app.terminate()
        }
    }
}
