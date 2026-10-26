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
    private func shoot(_ name: String, expecting state: CaptureState) {
        let env = ProcessInfo.processInfo.environment
        guard let out = env["FARCOOLER_CAPTURE_OUT"] else { return }
        guard requireState(state, for: name) else { return }
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
        shoot("selection-\(word)", expecting: .shows(copy, "the Copy menu over the selection"))
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

    /// The blue bubble's extent along the screen row through each message's
    /// words, in points, as pixels say it: the element a message's
    /// identifier lands on is its text, not the bubble around it.
    private func bubbleExtents(_ app: XCUIApplication, _ messages: [XCUIElement]) -> [(left: CGFloat, right: CGFloat)] {
        guard let image = XCUIScreen.main.screenshot().image.cgImage else { return [] }
        let scale = CGFloat(image.width) / app.windows.firstMatch.frame.width
        var pixels = [UInt8](repeating: 0, count: image.width * image.height * 4)
        let drew = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: image.width, height: image.height, bitsPerComponent: 8,
                bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            return true
        }
        guard drew else { return [] }
        return messages.map { message in
            let row = min(max(Int(message.frame.midY * scale), 0), image.height - 1)
            var left = Int.max, right = -1
            for x in 0..<image.width {
                let at = (row * image.width + x) * 4
                let (r, g, b) = (Int(pixels[at]), Int(pixels[at + 1]), Int(pixels[at + 2]))
                if b > r + 60 && b > g + 30 { left = min(left, x); right = max(right, x) }
            }
            return (CGFloat(left) / scale, CGFloat(right + 1) / scale)
        }
    }

    private func bubbleWidths(_ app: XCUIApplication, _ messages: [XCUIElement]) -> [CGFloat] {
        bubbleExtents(app, messages).map { $0.right - $0.left }
    }

    private func bubbleEdges(_ app: XCUIApplication, _ messages: [XCUIElement]) -> [CGFloat] {
        bubbleExtents(app, messages).map { $0.right }
    }

    /// The person's messages of four sizes, each bubble as wide as its words
    /// (ov-809): a word, a line and a few lines are narrower than a long
    /// message's cap, and the longer the words, the wider the bubble. The
    /// bubble was as wide as its cap whatever it said. `-native-bubbles`.
    func testYourBubbleHugsItsWords() {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-bubbles"]
        app.launchDrawn()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "the conversation never showed")
        let starts = ["Thanks.", "Is the web interface", "Two things:", "Fold this where"]
        let bubbles = starts.map {
            app.descendants(matching: .any).matching(identifier: "native-prompt")
                .matching(NSPredicate(format: "label BEGINSWITH %@", $0)).firstMatch
        }
        for (bubble, start) in zip(bubbles, starts) {
            XCTAssertTrue(bubble.waitForExistence(timeout: 30), "no bubble for \"\(start)\": \(app.debugDescription)")
        }
        var widths = bubbleWidths(app, bubbles)
        XCTAssertEqual(widths.count, 4, "the screen couldn't be read")
        let firstWidths = widths
        let column = app.windows.firstMatch.frame.width
        shoot("bubbles", expecting: .shows(bubbles[3], "the four bubbles, the long one last"))
        // The cap is three quarters of the column; the long message fills it.
        XCTAssertLessThanOrEqual(widths[3], column * 0.75, "the long bubble is past its cap: \(widths)")
        XCTAssertGreaterThan(widths[3], column * 0.6, "the long bubble doesn't fill its cap: \(widths)")
        // A word is far from it and a few short lines are short of it. A
        // line is as wide as its words, which on the narrowest phones are
        // the cap's: it must not be wider than the few lines' bubble.
        XCTAssertLessThan(widths[0], column * 0.3, "a word's bubble is wide: \(widths)")
        XCTAssertLessThanOrEqual(widths[1], widths[3], "a line's bubble is wider than the cap's: \(widths)")
        XCTAssertLessThan(widths[2], widths[3] - 20, "a few lines' bubble is as wide as the cap: \(widths)")
        XCTAssertLessThan(widths[0], widths[1], "a word's bubble is no narrower than a line's: \(widths)")
        // Every bubble at the trailing edge, whatever its width.
        let edges = bubbleEdges(app, bubbles)
        XCTAssertLessThan((edges.max() ?? 0) - (edges.min() ?? 0), 2, "the bubbles don't share a trailing edge: \(edges)")

        // Scrolled up through thirty long paragraphs and back, the views the
        // paragraphs used (the lazy list gives them back, and the bubbles take
        // them) draw the same widths: a view that held a long text is not
        // the width of it for a short one.
        let list = element(app, "native-transcript")
        let top = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        for _ in 0..<6 { top.press(forDuration: 0.05, thenDragTo: top.withOffset(CGVector(dx: 0, dy: 500)), withVelocity: .fast, thenHoldForDuration: 0.1) }
        let bottom = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.7))
        for _ in 0..<12 where !bubbles[0].isHittable {
            bottom.press(forDuration: 0.05, thenDragTo: bottom.withOffset(CGVector(dx: 0, dy: -500)), withVelocity: .fast, thenHoldForDuration: 0.1)
        }
        XCTAssertTrue(wait(10) { bubbles.allSatisfy { $0.exists } }, "the bubbles never came back: \(app.debugDescription)")
        Thread.sleep(forTimeInterval: 1)
        shoot("bubbles-after-scroll", expecting: .shows(bubbles[3], "the four bubbles again"))
        widths = bubbleWidths(app, bubbles)
        // Within a point: an edge's pixel rounds with where the scroll stopped.
        for (n, width) in widths.enumerated() {
            XCTAssertEqual(width, firstWidths[n], accuracy: 1.5, "scrolling away and back changed bubble \(n)'s width: \(firstWidths) then \(widths)")
        }
    }

    /// Every block on the owner's screen (the Now/Next and Agents cards, the
    /// rows and the composer) stands on one horizontal margin (ov-611,
    /// ov-619). The composer stood 4 pt wider than the rest.
    func testEveryBlockSharesOneMargin() {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
        main.tap()
        let composer = element(app, "native-composer-surface")
        XCTAssertTrue(composer.waitForExistence(timeout: 60), "no composer: \(app.debugDescription)")
        let width = app.windows.firstMatch.frame.width
        let strip = element(app, "plan-strip"), agents = element(app, "agents-card")
        XCTAssertTrue(strip.waitForExistence(timeout: 30), "no plan strip")
        XCTAssertTrue(agents.waitForExistence(timeout: 30), "no Agents card")
        // The Now/Next card is the reference: the system's margin.
        let margin = strip.frame.minX
        XCTAssertTrue(margin == 16 || margin == 20, "the cards' margin is \(margin), not the system's")
        XCTAssertEqual(width - agents.frame.maxX, margin, accuracy: 0.5, "the Agents card ends \(width - agents.frame.maxX) from the edge")
        // The photo button leads the composer, outside its box (ov-677).
        let attach = element(app, "native-attach")
        let box = composer.frame, start = attach.exists ? attach.frame.minX : box.minX
        XCTAssertEqual(start, margin, accuracy: 0.5, "the composer starts at \(start), not \(margin)")
        XCTAssertEqual(width - box.maxX, margin, accuracy: 0.5, "the composer ends \(width - box.maxX) from the edge")
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
        if let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"],
            requireState(.shows(more, "Show More on the folded report"), for: "ov-609-after-folded-report")
        {
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

    /// Rows scroll on under the bars over the orchestrator, Liquid Glass's
    /// way (ov-678): the system's scroll edge effect softens them behind the
    /// navigation bar and the cards, where ov-611 stood a band of the page's
    /// ground with a fade under it. Read off the screen's pixels between the
    /// bar's buttons, under the title, twice: over a session long enough to
    /// run under the bars, and over one too short to, so the bars' own glass
    /// and tint are in both and only rows make the difference.
    func testRowsShowThroughTheBars() {
        // Light, where text through the bars' blur reads plainest.
        XCUIDevice.shared.appearance = .light
        defer { XCUIDevice.shared.appearance = .light }
        let long = barsBand(rows: true)
        let short = barsBand(rows: false)
        XCTAssertGreaterThan(
            long.inked - short.inked, 40,
            "rows behind the bars inked \(long.inked) pixels of \(long.band), and no rows \(short.inked)")
    }

    /// The orchestrator's screen, with rows behind its bars or none, and how
    /// many pixels in the band
    /// under the title differ from its commonest shade.
    private func barsBand(rows: Bool) -> (inked: Int, band: CGRect) {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-app.theme", "Solarized Light"]
                // The agents' rows and the polish rows: enough to run under the bars on the smallest phone.
                + (rows ? ["-native-agents", "-native-polish"] : []))
        defer { app.terminate() }
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
        main.tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "no conversation")
        let strip = element(app, "plan-strip")
        XCTAssertTrue(strip.waitForExistence(timeout: 30), "no plan strip")
        Thread.sleep(forTimeInterval: 2)
        // The list's visible top: well down the page when rows run under the
        // bars, and at most its start when the session is too short to.
        let said = element(app, "native-transcript").value as? String ?? ""
        let top = said.split(separator: " ").first { $0.hasPrefix("top=") }.flatMap { Int($0.dropFirst(4)) } ?? 0
        if rows {
            XCTAssertGreaterThan(top, 200, "nothing above the strip to show: \(said)")
        } else {
            XCTAssertLessThanOrEqual(top, 0, "the short session runs under the bars: \(said)")
        }
        let shot = XCUIScreen.main.screenshot()
        if rows, let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] {
            try? shot.pngRepresentation.write(to: URL(fileURLWithPath: out).appendingPathComponent("ov-678-rows-under-bars.png"))
        }
        // Between the navigation bar's buttons, clear of their glass, and
        // just above the cards: under the title, where the band stood.
        let width = app.windows.firstMatch.frame.width
        let band = CGRect(x: 72, y: strip.frame.minY - 9, width: width - 144, height: 6)
        var inked = Self.inkedPixels(in: shot.image, band: band, points: width, tolerance: 10)
        guard rows else { return (inked, band) }
        // The band is six points tall, so what's behind it at the tail can be
        // the gap between two lines (27 and 22 pixels inked after ov-685
        // changed the rows, Oct 22). Scrolled back a step at a time, a line
        // of a row passes under it; a band of the page's ground (ov-611)
        // stays blank at every step. A step of 23 points: shorter drags read
        // as a tap on a row and open it, and 23 isn't a line's height, so
        // successive steps land at different places in a line.
        let list = element(app, "native-transcript").firstMatch
        for _ in 0..<8 where inked <= 40 {
            let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: 23)), withVelocity: .slow, thenHoldForDuration: 0.3)
            inked = max(inked, Self.inkedPixels(in: XCUIScreen.main.screenshot().image, band: band, points: width, tolerance: 10))
        }
        return (inked, band)
    }

    // MARK: - The audit (ov-683)

    /// Apple's accessibility audit over the whole conversation screen, every
    /// kind of finding, judged nowhere less than everywhere: a new finding
    /// anywhere on it fails this. The harness's conversation holds a working
    /// turn, so the transcript's spinner is in it. The two DEBUG probes are
    /// elements the tests read, not controls a person meets.
    func testTheWholeConversationPassesTheAudit() throws {
        try assertAuditClean(launchConversation())
    }

    /// The same inside the workspace's own bars, the title and the plan
    /// strip above the rows: the screen as a person reaches it.
    func testTheConversationInTheWorkspacePassesTheAudit() throws {
        let app = openOrchestrator()
        defer { app.terminate() }
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 30))
        try assertAuditClean(app)
    }

    /// A light terminal theme: the quiet ink over its cream ground measured
    /// 3.4:1 as the system's secondary, and the audit said so (ov-685).
    func testTheConversationInALightThemePassesTheAudit() throws {
        let app = openOrchestrator(extra: ["-app.theme", "Solarized Light"])
        defer { app.terminate() }
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 30))
        try assertAuditClean(app, shot: "light")
    }

    /// The bare conversation at the largest accessibility size, where rows
    /// and bars were cut to a letter's width or a few letters (ov-685).
    func testTheBareConversationAtTheLargestSizePassesTheAudit() throws {
        for dark in [true, false] {
            XCUIDevice.shared.appearance = dark ? .dark : .light
            defer { XCUIDevice.shared.appearance = .light }
            let app = XCUIApplication()
            app.launchArguments = ["-native-agent-harness"] + Self.largestType + (dark ? [] : ["-app.theme", "Solarized Light"])
            app.launchDrawn()
            XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60))
            try assertAuditClean(app, shot: "bare-xxxl-\(dark ? "dark" : "light")", large: true)
            app.terminate()
        }
    }

    /// The workspace's conversation at the largest accessibility size, in
    /// both appearances, over the long rows of the polish harness.
    func testTheWorkspaceConversationAtTheLargestSizePassesTheAudit() throws {
        for dark in [true, false] {
            XCUIDevice.shared.appearance = dark ? .dark : .light
            defer { XCUIDevice.shared.appearance = .light }
            let app = openOrchestrator(extra: Self.largestType + ["-native-polish"] + (dark ? [] : ["-app.theme", "Solarized Light"]))
            XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 30))
            try assertAuditClean(app, shot: "workspace-xxxl-\(dark ? "dark" : "light")", large: true)
            app.terminate()
        }
    }

    /// The workspace at the default size, to FARCOOLER_CAPTURE_OUT, for a
    /// before-and-after of layout that must not move (ov-685).
    func testCaptureTheWorkspaceAtTheDefaultSize() throws {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { throw XCTSkip("no capture directory") }
        for dark in [true, false] {
            XCUIDevice.shared.appearance = dark ? .dark : .light
            defer { XCUIDevice.shared.appearance = .light }
            let app = openOrchestrator(extra: dark ? [] : ["-app.theme", "Solarized Light"])
            XCTAssertTrue(element(app, "plan-strip").waitForExistence(timeout: 30))
            Thread.sleep(forTimeInterval: 2)
            try XCUIScreen.main.screenshot().pngRepresentation.write(
                to: URL(fileURLWithPath: out).appendingPathComponent("default-\(dark ? "dark" : "light").png"))
            app.terminate()
        }
    }

    private static let largestType = ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]

    /// VoiceOver reads a screen top to bottom by where its elements sit, and
    /// nothing here sets a sort priority: so the bars come first, the rows
    /// next, and the composer last. Read off the frames of the text between.
    func testVoiceOverReadsBarsThenRowsThenComposer() throws {
        let app = openOrchestrator()
        defer { app.terminate() }
        let transcript = element(app, "native-transcript")
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        let bar = app.navigationBars.firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "no bars")
        let composer = element(app, "native-composer-stack").frame
        // Rows are the transcript's own text on the screen, clear of the bar.
        let screen = app.windows.firstMatch.frame
        let rows = transcript.descendants(matching: .staticText).allElementsBoundByIndex
            .map(\.frame).filter { $0.minY >= bar.frame.maxY && $0.maxY <= screen.maxY && !$0.isEmpty }
        XCTAssertFalse(rows.isEmpty, "no rows below the bars")
        XCTAssertLessThanOrEqual(bar.frame.maxY, rows.map(\.minY).min() ?? 0, "the bars are not read before the rows")
        XCTAssertLessThan(rows.map(\.minY).max() ?? .infinity, composer.minY, "the composer is not read after the rows")
    }

    /// A running turn's time is read with its state, "Working, 1:08", as one
    /// element beside a spinner VoiceOver skips (ov-683).
    func testARunningTurnReadsAsWorking() {
        let app = launchConversation()
        let time = element(app, "native-turn-running")
        XCTAssertTrue(time.waitForExistence(timeout: 30), "no running turn status")
        XCTAssertTrue(time.label.hasPrefix("Working"), "read as \"\(time.label)\"")
    }

    /// Every kind of finding, judged everywhere on the screen. What it lets
    /// pass is named element by element, with its reason: a new finding
    /// anywhere else fails this.
    private func assertAuditClean(_ app: XCUIApplication, shot: String? = nil, large: Bool = false) throws {
        Thread.sleep(forTimeInterval: 2)
        if let shot, let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] {
            try? XCUIScreen.main.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: out).appendingPathComponent("ov-685-\(shot).png"))
        }
        // Glass: the navigation bar and the two cards. Text drawn under one,
        // wholly inside its frame, is read by the audit through the blur,
        // which it cannot judge. A row that only overlaps a card's edge is
        // judged. The composer is not glass here: its placeholder, field and
        // buttons are all judged.
        let bar = app.navigationBars.firstMatch
        let cards = [element(app, "plan-strip"), element(app, "agents-card")].filter(\.exists).map(\.frame)
        // The cards stack with a gap between them: a row under both is under
        // glass as one.
        let glass = [bar.frame] + cards + (cards.isEmpty ? [] : [cards.reduce(cards[0]) { $0.union($1) }])
        // The bare harness's header is no navigation bar, and rows scroll up
        // under it: the screen above the rows' first line.
        let header = CGRect(x: 0, y: -10_000, width: app.windows.firstMatch.frame.width, height: 10_000 + max(bar.frame.maxY, 110))
        // The bar's own titles, which it draws and caps at its own sizes.
        let barTitles = Set(bar.staticTexts.allElementsBoundByIndex.map(\.label))
        // A row (a button or a group of the transcript's, which the texts
        // sit in) that starts above a glass piece's lower edge, or ends below
        // the composer's top, is scrolling under it: the audit flags its
        // texts, the ones in the clear too, for the part it cannot see.
        let rows = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier BEGINSWITH 'native-subagent-open' OR identifier BEGINSWITH 'native-row' OR identifier BEGINSWITH 'native-tool'"))
            .allElementsBoundByIndex.map(\.frame)
        let hasCards = element(app, "plan-strip").exists
        // The composer stack's top edge: a row that ends below it is
        // scrolling up under the composer, as one that starts above the
        // bar's lower edge scrolls under the bar.
        let composerTop = element(app, "native-composer-stack").exists ? element(app, "native-composer-stack").frame.minY : CGFloat.infinity
        func inRow(_ frame: CGRect) -> Bool { rows.contains { $0.insetBy(dx: -1, dy: -1).contains(frame) } }
        func underGlassOrComposer(_ frame: CGRect) -> Bool {
            rows.contains { row in
                row.insetBy(dx: -1, dy: -1).contains(frame) && (row.maxY > composerTop
                    || (glass + [header]).contains { $0.intersects(row) && row.minY < $0.maxY })
            }
        }
        var seen: [String] = []
        try app.performAccessibilityAudit(for: .all) { issue in
            let found = issue.element
            let id = found?.identifier ?? "-"
            let frame = found?.frame ?? .zero
            // The two DEBUG probes are elements the tests read, not controls
            // a person meets.
            if ["native-composer-surface", "native-composer-probe"].contains(id) { return true }
            let line = "\(issue.auditType.rawValue) \(issue.compactDescription) [\(id) \(found?.label ?? "-") type \(found?.elementType.rawValue ?? 0) \(frame)]"
            // The cards' own contents are no elements: each card is one button
            // that hides its children from accessibility, so the audit
            // reaches their text with nothing to name. The bare harness has
            // no cards and none of these.
            let unnamed = found == nil && hasCards
            switch issue.auditType {
            case .contrast:
                if unnamed { print("AUDIT unnamed \(line)"); return true }
                if let found, found.elementType == .staticText {
                    if glass.contains(where: { $0.contains(frame) }) { return true }
                    // Scrolling up under the bar, a card or the header: it
                    // starts above the piece's lower edge, so part of it is
                    // read through the blur. Which rows straddle an edge
                    // depends on the phone's width and the OS's metrics.
                    // The scroll edge effect (ov-678) fades rows for a band
                    // below each piece too, about 48 pt: a row that starts
                    // inside the band is read through it.
                    let fade: CGFloat = 48
                    if (glass + [header]).contains(where: { $0.insetBy(dx: 0, dy: -fade).intersects(frame) && frame.minY < $0.maxY + fade }) { return true }
                }
            case .dynamicType:
                if let found, found.elementType == .staticText, barTitles.contains(found.label) { return true }
                if let found, found.elementType == .staticText, underGlassOrComposer(frame) { return true }
                // iOS 26.5's audit calls the transcript's plain `.body` and
                // `.callout` texts ("Keep the old names?", "Thinking") partly
                // unsupported where iOS 27's finds none: a system Text always
                // scales. Judged on 27, which is what CI runs; on 26 only the
                // texts inside the transcript's own rows are let go.
                if ProcessInfo.processInfo.operatingSystemVersion.majorVersion < 27,
                    let found, found.elementType == .staticText, inRow(frame) { return true }
                // The strip's NOW and NEXT labels: caption2 at the default
                // size, as ov-616 drew it, which the audit finds partly
                // fixed. Judged at the largest size, where they are .caption.
                if !large, ["plan-band-now", "plan-band-next"].contains(id), ["NOW", "NEXT"].contains(found?.label) { return true }
            case .textClipped:
                if let found, found.elementType == .staticText, underGlassOrComposer(frame) { return true }
                // The strip's Now and Next are one line each at the default
                // size, by design (ov-616): the card opens the plan, and its
                // spoken label is the whole sentence. At the largest size they
                // are in full, and judged.
                if !large, ["plan-band-now", "plan-band-next"].contains(id) || unnamed { return true }
            default: break
            }
            seen.append(line)
            return true
        }
        XCTAssertTrue(seen.isEmpty, "audit findings: \(seen)")
    }

    // MARK: - The composer (ov-614)

    /// Main's conversation with four agents running: the owner's screen,
    /// where the box was squeezed.
    private func openOrchestrator(extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents"] + extra)
        let main = app.buttons["workspace-row-Main"]
        // At the accessibility sizes the list runs below the first screen.
        XCTAssertTrue(app.reveal(main), "no Main row")
        main.tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60), "no composer")
        return app
    }

    /// Everything in the box is inside its surface, the photo button beside
    /// it on its bottom row (ov-677), and that row above the keyboard where
    /// one is up.
    private func assertContained(_ app: XCUIApplication, _ when: String) {
        let surface = element(app, "native-composer-surface").frame
        for id in ["native-composer", "native-send"] {
            let frame = element(app, id).frame
            XCTAssertTrue(
                surface.insetBy(dx: -0.5, dy: -0.5).contains(frame),
                "\(when): \(id) at \(frame) is outside the composer's surface \(surface)")
        }
        let attach = element(app, "native-attach").frame
        XCTAssertLessThanOrEqual(attach.maxX, surface.minX + 0.5, "\(when): the photo button is inside the box")
        XCTAssertEqual(attach.maxY, surface.maxY, accuracy: 0.5, "\(when): the photo button left the bottom row")
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

    /// A photo sits inside the box, over the text row, and Send stays on the
    /// bottom row with the photo button beside it.
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
        shoot("terminal-selection", expecting: .shows(copy, "the Copy menu over the dragged selection"))
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
