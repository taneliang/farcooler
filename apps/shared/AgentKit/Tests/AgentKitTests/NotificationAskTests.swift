import Testing

@testable import AgentKit

/// The permission prompt must never be the first thing a new person sees.
struct NotificationAskTests {
    @Test func aPhoneWithNoRunnerDoesNotAskAtLaunch() {
        #expect(!NotificationAsk.asksAtLaunch(hasRunners: false))
    }

    @Test func aPhoneWithARunnerStillAsksAtLaunch() {
        #expect(NotificationAsk.asksAtLaunch(hasRunners: true))
    }

    @Test func theExplainerComesWithTheFirstRunnerOnly() {
        #expect(NotificationAsk.explainsAfterFirstRunner(hadRunners: false, hasRunners: true))
        #expect(!NotificationAsk.explainsAfterFirstRunner(hadRunners: true, hasRunners: true))
        #expect(!NotificationAsk.explainsAfterFirstRunner(hadRunners: true, hasRunners: false))
    }
}
