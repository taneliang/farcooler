import Foundation
import Testing

@testable import AgentKit

// First run (ov-205): the Welcome rows, the "not installed" exit, which
// harnesses a runner can start, and the words for all of it. Android mirrors
// these in `apps/android/.../model/FirstRunTest.kt`.

// MARK: - Welcome rows

private let claudeAndCodex = HarnessAvailability(agentsFound: ["claude", "codex"])

@Test func theMacRowFollowsWhatTheLocalRunnerSaid() {
    func mac(_ local: LocalRunnerState) -> MacStep {
        WelcomeSteps(local: local, hasRepositories: false, orchestratorSeated: false).mac
    }
    #expect(mac(.checking) == .checking)
    #expect(mac(.notRunning) == .serviceDown)
    #expect(mac(.running(tmuxFound: false, agents: claudeAndCodex)) == .noTmux)
    #expect(mac(.running(tmuxFound: true, agents: HarnessAvailability(agentsFound: []))) == .noAgent)
    #expect(mac(.running(tmuxFound: true, agents: claudeAndCodex)) == .ready([.claude, .codex]))
    // A runner that didn't say is not one with no agents.
    #expect(mac(.running(tmuxFound: true, agents: HarnessAvailability(agentsFound: nil))) == .ready([]))
}

@Test func onlyAMissingPieceIsAProblem() {
    #expect(MacStep.noTmux.isProblem && MacStep.noAgent.isProblem && MacStep.serviceDown.isProblem)
    #expect(!MacStep.checking.isProblem && !MacStep.ready([.claude]).isProblem)
}

@Test func theRepositoryRowIsDoneOnceAnyRunnerListsOne() {
    let ready = LocalRunnerState.running(tmuxFound: true, agents: claudeAndCodex)
    #expect(WelcomeSteps(local: ready, hasRepositories: false, orchestratorSeated: false).repository == .toDo)
    #expect(WelcomeSteps(local: ready, hasRepositories: true, orchestratorSeated: false).repository == .done)
    // Not held back by This Mac: the repository may be on a Linux runner.
    #expect(WelcomeSteps(local: .notRunning, hasRepositories: false, orchestratorSeated: false).repository == .toDo)
}

@Test func theOrchestratorRowWaitsForARepositoryAndAReadyMac() {
    let ready = LocalRunnerState.running(tmuxFound: true, agents: claudeAndCodex)
    let noTmux = LocalRunnerState.running(tmuxFound: false, agents: claudeAndCodex)
    #expect(WelcomeSteps(local: ready, hasRepositories: false, orchestratorSeated: false).orchestrator == .waiting)
    #expect(WelcomeSteps(local: noTmux, hasRepositories: true, orchestratorSeated: false).orchestrator == .waiting)
    #expect(WelcomeSteps(local: ready, hasRepositories: true, orchestratorSeated: false).orchestrator == .toDo)
    #expect(WelcomeSteps(local: ready, hasRepositories: true, orchestratorSeated: true).orchestrator == .done)
}

// MARK: - The 127 exit

@Test func aCommandNotFoundAtTheStartIsNotInstalled() {
    #expect(OrchestratorExit.classify(exitCode: 127, ranFor: 3) == .notInstalled)
    #expect(OrchestratorExit.classify(exitCode: 127, ranFor: 15) == .notInstalled)
}

@Test func anythingElseIsAnOrdinaryStop() {
    #expect(OrchestratorExit.classify(exitCode: 127, ranFor: 15.5) == nil)
    #expect(OrchestratorExit.classify(exitCode: 127, ranFor: 60) == nil)
    #expect(OrchestratorExit.classify(exitCode: 1, ranFor: 3) == nil)
    #expect(OrchestratorExit.classify(exitCode: 0, ranFor: 3) == nil)
    #expect(OrchestratorExit.classify(exitCode: nil, ranFor: 3) == nil)
}

// MARK: - Harness availability

@Test func cursorIsFoundAsCursorAgent() {
    let cursor = HarnessAvailability(agentsFound: ["cursor-agent"])
    #expect(cursor.isInstalled(.cursor))
    #expect(cursor.installed == [.cursor])
    #expect(cursor.missing == [.claude, .codex])
    // `cursor` is the editor's command, not the agent's.
    #expect(!HarnessAvailability(agentsFound: ["cursor"]).isInstalled(.cursor))
}

@Test func theProgramsAreTheOnesTheRunnerSends() {
    // `wire::AGENT_PROGRAMS` in crates/daemon, in the same order.
    #expect(AgentHarness.allCases.map(\.program) == ["claude", "codex", "cursor-agent"])
    #expect(AgentHarness.allCases.map(\.title) == ["Claude Code", "Codex", "Cursor"])
}

@Test func aRunnerThatDidNotSayOffersEveryHarness() {
    let unknown = HarnessAvailability(agentsFound: nil)
    #expect(!unknown.isKnown)
    #expect(unknown.installed == AgentHarness.allCases)
    #expect(unknown.missing.isEmpty)
    let none = HarnessAvailability(agentsFound: [])
    #expect(none.installed.isEmpty)
    #expect(none.missing == AgentHarness.allCases)
}

// MARK: - The words

@Test func agentListsUseTheSerialComma() {
    #expect(FirstRunCopy.Welcome.macReady([.claude]) == "Ready to run Claude Code.")
    #expect(FirstRunCopy.Welcome.macReady([.claude, .codex]) == "Ready to run Claude Code and Codex.")
    #expect(FirstRunCopy.Welcome.macReady(AgentHarness.allCases) == "Ready to run Claude Code, Codex, and Cursor.")
    #expect(FirstRunCopy.Conversation.someMissing([.cursor], on: .thisMac) == "Cursor isn’t installed on this Mac.")
    #expect(
        FirstRunCopy.Conversation.someMissing([.codex, .cursor], on: .runner("build-01"))
            == "Codex and Cursor aren’t installed on build-01.")
}

@Test func notInstalledNamesTheCommandAndTheRunner() {
    #expect(FirstRunCopy.Conversation.notInstalledTitle(.claude) == "Claude Code Isn’t Installed")
    let remote = FirstRunCopy.Conversation.notInstalledBody(.cursor, on: .runner("build-01"))
    #expect(remote.contains("no cursor-agent command on build-01"))
    #expect(FirstRunCopy.Conversation.notInstalledBody(.claude, on: .thisMac).contains("runs in Terminal"))
    // Installing Cursor, the editor, doesn't give you cursor-agent.
    #expect(FirstRunCopy.Conversation.notInstalledTitle(.cursor) == "Cursor CLI Isn’t Installed")
    #expect(remote.contains("Install the Cursor CLI there"))
}

@Test func theExplainerPromisesNothingAboutAClosedApp() {
    // A push needs a signed-in device and a paired runner; a first run has
    // neither.
    #expect(!NotificationAsk.message.contains("closed"))
}

/// The voice check. Every string the first run shows, against the rules a
/// reviewer would otherwise have to remember.
@Test func everyStringReadsInTheAppsOwnVoice() {
    let stock = ["seamless", "effortless", "unlock", "supercharge", "elevate", "dive in", "get started", "let’s"]
    #expect(FirstRunCopy.all.count > 60)
    for string in FirstRunCopy.all {
        #expect(!string.contains("!"), "an exclamation mark: \(string)")
        #expect(!string.contains("'"), "a straight apostrophe: \(string)")
        #expect(!string.contains(" — "), "a spaced em dash: \(string)")
        #expect(!string.hasSuffix(" "), "a trailing space: \(string)")
        for phrase in stock {
            #expect(!string.lowercased().contains(phrase), "\"\(phrase)\": \(string)")
        }
    }
}
