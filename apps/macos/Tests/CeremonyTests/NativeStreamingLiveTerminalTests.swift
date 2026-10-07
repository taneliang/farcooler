import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// ov-394: `NativeStreamingStallTests`' check, with what the first one left
/// out: the real `TerminalSurface` live underneath, streaming a real pane of a
/// scratch daemon that prints steadily, while the conversation shows and a
/// reply streams into it. Run it in a release build, five times over:
///
///   FARCOOLER_PERF=1 apps/macos/test.sh -c release -Xswiftc -enable-testing -j 4 \
///     --filter NativeStreamingLiveTerminalTests
///
/// The bar is a longest stall of 35 ms on every run, the 50 ms bar and real
/// headroom. Opt-in, as a timing harness on a shared machine can't be a gate.
@MainActor
@Suite(
    .serialized,
    .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1" && NativeAgentRunnerTests.runnable))
struct NativeStreamingLiveTerminalTests {
    typealias HangMonitor = StreamingReplyStallTests.HangMonitor

    static let runs = 5
    static let barMs = 35.0

    private static func environment(_ home: String) -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = home + "/h"
        environment["FARCOOLER_TEST_STUB_AGENTS"] = "1"
        environment["FARCOOLER_CONFIG"] = home + "/config.toml"
        return environment
    }

    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        let cli = NativeAgentRunnerTests.cliPath!
        let ran = await ProcessRunner.run(cli, args, environment: Self.environment(home), deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    @Test func aReplyStreamsOverALiveTerminalWithoutStalling() async throws {
        let home = "/tmp/fcl-\(UUID().uuidString.prefix(6))"
        defer { try? FileManager.default.removeItem(atPath: home) }
        let demo = home + "/repos/demo"
        try FileManager.default.createDirectory(atPath: demo, withIntermediateDirectories: true)
        for args in [["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]] {
            _ = await ProcessRunner.run("/usr/bin/git", ["-C", demo] + args, deadline: 30)
        }
        _ = await farcooler(["--json", "daemon", "ensure"], home: home)
        var worst: [Double] = []
        do {
            _ = await farcooler(["root", "add", home + "/repos"], home: home)
            _ = await farcooler(["repo", "register", demo], home: home)
            let made = await farcooler(["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"], home: home)
            let worktree = try #require((try JSONSerialization.jsonObject(with: made.out) as? [String: Any])?["short"] as? String, "\(made.err)")
            let created = await farcooler(["--json", "terminal", "create", worktree], home: home)
            let object = try #require(try JSONSerialization.jsonObject(with: created.out) as? [String: Any], "\(created.err)")
            let id = try #require(object["id"] as? String)
            let short = try #require(object["short"] as? String)
            // A pane that prints about fifty lines a second, for ever.
            _ = await farcooler(["terminal", "send", id, "while :; do printf 'row %s abcdefghijklmnopqrstuvwxyz\\n' $RANDOM; sleep 0.02; done\r"], home: home)

            // The pane is printing, so what the surface streams is live.
            var printing = false
            for _ in 0..<100 where !printing {
                let screen = await farcooler(["terminal", "screen", id], home: home)
                printing = String(decoding: screen.out, as: UTF8.self).contains("row ")
                if !printing { try await Task.sleep(for: .milliseconds(100)) }
            }
            #expect(printing, "the scratch pane never printed")

            for run in 1...Self.runs {
                let terminal = try NativeAgentTests.terminal(id: id, program: "claude")
                let model = NativeAgentTests.model(terminal)
                let source = NativeStreamingStallTests.Streaming(held: 5_000, chars: 25_600, every: .milliseconds(52))
                model.source = source
                model.showsNative = true
                let agents = NativeAgentTests.agents(model: model)
                let env = Self.environment(home)
                let window = NativeAgentTests.window(
                    NativeSwitch(terminal: terminal, target: "", isFocused: true, agents: agents) { focused in
                        TerminalSurface(
                            terminal: short, binary: NativeAgentRunnerTests.cliPath, environment: env, hostArguments: [],
                            linkGeneration: 0, onResize: { _, _ in }, isFocused: focused)
                    })
                // Let the terminal attach and fill, and the first page draw.
                let began = ContinuousClock.now
                while model.store.ids.isEmpty, ContinuousClock.now - began < .seconds(30) {
                    try await Task.sleep(for: .milliseconds(10))
                }
                try await Task.sleep(for: .seconds(2))
                let monitor = HangMonitor()
                monitor.start()
                _ = monitor.take()
                let streamed = ContinuousClock.now
                while await !source.done, ContinuousClock.now - streamed < .seconds(90) {
                    try await Task.sleep(for: .milliseconds(50))
                }
                let when = monitor.when()
                let stalls = monitor.take()
                monitor.stop()
                let longest = stalls.max() ?? .infinity
                worst.append(longest)
                print("PERF[native+terminal] run \(run): \(HangMonitor.describe(stalls)) [\(when)]")
                model.store.stop()
                window.close()
                NativePaneModel.remember(false, for: terminal.id)
            }
        } catch {
            Issue.record("\(error)")
        }
        await ScratchDaemon.stop(cli: NativeAgentRunnerTests.cliPath!, farcoolerHome: home + "/h")
        print("PERF[native+terminal] longest stall per run: \(worst.map { String(format: "%.0f", $0) }) ms")
        #expect(worst.count == Self.runs)
        #expect((worst.max() ?? .infinity) <= Self.barMs, "longest \(worst) ms, bar \(Self.barMs) ms")
    }
}
