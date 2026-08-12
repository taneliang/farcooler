import XCTest

final class KeyboardTabStripTests: XCTestCase {
    func testTabStripRemainsInteractiveAfterKeyboardDismissal() throws {
        #if targetEnvironment(simulator)
        throw XCTSkip("This regression needs a real iPhone with a configured workspace.")
        #endif

        let app = XCUIApplication()
        app.launch()

        let composer = app.textViews.firstMatch
        guard composer.waitForExistence(timeout: 10) else {
            throw XCTSkip("The current device state has no agent composer to exercise.")
        }

        let otherTab = app.buttons.matching(
            NSPredicate(format: "label ==[c] %@ AND value != %@", "changes", "current")
        ).firstMatch
        guard otherTab.waitForExistence(timeout: 5) else {
            throw XCTSkip("The current workspace has no unselected changes pane.")
        }
        let initialTabFrame = otherTab.frame
        let initialSwitcherFrame = app.buttons["Switch terminal"].frame
        let originalTab = app.buttons.matching(
            NSPredicate(format: "value == %@", "current")
        ).firstMatch
        let originalTabIdentifier = originalTab.identifier
        let transcript = app.scrollViews["agent-transcript"]
        guard transcript.waitForExistence(timeout: 5) else {
            throw XCTSkip("The current agent pane has no transcript scroll view.")
        }
        // Establish the exact precondition under test. The live transcript can
        // grow while this suite is being rebuilt, so do not depend on how far
        // a previous app session happened to leave it scrolled.
        for _ in 0..<10 {
            if (transcript.value as? String)?.hasSuffix("tail=true") == true { break }
            transcript.swipeUp(velocity: .fast)
        }
        XCTAssertTrue(
            (transcript.value as? String)?.hasSuffix("tail=true") == true,
            "Could not establish a transcript parked at its tail"
        )

        composer.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let remainsAtTail = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value ENDSWITH %@", "tail=true"),
            object: transcript
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [remainsAtTail], timeout: 2), .completed,
            "Opening the keyboard pulled a tail-following transcript off the bottom"
        )
        let keyboardTabFrame = otherTab.frame
        XCTAssertEqual(
            keyboardTabFrame.minY, initialTabFrame.minY, accuracy: 2,
            "The tab strip moved when the keyboard appeared"
        )

        app.swipeDown()
        let keyboardDismissed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == NO"),
            object: app.keyboards.firstMatch
        )
        XCTAssertEqual(XCTWaiter.wait(for: [keyboardDismissed], timeout: 5), .completed)

        let dismissedTabFrame = otherTab.frame
        XCTAssertEqual(
            dismissedTabFrame.minY, initialTabFrame.minY, accuracy: 2,
            "The tab strip moved after the keyboard disappeared"
        )
        let transcriptInsetReset = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value BEGINSWITH %@", "keyboard=0;"),
            object: transcript
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [transcriptInsetReset], timeout: 2), .completed,
            "The transcript retained the keyboard-height bottom inset after dismissal: \(String(describing: transcript.value))"
        )
        XCTAssertTrue(otherTab.isHittable, "The tab strip moved behind the navigation bar")
        otherTab.tap()

        let currentTab = app.buttons.matching(
            NSPredicate(format: "value == %@", "current")
        ).firstMatch
        let selected = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label ==[c] %@", "changes"), object: currentTab)
        XCTAssertEqual(
            XCTWaiter.wait(for: [selected], timeout: 5), .completed,
            "The changes tab did not become the current pane after the tap"
        )
        let changesSwitcher = app.buttons["Switch terminal"]
        let reviewOptions = app.buttons["Review options"]
        XCTAssertTrue(reviewOptions.waitForExistence(timeout: 2))
        XCTAssertGreaterThan(
            changesSwitcher.frame.minX, reviewOptions.frame.minX,
            "The worktree switcher was not the rightmost changes-pane control"
        )
        XCTAssertEqual(
            changesSwitcher.frame.maxX, initialSwitcherFrame.maxX, accuracy: 2,
            "The worktree switcher moved when the changes toolbar appeared"
        )
        let toolbarPost = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        toolbarPost.name = "worktree-button-order-and-transcript-type-post"
        toolbarPost.lifetime = .keepAlways
        add(toolbarPost)

        let originalTabAfterSwitch = app.buttons[originalTabIdentifier]
        XCTAssertTrue(originalTabAfterSwitch.waitForExistence(timeout: 2))
        originalTabAfterSwitch.tap()
        XCTAssertFalse(
            app.buttons["Review options"].exists,
            "The hidden changes pane left its review menu in the agent toolbar"
        )

        let tabPrefix = "terminal-tab-"
        XCTAssertTrue(otherTab.identifier.hasPrefix(tabPrefix))
        let terminalID = String(otherTab.identifier.dropFirst(tabPrefix.count))
        app.buttons["Switch terminal"].tap()
        let modalRow = app.buttons["fleet-terminal-\(terminalID)"]
        XCTAssertTrue(modalRow.waitForExistence(timeout: 3))
        XCTAssertTrue(modalRow.isHittable, "The terminal row was visible but not tappable")
        modalRow.tap()

        let selectedFromModal = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label ==[c] %@", "changes"),
            object: app.buttons.matching(
                NSPredicate(format: "value == %@", "current")
            ).firstMatch
        )
        let modalSelectionResult = XCTWaiter.wait(for: [selectedFromModal], timeout: 3)
        let currentLabel = app.buttons.matching(
            NSPredicate(format: "value == %@", "current")
        ).firstMatch.label
        XCTAssertEqual(
            modalSelectionResult, .completed,
            "Tapping a terminal in the modal did not navigate to it; current tab was \(currentLabel)"
        )
        XCTAssertFalse(app.buttons["Done"].exists, "The terminal modal did not dismiss")
    }
}
