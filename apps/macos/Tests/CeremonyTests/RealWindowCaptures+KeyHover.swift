import AgentKit
import AppKit
import Testing

@testable import Far_Cooler

/// A task key's hovercard in a real-window capture (ov-299), with no input
/// sent: `FARCOOLER_CAPTURE_KEY_HOVER` names the key. A terminal showing it
/// is hovered through its own hover path, at the key's first cell; text and
/// plan rows showing it are told through `TaskKeyHoverForcing`. The card is a
/// popover, a window of its own, so it's drawn into the capture where it sits.
extension RealWindowCaptures {
    /// Show `key`'s card where `window` draws it: in the first terminal
    /// showing it, or else the first text.
    static func hoverKey(_ key: String, in window: NSWindow) async throws {
        // A popover stays on its screen, so a window reaching above the top
        // would have its card pushed down off the key: the whole window is
        // kept below the top while the card is up.
        if let screen = window.screen ?? NSScreen.main, window.frame.maxY > screen.visibleFrame.maxY {
            window.setFrameOrigin(NSPoint(x: window.frame.minX, y: screen.visibleFrame.maxY - window.frame.height))
            try await TitleBarHarness.settle(window)
        }
        let root = try #require(window.contentView)
        let shown = terminals(in: root).lazy.compactMap { view in firstCell(of: key, in: view).map { (view, $0) } }.first
        if let (view, cell) = shown {
            view.keyHover.delay = .zero
            let width = TerminalMetrics.cell(Preferences.shared.terminalFont()).width
            let x = TerminalMetrics.padding.left + (CGFloat(cell.column) + 0.5) * width
            view.keyHover.hover(view, row: cell.row, column: cell.column, x: x)
        } else {
            TaskKeyHoverForcing.key = key
        }
        try await Task.sleep(for: .seconds(2))

    }

    static func terminals(in view: NSView) -> [TerminalRenderView] {
        (view as? TerminalRenderView).map { [$0] } ?? view.subviews.flatMap(terminals(in:))
    }

    /// The first cell the link hit-testing reads as `key`.
    static func firstCell(of key: String, in view: TerminalRenderView) -> (row: Int, column: Int)? {
        let grid = view.grid
        for row in 0..<grid.rows {
            for column in 0..<grid.columns where view.taskKeyCard(atRow: row, column: column)?.card.key == key {
                return (row, column)
            }
        }
        return nil
    }

    /// `rep` of `window`, or, while a popover is up over it, the window as
    /// the window server composites it, card and glass included: a view's
    /// own bitmap draws neither. Read back by this process's own window
    /// number, never the screen.
    static func withPopovers(_ rep: NSBitmapImageRep, of window: NSWindow, scale: Int) -> NSBitmapImageRep {
        let popovers = NSApp.windows.filter {
            $0 !== window && $0.isVisible && String(describing: type(of: $0)).contains("Popover")
        }
        guard !popovers.isEmpty, let image = windowImage(window) else { return rep }
        return image
    }

    /// `window` as the window server composites it, through
    /// `screencapture -l`, or nil when that isn't allowed here.
    static func windowImage(_ window: NSWindow) -> NSBitmapImageRep? {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("key-hover-\(window.windowNumber).png")
        defer { try? FileManager.default.removeItem(at: file) }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        process.arguments = ["-x", "-o", "-l", String(window.windowNumber), file.path]
        guard (try? process.run()) != nil else { return nil }
        process.waitUntilExit()
        guard process.terminationStatus == 0, let data = try? Data(contentsOf: file) else { return nil }
        return NSBitmapImageRep(data: data)
    }
}
