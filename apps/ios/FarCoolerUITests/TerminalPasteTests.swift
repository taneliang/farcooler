import XCTest

/// Paste into a terminal pane, from the system edit menu (ov-604).
///
/// Against the demo runner (`./scripts/demo-host.sh`). The pane is pixels, so
/// what the shell received is read off the Mac instead: the pasted command
/// appends one line to a marker file, and nothing runs until Return. That makes
/// three things checkable. The paste arrives, so the file gets its line. It
/// arrives as ONE write, so the line is exactly `PASTED`: a paste sent twice
/// would be `echo PASTED >> fileecho PASTED >> file`, which writes somewhere
/// else. And nothing presses Return for the person, so the file does not exist
/// until the test presses it.
///
/// A long press on a link still offering the link is `TerminalTaskKeyTests`.
final class TerminalPasteTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    private func surface(_ app: XCUIApplication) -> XCUIElement? {
        let all = app.otherElements.matching(identifier: "terminal-surface")
        for i in 0..<all.count {
            let element = all.element(boundBy: i)
            if element.exists, let value = element.value as? String, value.contains("visible=1") {
                return element
            }
        }
        return nil
    }

    private func openAShellPane(_ app: XCUIApplication) throws -> XCUIElement {
        try LiveRunner.openWorktree(app, named: "scrolling")
        for _ in 0..<6 {
            let deadline = Date().addingTimeInterval(3)
            repeat {
                if let found = surface(app) { return found }
            } while Date() < deadline
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: 0.42)).press(
                forDuration: 0.05,
                thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: 0.42)),
                withVelocity: .slow, thenHoldForDuration: 0.4)
        }
        throw HarnessFailure("no shell pane drew: \(app.debugDescription)")
    }

    private func waitFor(_ seconds: TimeInterval, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.5)
        } while Date() < deadline
        return condition()
    }

    func testPasteFromTheEditMenuArrivesOnceAndPressesNoReturn() throws {
        let marker = "/tmp/farcooler-ui-paste-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(atPath: marker) }
        let app = LiveRunner.launch()
        let pane = try openAShellPane(app)

        UIPasteboard.general.string = "echo PASTED >> \(marker)"
        // Away from any link: the pane's lower half is empty rows.
        pane.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.6)).press(forDuration: 1.2)
        let paste = app.menuItems["Paste"].exists ? app.menuItems["Paste"] : app.buttons["Paste"]
        XCTAssertTrue(paste.waitForExistence(timeout: 10), "a long press on empty terminal offered no Paste")
        XCTAssertFalse(app.buttons["AutoFill"].exists || app.menuItems["AutoFill"].exists,
            "the menu offers AutoFill, which would type a saved password into the shell")
        if let shot = ProcessInfo.processInfo.environment["PASTE_SHOT"] {
            try? app.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: shot))
        }
        paste.tap()

        // Pasted, not run: nothing pressed Return.
        Thread.sleep(forTimeInterval: 3)
        XCTAssertFalse(FileManager.default.fileExists(atPath: marker), "the paste ran without a Return")

        // The software keyboard's Return key, which sends a newline. By
        // coordinate: the keyboard is not a scroll view, so a tap that first
        // scrolls the key to visible fails on it.
        let send = app.keyboards.buttons.matching(NSPredicate(format: "label ==[c] 'send'")).firstMatch
        XCTAssertTrue(send.waitForExistence(timeout: 10), "no Return key on the keyboard")
        // The key is still sliding up when it first exists, and a tap that lands
        // early is lost; tap again until the line has run (a Return on an empty
        // prompt is harmless).
        for _ in 0..<4 where !FileManager.default.fileExists(atPath: marker) {
            Thread.sleep(forTimeInterval: 2)
            if send.frame.maxY <= app.frame.maxY {
                send.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            } else {
                // The simulator hides the software keyboard while a hardware one
                // is attached; Return then comes from the hardware keyboard.
                app.typeKey(.return, modifierFlags: [])
            }
        }
        XCTAssertTrue(
            waitFor(30) { FileManager.default.fileExists(atPath: marker) },
            "the pasted command never reached the shell")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(try String(contentsOfFile: marker, encoding: .utf8), "PASTED\n")
    }
}
