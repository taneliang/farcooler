import XCTest

/// Captures of the iPhone conversation screen for a review (ov-609, ov-610,
/// ov-611), light and dark, at the reader's default type size and a large
/// one. Opt-in, as `PlanUITests.testCaptures` is: it flips the device's
/// appearance. `TEST_RUNNER_FARCOOLER_CAPTURE_OUT=<dir>` turns it on, and
/// `TEST_RUNNER_FARCOOLER_CAPTURE_TAG=<phase>-<device>` names the files.
///
/// Two screens: the owner's (Main's conversation, with the Now/Next and
/// Agents cards, the conversation and the composer;
/// `-phone-native`), and a pane's conversation over a redacted real
/// transcript (`-native-harness-file`).
final class ConversationCapturesUITests: XCTestCase {
    private var out: String? { ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] }
    private var tag: String { ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_TAG"] ?? "capture" }

    /// `test/fixtures/<name>`, found from this file.
    private func fixture(_ name: String) -> String {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("test/fixtures/\(name)").path
    }

    private static let sizes: [(name: String, category: String)] = [
        ("default", "UICTContentSizeCategoryL"), ("axl", "UICTContentSizeCategoryAccessibilityL"),
    ]

    /// `state` is what the screen must show before the shot: it fails the test, and writes nothing, when it never does (ov-642).
    private func shoot(_ name: String, expecting state: CaptureState) {
        guard let out else { return }
        guard requireState(state, for: "\(tag)-\(name)") else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("\(tag)-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    // One method per appearance and size: the four in one method launched the
    // app twelve times, about 100 s a round on the nightly's runner, and ran
    // into the six-minute allowance a test gets (TEST_TIME_LIMIT in
    // scripts/ios-ui-tests.sh) in its fourth round on every night from Oct 18
    // to 22. The runner's kill then left the simulator refusing to terminate
    // the app, which failed the classes after it.
    func testCapturesLightDefault() throws { try captures("light", .light, Self.sizes[0]) }
    func testCapturesLightLarge() throws { try captures("light", .light, Self.sizes[1]) }
    func testCapturesDarkDefault() throws { try captures("dark", .dark, Self.sizes[0]) }
    func testCapturesDarkLarge() throws { try captures("dark", .dark, Self.sizes[1]) }

    private func captures(
        _ scheme: String, _ appearance: XCUIDevice.Appearance, _ size: (name: String, category: String)
    ) throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT to take them")
        defer { XCUIDevice.shared.appearance = .light }
        XCUIDevice.shared.appearance = appearance
        // The conversation's ground is the terminal theme's (ov-342), so a
        // light capture takes a light theme too.
        let type = ["-UIPreferredContentSizeCategoryName", size.category]
            + ["-app.theme", scheme == "light" ? "Solarized Light" : "Nord"]
        workspace(type, name: "workspace-\(scheme)-\(size.name)")
        transcript(type, name: "transcript-\(scheme)-\(size.name)")
        bubble(type, name: "bubble-\(scheme)-\(size.name)")
    }

    /// Main's Orchestrator tab, as the owner's screenshot shows it.
    private func workspace(_ type: [String], name: String) {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", fixture("plan-seeded.json"),
             "-native-agents"] + type)
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30), "no Main row")
        main.tap()
        // The composer, which only the conversation has.
        let field = app.descendants(matching: .any)["native-composer"]
        XCTAssertTrue(field.waitForExistence(timeout: 30), "the orchestrator showed no conversation")
        Thread.sleep(forTimeInterval: 3)
        shoot(name, expecting: .shows(field, "the orchestrator's composer"))
        // Scrolled back a little over a line, so a row stands part way under
        // the strip's edge (ov-611).
        let list = app.descendants(matching: .any)["native-transcript"].firstMatch
        if list.exists {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 37)), withVelocity: .slow, thenHoldForDuration: 0.3)
            Thread.sleep(forTimeInterval: 1.5)
            shoot("\(name)-scrolled", expecting: .shows(list, "the transcript, scrolled"))
        }
        // The composer (ov-614): many lines, then a photo with them.
        if field.exists {
            field.tap()
            Thread.sleep(forTimeInterval: 1)
            app.typeText((1...9).map { "Line \($0) of a long message" }.joined(separator: "\n"))
            Thread.sleep(forTimeInterval: 1.5)
            shoot("\(name)-composer-lines", expecting: .when("the box holds nine lines") { "\(field.value ?? "")".contains("Line 9") })
            // Scrolled back half a line, so the top line stands part way past
            // the box's edge, where it fades (ov-614).
            let middle = field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            middle.press(forDuration: 0.05, thenDragTo: middle.withOffset(CGVector(dx: 0, dy: 11)), withVelocity: .slow, thenHoldForDuration: 0.5)
            Thread.sleep(forTimeInterval: 1)
            shoot("\(name)-composer-midline", expecting: .when("the box holds nine lines") { "\(field.value ?? "")".contains("Line 9") })
            CFNotificationCenterPostNotification(
                CFNotificationCenterGetDarwinNotifyCenter(),
                CFNotificationName("com.farcooler.harness.native-photo" as CFString), nil, nil, true)
            Thread.sleep(forTimeInterval: 3)
            shoot("\(name)-composer-photo", expecting: .shows(app.descendants(matching: .any)["native-image-chip"].firstMatch, "the photo chip in the composer"))
        }
        app.terminate()
    }

    /// The person's long message, folded with Show More, as the site's blue
    /// bubble (ov-610).
    private func bubble(_ type: [String], name: String) {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-polish"] + type
        app.launchDrawn()
        // The queued row: by its own name, or its row's where an OS folds
        // the two containers into one.
        let queued = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier IN %@", ["native-queued", "native-row-queued:s0"])).firstMatch
        let list = app.descendants(matching: .any)["native-transcript"].firstMatch
        XCTAssertTrue(list.waitForExistence(timeout: 30), "no conversation")
        // Up the list until the message is built: the list is lazy.
        for _ in 0..<12 where !queued.exists {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 200)), withVelocity: .slow, thenHoldForDuration: 0.3)
        }
        guard queued.exists else { return XCTFail("no long message") }
        let composer = app.descendants(matching: .any)["native-composer-surface"].firstMatch
        // Down a little at a time until the whole bubble, Show More and all,
        // stands clear of the composer.
        for _ in 0..<12 where queued.frame.maxY > composer.frame.minY - 60 {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 80)), withVelocity: .slow, thenHoldForDuration: 0.3)
        }
        Thread.sleep(forTimeInterval: 1.5)
        shoot(name, expecting: .shows(queued, "the long message, clear of the composer"))
        app.terminate()
    }

    /// A pane's conversation over a real transcript's rows.
    private func transcript(_ type: [String], name: String) {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-harness-file", fixture("agent-rows-harness.json")] + type
        app.launchDrawn()
        let transcript = app.descendants(matching: .any)["native-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30), "no conversation")
        Thread.sleep(forTimeInterval: 3)
        shoot(name, expecting: .shows(transcript, "the pane's transcript"))
        app.terminate()
    }

    /// The composer laid out like Messages (ov-677), light and dark: at rest,
    /// claude working (Stop), typing (Send), grown to its cap, and a photo
    /// in it. `-native-suggestion` is at rest; the plain harness is working.
    ///
    /// One method per appearance: together they took 289 s of the six
    /// minutes on the Oct 22 nightly.
    func testComposerCapturesLight() throws { try composerCaptures("light", .light) }
    func testComposerCapturesDark() throws { try composerCaptures("dark", .dark) }

    private func composerCaptures(_ scheme: String, _ appearance: XCUIDevice.Appearance) throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT to take them")
        defer { XCUIDevice.shared.appearance = .light }
        XCUIDevice.shared.appearance = appearance
        let theme = ["-app.theme", scheme == "light" ? "Solarized Light" : "Nord"]
        for size in Self.sizes {
            let type = ["-UIPreferredContentSizeCategoryName", size.category] + theme
            composer(type, name: "composer-\(scheme)-\(size.name)")
        }
    }

    private func composer(_ type: [String], name: String) {
        let rest = XCUIApplication()
        rest.launchArguments = ["-native-agent-harness", "-native-suggestion"] + type
        rest.launchDrawn()
        let suggestion = rest.descendants(matching: .any)["native-suggestion"].firstMatch
        XCTAssertTrue(suggestion.waitForExistence(timeout: 60), "claude never came to rest")
        Thread.sleep(forTimeInterval: 1)
        shoot("\(name)-rest", expecting: .shows(suggestion, "the box at rest"))
        rest.terminate()

        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"] + type
        app.launchDrawn()
        let stop = app.descendants(matching: .any)["native-stop"].firstMatch
        XCTAssertTrue(stop.waitForExistence(timeout: 60), "no Stop while claude works")
        Thread.sleep(forTimeInterval: 1)
        shoot("\(name)-working", expecting: .shows(stop, "Stop in the box"))
        let field = app.descendants(matching: .any)["native-composer"].firstMatch
        field.tap()
        Thread.sleep(forTimeInterval: 1)
        app.typeText("Can you check the build on the 17e?")
        Thread.sleep(forTimeInterval: 1)
        let send = app.descendants(matching: .any)["native-send"].firstMatch
        shoot("\(name)-typing", expecting: .shows(send, "Send in the box"))
        app.typeText((2...14).map { "\nLine \($0) of a long message" }.joined())
        Thread.sleep(forTimeInterval: 1.5)
        shoot("\(name)-cap", expecting: .when("the box holds fourteen lines") { "\(field.value ?? "")".contains("Line 14") })
        app.terminate()

        let photo = XCUIApplication()
        photo.launchArguments = ["-native-agent-harness"] + type
        photo.launchDrawn()
        let box = photo.descendants(matching: .any)["native-composer"].firstMatch
        XCTAssertTrue(box.waitForExistence(timeout: 60), "no composer")
        box.tap()
        Thread.sleep(forTimeInterval: 1)
        photo.typeText("What is this")
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.native-photo" as CFString), nil, nil, true)
        let chip = photo.descendants(matching: .any)["native-image-chip"].firstMatch
        XCTAssertTrue(chip.waitForExistence(timeout: 30), "no chip for the photo")
        Thread.sleep(forTimeInterval: 1.5)
        shoot("\(name)-photo", expecting: .shows(chip, "the photo chip in the box"))
        photo.terminate()
    }

    /// Rows under the bars, Liquid Glass's way (ov-678): the orchestrator's
    /// navigation bar and cards, and a pane's bar, over busy rows (tool
    /// calls, a prompt's images, the blue bubble), light and dark, at the
    /// tail and scrolled back by steps so each kind passes under.
    func testBarsCaptures() throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT to take them")
        defer { XCUIDevice.shared.appearance = .light }
        for (scheme, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let theme = ["-app.theme", scheme == "light" ? "Solarized Light" : "Nord"]
            let orchestrator = XCUIApplication.phoneHarness(
                ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", fixture("plan-seeded.json"),
                 "-native-agents", "-native-polish", "-native-images"] + theme)
            let main = orchestrator.buttons["workspace-row-Main"]
            XCTAssertTrue(main.waitForExistence(timeout: 30), "no Main row")
            main.tap()
            underBars(orchestrator, name: "bars-orchestrator-\(scheme)")
            let pane = XCUIApplication()
            pane.launchArguments = ["-native-agent-harness", "-native-polish", "-native-images"] + theme
            pane.launchDrawn()
            underBars(pane, name: "bars-pane-\(scheme)")
        }
    }

    private func underBars(_ app: XCUIApplication, name: String) {
        let list = app.descendants(matching: .any)["native-transcript"].firstMatch
        XCTAssertTrue(list.waitForExistence(timeout: 60), "no conversation")
        Thread.sleep(forTimeInterval: 2)
        shoot("\(name)-0", expecting: .shows(list, "the transcript at its tail"))
        for step in 1...3 {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 140)), withVelocity: .slow, thenHoldForDuration: 0.3)
            Thread.sleep(forTimeInterval: 1.5)
            shoot("\(name)-\(step)", expecting: .shows(list, "the transcript, scrolled back"))
        }
        app.terminate()
    }
}
