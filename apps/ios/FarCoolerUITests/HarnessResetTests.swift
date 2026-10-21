import XCTest

/// The harnesses start from clean drafts (ov-672). A second pane's draft text,
/// photo, sent-image cache and row cache are seeded (`-harness-seed-stores`),
/// the app is launched again, and they are gone; launched with
/// `-native-keep-photos` they stay, but for the row cache, which goes either way.
/// Run for both harnesses, because each one used to clear the panes it knew.
final class HarnessResetTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private static let seeded = "text=1 pane=1 worktree=1 photos=1 sent=2 rows=1"
    private static let cleared = "text=0 pane=0 worktree=0 photos=0 sent=0 rows=0"
    private static let kept = "text=1 pane=1 worktree=1 photos=1 sent=2 rows=0"

    private enum Harness { case native, phone }

    private func launch(_ harness: Harness, _ flags: [String]) -> XCUIApplication {
        switch harness {
        case .phone:
            return XCUIApplication.phoneHarness(flags)
        case .native:
            let app = XCUIApplication()
            app.launchArguments = ["-native-agent-harness"] + flags
            app.launchDrawn()
            XCTAssertTrue(
                app.descendants(matching: .any)["native-transcript"].waitForExistence(timeout: 60),
                "the conversation never showed")
            return app
        }
    }

    /// What the second pane holds, read once the probe has looked.
    private func stores(_ app: XCUIApplication) -> String {
        let probe = app.descendants(matching: .any)["harness-stores"]
        XCTAssertTrue(probe.waitForExistence(timeout: 60), "no stores probe")
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline, (probe.value as? String ?? "").isEmpty { Thread.sleep(forTimeInterval: 0.25) }
        return probe.value as? String ?? ""
    }

    private func check(_ harness: Harness) {
        let seeding = launch(harness, ["-harness-seed-stores"])
        XCTAssertEqual(stores(seeding), Self.seeded, "the seed didn't land, so the test below proves nothing")
        seeding.terminate()

        let clean = launch(harness, [])
        XCTAssertEqual(stores(clean), Self.cleared, "a relaunch kept a second pane's drafts")
        clean.terminate()

        let seededAgain = launch(harness, ["-harness-seed-stores"])
        XCTAssertEqual(stores(seededAgain), Self.seeded)
        seededAgain.terminate()

        let keeping = launch(harness, ["-native-keep-photos"])
        XCTAssertEqual(stores(keeping), Self.kept, "the opt-out didn't keep the drafts")
        keeping.terminate()
    }

    func testTheNativeHarnessClearsEveryPanesDrafts() { check(.native) }

    func testThePhoneHarnessClearsEveryPanesDrafts() { check(.phone) }
}
