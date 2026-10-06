import XCTest

/// The one way a UI test reads the shell's `shell-state` probe (ov-354).
///
/// **It waits for `busy=0`, because XCUITest does not.** A drag returns, and
/// the next query is answered, once XCUITest calls the app idle, and a page
/// turn changes `tab` only when its spring's completion re-seats the
/// position, which that idle check does not wait for. Read straight after the
/// return swipe under `-shell-slow-frame`, the probe said `tab=1 busy=1` in 15
/// reads of 15, with the release already decided as a commit to tab 0
/// (ov-346; `ShellRootView.busy`). Every reader that parses the probe goes
/// through here, so none of them can see the old page. A shell that never
/// settles, a release that never ran, still fails, here, by name.
extension XCUIApplication {
    /// How long the shell may stay busy after the last gesture.
    private static let shellSettle: TimeInterval = 10

    /// `ws`, `tab`, `tabs` and the rest of `ShellRootView.probe`'s fields, read
    /// once the shell has let go of the last gesture.
    func shellState() throws -> [String: Int] {
        let probe = descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 30) else {
            print(debugDescription)
            throw HarnessFailure("The shell never rendered its probe.")
        }
        let deadline = Date().addingTimeInterval(Self.shellSettle)
        var value = probe.value as? String ?? ""
        while value.split(separator: " ").contains("busy=1") {
            guard Date() < deadline else {
                throw HarnessFailure("The shell never let go of the last gesture (\(value)).")
            }
            Thread.sleep(forTimeInterval: 0.1)
            value = probe.value as? String ?? ""
        }
        var parsed: [String: Int] = [:]
        for pair in value.split(separator: " ") {
            let halves = pair.split(separator: "=")
            guard halves.count == 2, let number = Int(halves[1]) else { continue }
            parsed[String(halves[0])] = number
        }
        return parsed
    }
}
