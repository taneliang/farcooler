import Foundation
import Testing

@testable import AgentKit

/// A table cell's web link says where it goes to VoiceOver as well as on
/// screen (ov-285, review L2): every link shows its domain.
struct PageSpokenLinkTests {
    static let cell = PageCell(text: "Click here", ref: PageRef(.url("https://evil.example/x")))
    static let columns = [PageColumn(title: "Lane"), PageColumn(title: "Run")]

    @Test("a row reads a cell's link with its domain")
    func aRowSaysTheDomain() {
        let spoken = PageLayout.spokenRow(columns: Self.columns, cells: [PageCell(text: "mac-ux"), Self.cell], world: PageWorld())
        #expect(spoken == "Lane, mac-ux. Run, Click here, link to evil.example.")
    }

    @Test("a cell's Open action names the domain; one whose words are the domain doesn't repeat it")
    func theActionSaysTheDomain() {
        #expect(PageWorld().actions([Self.cell]).map(\.name) == ["Open Click here, link to evil.example"])
        let bare = PageCell(ref: PageRef(.url("https://github.com/x")))
        #expect(PageWorld().actions([bare]).map(\.name) == ["Open github.com"])
    }
}
