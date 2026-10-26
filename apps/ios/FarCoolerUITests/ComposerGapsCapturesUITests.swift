import XCTest

/// Measurements and captures for the composer's gaps against the system's
/// (ov-804): Messages' field and +, the bottom bars of system apps that put
/// a field at the screen's foot, and ours, each with the keyboard down and up, empty and with a
/// long draft. Opt-in by its own switch, not the capture jobs' one, because
/// it drives the simulator's own apps, whose content a CI image
/// doesn't promise: `TEST_RUNNER_FARCOOLER_GAPS_OUT=<dir>` turns it on, and
/// `TEST_RUNNER_FARCOOLER_CAPTURE_TAG=<phase>-<device>` names the files.
///
/// Each state writes a screenshot, the app's accessibility tree, and a JSON
/// line of the frames that matter, in points, with the screenshot's width in pixels, so the
/// tables are counted from the same run that drew the pictures.
final class ComposerGapsCapturesUITests: XCTestCase {
    private var out: String? { ProcessInfo.processInfo.environment["FARCOOLER_GAPS_OUT"] }
    private var tag: String { ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_TAG"] ?? "capture" }

    override func setUp() {
        continueAfterFailure = true
    }

    /// Several lines, one of them long enough to wrap.
    static let draft = "Before you land it, check the composer at both widths.\nThe gaps should match Messages,\nwith the keyboard down and up,\nand the + no taller than the field when the draft wraps across more than one line."

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

    private func rect(_ r: CGRect) -> [String: Double] {
        ["x": r.minX, "y": r.minY, "w": r.width, "h": r.height]
    }

    /// The screenshot, the tree and the frames of `parts` for one state.
    private func record(_ app: XCUIApplication, _ name: String, _ parts: [String: XCUIElement]) {
        guard let out else { return }
        let base = URL(fileURLWithPath: out).appendingPathComponent("\(tag)-\(name)")
        Thread.sleep(forTimeInterval: 1.5)
        let shot = XCUIScreen.main.screenshot()
        try? shot.pngRepresentation.write(to: base.appendingPathExtension("png"))
        try? app.debugDescription.write(to: base.appendingPathExtension("tree.txt"), atomically: true, encoding: .utf8)
        var frames: [String: Any] = [
            "state": name, "pixelWidth": shot.image.cgImage?.width ?? 0,
            "window": rect(app.windows.firstMatch.frame),
            "keyboard": app.keyboards.firstMatch.exists ? rect(app.keyboards.firstMatch.frame) : NSNull(),
        ]
        for (key, element) in parts where element.exists { frames[key] = rect(element.frame) }
        if let data = try? JSONSerialization.data(withJSONObject: frames, options: [.sortedKeys]) {
            try? data.write(to: base.appendingPathExtension("json"))
        }
    }

    /// The keyboard put down by a drag down the content above the field.
    private func lowerKeyboard(_ app: XCUIApplication, above field: XCUIElement) {
        let window = app.windows.firstMatch.frame
        let from = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: window.midX, dy: window.height * 0.3))
        from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: window.height * 0.5)))
        _ = wait(5) { !app.keyboards.firstMatch.exists }
    }

    func testMessages() throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_GAPS_OUT")
        XCUIDevice.shared.appearance = .light
        let messages = XCUIApplication(bundleIdentifier: "com.apple.MobileSMS")
        messages.launch()
        let conversation = messages.cells.firstMatch
        if conversation.waitForExistence(timeout: 15) { conversation.tap() }
        let field = messages.textFields["messageBodyField"].firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no message field: \(messages.debugDescription)")
        let parts = ["field": field, "plus": messages.buttons["add"].firstMatch]
        record(messages, "messages-collapsed", parts)
        field.tap()
        _ = wait(5) { messages.keyboards.firstMatch.exists }
        record(messages, "messages-focused", parts)
        field.typeText(Self.draft)
        record(messages, "messages-focused-draft", parts)
        lowerKeyboard(messages, above: field)
        record(messages, "messages-collapsed-draft", parts)
        // The draft taken back out, so the simulator's Messages is as found.
        field.tap()
        _ = wait(5) { messages.keyboards.firstMatch.exists }
        field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: Self.draft.count + 4))
        messages.terminate()
    }

    /// System apps whose own bars put a field beside a button at the
    /// screen's foot (Contacts' and Reminders' searchable bars, Safari's
    /// address bar), as UIKit lays them out: each shot as it opens, then
    /// with its bottom field focused.
    func testSystemBars() throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_GAPS_OUT")
        XCUIDevice.shared.appearance = .light
        let apps = [
            "contacts": "com.apple.MobileAddressBook", "reminders": "com.apple.reminders",
            "safari": "com.apple.mobilesafari", "files": "com.apple.DocumentsApp", "shortcuts": "com.apple.shortcuts",
        ]
        for (name, id) in apps.sorted(by: { $0.key < $1.key }) {
            let app = XCUIApplication(bundleIdentifier: id)
            app.launch()
            Thread.sleep(forTimeInterval: 4)
            for label in ["Continue", "Not Now", "Get Started"] where app.buttons[label].exists { app.buttons[label].tap() }
            let height = app.windows.firstMatch.frame.height
            let low = app.descendants(matching: .any).matching(
                NSPredicate(format: "elementType == %d OR elementType == %d", XCUIElement.ElementType.searchField.rawValue,
                            XCUIElement.ElementType.textField.rawValue))
                .allElementsBoundByIndex.first { $0.exists && $0.frame.minY > height * 0.6 }
            let parts = low.map { ["search": $0] } ?? [:]
            record(app, "\(name)-collapsed", parts)
            if let low {
                low.tap()
                _ = wait(5) { app.keyboards.firstMatch.exists }
                record(app, "\(name)-focused", parts)
            }
            app.terminate()
        }
    }

    /// The keyboard up and down three times in Messages, then in ours, for
    /// a screen recording read frame by frame (`xcrun simctl io <device>
    /// recordVideo` around the run): the bar's widening has to go with the
    /// keyboard. `TEST_RUNNER_FARCOOLER_GAPS_FRAMES=1` turns it on.
    func testKeyboardMotionForFrames() throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["FARCOOLER_GAPS_FRAMES"] == nil, "frames only; set TEST_RUNNER_FARCOOLER_GAPS_FRAMES")
        XCUIDevice.shared.appearance = .light
        let messages = XCUIApplication(bundleIdentifier: "com.apple.MobileSMS")
        messages.launch()
        let conversation = messages.cells.firstMatch
        if conversation.waitForExistence(timeout: 15) { conversation.tap() }
        let theirs = messages.textFields["messageBodyField"].firstMatch
        XCTAssertTrue(theirs.waitForExistence(timeout: 15), "no message field")
        cycleKeyboard(messages, theirs)
        messages.terminate()
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"]
        app.launchDrawn()
        let ours = element(app, "native-composer")
        XCTAssertTrue(ours.waitForExistence(timeout: 60), "no composer")
        cycleKeyboard(app, ours)
        app.terminate()
    }

    private func cycleKeyboard(_ app: XCUIApplication, _ field: XCUIElement) {
        Thread.sleep(forTimeInterval: 2)
        for _ in 0..<3 {
            field.tap()
            _ = wait(5) { app.keyboards.firstMatch.exists }
            Thread.sleep(forTimeInterval: 2)
            lowerKeyboard(app, above: field)
            Thread.sleep(forTimeInterval: 2)
        }
    }

    func testOurs() throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_GAPS_OUT")
        XCUIDevice.shared.appearance = .light
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"]
        app.launchDrawn()
        let field = element(app, "native-composer")
        XCTAssertTrue(field.waitForExistence(timeout: 60), "no composer")
        let parts = [
            "field": element(app, "native-composer-surface"), "text": field, "plus": element(app, "native-attach"),
            "stack": element(app, "native-composer-stack"),
        ]
        record(app, "ours-collapsed", parts)
        field.tap()
        _ = wait(10) { app.keyboards.firstMatch.exists }
        record(app, "ours-focused", parts)
        app.typeText(Self.draft)
        record(app, "ours-focused-draft", parts)
        lowerKeyboard(app, above: field)
        record(app, "ours-collapsed-draft", parts)
        app.terminate()
    }
}
