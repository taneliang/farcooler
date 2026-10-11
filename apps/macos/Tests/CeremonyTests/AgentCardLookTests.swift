import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The glass family over a conversation has one radius (ov-223).
@MainActor
struct AgentCardLookTests {
    @Test("The plan, the queue and the composer share one concentric radius, never under six")
    func oneRadiusForTheFamily() {
        #expect(ChatFamily.radius == Radius.concentric(outer: Radius.medium, padding: 10))
        #expect(ChatFamily.radius >= Radius.small)
        // The composer was 20 and the queue's bubble 14.
        #expect(ChatFamily.radius < 14)
    }
}
