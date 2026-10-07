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

    /// The inset reads `keyboard=N;bar=M;` off the transcript's probe.
    private func insets(_ transcript: XCUIElement) -> (keyboard: Int, bar: Int)? {
        let parts = (transcript.value as? String ?? "").split(separator: ";")
        func number(_ key: String) -> Int? {
            parts.first { $0.hasPrefix(key + "=") }.flatMap { Int($0.dropFirst(key.count + 1)) }
        }
        guard let keyboard = number("keyboard"), let bar = number("bar") else { return nil }
        return (keyboard, bar)
    }

    /// Whether `keyboard` is the docked bar and nothing else: the bar's
    /// height, less at most the home indicator's strip, which the bar
    /// counts and the keyboard's frame doesn't (measured 184 against 218).
    private func isTheBar(_ inset: (keyboard: Int, bar: Int)) -> Bool {
        inset.keyboard > 0 && inset.keyboard <= inset.bar + 2 && inset.keyboard >= inset.bar - 40
    }

    /// The keyboard going away leaves the docked composer, and the cover
    /// settles at its height (ov-386): not zero, and not whatever the
    /// composer reported while it slid. Sampled from the tap on, so a zero
    /// held until a timeout, or a mid-slide number, is seen if it's ever
    /// read, rather than missed by a single look at the end.
    /// Best effort: on the simulator the hide's frame follows `willHide` too
    /// closely for this to see a zero that `KeyboardCover.willHide` used to
    /// leave (checked: it stays green with that mutation), so the rule itself
    /// is pinned by `KeyboardCoverTests.aHideAfterItsFrameKeepsTheBarsCover`.
    func testTheCoverSettlesAtTheBarWhenTheKeyboardHides() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        app.textViews.firstMatch.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(
            wait(30) { (insets(transcript).map { $0.keyboard > $0.bar + 100 }) == true },
            "the keyboard didn't raise the cover: \(transcript.value ?? "")")
        waitForHideKeyboardKey(app, Self.composerHideKeyboard).tap()
        var read: [String] = []
        var below = 0
        let settled = wait(30) {
            let value = transcript.value as? String ?? ""
            read.append(value)
            if let inset = insets(transcript), inset.keyboard < inset.bar - 40 { below += 1 }
            return !app.keyboards.firstMatch.exists && insets(transcript).map(isTheBar) == true
        }
        XCTAssertTrue(settled, "the cover didn't settle at the bar: \(read.suffix(3))")
        XCTAssertEqual(below, 0, "the cover read under the bar while the keyboard hid: \(read)")
        // And it stays: nothing late moves it.
        Thread.sleep(forTimeInterval: 2)
        let after = try XCTUnwrap(insets(transcript))
        XCTAssertTrue(isTheBar(after), "a late report moved the cover: \(transcript.value ?? "")")
    }

    /// With the keys up the composer growing sends no keyboard frame, so the
    /// cover follows what the composer itself reports: through the scope
    /// `AgentView` gives `DockedBar` and the report's `reportCover` (ov-386).
    /// The cover must rise by within 40 pt of how far the composer's top did.
    /// Not exactly: measured 32 of 66 pt on iPhone 17 (the report is read at
    /// layout, as the accessory settles), against none at all when the report
    /// doesn't reach the inset.
    func testTheCoverFollowsTheComposerGrowingWithTheKeysUp() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        app.textViews.firstMatch.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(wait(30) { (insets(transcript).map { $0.keyboard > $0.bar + 100 }) == true }, "\(transcript.value ?? "")")
        Thread.sleep(forTimeInterval: 1)
        let before = composerTop(app)
        let coverBefore = try XCTUnwrap(insets(transcript)).keyboard
        app.typeText(
            "A draft long enough to wrap onto five or six separate lines inside the composer "
                + "card, so that the field grows well past its resting height and covers more")
        XCTAssertTrue(wait(30) { composerTop(app) < before - 30 }, "the composer didn't grow")
        Thread.sleep(forTimeInterval: 1)
        let grew = Int(before - composerTop(app))
        XCTAssertTrue(
            wait(30) { insets(transcript).map { $0.keyboard - coverBefore >= min(grew - 40, 20) } == true },
            "the composer's top rose \(grew) pt but the cover went from \(coverBefore) to \(transcript.value ?? "")")
    }
}
