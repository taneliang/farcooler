import Foundation
import Testing

@testable import AgentKit

/// The half of the phone's scrollback fix that does not need a runner.
///
/// Deliberately the twin of Android's `TerminalScrollbackTest`, which has
/// asserted exactly this since the day that fix landed while iOS asserted
/// nothing. What both are protecting is the ASK — the phone telling the host
/// how much history to send above the screen — because a pane that never asks
/// looks identical to a pane that asked and had none, and only one of those is
/// a bug.
///
/// Negative controls, run:
///
///   * `historyLines` set to 0 — `aPollAsksForScrollback` fails with
///     `Expectation failed: TerminalScreenAsk.historyLines > 0`.
///   * `"historyLines": String(historyLines)` — `theNumberIsSentUnquoted`
///     fails, because the JSON reads `"historyLines":"2000"` and the host's
///     `as_u64()` answers nil for that and sends no scrollback at all.
///
/// Neither of those turns a single scroll test red: they turn every one of them
/// into a skip. That is why the guard is here, in a package
/// `swift test --package-path apps/shared/AgentKit` runs on every commit,
/// rather than in the UI suite CI compiles and never executes.
struct TerminalScreenAskTests {
    /// The JSON the core is handed, spelled the way `withJSON` spells it.
    private func encoded(_ arguments: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: arguments, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    @Test("A poll asks for scrollback, and names the pane it wants")
    func aPollAsksForScrollback() throws {
        #expect(TerminalScreenAsk.historyLines > 0)
        let arguments = TerminalScreenAsk.arguments(terminal: "t")
        #expect(arguments["terminal"] as? String == "t")
        #expect(arguments["historyLines"] as? Int == TerminalScreenAsk.historyLines)
    }

    /// The host reads this with `as_u64()` (`crates/client/src/ffi.rs`), which
    /// answers nil for a string and falls back to zero without complaining — so
    /// quoting the number would not fail anywhere, it would just ask for no
    /// history and quietly undo the whole fix.
    @Test("The number crosses the wire unquoted")
    func theNumberIsSentUnquoted() throws {
        let json = try encoded(TerminalScreenAsk.arguments(terminal: "t"))
        #expect(
            json.contains("\"historyLines\":\(TerminalScreenAsk.historyLines)"),
            "expected an unquoted number, got \(json)")
        #expect(json.contains("\"terminal\":\"t\""), "the ask must still name the pane")
    }

    /// Enough to be worth asking for. A handful of lines would satisfy every
    /// assertion above and still leave a swipe with nowhere to go — tmux's own
    /// default `history-limit` is 2000, and asking for less than a pane holds
    /// is a scrollback that ends before the pane's does.
    @Test("It asks for as much as an unconfigured runner actually keeps")
    func itAsksForEverythingTmuxKeeps() {
        #expect(TerminalScreenAsk.historyLines >= 2_000)
    }
}
