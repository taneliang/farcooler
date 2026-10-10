import XCTest

/// The agent tray (ov-453) on `-native-agent-harness -native-agents`: four
/// subagents running, pinned above the composer with main, folding to its
/// header, and each opening to its own conversation with a way back. The
/// runner is the harness's, which serves each agent's own rows.
final class NativeAgentTrayUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-agents"]
        // A capture run's terminal theme, which the conversation's scheme
        // follows (`TEST_RUNNER_FARCOOLER_CAPTURE_THEME`).
        if let theme = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_THEME"] {
            app.launchArguments += ["-app.theme", theme]
        }
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func harness(_ app: XCUIApplication) -> String {
        element(app, "native-harness").value as? String ?? ""
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// The screen, to `FARCOOLER_CAPTURE_OUT/ios-<name>.png` when set.
    private func capture(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("ios-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func tray(_ app: XCUIApplication) -> XCUIElement {
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 60), "the conversation never showed")
        let tray = element(app, "native-agent-tray")
        XCTAssertTrue(tray.waitForExistence(timeout: 30), "no tray while agents run")
        return tray
    }

    func testTheTrayListsMainAndEveryRunningAgentAndFolds() {
        let app = launch()
        _ = tray(app)
        for id in ["tray:main", "sub:a1", "sub:a2", "sub:a3", "sub:a4"] {
            XCTAssertTrue(element(app, "native-agent-tray-\(id)").exists, "\(id) is listed")
        }
        XCTAssertFalse(element(app, "native-agent-tray-sub:a5").exists, "one that ended is history, not the tray's")
        let a1 = element(app, "native-agent-tray-sub:a1")
        XCTAssertTrue(a1.label.contains("ov-452 conversation view hierarchy") && a1.label.contains("87.2K tokens"), a1.label)
        let composer = element(app, "native-composer")
        XCTAssertTrue(composer.exists)
        XCTAssertLessThanOrEqual(element(app, "native-agent-tray").frame.maxY, composer.frame.minY + 1, "above the composer")
        capture("tray")

        element(app, "native-agent-tray-header").tap()
        XCTAssertTrue(wait(10) { !element(app, "native-agent-tray-sub:a1").exists }, "folded to its header")
        XCTAssertTrue(element(app, "native-agent-tray-header").exists)
        capture("collapsed")
        element(app, "native-agent-tray-header").tap()
        XCTAssertTrue(element(app, "native-agent-tray-sub:a1").waitForExistence(timeout: 10))
    }

    func testAnAgentOpensToItsOwnConversationAndBackReturns() {
        let app = launch()
        _ = tray(app)
        element(app, "native-agent-tray-sub:a1").tap()
        XCTAssertTrue(element(app, "native-agent-back").waitForExistence(timeout: 20), "a clear way back")
        XCTAssertTrue(wait(20) { harness(app).contains("opened=agent-a1") }, harness(app))
        XCTAssertTrue(element(app, "native-row-prose:agent-a1-1").waitForExistence(timeout: 20), "the agent's own words")
        XCTAssertFalse(element(app, "native-composer").exists, "nothing to send to an agent")
        XCTAssertTrue(element(app, "native-agent-tray").exists, "the tray stays, to go to another")
        capture("opened")

        element(app, "native-agent-back").tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 20))
        XCTAssertFalse(element(app, "native-agent-back").exists)

        // Main, in the tray, goes back too.
        element(app, "native-agent-tray-sub:a3").tap()
        XCTAssertTrue(element(app, "native-agent-back").waitForExistence(timeout: 20))
        element(app, "native-agent-tray-tray:main").tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 20))
    }

    /// Opt-in captures in landscape too, the phone's wide width.
    func testCaptureWide() throws {
        guard ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil else { throw XCTSkip("captures only") }
        let app = launch()
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        _ = tray(app)
        Thread.sleep(forTimeInterval: 1.5)
        capture("wide-tray")
        element(app, "native-agent-tray-sub:a1").tap()
        XCTAssertTrue(element(app, "native-agent-back").waitForExistence(timeout: 20))
        Thread.sleep(forTimeInterval: 1.5)
        capture("wide-opened")
    }
}
