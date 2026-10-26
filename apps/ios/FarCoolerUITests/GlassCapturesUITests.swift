import XCTest

/// Captures of the iPhone's glass controls for a review (ov-789, ov-792,
/// ov-794), light and dark: the composer at one, three and its most lines,
/// with a photo and the photo held; the Now/Next and Agents cards at rest,
/// each held, and the Plan opened from its card. Opt-in, as
/// `ConversationCapturesUITests` is: it flips the device's appearance.
/// `TEST_RUNNER_FARCOOLER_CAPTURE_OUT=<dir>` turns it on, and
/// `TEST_RUNNER_FARCOOLER_CAPTURE_TAG=<phase>-<device>` names the files.
final class GlassCapturesUITests: XCTestCase {
    private var out: String? { ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] }
    private var tag: String { ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_TAG"] ?? "capture" }

    override func setUp() {
        continueAfterFailure = true
    }

    override func tearDown() {
        XCUIDevice.shared.appearance = .light
    }

    /// `test/fixtures/<name>`, found from this file.
    private func fixture(_ name: String) -> String {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("test/fixtures/\(name)").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// `state` is what the screen must show before the shot.
    private func shoot(_ name: String, expecting state: CaptureState) {
        guard let out else { return }
        guard requireState(state, for: "\(tag)-\(name)") else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("\(tag)-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    func testComposerLight() throws { try composer("light", .light) }
    func testComposerDark() throws { try composer("dark", .dark) }
    func testCardsLight() throws { try cards("light", .light) }
    func testCardsDark() throws { try cards("dark", .dark) }

    /// Long holds and drags for frames: the glass's response to a finger is
    /// motion, so it's judged from screenshots taken outside the test
    /// (`xcrun simctl io <device> screenshot` in a loop) while these hold.
    /// `TEST_RUNNER_FARCOOLER_CAPTURE_FRAMES=1` turns them on.
    private var frames: Bool { ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_FRAMES"] != nil }

    func testHoldsForFramesComposer() throws {
        try XCTSkipIf(!frames, "frames only; set TEST_RUNNER_FARCOOLER_CAPTURE_FRAMES")
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"]
        app.launchDrawn()
        let field = element(app, "native-composer"), plus = element(app, "native-attach")
        XCTAssertTrue(field.waitForExistence(timeout: 60), "no composer")
        Thread.sleep(forTimeInterval: 2)
        // The field held, then dragged sideways; the + the same.
        let start = field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 2, thenDragTo: start.withOffset(CGVector(dx: 40, dy: -10)), withVelocity: .slow, thenHoldForDuration: 1.5)
        Thread.sleep(forTimeInterval: 1.5)
        let tip = plus.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        tip.press(forDuration: 2, thenDragTo: tip.withOffset(CGVector(dx: 30, dy: -20)), withVelocity: .slow, thenHoldForDuration: 1.5)
        Thread.sleep(forTimeInterval: 1.5)
        app.terminate()
    }

    func testHoldsForFramesCards() throws {
        try XCTSkipIf(!frames, "frames only; set TEST_RUNNER_FARCOOLER_CAPTURE_FRAMES")
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", fixture("plan-seeded.json"), "-native-agents"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30), "no Main row")
        main.tap()
        let plan = app.buttons["plan-strip"], agents = app.buttons["agents-card"]
        XCTAssertTrue(agents.waitForExistence(timeout: 30), "no Agents card")
        Thread.sleep(forTimeInterval: 2)
        // A drag that starts on a card and leaves it, so no tap: the glass's
        // swell, without opening anything.
        let start = plan.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.3, thenDragTo: start.withOffset(CGVector(dx: 60, dy: 200)), withVelocity: .slow, thenHoldForDuration: 1)
        Thread.sleep(forTimeInterval: 1.5)
        // The tap, which grows the sheet out of the card, then Done.
        plan.tap()
        XCTAssertTrue(element(app, "plan-sheet").waitForExistence(timeout: 15))
        Thread.sleep(forTimeInterval: 2)
        app.buttons["plan-sheet-done"].tap()
        Thread.sleep(forTimeInterval: 2)
        agents.tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForExistence(timeout: 15))
        Thread.sleep(forTimeInterval: 2)
        app.terminate()
    }

    /// Messages, for the side-by-side (ov-789): its field and + held and
    /// dragged, as `testHoldsForFramesComposer` does ours. With
    /// `TEST_RUNNER_FARCOOLER_CAPTURE_FRAMES=1`.
    func testHoldsForFramesMessages() throws {
        try XCTSkipIf(!frames, "frames only; set TEST_RUNNER_FARCOOLER_CAPTURE_FRAMES")
        let messages = XCUIApplication(bundleIdentifier: "com.apple.MobileSMS")
        messages.launch()
        Thread.sleep(forTimeInterval: 3)
        // The simulator's Messages opens on its list of conversations: the
        // first one, for a field with a + beside it.
        let conversation = messages.cells.firstMatch
        if conversation.waitForExistence(timeout: 10) { conversation.tap() }
        let anyField = messages.textFields["messageBodyField"].firstMatch
        XCTAssertTrue(anyField.waitForExistence(timeout: 10), "no message field in Messages: \(messages.debugDescription)")
        Thread.sleep(forTimeInterval: 1)
        if let out { try? XCUIScreen.main.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: out).appendingPathComponent("\(tag)-messages-rest.png")) }
        let start = anyField.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 2, thenDragTo: start.withOffset(CGVector(dx: 40, dy: -10)), withVelocity: .slow, thenHoldForDuration: 1.5)
        Thread.sleep(forTimeInterval: 1.5)
        let plus = messages.buttons["add"].firstMatch
        if plus.exists {
            let tip = plus.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            tip.press(forDuration: 2, thenDragTo: tip.withOffset(CGVector(dx: 30, dy: -20)), withVelocity: .slow, thenHoldForDuration: 1.5)
        }
        Thread.sleep(forTimeInterval: 1.5)
        messages.terminate()
    }

    /// Words with descenders on every line, so a fade over the caret's line
    /// shows as cut tails.
    private static let lines = [
        "Typing past the edge: pg qy jp", "gypsy jumping pug", "quietly paying the toggle",
        "a gap, a jog, a yogurt", "jpeg, png, svg", "piggy bank, ginger, query", "yet another long typing line",
        "and the caret's line, typing",
    ]

    private func composer(_ scheme: String, _ appearance: XCUIDevice.Appearance) throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT to take them")
        XCUIDevice.shared.appearance = appearance
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-app.theme", scheme == "light" ? "Solarized Light" : "Nord"]
        app.launchDrawn()
        let field = element(app, "native-composer")
        XCTAssertTrue(field.waitForExistence(timeout: 60), "no composer")
        shoot("composer-rest-\(scheme)", expecting: .shows(field, "the empty composer"))
        field.tap()
        XCTAssertTrue(wait(30) { field.value(forKey: "hasKeyboardFocus") as? Bool == true }, "no keyboard")
        app.typeText(Self.lines[0])
        Thread.sleep(forTimeInterval: 1)
        shoot("composer-1-line-\(scheme)", expecting: .when("one line typed") { (field.value as? String ?? "").hasSuffix("pg qy jp") })
        for line in Self.lines[1...2] { app.typeText("\n" + line) }
        Thread.sleep(forTimeInterval: 1)
        shoot("composer-3-lines-\(scheme)", expecting: .when("three lines typed") { (field.value as? String ?? "").hasSuffix("toggle") })
        for line in Self.lines[3...] { app.typeText("\n" + line) }
        Thread.sleep(forTimeInterval: 1)
        shoot("composer-most-lines-\(scheme)", expecting: .when("eight lines typed") { (field.value as? String ?? "").hasSuffix("typing") })
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.native-photo" as CFString), nil, nil, true)
        let chip = element(app, "native-image-chip")
        XCTAssertTrue(chip.waitForExistence(timeout: 60), "no chip")
        Thread.sleep(forTimeInterval: 1)
        shoot("composer-photo-\(scheme)", expecting: .shows(chip, "a photo chip"))
        chip.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.7)).press(forDuration: 1.2)
        let preview = element(app, "native-image-preview")
        _ = preview.waitForExistence(timeout: 10)
        Thread.sleep(forTimeInterval: 1)
        shoot("composer-photo-held-\(scheme)", expecting: .shows(preview, "the held photo's preview"))
        app.terminate()
    }

    private func cards(_ scheme: String, _ appearance: XCUIDevice.Appearance) throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT to take them")
        XCUIDevice.shared.appearance = appearance
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", fixture("plan-seeded.json"),
             "-native-agents", "-app.theme", scheme == "light" ? "Solarized Light" : "Nord"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30), "no Main row")
        main.tap()
        let plan = app.buttons["plan-strip"], agents = app.buttons["agents-card"]
        XCTAssertTrue(plan.waitForExistence(timeout: 30), "no Now/Next card")
        XCTAssertTrue(agents.waitForExistence(timeout: 30), "no Agents card")
        Thread.sleep(forTimeInterval: 2)
        shoot("cards-rest-\(scheme)", expecting: .shows(plan, "the Now/Next card"))

        plan.press(forDuration: 1.2)
        let planMenu = element(app, "plan-strip-preview")
        _ = planMenu.waitForExistence(timeout: 10)
        Thread.sleep(forTimeInterval: 1)
        shoot("cards-plan-held-\(scheme)", expecting: .shows(planMenu, "the Plan's preview"))
        dismissMenu(app)
        XCTAssertFalse(planMenu.exists, "the Plan's menu stayed up")

        agents.press(forDuration: 1.2)
        let agentsMenu = element(app, "agents-card-preview")
        _ = agentsMenu.waitForExistence(timeout: 10)
        Thread.sleep(forTimeInterval: 1)
        shoot("cards-agents-held-\(scheme)", expecting: .shows(agentsMenu, "the agents' preview"))
        dismissMenu(app)

        plan.tap()
        let sheet = element(app, "plan-sheet")
        XCTAssertTrue(sheet.waitForExistence(timeout: 15), "the Plan never opened")
        Thread.sleep(forTimeInterval: 1.5)
        shoot("cards-plan-open-\(scheme)", expecting: .shows(sheet, "the Plan, opened from its card"))
        app.terminate()
    }

    /// A tap outside the menu, low on the screen, puts it away. (At the top it
    /// landed on the status bar, and the menu stayed.)
    private func dismissMenu(_ app: XCUIApplication) {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.7)).tap()
        _ = app.otherElements["PopoverDismissRegion"].waitForNonExistence(timeout: 10)
        Thread.sleep(forTimeInterval: 1)
    }
}
