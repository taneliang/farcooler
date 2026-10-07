import XCTest

/// `HarnessRetry` retries the simulator's own errors, and only those (ov-397).
/// Needs no app and no runner: the operations here are closures that record
/// the issue a real `launch()` or `terminate()` would.
final class HarnessRetryTests: XCTestCase {
    private func noPause(_ operation: () -> Void) -> (retries: Int, log: [String]) {
        var log: [String] = []
        let retries = HarnessRetry.run("launch", pause: 0, log: { log.append($0) }, operation)
        return (retries, log)
    }

    func testTheFourSimulatorErrorsAreRecognized() {
        for message in [
            "Failed to terminate com.farcooler.ios.local:1234",
            "Application 'com.farcooler.ios.local' does not have a process ID",
            "Application 'com.farcooler.ios.local' has not loaded accessibility",
            "Timed out: the main thread busy",
        ] {
            XCTAssertTrue(HarnessRetry.isSimulatorError(message), message)
        }
        for message in [
            "XCTAssertTrue failed - the harness never stood its runner up",
            "XCTAssertEqual failed: (\"1\") is not equal to (\"2\")",
            "Failed to find the button",
        ] {
            XCTAssertFalse(HarnessRetry.isSimulatorError(message), message)
        }
    }

    func testASimulatorErrorIsRetriedOnceThenPasses() {
        var calls = 0
        let (retries, log) = noPause {
            calls += 1
            if calls == 1 { XCTFail("Application 'x' does not have a process ID") }
        }
        XCTAssertEqual(calls, 2)
        XCTAssertEqual(retries, 1)
        XCTAssertEqual(log.count, 1, "one line per retry")
        XCTAssertTrue(log.first?.hasPrefix("harness-retry: launch attempt 1 of 3") ?? false, "\(log)")
    }

    func testASimulatorErrorThatNeverClearsStopsAtTwoRetriesAndFails() {
        var calls = 0
        var result: (retries: Int, log: [String]) = (0, [])
        // The third attempt is bare, so its issue fails the test; this outer
        // expectation is strict, so it fails if that issue is NOT recorded.
        XCTExpectFailure("the last attempt's simulator error must still fail") {
            result = noPause {
                calls += 1
                XCTFail("Failed to terminate com.farcooler.ios.local")
            }
        }
        XCTAssertEqual(calls, 3)
        XCTAssertEqual(result.retries, 2)
        XCTAssertEqual(result.log.count, 2)
    }

    func testAnAssertionFailureIsNeverRetried() {
        var calls = 0
        var result: (retries: Int, log: [String]) = (0, [])
        XCTExpectFailure("an assertion failure must reach the test, strictly") {
            result = noPause {
                calls += 1
                XCTAssertTrue(false, "the harness never stood its runner up")
            }
        }
        XCTAssertEqual(calls, 1, "an assertion failure ran the operation again")
        XCTAssertEqual(result.retries, 0)
        XCTAssertTrue(result.log.isEmpty)
    }
}
