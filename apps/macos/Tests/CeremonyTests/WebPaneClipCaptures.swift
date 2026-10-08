import AgentKit
import AppKit
import SwiftUI
import Testing
import WebKit

@testable import Far_Cooler

/// A web pane against the card's rounded corners, in a real window (ov-436).
///
/// Opt-in (FARCOOLER_CAPTURE_OUT). A `WKWebView` draws in WebKit's own
/// process, so a snapshot of the view proves nothing about the window: this
/// asks the window server for the window's pixels (`screencapture -l`), for
/// the pane's own page on a ground that shows any bleed, in light and dark.
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
@MainActor
struct WebPaneClipCaptures {
    @Test("A web page stays inside the pane card's rounded corners", arguments: ["light", "dark"])
    func clipped(appearance: String) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let suite = "WebPaneClip-\(UUID().uuidString)"
        let model = WebPaneModel(terminal: "clip-\(appearance)", memory: WebPaneMemory(defaults: UserDefaults(suiteName: suite)!))
        // A page whose every pixel is magenta, so a corner it reaches is plain.
        model.webView.loadHTMLString(
            "<body style='margin:0;background:#ff00ff'></body>", baseURL: nil)

        let card = WebPageView(model: model)
            .frame(width: 360, height: 240)
            .paneCard()
            .padding(30)
            .background(Color(white: 0.5))
        let host = NSHostingView(rootView: card)
        let window = NSWindow(
            contentRect: NSRect(x: 200, y: 200, width: 420, height: 300), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: appearance == "dark" ? .darkAqua : .aqua)
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        for _ in 0..<60 where model.isLoading || model.webView.superview == nil || model.webView.url == nil {
            try await Task.sleep(for: .milliseconds(100))
        }
        try await Task.sleep(for: .seconds(1.5))

        let path = "\(out)/web-clip-\(appearance).png"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        process.arguments = ["-x", "-o", "-l", String(window.windowNumber), path]
        try process.run()
        process.waitUntilExit()
        let image = try #require(NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: path))))
        let scale = CGFloat(image.pixelsWide) / window.frame.width
        // The card's top-left corner, in the window's content (below the
        // title bar): its outermost pixel must not be the page's.
        let titleBar = window.frame.height - window.contentLayoutRect.height
        func pixel(_ x: CGFloat, _ y: CGFloat) -> NSColor {
            image.colorAt(x: Int(x * scale), y: Int((y + titleBar) * scale))!.usingColorSpace(.sRGB)!
        }
        func isPage(_ c: NSColor) -> Bool { c.redComponent > 0.8 && c.greenComponent < 0.4 && c.blueComponent > 0.8 }
        #expect(isPage(pixel(210, 150)), "the page draws at all")
        #expect(!isPage(pixel(30.5, 30.5)), "the page reaches the card's top-left corner")
        #expect(!isPage(pixel(389.5, 269.5)), "the page reaches the card's bottom-right corner")
    }
}
