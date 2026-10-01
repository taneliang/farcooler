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
    /// The Layout menu named ⌃B in every title whatever the setting said.
    @Test("A Layout menu title names the prefix that is set")
    func menuTitlesFollowThePrefix() {
        #expect(PrefixKey.menuTitle("Split Right", keys: "%", stored: "b") == "Split Right (⌃B %)")
        #expect(PrefixKey.menuTitle("Split Right", keys: "%", stored: "a") == "Split Right (⌃A %)")
        #expect(PrefixKey.menuTitle("Zoom Pane", keys: "z", stored: " ") == "Zoom Pane (⌃Space z)")
    }
}

extension PrefixDisplayTests {
    /// A menu item that types ⌃B into its own title goes on saying ⌃B after the
    /// setting changes, so Commands.swift may not contain the literal in a
    /// button title.
    @Test("No menu title hard-codes the default prefix")
    func noMenuTitleHardCodesThePrefix() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler/Commands.swift")
        let titles = try String(contentsOf: url, encoding: .utf8)
            .split(separator: "\n")
            .filter { $0.contains("Button(\"") && $0.contains("⌃B") }
        #expect(titles.isEmpty, "\(titles)")
    }
}
