import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A task's Usage section in each state it can be in (ov-195), rendered in
/// both appearances, written where `FARCOOLER_GLANCE_OUT` says. Not an
/// assertion beyond "it drew" and "it says so".
@MainActor
struct TaskUsageSpecimenTests {
    /// bil-7's spend as the iOS harness has it: a claude turn that stated
    /// only part, and codex nobody can price.
    static let spend = "specimen: claude in part, codex unpriced"

    /// The client core's answer for `name` in `test/fixtures/task-usage-shown.json` (ov-1048).
    static func shown(_ name: String) throws -> TaskUsage {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/task-usage-shown.json"))
        let entries = try JSONSerialization.jsonObject(with: data) as? [[String: Any]] ?? []
        let entry = try #require(entries.first { $0["case"] as? String == name })
        return try TaskUsage.decode(JSONSerialization.data(withJSONObject: entry["shown"] ?? [:]))
    }

    @Test("Write the task usage sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let usage = try Self.shown(Self.spend)
        #expect(usage.said.rows.first?.detail == "1.1M tokens · $2.87 partly not reported")
        let nothing = try Self.shown("nothing recorded")
        let key = "task.usage.breakdown.open"
        let was = UserDefaults.standard.object(forKey: key)
        UserDefaults.standard.set(true, forKey: key)
        defer { UserDefaults.standard.set(was, forKey: key) }
        for dark in [false, true] {
            let host = NSHostingView(
                rootView: Specimen(usage: usage, nothing: nothing)
                    .background(dark ? Color(white: 0.12) : Color.white))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("task-usage-\(dark ? "dark" : "light").png"))
        }
    }
}

private struct Specimen: View {
    let usage: TaskUsage
    let nothing: TaskUsage

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            caption("Spend, with its breakdown open")
            TaskUsageView(state: .loaded(usage))
            caption("An older runner")
            TaskUsageView(state: .needsUpdate)
            caption("A read that didn’t come back")
            TaskUsageView(state: .failed)
            caption("Nothing yet")
            TaskUsageView(state: .loaded(nothing))
            caption("Reading")
            TaskUsageView(state: .loading)
        }
        .padding(20)
        .frame(width: 520, alignment: .leading)
    }

    private func caption(_ text: String) -> some View {
        Text(text.uppercased())
            .font(.system(size: 9, weight: .semibold))
            .foregroundStyle(.tertiary)
    }
}
