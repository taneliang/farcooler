import XCTest

/// The conversation view of a terminal-mode claude pane (ov-373), on
/// `-native-agent-harness`: the real shell, `TerminalView`, `NativeSwitch`
/// and `NativeAgentView` over a canned runner reached through the client
/// core's own call path. No runner.
final class NativeAgentViewTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ flags: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"] + flags
        // A capture run's terminal theme, which the pane's scheme follows
        // (`TEST_RUNNER_FARCOOLER_CAPTURE_THEME`, as `-app.theme`). Unset,
        // the app's default.
        if let theme = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_THEME"] {
            app.launchArguments += ["-app.theme", theme]
        }
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The screen, to `FARCOOLER_CAPTURE_OUT/native-<name>.png` when that's
    /// set (as `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`): captures for a review,
    /// off in CI.
    private func capture(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("native-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// What the canned runner has been asked: `follows=N sent=a|b`.
    private func harness(_ app: XCUIApplication) -> String {
        element(app, "native-harness").value as? String ?? ""
    }

    private func follows(_ app: XCUIApplication) -> Int {
        let said = harness(app)
        guard let range = said.range(of: #"follows=(\d+)"#, options: .regularExpression) else { return -1 }
        return Int(said[range].dropFirst("follows=".count)) ?? -1
    }

    /// Which of the pane's two views shows: `conversation`, `terminal`, or
    /// `terminal-only` where the conversation isn't offered.
    private func showing(_ app: XCUIApplication) -> String {
        element(app, "native-showing").value as? String ?? ""
    }

    private func conversation(_ app: XCUIApplication) -> XCUIElement {
        let transcript = element(app, "native-transcript")
        XCTAssertTrue(transcript.waitForExistence(timeout: 60), "the conversation never showed")
        XCTAssertTrue(wait(10) { showing(app) == "conversation" }, "the pane shows \(showing(app))")
        return transcript
    }

    private func send(_ app: XCUIApplication, _ words: String) {
        let field = element(app, "native-composer")
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.tap()
        app.typeText(words)
        element(app, "native-send").tap()
    }

    /// R-27: a phone opens a claude pane on its conversation, and switching
    /// to the terminal and back keeps the draft and never builds the
    /// terminal again.
    func testTheSwitchKeepsTheDraftAndTheTerminal() {
        let app = launch()
        _ = conversation(app)
        // The terminal's key sink is the one text view on the pane.
        Thread.sleep(forTimeInterval: 2)
        XCTAssertEqual(
            app.textViews.matching(NSPredicate(format: "hasKeyboardFocus == true")).count, 0,
            "the hidden terminal took the keyboard on arrival")
        let mount = element(app, "native-terminal-mount")
        let mounted = mount.value as? String
        XCTAssertNotNil(mounted)

        let field = element(app, "native-composer")
        field.tap()
        app.typeText("Half a thought")
        capture("draft")

        element(app, "native-switch").tap()
        XCTAssertTrue(element(app, "terminal-surface").waitForExistence(timeout: 5), "Show Terminal didn't show it")
        XCTAssertTrue(wait(5) { showing(app) == "terminal" }, "the conversation still shows")
        capture("terminal")

        element(app, "native-switch").tap()
        XCTAssertTrue(wait(5) { showing(app) == "conversation" })
        XCTAssertEqual(element(app, "native-composer").value as? String, "Half a thought", "the draft was lost")
        XCTAssertEqual(mount.value as? String, mounted, "switching built the terminal again")
    }

    /// The runner changes a row: the same row, drawn again with its new
    /// words, and no second one.
    func testAFollowUpdatesARowInPlace() {
        let app = launch()
        _ = conversation(app)
        XCTAssertTrue(app.staticTexts["Reading the parser now."].waitForExistence(timeout: 10))
        XCTAssertTrue(wait(20) { harness(app).contains("changed=true") }, "the runner never changed the row: \(harness(app))")
        let updated = app.staticTexts["Read the parser. It’s tidy now: three functions, no globals."]
        XCTAssertTrue(updated.waitForExistence(timeout: 10), "the follow's change never reached the row")
        XCTAssertFalse(app.staticTexts["Reading the parser now."].exists, "the old words stayed")
        XCTAssertEqual(
            app.descendants(matching: .any).matching(identifier: "native-row-prose:a1:0").count, 1,
            "the change arrived as a second row")
        capture("conversation")
    }

    /// A message goes through `terminal.compose`, the box empties, and the
    /// message comes into view above the composer when the runner shows it.
    func testComposeSends() {
        let app = launch()
        _ = conversation(app)
        let words = "Now the lexer, please"
        send(app, words)
        XCTAssertTrue(wait(5) { harness(app).contains("sent=\(words)") }, "compose never reached the runner: \(harness(app))")
        XCTAssertTrue(wait(5) { (element(app, "native-composer").value as? String ?? "").isEmpty || element(app, "native-composer").value as? String == "Message Claude" })
        let shown = app.staticTexts[words]
        XCTAssertTrue(shown.waitForExistence(timeout: 10), "the sent turn never showed")
        let composerTop = element(app, "native-composer-stack").frame.minY
        XCTAssertTrue(
            wait(5) { shown.isHittable && shown.frame.maxY <= element(app, "native-composer-stack").frame.minY },
            "the sent message ends at \(shown.frame.maxY), under the composer at \(composerTop)")
        capture("sent")
    }

    /// R-29: claude is working, its queue takes the message, and the
    /// conversation says Queued.
    func testBusyShowsQueued() {
        let app = launch(["-native-busy"])
        _ = conversation(app)
        send(app, "After this, the tests")
        let queued = element(app, "native-queued")
        XCTAssertTrue(queued.waitForExistence(timeout: 10), "no Queued row")
        XCTAssertTrue(queued.label.contains("Queued"), queued.label)
        XCTAssertTrue(queued.label.contains("After this, the tests"), queued.label)
        capture("queued")
    }

    /// A dialog in the terminal refuses the send and hands off, and Show
    /// Terminal shows it.
    func testADialogShowsHandoff() {
        let app = launch(["-native-dialog"])
        _ = conversation(app)
        send(app, "Anything")
        XCTAssertTrue(element(app, "native-handoff").waitForExistence(timeout: 10), "no Handoff row")
        capture("handoff")
        element(app, "native-handoff-show-terminal").tap()
        XCTAssertTrue(wait(5) { showing(app) == "terminal" }, "Show Terminal didn't show it")
    }

    /// R-28: a draft in the terminal's box refuses, with Show Terminal.
    func testADraftInTheTerminalRefusesWithShowTerminal() {
        let app = launch(["-native-draft"])
        _ = conversation(app)
        send(app, "Anything")
        let issue = element(app, "native-send-issue")
        XCTAssertTrue(issue.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["The terminal’s box already holds a draft. Send or clear it there first."].exists)
        issue.buttons["Show Terminal"].tap()
        XCTAssertTrue(wait(5) { showing(app) == "terminal" }, "Show Terminal didn't show it")
    }

    /// A runner whose projector is off: the terminal, and no switch.
    func testFlagOffShowsTheTerminal() {
        let app = launch(["-native-flag-off"])
        XCTAssertTrue(element(app, "terminal-surface").waitForExistence(timeout: 60), "no terminal")
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(element(app, "native-switch").exists, "a switch to a view the runner doesn't serve")
        XCTAssertEqual(showing(app), "terminal-only")
        XCTAssertFalse(element(app, "native-transcript").exists)
        XCTAssertEqual(follows(app), 0, "rows read from a runner that doesn't serve them")
    }

    /// A turn nobody typed, a background task finishing, is a notice line,
    /// never the person's message.
    func testNoticeTurnsRenderAsNotices() {
        let app = launch()
        _ = conversation(app)
        let notice = element(app, "native-notice-turn")
        XCTAssertTrue(notice.waitForExistence(timeout: 10), "no notice turn")
        XCTAssertEqual(notice.label, "Agent Count the lines finished")
        let prompts = app.descendants(matching: .any).matching(identifier: "native-prompt")
        XCTAssertFalse(
            prompts.allElementsBoundByIndex.contains { $0.label.contains("Count the lines") },
            "a notification drawn as a message the person typed")
        XCTAssertTrue(prompts.allElementsBoundByIndex.contains { $0.label == "Tidy the parser, please." })
    }

    /// The follow runs only while the conversation is on screen: not behind
    /// the terminal, and not with the app in the background.
    func testTheFollowStopsOffScreen() {
        let app = launch()
        _ = conversation(app)
        XCTAssertTrue(wait(10) { follows(app) >= 2 }, harness(app))

        element(app, "native-switch").tap()
        XCTAssertTrue(wait(5) { showing(app) == "terminal" })
        Thread.sleep(forTimeInterval: 1)
        let behind = follows(app)
        Thread.sleep(forTimeInterval: 3)
        XCTAssertLessThanOrEqual(follows(app), behind, "still following behind the terminal")

        element(app, "native-switch").tap()
        XCTAssertTrue(wait(5) { follows(app) > behind }, "didn't follow again on return")

        XCUIDevice.shared.press(.home)
        Thread.sleep(forTimeInterval: 5)
        app.activate()
        XCTAssertTrue(conversation(app).waitForExistence(timeout: 10))
        XCTAssertTrue(harness(app).contains("background=0"), "followed in the background: \(harness(app))")
        XCTAssertTrue(wait(5) { follows(app) > behind + 1 }, "didn't follow again in front")
    }
}
