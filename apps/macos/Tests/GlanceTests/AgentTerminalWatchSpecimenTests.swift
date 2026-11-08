import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// The whole Watch Agent sheet, drawn offscreen over a recorded stream, at the width `FARCOOLER_WATCH_WIDTH` names (ov-972). Runs only when `FARCOOLER_WATCH_STREAM` names a script that prints the
/// stream (a size marker, then a real agent screen); written where `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct AgentTerminalWatchSpecimenTests {
    @Test("Write the Watch Agent sheet at one width")
    func writeSheets() async throws {
        guard let script = ProcessInfo.processInfo.environment["FARCOOLER_WATCH_STREAM"] else { return }
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // One width per run (`FARCOOLER_WATCH_WIDTH`, default 640): a second terminal window in the same
        // process comes up blank.
        let width = Double(ProcessInfo.processInfo.environment["FARCOOLER_WATCH_WIDTH"] ?? "") ?? 640
        let name = "\(Int(width))pt"
        do {
            let sheet = AgentTerminalWatch(
                workspace: "w", item: "ov-972", binary: script, environment: [:], hostArguments: [], linkGeneration: 0)
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: width, height: 700), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: .darkAqua)
            window.contentView = NSHostingView(rootView: sheet)
            TestWindow.show(window)
            let until = Date().addingTimeInterval(3)
            while Date() < until { try await Task.sleep(for: .milliseconds(50)) }
            // A real screenshot of the window: the terminal draws on the GPU, which an offscreen
            // `cacheDisplay` of the view does not capture.
            let shot = Process()
            shot.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            shot.arguments = ["-x", "-o", "-l", String(window.windowNumber), directory.appendingPathComponent("watch-agent-\(name).png").path]
            try shot.run()
            shot.waitUntilExit()
            window.close()
        }
    }
}
