import AgentKit
import AppKit
import SwiftUI
import Testing
@testable import Far_Cooler

/// Fixture-only capture, opt-in. Never starts a runner or uses a row cache.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativeOperationCaptures {
    @Test func fixtureRows() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/agent-rows-ov893.json"))
        let page = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let rows = try #require(page["rows"] as? [[String: Any]])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (width, size) in [(CGFloat(420), "narrow"), (CGFloat(720), "normal")] {
            // Positive console unlock immediately before each window, never assume absent lock key is unlocked.
            let check = Process(); let pipe = Pipe()
            check.executableURL = URL(fileURLWithPath: "/usr/sbin/ioreg")
            check.arguments = ["-n", "Root", "-d1"]; check.standardOutput = pipe
            try check.run(); check.waitUntilExit()
            let state = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            try #require(check.terminationStatus == 0 && state.contains("\"IOConsoleLocked\" = No"), "Mac console must be positively unlocked")
            let model = NativeAgentTests.model(try NativeAgentTests.terminal())
            model.store.apply(try await model.store.ledger.page(NativeAgentTests.page(rows)))
            let frames = NativePolishTests.Frames()
            let window = NativePolishTests.window(width: width, height: 900, NativePolishTests.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
            defer { window.close() }
            try await Capture.wait(expecting: "fixture tool group", until: { frames.views["native-tool-group"] != nil })
            await NativeAgentTests.settle(window, 300)
            try await Capture.keep(to: URL(fileURLWithPath: out).appendingPathComponent("mac-\(size)-group.png"), expecting: "collapsed group", until: { frames.views["native-tool-group"] != nil }) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            try #require(NativePolishTests.click("section-header-toolgroup.tools:tool:note", frames: frames, in: window))
            await NativeAgentTests.settle(window, 400)
            try #require(NativePolishTests.click("section-header-tool.tool:note", frames: frames, in: window))
            await NativeAgentTests.settle(window, 400)
            try await Capture.keep(to: URL(fileURLWithPath: out).appendingPathComponent("mac-\(size)-expanded.png"), expecting: "raw excerpt open", until: { frames.views["native-tool-detail"] != nil }) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }
}
