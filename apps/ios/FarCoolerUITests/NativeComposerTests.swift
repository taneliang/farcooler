import XCTest

/// The conversation composer at the Mac's parity (ov-404), on
/// `-native-agent-harness`: the real `NativeComposer` over a canned runner
/// that has `compose` and `terminal_interrupt`, reached through the client
/// core's own call path. Multi-line text, photos (picked, pasted, and a 10 MB
/// one), the Mac's refusal copy, Stop and Send Now. The system's photo picker
/// and paste menu can't be driven, so the harness posts the photo a picker
/// would hand over (`NativePhotoHarness`); each takes the composer's own path.
final class NativeComposerTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ flags: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"] + flags
        if let theme = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_THEME"] {
            app.launchArguments += ["-app.theme", theme]
        }
        app.launchDrawn()
        let transcript = element(app, "native-transcript")
        XCTAssertTrue(transcript.waitForExistence(timeout: 60), "the conversation never showed")
        return app
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

    /// The screen, to `FARCOOLER_CAPTURE_OUT/composer-<name>.png` when that's
    /// set (as `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`): captures for a review,
    /// off in CI.
    private func capture(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let suffix = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_THEME"].map { "-\($0)" } ?? ""
        let url = URL(fileURLWithPath: out).appendingPathComponent("composer-\(name)\(suffix).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    /// What the canned runner has been asked: `sent=a|b images=mime:bytes pressed=stop`.
    private func harness(_ app: XCUIApplication) -> String {
        element(app, "native-harness").value as? String ?? ""
    }

    /// The part of the probe after `key=` up to the next space (or the end).
    private func said(_ app: XCUIApplication, _ key: String) -> String {
        let all = harness(app)
        guard let range = all.range(of: "\(key)=") else { return "" }
        let rest = all[range.upperBound...]
        // `sent` can hold spaces; it ends at ` images=`.
        let end = key == "sent" ? rest.range(of: " images=")?.lowerBound : rest.firstIndex(of: " ")
        return String(rest[..<(end ?? rest.endIndex)])
    }

    private func focusField(_ app: XCUIApplication) -> XCUIElement {
        let field = element(app, "native-composer")
        XCTAssertTrue(field.waitForExistence(timeout: 60))
        if field.value(forKey: "hasKeyboardFocus") as? Bool != true { field.tap() }
        XCTAssertTrue(
            wait(30) { field.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the box never took the keyboard")
        return field
    }

    /// Type once the box holds the keyboard.
    private func type(_ app: XCUIApplication, _ words: String) {
        _ = focusField(app)
        app.typeText(words)
    }

    private func sendButton(_ app: XCUIApplication) -> XCUIElement { element(app, "native-send") }

    private func chips(_ app: XCUIApplication) -> Int {
        app.descendants(matching: .any).matching(identifier: "native-image-chip").count
    }

    // MARK: Lines

    /// Return is a new line, not a send, and Send sends every line, as typed.
    func testReturnIsANewLineAndSendSendsEveryLine() {
        let app = launch()
        type(app, "First line")
        app.typeText("\n")
        app.typeText("Second line")
        let field = element(app, "native-composer")
        XCTAssertEqual(field.value as? String, "First line\nSecond line", "Return didn't make a new line")
        XCTAssertEqual(said(app, "sent"), "", "Return sent the message")
        capture("lines")
        sendButton(app).tap()
        XCTAssertTrue(
            wait(30) { self.said(app, "sent") == "First line⏎Second line" },
            "the lines never reached the runner as typed: \(harness(app))")
        XCTAssertTrue(wait(30) { (field.value as? String ?? "").isEmpty }, "the box kept the sent draft")
    }

    /// A hardware keyboard's ⌘↩ sends, as on the Mac's Return.
    func testCommandReturnSends() {
        let app = launch()
        type(app, "Sent from the keys")
        app.typeKey("\n", modifierFlags: .command)
        XCTAssertTrue(
            wait(30) { self.said(app, "sent") == "Sent from the keys" }, "⌘↩ never sent: \(harness(app))")
    }

    /// A slash command goes to the runner, which drives claude's picker,
    /// where the one-line box refused it.
    func testASlashCommandIsSent() {
        let app = launch()
        type(app, "/compact")
        sendButton(app).tap()
        XCTAssertTrue(wait(30) { self.said(app, "sent") == "/compact" }, "the command never reached the runner: \(harness(app))")
    }

    /// A runner without `compose` gets one line and no photos, as before.
    func testWithoutComposeItIsOneLineWithNoPhotos() {
        let app = launch(["-native-no-compose"])
        XCTAssertFalse(element(app, "native-attach").exists, "a photo button over a runner that takes none")
        type(app, "One line")
        app.typeText("\n")
        XCTAssertTrue(wait(30) { self.said(app, "sent") == "One line" }, "Return didn't send on a one-line runner: \(harness(app))")
    }

    // MARK: Photos

    /// A photo is a chip with a button to take it out, goes with the message,
    /// and the message is Sent, shown as a turn.
    func testAPhotoComposesWithTheMessage() {
        let app = launch()
        XCTAssertTrue(element(app, "native-attach").exists, "no photo button")
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 }, "no chip for the photo")
        capture("chip")
        type(app, "What is this")
        XCTAssertTrue(sendButton(app).isEnabled)
        sendButton(app).tap()
        XCTAssertTrue(wait(30) { self.said(app, "sent") == "What is this" }, harness(app))
        let image = said(app, "images")
        XCTAssertTrue(image.hasPrefix("image/png:"), "the photo's type and size at the runner: \(image)")
        XCTAssertTrue(wait(30) { self.chips(app) == 0 }, "the chip stayed after the send")
        XCTAssertTrue(app.staticTexts["What is this"].waitForExistence(timeout: 30), "the sent turn never showed")
    }

    /// A photo on its own is a message: Send is on with no words.
    func testAPhotoAloneCanBeSent() {
        let app = launch()
        XCTAssertFalse(sendButton(app).isEnabled)
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 })
        XCTAssertTrue(wait(30) { self.sendButton(app).isEnabled }, "a photo alone can't be sent")
        sendButton(app).tap()
        XCTAssertTrue(wait(30) { !self.said(app, "images").isEmpty }, harness(app))
    }

    /// The remove button takes the chip out of the message.
    func testARemovedPhotoIsNotSent() {
        let app = launch()
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 })
        element(app, "native-image-remove").tap()
        XCTAssertTrue(wait(30) { self.chips(app) == 0 }, "Remove Photo left the chip")
        type(app, "Words only")
        sendButton(app).tap()
        XCTAssertTrue(wait(30) { self.said(app, "sent") == "Words only" }, harness(app))
        XCTAssertEqual(said(app, "images"), "", "a removed photo was sent")
    }

    /// An image on the pasteboard, pasted into the box, is a chip, and not
    /// text. Through the field's own `paste(_:)`.
    func testAPastedImageIsAChip() {
        let app = launch()
        _ = focusField(app)
        post("com.farcooler.harness.native-paste")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 }, "the pasted image never became a chip")
        XCTAssertEqual(element(app, "native-composer").value as? String ?? "", "", "the paste put something in the text")
    }

    /// ov-393's iPhone half: a 10 MB photo composes. It is picked the way a
    /// photo is, read, kept as it is (a JPEG the runner reads, under its
    /// 16 MB), sent through the client core's own `terminal.compose` call, and
    /// arrives at the runner whole.
    func testATenMegabytePhotoComposesFromTheIPhone() {
        let app = launch()
        post("com.farcooler.harness.native-big-photo")
        XCTAssertTrue(wait(120) { self.chips(app) == 1 }, "the 10 MB photo never became a chip: \(harness(app))")
        type(app, "A big one")
        sendButton(app).tap()
        XCTAssertTrue(wait(120) { self.said(app, "sent") == "A big one" }, "the compose never reached the runner: \(harness(app))")
        let parts = said(app, "images").split(separator: ":")
        XCTAssertEqual(parts.first.map(String.init), "image/jpeg")
        let bytes = parts.last.flatMap { Int($0) } ?? 0
        XCTAssertGreaterThanOrEqual(bytes, 10 * 1024 * 1024, "the photo arrived smaller than it was: \(bytes)")
        XCTAssertLessThan(bytes, 16 * 1024 * 1024)
        XCTAssertTrue(wait(60) { self.chips(app) == 0 }, "the chip stayed after the send")
        XCTAssertTrue(app.staticTexts["A big one"].waitForExistence(timeout: 30), "Sent: the turn never showed")
    }

    /// A photo sent while claude works is Queued, and says so with the photo.
    func testAPhotoSentWhileClaudeWorksIsQueued() {
        let app = launch(["-native-busy"])
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 })
        type(app, "After this")
        sendButton(app).tap()
        let queued = element(app, "native-queued")
        XCTAssertTrue(queued.waitForExistence(timeout: 30), "no Queued row")
        XCTAssertTrue(queued.staticTexts["[Image] After this"].exists, "\(queued.debugDescription)")
        XCTAssertTrue(queued.staticTexts["Queued"].exists)
        capture("queued")
    }

    // MARK: The Mac's refusal copy

    func testARefusedImageSaysWhichLimitItHit() {
        let app = launch()
        type(app, "image too large")
        sendButton(app).tap()
        XCTAssertTrue(
            app.staticTexts["That image is too large to send. Use a smaller one."].waitForExistence(timeout: 30),
            "\(app.staticTexts.allElementsBoundByIndex.map(\.label))")
        capture("refused")
    }

    func testABackslashRefusalSaysHowToFixIt() {
        let app = launch()
        type(app, "backslash")
        sendButton(app).tap()
        XCTAssertTrue(
            app.staticTexts["Claude reads a backslash at the end as a new line, so the message wasn’t sent. Remove it, or add a word after it."]
                .waitForExistence(timeout: 30))
    }

    // MARK: Stop and Send Now

    /// While claude works Stop is beside Send, and presses the key once.
    func testStopShowsWhileClaudeWorksAndPressesTheKey() {
        let app = launch()
        let stop = element(app, "native-stop")
        XCTAssertTrue(stop.waitForExistence(timeout: 60), "no Stop while claude works")
        capture("stop")
        stop.tap()
        XCTAssertTrue(wait(30) { self.said(app, "pressed") == "stop" }, "Stop never reached the runner: \(harness(app))")
    }

    /// Under a dialog the registry says Waiting, and an Esc would answer it
    /// No: Stop is hidden.
    func testStopIsHiddenUnderAWaitingDialog() {
        let app = launch(["-native-waiting"])
        XCTAssertTrue(app.staticTexts["And the lexer."].waitForExistence(timeout: 60))
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(element(app, "native-stop").exists, "Stop shown over a waiting dialog")
    }

    /// A runner without `terminal_interrupt` offers neither Stop nor Send Now.
    func testNoStopWithoutTheCapability() {
        let app = launch(["-native-no-interrupt", "-native-busy"])
        XCTAssertTrue(app.staticTexts["And the lexer."].waitForExistence(timeout: 60))
        type(app, "Wait for me")
        sendButton(app).tap()
        XCTAssertTrue(element(app, "native-queued").waitForExistence(timeout: 30))
        Thread.sleep(forTimeInterval: 1)
        XCTAssertFalse(element(app, "native-stop").exists, "Stop on a runner that can't press it")
        XCTAssertFalse(element(app, "native-send-now").exists, "Send Now on a runner that can't press it")
    }

    /// A Queued row has Send Now while claude works, and it presses the key.
    func testSendNowOnAQueuedRow() {
        let app = launch(["-native-busy"])
        type(app, "Next, the tests")
        sendButton(app).tap()
        let queued = element(app, "native-queued")
        XCTAssertTrue(queued.waitForExistence(timeout: 30), "no Queued row")
        let sendNow = queued.buttons["Send Now"]
        XCTAssertTrue(sendNow.waitForExistence(timeout: 30), "no Send Now on the Queued row")
        capture("send-now")
        sendNow.tap()
        XCTAssertTrue(wait(30) { self.said(app, "pressed") == "sendnow" }, "Send Now never reached the runner: \(harness(app))")
    }

    /// A key the runner refuses as `settling` says which key to try again.
    func testAStopThatSettlingRefusesSaysToTryAgain() {
        let app = launch(["-native-settling"])
        let stop = element(app, "native-stop")
        XCTAssertTrue(stop.waitForExistence(timeout: 60))
        stop.tap()
        XCTAssertTrue(
            app.staticTexts["Claude is starting a step. Try Stop again in a moment."].waitForExistence(timeout: 30),
            "\(app.staticTexts.allElementsBoundByIndex.map(\.label))")
        XCTAssertEqual(said(app, "pressed"), "")
    }

    /// Under a dialog Stop is hidden, and so is Send Now on a message queued
    /// before it (ov-404 review 1: the gate was tested only for Stop).
    func testSendNowIsHiddenUnderAWaitingDialog() {
        let app = launch(["-native-waiting", "-native-busy"])
        XCTAssertTrue(app.staticTexts["And the lexer."].waitForExistence(timeout: 60))
        type(app, "Wait for me")
        sendButton(app).tap()
        XCTAssertTrue(element(app, "native-queued").waitForExistence(timeout: 30), "no Queued row")
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(element(app, "native-send-now").exists, "Send Now offered over a waiting dialog")
        XCTAssertFalse(element(app, "native-stop").exists)
    }

    /// Rows held and the runner not answering: neither key is offered, though
    /// claude was working when the rows were last read.
    func testNeitherKeyIsOfferedOverStaleRows() {
        let app = launch(["-native-stale"])
        // Stop shows while the rows are live (`testStopShowsWhileClaudeWorksAndPressesTheKey`);
        // the fourth follow's failure comes within seconds, so only the end is read.
        XCTAssertTrue(element(app, "native-stale").waitForExistence(timeout: 60), "the rows never went stale: \(harness(app))")
        XCTAssertTrue(wait(30) { !self.element(app, "native-stop").exists }, "Stop offered over stale rows")
        XCTAssertFalse(element(app, "native-send-now").exists)
    }
}
