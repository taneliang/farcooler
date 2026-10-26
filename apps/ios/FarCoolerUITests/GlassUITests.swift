import XCTest

/// The composer and the cards over a conversation, held and typed into
/// (ov-789, ov-792, ov-794), on the in-app harnesses: no runner.
///
/// - The field's edge fade is never over the caret's line, at one, three and
///   eight lines, and the bottom fades only with text out of view below it
///   (`ComposerField.fade`, read off the composer's DEBUG probe).
/// - A photo waiting in the field, held, shows large, with Remove.
/// - Each card, held, shows its preview and quick actions, and a tap opens
///   its sheet.
final class GlassUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
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

    // MARK: The composer

    private func composer() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"]
        app.launchDrawn()
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 60), "the conversation never showed")
        return app
    }

    /// The probe's `fade=… caretFaded=…`: the edges the field fades, and
    /// whether their fade lies over the caret's line, measured from the
    /// geometry rather than the rule (`ComposerFade.covers`).
    private func fade(_ app: XCUIApplication) -> String {
        let said = element(app, "native-composer-probe").value as? String ?? ""
        guard let at = said.range(of: "fade="), let end = said.range(of: " glass=") else { return said }
        return String(said[at.lowerBound..<end.lowerBound])
    }

    /// The probe's `glass=live|still`: whether the field's glass answers a
    /// touch, or stood down for a held photo's menu.
    private func glass(_ app: XCUIApplication) -> String {
        let said = element(app, "native-composer-probe").value as? String ?? ""
        guard let at = said.range(of: "glass=") else { return said }
        return String(said[at.upperBound...])
    }

    private func postPhoto() {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.native-photo" as CFString), nil, nil, true)
    }

    /// Hold the photo in its lower left, clear of its remove badge, and wait
    /// for its preview.
    @discardableResult
    private func holdPhoto(_ app: XCUIApplication) -> XCUIElement {
        let chip = element(app, "native-image-chip")
        XCTAssertTrue(chip.waitForExistence(timeout: 60), "no chip for the photo")
        chip.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.7)).press(forDuration: 1.2)
        let preview = element(app, "native-image-preview")
        XCTAssertTrue(preview.waitForExistence(timeout: 10), "holding the photo showed no preview: \(app.debugDescription)")
        return preview
    }

    /// Words with tails on every line.
    private static let lines = [
        "Typing past the edge: pg qy jp", "gypsy jumping pug", "quietly paying the toggle", "a gap, a jog, a yogurt",
        "jpeg, png, svg", "piggy bank, ginger, query", "yet another long typing line", "and the caret's line, typing",
    ]

    /// **The caret's line is never under the field's fade, and the bottom
    /// fades only while text is out of view below** (ov-789), in the real
    /// text view. At eight lines the field scrolls; with the caret at the end
    /// the old rule faded the bottom over it, cutting the tails of its p's and
    /// g's, because the text's own 2 pt inset counted as more below. That
    /// geometry depends on the field's height (its six lines with a hardware
    /// keyboard, a share of the room over the onscreen one), so the rule is
    /// held at it exactly by AgentKit's `ComposerFadeTests`; this holds the
    /// field's use of it.
    func testTheFadeNeverCoversTheCaretsLine() {
        let app = composer()
        let field = element(app, "native-composer")
        XCTAssertTrue(field.waitForExistence(timeout: 60))
        field.tap()
        XCTAssertTrue(wait(30) { field.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the box never took the keyboard")

        app.typeText(Self.lines[0])
        XCTAssertTrue(wait(30) { (field.value as? String ?? "").hasSuffix("jp") })
        XCTAssertTrue(wait(5) { self.fade(app) == "fade=none caretFaded=0" }, "one line: \(fade(app))")

        for line in Self.lines[1...2] { app.typeText("\n" + line) }
        XCTAssertTrue(wait(30) { (field.value as? String ?? "").hasSuffix("toggle") })
        XCTAssertTrue(wait(5) { self.fade(app) == "fade=none caretFaded=0" }, "three lines: \(fade(app))")

        for line in Self.lines[3...] { app.typeText("\n" + line) }
        XCTAssertTrue(wait(30) { (field.value as? String ?? "").hasSuffix("line, typing") })
        // Scrolled to the caret at the end: the top fades over the lines out
        // of view above, and nothing fades below.
        XCTAssertTrue(wait(5) { self.fade(app) == "fade=above caretFaded=0" }, "eight lines, typing at the end: \(fade(app))")

        // The caret to the start: the field scrolls to its top, and now the
        // lines below are out of view, so the bottom fades, and the top
        // doesn't.
        // A bare key first, which moves nothing at the end: the first key
        // XCUI presses after typeText can arrive without its modifiers
        // (`NativeComposerTests.testCommandReturnSends`), and ⌘↑ came as ↑,
        // one line up, still scrolled to the end.
        app.typeKey(XCUIKeyboardKey.rightArrow, modifierFlags: [])
        app.typeKey(XCUIKeyboardKey.upArrow, modifierFlags: .command)
        XCTAssertTrue(wait(10) { self.fade(app) == "fade=below caretFaded=0" }, "eight lines, the caret at the start: \(fade(app))")
    }

    /// **A photo waiting in the field, held, shows large, with Remove**
    /// (ov-792), and Remove takes it out of the message.
    func testHoldingAPhotoPreviewsItWithRemove() {
        let app = composer()
        postPhoto()
        let chip = element(app, "native-image-chip")
        XCTAssertTrue(chip.waitForExistence(timeout: 60), "no chip for the photo")
        let small = chip.frame
        let preview = holdPhoto(app)
        XCTAssertGreaterThan(preview.frame.width, small.width * 3, "the preview \(preview.frame) isn't large beside the chip \(small)")
        let remove = element(app, "native-image-preview-remove")
        XCTAssertTrue(remove.waitForExistence(timeout: 10), "no Remove under the preview")
        XCTAssertEqual(remove.label, "Remove Photo", "the menu names it as the badge does")
        // Over the field: its interactive glass took this tap, until it
        // stood down for the menu.
        remove.tap()
        XCTAssertTrue(wait(30) { !chip.exists }, "Remove left the photo in the message")
        XCTAssertFalse(preview.exists, "the preview stayed after Remove")
        XCTAssertTrue(wait(10) { self.glass(app) == "live" }, "the field's glass stayed still after Remove: \(glass(app))")
    }

    /// **Each photo, held, previews that photo** (ov-805). Two strips cut
    /// from a screenshot (`-native-wide-photos`), filled into their squares,
    /// were drawn about 100 pt past each side, and took touches there: the
    /// right one's covered the whole of the left chip, so holding the left
    /// photo previewed the right one. Each preview says which photo it is.
    func testHoldingEachPhotoPreviewsThatPhoto() {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-wide-photos"]
        app.launchDrawn()
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 60), "the conversation never showed")
        let chips = app.descendants(matching: .any).matching(identifier: "native-image-chip")
        postPhoto()
        XCTAssertTrue(wait(60) { chips.count == 1 }, "no chip for the first photo")
        postPhoto()
        XCTAssertTrue(wait(60) { chips.count == 2 }, "no chip for the second photo")
        let badges = app.buttons.matching(identifier: "native-image-remove")
        XCTAssertLessThan(badges.element(boundBy: 0).frame.midX, badges.element(boundBy: 1).frame.midX, "the chips aren't left to right")
        for i in 0..<2 {
            let photo = chips.element(boundBy: i).value as? String
            XCTAssertNotNil(photo, "chip \(i + 1) doesn't say which photo it is")
            // Where the photo is seen, from its remove badge, whose 44 pt
            // target ends at the chip's top right corner; not from the chip's
            // own frame, which reached as far as the photo's touches did. A
            // point in its lower left, clear of the badge.
            let badge = badges.element(boundBy: i).frame
            app.coordinate(withNormalizedOffset: .zero)
                .withOffset(CGVector(dx: badge.maxX - 45, dy: badge.minY + 45))
                .press(forDuration: 1.2)
            let preview = element(app, "native-image-preview")
            XCTAssertTrue(preview.waitForExistence(timeout: 10), "holding photo \(i + 1) showed no preview")
            XCTAssertEqual(preview.value as? String, photo, "holding photo \(i + 1) of 2 previewed another photo")
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
            XCTAssertTrue(preview.waitForNonExistence(timeout: 10), "the preview stayed")
        }
    }

    /// **The field's glass comes back however the menu goes** (ov-792 review):
    /// it stands down while a held photo's menu is up, and answers a touch
    /// again once the menu is dismissed, and once the app has left the
    /// foreground with the menu up.
    func testTheFieldsGlassComesBackAfterAHeldPhoto() {
        let app = composer()
        postPhoto()
        holdPhoto(app)
        XCTAssertTrue(wait(10) { self.glass(app) == "still" }, "the field's glass didn't stand down under the menu: \(glass(app))")
        // Dismissed by a tap outside it.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
        XCTAssertTrue(element(app, "native-image-preview").waitForNonExistence(timeout: 10), "the menu stayed up")
        XCTAssertTrue(wait(10) { self.glass(app) == "live" }, "dismissed, the field's glass stayed still: \(glass(app))")

        // Interrupted: the app goes to the background with the menu up.
        holdPhoto(app)
        XCTAssertTrue(wait(10) { self.glass(app) == "still" }, glass(app))
        XCUIDevice.shared.press(.home)
        Thread.sleep(forTimeInterval: 2)
        app.activate()
        XCTAssertTrue(element(app, "native-composer-probe").waitForExistence(timeout: 30))
        XCTAssertTrue(wait(10) { self.glass(app) == "live" }, "back from the background, the field's glass stayed still: \(glass(app))")
    }

    // MARK: The cards

    private static var planFixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    private func cards() -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", Self.planFixture, "-native-agents"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30), "no Main row")
        main.tap()
        XCTAssertTrue(app.buttons["plan-strip"].waitForExistence(timeout: 30), "no Now/Next card")
        XCTAssertTrue(app.buttons["agents-card"].waitForExistence(timeout: 30), "no Agents card")
        return app
    }

    /// **Now/Next, held, previews the plan with its quick actions** (ov-794),
    /// and Open Plan opens it.
    func testHoldingNowNextPreviewsThePlan() {
        let app = cards()
        app.buttons["plan-strip"].press(forDuration: 1.2)
        let preview = element(app, "plan-strip-preview")
        XCTAssertTrue(preview.waitForExistence(timeout: 10), "holding Now/Next showed no preview: \(app.debugDescription)")
        XCTAssertTrue(preview.label.contains("Orchestrator"), "the preview doesn't say the orchestrator: \(preview.label)")
        XCTAssertTrue(app.buttons["Show Needs You"].exists, "no Needs You with two waiting")
        let open = app.buttons["Open Plan"].firstMatch
        XCTAssertTrue(open.exists, "no Open Plan")
        open.tap()
        XCTAssertTrue(element(app, "plan-sheet").waitForExistence(timeout: 15), "Open Plan didn't open it")
    }

    /// **Agents, held, previews who's at work with each as a quick action**,
    /// and Show All Agents opens the view.
    func testHoldingAgentsPreviewsThem() {
        let app = cards()
        app.buttons["agents-card"].press(forDuration: 1.2)
        let preview = element(app, "agents-card-preview")
        XCTAssertTrue(preview.waitForExistence(timeout: 10), "holding Agents showed no preview: \(app.debugDescription)")
        XCTAssertTrue(preview.label.contains("4 agents"), preview.label)
        XCTAssertTrue(preview.label.contains("ov-452 conversation view hierarchy"), "the preview doesn't say what they're doing: \(preview.label)")
        let all = app.buttons["Show All Agents"].firstMatch
        XCTAssertTrue(all.exists, "no Show All Agents")
        all.tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForExistence(timeout: 15), "Show All Agents didn't open the view")
    }

    /// **A tap still opens each card's sheet**, now grown out of the card,
    /// and Done puts it back.
    func testATapOpensEachCardsSheet() {
        let app = cards()
        app.buttons["plan-strip"].tap()
        XCTAssertTrue(element(app, "plan-sheet").waitForExistence(timeout: 15), "the Plan never opened")
        app.buttons["plan-sheet-done"].tap()
        XCTAssertTrue(element(app, "plan-sheet").waitForNonExistence(timeout: 15), "the Plan never closed")
        app.buttons["agents-card"].tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForExistence(timeout: 15), "the agents view never opened")
        app.buttons["agents-done"].tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForNonExistence(timeout: 15), "the agents view never closed")
    }
}
