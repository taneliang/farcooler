import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in capture of the production task subagent section (ov-885).
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct ModelBrandCaptures {
    @Test func subagent() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        let now = Date()
        var row = TaskRow(id: "capture", key: "ov-885", title: "Model branding", status: .inProgress,
                          statusSince: now, createdAt: now, updatedAt: now)
        row.workers = [TaskWorker(harness: "codex", state: .finished, startedAt: now.addingTimeInterval(-60),
                                  endedAt: now, doing: "", orchestratorTerminalID: nil, model: "gpt-6.1-sol")]
        let window = NativePolishTests.window(width: 480, height: 140,
            TaskWorkersSection(row: row).padding(20).frame(maxWidth: .infinity, alignment: .leading))
        defer { window.close() }
        try await Capture.keep(window, to: URL(fileURLWithPath: out).appendingPathComponent("ov-885-after.png"),
                               expecting: "the task subagent section") { true }
    }
}
