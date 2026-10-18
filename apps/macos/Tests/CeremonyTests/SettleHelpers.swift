import AppKit

/// Waits that end when the thing waited for has happened, not after a fixed
/// time (ov-537). A fixed sleep is both too long on a fast Mac, where it was
/// paid in full by every window of a sweep, and too short on a loaded runner;
/// a poll is as long as it needs to be, and the limit is only a stop for a
/// hang, far past anything a slow runner needs.
@MainActor
enum Settle {
    /// The most any wait here holds out for something that never comes.
    static let limit: Duration = .seconds(120)

    /// Polls `condition` every `step` until it holds, and says whether it did
    /// before `limit`. The condition is read first, so one already true costs
    /// nothing.
    @discardableResult
    static func until(
        limit: Duration = Settle.limit, step: Duration = .milliseconds(5), _ condition: () -> Bool
    ) async -> Bool {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: limit)
        while !condition() {
            if clock.now >= deadline { return false }
            try? await Task.sleep(for: step)
        }
        return true
    }

    /// Lays `view` out until what `reading` reads has held still for `passes`
    /// passes in a row, `step` apart. A layout that is a pass or two late
    /// changes the reading and starts the count again; one that is done is
    /// confirmed in `passes` steps instead of a fixed second.
    static func still(
        _ view: NSView?, reading: () -> String, passes: Int = 5, step: Duration = .milliseconds(10)
    ) async {
        var last: String?
        var held = 0
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: limit)
        while held < passes, clock.now < deadline {
            view?.layoutSubtreeIfNeeded()
            let now = reading()
            held = now == last ? held + 1 : 0
            last = now
            if held < passes { try? await Task.sleep(for: step) }
        }
    }

    /// Where each scroll view under `view` is scrolled to.
    static func scrollOffsets(under view: NSView?) -> String {
        guard let view else { return "" }
        var out = ""
        func walk(_ view: NSView) {
            if let scroll = view as? NSScrollView { out += "\(scroll.contentView.bounds)\n" }
            view.subviews.forEach(walk)
        }
        walk(view)
        return out
    }

    /// Every view's class and frame under `view`: the layout, as a string to
    /// compare.
    static func frames(under view: NSView?) -> String {
        guard let view else { return "" }
        var out = ""
        func walk(_ view: NSView) {
            out += "\(type(of: view)) \(view.frame) \(view.isHidden)\n"
            view.subviews.forEach(walk)
        }
        walk(view)
        return out
    }
}

extension NativeAgentTests {
    /// Draws `window` until the frames its probes reported (`frames`, which
    /// the tests below read) have held still for `passes` passes. What a test
    /// reads is what is waited for, so a window whose layout is done is
    /// confirmed in tens of milliseconds, not the 300 to 1,500 the callers
    /// used to sleep (ov-537).
    static func settle(_ window: NSWindow, still frames: NativePolishTests.Frames, passes: Int = 5) async {
        await Settle.still(
            window.contentView,
            reading: {
                window.displayIfNeeded()
                // Where every scroll view is scrolled to as well: a list
                // that follows its end moves after its rows land.
                return frames.views.sorted { $0.key < $1.key }.map { "\($0.key) \($0.value)" }.joined(separator: "\n")
                    + "\n" + Settle.scrollOffsets(under: window.contentView)
            }, passes: passes)
    }
}
