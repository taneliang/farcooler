import Foundation
import Testing

@testable import AgentKit

/// What a workspace's orchestrator tab is called. The iOS target has no unit
/// tests, so what the bar says is decided here and read here.
struct ShellWorkspacesTests {
    /// Two repositories: `r1` split into Main and Billing, Billing's
    /// orchestrator running in Main's checkout, Main's naming a terminal
    /// nobody listed; `r2` with Main alone.
    static let fleet = """
    {
      "runtime_healthy": true, "live_panes": 3,
      "workspaces": [
        {"id": "b1", "repository": "r1", "name": "Billing", "task_prefix": "bil",
         "is_main": false, "ordinal": 1, "orchestrator": "orch-b"},
        {"id": "m1", "repository": "r1", "name": "Main", "task_prefix": "ov",
         "is_main": true, "ordinal": 0, "orchestrator": "gone"},
        {"id": "m2", "repository": "r2", "name": "Main", "task_prefix": "sc",
         "is_main": true, "ordinal": 0}
      ],
      "worktrees": [
        {"id": "checkout", "short": "c", "repository": "r1", "task": "overnight", "branch": "main",
         "state": "active", "workspace": "m1",
         "terminals": [
           {"id": "shell", "short": "s", "title": "", "preset": "zsh", "state": "running",
            "epoch": 1, "workspace": "m1", "role": "shell"},
           {"id": "orch-b", "short": "o", "title": "", "preset": "claude", "state": "running",
            "epoch": 1, "workspace": "b1", "role": "orchestrator"}
         ]},
        {"id": "loose", "short": "l", "repository": "r1", "task": "loose", "branch": "l",
         "state": "active", "workspace": null, "terminals": []},
        {"id": "invoices", "short": "i", "repository": "r1", "task": "invoices", "branch": "i",
         "state": "active", "workspace": "b1", "terminals": []},
        {"id": "scratch", "short": "x", "repository": "r2", "task": "scratch", "branch": "x",
         "state": "active", "workspace": "m2", "terminals": []}
      ]
    }
    """

    /// An orchestrator's tab is called after its workspace, so the bar in
    /// Main's checkout says the pane is Billing's manager and not one more
    /// of Main's terminals. Only a listed orchestrator gets a title: Main's
    /// names a terminal nobody listed, so Main has none rather than one that
    /// names a tab that does not exist.
    @Test func anOrchestratorsTabIsNamedForItsWorkspace() throws {
        let fleet = try FleetDecodeTests.decodeFleet(Self.fleet)
        #expect(fleet.orchestratorTitles() == ["orch-b": "Billing Orchestrator"])
    }

    /// A runner without `workstreams` has no workspaces to name a tab after.
    @Test func aRunnerWithoutWorkspacesHasNoTitles() throws {
        let old = """
        {"runtime_healthy": true, "live_panes": 0,
         "worktrees": [{"id": "w1", "short": "w1", "repository": "r1", "task": "t",
                        "branch": "b", "state": "active", "terminals": []}]}
        """
        #expect(try FleetDecodeTests.decodeFleet(old).orchestratorTitles() == nil)
    }
}
