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
