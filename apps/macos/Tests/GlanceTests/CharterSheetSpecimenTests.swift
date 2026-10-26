import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Show Charter's read-only sheet (ov-755), a long charter and a short one, in
/// both appearances and at the narrowest and the usual width, written where
/// `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct CharterSheetSpecimenTests {
    @Test("Write the charter sheets")
    func writeSheets() async throws {
        let workspace = WorkspaceSummary(
            id: "w", name: "Billing and Invoicing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: "r")
        let long = (1...40).map { "## Section \($0)\nThe lead decides what always goes to the owner, and everything else is the orchestrator’s call, written down here so a new member reads the same rules as everyone else.\n" }
            .joined(separator: "\n")
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for (name, text, width, height) in [("long", long, 520.0, 440.0), ("narrow", long, 320.0, 300.0), ("short", "## Who decides\nThe lead.\n", 520.0, 240.0)] {
            for dark in [false, true] {
                let sheet = CharterSheet(workspace: workspace) { .text(text) }
                let host = NSHostingView(
                    rootView: sheet.frame(width: width, height: height)
                        .background(dark ? Color(white: 0.16) : Color(white: 0.96))
                        .environment(\.colorScheme, dark ? .dark : .light))
                host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                host.frame = CGRect(origin: .zero, size: CGSize(width: width, height: height))
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(300))
                let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: rep)
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(to: directory.appendingPathComponent("charter-\(name)-\(dark ? "dark" : "light").png"))
            }
        }
    }
}
