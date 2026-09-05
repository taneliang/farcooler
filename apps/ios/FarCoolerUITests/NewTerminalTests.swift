import XCTest

/// The phone can make a terminal.
///
/// This is the gap the suite went without an assertion for, and the shape of
/// the gap is the reason it needs a UI test rather than a unit one. Nothing was
/// broken: `terminal.create` has been a wire method since the protocol had one,
/// the daemon has always served it (`crates/daemon/src/rpc.rs`), and
/// `Connection.createTerminal` was written on this side and compiled fine. It
/// simply had no caller. A workspace on the phone could show its terminals,
/// switch between them, scroll them and type into them, and the first thing
/// anybody wants in a fresh worktree was the one thing there was no way to ask
/// for — and every Rust test, every AgentKit test and every other test in this
/// bundle passed the whole time.
///
/// So the assertion has to be on the phone DOING it, end to end, against a real
/// runner: a menu opened with a finger, a row tapped, and a tmux window that
/// exists afterwards. `tabs` off `shell-state` is what makes the last part
/// observable — the same probe `ShellGestureTests` reads, and the only place
/// the shell says out loud how many tabs a workspace has.
///
/// Needs a runner, like `TerminalScrollTests` and for the same reason. Stand
/// one up with `./scripts/demo-host.sh`, then run this through
/// `./scripts/ios-ui-tests.sh` — which is what forwards `DEMO_USER` (see that
/// script on why the assignment goes BEFORE `xcodebuild`) and what refuses a
/// run in which nothing executed. Skipped rather than failed with no runner:
/// a suite that goes red on a laptop with no demo host teaches people to
/// ignore it.
final class NewTerminalTests: XCTestCase {
    /// The runner string the app was last launched against, so a skip can name
    /// it. See `TerminalScrollTests.runner` — an empty `DEMO_USER` is invisible
    /// in every other line of output and costs a night.
    private var runner = "<never launched>"

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        runner = "\(user)@\(host)"
        app.launchArguments += ["-farcoolerDemoHost", runner]
        app.launch()
        return app
    }

    /// `ws`, `tab`, `workspaces`, `tabs`, … off the shell's probe.
    ///
    /// Read by NAME, never by position: `TerminalScrollTests.field` documents
    /// what reading that string positionally cost, and this one has grown four
    /// fields since it was written.
    private func state(_ app: XCUIApplication) -> [String: Int] {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.exists else { return [:] }
        var parsed: [String: Int] = [:]
        for pair in (probe.value as? String ?? "").split(separator: " ") {
            let halves = pair.split(separator: "=")
            guard halves.count == 2, let value = Int(halves[1]) else { continue }
            parsed[String(halves[0])] = value
        }
        return parsed
    }

    /// Wait for the shell to render at all, or skip saying which runner it was
    /// waiting on.
    ///
    /// 180 seconds, copied from `TerminalScrollTests.openATerminalInTheShell`
    /// along with its reasoning: `xcodebuild test` installs a fresh build and
    /// the first launches after an install are far slower than the rest, so a
    /// 60-second probe measured how recently the app was installed — and
    /// reported the answer by SKIPPING, which looks exactly like nothing being
    /// wrong.
    private func waitForShell(_ app: XCUIApplication) throws {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 180) else {
            print(app.debugDescription)
            throw XCTSkip(
                "The shell never rendered against \(runner); run ./scripts/demo-host.sh "
                    + "first, then ./scripts/ios-ui-tests.sh.")
        }
    }

    /// The overflow button of the pane actually on screen.
    ///
    /// By FRAME and not `firstMatch`, for the reason
    /// `TerminalScrollTests.visibleSurface` gives: `ShellPaneTrack` keeps the
    /// neighbouring panes mounted, so every one of them contributes a
    /// navigation bar with this button on it, and `firstMatch` returns whichever
    /// the accessibility tree happens to list first. Tapping an off-screen
    /// pane's menu would open a menu for a workspace nobody is looking at — and
    /// on a demo fleet the neighbour is a different workspace, so the terminal
    /// would be created in the wrong worktree and this test would go green on
    /// exactly the bug it would have caused.
    private func visibleOverflow(_ app: XCUIApplication) -> XCUIElement? {
        let all = app.buttons.matching(identifier: "Pane options")
        for i in 0..<all.count {
            let element = all.element(boundBy: i)
            guard element.exists, element.isHittable else { continue }
            if element.frame.midX > app.frame.minX, element.frame.midX < app.frame.maxX {
                return element
            }
        }
        return nil
    }

    /// Opening the overflow and tapping New Terminal makes one, and lands on it.
    ///
    /// Both halves are the feature. A create that leaves you where you were is
    /// a terminal you now have to go and find, which on a phone is a drag up
    /// into the column and a release on the right row — most of the cost of the
    /// thing you just asked for. macOS's `openTerminalInNewLayout` makes the
    /// same argument and selects what it made.
    func testTheOverflowMenuMakesATerminalAndLandsOnIt() throws {
        let app = launch()
        try waitForShell(app)

        let before = state(app)
        let tabsBefore = try XCTUnwrap(before["tabs"], "the shell published no tab count")
        let tabBefore = try XCTUnwrap(before["tab"], "the shell published no current tab")

        guard let overflow = visibleOverflow(app) else {
            print(app.debugDescription)
            throw XCTSkip("No pane with an overflow menu on \(runner).")
        }
        overflow.tap()

        // The row itself. This is the assertion that fails first, and it is the
        // one that describes the gap: before this lane there was no such
        // control anywhere in the app.
        let item = app.buttons["New Terminal"]
        XCTAssertTrue(
            item.waitForExistence(timeout: 10),
            "the pane's overflow menu offers no way to create a terminal")
        item.tap()

        // A create is a tmux window opening and then a poll bringing the fleet
        // back, so the tab cannot be there on the next frame. Polled rather
        // than slept on: `DaemonClient.createTerminal` waits up to three
        // seconds for exactly this on the Mac.
        let grew = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (self.state(app)["tabs"] ?? 0) > tabsBefore },
            object: nil)
        guard XCTWaiter.wait(for: [grew], timeout: 30) == .completed else {
            XCTFail(
                "the workspace still has \(tabsBefore) tabs, so tapping New Terminal "
                    + "created nothing on \(runner)")
            return
        }

        let after = state(app)
        XCTAssertEqual(
            after["tabs"], tabsBefore + 1,
            "one tap should add exactly one tab")
        // Landed on it. `tab` is an index into a list that just grew, so the
        // claim is only that the shell MOVED — asserting a particular index
        // here would be asserting the daemon's ordering of a workspace's
        // terminals, which is not this app's to pin.
        XCTAssertNotEqual(
            after["tab"], tabBefore,
            "the terminal was created but the shell stayed on tab \(tabBefore)")
        // Tab 0 is the workspace's Changes pane — see `ShellFleetMap.of` — so
        // anything else is a terminal. Without this, a shell that merely
        // rearranged itself would satisfy the line above.
        XCTAssertGreaterThan(
            try XCTUnwrap(after["tab"]), 0,
            "the shell landed on the Changes tab rather than on the new terminal")
    }
}
