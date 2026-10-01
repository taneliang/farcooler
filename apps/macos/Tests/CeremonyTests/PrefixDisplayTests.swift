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
