import XCTest

/// A half-written message outlives the process that was writing it.
///
/// **The only irreversible thing a pane's teardown costs.** The panes are tmux
/// sessions on the runner, so nothing there ends: the scrollback refetches, the
/// transcript is on disk, and the workspace is where it was. The words in the
/// composer exist nowhere but this phone, and until `PaneDraftStore` they went
/// with the pane — or with the process, which on a phone is killed in a pocket
/// with no warning. `docs/jobs-to-be-done.md` F4 states the requirement in
/// those terms; Android has met it since `WorkspaceScreen`'s
/// `SaveableStateHolder` landed, and this is the phone that had not.
///
/// **Needs no runner.** It stands on `-agent-layout-harness`, whose fixture is
/// built in the app, so this suite cannot skip itself green when the demo
/// daemon is down — the same property `TerminalLigatureTests` and
/// `RunnerReachTests` are in the source list for.
///
/// `app.terminate()` and a second `launch()` is the whole mechanism: the two
/// launches share one `UserDefaults` container, so what survives between them
/// is what would survive the system killing the app in the background. There
/// is no way to ask XCUITest for a real jetsam kill, and there does not need to
/// be — what is under test is that the words were WRITTEN DOWN, not which
/// signal ended the process.
final class AgentDraftTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    /// The harness forgets drafts on the way in unless asked not to, so a
    /// launch that wants to READ one has to say so. See
    /// `AgentLayoutHarness.stand()`.
    private func launch(keepingDrafts: Bool) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments =
            ["-agent-layout-harness", "-plain"] + (keepingDrafts ? ["-keep-drafts"] : [])
        app.launch()
        return app
    }

    /// What the field reads, with an absent value spelled as an empty one.
    ///
    /// A `UITextView` with nothing in it hands back nil rather than "", and the
    /// two mean the same thing here — the placeholder above it is a separate
    /// SwiftUI `Text`, so nothing else can be occupying this value.
    private func text(of element: XCUIElement) -> String {
        (element.value as? String) ?? ""
    }

    private func composer(in app: XCUIApplication) throws -> XCUIElement {
        let composer = app.textViews.firstMatch
        guard composer.waitForExistence(timeout: 30) else {
            XCTFail("The agent pane drew no composer")
            throw XCTSkip("no composer")
        }
        return composer
    }

    /// The whole feature, in one journey: type, be killed, come back to it.
    func testAHalfWrittenMessageComesBackAfterTheAppIsKilled() throws {
        let typed = "the thought I was halfway through"

        let first = launch(keepingDrafts: false)
        let field = try composer(in: first)
        field.tap()
        first.typeText(typed)
        // Read it back before killing the app, so a failure below is about
        // persistence rather than about the keystrokes never landing — two
        // very different bugs that a single assertion after the relaunch
        // reports identically. Waited on rather than read once: the field is a
        // `UITextView` behind a SwiftUI binding, and its accessibility value
        // settles a frame after the keystrokes.
        XCTAssertTrue(
            waitFor(field, toRead: typed, timeout: 10),
            "The composer did not take the typing at all, so nothing was persisted: "
                + "\(text(of: field))")

        first.terminate()

        let second = launch(keepingDrafts: true)
        let restored = try composer(in: second)
        XCTAssertTrue(
            waitFor(restored, toRead: typed, timeout: 10),
            "The pane came back with an empty composer: the half-written message was lost, "
                + "which is the one thing a teardown cannot give back")
    }

    /// Sending is not losing, and the store has to tell the two apart.
    ///
    /// A draft cleared because the message WENT must not come back on the next
    /// launch — otherwise every message anybody sends is re-typed into the
    /// field the next time they open that pane.
    func testASentMessageDoesNotComeBackAsADraft() throws {
        let first = launch(keepingDrafts: false)
        let field = try composer(in: first)
        field.tap()
        first.typeText("this one goes")

        let send = first.buttons["agent-send"]
        XCTAssertTrue(send.waitForExistence(timeout: 10), "The composer offered no Send")
        send.tap()
        // The field empties as the message leaves; that emptying is what
        // removes the stored draft.
        XCTAssertTrue(
            waitFor(field, toRead: "", timeout: 10),
            "Send left the message in the field: \(text(of: field))")

        first.terminate()

        let second = launch(keepingDrafts: true)
        let restored = try composer(in: second)
        XCTAssertEqual(
            text(of: restored), "",
            "A message that was SENT came back as a draft")
    }

    /// The rule every other test in this suite depends on: a harness launch
    /// starts from the fixture and not from whatever the last test typed.
    ///
    /// Without this the failure is not a red test, it is a slow poisoning —
    /// `AgentTranscriptScrollTests.testTypingAMultiLineMessageMakesRoomForIt`
    /// measures the composer's resting height before typing a word, and a
    /// composer restored two lines tall makes that baseline a different number
    /// depending on which tests ran first.
    func testTheHarnessStartsWithAnEmptyComposer() throws {
        let first = launch(keepingDrafts: false)
        let field = try composer(in: first)
        field.tap()
        first.typeText("left behind by the previous test")
        first.terminate()

        let second = launch(keepingDrafts: false)
        let fresh = try composer(in: second)
        XCTAssertEqual(
            text(of: fresh), "",
            "The harness inherited the last launch's typing: \(text(of: fresh))")
    }

    /// `value` settles a frame or two after the tap that changes it, so this
    /// waits rather than reading once.
    private func waitFor(
        _ element: XCUIElement, toRead expected: String, timeout: TimeInterval
    ) -> Bool {
        let matched = XCTNSPredicateExpectation(
            predicate: NSPredicate { object, _ in
                ((object as? XCUIElement)?.value as? String ?? "") == expected
            }, object: element)
        return XCTWaiter.wait(for: [matched], timeout: timeout) == .completed
    }
}
