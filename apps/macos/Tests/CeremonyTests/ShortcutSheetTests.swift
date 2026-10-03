import Foundation
import Testing

@testable import Far_Cooler

/// Every key equivalent in the menu bar is also in the ⌘/ sheet.
///
/// Nothing tied the two together. `Commands.swift` declares what the keys do
/// and `Shortcuts.swift` lists them for a person to read, and a shortcut added
/// to the first and not the second is invisible from the sheet with no sign
/// anything is missing. ⇧⌘B was found that way by a reviewer, and ⇧⌘Z and
/// ⇧⌘Space, the Layout menu's zoom and cycle, had been missing longer.
///
/// It also refuses a chord the system's own menus already hold. Zoom Pane sat
/// on ⇧⌘Z beside Edit ▸ Redo, so the menu bar showed ⇧⌘Z twice and which one
/// a keypress reached depended on whether the field you were typing in had
/// anything to redo. It is on ⇧⌘↩ now.
///
/// So this reads `Commands.swift` as text, the way the protocol tests read the
/// proto, and asks the sheet for each combination it finds. One direction
/// only: the sheet also lists keys no menu item carries (the ⌃B prefix keys
/// and ⌃HJKL, which are intercepted before the menu bar sees them), so "every
/// sheet row is a menu key" is not a rule this app keeps.
struct ShortcutSheetTests {
    /// `apps/macos/Sources/FarCooler`, found from this file.
    private static let sources = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // CeremonyTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // apps/macos
        .appendingPathComponent("Sources/FarCooler")

    /// `Commands.swift`, the one file that builds the menu bar. The test
    /// below that no other file declares a menu is what makes reading only
    /// this one enough.
    private static let commands: String = {
        let url = sources.appendingPathComponent("Commands.swift")
        return (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }()

    /// Chords the system's standard menus hold on every Mac:
    ///
    /// - Edit: Undo, Redo, Cut, Copy, Paste, Select All, and Find's own
    ///   submenu (Find, Find Next and Previous, Use Selection for Find, Jump
    ///   to Selection, Find and Replace), and Emoji & Symbols.
    /// - The app menu: Settings, Hide, Hide Others, Quit.
    /// - File ▸ Close.
    /// - View: Show Sidebar, Show All Tabs, Enter Full Screen.
    /// - Window: Minimize.
    ///
    /// A menu item of this app's on one of them is the same chord in two
    /// menus, and the one a keypress reaches depends on menu order and on
    /// which of them is enabled at the time.
    static let systemChords: Set<String> = [
        "⌘Z", "⇧⌘Z", "⌘X", "⌘C", "⌘V", "⌘A",
        "⌘F", "⌘G", "⇧⌘G", "⌘E", "⌘J", "⌥⌘F", "⌃⌘Space",
        "⌘,", "⌘H", "⌥⌘H", "⌘Q",
        "⌘W",
        "⌃⌘S", "⇧⌘\\", "⌃⌘F",
        "⌘M",
    ]

    /// The system chords this app takes on purpose, what each does in which
    /// window, and why that is acceptable. A new entry needs the same.
    ///
    /// - ⌘W is Close Terminal, the tabbed-app convention the sheet opens
    ///   with, and it is ahead of File ▸ Close in the File menu. It is
    ///   ENABLED only while the main window is key (`MainWindowFocus`): there
    ///   ⌘W closes the selected terminal, and never the window. In any other
    ///   window (Settings, About) Close Terminal is disabled, the chord goes
    ///   on to File ▸ Close, and that window closes. Before that gate, ⌘W in
    ///   Settings stopped and removed the terminal behind it.
    /// - ⌘F is Find Workspace, Task or Agent, in the View menu, which comes after
    ///   Edit. So ⌘F reaches Find Worktree whenever Edit ▸ Find is disabled,
    ///   which is whenever the focused view offers no find: a terminal, the
    ///   sidebar, a diff. A text view that does offer find (the adapter
    ///   editor, a SwiftUI `TextEditor`, is one) takes ⌘F while it has focus
    ///   and finds in its own text, which is what someone typing in an editor
    ///   means by it. Not verified per field; that is the menu rule, stated.
    static let takenOnPurpose: Set<String> = ["⌘W", "⌘F"]

    /// One `.keyboardShortcut(key, modifiers: …)`, as the sheet would spell it.
    ///
    /// Modifiers in the order macOS draws them, ⌃⌥⇧⌘, whatever order the
    /// source lists them in. A letter is drawn capitalized, as a menu draws it.
    static func spelled(key: String, modifiers: String) -> String? {
        let order: [(String, String)] = [
            (".control", "⌃"), (".option", "⌥"), (".shift", "⇧"), (".command", "⌘"),
        ]
        let glyphs = order.filter { modifiers.contains($0.0) }.map(\.1).joined()
        let named: [String: String] = [
            ".space": "Space", ".upArrow": "↑", ".downArrow": "↓", ".leftArrow": "←",
            ".rightArrow": "→", ".return": "↩", ".delete": "⌫", ".escape": "⎋", ".tab": "⇥",
        ]
        let drawn: String
        if let special = named[key] {
            drawn = special
        } else if key.hasPrefix("\""), key.hasSuffix("\""), key.count == 3 {
            drawn = String(key.dropFirst().dropLast()).uppercased()
        } else if key.hasPrefix("KeyEquivalent(Character(\"\\(") {
            // `Workspace \(n)` and `Terminal \(n)`, each bound in a
            // `ForEach` over 1 to 9. The sheet lists a range as ⌘1 … ⌘9, so
            // its first member stands for it.
            drawn = "1"
        } else {
            return nil
        }
        return glyphs + drawn
    }

    /// Every declaration in `Commands.swift`, as the sheet would spell it, and
    /// every one the scraper could not read.
    static func scraped(_ source: String) -> (spelled: [String], unread: [String]) {
        let pattern = #/\.keyboardShortcut\(\s*(.+?),\s*modifiers:\s*(\[[^\]]*\]|\.[a-z]+)\s*\)/#
        var spelled: [String] = []
        var unread: [String] = []
        for line in source.split(separator: "\n") where line.contains(".keyboardShortcut(") {
            guard let match = line.firstMatch(of: pattern),
                let combination = Self.spelled(
                    key: String(match.output.1), modifiers: String(match.output.2))
            else {
                unread.append(line.trimmingCharacters(in: .whitespaces))
                continue
            }
            spelled.append(combination)
        }
        return (spelled, unread)
    }

    /// Every key the sheet lists, one combination per whitespace-separated
    /// token, so "⌥⌘↓ ⌥⌘↑" counts as both and "⌘1 … ⌘9" as ⌘1.
    private static var listed: Set<String> {
        Set(
            Shortcut.groups.flatMap { $0.1 }.flatMap {
                $0.keys.split(whereSeparator: \.isWhitespace).map(String.init)
            })
    }

    /// The scraper reads what it is pointed at. A regex that matched nothing
    /// would make the test below pass over an empty list.
    @Test("The scraper reads every declaration in Commands.swift")
    func theScraperReadsEveryDeclaration() {
        let (spelled, unread) = Self.scraped(Self.commands)
        #expect(unread.isEmpty, "declarations the scraper could not read: \(unread)")
        #expect(spelled.count >= 20, "only \(spelled.count) declarations found: \(spelled)")
        #expect(spelled.contains("⇧⌘↩"))
        #expect(spelled.contains("⇧⌘Space"))
        #expect(spelled.contains("⌥⌘↓"))
    }

    /// Red on ⇧⌘Z while Zoom Pane held it, which is the case it was written
    /// for.
    @Test("No menu bar shortcut takes a chord the system's menus already use")
    func noShortcutTakesASystemChord() {
        let taken = Self.scraped(Self.commands).spelled
            .filter { Self.systemChords.contains($0) && !Self.takenOnPurpose.contains($0) }
        #expect(taken.isEmpty, "bound here and in a standard system menu: \(taken)")
    }

    /// Reading only `Commands.swift` is only enough while it is the only file
    /// that builds a menu. A `CommandMenu` added anywhere else would carry
    /// shortcuts the two tests above cannot see.
    @Test("Only Commands.swift builds the menu bar")
    func onlyCommandsBuildsTheMenuBar() throws {
        // Every subdirectory too (`Ceremony/`), so a menu cannot hide in one.
        let walk = FileManager.default.enumerator(at: Self.sources, includingPropertiesForKeys: nil)
        let files = (walk?.allObjects as? [URL] ?? []).filter { $0.pathExtension == "swift" }
        #expect(files.count > 20, "only \(files.count) sources found at \(Self.sources.path)")
        let others = try files.filter { file in
            guard file.lastPathComponent != "Commands.swift" else { return false }
            let text = try String(contentsOf: file, encoding: .utf8)
            return text.contains("CommandMenu(") || text.contains("CommandGroup(")
        }.map(\.lastPathComponent)
        #expect(others.isEmpty, "menus built outside Commands.swift: \(others)")
    }

    @Test("Every menu bar shortcut is in the ⌘/ sheet")
    func everyMenuBarShortcutIsInTheSheet() {
        let missing = Self.scraped(Self.commands).spelled.filter { !Self.listed.contains($0) }
        #expect(missing.isEmpty, "in the menu bar and not in the ⌘/ sheet: \(missing)")
    }

    /// One chord, one menu item. Two items on one chord reach whichever the
    /// menu bar finds first, and which that is changes with what's enabled.
    /// It would have caught Back on ⌘[ (Previous Terminal) or Focus on
    /// ⇧⌘↩ (Zoom Pane), which is why they're on ⌃⌘← and ⌃⌘↩.
    @Test("No two menu bar items share a chord")
    func noTwoMenuBarItemsShareAChord() {
        let spelled = Self.scraped(Self.commands).spelled
        let counts = Dictionary(spelled.map { ($0, 1) }, uniquingKeysWith: +)
        let shared = counts.filter { $0.value > 1 }.keys.sorted()
        #expect(shared.isEmpty, "chords bound twice: \(shared)")
        #expect(spelled.contains("⌃⌘←") && spelled.contains("⌃⌘↩") && spelled.contains("⌥⌘1"))
    }

    /// The shortcut on the line after the menu item titled `title`.
    private static func shortcut(after title: String) -> String? {
        let lines = commands.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard let at = lines.firstIndex(where: { $0.contains("Button(\"\(title)\")") }), at + 1 < lines.count
        else { return nil }
        let pattern = #/\.keyboardShortcut\(\s*(.+?),\s*modifiers:\s*(\[[^\]]*\]|\.[a-z]+)\s*\)/#
        guard let match = lines[at + 1].firstMatch(of: pattern) else { return nil }
        return spelled(key: String(match.output.1), modifiers: String(match.output.2))
    }

    /// ⌘1 through ⌘9 go to the first nine workspaces (ov-86). They were the
    /// terminals on screen, which move to ⌃⌘1 through ⌃⌘9, beside ⌃⌘↑ and
    /// ⌃⌘↓, which walk a workspace's worktrees.
    @Test("⌘-numbers are workspaces, and the terminals moved to ⌃⌘-numbers")
    func numbersAreWorkspaces() {
        #expect(Self.shortcut(after: "Workspace \\(n)") == "⌘1")
        #expect(Self.shortcut(after: "Terminal \\(n)") == "⌃⌘1")
        #expect(Self.shortcut(after: "Next Worktree") == "⌃⌘↓")
        #expect(Self.shortcut(after: "Previous Worktree") == "⌃⌘↑")
        let rows = Shortcut.groups.flatMap { $0.1 }
        #expect(rows.first { $0.keys == "⌘1 … ⌘9" }?.action.contains("workspace") == true)
        #expect(rows.first { $0.keys == "⌃⌘1 … ⌃⌘9" }?.action.contains("terminal") == true)
    }

    /// A task's tabs take ⌃⌘] and ⌃⌘[ (ov-98), the owner's keys, so the
    /// diff's commits, which held them, move out one modifier to ⌃⌥⌘] and
    /// ⌃⌥⌘[, still the outermost list on the most modifiers.
    @Test("⌃⌘] and ⌃⌘[ walk a task's tabs, and the commits moved to ⌃⌥⌘")
    func taskTabsTakeControlCommandBrackets() {
        #expect(Self.shortcut(after: "Next Task Tab") == "⌃⌘]")
        #expect(Self.shortcut(after: "Previous Task Tab") == "⌃⌘[")
        #expect(Self.shortcut(after: "Next Commit") == "⌃⌥⌘]")
        #expect(Self.shortcut(after: "Previous Commit") == "⌃⌥⌘[")
        let rows = Shortcut.groups.flatMap { $0.1 }
        #expect(rows.first { $0.keys == "⌃⌘] ⌃⌘[" }?.action.contains("tab") == true)
        #expect(rows.first { $0.keys == "⌃⌥⌘] ⌃⌥⌘[" }?.action.contains("commit") == true)
    }

    /// "Repository" and "workspace" replaced "project" (spec §1): not in the
    /// ⌘/ sheet, and not in a menu item's title.
    @Test("No shortcut copy says project")
    func noShortcutCopySaysProject() {
        let sheet = Shortcut.groups.flatMap { [$0.0] + $0.1.flatMap { [$0.keys, $0.action] } }
        #expect(!sheet.contains { $0.lowercased().contains("project") }, "\(sheet.filter { $0.lowercased().contains("project") })")
        let titles = Self.commands.split(separator: "\n")
            .filter { $0.contains("Button(\"") || $0.contains("CommandMenu(\"") || $0.contains("Menu(\"") }
        #expect(!titles.contains { $0.lowercased().contains("project") }, "\(titles.filter { $0.lowercased().contains("project") })")
    }
}
