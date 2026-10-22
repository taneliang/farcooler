import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Quiet text on the grounds the native conversation is drawn on (ov-686).
///
/// The conversation sits on `WorkspaceStyle.paper`: the system's text
/// background with 3% of the chosen terminal theme mixed in, in the aqua or
/// dark aqua appearance (the theme picks which, or the Appearance setting does).
/// So the ground a `.secondary` caption meets is nearly white or nearly black,
/// tinted by the theme, and never the theme's own background. This draws the
/// ink as SwiftUI draws it, over each ground, for every theme the CLI lists in
/// both appearances, and computes WCAG's contrast ratio from the pixels.
///
/// `.secondary` and `.tertiary` are measured and printed, not asserted: they
/// are the "before" column, and the system owns them. What is asserted is the
/// token that replaces them, `Ink.quiet`, at the 4.5:1 text needs.
@MainActor
struct QuietInkTests {
    static let appearances: [NSAppearance.Name] = [.aqua, .darkAqua]

    /// A ground a conversation row's text sits on.
    struct Ground {
        let name: String
        let view: AnyView
    }

    static let grounds: [Ground] = [
        // The conversation itself.
        Ground(name: "paper", view: AnyView(Color.clear.background(WorkspaceStyle.paper))),
        // A tool block, a note or a code box: the inset wash over the paper.
        Ground(
            name: "inset",
            view: AnyView(Color.clear.background(WorkspaceStyle.paper).overlay(Fill.inset()))),
        // The pane around a card, which is a little more themed than the paper.
        Ground(name: "document", view: AnyView(Color.clear.background(WorkspaceStyle.document))),
    ]

    private func pixel<V: View>(_ view: V, _ name: NSAppearance.Name) throws -> NSColor {
        let size = CGSize(width: 40, height: 40)
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.lookBitmap(scale: 1))
        return rep.color(atPoint: 20, 20)
    }

    /// WCAG 2.x contrast ratio of two opaque sRGB colors.
    static func ratio(_ a: NSColor, _ b: NSColor) -> Double {
        func luminance(_ c: NSColor) -> Double {
            func channel(_ v: CGFloat) -> Double {
                let v = Double(v)
                return v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * channel(c.redComponent) + 0.7152 * channel(c.greenComponent) + 0.0722 * channel(c.blueComponent)
        }
        let (hi, lo) = (max(luminance(a), luminance(b)), min(luminance(a), luminance(b)))
        return (hi + 0.05) / (lo + 0.05)
    }

    /// The contrast of `style` as text over `ground`, in `appearance`.
    private func contrast<S: ShapeStyle>(_ style: S, over ground: Ground, _ appearance: NSAppearance.Name) throws -> Double {
        let under = try pixel(ground.view, appearance)
        // The same ground with a swatch of the ink on it, in the foreground
        // style a `Text` takes, so `.secondary` is the hierarchy level it is
        // in a conversation and not a guess at its value.
        let inked = try pixel(ground.view.overlay(Rectangle().foregroundStyle(style)), appearance)
        return Self.ratio(inked, under)
    }

    private func themes() async throws -> [Theme] {
        let cli = try #require(RealCLIPagesTests.cli, "no farcooler CLI in target/: build it first (apps/macos/test.sh does)")
        // `theme list` starts a daemon if none is running: give it a scratch
        // home and stop it after, so the run leaves no daemon behind.
        let home = "/tmp/fcq-\(UUID().uuidString.prefix(6))"
        var environment = ProcessInfo.processInfo.environment
        ScratchDaemon.isolate(&environment, home: home + "/h")
        let ran = await ProcessRunner.run(cli, ["--json", "theme", "list"], environment: environment, deadline: 30)
        await ScratchDaemon.stop(cli: cli, farcoolerHome: home + "/h")
        try? FileManager.default.removeItem(atPath: home)
        struct Reply: Decodable { var themes: [Theme] }
        let reply = try JSONDecoder().decode(Reply.self, from: ran.stdout)
        try #require(reply.themes.count >= 11, "the CLI listed \(reply.themes.count) themes")
        return reply.themes
    }

    /// Every theme, both appearances, every ground: the ratio of each ink.
    private func table() async throws -> [(row: String, secondary: Double, tertiary: Double, quiet: Double)] {
        let saved = Themes.shared.current.backgroundColor
        defer { ThemeBackdrop.set(saved) }
        var rows: [(String, Double, Double, Double)] = []
        for theme in try await themes() {
            ThemeBackdrop.set(theme.backgroundColor)
            for appearance in Self.appearances {
                for ground in Self.grounds {
                    rows.append((
                        "\(theme.name) / \(appearance == .aqua ? "light" : "dark") / \(ground.name)",
                        try contrast(.secondary, over: ground, appearance),
                        try contrast(.tertiary, over: ground, appearance),
                        try contrast(.quietInk, over: ground, appearance)))
                }
            }
        }
        return rows.map { (row: $0.0, secondary: $0.1, tertiary: $0.2, quiet: $0.3) }
    }

    @Test("Quiet ink is at least 4.5:1 on every theme's ground, in both appearances")
    func quietInkReads() async throws {
        let rows = try await table()
        print("QUIET-INK theme / appearance / ground | secondary | tertiary | quietInk")
        for r in rows {
            print(String(format: "QUIET-INK %@ | %.2f | %.2f | %.2f", r.row, r.secondary, r.tertiary, r.quiet))
        }
        for r in rows {
            #expect(r.quiet >= 4.5, "\(r.row): quiet ink is \(r.quiet):1")
        }
    }

    @Test("The check can fail: the system's .secondary is under 4.5:1 on a light ground")
    func secondaryFallsShort() async throws {
        let rows = try await table()
        #expect(rows.contains { $0.row.contains("/ light /") && $0.secondary < 4.5 })
    }

    @Test("Quiet ink is still a step under the body")
    func quietInkIsQuieter() throws {
        for appearance in Self.appearances {
            let ground = Self.grounds[0]
            let body = try contrast(.primary, over: ground, appearance)
            let quiet = try contrast(.quietInk, over: ground, appearance)
            #expect(quiet < body - 1, "quiet \(quiet):1 is not under body \(body):1")
        }
    }
}
