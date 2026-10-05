import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's center says whose state it shows (ov-320).
@MainActor
@Suite(.serialized)
struct TitleOrchestratorWordsTests {
    @Test func statesNameTheOrchestrator() {
        #expect(TitleStatus.orchestratorWords(.working) == "Orchestrator · Working")
        #expect(TitleStatus.orchestratorWords(.needsYou) == "Orchestrator · Needs You")
        #expect(TitleStatus.orchestratorWords(OrchestratorRow.State.none) == "No Orchestrator")
    }

    @Test func tooltipSaysWhatItShows() {
        #expect(TitleStatus.orchestratorHelp == "The orchestrator’s state and its current session")
    }

    /// VoiceOver reads the same naming, with what it's doing last.
    @Test func voiceOverLabelNamesTheOrchestrator() {
        let model = TitleStatus.Model(
            orchestrator: .working, status: nil, nowDoing: "User test issues", needYou: 0, running: [], inReview: [])
        #expect(TitleStatus.orchestratorLabel(model) == "Orchestrator, Working, User test issues")
    }
}

/// The orchestrator's symbol stands before its name where the mark is a
/// status dot, and not twice where the mark is that symbol (ov-320).
@MainActor
@Suite struct TitleOrchestratorGlyphTests {
    @Test func glyphBeforeTheNameWhileTheMarkIsADot() {
        #expect(TitleStatus.showsGlyph(.working, form: .medium))
        #expect(TitleStatus.showsGlyph(.needsYou, form: .wide))
        #expect(TitleStatus.showsGlyph(.failed(.failed), form: .wide))
    }

    @Test func noGlyphWhereTheMarkIsTheSymbolOrNoNameShows() {
        #expect(!TitleStatus.showsGlyph(.idle, form: .wide))
        #expect(!TitleStatus.showsGlyph(OrchestratorRow.State.none, form: .wide))
        #expect(!TitleStatus.showsGlyph(.working, form: .short))
    }
}
