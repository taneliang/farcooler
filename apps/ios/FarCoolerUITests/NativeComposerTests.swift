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
    private func capture(_ name: String, expecting state: CaptureState) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        guard requireState(state, for: "composer-\(name)") else { return }
        let suffix = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_THEME"].map { "-\($0)" } ?? ""
        let url = URL(fileURLWithPath: out).appendingPathComponent("composer-\(name)\(suffix).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    /// What the canned runner has been asked: `sent=a|b images=mime:bytes pressed=stop`.
    private func harness(_ app: XCUIApplication) -> String {
        let said = element(app, "native-harness").value as? String ?? ""
        // The composer's probe: `overwrote=N` (`NativeComposerProbe`).
        let probe = element(app, "native-composer-probe")
        return probe.exists ? "\(said) \(probe.value as? String ?? "")" : said
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
        let field = focusField(app)
        app.typeText(words)
        holdsTyped(app, field, words)
    }

    /// The box ends with what was just typed. On CI the third typed letter
    /// has gone missing (ov-470 investigation): this says whether the box
    /// lost it, and `overwrote` whether the draft replaced the box's text.
    private func holdsTyped(_ app: XCUIApplication, _ field: XCUIElement, _ words: String) {
        XCTAssertTrue(
            wait(30) { (field.value as? String ?? "").hasSuffix(words) },
            "the box holds \"\(field.value as? String ?? "nil")\" after typing \"\(words)\": \(harness(app))")
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
        capture("lines", expecting: .when("the box holds both lines") { field.value as? String == "First line\nSecond line" })
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
        // A hardware key before ⌘↩, which moves nothing (the caret is at the
        // end already). typeText types on the onscreen keyboard, and on the
        // iOS 26.5 simulator the first key XCUI presses after it arrives
        // without its modifiers: ⌘↩ came as a bare Return, a new line (ov-682,
        // measured at UIApplication.sendEvent). On iOS 27 it kept them. A
        // person's ⌘ is a key press of its own, ahead of the Return, so the
        // Return is never that first key. And it is "\n": XCUI sends "\r"
        // (XCUIKeyboardKey.return) as no key at all.
        app.typeKey(XCUIKeyboardKey.rightArrow, modifierFlags: [])
        // typeText returns before the box has taken every key, and a loaded
        // runner can lose the keyboard between the two: press ⌘↩ only once the
        // box holds the whole draft and the keyboard again, or it sends a
        // shorter draft, or nothing.
        let field = element(app, "native-composer")
        XCTAssertTrue(
            wait(30) {
                field.value(forKey: "hasKeyboardFocus") as? Bool == true
                    && field.value as? String == "Sent from the keys"
            }, "the box never held the draft and the keyboard: \(field.value as? String ?? "nil")")
        app.typeKey("\n", modifierFlags: .command)
        XCTAssertTrue(
            wait(30) { self.said(app, "sent") == "Sent from the keys" }, "⌘↩ never sent: \(harness(app)) box=\(String(reflecting: field.value as? String ?? "nil"))")
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

    // MARK: claude's suggestion (ov-409)

    /// The suggestion stands where "Message Claude" does; a tap on it makes it
    /// the draft, the box takes the keyboard, and nothing is sent.
    func testATapTakesClaudesSuggestionAsADraftAndSendsNothing() {
        let app = launch(["-native-suggestion"])
        let suggestion = element(app, "native-suggestion")
        XCTAssertTrue(suggestion.waitForExistence(timeout: 60), "the suggestion never showed")
        XCTAssertEqual(suggestion.label, "Suggested message: Run the tests again.")
        capture("suggestion", expecting: .reads(suggestion, "Run the tests again."))
        suggestion.tap()
        let field = element(app, "native-composer")
        XCTAssertTrue(
            wait(30) { field.value as? String == "Run the tests again." }, "the draft is \(field.value as? String ?? "nil")")
        XCTAssertTrue(wait(30) { !suggestion.exists }, "a draft leaves the suggestion standing")
        XCTAssertTrue(
            wait(30) { field.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the box never took the keyboard to edit")
        capture("suggestion-taken", expecting: .all(.gone(suggestion, "the suggestion gone"), .when("the draft is the suggestion") { field.value as? String == "Run the tests again." }))
        // Nothing went to the runner on its own; the person's Send does it.
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(said(app, "sent"), "", "taking the suggestion sent it: \(harness(app))")
        sendButton(app).tap()
        XCTAssertTrue(wait(30) { self.said(app, "sent") == "Run the tests again." }, "Send never sent: \(harness(app))")
    }

    /// Typing replaces the suggestion, as it does "Message Claude".
    func testTypingReplacesTheSuggestion() {
        let app = launch(["-native-suggestion"])
        XCTAssertTrue(element(app, "native-suggestion").waitForExistence(timeout: 60))
        // A tap beside the suggestion's words, not on them, puts the caret in the box.
        let field = element(app, "native-composer")
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.5)).tap()
        XCTAssertTrue(wait(30) { field.value(forKey: "hasKeyboardFocus") as? Bool == true }, "the box never took the keyboard")
        XCTAssertTrue(element(app, "native-suggestion").exists, "a tap beside the words took the suggestion")
        app.typeText("No")
        XCTAssertTrue(wait(30) { !self.element(app, "native-suggestion").exists }, "the suggestion stayed over typed text")
        XCTAssertEqual(element(app, "native-composer").value as? String, "No")
    }

    /// claude's `Try "…"` example stands where "Message Claude" does, as a
    /// hint: a tap puts the caret in the box and takes nothing, and the hint's
    /// row is not in the transcript.
    func testTheTryExampleIsAHintAndNothingTakesIt() {
        let app = launch(["-native-hint"])
        let hint = element(app, "native-hint")
        XCTAssertTrue(hint.waitForExistence(timeout: 60), "the example never showed")
        XCTAssertEqual(hint.label, "Try \"how does <filepath> work?\"")
        XCTAssertFalse(element(app, "native-suggestion").exists, "an example is not a prediction")
        XCTAssertFalse(element(app, "native-row-hint:composer").exists, "the hint's row is in the transcript")
        capture("hint", expecting: .reads(hint, "how does"))
        let field = focusField(app)
        XCTAssertEqual(field.value as? String, "", "a tap took the example")
        XCTAssertTrue(hint.exists, "the example stays until something is typed")
        app.typeText("No")
        XCTAssertTrue(wait(30) { !hint.exists }, "the example stayed over typed text")
        XCTAssertEqual(said(app, "sent"), "")
    }

    /// While claude works, its dim line is a hint and none is offered.
    func testNoSuggestionWhileClaudeWorks() {
        let app = launch()
        XCTAssertFalse(element(app, "native-suggestion").exists)
    }

    // MARK: Photos

    /// A photo is a chip with a button to take it out, goes with the message,
    /// and the message is Sent, shown as a turn.
    func testAPhotoComposesWithTheMessage() {
        let app = launch()
        XCTAssertTrue(element(app, "native-attach").exists, "no photo button")
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 }, "no chip for the photo")
        capture("chip", expecting: .when("one photo chip") { self.chips(app) == 1 })
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
        XCTAssertFalse(sendButton(app).exists && sendButton(app).isEnabled, "Send on with nothing to send")
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 })
        XCTAssertTrue(wait(30) { self.sendButton(app).isEnabled }, "a photo alone can't be sent")
        sendButton(app).tap()
        XCTAssertTrue(wait(30) { !self.said(app, "images").isEmpty }, harness(app))
    }

    /// ov-622: the photos in an unsent message are still there after the app
    /// is killed and opened again, and still go with the message.
    func testAPhotoInTheDraftSurvivesARestart() {
        let app = launch()
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 }, "no chip for the photo")
        type(app, "Kept")
        // The draft's words wait a moment before they are written.
        Thread.sleep(forTimeInterval: 1.5)
        app.terminate()
        let again = launch(["-native-keep-photos"])
        XCTAssertTrue(wait(60) { self.chips(again) == 1 }, "the photo didn't come back after a restart")
        XCTAssertEqual(element(again, "native-composer").value as? String ?? "", "Kept", "the words didn't come back")
        sendButton(again).tap()
        XCTAssertTrue(wait(30) { self.said(again, "images").hasPrefix("image/png:") }, "the restored photo wasn't sent: \(harness(again))")
        XCTAssertTrue(wait(30) { self.chips(again) == 0 }, "the chip stayed after the send")
        // Sent, it isn't kept: a third launch has no photo. (The file's
        // removal is queued: on a loaded machine it can trail the chip.)
        Thread.sleep(forTimeInterval: 5)
        again.terminate()
        let third = launch(["-native-keep-photos"])
        Thread.sleep(forTimeInterval: 3)
        XCTAssertEqual(chips(third), 0, "a sent photo came back")
    }

    /// A photo taken out of the draft doesn't come back either.
    func testARemovedPhotoDoesNotComeBackAfterARestart() {
        let app = launch()
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 })
        element(app, "native-image-remove").tap()
        XCTAssertTrue(wait(30) { self.chips(app) == 0 })
        Thread.sleep(forTimeInterval: 5)
        app.terminate()
        let again = launch(["-native-keep-photos"])
        Thread.sleep(forTimeInterval: 3)
        XCTAssertEqual(chips(again), 0, "a removed photo came back")
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
        capture("queued", expecting: .all(.shows(queued.staticTexts["[Image] After this"]), .shows(queued.staticTexts["Queued"])))
    }

    // MARK: The Mac's refusal copy

    func testARefusedImageSaysWhichLimitItHit() {
        let app = launch()
        type(app, "image too large")
        sendButton(app).tap()
        XCTAssertTrue(
            app.staticTexts["That image is too large to send. Use a smaller one."].waitForExistence(timeout: 30),
            "\(app.staticTexts.allElementsBoundByIndex.map(\.label))")
        capture("refused", expecting: .shows(app.staticTexts["That image is too large to send. Use a smaller one."]))
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

    /// While claude works and the box is empty, Stop is the box's action,
    /// and presses the key once.
    func testStopShowsWhileClaudeWorksAndPressesTheKey() {
        let app = launch()
        let stop = element(app, "native-stop")
        XCTAssertTrue(stop.waitForExistence(timeout: 60), "no Stop while claude works")
        XCTAssertTrue(wait(10) { stop.isEnabled }, "Stop never armed")
        capture("stop", expecting: .shows(stop, "Stop in the box"))
        stop.tap()
        XCTAssertTrue(wait(30) { self.said(app, "pressed") == "stop" }, "Stop never reached the runner: \(harness(app))")
    }

    /// ov-677: one action at the end of the box, as in Messages. While
    /// claude works it is Stop with the box empty and Send with words in it,
    /// never both, and Stop comes back when the words are gone.
    func testTheActionIsStopWhenEmptyAndSendWithWords() {
        let app = launch()
        let stop = element(app, "native-stop")
        XCTAssertTrue(stop.waitForExistence(timeout: 60), "no Stop while claude works")
        XCTAssertFalse(sendButton(app).exists, "Send beside Stop with nothing to send")
        type(app, "Hi")
        XCTAssertTrue(wait(10) { self.sendButton(app).exists && !stop.exists }, "Stop stayed with words in the box")
        XCTAssertTrue(sendButton(app).isEnabled)
        capture("action-send", expecting: .shows(sendButton(app), "Send in Stop's place"))
        app.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 2))
        XCTAssertTrue(wait(10) { stop.exists && !self.sendButton(app).exists }, "Stop didn't come back to an empty box")
        // In the same place: the one target at the box's end.
        XCTAssertTrue(wait(10) { stop.isEnabled }, "Stop never armed")
    }

    /// At rest with an empty box there's nothing to send or stop: no action,
    /// and the text has the box's width. Words bring Send.
    func testAtRestAnEmptyBoxHasNoAction() {
        let app = launch(["-native-suggestion"])
        XCTAssertTrue(element(app, "native-suggestion").waitForExistence(timeout: 60), "claude never came to rest")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertFalse(sendButton(app).exists, "a Send with nothing to send")
        XCTAssertFalse(element(app, "native-stop").exists)
        type(app, "Yes")
        XCTAssertTrue(wait(10) { self.sendButton(app).exists }, "no Send with words in the box")
    }

    /// A quick second tap on Send never lands on the Stop that takes its
    /// place as the box clears (claude-code#100239: a shared button turned a
    /// Send into a Stop under the finger).
    func testADoubleTapOnSendNeverStops() {
        let app = launch(["-native-busy"])
        type(app, "Next, the tests")
        sendButton(app).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).doubleTap()
        XCTAssertTrue(element(app, "native-queued").waitForExistence(timeout: 30), "no Queued row")
        XCTAssertTrue(element(app, "native-stop").waitForExistence(timeout: 30), "no Stop after the send")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(said(app, "pressed"), "", "the second tap pressed Stop: \(harness(app))")
    }

    /// ov-677: the photo button is its own 44 pt circle outside the box, on
    /// its left, its foot on the box's foot, as Messages' + is.
    func testThePhotoButtonSitsOutsideTheBox() {
        let app = launch()
        let attach = element(app, "native-attach"), surface = element(app, "native-composer-surface")
        XCTAssertTrue(attach.waitForExistence(timeout: 60), "no photo button")
        let button = attach.frame, box = surface.frame
        XCTAssertLessThanOrEqual(button.maxX, box.minX + 0.5, "the photo button at \(button) is inside the box \(box)")
        XCTAssertGreaterThanOrEqual(button.width, 44, "the photo button is \(button.width) pt wide")
        XCTAssertGreaterThanOrEqual(button.height, 44, "the photo button is \(button.height) pt tall")
        XCTAssertEqual(button.maxY, box.maxY, accuracy: 0.5, "the photo button's foot isn't the box's")
        XCTAssertEqual(box.minX - button.maxX, 12, accuracy: 0.5, "the gap between them")
        // The text starts close to the box's edge, as in Messages.
        let field = element(app, "native-composer").frame
        XCTAssertEqual(field.minX - box.minX, 16, accuracy: 0.5, "the text starts \(field.minX - box.minX) pt in")
        // With a photo, the button stays on the text's row.
        post("com.farcooler.harness.native-photo")
        XCTAssertTrue(wait(60) { self.chips(app) == 1 }, "no chip for the photo")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(attach.frame.maxY, surface.frame.maxY, accuracy: 0.5, "the photo button left the text's row")
        let chip = element(app, "native-image-chip").frame
        XCTAssertTrue(surface.frame.contains(chip), "the photo at \(chip) isn't inside the box \(surface.frame)")
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
        capture("send-now", expecting: .shows(sendNow, "Send Now on the Queued row"))
        sendNow.tap()
        XCTAssertTrue(wait(30) { self.said(app, "pressed") == "sendnow" }, "Send Now never reached the runner: \(harness(app))")
    }

    /// A key the runner refuses as `settling` says which key to try again.
    func testAStopThatSettlingRefusesSaysToTryAgain() {
        let app = launch(["-native-settling"])
        let stop = element(app, "native-stop")
        XCTAssertTrue(stop.waitForExistence(timeout: 60))
        XCTAssertTrue(wait(10) { stop.isEnabled }, "Stop never armed")
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
