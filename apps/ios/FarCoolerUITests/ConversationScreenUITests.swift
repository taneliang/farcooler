import XCTest

/// Select and copy in the iPhone's conversation (ov-609), the person's
/// message as the site's bubble (ov-610), and the screen's one margin
/// (ov-611), over the in-app harnesses: no runner.
final class ConversationScreenUITests: XCTestCase {
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

    /// The screen, to `FARCOOLER_CAPTURE_OUT/<tag>-<name>.png` when that's set:
    /// the selection's handles and menu, for a review.
    private func shoot(_ name: String) {
        let env = ProcessInfo.processInfo.environment
        guard let out = env["FARCOOLER_CAPTURE_OUT"] else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("\(env["FARCOOLER_CAPTURE_TAG"] ?? "capture")-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func launchConversation() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"]
        app.launchDrawn()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "the conversation never showed")
        return app
    }

    /// The system edit menu's Copy, as a menu item or (older systems) a button.
    private func copyItem(_ app: XCUIApplication) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(10)
        repeat {
            if app.menuItems["Copy"].exists { return app.menuItems["Copy"] }
            if app.buttons["Copy"].exists { return app.buttons["Copy"] }
            Thread.sleep(forTimeInterval: 0.25)
        } while Date() < deadline
        return nil
    }

    /// A long press on the first word of `text`, then Copy: the pasteboard
    /// holds that word and nothing else. With the whole-text Copy that
    /// `.textSelection(.enabled)` gives on iOS, it holds the whole message.
    private func selectFirstWordAndCopy(_ app: XCUIApplication, in text: XCUIElement, expect word: String) {
        // Inside the first word: a few points in from the text's corner.
        text.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 8, dy: 10)).press(forDuration: 1.0)
        guard let copy = copyItem(app) else {
            return XCTFail("a long press offered no Copy: \(app.debugDescription)")
        }
        shoot("selection-\(word)")
        copy.tap()
        // Read by the app: the runner reading it would be a paste from
        // another app, which the system stops to ask about.
        let copied = element(app, "harness-pasteboard")
        XCTAssertTrue(
            wait(10) { copied.value as? String == word },
            "Copy put \"\(copied.value as? String ?? "nil")\" on the pasteboard, not the word \"\(word)\"")
    }

    /// The person's message: a long press selects its first word, and Copy
    /// copies exactly that.
    func testALongPressSelectsAWordInYourMessage() {
        let app = launchConversation()
        let prompt = app.descendants(matching: .any).matching(identifier: "native-prompt")
            .matching(NSPredicate(format: "label BEGINSWITH 'Tidy the parser'")).firstMatch
        XCTAssertTrue(prompt.waitForExistence(timeout: 30), "no message of the person's: \(app.debugDescription)")
        selectFirstWordAndCopy(app, in: prompt, expect: "Tidy")
    }

    /// The agent's reply: the same.
    func testALongPressSelectsAWordInTheReply() {
        let app = launchConversation()
        let reply = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH 'Reading the parser'")).firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 30), "no reply: \(app.debugDescription)")
        selectFirstWordAndCopy(app, in: reply, expect: "Reading")
    }

    /// The person's message sits at the trailing edge, as wide as its words
    /// up to three quarters of the column (ov-610).
    func testYourMessageIsATrailingBubble() {
        let app = launchConversation()
        let prompt = element(app, "native-prompt")
        XCTAssertTrue(prompt.waitForExistence(timeout: 30))
        let row = app.descendants(matching: .any).matching(identifier: "native-transcript").firstMatch
        let column = app.windows.firstMatch.frame
        XCTAssertLessThanOrEqual(prompt.frame.width, column.width * 0.75, "the bubble is wider than three quarters: \(prompt.frame)")
        XCTAssertGreaterThan(prompt.frame.minX, column.width * 0.2, "the bubble isn't at the trailing edge: \(prompt.frame) \(row.frame)")
    }

    /// Every block on the owner's screen (the segment bar, the plan's strip,
    /// the rows, the tray and the composer) stands on one horizontal margin
    /// (ov-611). The composer stood 4 pt wider than the rest.
    func testEveryBlockSharesOneMargin() {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
        main.tap()
        let composer = element(app, "native-composer-surface")
        XCTAssertTrue(composer.waitForExistence(timeout: 60), "no composer: \(app.debugDescription)")
        let width = app.windows.firstMatch.frame.width
        let segments = element(app, "workspace-segments")
        XCTAssertTrue(segments.waitForExistence(timeout: 10))
        // The segment bar is the reference: the system's margin.
        let margin = segments.frame.minX
        XCTAssertTrue(margin == 16 || margin == 20, "the segment bar's margin is \(margin), not the system's")
        let blocks = ["workspace-segments", "plan-strip", "native-agent-tray-surface", "native-composer-surface"]
        XCTAssertTrue(element(app, "plan-strip").waitForExistence(timeout: 30), "no plan strip")
        XCTAssertTrue(element(app, "native-agent-tray-surface").waitForExistence(timeout: 30), "no tray")
        for id in blocks {
            let frame = element(app, id).frame
            XCTAssertEqual(frame.minX, margin, accuracy: 0.5, "\(id) starts at \(frame.minX), not \(margin)")
            XCTAssertEqual(width - frame.maxX, margin, accuracy: 0.5, "\(id) ends \(width - frame.maxX) from the edge, not \(margin)")
        }
        // A reply's words start on the same line.
        let reply = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'native-row-prose'")).firstMatch
        XCTAssertTrue(reply.exists, "no reply on screen to measure")
        XCTAssertEqual(reply.frame.minX, margin, accuracy: 0.5, "a reply starts at \(reply.frame.minX)")
    }

    /// A folded report's last line shows, fading, above its Show More
    /// (review 1): drawn as a range-selecting text view under the fold's mask
    /// and clip, it went blank. Read off the screen's pixels in the band just
    /// above the button: some of them must differ from the page.
    func testAFoldedReportsLastLineShows() throws {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        // Light, where the fold went blank (dark drew it at full ink).
        XCUIDevice.shared.appearance = .light
        let app = XCUIApplication()
        app.launchArguments = [
            "-native-agent-harness", "-native-harness-file", url.appendingPathComponent("test/fixtures/agent-rows-harness.json").path,
            "-app.theme", "Solarized Light",
        ]
        app.launchDrawn()
        let report = element(app, "native-row-turn:p2")
        XCTAssertTrue(report.waitForExistence(timeout: 30), "no report row")
        let more = report.buttons["Show More"]
        XCTAssertTrue(more.waitForExistence(timeout: 10), "the report doesn't fold")
        Thread.sleep(forTimeInterval: 1)
        let shot = XCUIScreen.main.screenshot().image
        // From the words' own leading edge (Show More's), clear of the
        // report's rule at its leading edge.
        let band = CGRect(x: more.frame.minX, y: more.frame.minY - 30, width: report.frame.maxX - more.frame.minX - 8, height: 24)
        let inked = Self.inkedPixels(in: shot, band: band, points: app.windows.firstMatch.frame.width)
        if let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] {
            try? shot.pngData()?.write(to: URL(fileURLWithPath: out).appendingPathComponent("ov-609-after-folded-report.png"))
        }
        XCTAssertGreaterThan(inked, 40, "nothing is drawn in the fold's last line above Show More (\(band))")
    }

    /// Pixels in `band` (points, on a screen `width` points wide) that differ
    /// from the band's own commonest shade by more than `tolerance` of 255.
    private static func inkedPixels(in image: UIImage, band: CGRect, points width: CGFloat, tolerance: Int = 30) -> Int {
        guard let cg = image.cgImage, width > 0 else { return 0 }
        // Pixels per point, from the image itself: a screenshot's `scale`
        // says 1 whatever the screen's is.
        let scale = CGFloat(cg.width) / width
        let rect = CGRect(x: band.minX * scale, y: band.minY * scale, width: band.width * scale, height: band.height * scale).integral
        guard let crop = cg.cropping(to: rect) else { return 0 }
        let width = crop.width, height = crop.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard let context = CGContext(
            data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return 0 }
        context.draw(crop, in: CGRect(x: 0, y: 0, width: width, height: height))
        var counts: [Int: Int] = [:]
        var shades: [Int] = []
        for i in stride(from: 0, to: pixels.count, by: 4) {
            let shade = (Int(pixels[i]) + Int(pixels[i + 1]) + Int(pixels[i + 2])) / 3
            shades.append(shade)
            counts[shade, default: 0] += 1
        }
        let page = counts.max { $0.value < $1.value }?.key ?? 0
        return shades.filter { abs($0 - page) > tolerance }.count
    }

    /// Nothing shows behind the bars over the orchestrator (ov-611, review
    /// 1): rows scrolled up under the plan strip must not reappear behind the
    /// segment bar or the navigation bar. Read off the screen's pixels beside
    /// the title: all the page's ground.
    func testNothingShowsBehindTheBars() {
        // Light, where text through the bars' blur reads plainest.
        XCUIDevice.shared.appearance = .light
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents",
             "-app.theme", "Solarized Light"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
        main.tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "no conversation")
        let segments = element(app, "workspace-segments"), strip = element(app, "plan-strip")
        XCTAssertTrue(strip.waitForExistence(timeout: 30), "no plan strip")
        Thread.sleep(forTimeInterval: 2)
        // Rows stand behind the bars: the page is longer than the screen and
        // sits on its tail, so the list's visible top is well down it.
        let said = element(app, "native-transcript").value as? String ?? ""
        let top = said.split(separator: " ").first { $0.hasPrefix("top=") }.flatMap { Int($0.dropFirst(4)) } ?? 0
        XCTAssertGreaterThan(top, 200, "nothing above the strip to hide: \(said)")
        let shot = XCUIScreen.main.screenshot().image
        // Between the navigation bar's buttons and the segment bar, across
        // the screen: rows there showed through the bars' blur.
        let width = app.windows.firstMatch.frame.width
        let behind = CGRect(x: 16, y: segments.frame.minY - 14, width: width - 32, height: 10)
        let inked = Self.inkedPixels(in: shot, band: behind, points: app.windows.firstMatch.frame.width, tolerance: 10)
        if let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] {
            try? shot.pngData()?.write(to: URL(fileURLWithPath: out).appendingPathComponent("ov-611-after-bars-behind.png"))
        }
        XCTAssertLessThan(inked, 40, "rows show behind the navigation bar (\(behind)): \(inked) pixels")
    }

    // MARK: - The composer (ov-614)

    /// Main's Orchestrator tab with four agents in the tray: the owner's
    /// screen, where the box was squeezed.
    private func openOrchestrator() -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
        main.tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "no composer")
        return app
    }

    /// Everything in the box is inside its surface, and its bottom row is
    /// above the keyboard where one is up.
    private func assertContained(_ app: XCUIApplication, _ when: String) {
        let surface = element(app, "native-composer-surface").frame
        for id in ["native-composer", "native-send", "native-attach"] {
            let frame = element(app, id).frame
            XCTAssertTrue(
                surface.insetBy(dx: -0.5, dy: -0.5).contains(frame),
                "\(when): \(id) at \(frame) is outside the composer's surface \(surface)")
        }
        let keyboard = app.keyboards.firstMatch
        if keyboard.exists, keyboard.frame.height > 0 {
            XCTAssertLessThanOrEqual(
                element(app, "native-send").frame.maxY, keyboard.frame.minY + 0.5, "\(when): Send is under the keyboard")
        }
    }

    /// One line to twenty: the box grows with its text to its cap, then holds
    /// that height and scrolls inside, with nothing drawn outside it.
    func testTheComposerGrowsToItsCapThenScrolls() {
        let app = openOrchestrator()
        let field = element(app, "native-composer")
        field.tap()
        XCTAssertTrue(wait(30) { field.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the box never took the keyboard")
        var heights: [CGFloat] = []
        for line in 1...20 {
            app.typeText(line == 1 ? "Line 1" : "\nLine \(line)")
            Thread.sleep(forTimeInterval: 0.4)
            heights.append(element(app, "native-composer-surface").frame.height)
            assertContained(app, "at \(line) lines")
        }
        // Three more lines of body text are at least 40 pt more.
        XCTAssertGreaterThan(heights[3], heights[0] + 40, "four lines are hardly taller than one: \(heights)")
        for i in 1..<heights.count {
            XCTAssertGreaterThanOrEqual(heights[i], heights[i - 1] - 0.5, "the box shrank as a line was added: \(heights)")
        }
        XCTAssertEqual(heights[19], heights[15], accuracy: 0.5, "the box kept growing past its cap: \(heights)")
        XCTAssertTrue((field.value as? String ?? "").hasSuffix("Line 20"), "the box lost what was typed")
    }

    /// A photo sits inside the box, over the text row, and attach and Send
    /// stay on the bottom row.
    func testAPhotoSitsInsideTheComposer() {
        let app = openOrchestrator()
        let field = element(app, "native-composer")
        field.tap()
        app.typeText("What is this")
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.native-photo" as CFString), nil, nil, true)
        let chip = element(app, "native-image-chip")
        XCTAssertTrue(chip.waitForExistence(timeout: 30), "no chip for the photo")
        Thread.sleep(forTimeInterval: 1)
        let surface = element(app, "native-composer-surface").frame
        XCTAssertTrue(surface.contains(chip.frame), "the photo at \(chip.frame) is outside the composer \(surface)")
        XCTAssertLessThan(chip.frame.maxY, field.frame.minY + 0.5, "the photo isn't above the text")
        assertContained(app, "with a photo")
    }

    // MARK: - The terminal

    private static let shell = "0198f2c0-0000-7000-8000-00000000d003"

    /// The shell pane over the canned screen `bil-9 is done; build finished
    /// (v1.2, bil-1.2)` then `$ `, and where its cells are drawn.
    private func openTerminal(_ extra: [String] = []) throws -> (XCUIApplication, XCUIElement, CGFloat, CGFloat, CGPoint) {
        let app = XCUIApplication.phoneHarness(["-phone-terminal-key", "-deep-link", Self.shell] + extra)
        let deadline = Date().addingTimeInterval(60)
        var surface: XCUIElement?
        repeat {
            let all = app.otherElements.matching(identifier: "terminal-surface")
            for i in 0..<all.count where (all.element(boundBy: i).value as? String ?? "").contains("visible=1") {
                surface = all.element(boundBy: i)
            }
            if surface == nil { Thread.sleep(forTimeInterval: 0.5) }
        } while surface == nil && Date() < deadline
        guard let surface else { throw XCTSkip("the shell pane never drew: \(app.debugDescription)") }
        let value = surface.value as? String ?? ""
        func field(_ key: String) -> String? {
            value.split(separator: " ").first { $0.hasPrefix("\(key)=") }.map { String($0.dropFirst(key.count + 1)) }
        }
        let height = field("cell").flatMap(Double.init) ?? 17
        let width = field("cellw").flatMap(Double.init) ?? 9
        let origin = field("grid").map { $0.split(separator: ",").compactMap { Double($0) } } ?? [6, 6]
        return (app, surface, CGFloat(width), CGFloat(height), CGPoint(x: origin[0], y: origin[1]))
    }

    /// The middle of a cell.
    private func cell(_ surface: XCUIElement, _ column: Int, _ row: Int, _ w: CGFloat, _ h: CGFloat, _ o: CGPoint) -> XCUICoordinate {
        surface.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: o.x + (CGFloat(column) + 0.5) * w, dy: o.y + (CGFloat(row) + 0.5) * h))
    }

    /// A long press on a word in the terminal selects it, and Copy copies it.
    /// Before, a press there offered Paste alone.
    func testALongPressSelectsAWordInTheTerminal() throws {
        let (app, surface, w, h, o) = try openTerminal()
        // "build", columns 15 to 19 of the first row.
        cell(surface, 17, 0, w, h, o).press(forDuration: 1.0)
        guard let copy = copyItem(app) else { return XCTFail("a long press on text offered no Copy: \(app.debugDescription)") }
        copy.tap()
        let copied = element(app, "harness-pasteboard")
        XCTAssertTrue(wait(10) { copied.value as? String == "build" }, "Copy gave \"\(copied.value as? String ?? "nil")\"")
    }

    /// Dragged on from the press, the selection runs across rows, and Copy
    /// gives the text with its line break as shown.
    func testADragSelectsAcrossRowsWithTheirLineBreak() throws {
        let (app, surface, w, h, o) = try openTerminal()
        cell(surface, 17, 0, w, h, o).press(forDuration: 1.0, thenDragTo: cell(surface, 0, 1, w, h, o))
        guard let copy = copyItem(app) else { return XCTFail("no Copy after a drag: \(app.debugDescription)") }
        shoot("terminal-selection")
        copy.tap()
        let copied = element(app, "harness-pasteboard")
        XCTAssertTrue(
            wait(10) { copied.value as? String == "build finished (v1.2, bil-1.2)\n$" },
            "Copy gave \"\(copied.value as? String ?? "nil")\"")
    }

    /// A press on a task key still offers the task, and one on empty rows
    /// selects nothing (ov-215, ov-604).
    func testAKeyStillOffersItsTaskAndBlankSelectsNothing() throws {
        let (app, surface, w, h, o) = try openTerminal()
        cell(surface, 20, 6, w, h, o).press(forDuration: 1.0)
        Thread.sleep(forTimeInterval: 1)
        XCTAssertFalse(app.menuItems["Copy"].exists || app.buttons["Copy"].exists, "a press on empty rows selected something")
        app.tap()
        cell(surface, 1, 0, w, h, o).press(forDuration: 1.0)
        XCTAssertTrue(app.buttons["Open Task"].waitForExistence(timeout: 10), "a press on bil-9 no longer offers its task")
        XCTAssertFalse(app.menuItems["Copy"].exists, "a press on a key selected it")
    }

    /// A selection held at the top edge of a full-screen program's screen
    /// sends the program nothing (review 1): the edge scroll moved the
    /// emulator through `TerminalSession.scroll`, which on the alternate
    /// screen writes mouse-wheel bytes, ten a second.
    func testASelectionAtTheEdgeOfAProgramsScreenSendsItNothing() throws {
        let (app, surface, w, h, o) = try openTerminal(["-phone-terminal-alt"])
        XCTAssertTrue((surface.value as? String ?? "").contains("mouse=on"), "the program didn't take the mouse: \(surface.value ?? "")")
        // A press on "build" (columns 15 to 19 of the first row), dragged to
        // the top edge and held there, where a selection scrolls.
        cell(surface, 17, 0, w, h, o).press(
            forDuration: 1.0, thenDragTo: surface.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: o.x + 3 * w, dy: 1)),
            withVelocity: .slow, thenHoldForDuration: 2)
        XCTAssertNotNil(copyItem(app), "the press selected nothing: \(app.debugDescription)")
        Thread.sleep(forTimeInterval: 1)
        let sent = element(app, "harness-sent").value as? String ?? ""
        XCTAssertFalse(sent.contains("write "), "the program was sent bytes during a selection: \(sent)")
    }

    /// `test/fixtures/plan-seeded.json`, found from this file.
    private var planFixture: String {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }
}
