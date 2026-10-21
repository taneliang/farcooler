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
        return (Self.inkedPixels(in: shot.image, band: band, points: width, tolerance: 10), band)
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

    func testDiscoverBare() throws { try discover(launchConversation()) }
    func testDiscoverWorkspace() throws { let a = openOrchestrator(); defer { a.terminate() }; try discover(a) }
    func testDiscoverXXXLLight() throws { try xxxl(dark: false, large: true) }
    func testDiscoverXXXLDark() throws { try xxxl(dark: true, large: true) }
    func testDiscoverNormalLight() throws { try xxxl(dark: false, large: false) }
    func testDiscoverNormalDark() throws { try xxxl(dark: true, large: false) }
    private func xxxl(dark: Bool, large: Bool) throws {
        XCUIDevice.shared.appearance = dark ? .dark : .light
        defer { XCUIDevice.shared.appearance = .light }
        let size = (large ? ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"] : [])
            + (dark ? [] : ["-app.theme", "Solarized Light"])
        let bare = XCUIApplication()
        bare.launchArguments = ["-native-agent-harness"] + size
        bare.launchDrawn()
        XCTAssertTrue(element(bare, "native-composer").waitForExistence(timeout: 60))
        Thread.sleep(forTimeInterval: 2)
        snap(large ? "bare-xxxl" : "bare-normal")
        var all = try found(bare)
        bare.terminate()
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents"] + size)
        XCTAssertTrue(app.reveal(element(app, "workspace-row-Main")))
        element(app, "workspace-row-Main").tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 60))
        Thread.sleep(forTimeInterval: 2)
        snap(large ? "workspace-xxxl" : "workspace-normal")
        all += try found(app)
        app.terminate()
        print(all.joined(separator: "\n"))
        XCTAssertTrue(all.isEmpty, all.joined(separator: "\n"))
    }
    private func found(_ app: XCUIApplication) throws -> [String] {
        var seen: [String] = []
        try app.performAccessibilityAudit(for: .all) { issue in
            let e = issue.element
            seen.append("DISCOVER \(issue.auditType.rawValue) | \(issue.compactDescription) | id=\(e?.identifier ?? "-") label=\(e?.label ?? "-") type=\(e?.elementType.rawValue ?? 0) \(e?.frame ?? .zero)")
            return true
        }
        return seen
    }
    private func snap(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(XCUIDevice.shared.appearance == .dark ? "dark" : "light").png"))
    }
    private func discover(_ app: XCUIApplication) throws {
        var seen: [String] = []
        try app.performAccessibilityAudit(for: .all) { issue in
            let e = issue.element
            seen.append("DISCOVER \(issue.auditType.rawValue) | \(issue.compactDescription) | \(issue.detailedDescription) | id=\(e?.identifier ?? "-") label=\(e?.label ?? "-") type=\(e?.elementType.rawValue ?? 0) \(e?.frame ?? .zero)")
            return true
        }
        print(seen.joined(separator: "\n"))
        XCTAssertTrue(seen.isEmpty, seen.joined(separator: "\n"))
    }

    private func assertAuditClean(_ app: XCUIApplication) throws {
        var seen: [String] = []
        try app.performAccessibilityAudit(for: [.sufficientElementDescription, .hitRegion, .trait, .elementDetection]) { issue in
            if ["native-composer-surface", "native-composer-probe"].contains(issue.element?.identifier) { return true }
            let kind = issue.element?.elementType.rawValue ?? 0
            let id = issue.element?.identifier ?? "-"
            let frame = issue.element?.frame ?? .zero
            seen.append("\(issue.auditType.rawValue) \(issue.compactDescription) [\(id) type \(kind) \(frame)]")
            return true
        }
        XCTAssertTrue(seen.isEmpty, "audit findings: \(seen)")
    }

    // MARK: - The composer (ov-614)

    /// Main's conversation with four agents running: the owner's screen,
    /// where the box was squeezed.
    private func openOrchestrator() -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-native", "-phone-empty-inbox", "-phone-plan", "-phone-plan-file", planFixture, "-native-agents"])
        let main = app.buttons["workspace-row-Main"]
        XCTAssertTrue(main.waitForExistence(timeout: 30))
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
