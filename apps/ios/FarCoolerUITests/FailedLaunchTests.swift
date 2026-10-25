import XCTest

/// A launch the simulator fails fails the test (ov-703).
///
/// `HarnessRetry` (ov-397) ran each launch and terminate inside
/// `XCTExpectFailure`, to absorb the simulator's own errors ("does not have a
/// process ID", "Failed to terminate") and try again. On CI it never tried
/// again: each absorbed error was its test's last act, and an expected failure
/// is a pass. No nightly logged a "harness-retry" line; each absorbed error
/// closed its test green with the rest unrun. On Oct 24 (run 40103487778) the
/// largest-size conversation audit "passed" in 188 s without its app, as did
/// NavigationCapturesUITests' testConversationAndAgents and NativeAgentViewTests'
/// testAPromptsImagesShowAndOpen. Each launch and terminate now runs bare, and
/// the simulator's error is the test's failure, in its own words.
final class FailedLaunchTests: XCTestCase {
    /// An app that isn't installed: its launch fails as a UI Automation
    /// Failure, as the simulator's own errors do.
    private static let nowhere = "com.farcooler.nowhere"

    /// Whether the launch's failure reached this test's own expectation,
    /// rather than being absorbed before it. Read in `tearDown`, which runs
    /// however the body ends.
    private var reachedTheTest = false

    override func tearDown() {
        XCTAssertTrue(reachedTheTest, "the failed launch was absorbed before it reached the test, which ended green")
        super.tearDown()
    }

    func testAFailedLaunchReachesTheTest() {
        let options = XCTExpectedFailure.Options()
        options.issueMatcher = { [unowned self] issue in
            guard issue.compactDescription.contains(Self.nowhere) else { return false }
            reachedTheTest = true
            return true
        }
        XCTExpectFailure("the launch fails, as the simulator's errors do", options: options) {
            XCUIApplication(bundleIdentifier: Self.nowhere).launchDrawn()
        }
    }
}
