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
/// So this reads `Commands.swift` as text, the way the protocol tests read the
/// proto, and asks the sheet for each combination it finds. One direction
/// only: the sheet also lists keys no menu item carries (the ⌃B prefix keys
/// and ⌃HJKL, which are intercepted before the menu bar sees them), so "every
/// sheet row is a menu key" is not a rule this app keeps.
struct ShortcutSheetTests {
    /// `apps/macos/Sources/FarCooler/Commands.swift`, found from this file.
    private static let commands: String = {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // CeremonyTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // apps/macos
            .appendingPathComponent("Sources/FarCooler/Commands.swift")
        return (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }()

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
            // `Terminal \(n)`, bound in a `ForEach(1...9)`. The sheet lists
            // the range as ⌘1 … ⌘9, so its first member stands for it.
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
        #expect(spelled.contains("⇧⌘Z"))
        #expect(spelled.contains("⇧⌘Space"))
        #expect(spelled.contains("⌥⌘↓"))
    }

    @Test("Every menu bar shortcut is in the ⌘/ sheet")
    func everyMenuBarShortcutIsInTheSheet() {
        let missing = Self.scraped(Self.commands).spelled.filter { !Self.listed.contains($0) }
        #expect(missing.isEmpty, "in the menu bar and not in the ⌘/ sheet: \(missing)")
    }
}
