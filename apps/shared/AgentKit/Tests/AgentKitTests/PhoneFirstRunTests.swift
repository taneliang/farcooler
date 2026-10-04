import Foundation
import Testing

@testable import AgentKit

// The phones' Needs You before anything needs answering (ov-205).

private func row(_ id: String, orchestrator: String?, implicit: Bool = false) -> PhoneWorkspaceRow {
    PhoneWorkspaceRow(
        place: PhoneWorkspace(runner: "r", workspace: id), name: id, isImplicit: implicit,
        orchestrator: orchestrator, count: 0, unread: false)
}

private func section(_ repository: String, _ rows: [PhoneWorkspaceRow]) -> PhoneRepositorySection {
    PhoneRepositorySection(
        runner: "r", repository: repository, workspaces: rows, unclaimed: [], unclaimedCount: 0, hidden: [])
}

@Test func aRunnerWithNoSectionsHasNoRepositories() {
    #expect(PhoneFirstRun.hasNoRepositories([]))
    #expect(!PhoneFirstRun.hasNoRepositories([section("repo", [row("main", orchestrator: nil)])]))
}

@Test func noOrchestratorAnywhereNeedsAWorkspaceAndNoOrchestratorOnAny() {
    #expect(PhoneFirstRun.noOrchestratorAnywhere([section("a", [row("main", orchestrator: nil)])]))
    // One running anywhere, in another repository, is enough to say nothing.
    #expect(
        !PhoneFirstRun.noOrchestratorAnywhere([
            section("a", [row("main", orchestrator: nil)]), section("b", [row("w", orchestrator: "t1")]),
        ]))
    // Nothing at all is the other empty state, not this one.
    #expect(!PhoneFirstRun.noOrchestratorAnywhere([]))
    // An implicit workspace can't run one, so it can't be missing one.
    #expect(!PhoneFirstRun.noOrchestratorAnywhere([section("a", [row("a", orchestrator: nil, implicit: true)])]))
}

@Test func theNoOrchestratorCopyTeachesWhatAWorkspaceIsFor() {
    let copy = PhoneFirstRun.noOrchestratorCopy(sections: [section("a", [row("main", orchestrator: nil)])])
    #expect(copy == PhoneEmptyStates.noAgentsWorking)
    #expect(copy?.rows.contains { $0.text.contains("one line of work") } == true)
    #expect(PhoneFirstRun.noOrchestratorCopy(sections: []) == nil)
    // Agents already working make "no agents are working yet" untrue.
    #expect(PhoneFirstRun.noOrchestratorCopy(sections: [section("a", [row("main", orchestrator: nil)])], working: 2) == nil)
}

@Test func aQuickExit127NamesTheAgentThatWasAskedFor() {
    func missing(_ code: Int?, _ after: TimeInterval?, _ asked: AgentHarness? = .codex, _ preset: String = "claude") -> AgentHarness? {
        OrchestratorExit.missingAgent(exitCode: code, endedAfter: after, asked: asked, preset: preset)
    }
    #expect(missing(127, 6) == .codex)
    #expect(missing(127, 6, nil) == .claude)
    #expect(missing(127, nil) == nil)
    #expect(missing(127, 60) == nil)
    #expect(missing(1, 6) == nil)
    #expect(missing(nil, 6) == nil)
    #expect(missing(127, 6, nil, "shell") == nil)
}

@Test func aDaemonBuildReadsWhichHarnessesTheRunnerFound() {
    let said = DaemonBuild(version: "1", matches: true, platform: "linux", agentsFound: ["codex"])
    #expect(said.availability.installed == [.codex])
    #expect(said.availability.missing == [.claude, .cursor])
    let silent = DaemonBuild(version: "1", matches: true, platform: "linux")
    #expect(silent.availability.installed == AgentHarness.allCases)
}
