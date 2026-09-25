import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Starting a task from the ⌘N panel, against a runner that is only a stub.
///
/// The failure these guard is invisible by looking at code that seems to work:
/// `startTask` used to hold the window for up to a minute while it waited for
/// the agent to look idle, and gave up without a word when a trust screen or
/// an update offer made it look blocked instead. The stub's agent never
/// becomes idle unless a test says so, which is exactly the runner that made
/// the old code wait.
@MainActor
struct StartTaskTests {
    /// A runner in miniature: what the CLI would print, and every command it
    /// was asked to run.
    @MainActor
    final class Runner {
        var calls: [[String]] = []
        var capabilities: [String]
        var workspaceMade = false
        var terminalMade = false
        var activity: String?

        init(capabilities: [String]) { self.capabilities = capabilities }

        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            calls.append(args)
            switch Array(args.prefix(2)) {
            case ["workspace", "create"]: workspaceMade = true
            case ["terminal", "create"]: terminalMade = true
            case ["workspace", "list"]: return (fleet(), nil)
            case ["--json", "status"]:
                let body: [String: Any] = [
                    "daemonVersion": "0.1.0+test", "buildsMatch": true, "platform": "macos",
                    "capabilities": capabilities,
                ]
                return (try? JSONSerialization.data(withJSONObject: body), nil)
            default: break
            }
            return (Data(), nil)
        }

        private func fleet() -> Data {
            var terminals: [[String: Any]] = []
            if terminalMade {
                var terminal: [String: Any] = [
                    "id": "t-new", "short": "tnew", "title": "claude", "preset": "claude",
                    "state": "running", "epoch": 0,
                ]
                if let activity { terminal["activity"] = activity }
                terminals.append(terminal)
            }
            var workspaces: [[String: Any]] = [
                [
                    "id": "w-main", "short": "wmain", "task": "main", "branch": "main",
                    "worktree": "/tmp/repo", "state": "active", "terminals": [],
                ]
            ]
            if workspaceMade {
                workspaces.append([
                    "id": "w-new", "short": "wnew", "task": "fix-flaky-test", "branch": "fix",
                    "worktree": "/tmp/worktrees/repo/fix", "state": "active",
                    "terminals": terminals,
                ])
            }
            let body: [String: Any] = [
                "runtime_healthy": true, "live_panes": 0, "workspaces": workspaces,
            ]
            return (try? JSONSerialization.data(withJSONObject: body)) ?? Data()
        }

        var sent: [[String]] { calls.filter { $0.starts(with: ["terminal", "send"]) } }
        var terminalCreate: [String]? { calls.first { $0.starts(with: ["terminal", "create"]) } }
    }

    /// A client wired to `runner`, connected, with the runner's build read.
    private func client(_ runner: Runner) async -> DaemonClient {
        let client = DaemonClient(target: "")
        client.commandRunnerForTesting = { args in runner.answer(args) }
        await client.refresh()
        // The build is read in a detached task on the first connection.
        for _ in 0..<100 where client.daemonBuild == nil {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(client.daemonBuild != nil, "the stub's status was read")
        return client
    }

    private static let description = "Please fix the \"flaky\" reconnect test — it's $HOME"

    @Test func aRunnerThatTakesALaunchPromptGetsTheTaskAsOneAndNothingIsTyped() async {
        let runner = Runner(capabilities: ["workspaces", "terminals", "launch_prompt"])
        let client = await client(runner)

        let started = ContinuousClock.now
        let workspace = await client.startTask(
            project: "repo", description: Self.description, agent: "claude")
        let took = ContinuousClock.now - started

        #expect(workspace == "w-new")
        #expect(took < .seconds(2), "returned once the terminal existed, not after a wait: \(took)")
        #expect(runner.terminalCreate?.contains("--prompt=\(Self.description)") == true)
        // Nothing is typed, then or later — the agent already has it. Idle
        // now, which is what the typing path waits for, so a typing path
        // still running would type within its next poll.
        runner.activity = "idle"
        try? await Task.sleep(for: .milliseconds(1500))
        #expect(runner.sent.isEmpty, "typed as well: \(runner.sent)")
    }

    @Test func anOlderRunnerStillGetsTheTaskTypedButTheWindowDoesNotWaitForIt() async {
        let runner = Runner(capabilities: ["workspaces", "terminals"])
        let client = await client(runner)

        let started = ContinuousClock.now
        let workspace = await client.startTask(
            project: "repo", description: Self.description, agent: "claude")
        let took = ContinuousClock.now - started

        #expect(workspace == "w-new")
        #expect(took < .seconds(2), "the typing no longer holds the window: \(took)")
        #expect(runner.terminalCreate?.contains { $0.hasPrefix("--prompt") } == false)

        // Once the agent is ready, the description is typed, as before.
        runner.activity = "idle"
        for _ in 0..<60 where runner.sent.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        #expect(runner.sent.first == ["terminal", "send", "tnew", Self.description])
    }

    @Test func aPromptThatStartsWithADashIsStillOneArgument() {
        let args = DaemonClient.taskTerminalArguments(
            workspace: "w", agent: "codex", prompt: "--help me")
        #expect(args == ["terminal", "create", "w", "--preset", "codex", "--title", "codex",
                         "--prompt=--help me"])
        #expect(
            !DaemonClient.taskTerminalArguments(workspace: "w", agent: "codex", prompt: nil)
                .contains { $0.hasPrefix("--prompt") })
    }
}
