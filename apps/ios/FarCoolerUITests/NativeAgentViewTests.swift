import XCTest

/// The conversation view of a terminal-mode claude pane (ov-373), on
/// `-native-agent-harness`: the real shell, `TerminalView`, `NativeSwitch`
/// and `NativeAgentView` over a canned runner reached through the client
/// core's own call path. No runner.
final class NativeAgentViewTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ flags: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"] + flags
        // A capture run's terminal theme, which the pane's scheme follows
        // (`TEST_RUNNER_FARCOOLER_CAPTURE_THEME`, as `-app.theme`). Unset,
        // the app's default.
        if let theme = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_THEME"] {
            app.launchArguments += ["-app.theme", theme]
        }
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The screen, to `FARCOOLER_CAPTURE_OUT/native-<name>.png` when that's
    /// set (as `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`): captures for a review,
    /// off in CI.
    private func capture(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("native-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// What the canned runner has been asked: `follows=N sent=a|b`.
    private func harness(_ app: XCUIApplication) -> String {
        element(app, "native-harness").value as? String ?? ""
    }

    private func follows(_ app: XCUIApplication) -> Int {
        let said = harness(app)
        guard let range = said.range(of: #"follows=(\d+)"#, options: .regularExpression) else { return -1 }
        return Int(said[range].dropFirst("follows=".count)) ?? -1
    }

    /// Which of the pane's two views shows: `conversation`, `terminal`, or
    /// `terminal-only` where the conversation isn't offered, then
    /// `left=N`, the times the conversation stopped showing.
    private func showing(_ app: XCUIApplication) -> String {
        element(app, "native-showing").value as? String ?? ""
    }

    private func conversation(_ app: XCUIApplication) -> XCUIElement {
        let transcript = element(app, "native-transcript")
        XCTAssertTrue(transcript.waitForExistence(timeout: 60), "the conversation never showed")
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("conversation") }, "the pane shows \(showing(app))")
        return transcript
    }

    /// Type into the box once it holds the keyboard: on a loaded runner the
    /// focus lands well after the tap, and typing before it is lost.
    private func type(_ app: XCUIApplication, _ words: String) {
        let field = element(app, "native-composer")
        XCTAssertTrue(field.waitForExistence(timeout: 60))
        // Tapped again every 5 s while it hasn't the keyboard: a tap just
        // after launch on CI's runner can come to nothing (PR 7's first
        // run: one tap, then 30 s without focus).
        var tapped = Date.distantPast
        XCTAssertTrue(
            wait(60) {
                if field.value(forKey: "hasKeyboardFocus") as? Bool == true { return true }
                if Date().timeIntervalSince(tapped) >= 5 {
                    field.tap()
                    tapped = Date()
                }
                return false
            }, "the box never took the keyboard")
        app.typeText(words)
    }

    private func send(_ app: XCUIApplication, _ words: String) {
        type(app, words)
        element(app, "native-send").tap()
    }

    /// R-27: a phone opens a claude pane on its conversation, and switching
    /// to the terminal and back keeps the draft and never builds the
    /// terminal again.
    func testTheSwitchKeepsTheDraftAndTheTerminal() {
        let app = launch()
        _ = conversation(app)
        // The terminal's key sink is the one text view on the pane.
        Thread.sleep(forTimeInterval: 2)
        XCTAssertEqual(
            app.textViews.matching(NSPredicate(format: "hasKeyboardFocus == true")).count, 0,
            "the hidden terminal took the keyboard on arrival")
        let mount = element(app, "native-terminal-mount")
        let mounted = mount.value as? String
        XCTAssertNotNil(mounted)

        XCTAssertFalse(app.buttons["Send an image"].exists, "Send an Image would type into the covered terminal")
        type(app, "Half a thought")
        capture("draft")

        element(app, "native-switch").tap()
        XCTAssertTrue(element(app, "terminal-surface").waitForExistence(timeout: 30), "Show Terminal didn't show it")
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("terminal ") }, "the conversation still shows")
        XCTAssertTrue(app.buttons["Send an image"].waitForExistence(timeout: 30), "the terminal's own controls didn't come back")
        capture("terminal")

        element(app, "native-switch").tap()
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("conversation") })
        XCTAssertEqual(element(app, "native-composer").value as? String, "Half a thought", "the draft was lost")
        XCTAssertEqual(mount.value as? String, mounted, "switching built the terminal again")
    }

    /// A codex pane (ov-416) opens on its conversation where the runner says
    /// it serves codex, its box and its refusals naming Codex, with no Stop
    /// while it works: the runner presses keys in claude alone.
    func testACodexPaneOffersItsConversation() {
        let app = launch(["-native-codex", "-native-busy"])
        _ = conversation(app)
        let box = element(app, "native-composer")
        XCTAssertTrue(box.waitForExistence(timeout: 30))
        XCTAssertEqual(box.label, "Message Codex", "the box doesn't name Codex")
        XCTAssertFalse(element(app, "native-stop").exists, "Stop in a codex pane")
        send(app, "look at @src")
        let said = element(app, "native-send-issue")
        XCTAssertTrue(said.waitForExistence(timeout: 30), "the refusal wasn't said")
        XCTAssertTrue(
            app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Codex would open a picker'")).firstMatch.waitForExistence(timeout: 30),
            "the picker refusal doesn't name Codex")
        capture("codex")
    }

    /// The same pane on a runner from before `codex_view`: the terminal, and
    /// no switch.
    func testACodexPaneOnAnOlderRunnerShowsItsTerminal() {
        let app = launch(["-native-codex-before"])
        XCTAssertTrue(element(app, "terminal-surface").waitForExistence(timeout: 60))
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("terminal-only") }, "the pane shows \(showing(app))")
        XCTAssertFalse(element(app, "native-switch").exists)
    }

    /// The runner changes a row: the same row, drawn again with its new
    /// words, and no second one.
    func testAFollowUpdatesARowInPlace() {
        let app = launch()
        _ = conversation(app)
        XCTAssertTrue(app.staticTexts["Reading the parser now."].waitForExistence(timeout: 30))
        // The runner changes the reply only now, having been seen as it was.
        send(app, "change the reply")
        XCTAssertTrue(wait(60) { harness(app).contains("changed=true") }, "the runner never changed the row: \(harness(app))")
        let updated = app.staticTexts["Read the parser. It’s tidy now: three functions, no globals."]
        XCTAssertTrue(updated.waitForExistence(timeout: 30), "the follow's change never reached the row")
        XCTAssertFalse(app.staticTexts["Reading the parser now."].exists, "the old words stayed")
        XCTAssertEqual(
            app.descendants(matching: .any).matching(identifier: "native-row-prose:a1:0").count, 1,
            "the change arrived as a second row")
        capture("conversation")
    }

    /// A message goes through `terminal.compose`, the box empties, and the
    /// message comes into view above the composer when the runner shows it.
    func testComposeSends() {
        let app = launch()
        _ = conversation(app)
        let words = "Now the lexer, please"
        send(app, words)
        XCTAssertTrue(wait(30) { harness(app).contains("sent=\(words)") }, "compose never reached the runner: \(harness(app))")
        XCTAssertTrue(wait(30) { (element(app, "native-composer").value as? String ?? "").isEmpty || element(app, "native-composer").value as? String == "Message Claude" })
        let shown = app.staticTexts[words]
        XCTAssertTrue(shown.waitForExistence(timeout: 30), "the sent turn never showed")
        let composerTop = element(app, "native-composer-stack").frame.minY
        XCTAssertTrue(
            wait(30) { shown.isHittable && shown.frame.maxY <= element(app, "native-composer-stack").frame.minY },
            "the sent message ends at \(shown.frame.maxY), under the composer at \(composerTop)")
        capture("sent")
    }

    /// R-29: claude is working, its queue takes the message, and the
    /// conversation says Queued.
    func testBusyShowsQueued() {
        let app = launch(["-native-busy"])
        _ = conversation(app)
        send(app, "After this, the tests")
        let queued = element(app, "native-queued")
        XCTAssertTrue(queued.waitForExistence(timeout: 30), "no Queued row")
        XCTAssertTrue(queued.staticTexts["Queued"].exists, "\(queued.debugDescription)")
        XCTAssertTrue(queued.staticTexts["After this, the tests"].exists, "\(queued.debugDescription)")
        capture("queued")
    }

    /// A dialog in the terminal refuses the send and hands off, and Show
    /// Terminal shows it.
    func testADialogShowsHandoff() {
        let app = launch(["-native-dialog"])
        _ = conversation(app)
        send(app, "Anything")
        XCTAssertTrue(element(app, "native-handoff").waitForExistence(timeout: 30), "no Handoff row")
        capture("handoff")
        element(app, "native-handoff-show-terminal").tap()
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("terminal ") }, "Show Terminal didn't show it")
    }

    /// R-28: a draft in the terminal's box refuses, with Show Terminal.
    func testADraftInTheTerminalRefusesWithShowTerminal() {
        let app = launch(["-native-draft"])
        _ = conversation(app)
        send(app, "Anything")
        let issue = element(app, "native-send-issue")
        XCTAssertTrue(issue.waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts["The terminal’s box already holds a draft. Send or clear it there first."].exists)
        issue.buttons["Show Terminal"].tap()
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("terminal ") }, "Show Terminal didn't show it")
    }

    /// A runner whose projector is off: the terminal, and no switch.
    func testFlagOffShowsTheTerminal() {
        let app = launch(["-native-flag-off"])
        XCTAssertTrue(element(app, "terminal-surface").waitForExistence(timeout: 60), "no terminal")
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(element(app, "native-switch").exists, "a switch to a view the runner doesn't serve")
        XCTAssertTrue(showing(app).hasPrefix("terminal-only"), showing(app))
        XCTAssertFalse(element(app, "native-transcript").exists)
        XCTAssertEqual(follows(app), 0, "rows read from a runner that doesn't serve them")
    }

    /// A turn nobody typed, a background task finishing, is a notice line,
    /// never the person's message.
    func testNoticeTurnsRenderAsNotices() {
        let app = launch()
        _ = conversation(app)
        let notice = element(app, "native-notice-turn")
        XCTAssertTrue(notice.waitForExistence(timeout: 30), "no notice turn")
        XCTAssertEqual(notice.label, "Agent Count the lines finished")
        let prompts = app.descendants(matching: .any).matching(identifier: "native-prompt")
        XCTAssertFalse(
            prompts.allElementsBoundByIndex.contains { $0.label.contains("Count the lines") },
            "a notification drawn as a message the person typed")
        XCTAssertTrue(prompts.allElementsBoundByIndex.contains { $0.label == "Tidy the parser, please." })
        XCTAssertFalse(
            element(app, "native-row-turn:n1").staticTexts["Took 0:42"].exists,
            "a notice says a time as if it were a message sent")
    }

    /// A link coming up again leaves the build unread for a round trip: the
    /// conversation stays, and so does the keyboard on its box.
    func testAReconnectKeepsTheConversationAndTheKeyboard() {
        let app = launch(["-native-reconnect"])
        _ = conversation(app)
        type(app, "Mid-sentence")
        let field = element(app, "native-composer")
        XCTAssertTrue(wait(60) { harness(app).contains("linked=1") }, "the reconnect never finished: \(harness(app))")
        XCTAssertTrue(showing(app).hasPrefix("conversation left=0 "), "the conversation came down for the reconnect: \(showing(app))")
        XCTAssertEqual(field.value as? String, "Mid-sentence")
        XCTAssertTrue(field.value(forKey: "hasKeyboardFocus") as? Bool == true, "the box lost the keyboard")
    }

    /// The setting turned off and on again: the pane shows its terminal while
    /// it's off, and follows again once it's on, with no relaunch.
    func testOffThenOnFollowsAgain() {
        let app = launch(["-native-off-on"])
        _ = conversation(app)
        // Off shows the terminal with no switch, which the probe counts, so a
        // slow query can't miss it; the runner turns rows on again only then.
        XCTAssertTrue(wait(60) { showing(app).contains("dropped=1") }, "off didn't show the terminal: \(showing(app))")
        XCTAssertTrue(wait(60) { showing(app).hasPrefix("conversation") }, "on didn't bring the conversation back: \(showing(app))")
        let back = follows(app)
        XCTAssertTrue(wait(30) { follows(app) >= back + 2 }, "it never followed again: \(harness(app))")
        XCTAssertFalse(element(app, "native-stale").exists, "still says it isn't being read")
    }

    /// The conversation coming to cover a terminal that holds the keyboard,
    /// other than by the switch (here, the runner's projector coming on as
    /// the person types in the terminal): the terminal lets the keyboard go,
    /// so keys never go on into a terminal nobody can see.
    func testCoveringTheTerminalTakesItsKeyboardAway() {
        let app = launch(["-native-on-later"])
        XCTAssertTrue(wait(60) { showing(app).hasPrefix("terminal-only") }, showing(app))
        let sinkFocused = { app.textViews.matching(NSPredicate(format: "hasKeyboardFocus == true")).count > 0 }
        if !sinkFocused() { element(app, "terminal-surface").tap() }
        XCTAssertTrue(wait(30) { sinkFocused() }, "the terminal never took the keyboard")
        app.typeText("claude")
        XCTAssertTrue(wait(60) { showing(app).hasPrefix("conversation") }, "the conversation never came: \(showing(app)) \(harness(app))")
        XCTAssertTrue(wait(30) { !sinkFocused() }, "the covered terminal kept the keyboard")
    }

    /// Rows held and the runner not answering: said over them, and Send waits.
    func testStaleRowsSaySoAndSendWaits() {
        let app = launch(["-native-stale"])
        _ = conversation(app)
        let banner = element(app, "native-stale")
        // Four follows in, then the first retry's failure: slow under load.
        XCTAssertTrue(banner.waitForExistence(timeout: 40), "no stale banner: \(harness(app))")
        XCTAssertTrue(app.staticTexts["Can’t reach the runner, so this may be out of date. Trying again…"].exists)
        type(app, "Anything")
        XCTAssertFalse(element(app, "native-send").isEnabled, "Send went on over stale rows")
        capture("stale")
    }

    /// A send that may have arrived never says it wasn't sent; a device that
    /// may not type is told so.
    func testSendFailuresSayWhatTheyMean() {
        let app = launch()
        _ = conversation(app)
        let mayHave = "The runner didn’t answer in time. The message may have been sent, so check the terminal before sending it again."
        for words in ["time out", "garble"] {
            send(app, words)
            XCTAssertTrue(app.staticTexts[mayHave].waitForExistence(timeout: 30), "\(words): \(app.staticTexts.allElementsBoundByIndex.map(\.label))")
            element(app, "native-send-issue").buttons["Dismiss"].tap()
            XCTAssertTrue(wait(30) { !app.staticTexts[mayHave].exists })
            type(app, String(repeating: XCUIKeyboardKey.delete.rawValue, count: words.count))
        }
        send(app, "read only")
        XCTAssertTrue(app.staticTexts["This device can’t send messages to this runner."].waitForExistence(timeout: 30))
    }

    /// The follow runs only while the conversation is on screen: not behind
    /// the terminal, and not with the app in the background.
    func testTheFollowStopsOffScreen() {
        let app = launch()
        _ = conversation(app)
        XCTAssertTrue(wait(30) { follows(app) >= 2 }, harness(app))

        element(app, "native-switch").tap()
        XCTAssertTrue(wait(30) { showing(app).hasPrefix("terminal ") })
        Thread.sleep(forTimeInterval: 1)
        let behind = follows(app)
        Thread.sleep(forTimeInterval: 3)
        XCTAssertLessThanOrEqual(follows(app), behind, "still following behind the terminal")

        element(app, "native-switch").tap()
        XCTAssertTrue(wait(30) { follows(app) > behind }, "didn't follow again on return")

        XCUIDevice.shared.press(.home)
        Thread.sleep(forTimeInterval: 5)
        app.activate()
        XCTAssertTrue(conversation(app).waitForExistence(timeout: 30))
        XCTAssertTrue(harness(app).contains("background=0"), "followed in the background: \(harness(app))")
        XCTAssertTrue(wait(30) { follows(app) > behind + 1 }, "didn't follow again in front")
    }
}
