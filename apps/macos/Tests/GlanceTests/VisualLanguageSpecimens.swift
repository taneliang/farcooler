import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Where the visual-language specimens (ov-220 to ov-225) are written, and how
/// one is drawn: a real `NSHostingView` in a bitmap, light and dark, over a
/// stand-in wallpaper, so a frosted plane (which a bitmap can't blur) shows what
/// is behind it. `FARCOOLER_VIS_OUT` names the folder and `FARCOOLER_VIS_STAGE`
/// is `before` or `after`. Nothing here asserts: it is a rendering to look at.
@MainActor
enum VisualSpecimen {
    static var directory: URL {
        URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_VIS_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/visual")
    }
    static var stage: String { ProcessInfo.processInfo.environment["FARCOOLER_VIS_STAGE"] ?? "after" }

    /// `view` at `size` in both appearances, with the high-contrast appearance when
    /// asked (Reduce Transparency can't be set on a bitmap), written as `<stage>-<name>-<light|dark>.png` (`-contrast` for the high-contrast appearance).
    static func shoot<V: View>(
        _ name: String, size: CGSize, increasedContrast: Bool = false, _ view: @autoclosure () -> V
    ) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let wallpaper = LinearGradient(
                colors: dark
                    ? [Color(red: 0.10, green: 0.14, blue: 0.24), Color(red: 0.24, green: 0.10, blue: 0.20)]
                    : [Color(red: 0.62, green: 0.76, blue: 0.95), Color(red: 0.95, green: 0.78, blue: 0.70)],
                startPoint: .topLeading, endPoint: .bottomTrailing)
            let host = NSHostingView(
                rootView: view()
                    .frame(width: size.width, height: size.height)
                    .background(wallpaper))
            host.appearance = NSAppearance(
                named: increasedContrast
                    ? (dark ? .accessibilityHighContrastDarkAqua : .accessibilityHighContrastAqua)
                    : (dark ? .darkAqua : .aqua))
            host.frame = CGRect(origin: .zero, size: size)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("\(stage)-\(name)-\(dark ? "dark" : "light").png"))
        }
    }
}
