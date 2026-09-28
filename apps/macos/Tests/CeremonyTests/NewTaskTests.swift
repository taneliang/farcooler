import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// New Task… and a question's Answer buttons are Control-scope writes, so a
/// connection granted only Read sees the board without them (spec §2.5, §5).
///
/// Held to `TaskBoardWrites.offered`, which the board's header, its empty
/// note and the card all read through `TaskBoardStore.offersWrites`. The Mac
/// reaches a runner over its own shell key, which the daemon reads as
/// host_admin, so a Mac never meets `read` today; the rule is here so a
/// read-scoped Mac gets no dead button when it does.
@MainActor
struct NewTaskTests {
    private static func build(_ scope: String) -> DaemonBuild {
        DaemonBuild(
            version: "1", matches: true, platform: "", capabilities: ["tasks"], grantedScope: scope)
    }

    @Test("A read-only runner offers no New Task")
    func aReadOnlyRunnerOffersNoNewTask() {
        #expect(!TaskBoardWrites.offered(by: Self.build("read")))
        #expect(TaskBoardWrites.offered(by: Self.build("control")))
        #expect(TaskBoardWrites.offered(by: Self.build("host_admin")))
        // No answer, or one this build has no word for, is not a refusal.
        #expect(TaskBoardWrites.offered(by: Self.build("unspecified")))
        #expect(TaskBoardWrites.offered(by: nil))
    }
}
