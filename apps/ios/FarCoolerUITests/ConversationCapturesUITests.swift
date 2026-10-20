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

    private func shoot(_ name: String) {
        guard let out else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("\(tag)-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    func testCaptures() throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT to take them")
        defer { XCUIDevice.shared.appearance = .light }
        for (scheme, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            for size in Self.sizes {
                // The conversation's ground is the terminal theme's (ov-342), so a
                // light capture takes a light theme too.
                let type = ["-UIPreferredContentSizeCategoryName", size.category]
                    + ["-app.theme", scheme == "light" ? "Solarized Light" : "Nord"]
                workspace(type, name: "workspace-\(scheme)-\(size.name)")
                transcript(type, name: "transcript-\(scheme)-\(size.name)")
                bubble(type, name: "bubble-\(scheme)-\(size.name)")
            }
        }
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
        shoot(name)
        // Scrolled back a little over a line, so a row stands part way under
        // the strip's edge (ov-611).
        let list = app.descendants(matching: .any)["native-transcript"].firstMatch
        if list.exists {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 37)), withVelocity: .slow, thenHoldForDuration: 0.3)
            Thread.sleep(forTimeInterval: 1.5)
            shoot("\(name)-scrolled")
        }
        // The composer (ov-614): many lines, then a photo with them.
        if field.exists {
            field.tap()
            Thread.sleep(forTimeInterval: 1)
            app.typeText((1...9).map { "Line \($0) of a long message" }.joined(separator: "\n"))
            Thread.sleep(forTimeInterval: 1.5)
            shoot("\(name)-composer-lines")
            // Scrolled back half a line, so the top line stands part way past
            // the box's edge, where it fades (ov-614).
            let middle = field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            middle.press(forDuration: 0.05, thenDragTo: middle.withOffset(CGVector(dx: 0, dy: 11)), withVelocity: .slow, thenHoldForDuration: 0.5)
            Thread.sleep(forTimeInterval: 1)
            shoot("\(name)-composer-midline")
            CFNotificationCenterPostNotification(
                CFNotificationCenterGetDarwinNotifyCenter(),
                CFNotificationName("com.farcooler.harness.native-photo" as CFString), nil, nil, true)
            Thread.sleep(forTimeInterval: 3)
            shoot("\(name)-composer-photo")
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
        shoot(name)
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
        shoot(name)
        app.terminate()
    }
}
