import XCTest

/// A member's writes carry their name (ov-707), over the canned runner: bil-7's
/// record on a shared runner (`-phone-shared -phone-member-notes`), read on
/// Bob's phone and on the owner's (`-phone-rep-owner`). Which word each actor
/// reads as is AgentKit's, pinned in `TaskBoardModelTests`. Decided For You's
/// Keep, which a member was offered too (ov-736), left the phone's Plan with
/// ov-874.
final class MemberBylinesUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    /// The first text on screen containing `words`.
    private func text(_ app: XCUIApplication, containing words: String) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", words)).firstMatch
    }

    private func record(owner: Bool) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-push-task", "bil-7", "-phone-shared", "-phone-member-notes"] + (owner ? ["-phone-rep-owner"] : []))
        let alexandra = text(app, containing: "Alexandra Montgomery-Fitzgerald")
        XCTAssertTrue(app.reveal(alexandra), "no byline with Alexandra's name: \(app.debugDescription)")
        return app
    }

    func testAMembersNoteShowsTheirNameAndTheOwnersReadsOwner() {
        let app = record(owner: false)
        XCTAssertTrue(app.reveal(text(app, containing: "· Owner ·")), "the owner's comment isn't signed Owner")
        XCTAssertTrue(app.reveal(text(app, containing: "· You ·")), "Bob's own comment isn't signed You")
    }

    /// Light, on Bob's phone and the owner's. Opt-in: `TEST_RUNNER_FC_CAPTURES=1`.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take them")
        XCUIDevice.shared.appearance = .light
        for (name, owner) in [("member", false), ("owner", true)] {
            let record = record(owner: owner)
            keep(record, "member-record-\(name)-light", expecting: .shows(text(record, containing: "Alexandra")))
            record.terminate()
        }
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
