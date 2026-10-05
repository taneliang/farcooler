import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the production pull-down in a real titled
/// window's toolbar with its menu open, light and dark (ov-319). Drives the
/// control's own coordinator; sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PullDownMenuCaptures {
    private struct Bar: View {
        var body: some View {
            Color.clear
                .toolbar {
                    ToolbarItem(placement: .principal) {
                        PullDownMenu(entries: [
                            .item("ov-1 Mac: status menus drop below") {}, .item("ov-2 Relay: review count") {},
                            .header("Queued"), .item("ov-3 Daemon: size ceiling") {},
                        ]) {
                            HStack(spacing: 4) {
                                Image(systemName: "circle.dotted")
                                Text("2 running · 1 queued")
                            }
                            .foregroundStyle(.secondary)
                        }
                    }
                }
        }
    }

    @Test(arguments: (ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_VARIANT"] ?? "light,dark").split(separator: ",").map(String.init)) func menuOpenBelowItsButton(variant: String) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        let window = try await TitleBarHarness.window(Bar(), width: 700, height: 300)
        window.appearance = NSAppearance(named: variant == "dark" ? .darkAqua : .aqua)
        try await TitleBarHarness.settle(window)
        var anchor: NSView?
        func find(_ v: NSView) {
            if v is PullDownAnchorView { anchor = v }
            v.subviews.forEach(find)
        }
        window.contentView?.superview.map(find)
        let coordinator = try #require((anchor as? PullDownAnchorView)?.coordinator)

        let probe = Timer(timeInterval: 0.6, repeats: false) { _ in
            MainActor.assumeIsolated {
                let menu = NSApp.windows.first { String(describing: type(of: $0)).contains("Menu") }
                // The screen region both windows cover, as the window server
                // composites it: the window, and the menu over it.
                let frames = [window.frame] + (menu.map { [$0.frame] } ?? [])
                let union = frames.dropFirst().reduce(frames[0]) { $0.union($1) }
                let top = (NSScreen.screens.first?.frame.height ?? 0) - union.maxY
                let task = Process()
                task.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                task.arguments = [
                    "-x", "-R", "\(Int(union.minX)),\(Int(top)),\(Int(union.width)),\(Int(union.height))",
                    URL(fileURLWithPath: out).appendingPathComponent("menu-below-\(variant).png").path,
                ]
                try? task.run()
                task.waitUntilExit()
                coordinator.lastMenu?.cancelTracking()
            }
        }
        RunLoop.main.add(probe, forMode: .common)
        coordinator.open()
        window.close()
    }
}
