import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator's panes at rest and scrolled (ov-258), in both appearances,
/// so the rules can be looked at: the scroll content runs up to each line,
/// and at rest the rows keep the rhythm's room from it. A rendering written
/// where `FARCOOLER_GLANCE_OUT` says; the panes are scrolled through their own
/// scroll views, with no input sent.
@MainActor
struct NavigatorSplitSpecimenTests {
    @Test("Write the navigator's split sheets")
    func writeSheets() async throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let drawn = NavigatorSplitTests.Drawn(
                await NavigatorSplitTests.board(tasks: 120, terminals: 30), height: 800, dark: dark)
            await drawn.settle()
            try Self.write(drawn, to: directory.appendingPathComponent("split-rest-\(dark ? "dark" : "light").png"))
            for scroll in drawn.scrollViews().prefix(2) {
                let clip = scroll.contentView
                clip.scroll(to: NSPoint(x: 0, y: clip.bounds.origin.y + 100))
                scroll.reflectScrolledClipView(clip)
            }
            await drawn.settle()
            try Self.write(drawn, to: directory.appendingPathComponent("split-scrolled-\(dark ? "dark" : "light").png"))
        }
    }

    private static func write(_ drawn: NavigatorSplitTests.Drawn, to url: URL) throws {
        let png = try #require(drawn.pixels().representation(using: .png, properties: [:]))
        try png.write(to: url)
    }
}
