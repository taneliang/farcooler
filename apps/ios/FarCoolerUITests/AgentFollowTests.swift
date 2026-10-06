import XCTest

/// The chat follows like Messages (ov-383), on `-agent-layout-harness`: the
/// real `AgentView` over a canned transcript, with no runner behind it, so
/// a send draws its message and then fails, which is fine here: what's
/// measured is where the message is drawn.
///
/// `AgentTranscriptScrollTests` holds the rest of the rules: it opens at the
/// tail, the keyboard doesn't drag a reader who scrolled away, Jump to
/// Latest returns, and the inset matches the composer.
final class AgentFollowTests: XCTestCase {
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launchDrawn()
        return app
    }

    /// The screen, to `FARCOOLER_CAPTURE_OUT/agent-follow-<name>.png` when
    /// that's set (as `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`): captures for a
    /// review, off in CI.
    private func capture(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("agent-follow-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    /// Whether the probe says the transcript is following its tail.
    private func following(_ transcript: XCUIElement) -> Bool {
        (transcript.value as? String)?.hasSuffix("tail=true") == true
    }

    /// Polls `done` until it holds or `timeout` passes; whether it held.
    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// The top of what rests on the transcript: the composer stack, the card
    /// and anything over it, such as the harness's failed-send banner.
    private func composerTop(_ app: XCUIApplication) -> CGFloat {
        app.descendants(matching: .any).matching(identifier: "agent-composer-stack").firstMatch.frame.minY
    }

    /// The room the last row rests with above the composer: the
    /// transcript's padding (`PaneMetrics.card`), not a row against the
    /// glass. Measured 13 pt (the padding and the 1 pt anchor).
    private static let clearance: CGFloat = 12

    /// The bottom of the transcript's last row: the harness's pane is
    /// working, so Working… under `message` when it's drawn.
    private func lastRowBottom(_ app: XCUIApplication, after message: XCUIElement) -> CGFloat {
        let working = app.descendants(matching: .any).matching(identifier: "agent-working").firstMatch
        return working.exists ? max(message.frame.maxY, working.frame.maxY) : message.frame.maxY
    }

    /// Scrolled up, then a message sent: it comes into view above the
    /// composer, and following resumes.
    func testASentMessageComesIntoViewFromWhereverTheReaderWas() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        transcript.swipeDown(velocity: .fast)
        transcript.swipeDown(velocity: .fast)
        XCTAssertTrue(wait(3) { !following(transcript) }, "scrolling away didn't unpin")
        XCTAssertTrue(app.buttons["jump-to-latest"].waitForExistence(timeout: 3))
        capture("1-scrolled-up")

        let field = app.textViews.firstMatch
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        let words = "Bring this one into view"
        app.typeText(words)
        app.buttons["agent-send"].tap()

        let sent = app.staticTexts[words]
        XCTAssertTrue(sent.waitForExistence(timeout: 10), "the sent message was never drawn")

        XCTAssertTrue(
            wait(5) { following(transcript) && sent.isHittable && composerTop(app) - lastRowBottom(app, after: sent) >= Self.clearance },
            "the sent message is at \(sent.frame), the composer starts at \(composerTop(app)), "
                + "following: \(following(transcript)); \(transcript.value ?? "")")
        XCTAssertFalse(app.buttons["jump-to-latest"].exists, "Jump to Latest outlived the send")
        capture("2-sent")
    }

    /// Pinned, the follow survives the composer growing: the last message
    /// stays above it as a long draft takes more lines.
    func testFollowingSurvivesTheComposerGrowing() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        let field = app.textViews.firstMatch
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        let words = "The last thing said"
        app.typeText(words)
        app.buttons["agent-send"].tap()
        let sent = app.staticTexts[words]
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertTrue(
            wait(5) { composerTop(app) - lastRowBottom(app, after: sent) >= Self.clearance },
            "the failed send's banner covers the message: it ends at \(sent.frame.maxY), the "
                + "composer starts at \(composerTop(app)); \(transcript.value ?? "")")
        // Past the send's own settling re-anchors (`settleLadder`, 1 s in
        // all), so what keeps the message in view below is the growth.
        Thread.sleep(forTimeInterval: 2)

        let before = composerTop(app)
        let ended = sent.frame.maxY
        if !app.keyboards.firstMatch.exists { field.tap() }
        app.typeText(
            "A draft long enough to wrap onto five or six separate lines inside the composer "
                + "card, so that the field grows well past its resting height and covers more")
        XCTAssertTrue(wait(5) { composerTop(app) < before - 30 }, "the composer didn't grow")
        // The message rises at least as far as the composer's field did:
        // measured, it rose 108 pt for the field's 75, and with the
        // `obstruction` re-anchor removed, 47, so the composer crept 28 pt
        // closer over it.
        XCTAssertTrue(
            wait(5) { following(transcript) && ended - sent.frame.maxY >= before - composerTop(app) - 5 },
            "the composer grew \(before - composerTop(app)) pt and the last message rose "
                + "\(ended - sent.frame.maxY) pt")
        XCTAssertGreaterThanOrEqual(composerTop(app) - lastRowBottom(app, after: sent), Self.clearance)
        capture("3-composer-grown")
    }
}
