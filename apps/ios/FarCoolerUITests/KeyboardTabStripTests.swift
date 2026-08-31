import XCTest

/// The shell's bar has to survive the keyboard, and still answer a finger
/// afterwards.
///
/// This was `testTabStripRemainsInteractiveAfterKeyboardDismissal`, written
/// against `TerminalTabStrip` — a floating strip of chips under a navigation
/// bar, in a `WorkspaceView`. Neither exists. What the regression was ABOUT
/// does: an input accessory lives in the keyboard's window, and a pane host
/// that lets the keyboard into its own safe area gets its furniture shoved up
/// the screen and then left there. The shell's answer is
/// `ShellRootView.body`'s full-height stack and every pane's
/// `.ignoresSafeArea(.keyboard, edges: .bottom)`, and this is the assertion
/// that they hold.
///
/// So the same five moments, against the surfaces that carry them now:
///
/// | Then | Now |
/// | --- | --- |
/// | the tab strip's frame | `shell-bar`'s frame |
/// | `workspace-tab-changes`'s `value == "current"` | `shell-state`'s `tab=` |
/// | a lift is a tap on a chip | a lift on the bar, which is how a tab is chosen |
/// | the switcher sheet's `fleet-terminal-<id>` row | an overview card, `shell-card-<id>` |
///
/// **Two of the old assertions have no equivalent and are not replaced.** The
/// changes pane's "Review options" menu and the "Switch workspace" button were
/// items in `WorkspaceView`'s navigation-bar toolbar, and the test checked
/// their ORDER within it — that the switcher stayed rightmost as the contextual
/// menu came and went. There is no navigation bar over a pane any more and
/// neither control has a host, so there is no order left to be wrong. That is a
/// gap in the shell rather than in this test; see `ChangesToolbarMenu`, which
/// is still in the tree and is currently mounted by nothing.
final class KeyboardTabStripTests: XCTestCase {
    func testTheBarRemainsInteractiveAfterKeyboardDismissal() throws {
        #if targetEnvironment(simulator)
        throw XCTSkip("This regression needs a real iPhone with a configured workspace.")
        #endif

        let app = XCUIApplication()
        app.launch()

        let composer = app.textViews.firstMatch
        guard composer.waitForExistence(timeout: 10) else {
            throw XCTSkip("The current device state has no agent composer to exercise.")
        }

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        guard bar.waitForExistence(timeout: 5) else {
            throw XCTSkip("The current screen is not the shell.")
        }
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 5) else {
            throw XCTSkip("The shell never reported where it was.")
        }
        func field(_ name: String) -> Int? {
            (probe.value as? String ?? "").split(separator: " ")
                .first { $0.hasPrefix("\(name)=") }
                .flatMap { Int($0.split(separator: "=")[1]) }
        }
        let home = try XCTUnwrap(field("tab"))
        let initialBarFrame = bar.frame

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
        XCTAssertEqual(
            bar.frame.minY, initialBarFrame.minY, accuracy: 2,
            "The bar moved when the keyboard appeared"
        )

        app.swipeDown()
        let keyboardDismissed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == NO"),
            object: app.keyboards.firstMatch
        )
        XCTAssertEqual(XCTWaiter.wait(for: [keyboardDismissed], timeout: 5), .completed)

        XCTAssertEqual(
            bar.frame.minY, initialBarFrame.minY, accuracy: 2,
            "The bar moved after the keyboard disappeared"
        )
        let transcriptInsetReset = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value BEGINSWITH %@", "keyboard=0;"),
            object: transcript
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [transcriptInsetReset], timeout: 2), .completed,
            "The transcript retained the keyboard-height bottom inset after dismissal: \(String(describing: transcript.value))"
        )
        XCTAssertTrue(bar.isHittable, "The bar ended up somewhere nothing can touch")

        // And it still ANSWERS, which is the half the frame check cannot make.
        // A lift onto the first row is how a tab is chosen in the shell — the
        // column unfurls one row per 34 points, and the first row is the diff,
        // which every workspace has. That is the same move the Changes chip
        // used to be.
        try XCTSkipUnless(home != 0, "This pane is already the first tab; nothing to move to.")
        lift(bar, by: 20)
        let landed = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in field("tab") == 0 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [landed], timeout: 5), .completed,
            "A lift on the bar after the keyboard had come and gone chose nothing: "
                + "\(probe.value ?? "")"
        )

        let toolbarPost = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        toolbarPost.name = "shell-bar-after-keyboard"
        toolbarPost.lifetime = .keepAlways
        add(toolbarPost)

        // The cross-worktree jump, which used to be the switcher sheet in the
        // toolbar and is the overview now. Same shape of assertion: reach every
        // workspace on the runner from inside a pane, and land in one.
        try XCTSkipUnless(
            (field("workspaces") ?? 0) > 1, "One workspace: there is nowhere to cross to.")
        let ws = try XCTUnwrap(field("ws"))
        lift(bar, by: 320)
        let opened = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in field("overview") == 1 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [opened], timeout: 5), .completed,
            "The long lift never reached the overview: \(probe.value ?? "")")

        let other = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "shell-card-")
        ).allElementsBoundByIndex.first { $0.isHittable }
        let card = try XCTUnwrap(other, "The overview drew no cards")
        card.tap()
        let arrived = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in field("overview") == 0 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [arrived], timeout: 5), .completed,
            "Tapping a card did not leave the overview: \(probe.value ?? "")")
        XCTAssertNotNil(field("ws"), "The shell stopped reporting where it was")
        _ = ws
    }

    /// Lift the bar by `points`, and hold there so the column is at that height
    /// when the finger leaves. The same helper `ShellGestureTests` uses, and it
    /// has to stay the same: a lift that releases in the frame of its last
    /// movement can leave the final translation unreported.
    private func lift(_ bar: XCUIElement, by points: CGFloat) {
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = from.withOffset(CGVector(dx: 0, dy: -points))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.5)
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

/// A session that goes away with a conversation still on the screen.
///
/// Android's port of the loading-state fix found the case iOS did not handle
/// (`83b1682`): the epoch is the daemon's own answer to whether a session
/// exists, and this app asked it only where the transcript was EMPTY. So a shim
/// that went away mid-conversation replied epoch 0 with rows on screen, the pane
/// stayed `.live`, and it went on inviting a message into a session that had
/// just ended — one `AgentSupervisor::send` drops on the floor, because there is
/// no shim registered to receive it, while `terminal.agent_prompt` still answers
/// OK. The message would have been echoed into the transcript looking sent.
///
/// `-ended` is that state: the canned conversation, with the daemon answering
/// epoch 0 underneath it. What it has to show is both halves — the conversation
/// is still readable, AND nothing can be sent into it.
final class AgentEndedSessionTests: XCTestCase {
    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness"] + arguments
        app.launch()
        return app
    }

    /// Typing into the pane and finding Send dead is the assertion, because
    /// with nothing typed Send is dead anyway — that is the composer's own
    /// rule and it would have passed against the bug.
    private func sendIsOffered(in app: XCUIApplication) -> Bool {
        let transcript = app.scrollViews["agent-transcript"]
        XCTAssertTrue(transcript.waitForExistence(timeout: 30), "No transcript was drawn")
        let field = app.textViews.firstMatch
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        app.typeText("Anything at all")
        let send = app.buttons["agent-send"]
        XCTAssertTrue(send.waitForExistence(timeout: 5), "The composer had no Send button")
        return send.isEnabled
    }

    func testAConversationOutlivesTheSessionThatProducedIt() {
        let app = launch(["-ended"])
        XCTAssertTrue(
            app.scrollViews["agent-transcript"].waitForExistence(timeout: 30),
            "The conversation was taken away with the session that produced it")
        XCTAssertTrue(
            app.staticTexts["agent-session-ended"].waitForExistence(timeout: 5),
            "A pane with no session behind it said nothing about it")
        XCTAssertFalse(
            sendIsOffered(in: app),
            "A message was offered into a session that had already ended")
    }

    /// The other half, which is what makes it a distinction rather than a
    /// deletion: a session that IS being served still takes messages.
    func testASessionThatStillExistsStillTakesMessages() {
        let app = launch(["-plain"])
        XCTAssertTrue(sendIsOffered(in: app), "A live session refused a typed message")
        XCTAssertFalse(
            app.staticTexts["agent-session-ended"].exists,
            "A live session was announced as ended")
    }

    /// The queue is the same drop, one surface up.
    ///
    /// `terminal.agent_steer_queued`, `terminal.agent_edit_queued` and
    /// `terminal.agent_cancel_queued` are each a single `svc.agents().send(…)`
    /// in `crates/daemon/src/rpc.rs`, and that call finds no writer for a
    /// terminal with no shim and returns having done nothing. Remove is the one
    /// that looks like it should survive and does not: the queue lives in the
    /// shim, and a client only ever RECEIVES it as `promptQueue`, so with the
    /// shim gone there is nothing to remove it from and nothing that would ever
    /// report it removed.
    ///
    /// Note the fixture: `-plain` filters `promptQueue` out, so the live half
    /// runs on the default conversation rather than on that flag.
    func testTheQueueStopsOfferingActionsNothingCanCarryOut() {
        let app = launch(["-ended"])
        let state = app.staticTexts["agent-queued-state"]
        XCTAssertTrue(state.waitForExistence(timeout: 30), "The queued message was taken away")
        XCTAssertEqual(
            state.label, "Not sent",
            "A message nothing can send was still described as queued")
        for action in ["Send now", "Edit", "Remove"] {
            XCTAssertFalse(
                app.buttons[action].exists,
                "\(action) was offered on a queue nothing is reading")
        }
    }

    func testALiveSessionKeepsAllThreeQueueActions() {
        let app = launch([])
        let state = app.staticTexts["agent-queued-state"]
        XCTAssertTrue(state.waitForExistence(timeout: 30), "The queued message was not drawn")
        XCTAssertEqual(state.label, "Queued", "A live queue did not say it was queued")
        for action in ["Send now", "Edit", "Remove"] {
            XCTAssertTrue(
                app.buttons[action].exists,
                "\(action) went missing from a queue that can still be acted on")
        }
    }
}
