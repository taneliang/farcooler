import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the prompt this Mac asks before joining a
/// teammate's runners (ov-543), with a long email and long runner names, light
/// and dark. The production `TeammateJoinPrompt` in the Join sheet's frame.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct TeammateJoinCaptures {
    static let email = "alexandra.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"

    func shoot(_ name: String, from: String, labels: [String], to out: URL) async throws {
        let runners = labels.map(TeammateJoinPromptTests.runner)
        for scheme in [ColorScheme.light, .dark] {
            let host = NSHostingView(
                rootView: TeammateJoinPrompt(from: from, runners: runners, join: {}, cancel: {})
                    .padding(20).frame(width: 460)
                    .environment(\.colorScheme, scheme)
                    // style-exempt: the system window background, as the sheet's window has
                    .background(Color(nsColor: .windowBackgroundColor)))
            let size = host.fittingSize
            let window = NSWindow(
                contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            window.contentView = host
            TestWindow.show(window)
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            try #require(rep.representation(using: .png, properties: [:]))
                .write(to: out.appendingPathComponent("mac-\(name)-\(scheme == .dark ? "dark" : "light").png"))
            window.close()
        }
    }

    @Test func thePrompt() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let long = "Platform Builds, EU West (Shared) and Staging Runner 7"
        try await shoot("teammate-join-one", from: "alice@example.com", labels: ["build-box"], to: out)
        try await shoot("teammate-join-one-long", from: Self.email, labels: [long], to: out)
        try await shoot(
            "teammate-join-many-long", from: Self.email,
            labels: [long, "build-box", "gpu-workstation-with-an-unreasonably-long-name-for-a-machine"], to: out)
    }
}
