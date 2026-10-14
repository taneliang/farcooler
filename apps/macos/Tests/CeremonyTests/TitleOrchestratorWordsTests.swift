import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's center says what the orchestrator is doing, without its name or icon (ov-469).
@MainActor
@Suite(.serialized)
struct TitleOrchestratorWordsTests {
    @Test func theActivityPanelStillNamesTheOrchestrator() {
        #expect(TitleStatus.orchestratorWords(.working) == "Orchestrator · Working")
        #expect(TitleStatus.orchestratorWords(.needsYou) == "Orchestrator · Needs You")
        #expect(TitleStatus.orchestratorWords(OrchestratorRow.State.none) == "Orchestrator Not Started")
    }

    /// VoiceOver still names the orchestrator, with what it's doing last.
    @Test func voiceOverLabelNamesTheOrchestrator() {
        let model = TitleStatus.Model(
            orchestrator: .working, status: nil, nowDoing: "User test issues", needYou: 0, running: [], inReview: [])
        #expect(TitleStatus.orchestratorLabel(model) == "Orchestrator, Working, User test issues")
    }
}
