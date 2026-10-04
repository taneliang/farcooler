import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// What the app does while nobody is looking at it (ov-229).
///
/// Idle with a busy board, the app used 23-52 % of a core. On a scratch board
/// of six working agents it sampled at 45-50 %, in front or hidden. The causes
/// were each one line: a SwiftUI `repeatForever` that rendered the whole
/// window every frame, a display-rate timeline, and terminals drawing into
/// windows nobody could see. These fail if one of them comes back.
@MainActor
@Suite(.serialized)
struct IdleCostTests {
    // MARK: - Visibility

    @Test func aWindowIsVisibleOnlyWhenOnScreenUnminimizedAndItsAppShown() {
        #expect(WindowVisibility.isVisible(occlusion: .visible, miniaturized: false, appHidden: false))
        #expect(!WindowVisibility.isVisible(occlusion: [], miniaturized: false, appHidden: false))
        #expect(!WindowVisibility.isVisible(occlusion: .visible, miniaturized: true, appHidden: false))
        #expect(!WindowVisibility.isVisible(occlusion: .visible, miniaturized: false, appHidden: true))
    }

    /// A window that was never put on screen is not visible, and the watch
    /// says so: the state a view starts in, and the one it returns to when its
    /// window goes away.
    @Test func noWindowAndAnUnshownWindowAreNotVisible() {
        var heard: [Bool] = []
        let watch = WindowVisibilityWatch { heard.append($0) }
        watch.follow(nil)
        #expect(!watch.isVisible)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled],
            backing: .buffered, defer: true)
        watch.follow(window)
        #expect(!watch.isVisible)
        #expect(heard.isEmpty, "a watch that never saw a change reported \(heard)")
    }

    // MARK: - The breathing dot

    /// The breath is a Core Animation animation on a layer, which the render
    /// server runs with nothing from this process. A SwiftUI `repeatForever`
    /// in its place rendered the whole window at the display's refresh rate.
    @Test func theBreathIsALayerAnimation() throws {
        let view = BreathingView(Circle().frame(width: 8, height: 8))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 40, height: 40), styleMask: [.titled],
            backing: .buffered, defer: true)
        window.contentView?.addSubview(view)
        let breath = try #require(
            view.layer?.animation(forKey: BreathingAnimation.key) as? CABasicAnimation,
            "a breathing dot in a window has no layer animation")
        #expect(breath.keyPath == "opacity")
        #expect(breath.repeatCount == .infinity)
        #expect(breath.autoreverses)
    }

    // MARK: - Terminals

    private static func paneInWindow() -> (TerminalRenderView, NSWindow) {
        let view = TerminalRenderView()
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 200), styleMask: [.titled],
            backing: .buffered, defer: true)
        window.contentView?.addSubview(view)
        return (view, window)
    }

    /// A pane in a window nobody can see doesn't tick, and one that can be
    /// seen does, until its layout hides it.
    @Test func aPaneTicksOnlyWhileItCanBeSeen() throws {
        let (hidden, hiddenWindow) = Self.paneInWindow()
        let link = try #require(hidden.displayLink, "a pane in a window has no display link")
        #expect(link.isPaused, "a pane in a window nobody can see is still ticking")
        _ = hiddenWindow

        WindowVisibility.assumeVisible = true
        defer { WindowVisibility.assumeVisible = false }
        let (shown, shownWindow) = Self.paneInWindow()
        _ = shownWindow
        #expect(shown.displayLink?.isPaused == false, "a pane in a visible window isn't ticking")

        shown.isShown = false
        #expect(shown.displayLink?.isPaused == true, "a pane its layout hides is still ticking")
        shown.isShown = true
        #expect(shown.displayLink?.isPaused == false, "a pane shown again didn't resume")
    }

    /// The rows a frame changed, from a real emulator.
    private static func damaged(after bytes: String, on core: VTCore, _ damage: inout TerminalDamage) -> IndexSet? {
        core.feed(Array(bytes.utf8))
        return core.withSnapshot { damage.rows(changedIn: $0) } ?? nil
    }

    @Test func aFrameRedrawsOnlyTheRowsItChanged() {
        let core = VTCore(columns: 20, rows: 6)
        var damage = TerminalDamage()
        #expect(Self.damaged(after: "one\r\ntwo", on: core, &damage) == nil, "the first frame must draw everything")
        #expect(Self.damaged(after: "", on: core, &damage) == IndexSet(), "an unchanged frame redrew something")

        // A spinner: the last row rewritten in place, cursor staying on it.
        let spun = Self.damaged(after: "\r\u{1B}[Kthree", on: core, &damage)
        #expect(spun == IndexSet(integer: 1), "a rewritten row redrew \(spun.map(Array.init) ?? [])")

        // A new line: the row it lands on, and the row the cursor left.
        let moved = Self.damaged(after: "\r\nfour", on: core, &damage)
        #expect(moved == IndexSet([1, 2]), "a new line redrew \(moved.map(Array.init) ?? [])")

        core.resize(columns: 30, rows: 6)
        #expect(Self.damaged(after: "", on: core, &damage) == nil, "a resize must draw everything")
    }

    // MARK: - Duration labels

    /// `Working 42s`, `12m`, `3h`: woken when the text can change, not every
    /// second for good.
    @Test func aDurationLabelWakesOnlyWhenItsTextCanChange() {
        let start = Date(timeIntervalSince1970: 1_000_000)
        func wake(_ elapsed: TimeInterval) -> TimeInterval {
            ElapsedSchedule.wake(after: start.addingTimeInterval(elapsed), since: start).timeIntervalSince(start)
        }
        #expect(wake(2) == 5)
        #expect(wake(30.2) == 31)
        #expect(wake(90.5) == 120)
        #expect(wake(4000) == 7200)

        var entries = ElapsedSchedule(since: start).entries(from: start.addingTimeInterval(60), mode: .normal)
        var wakes = 0
        while let next = entries.next(), next < start.addingTimeInterval(3600) { wakes += 1 }
        #expect(wakes == 59, "a row woke \(wakes) times between its first minute and its first hour")
    }

    @Test func noLabelTicksEverySecond() throws {
        let found = try Self.offenders { $0.contains(".periodic(from:") && $0.contains("by: 1)") }
        #expect(found.isEmpty, "Use `ElapsedSchedule` (see `Ticking`):\n\(found.joined(separator: "\n"))")
    }

    // MARK: - The sources

    /// Every view file the Mac draws with, AgentKit's included.
    private static func sources() throws -> [(name: String, lines: [Substring])] {
        let here = URL(fileURLWithPath: #filePath)
        let macos = here.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let roots = [
            macos.appendingPathComponent("Sources/FarCooler"),
            macos.appendingPathComponent("../shared/AgentKit/Sources/AgentKit"),
        ]
        var found: [(String, [Substring])] = []
        for root in roots {
            let walker = try #require(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            for case let url as URL in walker where url.pathExtension == "swift" {
                let text = try String(contentsOf: url, encoding: .utf8)
                found.append((url.lastPathComponent, text.split(separator: "\n", omittingEmptySubsequences: false)))
            }
        }
        #expect(found.count > 50, "found only \(found.count) source files; the paths above have moved")
        return found
    }

    /// Code lines matching `bad`, unless the line carries `idle-cost-exempt:`.
    private static func offenders(_ bad: (Substring) -> Bool) throws -> [String] {
        try sources().flatMap { file in
            file.lines.enumerated().compactMap { index, line in
                let code = line.trimmingCharacters(in: .whitespaces)
                guard !code.hasPrefix("//"), bad(line), !line.contains("idle-cost-exempt:") else { return nil }
                return "\(file.name):\(index + 1): \(code)"
            }
        }
    }

    /// A SwiftUI animation that never ends renders its window on every frame
    /// for as long as it is on screen, and behind other windows too.
    @Test func noViewAnimatesForever() throws {
        let found = try Self.offenders { $0.contains(".repeatForever(") }
        #expect(found.isEmpty, "Use a layer animation (see `BreathingView`):\n\(found.joined(separator: "\n"))")
    }

    /// An `.animation` timeline renders at the display's rate; it has to say
    /// when it pauses.
    @Test func everyAnimationTimelinePauses() throws {
        let found = try Self.offenders { $0.contains("TimelineView(.animation") && !$0.contains("paused:") }
        #expect(
            found.isEmpty,
            "Pass `paused: !windowVisible` (see `WorkingRow`):\n\(found.joined(separator: "\n"))")
    }
}
