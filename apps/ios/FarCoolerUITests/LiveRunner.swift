import XCTest

/// The demo runner `scripts/demo-host.sh` stands up, and the way into one of
/// its worktrees through the phone's stack (ov-55).
///
/// The app opens on Needs You now, not on a pane, so every live-runner test
/// walks in: Needs You, then the workspace (or the Unclaimed group) that
/// holds the worktree, then its row, which opens the worktree's panes.
///
/// **A missing runner is not a pass.** When Needs You never lists a
/// worktree, the test skips with `LiveRunner.missing` at the front of its
/// message, and `scripts/ios-ui-tests.sh` goes red on any skip that says so.
/// A skip that read as green is how the whole live half of this suite once
/// stopped testing anything for a night.
enum LiveRunner {
    /// What a skip for an absent runner starts with. The script greps for it.
    static let missing = "NO LIVE RUNNER:"

    /// `user@host:port`, forwarded by xcodebuild as `TEST_RUNNER_DEMO_*`.
    static var address: String {
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        return "\(user)@\(host)"
    }

    /// Launch the app against the demo runner.
    static func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += extra + ["-farcoolerDemoHost", address]
        app.launch()
        return app
    }

    /// Walk from Needs You into the worktree called `name`, landing on the
    /// pane it would open on, and wait until the shell is drawn.
    ///
    /// 180 s for the first sight of the runner's worktrees: a first launch
    /// after an install is slow on the simulator (see
    /// `TerminalScrollTests.openATerminalInTheShell`'s measurement).
    static func openWorktree(_ app: XCUIApplication, named name: String) throws {
        let row = app.buttons["worktree-row-\(name)"]
        let needsYou = app.navigationBars["Needs You"]
        let deadline = Date().addingTimeInterval(180)
        // The launch may have pushed the last workspace; Needs You is under it.
        while Date() < deadline {
            if needsYou.exists, app.buttons.matching(identifierPrefix: "workspace-row-").count > 0 {
                break
            }
            let back = app.navigationBars.buttons["BackButton"].firstMatch
            if back.exists && back.isHittable { back.tap() }
            Thread.sleep(forTimeInterval: 1)
        }
        guard needsYou.exists else {
            print(app.debugDescription)
            throw XCTSkip(
                "\(missing) Needs You never listed a workspace from \(address); run "
                    + "./scripts/demo-host.sh first, then ./scripts/ios-ui-tests.sh.")
        }

        // Unclaimed first: the demo's worktrees are made with the CLI, which
        // claims nothing on a runner that predates workspaces and Main on one
        // that has them.
        let unclaimed = app.descendants(matching: .any)["worktrees-unclaimed"].firstMatch
        if unclaimed.exists {
            if !row.exists { unclaimed.tap() }
            if row.waitForExistence(timeout: 3) {
                row.tap()
                return try waitForShell(app, name)
            }
        }
        let workspaces = app.buttons.matching(identifierPrefix: "workspace-row-")
        for index in 0..<workspaces.count {
            workspaces.element(boundBy: index).tap()
            // A workspace with an orchestrator has its worktrees in its tree
            // (ov-300): one with no card is under Loose Worktrees.
            guard app.buttons["segment-board"].waitForExistence(timeout: 10) else { continue }
            let segment = app.buttons["segment-tree"].exists ? app.buttons["segment-tree"] : app.buttons["segment-worktrees"]
            segment.tap()
            let loose = app.buttons["tree-row-Loose Worktrees"]
            var pushed = false
            if !row.waitForExistence(timeout: 3), loose.exists {
                loose.tap()
                pushed = true
            }
            if row.waitForExistence(timeout: 5) {
                row.tap()
                return try waitForShell(app, name)
            }
            if pushed { app.navigationBars.buttons["BackButton"].firstMatch.tap() }
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
        }
        print(app.debugDescription)
        throw XCTSkip(
            "\(missing) no worktree called '\(name)' on \(address); run ./scripts/demo-host.sh.")
    }

    /// The worktree's shell, up: its pane bar's Back and the shell's probe.
    private static func waitForShell(_ app: XCUIApplication, _ name: String) throws {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 30),
            probe.waitForExistence(timeout: 30)
        else {
            print(app.debugDescription)
            XCTFail("the worktree '\(name)' did not open")
            return
        }
    }
}

extension XCUIElementQuery {
    /// Elements whose identifier starts with `prefix`.
    func matching(identifierPrefix prefix: String) -> XCUIElementQuery {
        matching(NSPredicate(format: "identifier BEGINSWITH %@", prefix))
    }
}
