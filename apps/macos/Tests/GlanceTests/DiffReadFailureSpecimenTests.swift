import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The row a file shows when its changes could not be read (ov-155), drawn in
/// both appearances so it can be looked at. Written where `FARCOOLER_GLANCE_OUT`
/// says.
@MainActor
struct DiffReadFailureSpecimenTests {
    @Test("Write the diff failure sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let specimen = VStack(alignment: .leading, spacing: 4) {
                Text("Sources/FarCooler/ChangesModel.swift")
                    .font(.system(size: 12, weight: .semibold))
                    .padding(.horizontal, 10)
                DiffReadFailureRow(font: .system(size: 12, design: .monospaced)) {}
            }
            .frame(width: 460, alignment: .leading)
            .padding(16)
            let host = NSHostingView(
                rootView: specimen.background(dark ? Color(white: 0.12) : Color(white: 0.96)))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("diff-read-failure-\(dark ? "dark" : "light").png"))
        }
    }
}
