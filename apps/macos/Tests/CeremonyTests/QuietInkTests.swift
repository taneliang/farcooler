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

    // MARK: The Plan view and the task card (ov-687)

    /// The surfaces whose quiet text moved to `Ink.quiet`, with the ground each
    /// is drawn on: every one is a content card on `WorkspaceStyle.paper`
    /// (`contentCard()`), and a decision's note sits on the inset wash over it.
    static let surfaces: [(name: String, ground: Int)] = [
        ("Plan home and peek", 0), ("Plan theme page", 0), ("Plan lane page", 0), ("Plan orchestrator page", 0),
        ("Plan train card", 0), ("Plan Needs You page", 0), ("Plan rulings", 0),
        ("Task card: intent, acceptance, constraints", 0), ("Task card: usage and subagents", 0),
        ("Task card: record, a decision note", 1), ("Board history", 0), ("Unread summary", 0),
    ]

    @Test("Each changed surface's quiet ink is at least 4.5:1 on its ground, light and dark, and the system's is printed beside it")
    func changedSurfacesRead() async throws {
        let saved = Themes.shared.current.backgroundColor
        defer { ThemeBackdrop.set(saved) }
        // Every built-in theme: the tint is 3% or 9% of its ground.
        for theme in try await themes() {
            ThemeBackdrop.set(theme.backgroundColor)
            for appearance in Self.appearances {
                for surface in Self.surfaces {
                    let ground = Self.grounds[surface.ground]
                    let quiet = try contrast(.quietInk, over: ground, appearance)
                    let secondary = try contrast(.secondary, over: ground, appearance)
                    let tertiary = try contrast(.tertiary, over: ground, appearance)
                    if theme.name == "Solarized Light" || theme.name == "Nord" {
                        print(String(
                            format: "QUIET-SURFACE %@ / %@ / %@ | secondary %.2f | tertiary %.2f | quiet %.2f", surface.name,
                            theme.name, appearance == .aqua ? "light" : "dark", secondary, tertiary, quiet))
                    }
                    #expect(quiet >= 4.5, "\(surface.name) / \(theme.name): quiet ink is \(quiet):1")
                }
            }
        }
    }

    /// Text drawn in the system's `.secondary` or `.tertiary` in the files that
    /// hold the changed surfaces: none, but for what is named below.
    @Test("The Plan view and the task card draw no text in .secondary or .tertiary on paper")
    func noSystemQuietTextOnPaper() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let sources = root.appendingPathComponent("Sources/FarCooler")
        let files = [
            "Plan/PlanPages", "Plan/PlanOverview", "Plan/PlanRulings", "Plan/PlanTrainCard", "Plan/PlanThemeGridView",
            "Plan/PlanOrchestratorPages", "Plan/PlanAgentsView", "Plan/PlanHome", "Plan/PlanOutsideRow",
            "Plan/ContentView+OneTree", "TaskColumn", "TaskUsageView", "BoardHistoryView", "Components/TaskWorkers",
            "BoardSummaryStrip", "TaskNoteStyle", "TaskBoard",
        ]
        // Left on purpose: a disabled control (Ask, when it can't send) is exempt from contrast,
        // (a rule or a bar's fill is not text, and none of those goes through foregroundStyle).
        let allowed = ["actions.canAsk ? .secondary : .tertiary"]
        let pattern = try NSRegularExpression(pattern: #"foregroundStyle\([^\n]*\.(secondary|tertiary)\b|\bColor\.secondary\b|case \.secondary: return \.secondary"#)
        for file in files {
            var text = try String(contentsOf: sources.appendingPathComponent(file + ".swift"), encoding: .utf8)
            // The board's list is on the sidebar; the card is after this line.
            if file == "TaskBoard", let cut = text.range(of: "struct TaskCard: View") { text = String(text[cut.lowerBound...]) }
            for line in text.split(separator: "\n") {
                let l = String(line)
                guard !l.contains("SidebarInk.secondary"), !allowed.contains(where: l.contains) else { continue }
                let found = pattern.firstMatch(in: l, range: NSRange(l.startIndex..., in: l)) != nil
                #expect(!found, "\(file): \(l.trimmingCharacters(in: .whitespaces))")
            }
        }
    }
}
