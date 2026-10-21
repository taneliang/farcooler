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

    private func post(_ name: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name as CFString), nil, nil, true)
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

    /// The owner's screen, where the list's bottom inset is largest (the
    /// composer): a reader up a line stays there while rows stream
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
        // The settle is the event under test; a loaded machine only makes it
        // later, so wait for it rather than a fixed time.
        XCTAssertTrue(wait(60) { !element(app, "native-queued").exists }, "the queued message never settled, so this proved nothing")
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
    /// (which the harness holds until the test posts `native-older`) and after it.
    func testLoadingOlderRowsKeepsThePlace() {
        let app = launch(["-native-live-older"])
        let list = element(app, "native-transcript")
        let first = element(app, "native-row-prose:live0")
        let older = element(app, "native-row-prose:old14")
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        // Up to the page's first row; the older rows above it wait for the
        // test's signal, so its place is taken before they come.
        for _ in 0..<12 where !(first.exists && first.frame.minY > 0) {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 500)), withVelocity: .fast, thenHoldForDuration: 0)
        }
        XCTAssertTrue(first.exists && first.frame.minY > 0, "never reached the page's first row: \(said(app))")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertFalse(older.exists, "the older rows came before the place could be taken")
        let before = first.frame.minY
        post("com.farcooler.harness.native-older")
        XCTAssertTrue(older.waitForExistence(timeout: 60), "the older rows never loaded: \(said(app))")
        Thread.sleep(forTimeInterval: 1)
        // Within 2 pt (it was pushed down by the new rows' whole height,
        // 149 to 1002 pt, before ov-629).
        XCTAssertEqual(first.frame.minY, before, accuracy: 2, "the reader's row moved from \(before) as the older rows came: \(said(app))")
    }

    /// A page that comes while the reader is still flinging up to the top
    /// (the list asks for it as the spinner comes into view, mid-fling, and a
    /// fast runner answers at once: the harness answers as the finger lands)
    /// waits for the scroll to rest, and is then put in without moving the row
    /// the reader is on (ov-629 review): the store says how many changes
    /// waited, and the held row says where it was and where it is.
    func testAPageThatComesMidFlingKeepsThePlace() {
        let app = launch(["-native-live-older", "-native-live-older-on-scroll"])
        let list = element(app, "native-transcript")
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        // Flings up, until a page has been asked for and answered while a finger
        // was on the list; then no more, so what is measured is the rest after it.
        func waited() -> Int { field(app, "hold").split(separator: "/").first.flatMap { Int($0) } ?? 0 }
        for _ in 0..<12 where waited() < 1 {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 500)), withVelocity: .fast, thenHoldForDuration: 0)
        }
        XCTAssertGreaterThanOrEqual(waited(), 1, "no page waited for the scroll to rest, so this proved nothing: \(said(app))")
        // The page goes in when the list comes to rest, and the row held is put
        // right over a second or so.
        XCTAssertTrue(wait(30) { self.field(app, "hold").split(separator: "/").last.map { $0 != "none" } ?? false },
            "the page never went in with a row to hold: \(said(app))")
        Thread.sleep(forTimeInterval: 3)
        // `before>after@id`: the held row's place before the page went in, as
        // the list measured it, and its id. Where it is now is read from the
        // screen: the row may have left the list and come back, so the last
        // place the list heard of it says nothing.
        let held = String(field(app, "hold").split(separator: "/").last ?? "none")
        let id = held.split(separator: "@", maxSplits: 1).last.map(String.init) ?? ""
        let before = held.split(separator: "@").first.flatMap { $0.split(separator: ">").first }.flatMap { Double($0) }
        XCTAssertNotNil(before, "no row was held: \(said(app))")
        let row = element(app, "native-row-\(id)")
        XCTAssertTrue(row.waitForExistence(timeout: 20), "the held row \(id) is not on the screen: \(said(app))")
        // The list's places are from the top of its content inset.
        let inset = Double(field(app, "inset")) ?? 0
        if let before {
            XCTAssertEqual(row.frame.minY, before + inset, accuracy: 2, "the held row moved from \(before + inset) as the page went in: \(said(app))")
        }
    }

    /// One look at the list, from a single snapshot so its numbers agree:
    /// its debug value, and the rows on the screen (clear of the bars above
    /// and the composer below), top first, by id and top edge.
    private struct Look {
        let said: String
        let rows: [(id: String, y: CGFloat)]

        func field(_ key: String) -> String {
            said.split(separator: " ").first { $0.hasPrefix("\(key)=") }.map { String($0.dropFirst(key.count + 1)) } ?? ""
        }

        var gap: Int { Int(field("gap")) ?? -1 }
        var top: CGFloat { CGFloat(Int(field("top")) ?? 0) }

        /// How far the rows both looks saw moved on the screen other than by
        /// the scroll: the largest, and the row's id.
        func drift(since before: Look) -> (id: String, by: CGFloat)? {
            let was = Dictionary(before.rows.map { ($0.id, $0.y) }, uniquingKeysWith: { a, _ in a })
            return rows.compactMap { row in
                was[row.id].map { (row.id, (row.y - $0) + (top - before.top)) }
            }.max { abs($0.1) < abs($1.1) }
        }
    }

    private func look(_ app: XCUIApplication) -> Look {
        let list = element(app, "native-transcript")
        guard let tree = try? list.snapshot() else { return Look(said: "", rows: []) }
        let frame = tree.frame
        var found: [(id: String, y: CGFloat)] = []
        func walk(_ node: XCUIElementSnapshot) {
            let f = node.frame
            if node.identifier.hasPrefix("native-row-") {
                if f.height > 0, f.maxY > frame.minY + 120, f.minY < frame.maxY - 200 { found.append((node.identifier, f.minY)) }
                return
            }
            node.children.forEach(walk)
        }
        walk(tree)
        return Look(said: tree.value as? String ?? "", rows: found.sorted { $0.y < $1.y })
    }

    /// Wait for the list to come to rest after a gesture: its top still for
    /// half a second.
    private func rest(_ app: XCUIApplication) {
        var last = top(app)
        _ = wait(10) {
            Thread.sleep(forTimeInterval: 0.5)
            let now = top(app)
            defer { last = now }
            return now == last
        }
    }

    /// Flick up `points`, as a thumb does, at `velocity`.
    private func flick(_ app: XCUIApplication, by points: CGFloat, velocity: XCUIGestureVelocity) {
        let from = element(app, "native-transcript").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        from.press(forDuration: 0.03, thenDragTo: from.withOffset(CGVector(dx: 0, dy: points)), withVelocity: velocity, thenHoldForDuration: 0)
    }

    /// The owner's case (ov-675): a reader who has just sent (the keyboard
    /// up) flicks up a little at a time to read back while the agent goes on,
    /// in a session as uneven as a real one (`-native-live-real`). A folded
    /// report built just above the screen used to lay out unfolded first and
    /// shrink a frame later, by 344 pt, which moved every row on the screen
    /// up and, near the end, the end itself past the screen's foot, so the
    /// list came to rest on its tail and followed it: back to the bottom.
    /// At every rest, the reader is not at the bottom and no row on the
    /// screen has moved other than by the reader's own scrolling.
    func testFlickingUpAmongTallRowsNeverJumps() {
        let app = launch(["-native-live-real", "-native-images"])
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "the list didn't start on its tail: \(said(app))")
        send(app, "Settle me later")
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "the send didn't bring the list to its tail: \(said(app))")
        Thread.sleep(forTimeInterval: 2)
        var seen: [String] = []
        var before = look(app)
        for n in 0..<8 {
            // Near the end first, where a row shrinking above the screen
            // brought the end up past its foot; then quicker, further up.
            flick(app, by: 160, velocity: n < 3 ? .slow : .fast)
            rest(app)
            let now = look(app)
            let drift = now.drift(since: before)
            seen.append("\(n): gap=\(now.gap) rows=\(now.rows.count) drift=\(drift.map { "\(Int($0.by))@\($0.id)" } ?? "-")")
            XCTAssertGreaterThan(now.gap, 60, "the reader was taken back to the bottom: \(seen.joined(separator: "; ")) \(now.said)")
            XCTAssertFalse(now.rows.isEmpty, "the list went blank: \(seen.joined(separator: "; ")) \(now.said)")
            if let drift { XCTAssertLessThanOrEqual(abs(drift.by), 4, "a row jumped: \(seen.joined(separator: "; ")) \(now.said)") }
            before = now
        }
        print("ov-675 flicks: \(seen.joined(separator: "; "))")
    }

    /// Flung up fast through the same session, screens at a time, while rows
    /// stream, the queued message settles and older pages load as the top
    /// nears (each answered 300 ms after it's asked for): at every rest the
    /// list shows rows, isn't at the bottom, and no row on the screen moved
    /// other than by the scroll (a page going in above keeps the reader's
    /// row, ov-629); then, through ten seconds more of streaming and late
    /// photos, the reader's place holds.
    func testFlingingUpThroughALongSessionHoldsThePlace() {
        let app = launch(["-native-live-real", "-native-images"])
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "the list didn't start on its tail: \(said(app))")
        send(app, "Settle me later")
        XCTAssertTrue(element(app, "native-queued").waitForExistence(timeout: 15), "the busy runner didn't queue it")
        XCTAssertTrue(wait(10) { field(app, "tail") == "true" }, "the send didn't bring the list to its tail: \(said(app))")
        let first = Int(field(app, "rows")) ?? 0
        var seen: [String] = []
        var before = look(app)
        for n in 0..<12 where n < 4 || (Int(field(app, "rows")) ?? 0) < first + 50 {
            // A thumb's fling, which coasts on for screens. (A press-and-drag
            // at this speed loses the test runner on iOS 26.5.)
            element(app, "native-transcript").swipeDown(velocity: XCUIGestureVelocity(3000))
            rest(app)
            let now = look(app)
            let drift = now.drift(since: before)
            seen.append("\(n): gap=\(now.gap) rows=\(now.rows.count)/\(now.field("rows")) drift=\(drift.map { "\(Int($0.by))@\($0.id)" } ?? "-")")
            XCTAssertGreaterThan(now.gap, 60, "the reader was taken back to the bottom: \(seen.joined(separator: "; ")) \(now.said)")
            XCTAssertFalse(now.rows.isEmpty, "the list went blank: \(seen.joined(separator: "; ")) \(now.said)")
            if let drift { XCTAssertLessThanOrEqual(abs(drift.by), 4, "a row jumped: \(seen.joined(separator: "; ")) \(now.said)") }
            before = now
        }
        XCTAssertGreaterThanOrEqual(Int(field(app, "rows")) ?? 0, first + 50, "older pages didn't come in, so this proved nothing: \(said(app))")
        for n in 0..<20 {
            Thread.sleep(forTimeInterval: 0.5)
            let now = look(app)
            let drift = now.drift(since: before)
            seen.append("after \(n): gap=\(now.gap) rows=\(now.rows.count) drift=\(drift.map { "\(Int($0.by))@\($0.id)" } ?? "-")")
            XCTAssertGreaterThan(now.gap, 60, "the reader was taken back to the bottom: \(seen.joined(separator: "; ")) \(now.said)")
            XCTAssertFalse(now.rows.isEmpty, "the list went blank: \(seen.joined(separator: "; ")) \(now.said)")
            if let drift { XCTAssertLessThanOrEqual(abs(drift.by), 4, "the reader's place moved: \(seen.joined(separator: "; ")) \(now.said)") }
            before = now
        }
        XCTAssertFalse(element(app, "native-queued").exists, "the queued message never settled, so this proved nothing")
        print("ov-675 flings: \(seen.joined(separator: "; "))")
    }
}
