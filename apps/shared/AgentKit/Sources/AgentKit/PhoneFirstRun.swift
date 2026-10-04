import Foundation

// What the phones' Needs You says before there's anything to answer (ov-205):
// a runner with no repositories, and repositories with no orchestrator.
//
// Pure, over the sections `Fleet.phoneSections` already makes, so the iPhone's
// screen and the tests read the same rule. The words are `FirstRunCopy.Phone`'s.

enum PhoneFirstRun {
    /// Whether a runner whose fleet was read lists no repository at all.
    /// `phoneSections` groups workspaces, worktrees and hidden worktrees by
    /// repository, so none means the runner has nothing to work on yet.
    static func hasNoRepositories(_ sections: [PhoneRepositorySection]) -> Bool {
        sections.isEmpty
    }

    /// Whether there are workspaces that could run an orchestrator and none
    /// does, on any runner. An implicit workspace (a repository on a runner
    /// too old for workspaces) can't have one, so it neither counts as a
    /// workspace here nor as a missing orchestrator.
    static func noOrchestratorAnywhere(_ sections: [PhoneRepositorySection]) -> Bool {
        let rows = sections.flatMap(\.workspaces).filter { !$0.isImplicit }
        return !rows.isEmpty && rows.allSatisfy { $0.orchestrator == nil }
    }

    /// What to say for no orchestrator, under an empty Needs You: nil when
    /// one is running somewhere, or when `working` agents are at it anyway
    /// (a person's own), since "No agents are working yet" would be untrue.
    static func noOrchestratorCopy(sections: [PhoneRepositorySection], working: Int = 0) -> PhoneEmptyCopy? {
        guard working == 0, noOrchestratorAnywhere(sections) else { return nil }
        return PhoneEmptyStates.noAgentsWorking
    }
}
