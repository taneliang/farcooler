import XCTest

/// A reader scrolled up in the iPhone's conversation stays where they are
/// (ov-626), over `-native-agent-harness -native-live`: a long page, a line
/// streamed on every follow, a queued message that settles six follows after
/// it's sent, and a reset (a resync) every fifth follow. Only their own send
/// and Jump to Latest take them to the bottom. The list says where it is in
/// its debug value: `tail=`, `top=` (the visible top, in points) and
/// `jumps=N last=<reason>` (every move to the tail, and why).
final class ConversationScrollUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-live", "-native-busy"] + extra
        app.launchDrawn()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "no conversation")
        return app
    }

    /// The list's debug value, and one of its fields.
    private func said(_ app: XCUIApplication) -> String {
        element(app, "native-transcript").value as? String ?? ""
    }

    private func field(_ app: XCUIApplication, _ key: String) -> String {
        said(app).split(separator: " ").first { $0.hasPrefix("\(key)=") }.map { String($0.dropFirst(key.count + 1)) } ?? ""
    }

    private func top(_ app: XCUIApplication) -> Int { Int(field(app, "top")) ?? -1 }

    /// Send `words` from the box.
    private func send(_ app: XCUIApplication, _ words: String) {
        let box = element(app, "native-composer")
        box.tap()
        XCTAssertTrue(wait(30) { box.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the box never took the keyboard")
        app.typeText(words)
        XCTAssertTrue(wait(30) { (box.value as? String ?? "").hasSuffix(words) }, "the box didn't take the words")
        element(app, "native-send").tap()
    }

    /// Up the list by about two screens, the way a reader does, and wait
    /// for it to come to rest.
    private func scrollUp(_ app: XCUIApplication) {
        let list = element(app, "native-transcript")
        for _ in 0..<2 {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 320)), withVelocity: .default, thenHoldForDuration: 0.1)
        }
        XCTAssertTrue(wait(10) { field(app, "tail") == "false" }, "the list didn't leave its tail: \(said(app))")
        // At rest: the top stops changing.
        var last = top(app)
        _ = wait(10) {
            Thread.sleep(forTimeInterval: 0.6)
            let now = top(app)
            defer { last = now }
            return now == last
        }
    }

    /// A reader who moves up only a line or two, to read again what just
    /// went by, stays there too: within 40 pt of the end the list called
    /// itself still on its tail, and the next streamed line took them back.
    func testAReaderScrolledUpALineStaysPut() {
        let app = launch()
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "the list didn't start on its tail: \(said(app))")
        let list = element(app, "native-transcript")
        let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        from.press(forDuration: 0.3, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 30)), withVelocity: .slow, thenHoldForDuration: 0.5)
        Thread.sleep(forTimeInterval: 1)
        let before = top(app)
        let jumps = field(app, "jumps")
        Thread.sleep(forTimeInterval: 5)
        XCTAssertEqual(field(app, "jumps"), jumps, "the list jumped back to its tail: \(said(app))")
        XCTAssertLessThanOrEqual(abs(top(app) - before), 2, "the list moved under the reader: \(said(app))")
    }

    /// Through ten follows (a line streamed on each, two resets) and the
    /// queued message settling into a turn, the reader stays put.
    func testAReaderScrolledUpStaysPut() {
        let app = launch()
        send(app, "Settle me later")
        XCTAssertTrue(element(app, "native-queued").waitForExistence(timeout: 15), "the busy runner didn't queue it")
        scrollUp(app)
        let before = top(app)
        let jumps = field(app, "jumps")
        // Ten follows of about 0.7 s: streaming, the settle and two resets.
        Thread.sleep(forTimeInterval: 9)
        XCTAssertFalse(element(app, "native-queued").exists, "the queued message never settled, so this proved nothing")
        let after = top(app)
        XCTAssertEqual(field(app, "tail"), "false", "the reader was taken back to the tail: \(said(app))")
        XCTAssertLessThanOrEqual(abs(after - before), 2, "the list moved from \(before) to \(after) under the reader: \(said(app))")
        XCTAssertEqual(field(app, "jumps"), jumps, "the list jumped to its tail: \(said(app))")
    }

    /// The owner's screen, where the list's bottom inset is largest (the tray
    /// and the composer): a reader up a line stays there while rows stream
    /// and a queued message settles.
    func testAReaderUpALineStaysPutOnTheOrchestrator() {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        let app = XCUIApplication.phoneHarness([
            "-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file",
            url.appendingPathComponent("test/fixtures/plan-seeded.json").path, "-native-agents", "-native-live", "-native-busy",
        ])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
        main.tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "no conversation")
        send(app, "Settle me later")
        XCTAssertTrue(element(app, "native-queued").waitForExistence(timeout: 15), "the busy runner didn't queue it")
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "the send didn't bring the list to its tail: \(said(app))")
        let list = element(app, "native-transcript")
        let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
        from.press(forDuration: 0.3, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 30)), withVelocity: .slow, thenHoldForDuration: 0.5)
        Thread.sleep(forTimeInterval: 1)
        let before = top(app)
        let jumps = field(app, "jumps")
        Thread.sleep(forTimeInterval: 6)
        XCTAssertFalse(element(app, "native-queued").exists, "the queued message never settled, so this proved nothing")
        XCTAssertEqual(field(app, "jumps"), jumps, "the list jumped back to its tail: \(said(app))")
        XCTAssertLessThanOrEqual(abs(top(app) - before), 2, "the list moved under the reader: \(said(app))")
    }

    /// Their own send, and Jump to Latest, still take them to the bottom.
    func testSendAndJumpToLatestGoToTheBottom() {
        let app = launch()
        scrollUp(app)
        send(app, "Back to the bottom")
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "a send didn't bring the reader down: \(said(app))")
        scrollUp(app)
        let jump = element(app, "native-jump-to-latest")
        XCTAssertTrue(jump.waitForExistence(timeout: 10), "no Jump to Latest")
        jump.tap()
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "Jump to Latest didn't: \(said(app))")
    }

    /// Reaching the top loads the older rows above, and the row the reader
    /// was on stays where it was on the screen: measured before the load
    /// (which the harness answers twenty seconds late, the list asking as it opens) and after it.
    func testLoadingOlderRowsKeepsThePlace() {
        let app = launch(["-native-live-older"])
        let list = element(app, "native-transcript")
        let first = element(app, "native-row-prose:live0")
        let older = element(app, "native-row-prose:old14")
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        // Up to the page's first row; the older rows above it take twenty
        // seconds to come, so its place is taken before they do.
        for _ in 0..<12 where !(first.exists && first.frame.minY > 0) {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 500)), withVelocity: .fast, thenHoldForDuration: 0)
        }
        XCTAssertTrue(first.exists && first.frame.minY > 0, "never reached the page's first row: \(said(app))")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertFalse(older.exists, "the older rows came before the place could be taken")
        let before = first.frame.minY
        XCTAssertTrue(older.waitForExistence(timeout: 40), "the older rows never loaded: \(said(app))")
        Thread.sleep(forTimeInterval: 1)
        // Not yet: the row is pushed down by the new rows' height (149 to
        // 1002 pt measured). ov-629 fixes it and takes this away.
        XCTExpectFailure("ov-629: older rows loading above move the reader's row")
        // Within a line.
        XCTAssertEqual(first.frame.minY, before, accuracy: 24, "the reader's row moved from \(before) as the older rows came: \(said(app))")
    }
}
