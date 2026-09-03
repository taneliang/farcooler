import Foundation

/// What each terminal last looked like, kept after its view has gone.
///
/// The Mac throws a pane away when you switch to another workspace: the panes
/// are keyed by terminal in `TileView`, so a different layout is a different
/// set of ids, and SwiftUI destroys every `TerminalRenderView` and builds new
/// ones. Each new one starts on an empty emulator and waits for a replay, which
/// is a `farcooler terminal stream` process, a `Runtime::open`, and four tmux
/// captures away — measured at 180-240ms on this machine's own runner, of which
/// the parse and the draw are about one and a half.
///
/// So the emulator outlives its view. A pane you have looked at before draws
/// its last frame in the same turn it mounts, and the replay — which is still
/// as far away as it ever was — arrives behind it. See
/// `TerminalRenderView.showRetained`.
///
/// This is what iOS and Android already do, arrived at from their own
/// constraints: iOS keeps every `TerminalSession` mounted in a `ZStack` and
/// swaps emulators on the first byte (`TerminalSession.consume`), and Android's
/// `PaneDeck` keeps three panes composed, least-recently-shown evicted. The
/// Mac's view lifecycle gives it no place to keep the VIEW, so it keeps the one
/// thing that actually holds the picture.
@MainActor
final class TerminalScreens {
    static let shared = TerminalScreens()

    /// How many screens are kept.
    ///
    /// Enough for two four-pane layouts, so the switch people actually make —
    /// away to another worktree and straight back — finds every pane it left.
    ///
    /// It is a memory budget and nothing else. A core holds `SCROLLBACK_LINES`
    /// = 10,000 lines of history, which `crates/vt` puts at a few megabytes per
    /// terminal, so eight is tens of megabytes on a machine that has them.
    /// Android caps at three because its low-memory killer counts native RSS;
    /// iOS does not cap at all. This is the middle, for the same reason the
    /// number differs there: the constraint is the platform's, not the design's.
    ///
    /// It costs the RUNNER nothing. A kept screen is an emulator in this
    /// process — no stream, no input channel, no ssh session. That is
    /// deliberate: every live stream is one of the ten sessions a default sshd
    /// allows across the whole app (`crates/client/src/ssh.rs`), and keeping
    /// panes streaming while nobody is watching them is how a phone used to run
    /// out of them.
    static let budget = 8

    private struct Kept {
        let core: VTCore
        /// When this screen was last handed back, for eviction. A counter
        /// rather than a clock: nothing here cares how long ago, only in what
        /// order, and a monotonic counter cannot be moved by the system clock.
        var usedAt: UInt64
    }

    private var kept: [String: Kept] = [:]
    private var tick: UInt64 = 0

    /// Take back a terminal's last screen, if one was kept.
    ///
    /// Removes it: the view that gets it now owns it, and will hand back
    /// whatever it is holding when it goes away — which after the swap is the
    /// live core, not this one.
    func take(_ terminal: String) -> VTCore? {
        kept.removeValue(forKey: terminal)?.core
    }

    /// Keep this terminal's screen for the next view to open on it.
    func keep(_ terminal: String, core: VTCore) {
        tick &+= 1
        kept[terminal] = Kept(core: core, usedAt: tick)
        evict()
    }

    /// How many screens are held, and whether one is. Read-only, and here for
    /// the tests: eviction has no visible symptom until the day it stops
    /// happening, at which point it is a memory graph rather than a screenshot.
    ///
    /// There is deliberately no `forget`. A terminal that is stopped leaves its
    /// last screen behind and the budget above collects it, which is the whole
    /// of the policy; a second way to drop one would be a method with no caller
    /// pretending to be a lifecycle.
    var count: Int { kept.count }
    func holds(_ terminal: String) -> Bool { kept[terminal] != nil }

    /// Drop the least recently kept, down to the budget.
    private func evict() {
        while kept.count > Self.budget {
            guard let oldest = kept.min(by: { $0.value.usedAt < $1.value.usedAt })?.key else {
                return
            }
            kept.removeValue(forKey: oldest)
        }
    }
}
