import XCTest

/// Retries a launch or a terminate that the simulator itself failed, and
/// nothing else (ov-397).
///
/// CI's iOS UI shards failed at `PhoneHarnessLaunch.swift:32` at least four
/// times on Oct 6 with errors that name the simulator, not the app:
/// `Application 'com.farcooler.ios.local' does not have a process ID` (run
/// 37570315019, the one message the logs still hold), and, from the card's
/// reading of the other runs, `Failed to terminate`, `has not loaded
/// accessibility` and `main thread busy`. Each one cost a re-run of a 30-minute
/// macOS job.
///
/// XCTest does not throw these. `launch()` and `terminate()` record them as
/// issues on the running test. So the attempt runs inside `XCTExpectFailure`
/// whose matcher accepts only an issue carrying one of `phrases`: a matching
/// issue is absorbed and counted, and any other issue, an assertion included,
/// is not matched, so it fails the test as it always did, and is never retried.
/// The last attempt runs bare, so a simulator that stays broken still fails the
/// test with its own message.
enum HarnessRetry {
    /// Retries after the first attempt, so three attempts at most.
    static let maxRetries = 2

    /// The simulator errors worth a retry, matched as substrings of the issue's
    /// description, ignoring case.
    static let phrases = [
        "Failed to terminate",
        "does not have a process ID",
        "has not loaded accessibility",
        "main thread busy",
    ]

    /// Retries so far in this test process, named in each retry's log line so
    /// a rising rate shows up in CI.
    nonisolated(unsafe) private(set) static var retriesSoFar = 0

    /// Test-only injection: `FC_INJECT_HARNESS_FAILURE` in the runner's
    /// environment (`TEST_RUNNER_FC_INJECT_HARNESS_FAILURE=<message>` on the
    /// xcodebuild command) makes the first `FC_INJECT_HARNESS_FAILURES`
    /// attempts (default 1) of each operation record `<message>` as an issue
    /// instead of running it. The injected issue goes through the same matcher
    /// and loop as a real one.
    private static var injected: (message: String, count: Int)? {
        let env = ProcessInfo.processInfo.environment
        guard let message = env["FC_INJECT_HARNESS_FAILURE"], !message.isEmpty else { return nil }
        return (message, Int(env["FC_INJECT_HARNESS_FAILURES"] ?? "") ?? 1)
    }

    static func isSimulatorError(_ description: String) -> Bool {
        phrases.contains { description.range(of: $0, options: .caseInsensitive) != nil }
    }

    /// Run `operation`, retrying it on a simulator error. Returns how many
    /// retries it took. `pause` is injectable so the tests need none.
    @discardableResult
    static func run(
        _ label: String,
        pause: TimeInterval = 2,
        log: (String) -> Void = { print($0) },
        _ operation: () -> Void
    ) -> Int {
        var retries = 0
        for attempt in 0...maxRetries {
            func step() {
                if let inject = injected, attempt < inject.count {
                    XCTFail(inject.message)
                } else {
                    operation()
                }
            }
            if attempt == maxRetries {
                step()
                break
            }
            var absorbed: String?
            let options = XCTExpectedFailure.Options()
            options.isStrict = false
            options.issueMatcher = { issue in
                guard isSimulatorError(issue.compactDescription) else { return false }
                absorbed = issue.compactDescription
                return true
            }
            XCTExpectFailure("a simulator error on \(label) is retried", options: options, failingBlock: step)
            guard let why = absorbed else { break }
            retries += 1
            retriesSoFar += 1
            log("harness-retry: \(label) attempt \(attempt + 1) of \(maxRetries + 1) hit a simulator error, retrying (retry \(retriesSoFar) this run): \(why)")
            Thread.sleep(forTimeInterval: pause)
        }
        return retries
    }
}

extension XCUIApplication {
    /// `terminate()`, retried on a simulator error (see `HarnessRetry`).
    func terminateRetrying() {
        HarnessRetry.run("terminate") { terminate() }
    }
}
