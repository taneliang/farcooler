import XCTest

/// A claude TUI pane's permission ask, answered from the phone.
///
/// Against the demo runner: `./scripts/demo-host.sh` opens a worktree
/// `asking` whose `claude` pane runs a stand-in that, on each typed line,
/// draws claude's permission dialog and runs the `PermissionRequest` hook Far
/// Cooler wrote for it. The daemon holds that hook and records the ask in the
/// pane's ring, which is what the phone's bar reads.
///
/// The stand-in takes its dialog down only when the hook exits, or when a
/// second line is typed. So a denial that never reached the daemon leaves the
/// pane blocked for the whole 60 s hold, and `blocked=0` soon after a tap is
/// the evidence that the tap was an answer and not just a card going away.
final class TerminalPermissionTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    private func launch() throws -> XCUIApplication {
        let app = XCUIApplication()
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        app.launchArguments += ["-farcoolerDemoHost", "\(user)@\(host)"]
        app.launch()
        let shell = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        // 180 s for `TerminalScrollTests.openATerminalInTheShell`'s measured
        // reason: the first launches of a fresh install are slow.
        guard shell.waitForExistence(timeout: 180) else {
            throw XCTSkip(
                "The shell never rendered against \(user)@\(host); run ./scripts/demo-host.sh.")
        }
        return app
    }

    private func probe(_ app: XCUIApplication, _ identifier: String) -> [String: String] {
        let element = app.descendants(matching: .any).matching(identifier: identifier).firstMatch
        guard element.exists else { return [:] }
        var parsed: [String: String] = [:]
        for field in (element.value as? String ?? "").split(separator: " ") {
            let halves = field.split(separator: "=", maxSplits: 1)
            guard halves.count == 2 else { continue }
            parsed[String(halves[0])] = String(halves[1])
        }
        return parsed
    }

    private func waitFor(
        _ timeout: TimeInterval, _ condition: @escaping () -> Bool
    ) -> Bool {
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in condition() }, object: nil)
        return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
    }

    /// Up into the overview, onto the `asking` card, and along to its claude
    /// tab: Changes, then the shell the worktree was made with, then claude.
    private func openTheAskingClaude(_ app: XCUIApplication) throws {
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        // `ShellBoardTests.testTheRealBoardRowLandsOnTheDispatchedPane`'s way
        // up, for its reasons: the keyboard away first, then a held lift or a
        // fling.
        for attempt in 0..<4 where probe(app, "shell-state")["overview"] != "1" {
            let hide = app.buttons[XCTestCase.terminalHideKeyboard]
            if hide.exists, hide.isHittable { hide.tap() }
            Thread.sleep(forTimeInterval: 0.5)
            let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            if attempt % 2 == 0 {
                from.press(
                    forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: -700)),
                    withVelocity: .slow, thenHoldForDuration: 0.5)
            } else {
                from.press(
                    forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: -400)),
                    withVelocity: XCUIGestureVelocity(rawValue: 12000), thenHoldForDuration: 0)
            }
            Thread.sleep(forTimeInterval: 1)
        }
        XCTAssertEqual(
            probe(app, "shell-state")["overview"], "1",
            "never reached the overview: \(probe(app, "shell-state"))")

        let card = app.buttons.matching(
            NSPredicate(
                format: "identifier BEGINSWITH %@ AND label == %@", "shell-card-", "asking")
        ).firstMatch
        XCTAssertTrue(
            card.waitForExistence(timeout: 30),
            "no `asking` card; did ./scripts/demo-host.sh make its asking claude?")
        card.tap()
        XCTAssertTrue(
            waitFor(15) { self.probe(app, "shell-state")["overview"] == "0" },
            "the overview did not close onto asking")
        XCTAssertTrue(bar.label.contains("asking"), "landed on \(bar.label), not asking")

        for _ in 0..<4 where probe(app, "shell-state")["tab"] != "2" {
            let y = 0.42
            let tab = Int(probe(app, "shell-state")["tab"] ?? "") ?? 0
            let (fromX, toX) = tab < 2 ? (0.78, 0.22) : (0.22, 0.78)
            app.coordinate(withNormalizedOffset: CGVector(dx: fromX, dy: y)).press(
                forDuration: 0.05,
                thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: toX, dy: y)),
                withVelocity: .slow, thenHoldForDuration: 0.4)
            _ = waitFor(3) { self.probe(app, "shell-state")["tab"] == "2" }
        }
        XCTAssertEqual(
            probe(app, "shell-state")["tab"], "2",
            "not on asking's claude tab: \(probe(app, "shell-state"))")
        XCTAssertTrue(bar.label.contains("asking"), "swiped off asking onto \(bar.label)")
        XCTAssertTrue(
            waitFor(10) { !self.probe(app, "terminal-ask").isEmpty },
            "the claude pane publishes no terminal-ask probe")
    }

    /// Type a line into the pane, which is what makes the stand-in ask.
    private func ask(_ app: XCUIApplication) {
        let surface = app.otherElements.matching(
            NSPredicate(format: "identifier == %@ AND value BEGINSWITH %@",
                "terminal-surface", "visible=1")
        ).firstMatch
        if surface.exists { surface.tap() }
        app.typeText("x\n")
    }

    private func card(_ app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: "terminal-permission").firstMatch
    }

    /// **The ask is on the pane, with the agent's own Allow and Deny, and Deny
    /// answers it.** Tapped, the card goes, and the pane stops being blocked
    /// well inside the 60 s hold, which only a delivered answer can do.
    func testATUIAskIsAnsweredFromThePhone() throws {
        let app = try launch()
        try openTheAskingClaude(app)
        XCTAssertEqual(
            probe(app, "terminal-ask")["blocked"], "0",
            "the asking claude is already blocked before anything asked: "
                + "\(probe(app, "terminal-ask"))")

        ask(app)
        let card = card(app)
        XCTAssertTrue(
            card.waitForExistence(timeout: 20),
            "no permission card over the pane: \(probe(app, "terminal-ask"))")
        XCTAssertTrue(
            probe(app, "terminal-ask")["ask"]?.hasPrefix("hook-ask-") == true,
            "the card is not the hook's ask: \(probe(app, "terminal-ask"))")
        XCTAssertTrue(card.staticTexts["Needs your approval"].exists, "the card has no heading")
        XCTAssertTrue(card.buttons["Allow touch x"].exists, "no Allow, in the agent's words")
        let deny = card.buttons["Deny"]
        XCTAssertTrue(deny.exists, "no Deny")

        deny.tap()
        XCTAssertTrue(waitFor(5) { !card.exists }, "the card stayed up after Deny")
        XCTAssertTrue(
            waitFor(20) { self.probe(app, "terminal-ask")["blocked"] == "0" },
            "the pane is still blocked 20 s after Deny, so the hook never got its answer: "
                + "\(probe(app, "terminal-ask"))")
        // And it landed as a deny: the daemon's `Resolved` for this ask names
        // the option it was settled with.
        XCTAssertTrue(
            waitFor(10) { self.probe(app, "terminal-ask")["resolved"] == "deny" },
            "the ask was not resolved as a deny: \(probe(app, "terminal-ask"))")
    }

    /// **Answered somewhere else, the card goes by itself.** A second line is
    /// the keyboard answering claude's own dialog; nothing on the phone is
    /// tapped. The card is not tied to the fleet's `blocked`, so only the
    /// daemon's `Resolved` for the ask can take it down, and the probe says
    /// the ask ended with no option chosen.
    func testATUIAskAnsweredAtTheKeyboardLeavesThePhone() throws {
        let app = try launch()
        try openTheAskingClaude(app)

        ask(app)
        let card = card(app)
        XCTAssertTrue(
            card.waitForExistence(timeout: 20),
            "no permission card over the pane: \(probe(app, "terminal-ask"))")
        // The fleet's blocked means the dialog is on the pane, so the stand-in
        // is holding the hook and the next line is the keyboard's answer.
        XCTAssertTrue(
            waitFor(20) { self.probe(app, "terminal-ask")["blocked"] == "1" },
            "the pane never read as blocked: \(probe(app, "terminal-ask"))")

        app.typeText("y\n")
        XCTAssertTrue(
            waitFor(15) { !card.exists },
            "the card is still up after the keyboard answered: \(probe(app, "terminal-ask"))")
        XCTAssertEqual(probe(app, "terminal-ask")["ask"], "-")
        XCTAssertEqual(
            probe(app, "terminal-ask")["resolved"], "none",
            "the card went, but not on a Resolved with no option chosen")
    }
}
