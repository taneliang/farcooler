import XCTest

/// A harness that did not stand up, thrown where a test used to skip.
///
/// The harnesses (`-shell-harness`, `-agent-layout-harness`, `-phone-harness`)
/// are canned and in-process: nothing outside the app can make one fail to
/// draw. So a probe that never appears is the app failing, and an `XCTSkip`
/// there turned exactly the regression the test guards into a skipped test —
/// which xcodebuild reports as a success (ov-127). Thrown, any error other
/// than `XCTSkip` fails the test with this description.
struct HarnessFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

extension XCUIApplication {
    /// How long the first frame may take. A first launch after an install has
    /// taken over a minute on a simulator (see `PhoneHarnessLaunch` below and
    /// `TerminalScrollTests.openATerminalInTheShell`), and CI installs fresh
    /// every run — so a 30-second probe there tested how recently the app was
    /// installed, and since the harness probes fail rather than skip (ov-127),
    /// it would have failed the build for it.
    static let firstFrame: TimeInterval = 180

    /// Launch, and wait until the app has drawn anything that names itself.
    ///
    /// Every harness screen carries accessibility identifiers, so the first
    /// element with one is the first frame. Asserts nothing: the caller's own
    /// probe, right after this, is the check, and it no longer races the cold
    /// launch. Returns at once on a warm launch.
    func launchDrawn() {
        HarnessRetry.run("launch") { launch() }
        _ = descendants(matching: .any)
            .matching(NSPredicate(format: "identifier != ''"))
            .firstMatch
            .waitForExistence(timeout: Self.firstFrame)
    }

    /// Launch the app on `-phone-harness` with `arguments`, and wait until
    /// the canned runner is stood up: the harness publishes
    /// `phone-harness-ready` once its fleet, its list and its boards are in.
    ///
    /// 180 seconds, because the first launch after `xcodebuild` installs a
    /// build can take a minute or more on the simulator (see
    /// `TerminalScrollTests.openATerminalInTheShell`), and a shorter wait
    /// tested how recently the app was installed rather than the app.
    static func phoneHarness(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-phone-harness"] + arguments
        app.launchDrawn()
        let ready = app.descendants(matching: .any)["phone-harness-ready"]
        XCTAssertTrue(
            ready.waitForExistence(timeout: 180),
            "the harness never stood its runner up: \(app.debugDescription)")
        return app
    }
}
