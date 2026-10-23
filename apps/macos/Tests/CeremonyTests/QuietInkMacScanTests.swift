import Foundation
import Testing

/// The Mac app's source scan for quiet text in the system's `.secondary` or
/// `.tertiary` (ov-687, the fifth batch), as AgentKit's
/// (`QuietInkTests.noSystemQuietTextInAgentKit`) is: a file not on the list
/// draws no text in a system ink, so a new site on our own ground goes red here.
///
/// A file is listed only for a ground the system owns, with its reason, and the
/// test fails when a listed file no longer has a site, so the list can't go
/// stale. A mark (an `Image`) is exempt, and so is a line that says why with
/// `// style-exempt: <reason>` on it or the line above, for the one site in a
/// file that otherwise reads on our own ground.
///
/// Where each ground's quiet text goes:
/// - the paper, the window's ground and the terminal theme's: `.quietInk`;
/// - the frosted plane (the sidebar and the chrome around the cards):
///   `SidebarInk.secondary`, which `NavigatorInkTests` also holds the
///   navigator's own files to;
/// - system material (a sheet, a grouped Form, a popover, a floating panel's
///   glass, a toolbar item): the system's inks, which it keeps readable there.
struct QuietInkMacScanTests {
    /// Whole files on a ground the system owns.
    static let systemMaterial: [String: String] = [
        // Sheets: the system's sheet material.
        "AboutSheet": "the About sheet",
        "AddFlow": "the Add a Runner sheet",
        "Ceremony/AddDeviceView": "the Add a Device sheet",
        "Ceremony/JoinView": "the Join sheet",
        "Ceremony/RemoteLoginView": "the Remote Login sheet",
        "Ceremony/TeammateJoinPrompt": "a prompt over the window, as a sheet",
        "ClaudeSignInSheet": "the Claude sign-in sheet",
        "NewWorkspaceSheet": "a sheet whose Form is grouped",
        "ResumeBranch": "the Resume Branch sheet",
        "RunnerTeamSheet": "the runner's team sheet",
        "Sheets": "the New Worktree, Remove and Add Repository sheets and their grouped Forms",
        "Shortcuts": "the Shortcuts sheet",
        // Settings: grouped Forms, whose rows the system draws on its own cells.
        "EditorsSettings": "a grouped Form in Settings",
        "MemberLoginsSection": "a Section of a grouped Form in Settings",
        "NativeAgent/ConversationPairingSection": "a Section of the grouped Form in Settings",
        "Preferences": "Settings' grouped Forms; its one themed swatch draws in the theme's own colors",
        "RunnerPeopleSection": "a Section of a grouped Form in Settings",
        "RunnerSettings": "a grouped Form in a sheet",
        "RunnersSettings": "a grouped Form in Settings",
        "FieldPrompt": "a text field's prompt inside a grouped Form",
        // Floating panels and popovers: Liquid Glass or the popover's material.
        "DaemonSkew": "the update card, in a popover",
        "NativeAgent/ComposerStyle": "the composer's marks, on its glass box; NativeComposerAppearanceTests measures them",
        "QuickCreate": "the quick-create panel, a floating panel's glass",
        "TitleActivity": "the Spent Today popover",
        "TitleBarStatus": "toolbar items and the popover they open",
        "TitleConsole": "the title console's palette, a floating panel's glass",
        "WorkspaceAgents": "the Agents popover",
        "WorkspaceSwitcher": "toolbar items and the update popover",
        // Line by line elsewhere.
        "ChangesPane": "its popovers are system material, and `noSystemQuietTextOnPaper` holds the rest to quiet ink",
    ]

    /// Text, a mark, or an NSColor in a system ink. `(?<![A-Za-z0-9_])` keeps
    /// `SidebarInk.secondary` and `PaneText.secondary` out.
    static let systemInk = try! NSRegularExpression(
        pattern: #"foreground(Style|Color)\([^\n]*(?<![A-Za-z0-9_])\.(secondary|tertiary)\b|foregroundColor = (?<![A-Za-z0-9_])\.(secondary|tertiary)\b|\b(secondary|tertiary)LabelColor\b|AnyShapeStyle\((?<![A-Za-z0-9_])\.(secondary|tertiary)\)|\bColor\.(secondary|tertiary)\b|return (?<![A-Za-z0-9_])\.(secondary|tertiary)\b"#)

    static let exempt = try! NSRegularExpression(pattern: #"//\s*style-exempt\s*:\s*\S"#)

    /// The sites in `text` that draw in a system ink: a line that matches, is
    /// no comment, has no exemption on it or above it, and whose statement (up
    /// to the first line that doesn't start a modifier) is not an `Image`.
    static func sites(in text: String) -> [(line: Int, text: String)] {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
        func matches(_ r: NSRegularExpression, _ l: String) -> Bool { r.firstMatch(in: l, range: NSRange(l.startIndex..., in: l)) != nil }
        var found: [(Int, String)] = []
        for (i, l) in lines.enumerated() where !l.hasPrefix("//") && matches(systemInk, l) {
            if matches(exempt, l) || (i > 0 && matches(exempt, lines[i - 1])) { continue }
            var head = i
            while head > 0, lines[head].hasPrefix(".") { head -= 1 }
            if lines[head].contains("Image(") { continue }
            found.append((i + 1, l))
        }
        return found
    }

    @Test("Every Mac source file but the system-material ones draws no text in .secondary or .tertiary")
    func noSystemQuietTextInTheMacApp() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let sources = root.appendingPathComponent("Sources/FarCooler")
        let walker = try #require(FileManager.default.enumerator(atPath: sources.path))
        var files: [String] = []
        for case let path as String in walker where path.hasSuffix(".swift") { files.append(path) }
        try #require(files.count > 150, "found \(files.count) Mac source files")
        var listedSites = Set<String>()
        for file in files {
            let name = String(file.dropLast(".swift".count))
            let sites = Self.sites(in: try String(contentsOf: sources.appendingPathComponent(file), encoding: .utf8))
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

    @Test("The scan can fail: text in .secondary, a theme-less NSColor or a ternary is a site; a mark, SidebarInk and an exemption are not")
    func theMacScanSeesASite() {
        #expect(Self.sites(in: "Text(\"a\")\n    .foregroundStyle(.secondary)").count == 1)
        #expect(Self.sites(in: "Text(\"a\").foregroundStyle(.tertiary)").count == 1)
        #expect(Self.sites(in: ".foregroundStyle(on ? .primary : .secondary)").count == 1)
        #expect(Self.sites(in: ".foregroundStyle(on ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))").count == 1)
        #expect(Self.sites(in: ".foregroundColor: NSColor.tertiaryLabelColor,").count == 1)
        #expect(Self.sites(in: "tint: Color.secondary)").count == 1)
        // A Text a line under an Image is still text.
        #expect(Self.sites(in: "Image(systemName: \"key\")\nText(\"a\")\n    .foregroundStyle(.secondary)").count == 1)
        #expect(Self.sites(in: "Image(systemName: \"key\")\n    .font(.caption)\n    .foregroundStyle(.tertiary)").isEmpty)
        #expect(Self.sites(in: ".foregroundStyle(SidebarInk.secondary)").isEmpty)
        #expect(Self.sites(in: ".foregroundStyle(.quietInk)").isEmpty)
        #expect(Self.sites(in: ".font(.system(size: WorkspaceStyle.PaneText.secondary))").isEmpty)
        #expect(Self.sites(in: ".foregroundStyle(.secondary)  // style-exempt: a mark").isEmpty)
        #expect(Self.sites(in: "// style-exempt: a mark\n.foregroundStyle(.secondary)").isEmpty)
        #expect(Self.sites(in: ".foregroundStyle(.secondary)  // style-exempt:").count == 1)
        #expect(Self.sites(in: "// .foregroundStyle(.secondary)").isEmpty)
    }

    /// The scan against the tree as it is: it finds the sites the list names,
    /// so it is reading the same files the list was made from.
    @Test("The scan reads the real tree: a planted site in a real file is found")
    func theScanReadsTheTree() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let text = try String(contentsOf: root.appendingPathComponent("Sources/FarCooler/GroupBar.swift"), encoding: .utf8)
        #expect(Self.sites(in: text).isEmpty, "GroupBar draws on the plane in SidebarInk")
        #expect(Self.sites(in: text + "\nText(\"x\").foregroundStyle(.secondary)\n").count == 1)
    }
}
