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
        app.launch()
        let ready = app.descendants(matching: .any)["phone-harness-ready"]
        XCTAssertTrue(
            ready.waitForExistence(timeout: 180),
            "the harness never stood its runner up: \(app.debugDescription)")
        return app
    }
}
