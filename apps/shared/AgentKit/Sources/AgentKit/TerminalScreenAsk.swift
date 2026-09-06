import Foundation

/// What a polling client asks the host for when it wants a pane's screen.
///
/// One constant and one dictionary, here in the shared package rather than
/// beside the only caller, because the caller cannot be tested anywhere that
/// runs. `TerminalSession` is compiled into the iOS app target, and the suite
/// that exercises it is `FarCoolerUITests` — which CI builds and never
/// executes (`.github/workflows/ci.yml`, "Build, including the UI test
/// bundle"). Android's half of this same fix has had a unit test since the day
/// it landed, `apps/android/app/src/test/java/com/farcooler/net/TerminalScrollbackTest.kt`,
/// and the phone's half has had none: the one line that makes a polled pane
/// scrollable at all was guarded on one platform out of two.
///
/// **What it guards is one number, and every way of losing it is silent.** A
/// poll carries `capture-pane -e -p` — the visible screen and no history — and
/// fed into an emulator exactly as tall as the screen that leaves nothing above
/// the top row for `scroll` to move through. `historyLines` is the ask that
/// puts the scrollback back (`crates/daemon/src/rpc.rs`, `terminal.screen`).
/// Drop the key and the daemon reads zero and sends none. Quote the number and
/// `crates/client/src/ffi.rs` reads it with `as_u64()`, which answers `nil` for
/// a string, and the daemon reads zero again. Both leave a terminal that
/// repaints several times a second, looks perfectly alive, and cannot be
/// scrolled.
///
/// **And the suite would not go red for either.** Every scroll assertion in
/// `TerminalScrollTests` stands on a pane that HAS scrollback, so losing this
/// ask does not fail one of them — it turns all of them into skips, and
/// `scripts/ios-ui-tests.sh` only refuses a run in which NOTHING executed. That
/// is this repo's own defining failure, and the reason the guard is a unit test
/// in a package CI actually runs rather than a comment above the call site.
public enum TerminalScreenAsk {
    /// How much scrollback a polled pane asks the host for.
    ///
    /// Two thousand rather than the emulator's full `SCROLLBACK_LINES` of
    /// 10,000: this crosses a phone's link base64'd, and at roughly 200 bytes a
    /// line the full history is a couple of megabytes to answer a swipe. Two
    /// thousand lines is far more than a thumb travels in one session and costs
    /// a few hundred kilobytes once per pane.
    ///
    /// It also matches tmux's own default `history-limit`, which is what any
    /// pane on a runner nobody has configured actually holds — so this asks for
    /// everything there is rather than for a slice of it.
    public static let historyLines = 2_000

    /// The arguments for `terminal.screen` on the path that needs scrollback.
    ///
    /// `Int`, deliberately and not `String`: see the type's note on `as_u64()`.
    /// `known_revision` is left out because the callers that want scrollback are
    /// the ones that have just built a fresh emulator and have no revision to
    /// compare against.
    public static func arguments(terminal: String) -> [String: Any] {
        ["terminal": terminal, "historyLines": historyLines]
    }
}
