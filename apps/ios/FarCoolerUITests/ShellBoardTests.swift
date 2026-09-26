import XCTest

/// A repository's board, opened from the overview, and the way from a card to
/// the agent working it.
///
/// **Why a UI test and not arithmetic.** Which repositories get a Board row,
/// what its counts are, which panes work which card and what a card says are
/// rules, and they are in `RunnerBoardsTests` and `TaskBoardAgentsTests` where
/// `swift test` runs them. What is left is what no rule can see: that the row
/// is actually drawn over the runner's cards, that tapping it opens the board,
/// that the board lists Needs Decision first, and that an Agent button closes
/// the board and lands the shell on that pane — through the shell's own
/// `request`, the path a Live Activity's tap takes.
///
/// No runner and no daemon: `-shell-board` puts a canned board on the
/// harness's runner (`HarnessBoard`), and its panes are the harness's tabs. So
/// nothing here waits on a connection, and nothing skips: every wait that
/// times out is a failure.
final class ShellBoardTests: XCTestCase {
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness", "-shell-overview", "-shell-4", "-shell-board"]
        app.launch()
        return app
    }

    /// `ShellGestureTests`'s probes, read the same way, and never skipped: the
    /// harness has no runner to be missing.
    private func probe(_ app: XCUIApplication, _ identifier: String) -> [String: String] {
        let probe = app.descendants(matching: .any).matching(identifier: identifier).firstMatch
        XCTAssertTrue(probe.waitForExistence(timeout: 30), "no \(identifier) in the tree")
        var parsed: [String: String] = [:]
        for field in (probe.value as? String ?? "").split(separator: " ") {
            let halves = field.split(separator: "=")
            guard halves.count == 2 else { continue }
            parsed[String(halves[0])] = String(halves[1])
        }
        return parsed
    }

    private func boardRow(_ app: XCUIApplication) -> XCUIElement {
        app.buttons["shell-board-harness-repo-overnight"]
    }

    /// Wait until `tab`'s pane says it is the one on screen.
    private func waitUntilVisible(_ app: XCUIApplication, _ tab: String) -> Bool {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if probe(app, "shell-pane-\(tab)")["visible"] == "1" { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return false
    }

    /// **The row is over the runner's cards, it says what the board holds, and
    /// it opens the board — Needs Decision first.**
    func testTheBoardRowOpensTheBoardNeedsDecisionFirst() throws {
        let app = launch()
        XCTAssertEqual(probe(app, "shell-state")["overview"], "1", "the harness did not open on the grid")

        let row = boardRow(app)
        XCTAssertTrue(row.waitForExistence(timeout: 10), "no Board row: \(app.debugDescription)")
        // The counts in words, because the digits alone say nothing about
        // which is which. One task in Needs Decision; agents on two tasks
        // (three panes, two of them on one task, and a dispatched shell that
        // is not an agent).
        XCTAssertEqual(row.value as? String, "1 task needs a decision, Agents are on 2 tasks")
        // Above the runner's first card, which is what "above its
        // workspaces" means on screen.
        let firstCard = app.buttons["shell-card-ws-0"]
        XCTAssertTrue(firstCard.waitForExistence(timeout: 5))
        XCTAssertLessThan(row.frame.maxY, firstCard.frame.minY, "the Board row is not above the cards")

        row.tap()
        let decision = app.descendants(matching: .any)["board-section-needs_decision"]
        let progress = app.descendants(matching: .any)["board-section-in_progress"]
        XCTAssertTrue(decision.waitForExistence(timeout: 10), "the board did not open")
        XCTAssertTrue(progress.exists)
        XCTAssertLessThan(decision.frame.minY, progress.frame.minY, "Needs Decision is not first")
        // No empty status is listed: nothing is in To Do on this board.
        XCTAssertFalse(app.descendants(matching: .any)["board-section-todo"].exists)

        // Acceptance, on the card, in AgentKit's words. A card's words are one
        // element, so they are read off its label.
        let card19 = app.descendants(matching: .any)["board-card--19"]
        let card18 = app.descendants(matching: .any)["board-card--18"]
        XCTAssertTrue(card19.exists && card18.exists, "the cards are not on the board")
        XCTAssertTrue(card19.label.contains("1 of 3"), "-19's acceptance: \(card19.label)")
        XCTAssertTrue(card18.label.contains("All 4 met"), "-18's acceptance: \(card18.label)")
        // And the question it is waiting on, which only Needs Decision asks.
        XCTAssertTrue(card19.label.contains("Answer to unblock this"), card19.label)
        // In progress with nobody on it: said, quietly. The dispatched shell
        // on it is not an agent.
        XCTAssertTrue(app.descendants(matching: .any)["board-no-agent--21"].exists)
        XCTAssertFalse(app.buttons["board-agent--21"].exists)
    }

    /// **One agent on a card is a button, and it lands on that agent's pane.**
    func testTheAgentButtonClosesTheBoardAndLandsOnThePane() throws {
        let app = launch()
        XCTAssertTrue(boardRow(app).waitForExistence(timeout: 30), "no Board row")
        boardRow(app).tap()

        let agent = app.buttons["board-agent--19"]
        XCTAssertTrue(agent.waitForExistence(timeout: 10), "-19 has no Agent button")
        agent.tap()

        XCTAssertTrue(
            waitUntilVisible(app, "ws-1-tab-1"),
            "the shell did not land on -19's agent: \(probe(app, "shell-pane-ws-1-tab-1"))")
        XCTAssertEqual(probe(app, "shell-state")["overview"], "0", "the overview is still up")
        XCTAssertFalse(app.descendants(matching: .any)["board"].exists, "the board is still up")
    }

    /// **Several agents on one card are a menu, and each item goes to its own
    /// pane** — here the second of two, so a menu that always took the first
    /// would fail.
    func testSeveralAgentsOnOneCardAreAMenuOfPanes() throws {
        let app = launch()
        XCTAssertTrue(boardRow(app).waitForExistence(timeout: 30), "no Board row")
        boardRow(app).tap()

        let menu = app.buttons["board-agent--20"]
        XCTAssertTrue(menu.waitForExistence(timeout: 10), "-20 has no agents control")
        menu.tap()
        let second = app.buttons["codex in fix/token-refresh"]
        XCTAssertTrue(second.waitForExistence(timeout: 5), "the menu does not list the second agent")
        XCTAssertTrue(app.buttons["claude in fix/token-refresh"].exists)
        second.tap()

        XCTAssertTrue(
            waitUntilVisible(app, "ws-2-tab-2"),
            "the shell did not land on the agent chosen: \(probe(app, "shell-pane-ws-2-tab-2"))")
    }

    /// **Tapping a card opens it: its intent, each acceptance line with its
    /// tick, and the same Agent button, which still lands on the pane.**
    func testACardOpensToItsIntentAcceptanceAndAgent() throws {
        let app = launch()
        XCTAssertTrue(boardRow(app).waitForExistence(timeout: 30), "no Board row")
        boardRow(app).tap()

        let card = app.buttons["board-card--19"]
        XCTAssertTrue(card.waitForExistence(timeout: 10), "-19's card is not a button")
        card.tap()

        let detail = app.descendants(matching: .any)["board-detail"]
        XCTAssertTrue(detail.waitForExistence(timeout: 10), "the card did not open")
        let heading = app.descendants(matching: .any)["board-detail-heading"]
        XCTAssertTrue(heading.label.contains("Board in the sidebar"), heading.label)
        XCTAssertTrue(heading.label.contains("Needs Decision"), heading.label)
        let intent = app.descendants(matching: .any)["board-detail-intent"]
        XCTAssertTrue(intent.exists, "no intent")
        XCTAssertTrue(intent.label.contains("Why task 19 exists"), intent.label)
        // Every line, each with its own tick: the first holds, the other two
        // do not. A detail that drew the count and not the lines, or ticked
        // every line, fails here.
        let lines = (0..<3).map { app.descendants(matching: .any)["board-acceptance-a19-\($0)"] }
        XCTAssertTrue(lines.allSatisfy(\.exists), "not every acceptance line is drawn")
        XCTAssertEqual(lines.map { $0.value as? String }, ["Met", "Not met", "Not met"])
        XCTAssertEqual(lines[1].label, "Line 2 of task 19 holds")

        let agent = app.buttons["board-agent--19"]
        XCTAssertTrue(agent.waitForExistence(timeout: 5), "the opened card has no Agent button")
        agent.tap()
        XCTAssertTrue(
            waitUntilVisible(app, "ws-1-tab-1"),
            "the opened card's Agent did not land: \(probe(app, "shell-pane-ws-1-tab-1"))")
    }

    /// **An agent whose pane has gone says so, on the board, and stays.**
    ///
    /// -20's menu offers a third pane the runner still names but the shell no
    /// longer has. Choosing it must not close the board onto nothing.
    func testAnAgentWhosePaneHasGoneSaysSoAndKeepsTheBoard() throws {
        let app = launch()
        XCTAssertTrue(boardRow(app).waitForExistence(timeout: 30), "no Board row")
        boardRow(app).tap()

        let menu = app.buttons["board-agent--20"]
        XCTAssertTrue(menu.waitForExistence(timeout: 10), "-20 has no agents control")
        menu.tap()
        let gone = app.buttons["aider in chore/put-away"]
        XCTAssertTrue(gone.waitForExistence(timeout: 5), "the menu does not list the closed pane")
        gone.tap()

        let notice = app.descendants(matching: .any)["board-notice"]
        XCTAssertTrue(notice.waitForExistence(timeout: 5), "nothing said the pane had closed")
        XCTAssertEqual(notice.label, "That agent’s pane has closed.")
        XCTAssertTrue(app.descendants(matching: .any)["board"].exists, "the board closed anyway")
        XCTAssertEqual(probe(app, "shell-state")["overview"], "1", "the shell moved anyway")
    }

    // MARK: - The real path, against the demo runner

    /// **Overview → Board row → card → Agent lands on the pane the runner
    /// dispatched for that task, through the app's own wiring.**
    ///
    /// Everything above drives the harness, whose Agent ids are tab ids handed
    /// straight to the shell. This drives what the app does with a real
    /// runner: `Connection` reading `task.list`, the fleet's `taskId`,
    /// `RunnerBoards.rows` over live panes, `BoardSheetHost`, and
    /// `ShellScreen.requestedTab` turning a terminal id into a tab.
    ///
    /// Needs `./scripts/demo-host.sh`, whose board fixture is one task and a
    /// `boarding` workspace with one `claude` pane opened for it — a stand-in
    /// that sleeps, never the real one. Skipped, naming why, when no runner
    /// answers, like every other demo-runner test; once one has, every step
    /// is an assertion, the Board row included.
    func testTheRealBoardRowLandsOnTheDispatchedPane() throws {
        let app = XCUIApplication()
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        app.launchArguments += ["-farcoolerDemoHost", "\(user)@\(host)"]
        app.launch()

        let shell = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard shell.waitForExistence(timeout: 180) else {
            throw XCTSkip("The shell never rendered against \(user)@\(host); run ./scripts/demo-host.sh.")
        }

        // Up past the last row, which is the only way into the overview. The
        // app reopens on whatever tab it was left on, and on a terminal the
        // keyboard can be up with the bar riding on it, so it is put away
        // first. Then a held lift, and if that did not arrive, a fling —
        // `ShellGestureTests.flickBar`'s velocity, for its reason: a
        // synthesized `.fast` is whatever the simulator negotiates.
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
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

        let rows = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "shell-board-"))
        // A failure and not a skip, once the runner has answered: the demo
        // script this suite is run against makes the board, and a row that
        // never came is the app not reading it — the one thing this test is
        // for. (If the script could not make the fixture safely it says so,
        // and this is where that shows.)
        XCTAssertTrue(
            rows.firstMatch.waitForExistence(timeout: 30),
            "no Board row from the demo runner; did ./scripts/demo-host.sh make its board?")
        let row = rows.firstMatch
        XCTAssertEqual(row.label, "scrollback Board")
        XCTAssertTrue(
            (row.value as? String ?? "").contains("An agent is on 1 task"),
            "the row does not count the stand-in agent: \(row.value ?? "nil")")
        row.tap()

        let card = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@",
                "board-card-", "Demo board task")).firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 15), "the demo task is not on the board")
        let key = String(card.identifier.dropFirst("board-card-".count))
        card.tap()
        XCTAssertTrue(
            app.descendants(matching: .any)["board-detail"].waitForExistence(timeout: 10),
            "the card did not open")

        let agent = app.buttons["board-agent-\(key)"]
        XCTAssertTrue(agent.waitForExistence(timeout: 10), "the opened card has no Agent button")
        agent.tap()

        // Landed: the overview is gone and the bar names the workspace the
        // dispatched pane is in, on that pane's tab. `boarding` holds the
        // Changes tab, the shell its creation opened, then the stand-in agent
        // — so tab 2, and not the shell a plain "open the workspace" might
        // have rested on.
        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline, probe(app, "shell-state")["overview"] != "0" {
            Thread.sleep(forTimeInterval: 0.3)
        }
        let state = probe(app, "shell-state")
        XCTAssertEqual(state["overview"], "0", "the overview is still up: \(state)")
        XCTAssertTrue(bar.label.contains("boarding"), "landed on \(bar.label), not boarding")
        XCTAssertEqual(state["tab"], "2", "not on the dispatched pane's tab: \(state)")
    }
}
