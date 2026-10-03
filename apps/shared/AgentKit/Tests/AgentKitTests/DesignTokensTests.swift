import SwiftUI
import Testing

@testable import AgentKit

// The scale and the numbers behind the tokens (ov-216). The lint
// (scripts/visual-tokens-lint.py) keeps views on the tokens; these pin what the
// tokens are.
@Suite struct DesignTokensTests {
    @Test func theScaleIsSixTenSixteen() {
        #expect([Radius.small, Radius.medium, Radius.large] == [6, 10, 16])
    }

    /// The composer padded 10 inside a card of 10 would be 0; the floor keeps it
    /// a rounded control.
    @Test func concentricNeverGoesBelowTheSmallStep() {
        #expect(Radius.concentric(outer: 10, padding: 10) == Radius.small)
        #expect(Radius.concentric(outer: 10, padding: 0) == 10)
        #expect(Radius.concentric(outer: 16, padding: 6) == 10)
        #expect(Radius.concentric(outer: 4, padding: 0) == Radius.small)
    }

    @Test func aWindowCornerOfSixteenMakesAMediumCardSitSixPointsIn() {
        #expect(16 - Radius.medium == 6)
    }

    @Test func insetAndSelectionStrengthenUnderIncreaseContrast() {
        #expect(Fill.insetOpacity(.standard) == 0.05)
        #expect(Fill.insetOpacity(.increased) == 0.10)
        #expect(Fill.selectionOpacity(active: true, contrast: .standard) == 0.13)
        #expect(Fill.selectionOpacity(active: true, contrast: .increased) == 0.25)
        #expect(Fill.selectionOpacity(active: false, contrast: .standard) == 0.09)
        #expect(Fill.selectionOpacity(active: false, contrast: .increased) == 0.09)
    }

    @Test func onlyTheContentLevelPaintsOpaqueAndTheWindowDrawsNothing() {
        #expect(Surface.content.paintsOpaque)
        #expect([Surface.window, .inset, .floating].allSatisfy { !$0.paintsOpaque })
        #expect(!Surface.window.drawsAnything)
        #expect([Surface.content, .inset, .floating].allSatisfy { $0.drawsAnything })
    }

    @Test func spacingIsTheEightPointRhythm() {
        #expect([Spacing.tight, Spacing.group, Spacing.inset, Spacing.section] == [4, 8, 12, 16])
    }
}
