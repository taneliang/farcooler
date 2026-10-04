import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The confirmation ⌘W raises over a working agent (ov-161), as the system's
/// dialog lays it out: its title, its message and its two buttons, from the
/// real strings. The dialog itself is AppKit's and can't be drawn offscreen,
/// so this is a stand-in around the real copy. Both appearances, written where
/// `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct CloseTerminalSpecimenTests {
    @Test("Write the close confirmation sheets")
    func writeSheets() throws {
        let json = """
            {"id":"t-1","short":"t1","title":"claude","preset":"claude","state":"running",
            "activity":"working","activitySince":\((Date().timeIntervalSince1970 - 240) * 1000),
            "turnStartedAt":\((Date().timeIntervalSince1970 - 240) * 1000),"epoch":0}
            """
        let terminal = try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
        let question = try #require(CloseTerminalGuard.question(for: terminal, at: Date()))
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let specimen = VStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 34)).foregroundStyle(.orange)
                Text(question.title).font(.headline)
                Text(question.message).font(.callout).multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Text("Cancel").padding(.horizontal, 12).padding(.vertical, 4)
                    Text(CloseTerminalGuard.confirm).padding(.horizontal, 12).padding(.vertical, 4)
                        .foregroundStyle(.red)
                }
                .font(.callout)
            }
            .frame(width: 300)
            .padding(24)
            .environment(\.colorScheme, dark ? .dark : .light)
            let host = NSHostingView(
                rootView: specimen.background(dark ? Color(white: 0.16) : Color(white: 0.96)))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("close-terminal-\(dark ? "dark" : "light").png"))
        }
    }
}
