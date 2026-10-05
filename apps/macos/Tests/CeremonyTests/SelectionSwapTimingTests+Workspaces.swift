import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Switching workspaces in the real window (ov-293): `ContentView` against
/// the seeded scratch daemon `scripts/mac-capture.sh` starts, read back
/// every few ms from the moment the switch lands.
///
/// Opt-in, like `RealWindowCaptures`, and never in CI: enabled by
/// `FARCOOLER_SWITCH_TIMING`, the id of a second workspace to switch to and
/// back from (the seed's own is the first). The switch goes through the
/// window's own way of opening a place (`DestinationOpener`), never input.
/// `OV293_HEARTBEAT=1` also prints how long the main actor is held after
/// each switch, `OV293_FRESH=<id>,<id>…` adds first visits, and
/// `OV293_FRAMES=1` prints every reading.
///
/// Measured on Oct 4 in a release build (`swift test -c release -Xswiftc
/// -enable-testing -Xswiftc -DDEBUG`), each workspace with a shell seated as
/// its orchestrator: a switch back holds the main actor 28–36 ms (17–28
/// without an orchestrator), and its first frame is the new workspace
/// whole, terminals included (`TerminalScreens`). A first visit holds it
/// 33–87 ms and draws the board's spinner and a blank terminal, filled in
/// as `task list` and the stream arrive, 100–140 ms in. Dropping the
/// `.id(scene.key)` rebuild made it worse, not better: 73–250 ms holds, the
/// reused views re-laying out against another workspace's terminals.
extension SelectionSwapTimingTests {
    nonisolated static let environment = ProcessInfo.processInfo.environment

    /// The window's pixels, every fourth in each direction.
    static func thumbnail(_ view: NSView) -> [UInt8] {
        guard let rep = view.lookBitmap(scale: 1), let data = rep.bitmapData else { return [] }
        var out: [UInt8] = []
        out.reserveCapacity((rep.pixelsWide / 4) * (rep.pixelsHigh / 4) * 3)
        for y in stride(from: 0, to: rep.pixelsHigh, by: 4) {
            for x in stride(from: 0, to: rep.pixelsWide, by: 4) {
                let p = data + y * rep.bytesPerRow + x * 4
                out.append(contentsOf: [p[0], p[1], p[2]])
            }
        }
        return out
    }

    /// The share of `a`'s pixels visibly unlike `b`'s.
    static func distance(_ a: [UInt8], _ b: [UInt8]) -> Double {
        guard a.count == b.count, !a.isEmpty else { return 1 }
        var differ = 0
        for i in stride(from: 0, to: a.count, by: 3)
        where abs(Int(a[i]) - Int(b[i])) + abs(Int(a[i + 1]) - Int(b[i + 1])) + abs(Int(a[i + 2]) - Int(b[i + 2])) > 24 {
            differ += 1
        }
        return Double(differ) / Double(a.count / 3)
    }

    static func ms(_ d: Duration) -> Double {
        Double(d.components.seconds) * 1000 + Double(d.components.attoseconds) / 1e15
    }

    /// One switch to `workspace`: each reading after it lands, as (ms, the
    /// share of pixels still unlike the last reading).
    static func switchWorkspace(to workspace: String, in window: NSWindow, span: Double) async -> [(Double, Double)] {
        let view = window.contentView!.superview!
        let opener = DestinationOpener.shared
        // Dated past the wait a window that isn't key leaves for the key one.
        let open = DestinationOpen(
            destination: Destination(runner: .init(host: ""), place: .workspace(workspace)),
            arrival: .notification, since: Date().addingTimeInterval(-1))
        opener.request(open)
        let clock = ContinuousClock()
        // It's finished in the turn it lands.
        while opener.pending?.id == open.id { try? await Task.sleep(for: .milliseconds(1)) }
        let start = clock.now
        if environment["OV293_HEARTBEAT"] != nil {
            // A 1 ms sleep that wakes late was waiting on the main actor.
            var last = start
            var held: [String] = []
            while start.duration(to: clock.now) < .milliseconds(600) {
                try? await Task.sleep(for: .milliseconds(1))
                let now = clock.now
                let gap = ms(last.duration(to: now))
                if gap > 8 { held.append(String(format: "%.0f ms held %.0f ms", ms(start.duration(to: now)), gap)) }
                last = now
            }
            print("ov-293 heartbeat: " + held.joined(separator: ", "))
        }
        var shots: [(Double, [UInt8])] = []
        while true {
            // Timed once drawn: drawing it is what takes the switch in.
            let shot = thumbnail(view)
            let at = ms(start.duration(to: clock.now))
            shots.append((at, shot))
            if at > span { break }
            try? await Task.sleep(for: .milliseconds(4))
        }
        let last = shots.last!.1
        return shots.map { ($0.0, distance($0.1, last)) }
    }

    /// When a switch is drawn: the first reading from which every one is
    /// within 1% of the last.
    static func drawnAt(_ curve: [(Double, Double)]) -> Double {
        var at = curve.last!.0
        for (ms, d) in curve.reversed() {
            if d > 0.01 { break }
            at = ms
        }
        return at
    }

    @Test(
        "Switching back to a workspace draws it whole in the first frame, with no fade",
        .enabled(if: SelectionSwapTimingTests.environment["FARCOOLER_SWITCH_TIMING"] != nil))
    func workspaceSwitchesAreWhole() async throws {
        let env = Self.environment
        let home = try #require(env["FARCOOLER_HOME"], "FARCOOLER_HOME must name the scratch daemon")
        try #require(home.hasPrefix("/tmp/") || home.hasPrefix("/private/tmp/"), "FARCOOLER_HOME isn't a scratch home: \(home)")
        try #require(env["FARCOOLER_BIN"] != nil, "FARCOOLER_BIN must name this checkout's CLI")
        unsetenv("FARCOOLER_WORKSPACE")
        let seed = try #require(
            env["FARCOOLER_CAPTURE_SEED"].flatMap { FileManager.default.contents(atPath: $0) }
                .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] })
        let first = try #require(seed["workspace"])
        let second = try #require(env["FARCOOLER_SWITCH_TIMING"])

        let defaults = UserDefaults.standard
        let touched = ["window.sessions.v1", SelectionMemory.destinationKey, SelectionMemory.key]
        let saved = Dictionary(uniqueKeysWithValues: touched.map { ($0, defaults.object(forKey: $0)) })
        defer { RealWindowCaptures.restore(saved, in: defaults) }
        defaults.removeObject(forKey: "window.sessions.v1")
        defaults.removeObject(forKey: SelectionMemory.destinationKey)
        defaults.set("|\(first)|", forKey: SelectionMemory.key)

        let window = try await TitleBarHarness.window(ContentView(), width: 1100, height: 700)
        try await Task.sleep(for: .seconds(5))
        var lines: [String] = []
        var revisits: [(String, [(Double, Double)])] = []
        for round in 0..<3 {
            for (name, target) in [("to second", second), ("to first", first)] {
                let curve = await Self.switchWorkspace(to: target, in: window, span: 1500)
                lines.append(String(format: "round %d %@: drawn %.0f ms, first frame %.0f ms (%.1f%% unlike the last)",
                    round, name, Self.drawnAt(curve), curve[0].0, curve[0].1 * 100))
                if env["OV293_FRAMES"] != nil {
                    lines.append(curve.prefix(40).map { String(format: "%.0f:%.3f", $0.0, $0.1) }.joined(separator: " "))
                }
                // The first round visits the second workspace for the first
                // time; after that, both have been drawn before.
                if round > 0 || name == "to first" { revisits.append(("round \(round) \(name)", curve)) }
                try await Task.sleep(for: .milliseconds(500))
            }
        }
        for (index, fresh) in (env["OV293_FRESH"] ?? "").split(separator: ",").map(String.init).enumerated() {
            let curve = await Self.switchWorkspace(to: fresh, in: window, span: 1500)
            lines.append(String(format: "first visit %d: drawn %.0f ms, first frame %.0f ms", index, Self.drawnAt(curve), curve[0].0))
            try await Task.sleep(for: .milliseconds(300))
        }
        window.close()
        print("ov-293 workspace switch\n" + lines.joined(separator: "\n"))
        for (name, curve) in revisits {
            // Whole at once: no fade, and nothing left to arrive.
            #expect(curve[0].1 <= 0.01, "\(name): the first frame was \(Int(curve[0].1 * 100))% unlike the last")
        }
    }
}
