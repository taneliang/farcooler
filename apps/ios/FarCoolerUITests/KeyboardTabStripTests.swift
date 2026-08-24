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

        // The Changes tab, by identifier rather than by label. Its label now
        // carries the diff's counts when there are any, and what they count —
        // "Changes, 82 added, 13 removed, including work that isn’t committed
        // yet" — so matching on the word alone found it only on a clean
        // workspace. It is also no longer a pane the host has to have opened:
        // every workspace has this chip. See `TerminalTabStrip`.
        let otherTab = app.buttons["workspace-tab-changes"]
        guard otherTab.waitForExistence(timeout: 5) else {
            throw XCTSkip("The current screen is not a workspace with a tab strip.")
        }
        guard otherTab.value as? String != "current" else {
            throw XCTSkip("The workspace opened on its Changes tab; nothing to switch to.")
        }
        let initialTabFrame = otherTab.frame
        let initialSwitcherFrame = app.buttons["Switch workspace"].frame
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

        let selected = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value == %@", "current"), object: otherTab)
        XCTAssertEqual(
            XCTWaiter.wait(for: [selected], timeout: 5), .completed,
            "The Changes tab did not become the current pane after the tap"
        )
        let changesSwitcher = app.buttons["Switch workspace"]
        let reviewOptions = app.buttons["Review options"]
        XCTAssertTrue(reviewOptions.waitForExistence(timeout: 2))
        XCTAssertGreaterThan(
            changesSwitcher.frame.minX, reviewOptions.frame.minX,
            "The workspace switcher was not the rightmost changes-pane control"
        )
        XCTAssertEqual(
            changesSwitcher.frame.maxX, initialSwitcherFrame.maxX, accuracy: 2,
            "The workspace switcher moved when the changes toolbar appeared"
        )
        let toolbarPost = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        toolbarPost.name = "workspace-button-order-and-transcript-type-post"
        toolbarPost.lifetime = .keepAlways
        add(toolbarPost)

        let originalTabAfterSwitch = app.buttons[originalTabIdentifier]
        XCTAssertTrue(originalTabAfterSwitch.waitForExistence(timeout: 2))
        originalTabAfterSwitch.tap()
        XCTAssertFalse(
            app.buttons["Review options"].exists,
            "The hidden changes pane left its review menu in the agent toolbar"
        )

        // The switcher sheet, which now hands its selection all the way back up
        // to `FleetView.show(_:)` rather than selecting inside the screen — a
        // pane in another workspace is a different screen. A pane in THIS one
        // comes straight back down and switches tabs without rebuilding
        // anything, which is what this checks.
        let tabPrefix = "terminal-tab-"
        let agentTab = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH %@ AND value != %@", tabPrefix, "current")
        ).firstMatch
        guard agentTab.waitForExistence(timeout: 3) else {
            throw XCTSkip("This workspace has no second agent pane to switch to.")
        }
        let terminalID = String(agentTab.identifier.dropFirst(tabPrefix.count))
        app.buttons["Switch workspace"].tap()
        let modalRow = app.buttons["fleet-terminal-\(terminalID)"]
        XCTAssertTrue(modalRow.waitForExistence(timeout: 3))
        XCTAssertTrue(modalRow.isHittable, "The terminal row was visible but not tappable")
        modalRow.tap()

        let selectedFromModal = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value == %@", "current"), object: agentTab)
        XCTAssertEqual(
            XCTWaiter.wait(for: [selectedFromModal], timeout: 3), .completed,
            "Tapping a terminal in the modal did not switch to its tab"
        )
        XCTAssertFalse(app.buttons["Done"].exists, "The terminal modal did not dismiss")
    }
}

/// What the agent transcript's scrolling has to do, checked against the layout
/// harness rather than against a runner.
///
/// Unlike the suite above, these run on a simulator: `AgentLayoutHarness` mounts
/// the real `AgentView` over a canned conversation, which is the only way this
/// screen can be exercised without an enrolled runner, a workspace and a turn in
/// flight. The transcript publishes its measurements as its accessibility value
/// in debug builds — see `AgentLayoutProbe` — and `inset` is the number the
/// reported bug was: the room the conversation leaves for the composer resting
/// on it.
final class AgentTranscriptScrollTests: XCTestCase {
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launch()
        return app
    }

    private func state(_ transcript: XCUIElement) -> [String: Int] {
        var parsed: [String: Int] = [:]
        for pair in (transcript.value as? String ?? "").split(separator: ";") {
            let halves = pair.split(separator: "=")
            guard halves.count == 2 else { continue }
            parsed[String(halves[0])] = Int(halves[1]) ?? (halves[1] == "true" ? 1 : 0)
        }
        return parsed
    }

    /// The reported bug: "text can be scrolled behind the message box and
    /// cannot be read... the scroll area seems to sometimes not have any inset
    /// area at the bottom."
    ///
    /// The composer is an `inputAccessoryView`, so its height is measured in the
    /// keyboard's window, which runs to the bottom of the screen — the
    /// home-indicator strip is inside that number. Counting the pane's own safe
    /// area on top of it reserved 196 points for a composer covering 162, and
    /// the conversation came to rest with its last row under the glass.
    func testTheConversationReservesExactlyTheComposerItHas() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        let measured = state(transcript)
        let covered = max(measured["keyboard"] ?? 0, measured["bar"] ?? 0)
        XCTAssertGreaterThan(covered, 0, "The composer never reported a height")
        XCTAssertEqual(
            measured["inset"] ?? -1, covered, accuracy: 1,
            "The transcript reserved \(measured["inset"] ?? -1) for a composer covering \(covered)")
    }

    /// It opens at the tail, and says so.
    func testItOpensAtTheTailWithNoWayBackOffered() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        XCTAssertEqual(state(transcript)["tail"], 1, "The transcript did not open at its tail")
        XCTAssertFalse(
            app.buttons["jump-to-latest"].exists,
            "A way back was offered to a reader who is already at the bottom")
    }

    /// Principle 4, which is the one a chat surface is judged on: a reader who
    /// has scrolled away is never dragged back. The keyboard is the event that
    /// used to do it — the viewport shortens before the scroll position
    /// catches up, and the transcript read that transient as the reader's own
    /// choice.
    func testTheKeyboardDoesNotDragAnUnpinnedReaderBack() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        transcript.swipeDown(velocity: .fast)
        transcript.swipeDown(velocity: .fast)
        XCTAssertEqual(state(transcript)["tail"], 0, "Scrolling away did not unpin the transcript")

        let back = app.buttons["jump-to-latest"]
        XCTAssertTrue(back.waitForExistence(timeout: 3), "No way back was offered")

        app.textViews.firstMatch.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        let stayedPut = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value ENDSWITH %@", "tail=false"), object: transcript)
        XCTAssertEqual(
            XCTWaiter.wait(for: [stayedPut], timeout: 3), .completed,
            "The keyboard pulled a reader who had scrolled away back to the tail")

        back.tap()
        let returned = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value ENDSWITH %@", "tail=true"), object: transcript)
        XCTAssertEqual(
            XCTWaiter.wait(for: [returned], timeout: 5), .completed,
            "Jump to Latest did not return the transcript to its tail")
    }

    /// Principle 3's other half. A message grown to several lines makes the
    /// composer taller, and the accessory used to keep the height one line
    /// measured: the draft is `@State` inside the composer, so typing never
    /// re-evaluated the view that measures the bar. SwiftUI drew four lines
    /// overflowing out of a bar UIKit still believed was one, and the transcript
    /// reserved room for the one.
    func testTypingAMultiLineMessageMakesRoomForIt() throws {
        let app = launch()
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30))
        let field = app.textViews.firstMatch
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        // The BAR's own number, not the keyboard's: with the keyboard up the
        // reported keyboard frame contains the bar as it was when the frame was
        // posted, so it is the bar's measurement that has to move.
        let oneLine = state(transcript)["bar"] ?? 0
        XCTAssertGreaterThan(oneLine, 0, "The composer never reported a height")

        app.typeText(
            "A message long enough to wrap onto five or six separate lines inside the "
            + "composer card so that the field grows well past its resting height")
        let grew = XCTNSPredicateExpectation(
            predicate: NSPredicate { object, _ in
                guard let value = (object as? XCUIElement)?.value as? String,
                    let bar = value.split(separator: ";").first(where: { $0.hasPrefix("bar=") }),
                    let height = Int(bar.dropFirst(4))
                else { return false }
                return height > oneLine
            }, object: transcript)
        XCTAssertEqual(
            XCTWaiter.wait(for: [grew], timeout: 5), .completed,
            "The composer grew and the bar went on reporting the height of one line: "
                + "\(String(describing: transcript.value))")

        let measured = state(transcript)
        XCTAssertEqual(
            measured["inset"] ?? -1,
            max(measured["keyboard"] ?? 0, measured["bar"] ?? 0), accuracy: 1,
            "The transcript did not make room for the composer it now has")
    }
}

/// What an agent pane with nothing in it is allowed to SAY.
///
/// The reported bug: "'Could not load this session' shows up for quite a long
/// time... when it's loading it shouldn't say 'could not load this session'."
/// It was one bit — is `connectionError` set — standing in for four different
/// true states, and three of the three things that set it are a pane still
/// trying. `AgentStream.Phase` and `AgentStream.Waited` are the distinction;
/// these are the assertions that it is real.
///
/// `-state` mounts one of those states over an empty transcript, which is the
/// only condition under which `AgentView.emptyState` draws at all — so before
/// the harness could reach them, none of these screens had ever been seen.
final class AgentEmptyStateTests: XCTestCase {
    private func launch(_ state: String, native: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments =
            ["-agent-layout-harness", "-empty-\(state)"] + (native ? ["-native"] : [])
        app.launch()
        return app
    }

    /// What the screen currently says, headline and sentence together.
    ///
    /// Every one of the five states has a headline under this identifier, which
    /// is the point: "what is this screen claiming" is one question and it is
    /// asked of one element whichever state is up.
    private func words(_ app: XCUIApplication, timeout: TimeInterval = 30) -> String {
        let title = app.staticTexts["agent-empty-title"]
        XCTAssertTrue(title.waitForExistence(timeout: timeout), "No empty state was drawn")
        let message = app.staticTexts["agent-empty-message"]
        return title.label + " " + (message.exists ? message.label : "")
    }

    /// THE REPORT. A pane whose agent has not started yet is not a pane that
    /// failed, and must not borrow one's words.
    func testALoadingSessionDoesNotWearADeadOnesSentence() {
        for state in ["opening", "starting", "waiting", "trying"] {
            let said = words(launch(state))
            XCTAssertFalse(
                said.localizedCaseInsensitiveContains("could not load"),
                "‘\(state)’ is still trying and called itself a failure: \(said)")
        }
    }

    /// And the other half, which is what makes it a distinction rather than a
    /// deletion: a wait that has genuinely gone on too long still says so.
    func testAWaitThatWentOnTooLongStillSaysItFailed() {
        XCTAssertTrue(
            words(launch("failed")).localizedCaseInsensitiveContains("could not load"),
            "A poll that has been failing past the alarm said nothing about it")
    }

    /// A spinner that never ends is its own bug: past `patience`, the waiting
    /// states name what they are waiting on instead of spinning silently.
    func testAWaitThatLingersExplainsItself() {
        XCTAssertTrue(
            words(launch("waiting")).localizedCaseInsensitiveContains("no agent"),
            "A pane that has been waiting a while did not say what it was waiting for")
    }

    /// "Say something to begin." belongs to the ONE state it was ever true for:
    /// a session that exists and has said nothing. It used to be what the
    /// screen showed before the first poll came back, inviting a message into a
    /// pane nothing knew anything about.
    func testTheInvitationBelongsToASessionThatActuallyExists() {
        XCTAssertTrue(words(launch("live")).localizedCaseInsensitiveContains("say something"))
        XCTAssertFalse(words(launch("opening")).localizedCaseInsensitiveContains("say something"))
    }

    /// Which protocol is carrying this chat, in the app's own words — the
    /// Mac's, which are `TileView`'s "Native" and "ACP".
    func testTheComposerNamesTheAdapterInUse() {
        let acp = XCUIApplication()
        acp.launchArguments = ["-agent-layout-harness", "-plain"]
        acp.launch()
        let acpBadge = acp.staticTexts["adapter-badge"]
        XCTAssertTrue(acpBadge.waitForExistence(timeout: 30), "No adapter badge on an ACP session")
        XCTAssertTrue(
            acpBadge.label.hasPrefix("ACP"),
            "An ACP session did not say ACP: \(acpBadge.label)")

        let native = XCUIApplication()
        native.launchArguments = ["-agent-layout-harness", "-plain", "-native"]
        native.launch()
        let nativeBadge = native.staticTexts["adapter-badge"]
        XCTAssertTrue(nativeBadge.waitForExistence(timeout: 30))
        XCTAssertTrue(
            nativeBadge.label.hasPrefix("Native"),
            "A native session did not say Native: \(nativeBadge.label)")
    }

    /// And it says nothing at all until a session has actually named one.
    /// `Transcript.backend` defaults to `acp`, so without the epoch gate this
    /// would announce a protocol for a pane nobody has heard from.
    func testNoBadgeBeforeASessionHasSaid() {
        let app = launch("starting")
        _ = words(app)
        XCTAssertFalse(
            app.staticTexts["adapter-badge"].exists,
            "A pane with no session named a protocol anyway")
    }
}
