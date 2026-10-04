import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The banners a refusal now shows (ov-160): the sentences are the real ones,
/// from the Mac's tables, so a table that stops going through `RunnerRefusal`
/// changes what is drawn. Both appearances, written where
/// `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct RefusalBannerSpecimenTests {
    private static let messages: [String] = [
        ActionCopy.sentence(.stop, subject: "“agent”", message: "error: x\ncode: scope-denied"),
        TaskFailure.sentence(for: "error: x\ncode: scope-denied"),
        ActionCopy.sentence(.stop, subject: "“agent”", message: "error: x\ncode: tmux-unavailable"),
    ]

    @Test("Write the refusal banner sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let specimen = VStack(spacing: 0) {
                // A stand-in for `ErrorBanner`, whose floating-panel glass does
                // not draw offscreen: the icon, the callout text and the close
                // mark, around the real sentences.
                ForEach(Self.messages, id: \.self) { message in
                    HStack(spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                        Text(message).font(.callout)
                        Spacer(minLength: 8)
                        Image(systemName: "xmark").foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                }
            }
            .frame(width: 720)
            .environment(\.colorScheme, dark ? .dark : .light)
            let host = NSHostingView(
                rootView: specimen.background(dark ? Color(white: 0.12) : Color(white: 0.96)))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("refusal-banners-\(dark ? "dark" : "light").png"))
        }
    }
}
