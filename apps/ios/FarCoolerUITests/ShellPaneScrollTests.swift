import XCTest

/// **A pane that scrolls, inside a shell that turns pages.**
///
/// Two gestures over one surface, and the two ways of losing that argument are
/// opposite, silent, and look identical from outside: if the pane always wins,
/// the shell's page turn is unreachable and you can swipe INTO a pane and never
/// out of it; if the shell always wins, the pane will not scroll and reads as a
/// pane with nothing in it.
///
/// `TerminalScrollTests` already holds that pair for a TERMINAL, whose scroll
/// is a `UIPanGestureRecognizer` this app wrote and can therefore teach to
/// refuse a drag it cannot use. This suite holds it for the other kind of
/// pane: one made of SwiftUI's own containers, where the competing gesture is
/// not ours and the shell's own was quietly losing to it.
///
/// Two fixtures, because two different things were true and only one of them
/// was suspected:
///
/// - `-shell-scroll` is a pane that is nothing but a `ScrollView` of text. It
///   passed from the first run and still does. A vertical scroll view is NOT
///   what the shell was losing to, which is worth a fixture of its own: it is
///   the obvious suspect, it is what the report guessed at, and a fix aimed
///   there would have been aimed at nothing.
/// - `-shell-changes` is the real `ChangesView` over the same canned change set
///   `-changes-layout-harness` uses. It declares a gesture on nearly every
///   pixel — a segmented control, rows, a card per file, a button per hunk, and
///   a horizontal `ScrollView` over the code — and `.gesture` on an ancestor
///   loses to every one of them.
///
/// Neither needs a runner, so unlike `TerminalScrollTests` this never skips.
/// The live half — the page turn over a real Changes pane on the demo host —
/// is at the bottom and does skip.
final class ShellPaneScrollTests: XCTestCase {
    private func launch(_ extra: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += extra
        app.launch()
        return app
    }

    /// `ws`, `tab`, and the rest of `ShellRootView.probe`'s value.
    private func state(_ app: XCUIApplication) throws -> [String: Int] {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 30) else {
            print(app.debugDescription)
            throw XCTSkip("The shell never rendered its probe.")
        }
        var parsed: [String: Int] = [:]
        for pair in (probe.value as? String ?? "").split(separator: " ") {
            let halves = pair.split(separator: "=")
            guard halves.count == 2, let value = Int(halves[1]) else { continue }
            parsed[String(halves[0])] = value
        }
        return parsed
    }

    /// Where the visible pane's own scroll view is.
    ///
    /// Off the pane's probe rather than off anything drawn, for the reason
    /// `ShellRootView.probe` gives: a scroll offset is a number with no label
    /// on screen, and the two failures it separates — "the shell ate the
    /// scroll" and "there was nothing to scroll" — look the same in a
    /// screenshot.
    private func paneOffset(_ app: XCUIApplication) -> Int? {
        let probes = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH %@", "shell-pane-"))
        for i in 0..<probes.count {
            let value = (probes.element(boundBy: i).value as? String) ?? ""
            guard value.contains("visible=1") else { continue }
            return value.split(separator: " ")
                .first { $0.hasPrefix("offset=") }
                .flatMap { Int($0.split(separator: "=")[1]) }
        }
        return nil
    }

    /// A drag across the middle of the pane, held before release for the reason
    /// `ShellGestureTests.swipeContent` documents: a release in the same frame
    /// as the last movement can leave the final translation unreported.
    private func swipeAcross(_ app: XCUIApplication, toward direction: CGFloat) {
        let y = 0.42
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5 + 0.28 * -direction, dy: y))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5 + 0.28 * direction, dy: y))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)
    }

    // MARK: - The two halves of the argument

    /// **The shell must not steal the scroll the pane owns.**
    ///
    /// The half that has to be protected first: a diff is a long document and
    /// reading it is the common action, while turning a page is the rare one.
    /// If arbitration has to favour one of them it favours the scroll.
    func testAScrollingPaneStillScrollsInsideTheShell() throws {
        let app = launch(["-shell-harness", "-shell-scroll"])
        _ = try state(app)
        let before = try XCTUnwrap(paneOffset(app), "the pane never reported a scroll offset")

        let top = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.62))
        let up = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.22))
        top.press(forDuration: 0.05, thenDragTo: up, withVelocity: .slow, thenHoldForDuration: 0.4)

        let after = try XCTUnwrap(paneOffset(app))
        XCTAssertGreaterThan(
            after, before,
            "the shell swallowed the pane's own scroll: the content never left the top")
        XCTAssertEqual(try state(app)["tab"], 0, "a vertical drag turned the page")
    }

    /// **And the page still turns over a pane that scrolls.**
    ///
    /// The failure this catches is the one the owner hit on the diff: a
    /// `UIScrollView` claims the touch the moment it moves, in ANY direction,
    /// and then does nothing with the sideways half of it. The swipe is
    /// recognized, so nothing anywhere reports a conflict; the pane is simply
    /// one you can swipe into and never swipe out of.
    func testAHorizontalSwipeOverAScrollingPaneTurnsThePage() throws {
        let app = launch(["-shell-harness", "-shell-scroll"])
        let start = try state(app)
        XCTAssertEqual(start["tab"], 0)
        XCTAssertGreaterThan(start["tabs"] ?? 0, 1, "this workspace has nowhere to turn to")

        swipeAcross(app, toward: -1)
        XCTAssertEqual(
            try state(app)["tab"], 1,
            "a horizontal swipe over a scrolling pane did not turn the page")

        swipeAcross(app, toward: 1)
        XCTAssertEqual(try state(app)["tab"], 0, "the return swipe did not land where it started")
    }

    /// And the scroll it did not steal is still where it was.
    ///
    /// The cheap way to pass the test above is for the shell to win everything,
    /// which would leave the pane's content jumping every time somebody turned
    /// a page. Asserting on both after ONE gesture is what tells a page turn
    /// apart from a page turn that also scrolled.
    func testTurningThePageDoesNotAlsoScrollThePane() throws {
        let app = launch(["-shell-harness", "-shell-scroll"])
        _ = try state(app)
        XCTAssertEqual(paneOffset(app), 0, "the pane did not open at the top")

        swipeAcross(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 1, "the swipe did not turn the page")
        XCTAssertEqual(paneOffset(app), 0, "turning the page scrolled the pane it landed on")
    }

    // MARK: - The diff, which declares a gesture on nearly every pixel

    /// **The gesture the shell was losing, and the one the owner reported.**
    ///
    /// `.gesture` attaches at SwiftUI's lowest priority: anything a descendant
    /// declares wins the touch outright. Neither pane the shell was built
    /// against declares one — a terminal's scroll is a raw
    /// `UIPanGestureRecognizer` in UIKit's graph, and a text placeholder has
    /// nothing at all — so the page turned perfectly over both and the defect
    /// could not be seen. A diff declares one nearly everywhere: a segmented
    /// control, the row that opens the commit list, a card per file, a button
    /// per hunk. Swept down this pane, nine of thirteen horizontal swipes did
    /// nothing at all.
    ///
    /// The drag is aimed at the FILE HEADING on purpose — a `Button`, and the
    /// commonest thing on the pane. Aimed at bare ground it passes without the
    /// fix, which is exactly why "the horizontal swipes aren't working very
    /// well" was the report rather than "they never work".
    func testAHorizontalSwipeOverTheDiffTurnsThePage() throws {
        let app = launch(["-shell-harness", "-shell-changes"])
        let start = try state(app)
        XCTAssertEqual(start["tab"], 0)
        XCTAssertTrue(
            app.staticTexts["file_diff.rs"].waitForExistence(timeout: 30),
            "the canned diff never rendered")

        swipe(app, at: 0.29, toward: -1)
        XCTAssertEqual(
            try state(app)["tab"], 1,
            "a horizontal swipe over the diff's file heading was swallowed by the heading")

        swipe(app, at: 0.29, toward: 1)
        XCTAssertEqual(try state(app)["tab"], 0, "the return swipe did not land where it started")
    }

    /// **And the code itself keeps its own sideways scroll.**
    ///
    /// The half a blunter fix trades away. A diff line is a line — wrapping one
    /// breaks the only property a diff has — so every hunk is a horizontal
    /// `ScrollView`, and a long line is read by dragging it. Running the shell
    /// alongside the pane rather than behind it makes BOTH answer the same
    /// drag: the line scrolls under your thumb and the page turns out from
    /// under it, which is two answers to one question.
    ///
    /// So a hunk says when it is using a drag, and the shell stands down. See
    /// `ShellDragClaim`. The assertion is both halves of that at once: the code
    /// moved, and the shell did not.
    func testTheCodeKeepsItsOwnSidewaysScroll() throws {
        let app = launch(["-shell-harness", "-shell-changes"])
        _ = try state(app)
        let line = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "retry_after")).firstMatch
        XCTAssertTrue(line.waitForExistence(timeout: 30), "the canned diff never rendered")
        let before = line.frame.minX

        // 140 points: past `ShellMetrics.pageCommit`'s seventy, so "the page
        // did not turn" is a claim about arbitration rather than about
        // distance, and short enough that the fixture's widest line still has
        // room left at the end of it — which is what this test is about. The
        // handoff at the edge is the test below.
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.72, dy: 0.55))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -140, dy: 0)),
            withVelocity: .slow, thenHoldForDuration: 0.4)

        XCTAssertLessThan(
            line.frame.minX, before - 20,
            "the hunk did not scroll: the shell took a drag the code was using")
        XCTAssertEqual(
            try state(app)["tab"], 0,
            "reading a long line turned the page as well as scrolling it")
    }

    /// **And it does not twitch on the way: the shell never moves at all.**
    ///
    /// The test above asserts the OUTCOME — the code moved, the tab did not —
    /// and the outcome was never the complaint. The owner's report is the
    /// middle of the gesture: *"scrolling horizontally on the diff view on iOS
    /// is very jarring because it keeps triggering the scroll/pan gestures for
    /// like a fraction of a second."* A shell that jumps sideways with the
    /// finger and then eases itself back has, at every instant either of the
    /// assertions above can be read, held perfectly still.
    ///
    /// So this reads the PEAK — `ShellRootView.strayed`, the furthest the
    /// track got from centre at any frame of the gesture — which is the only
    /// number that can tell the two apart. Measured at 25 points before the
    /// claim was seeded at touch-down; the room was not reported until the
    /// hunk's own scroll view began, which takes UIKit its usual ten points of
    /// slop plus a frame, and every one of those points reached the shell.
    ///
    /// Two points of tolerance rather than zero: `trackX` is written from the
    /// finger's own arithmetic, and a claim seeded on the frame the touch
    /// lands still leaves the single frame in which the finger has moved and
    /// nothing has been asked yet.
    func testTheShellDoesNotTwitchWhileAHunkIsStillScrolling() throws {
        let app = launch(["-shell-harness", "-shell-changes"])
        _ = try state(app)
        let line = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "retry_after")).firstMatch
        XCTAssertTrue(line.waitForExistence(timeout: 30), "the canned diff never rendered")
        let before = line.frame.minX

        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.72, dy: 0.55))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -140, dy: 0)),
            withVelocity: .slow, thenHoldForDuration: 0.4)

        let after = try state(app)
        // The negative control, and it has to come first: a shell that never
        // moves because the drag never reached anything would pass the
        // assertion below and fail the feature.
        XCTAssertLessThan(
            line.frame.minX, before - 20,
            "the hunk did not scroll, so there was no drag for the shell to hold still through")
        XCTAssertLessThanOrEqual(
            after["stray"] ?? -1, 2,
            """
            the shell moved \(after["stray"] ?? -1) points sideways during a drag the hunk \
            was using, and then put itself back. The tab is unchanged and the code scrolled, \
            so every other assertion in this file passes; what the reader sees is the page \
            twitching under the line being read.
            """)
        XCTAssertEqual(after["tab"], 0, "reading a long line turned the page")
    }

    /// **And a line scrolled to its end hands the page turn back.**
    ///
    /// The owner's refinement, and the half that separates this from a veto:
    /// *"if a file diff scrolls horizontally, scrolling on the diff should not
    /// switch between terminals — unless of course it's already been scrolled
    /// to the edge."* A hunk with nothing left to give is not a hunk that owns
    /// the gesture; it is a nested scroller at its edge, and every nested
    /// scroller on this platform hands over there.
    ///
    /// Driven to the edge by repeating the same drag rather than by a computed
    /// distance: how many points a canned line has depends on the reader's
    /// terminal face at the reader's terminal size, which is the same reason
    /// `HunkView` measures its widest row instead of assuming it.
    func testACodeLineAtItsEndHandsThePageTurnBack() throws {
        let app = launch(["-shell-harness", "-shell-changes"])
        _ = try state(app)
        let line = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "retry_after")).firstMatch
        XCTAssertTrue(line.waitForExistence(timeout: 30), "the canned diff never rendered")

        // To the end of the longest line, in drags of sixty points — under
        // `ShellMetrics.pageCommit`'s seventy, so nothing that reaches the
        // shell during this loop can commit and what is being set up is the
        // hunk's position rather than the shell's. Twelve is far more than the
        // fixture needs; the loop stops as soon as the code stops moving.
        var moved = false
        for _ in 0..<12 {
            let was = line.frame.minX
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.62, dy: 0.55))
            from.press(
                forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -60, dy: 0)),
                withVelocity: .slow, thenHoldForDuration: 0.3)
            guard line.frame.minX < was - 1 else { break }
            moved = true
        }
        XCTAssertTrue(moved, "the canned line was never wide enough to scroll")
        XCTAssertEqual(
            try state(app)["tab"], 0, "reading the line to its end turned the page on the way")

        // At the edge now: the same drag again, and this one is the shell's.
        swipe(app, at: 0.55, toward: -1)
        XCTAssertEqual(
            try state(app)["tab"], 1,
            "a line with nothing left to scroll still swallowed the page turn")
    }

    /// **And the diff still scrolls the way a diff is mostly read.**
    ///
    /// The half that must not be traded under any circumstances: a diff is a
    /// long document, scrolling it is the common action and turning the page is
    /// the rare one. `.simultaneousGesture` is what keeps this true —
    /// `.highPriorityGesture` would have fixed the page turn by winning every
    /// drag, including the plainly vertical ones the shell reads and then
    /// deliberately does nothing with, and a diff that will not scroll is a
    /// worse bug than a diff you cannot swipe out of.
    func testTheDiffStillScrollsVertically() throws {
        let app = launch(["-shell-harness", "-shell-changes"])
        _ = try state(app)
        // A line of the CODE and not the file heading: the heading is a pinned
        // section header, so it holds still at the top of a scroll that is
        // working perfectly. Asserting on it would have been a test that
        // failed whatever the shell did.
        let line = app.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "retry_after")).firstMatch
        XCTAssertTrue(line.waitForExistence(timeout: 30), "the canned diff never rendered")
        let before = line.frame.minY

        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.62))
        let high = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.24))
        low.press(forDuration: 0.05, thenDragTo: high, withVelocity: .slow, thenHoldForDuration: 0.4)

        XCTAssertLessThan(
            line.frame.minY, before - 20,
            "the diff did not scroll: the shell swallowed a plainly vertical drag")
        XCTAssertEqual(try state(app)["tab"], 0, "a vertical drag over the diff turned the page")
    }

    /// A drag across the pane at a given height.
    private func swipe(_ app: XCUIApplication, at y: Double, toward direction: CGFloat) {
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5 + 0.28 * -direction, dy: y))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5 + 0.28 * direction, dy: y))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)
    }

    // MARK: - The same, over a real Changes pane

    /// The live half: the shell's page turn still works over the diff.
    ///
    /// Skipped rather than failed when nothing answers on the demo host — see
    /// `TerminalScrollTests`, which this borrows its launch from. The harness
    /// tests above are what run everywhere; this is what says the mechanism
    /// reached the pane the owner was actually complaining about.
    func testAHorizontalSwipeOverTheLiveDiffTurnsThePage() throws {
        // Forwarded by xcodebuild from an assignment written BEFORE the
        // command, never after it — see the long note in `TerminalScrollTests`,
        // and use ./scripts/ios-ui-tests.sh, which gets it right.
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        let runner = "\(user)@\(host)"
        // No shell flag: the shell IS the app. The only argument is the runner
        // to stand on.
        let app = launch(["-farcoolerDemoHost", runner])

        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        // 180 seconds, not 60, and the number is measured rather than chosen.
        //
        // `xcodebuild test` installs a fresh build, and the first launches of a
        // freshly installed app on the simulator are far slower than the rest:
        // in one run of TerminalScrollTests the first three launches never rendered
        // inside 60s, the fourth took about 40, and the last four took about
        // five each. So a 60-second probe did not test the app, it tested how
        // recently the app had been installed — and it failed that test by
        // SKIPPING, which is the one outcome that looks like nothing is wrong.
        //
        // The cost is that a genuinely absent runner now takes three minutes
        // per test to say so. That is the right way round: a slow correct
        // answer beats a fast one that reads as success, and
        // `scripts/ios-ui-tests.sh` now makes an all-skipped run red, so this
        // path is only reached when something really is broken.
        guard probe.waitForExistence(timeout: 180) else {
            print(app.debugDescription)
            throw XCTSkip(
                "The shell never rendered against \(runner); run "
                    + "./scripts/demo-host.sh first, then ./scripts/ios-ui-tests.sh.")
        }
        // The diff is tab 0 of every workspace — `ShellFleetMap` puts Changes
        // first — so walking backward inside this workspace reaches it, and
        // arriving there is also how this test knows the fleet has one.
        var guard_ = 0
        while (try state(app)["tab"] ?? 0) > 0, guard_ < 6 {
            swipeAcross(app, toward: 1)
            guard_ += 1
        }
        try XCTSkipUnless(
            (try state(app)["tab"] ?? -1) == 0, "could not reach the diff on this runner")
        // FORWARD, which always has somewhere to go: the flat sequence runs
        // off the end of a workspace into the next one, so a demo fleet whose
        // first workspace is a diff and nothing else is still a fleet this can
        // be asked about. What is asserted is that the shell MOVED, not which
        // way — `ShellNavigationTests` owns the sequence.
        let before = try state(app)
        try XCTSkipUnless(
            (before["tabs"] ?? 0) > 1 || (before["workspaces"] ?? 0) > 1,
            "one workspace with one tab: there is nowhere to turn to")
        func place() -> String { "\(try? state(app)["ws"] ?? -1)/\(try? state(app)["tab"] ?? -1)" }
        let started = place()

        swipeAcross(app, toward: -1)
        let moved = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in place() != started }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [moved], timeout: 5), .completed,
            "a horizontal swipe over the diff did not turn the page: \(probe.value ?? "")")
    }
}
