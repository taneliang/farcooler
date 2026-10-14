import Foundation
import Testing

@testable import AgentKit

// The words empty states use to invite (ov-519), before any platform draws them.

@Test func theInvitationNamesTheOrchestratorsJob() {
    #expect(FirstRunCopy.Conversation.inviteLede == "The orchestrator plans the work and starts agents on it.")
    #expect(FirstRunCopy.Conversation.inviteTitle == "Start the Orchestrator")
    // Said by what it does, never where it sits.
    for word in ["left", "right", "below", "above", "sidebar", "title bar"] {
        #expect(!PlanWords.nothingPlannedStart.lowercased().contains(word))
        #expect(!PlanWords.nothingPlannedRunning.lowercased().contains(word))
    }
    #expect(PlanWords.nothingPlannedStart.contains("Start it"))
}
