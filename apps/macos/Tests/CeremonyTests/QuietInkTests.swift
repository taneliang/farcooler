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
        // The Changes pane's strip of controls, and a file's heading in the diff:
        // the pane chrome, at the opacity each is drawn with, over the paper.
        Ground(
            name: "pane strip",
            view: AnyView(Color.clear.background(WorkspaceStyle.paper).overlay(WorkspaceStyle.paneChrome.opacity(0.60)))),
        Ground(
            name: "file header",
            view: AnyView(Color.clear.background(WorkspaceStyle.paper).overlay(WorkspaceStyle.fileHeader))),
        // The line-number gutter: the inset wash over the paper, over a context
        // line, an added one and a removed one (their wash is under the gutter).
        Ground(name: "gutter", view: AnyView(Color.clear.background(WorkspaceStyle.paper).overlay(WorkspaceStyle.diffGutter))),
        Ground(
            name: "gutter, added line",
            view: AnyView(
                Color.clear.background(WorkspaceStyle.paper).overlay(DiffComputation.Kind.added.wash)
                    .overlay(WorkspaceStyle.diffGutter))),
        Ground(
            name: "gutter, removed line",
            view: AnyView(
                Color.clear.background(WorkspaceStyle.paper).overlay(DiffComputation.Kind.removed.wash)
                    .overlay(WorkspaceStyle.diffGutter))),
        // The amber strip the Changes pane says a failed read in.
        Ground(
            name: "attention strip",
            view: AnyView(Color.clear.attentionSurface(in: Rectangle()).background(WorkspaceStyle.paper))),
        // The window's own ground, where a page the app draws no card for sits
        // (the Charter), and the inset wash over it (its answers' box).
        Ground(name: "window", view: AnyView(Color.clear.background(Color(nsColor: .windowBackgroundColor)))),
        Ground(
            name: "window inset",
            view: AnyView(Color.clear.background(Color(nsColor: .windowBackgroundColor)).overlay(Fill.inset()))),
        // The Charter's bar: the system's text background, with no theme in it.
        Ground(name: "plain paper", view: AnyView(Color.clear.background(Surface.contentFill))),
        // The window's ground as macOS drew it before 26, and as a user's Increase
        // Contrast or a future release may again: the classic light gray, 236 of 255 (dark stays 30).
        // The "window" ground above reads the same as "plain paper" because on
        // today's macOS they are the same color (`windowGroundIsTheSystemsColor`),
        // so this one holds the lighter-gray case the Charter would meet.
        Ground(
            name: "window, classic gray",
            view: AnyView(
                Color.clear.background(
                    Color(nsColor: NSColor(name: nil) { $0.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? NSColor(white: 30 / 255, alpha: 1) : NSColor(white: 236 / 255, alpha: 1) })))),
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

    /// The window ground reads the same as the plain paper (3.95:1 for `.secondary`
    /// in light, 5.92:1 in dark, on every theme) and the question was whether the
    /// sampler read the wrong color. It does not: the pixel SwiftUI draws is the
    /// color AppKit resolves for `windowBackgroundColor` in that appearance, and on
    /// this macOS that color is the text background's, 255 in light and 30 in dark.
    @Test("The window ground is the system's windowBackgroundColor, which is the text background here")
    func windowGroundIsTheSystemsColor() throws {
        for appearance in Self.appearances {
            let drawn = try pixel(Self.grounds[9].view, appearance)
            var system = NSColor.clear
            var text = NSColor.clear
            NSAppearance(named: appearance)?.performAsCurrentDrawingAppearance {
                system = NSColor.windowBackgroundColor.usingColorSpace(.sRGB) ?? .clear
                text = NSColor.textBackgroundColor.usingColorSpace(.sRGB) ?? .clear
            }
            func gap(_ a: NSColor, _ b: NSColor) -> CGFloat {
                max(abs(a.redComponent - b.redComponent), abs(a.greenComponent - b.greenComponent), abs(a.blueComponent - b.blueComponent)) * 255
            }
            #expect(gap(drawn, system) <= 1.5, "\(appearance.rawValue): the sampler drew \(drawn), AppKit resolves \(system)")
            print(String(format: "QUIET-WINDOW %@ | window-vs-text-background gap %.1f / 255", appearance.rawValue, gap(system, text)))
        }
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
        // The second batch: the Changes and Files panes, and the diff.
        ("Changes: strip of controls", 3), ("Changes: file column and diff rows", 0), ("Changes: file headings", 4),
        ("Changes: line-number gutter", 5), ("Changes: gutter on an added line", 6), ("Changes: gutter on a removed line", 7),
        ("Changes: read problem", 8), ("Files: header and find bar", 4), ("Files: tree and filtered results", 0),
        ("Files: line numbers", 5), ("Diff view: code box", 1), ("Diff read failure row", 0),
        // The third batch: setup, panes and the changes notice.
        ("Welcome: page text", 0), ("Welcome: checks box", 1), ("Charter: page text", 9), ("Charter: answers box", 10),
        ("Charter: action bar", 11), ("Rep column", 0), ("Pane header", 0), ("Changes: LFS notice", 0),
        // The fourth batch: AgentKit's shared Plan cost and page views, drawn on the paper here.
        ("Plan cost: week, cost per card, a theme's spend", 0), ("Orchestrator page: text, stats, tables, lists, timeline", 0),
        ("Orchestrator page: a list's state words and details", 1),
        // The fifth batch: the cards' own grounds. The plane's text (the navigator's row, the
        // tab strip, the jump bar, the empty main area) is `SidebarInk`, measured on the plane
        // in `UnifiedChromeTests`; what is here is quiet ink on a card or a window.
        ("Needs You page", 0), ("Task view: terminals strip", 0), ("Pane: held draft bar", 0), ("Pane: web page header", 0),
        ("Charter: page text on the classic gray window", 12),
    ]

    @Test("Each changed surface's quiet ink is at least 4.5:1 on its ground, light and dark, and the system's is printed beside it")
    func changedSurfacesRead() async throws {
        let saved = Themes.shared.current.backgroundColor
        defer { ThemeBackdrop.set(saved) }
        // Every built-in theme: the tint is 3% or 9% of its ground.
        for theme in try await themes() {
            ThemeBackdrop.set(theme.backgroundColor)
            for appearance in Self.appearances {
                var measured: [Int: (Double, Double, Double)] = [:]
                for surface in Self.surfaces {
                    let ground = Self.grounds[surface.ground]
                    if measured[surface.ground] == nil {
                        measured[surface.ground] = (
                            try contrast(.quietInk, over: ground, appearance),
                            try contrast(.secondary, over: ground, appearance),
                            try contrast(.tertiary, over: ground, appearance))
                    }
                    let (quiet, secondary, tertiary) = try #require(measured[surface.ground])
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
    @Test("The Plan view, the task card, the Changes and Files panes, the diff, the Welcome and Charter pages, a rep's column and a pane's header draw no text in .secondary or .tertiary on paper")
    func noSystemQuietTextOnPaper() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let sources = root.appendingPathComponent("Sources/FarCooler")
        let files = [
            "Plan/PlanPages", "Plan/PlanOverview", "Plan/PlanRulings", "Plan/PlanTrainCard", "Plan/PlanThemeGridView",
            "Plan/PlanOrchestratorPages", "Plan/PlanAgentsView", "Plan/PlanHome", "Plan/PlanOutsideRow",
            "Plan/ContentView+OneTree", "TaskColumn", "TaskUsageView", "BoardHistoryView", "Components/TaskWorkers",
            "BoardSummaryStrip", "TaskNoteStyle", "TaskBoard",
            "ChangesPane", "ChangesLfsNotice", "RepColumn", "WelcomeView", "CharterSetupView", "TileView", "Files/FilesPane", "Files/CodeLineRow", "Files/OpenInFiles", "DiffView", "DiffReadFailureRow",
        ]
        // Left on purpose: a disabled control (Ask, when it can't send) is exempt from contrast,
        // (a rule or a bar's fill is not text, and none of those goes through foregroundStyle).
        // A tree row the app can't open (a socket, say) is drawn as a disabled control.
        let allowed = ["actions.canAsk ? .secondary : .tertiary", "row.kind == .other ? .secondary : .primary"]
        // The Changes pane's popovers (the commit list, the file jumper, the note
        // composer, the outbox) are system material, which keeps the system's inks.
        let popovers = [
            "ChangesPane": [
                ("private var commitList: some View", "private var fileColumn: some View"),
                ("private struct ReviewNoteComposer: View", "private struct DiffGapControl: View"),
                ("private struct ReviewOutbox: View", "private struct FileStatusBadge: View"),
            ]
        ]
        let pattern = try NSRegularExpression(
            pattern: #"foregroundStyle\([^\n]*\.(secondary|tertiary)\b|\bColor\.secondary\b|case \.secondary: return \.secondary|isEmpty \? AnyShapeStyle\(\.(secondary|tertiary)\)|return \.(secondary|tertiary)\b"#)
        for file in files {
            var text = try String(contentsOf: sources.appendingPathComponent(file + ".swift"), encoding: .utf8)
            // The board's list is on the sidebar; the card is after this line.
            if file == "TaskBoard", let cut = text.range(of: "struct TaskCard: View") { text = String(text[cut.lowerBound...]) }
            for (from, to) in popovers[file] ?? [] {
                let start = try #require(text.range(of: from), "\(file): no \(from)")
                let end = try #require(text.range(of: to), "\(file): no \(to)")
                try #require(start.lowerBound < end.lowerBound, "\(file): \(from) is not before \(to)")
                text.removeSubrange(start.lowerBound..<end.lowerBound)
            }
            for line in text.split(separator: "\n") {
                let l = String(line)
                guard !l.contains("SidebarInk.secondary"), !allowed.contains(where: l.contains) else { continue }
                let found = pattern.firstMatch(in: l, range: NSRange(l.startIndex..., in: l)) != nil
                #expect(!found, "\(file): \(l.trimmingCharacters(in: .whitespaces))")
            }
        }
    }

    /// Every AgentKit source file that is not on this list draws no text in
    /// the system's `.secondary` or `.tertiary`: its quiet text is
    /// `Ink.quiet`. A mark (an `Image`) is exempt. A file is listed only for
    /// a ground the system owns, with its reason, and the test fails if a
    /// listed file no longer has a site, so the list can't go stale.
    static let systemMaterial: [String: String] = [
        "PhoneRekeySection": "a Section of the iPhone's grouped Form",
        "AccountDevicesView": "Sections of a grouped Form",
        "AccountSection": "a Section of a grouped Form",
        "AccountDeviceKeys": "a Section of a grouped Form",
        "RendezvousSection": "a Section of a grouped Form",
        "RelaySection": "a Section of a grouped Form",
        "TaskKeyCardView": "the hovercard is a Mac popover and an iOS dialog, both system material",
    ]

    /// `.foregroundStyle(... .secondary)` and the older `.foregroundColor(.secondary)`,
    /// each with `.tertiary`, and an `AttributedString` run set to one.
    static let systemInk = try! NSRegularExpression(
        pattern: #"foreground(Style|Color)\([^\n]*\.(secondary|tertiary)\b|foregroundColor = \.(secondary|tertiary)\b"#)

    /// The sites in `text` that draw text in a system ink: a line that matches
    /// and whose statement, up to the first line that doesn't start a modifier,
    /// is not an `Image`.
    static func systemInkSites(in text: String) -> [(line: Int, text: String)] {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
        var found: [(Int, String)] = []
        for (i, l) in lines.enumerated() where !l.hasPrefix("//") {
            guard systemInk.firstMatch(in: l, range: NSRange(l.startIndex..., in: l)) != nil else { continue }
            var head = i
            while head > 0, lines[head].hasPrefix(".") { head -= 1 }
            if lines[head].contains("Image(") { continue }
            found.append((i + 1, l))
        }
        return found
    }

    @Test("Every AgentKit file but the system-material ones draws no text in .secondary or .tertiary")
    func noSystemQuietTextInAgentKit() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let sources = root.appendingPathComponent("apps/shared/AgentKit/Sources/AgentKit")
        let files = try FileManager.default.contentsOfDirectory(atPath: sources.path).filter { $0.hasSuffix(".swift") }
        try #require(files.count > 100, "found \(files.count) AgentKit files")
        var listedSites = Set<String>()
        for file in files {
            let name = String(file.dropLast(".swift".count))
            let sites = Self.systemInkSites(in: try String(contentsOf: sources.appendingPathComponent(file), encoding: .utf8))
            if Self.systemMaterial[name] != nil {
                if !sites.isEmpty { listedSites.insert(name) }
                continue
            }
            for site in sites { #expect(Bool(false), "\(name):\(site.line): \(site.text)") }
        }
        for name in Self.systemMaterial.keys {
            #expect(listedSites.contains(name), "\(name) is listed as system material and has no system-ink site: take it off the list")
        }
    }

    @Test("The scan can fail: a text in .secondary or .foregroundColor(.tertiary) is a site, a symbol is not")
    func theAgentKitScanSeesASite() {
        #expect(Self.systemInkSites(in: "Text(\"a\")\n    .foregroundStyle(.secondary)").count == 1)
        #expect(Self.systemInkSites(in: "Text(\"a\").foregroundColor(.tertiary)").count == 1)
        #expect(Self.systemInkSites(in: "x.foregroundColor = .secondary").count == 1)
        // A Text a line under an Image is still text.
        #expect(Self.systemInkSites(in: "Image(systemName: \"key\")\nText(\"a\")\n    .foregroundStyle(.secondary)").count == 1)
        #expect(Self.systemInkSites(in: "Image(systemName: \"key\")\n    .font(.caption)\n    .foregroundStyle(.tertiary)").isEmpty)
    }

    // MARK: The iPhone's grounds (ov-687)

    /// iOS's system colors in sRGB, as Apple publishes them: the ground and
    /// the label's primary ink, light and dark. The Mac test host can't draw
    /// UIKit, so these are the documented values; the iPhone captures in the
    /// lane's report are the check that they hold on a device.
    static let phoneGrounds: [(name: String, light: (Double, Double, Double), dark: (Double, Double, Double))] = [
        ("systemBackground", (255, 255, 255), (0, 0, 0)),
        ("secondarySystemBackground", (242, 242, 247), (28, 28, 30)),
        ("systemGroupedBackground", (242, 242, 247), (0, 0, 0)),
        ("secondarySystemGroupedBackground", (255, 255, 255), (28, 28, 30)),
    ]

    private static func srgb(_ c: (Double, Double, Double)) -> NSColor {
        NSColor(srgbRed: c.0 / 255, green: c.1 / 255, blue: c.2 / 255, alpha: 1)
    }

    /// `ink` at `opacity` over `ground`, as a flat color.
    private static func over(_ ink: (Double, Double, Double), _ opacity: Double, _ ground: (Double, Double, Double)) -> NSColor {
        srgb((ink.0 * opacity + ground.0 * (1 - opacity), ink.1 * opacity + ground.1 * (1 - opacity), ink.2 * opacity + ground.2 * (1 - opacity)))
    }

    @Test("Quiet ink is at least 4.5:1 on the iPhone's grounds, light and dark, and the system's secondary label is not")
    func quietInkReadsOnThePhone() {
        for g in Self.phoneGrounds {
            for (scheme, ground, label, secondary, secondaryOpacity) in [
                ("light", g.light, (0.0, 0.0, 0.0), (60.0, 60.0, 67.0), 0.6),
                ("dark", g.dark, (255.0, 255.0, 255.0), (235.0, 235.0, 245.0), 0.6),
            ] {
                let under = Self.srgb(ground)
                let quiet = Self.ratio(Self.over(label, Ink.quietOpacity, ground), under)
                let system = Self.ratio(Self.over(secondary, secondaryOpacity, ground), under)
                print(String(format: "QUIET-PHONE %@ / %@ | secondaryLabel %.2f | quietInk %.2f", g.name, scheme, system, quiet))
                #expect(quiet >= 4.5, "\(g.name) / \(scheme): quiet ink is \(quiet):1")
                // The check can fail: the same arithmetic puts the secondary
                // label under 4.5:1 on every light ground.
                if scheme == "light" { #expect(system < 4.5, "\(g.name): secondaryLabel is \(system):1") }
            }
        }
    }
}
