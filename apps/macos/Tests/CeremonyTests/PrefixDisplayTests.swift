import Foundation
import Testing

@testable import Far_Cooler

/// The tiling prefix is a setting, so nothing that names it may hard-code ⌃B.
struct PrefixDisplayTests {
    @Test("Each prefix reads as a person would say it")
    func displayNamesEachPrefix() {
        #expect(PrefixKey.display("b") == "⌃B")
        #expect(PrefixKey.display("a") == "⌃A")
        // The stored value is a space; uppercasing it drew "⌃" and a blank.
        #expect(PrefixKey.display(" ") == "⌃Space")
    }

    @Test("The shortcut sheet names the prefix that is set")
    func sheetNamesThePrefix() {
        let sheet = Shortcut.groups(prefix: "⌃A").flatMap { [$0.0] + $0.1.flatMap { [$0.keys, $0.action] } }
        #expect(sheet.contains("Tiling — press ⌃A, then"))
        #expect(!sheet.contains { $0.contains("⌃B") }, "\(sheet.filter { $0.contains("⌃B") })")
    }
}

extension PrefixDisplayTests {
    /// A prefix chord cannot be a key equivalent, so a menu title stays plain.
    @Test("Menu titles carry no chord")
    func noMenuTitleHardCodesThePrefix() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler/Commands.swift")
        let titles = try String(contentsOf: url, encoding: .utf8)
            .split(separator: "\n")
            .filter { $0.contains("Button(") && $0.contains("⌃") }
        #expect(titles.isEmpty, "\(titles)")
    }
}

extension PrefixDisplayTests {
    /// The HUD's label is `PrefixKey.current`, read from the stored prefix.
    @Test("The HUD shows the configured prefix")
    func hudShowsConfiguredPrefix() {
        let key = "tiling.prefixKey"
        let old = UserDefaults.standard.object(forKey: key)
        defer { UserDefaults.standard.set(old, forKey: key) }
        UserDefaults.standard.set(" ", forKey: key)
        #expect(PrefixKey.current == "⌃Space")
        UserDefaults.standard.set("a", forKey: key)
        #expect(PrefixKey.current == "⌃A")
    }
}

struct AdoptOnceTests {
    /// Return and a double-click can both reach Resume Branch's adopt; the
    /// second must be refused.
    @Test("A choice is adopted once")
    func adoptsOnce() {
        var gate = AdoptOnce()
        #expect(gate.begin())
        #expect(!gate.begin())
    }
}
