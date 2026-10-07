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
///   FARCOOLER_PERF=1 apps/macos/test.sh -c release -Xswiftc -enable-testing -Xswiftc -DDEBUG -j 4 \
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

    /// The terminal views under `view`, however deep.
    static func renderViews(in view: NSView) -> [TerminalRenderView] {
        (view as? TerminalRenderView).map { [$0] } ?? view.subviews.flatMap(renderViews(in:))
    }

    static let runs = 5
    static let barMs = 35.0

    private static func environment(_ home: String) -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        ScratchDaemon.isolate(&environment, home: home + "/h", config: home + "/config.toml")
        environment["FARCOOLER_TEST_STUB_AGENTS"] = "1"
        return environment
    }

    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        let cli = NativeAgentRunnerTests.cliPath!
        let ran = await ProcessRunner.run(cli, args, environment: Self.environment(home), deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    @Test func aReplyStreamsOverALiveTerminalWithoutStalling() async throws {
        // The offscreen window is visible to nobody, and a terminal in a
        // window nobody can see pauses its display link and draws nothing
        // (ov-229), which is the cost this measures.
        let wasVisible = WindowVisibility.assumeVisible
        WindowVisibility.assumeVisible = true
        defer { WindowVisibility.assumeVisible = wasVisible }
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
            _ = await farcooler(["terminal", "send", id, "sh -c \"while :; do printf 'row %s abcdefghijklmnopqrstuvwxyz\\n' \\$RANDOM; sleep 0.02; done\"\r"], home: home)

            // The pane is printing, so what the surface streams is live.
            var printing = false
            for _ in 0..<100 where !printing {
                let screen = await farcooler(["terminal", "screen", id], home: home)
                // Output, not the command's own echo, which has no digits after `row `.
                printing = String(decoding: screen.out, as: UTF8.self).contains(/row [0-9]+ abcdefghij/)
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
                let surfaces = Self.renderViews(in: try #require(window.contentView))
                try #require(surfaces.count == 1, "\(surfaces.count) terminal surfaces in the window")
                let surface = surfaces[0]
                let drawnBefore = surface.framesDrawn
                let revisionBefore = surface.core.revision
                // A display link of a window on no screen never fires, and a
                // window on a screen would sit in front of somebody's work. So
                // the test is the display link: it ticks the surface at 60 Hz
                // and lets the window draw what that invalidated, as vsync
                // would, on the main thread the monitor is timing.
                let vsync = Task { @MainActor in
                    while !Task.isCancelled {
                        surface.tick()
                        window.displayIfNeeded()
                        try? await Task.sleep(for: .milliseconds(16))
                    }
                }
                let monitor = HangMonitor()
                monitor.start()
                _ = monitor.take()
                let streamed = ContinuousClock.now
                while await !source.done, ContinuousClock.now - streamed < .seconds(90) {
                    try await Task.sleep(for: .milliseconds(50))
                }
                // The terminal attached, received the pane's stream and drew it
                // while the reply streamed: about fifty lines a second, for a
                // few seconds.
                vsync.cancel()
                let drawn = surface.framesDrawn - drawnBefore
                #expect(surface.core.revision != revisionBefore, "no byte of the pane's stream reached the terminal")
                #expect(drawn >= 20, "the terminal underneath drew \(drawn) frames, so it wasn't live")
                print("PERF[native+terminal] run \(run): the terminal drew \(drawn) frames")
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
